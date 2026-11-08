//! Runs of an experiment in the background, with progress and cancellation.
//!
//! Natively a run goes to worker threads and the caller polls it. In the browser, where a page
//! cannot block, it runs on the page's web workers, or, where it cannot start them, in slices:
//! each poll runs a bounded number of events ([`SLICE_EVENTS`] unless asked otherwise) between
//! frames, and a replication that does not finish in one slice carries on in the next. All give
//! exactly the results of a run in one go, since each replication's random stream depends only on
//! the seed and its index, and slicing a replication changes neither its events nor their order.
//! Cancelling a threaded run stops the replications running within a thousand or so events, and
//! no more start; a sliced run stops between slices.
//!
//! A replication in which simulated time stops moving ends with [`ErrorCode::TimeStopped`]
//! (see [`stochcraft_blocks::MAX_EVENTS_AT_ONE_TIME`]), and stops the whole run.

#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
use std::sync::Mutex;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_blocks::{Compiled, Live, Report, RunError, summarise};
use stochcraft_experiment::{Control, Experiment, ExperimentError};

use crate::workers::{self, Answer};
use crate::{CommandError, ErrorCode, wire};

/// How a background run is carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// On worker threads, while the caller carries on (not available in the browser).
    Threads,
    /// A slice of events each time `run.status` is called (how the browser runs on one core,
    /// when it cannot start web workers).
    Slices,
    /// By the caller's own workers (how the browser runs on every core): the engine hands out
    /// the run with [`crate::Engine::worker_task`] and collects each replication's answer with
    /// [`crate::Engine::deliver`] (see the `workers` module).
    Workers,
}

/// The most events a sliced run runs per `run.status` call unless asked for another number: what
/// the page can run between two frames and still draw about 60 of them a second.
///
/// An estimate, not a measurement in the browser. Measured: a native release build on one core of
/// a desktop CPU (i7-12700K, November 2026) ran the test models at about 1,900 (the bakery plant,
/// with shifts and shared pools) to 7,200 (M/M/1 with a stop) events per millisecond
/// (`cargo test --release -p stochcraft-engine --test slices -- --ignored --nocapture`).
/// Assumed: WebAssembly in the browser runs at about half native speed and a typical laptop at
/// about two thirds of that desktop, so about 650 to 2,400 events per millisecond. A frame at 60
/// a second has 16.7 ms; keeping at least half of it for drawing leaves about 8 ms, which 5,000
/// events take on the slowest model (about 2 ms on most).
pub const SLICE_EVENTS: u64 = 5_000;

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
    /// What the results come from (see [`crate::Engine`]'s run record), attached to them.
    pub record: Value,
}

impl Plan {
    /// Runs every replication now, on this thread and any workers.
    pub fn run_now(&self) -> Result<Value, CommandError> {
        let reports = replicate(&self.compiled, &self.experiment, &Control::new())?;
        Ok(self.results(&reports))
    }

    fn results(&self, reports: &[Report]) -> Value {
        let mut v = wire::results(
            self.compiled.model(),
            self.experiment.replications,
            self.experiment.seed,
            &summarise(reports),
        );
        v["record"] = self.record.clone();
        v
    }
}

/// Runs every replication of `experiment`, reporting to `control`, which cancels them. The first
/// replication in which time stops moving cancels the others: they would most likely stop the
/// same way, each only after its own [`stochcraft_blocks::MAX_EVENTS_AT_ONE_TIME`] events.
fn replicate(
    compiled: &Compiled,
    experiment: &Experiment,
    control: &Control,
) -> Result<Vec<Report>, CommandError> {
    // The replication with the lowest index among those that stopped, and why.
    let stopped: Mutex<Option<(u64, RunError)>> = Mutex::new(None);
    let outcome = experiment.run_controlled(
        |r| {
            let index = r.index;
            let report = compiled.run_watched(r.rng, &|| control.is_cancelled());
            if let Err(e @ RunError::TimeStopped { .. }) = &report {
                let mut s = stopped.lock().unwrap_or_else(|e| e.into_inner());
                if s.as_ref().is_none_or(|(i, _)| index < *i) {
                    *s = Some((index, e.clone()));
                }
                control.cancel();
            }
            report
        },
        control,
    );
    if let Some((index, e)) = stopped.into_inner().unwrap_or_else(|e| e.into_inner()) {
        return Err(run_failed(index, &e));
    }
    outcome
        .map_err(failed)?
        .into_iter()
        .zip(0u64..)
        .map(|(r, index)| r.map_err(|e| run_failed(index, &e)))
        .collect()
}

fn failed(e: ExperimentError) -> CommandError {
    match e {
        ExperimentError::Cancelled => CommandError::new(ErrorCode::Cancelled, e.to_string()),
        ExperimentError::ReplicationPanicked { .. } => {
            CommandError::new(ErrorCode::Internal, e.to_string())
        }
    }
}

/// The error for replication `index`, which stopped with `e`.
pub(crate) fn run_failed(index: u64, e: &RunError) -> CommandError {
    match e {
        RunError::Cancelled => failed(ExperimentError::Cancelled),
        RunError::TimeStopped { time, element, .. } => CommandError {
            code: ErrorCode::TimeStopped,
            message: format!("replication {index}: {e}"),
            details: Some(json!({
                "replication": index,
                "time": time,
                "at": element.as_ref().map(|(id, _)| id.as_str()),
            })),
        },
        // A model problem found only by running it: reported as the model's diagnostics are.
        RunError::TimeOutOfRange { element, .. } => CommandError {
            code: ErrorCode::InvalidModel,
            message: format!("replication {index}: {e}"),
            details: Some(json!([{
                "severity": "error",
                "at": element.as_ref().map(|(id, _)| id.as_str()),
                "code": "time_out_of_range",
                "message": e.to_string(),
            }])),
        },
    }
}

