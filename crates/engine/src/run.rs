//! Runs of an experiment in the background, with progress and cancellation.
//!
//! Natively a run goes to worker threads and the caller polls it. In the browser, where a page
//! cannot block, it runs in slices: each poll runs a few replications between frames. Both give
//! exactly the results of a run in one go, since each replication's random stream depends only on
//! the seed and its index. Cancelling stops new replications from starting; one already running
//! finishes first.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_blocks::{Compiled, Report, summarise};
#[cfg(not(target_arch = "wasm32"))]
use stochcraft_experiment::Control;
use stochcraft_experiment::{Experiment, ExperimentError};

use crate::{CommandError, ErrorCode, wire};

/// How a background run is carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// On worker threads, while the caller carries on (not available in the browser).
    Threads,
    /// A few replications each time `run.status` is called (how the browser runs).
    Slices,
}

impl RunMode {
    pub fn default_for_target() -> Self {
        if cfg!(target_arch = "wasm32") {
            RunMode::Slices
        } else {
            RunMode::Threads
        }
    }
}

/// The settings of one run: the model's experiment, with any overrides applied.
#[derive(Debug, Clone)]
pub struct Plan {
    pub compiled: Compiled,
    pub experiment: Experiment,
}

impl Plan {
    /// Runs every replication now, on this thread and any workers.
    pub fn run_now(&self) -> Result<Value, CommandError> {
        let reports = self
            .experiment
            .run(|r| self.compiled.run(r.rng))
            .map_err(failed)?;
        Ok(self.results(&reports))
    }

    fn results(&self, reports: &[Report]) -> Value {
        wire::results(
            self.compiled.model(),
            self.experiment.replications,
            self.experiment.seed,
            &summarise(reports),
        )
    }
}

fn failed(e: ExperimentError) -> CommandError {
    match e {
        ExperimentError::Cancelled => CommandError::new(ErrorCode::Cancelled, e.to_string()),
        ExperimentError::ReplicationPanicked { .. } => {
            CommandError::new(ErrorCode::Internal, e.to_string())
        }
    }
}

enum State {
    #[cfg(not(target_arch = "wasm32"))]
    Threads {
        control: Arc<Control>,
        handle: std::thread::JoinHandle<Result<Vec<Report>, ExperimentError>>,
    },
    Slices {
        reports: Vec<Report>,
        cancelled: bool,
    },
    Finished(Result<Value, CommandError>),
}

/// A run in the background.
pub struct Job {
    plan: Plan,
    state: State,
}

impl Job {
    pub fn start(plan: Plan, mode: RunMode) -> Result<Self, CommandError> {
        let state = match mode {
            RunMode::Slices => State::Slices {
                reports: Vec::new(),
                cancelled: false,
            },
            #[cfg(not(target_arch = "wasm32"))]
            RunMode::Threads => {
                let control = Arc::new(Control::new());
                let (c, p) = (Arc::clone(&control), plan.clone());
                let handle = std::thread::Builder::new()
                    .name("stochcraft-run".into())
                    .spawn(move || p.experiment.run_controlled(|r| p.compiled.run(r.rng), &c))
                    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
                State::Threads { control, handle }
            }
            #[cfg(target_arch = "wasm32")]
            RunMode::Threads => {
                return Err(CommandError::new(
                    ErrorCode::Unsupported,
                    "threads are not available here; use \"mode\": \"slices\"",
                ));
            }
        };
        Ok(Self { plan, state })
    }

    fn total(&self) -> u64 {
        self.plan.experiment.replications
    }

    /// Advances a sliced run by up to `step` replications, collects a finished threaded run,
    /// and reports where the run is.
    pub fn poll(&mut self, step: u64) -> Value {
        let total = self.total();
        match &mut self.state {
            State::Slices { reports, cancelled } => {
                if *cancelled {
                    self.state = State::Finished(Err(failed(ExperimentError::Cancelled)));
                } else {
                    let x = &self.plan.experiment;
                    let start = reports.len() as u64;
                    for index in start..(start.saturating_add(step.max(1))).min(total) {
                        reports.push(self.plan.compiled.run(x.replication(index).rng));
                    }
                    if reports.len() as u64 >= total {
                        let r = std::mem::take(reports);
                        self.state = State::Finished(Ok(self.plan.results(&r)));
                    }
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            State::Threads { handle, .. } => {
                if handle.is_finished() {
                    self.join();
                }
            }
            State::Finished(_) => {}
        }
        self.status()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn join(&mut self) {
        let state = std::mem::replace(&mut self.state, State::Finished(Ok(Value::Null)));
        self.state = match state {
            State::Threads { handle, .. } => State::Finished(match handle.join() {
                Ok(Ok(reports)) => Ok(self.plan.results(&reports)),
                Ok(Err(e)) => Err(failed(e)),
                Err(_) => Err(CommandError::new(ErrorCode::Internal, "the run panicked")),
            }),
            other => other,
        };
    }

    /// Stops the run: no more replications start. Returns once it has stopped.
    pub fn cancel(&mut self) -> Value {
        match &mut self.state {
            State::Slices { cancelled, .. } => *cancelled = true,
            #[cfg(not(target_arch = "wasm32"))]
            State::Threads { control, .. } => {
                control.cancel();
                self.join();
            }
            State::Finished(_) => {}
        }
        self.poll(0)
    }

    pub fn status(&self) -> Value {
        let total = self.total();
        let (state, done, outcome) = match &self.state {
            State::Slices { reports, .. } => ("running", reports.len() as u64, None),
            #[cfg(not(target_arch = "wasm32"))]
            State::Threads { control, .. } => ("running", control.done(), None),
            State::Finished(Ok(_)) => ("finished", total, Some(&self.state)),
            State::Finished(Err(e)) if e.code == ErrorCode::Cancelled => {
                ("cancelled", 0, Some(&self.state))
            }
            State::Finished(Err(_)) => ("failed", 0, Some(&self.state)),
        };
        let mut v = json!({ "state": state, "total": total });
        match outcome {
            Some(State::Finished(Ok(result))) => {
                v["done"] = json!(total);
                v["result"] = result.clone();
            }
            Some(State::Finished(Err(e))) => v["error"] = e.to_json(),
            _ => v["done"] = json!(done),
        }
        v
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Job {
    /// An engine dropped mid-run stops its workers rather than leaving them running.
    fn drop(&mut self) {
        if let State::Threads { control, .. } = &self.state {
            control.cancel();
        }
    }
}