enum State {
    #[cfg(not(target_arch = "wasm32"))]
    Threads {
        control: Arc<Control>,
        handle: std::thread::JoinHandle<Result<Vec<Report>, CommandError>>,
    },
    Slices {
        reports: Vec<Report>,
        /// The replication under way (index `reports.len()`), between slices.
        current: Option<Box<Live<'static>>>,
        cancelled: bool,
    },
    Workers {
        /// By replication index; `None` until its answer arrives.
        reports: Vec<Option<Report>>,
        received: u64,
        /// A report of the model with no time run, for the names and kinds answers leave out.
        template: Report,
        task: String,
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
                current: None,
                cancelled: false,
            },
            RunMode::Workers => {
                let rng = plan.experiment.stream(0);
                State::Workers {
                    reports: vec![None; usize::try_from(plan.experiment.replications).unwrap_or(0)],
                    received: 0,
                    template: plan.compiled.live(rng).report(),
                    task: workers::task(&plan.compiled, &plan.experiment),
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            RunMode::Threads => {
                let control = Arc::new(Control::new());
                let (c, p) = (Arc::clone(&control), plan.clone());
                let handle = std::thread::Builder::new()
                    .name("stochcraft-run".into())
                    .spawn(move || replicate(&p.compiled, &p.experiment, &c))
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

    /// Advances a sliced run until `step` replications have finished or `events` events have
    /// run, whichever comes first (a replication cut short carries on at the next poll; at
    /// least one event runs), collects a finished threaded run, and reports where the run is.
    pub fn poll(&mut self, step: u64, events: u64) -> Value {
        let total = self.total();
        match &mut self.state {
            State::Slices {
                reports,
                current,
                cancelled,
            } => {
                if *cancelled {
                    self.state = State::Finished(Err(failed(ExperimentError::Cancelled)));
                } else {
                    let x = &self.plan.experiment;
                    let (mut finished, mut left) = (0, events.max(1));
                    while finished < step.max(1) && (reports.len() as u64) < total && left > 0 {
                        let index = reports.len() as u64;
                        // The same replication as on threads: its stream, run to its end.
                        let live = current.get_or_insert_with(|| {
                            Box::new(self.plan.compiled.live(x.replication(index).rng))
                        });
                        let before = live.events();
                        match live.advance_by(left) {
                            Ok(done) => {
                                left = left.saturating_sub(live.events().saturating_sub(before));
                                if !done {
                                    break;
                                }
                                reports.push(live.report());
                                *current = None;
                                finished += 1;
                            }
                            Err(e) => {
                                self.state = State::Finished(Err(run_failed(index, &e)));
                                return self.status();
                            }
                        }
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
            State::Workers { .. } | State::Finished(_) => {}
        }
        self.status()
    }

    /// The run as its workers get it, while it waits for their answers.
    pub fn task(&self) -> Option<&str> {
        match &self.state {
            State::Workers { task, .. } => Some(task),
            _ => None,
        }
    }

    /// Takes a worker's answer for one replication. The first failure ends the run with it; an
    /// answer arriving after the run has finished is ignored.
    pub fn deliver(&mut self, answer: &str) -> Result<(), CommandError> {
        let State::Workers {
            reports,
            received,
            template,
            ..
        } = &mut self.state
        else {
            return Ok(());
        };
        let bad = |why: &str| CommandError::new(ErrorCode::InvalidParams, format!("answer: {why}"));
        let answer: Answer = serde_json::from_str(answer).map_err(|e| bad(&e.to_string()))?;
        if let Some(e) = answer.error() {
            self.state = State::Finished(Err(e));
            return Ok(());
        }
        let slot = usize::try_from(answer.index)
            .ok()
            .and_then(|i| reports.get_mut(i))
            .ok_or_else(|| bad("no such replication"))?;
        let report = answer
            .report
            .and_then(|r| r.decode(template))
            .ok_or_else(|| bad("not a report of this model"))?;
        if slot.replace(report).is_none() {
            *received += 1;
        }
        if *received == self.plan.experiment.replications {
            let r: Vec<Report> = std::mem::take(reports).into_iter().flatten().collect();
            self.state = State::Finished(Ok(self.plan.results(&r)));
        }
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn join(&mut self) {
        let state = std::mem::replace(&mut self.state, State::Finished(Ok(Value::Null)));
        self.state = match state {
            State::Threads { handle, .. } => State::Finished(match handle.join() {
                Ok(Ok(reports)) => Ok(self.plan.results(&reports)),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(CommandError::new(ErrorCode::Internal, "the run panicked")),
            }),
            other => other,
        };
    }

    /// Stops the run: the replications running stop soon after, and no more start. Returns once
    /// it has stopped.
    pub fn cancel(&mut self) -> Value {
        match &mut self.state {
            State::Slices { cancelled, .. } => *cancelled = true,
            State::Workers { .. } => {
                self.state = State::Finished(Err(failed(ExperimentError::Cancelled)));
            }
            #[cfg(not(target_arch = "wasm32"))]
            State::Threads { control, .. } => {
                control.cancel();
                self.join();
            }
            State::Finished(_) => {}
        }
        self.poll(0, 0)
    }

    pub fn status(&self) -> Value {
        let total = self.total();
        let (state, done, outcome) = match &self.state {
            State::Slices { reports, .. } => ("running", reports.len() as u64, None),
            State::Workers { received, .. } => ("running", *received, None),
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
        // A sliced run: how far the replication under way has got, in simulated time (0 to 1).
        if let State::Slices { current, .. } = &self.state {
            v["partial"] = json!(current.as_ref().map_or(0.0, |l| {
                if l.end() > 0.0 {
                    (l.now() / l.end()).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }));
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
