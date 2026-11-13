//! Replications run by the caller's workers: how the browser runs a model on every core.
//!
//! A page cannot start threads, but it can start web workers, which share nothing with it and
//! talk only in messages. A run started with `"mode": "workers"` hands its replications out:
//!
//! 1. [`crate::Engine::worker_task`] gives the run as text (the model, the seed and the number of
//!    replications), which the caller sends to each worker.
//! 2. Each worker makes a [`Worker`] from it, and runs the replications it is given, one at a
//!    time, with [`Worker::run`]. The answer is text too, sent back as it is.
//! 3. [`crate::Engine::deliver`] takes each answer. Once every replication has answered, the run
//!    has finished, with exactly the results of a run on threads or in slices: each
//!    replication's random stream depends only on the seed and its index, and its report comes
//!    back bit for bit.
//!
//! Which worker runs which replication, and in what order they answer, does not matter.

use serde::{Deserialize, Serialize};
use stochcraft_blocks::{Compiled, ElementReport, Report, compile};
use stochcraft_experiment::Experiment;

use crate::run::run_failed;
use crate::{CommandError, ErrorCode, wire};

/// A run as its workers get it.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Task {
    pub model: wire::ModelDoc,
    pub seed: u64,
    pub replications: u64,
}

/// One worker's side of a run: the model compiled, ready to run any of its replications.
pub struct Worker {
    compiled: Compiled,
    experiment: Experiment,
}

impl Worker {
    /// A worker for the run described by `task` (from [`crate::Engine::worker_task`]).
    pub fn new(task: &str) -> Result<Self, CommandError> {
        let task: Task = serde_json::from_str(task)
            .map_err(|e| CommandError::new(ErrorCode::InvalidParams, format!("task: {e}")))?;
        let compiled = compile(&task.model.into()).map_err(|ds| CommandError {
            code: ErrorCode::InvalidModel,
            message: "the model has errors and cannot run".into(),
            details: Some(wire::diagnostics(&ds)),
        })?;
        Ok(Self {
            compiled,
            experiment: Experiment::new(task.replications, task.seed),
        })
    }

    /// Runs replication `index` (counted from 0) and answers with its report, or why it
    /// stopped, for [`crate::Engine::deliver`].
    pub fn run(&self, index: u64) -> String {
        let answer = match self.compiled.run(self.experiment.stream(index)) {
            Ok(report) => Answer {
                index,
                report: Some(Encoded::of(&report)),
                error: None,
            },
            Err(e) => Answer {
                index,
                report: None,
                error: Some(run_failed(index, &e).to_json()),
            },
        };
        serde_json::to_string(&answer).unwrap_or_default()
    }
}

/// What a worker sends back for one replication.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Answer {
    pub index: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<Encoded>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Value>,
}

impl Answer {
    /// The error a worker sent, as the run's error.
    pub fn error(&self) -> Option<CommandError> {
        let e = self.error.as_ref()?;
        let code = e["code"]
            .as_str()
            .and_then(ErrorCode::named)
            .unwrap_or(ErrorCode::Internal);
        Some(CommandError {
            code,
            message: e["message"].as_str().unwrap_or_default().to_owned(),
            details: e.get("details").cloned(),
        })
    }
}

/// A report as numbers only: names and kinds are the same on both sides, since both compiled
/// the same model, so they stay behind. Every number goes as its bits, so it comes back exactly.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Encoded {
    warmup: u64,
    duration: u64,
    stopped_at: Option<u64>,
    measured_from: u64,
    measured_to: u64,
    after_warmup: bool,
    elements: Vec<EncodedElement>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedElement {
    entered: u64,
    left: u64,
    /// The optional metrics, in [`ElementReport`]'s order.
    metrics: [Option<u64>; 29],
    lost: Option<u64>,
    work_in_progress: Option<u64>,
    properties: Vec<Option<u64>>,
}

fn bits(x: Option<f64>) -> Option<u64> {
    x.map(f64::to_bits)
}

fn float(b: Option<u64>) -> Option<f64> {
    b.map(f64::from_bits)
}

impl Encoded {
    fn of(r: &Report) -> Self {
        Self {
            warmup: r.warmup.to_bits(),
            duration: r.duration.to_bits(),
            stopped_at: bits(r.stopped_at),
            measured_from: r.measured_from.to_bits(),
            measured_to: r.measured_to.to_bits(),
            after_warmup: r.after_warmup,
            elements: r
                .elements
                .iter()
                .map(|e| EncodedElement {
                    entered: e.entered,
                    left: e.left,
                    metrics: [
                        bits(e.mean_queue_length),
                        bits(e.mean_wait),
                        bits(e.utilisation),
                        bits(e.mean_contents),
                        bits(e.mean_time_in_system),
                        bits(e.mean_blocked_time),
                        bits(e.blocking_probability),
                        bits(e.availability),
                        bits(e.loss_probability),
                        bits(e.time_off_shift),
                        bits(e.time_processing),
                        bits(e.time_starved),
                        bits(e.time_blocked),
                        bits(e.time_failed),
                        bits(e.energy),
                        bits(e.specific_energy),
                        bits(e.mean_work_in_progress),
                        bits(e.time_setup),
                        bits(e.oee_availability),
                        bits(e.oee_effectiveness),
                        bits(e.setup_ratio),
                        bits(e.quality_ratio),
                        bits(e.scrap_ratio),
                        bits(e.rework_ratio),
                        bits(e.oee),
                        bits(e.takt),
                        bits(e.demand),
                        bits(e.demand_met),
                        bits(e.cycle_time),
                    ],
                    lost: e.lost,
                    work_in_progress: e.work_in_progress,
                    properties: e.properties.iter().map(|(_, v)| bits(*v)).collect(),
                })
                .collect(),
        }
    }

    /// The report, with names and kinds from `template` (a report of the same compiled model).
    pub fn decode(self, template: &Report) -> Option<Report> {
        if self.elements.len() != template.elements.len() {
            return None;
        }
        let elements = self
            .elements
            .into_iter()
            .zip(&template.elements)
            .map(|(e, t)| {
                if e.properties.len() != t.properties.len() {
                    return None;
                }
                let [
                    q,
                    w,
                    u,
                    c,
                    s,
                    b,
                    bp,
                    a,
                    lp,
                    off,
                    pr,
                    st,
                    bl,
                    fa,
                    en,
                    se,
                    mw,
                    su,
                    oa,
                    oe,
                    sr,
                    qr,
                    scr,
                    rr,
                    oe2,
                    tk,
                    dm,
                    dmet,
                    ct,
                ] = e.metrics;
                Some(ElementReport {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    kind: t.kind,
                    entered: e.entered,
                    left: e.left,
                    mean_queue_length: float(q),
                    mean_wait: float(w),
                    utilisation: float(u),
                    mean_contents: float(c),
                    work_in_progress: e.work_in_progress,
                    mean_work_in_progress: float(mw),
                    mean_time_in_system: float(s),
                    mean_blocked_time: float(b),
                    blocking_probability: float(bp),
                    availability: float(a),
                    lost: e.lost,
                    loss_probability: float(lp),
                    time_off_shift: float(off),
                    time_processing: float(pr),
                    time_starved: float(st),
                    time_blocked: float(bl),
                    time_failed: float(fa),
                    time_setup: float(su),
                    oee_availability: float(oa),
                    oee_effectiveness: float(oe),
                    setup_ratio: float(sr),
                    quality_ratio: float(qr),
                    scrap_ratio: float(scr),
                    rework_ratio: float(rr),
                    oee: float(oe2),
                    takt: float(tk),
                    demand: float(dm),
                    demand_met: float(dmet),
                    cycle_time: float(ct),
                    energy: float(en),
                    specific_energy: float(se),
                    properties: t
                        .properties
                        .iter()
                        .zip(e.properties)
                        .map(|((name, _), v)| (name.clone(), float(v)))
                        .collect(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Report {
            warmup: f64::from_bits(self.warmup),
            duration: f64::from_bits(self.duration),
            stopped_at: float(self.stopped_at),
            measured_from: f64::from_bits(self.measured_from),
            measured_to: f64::from_bits(self.measured_to),
            after_warmup: self.after_warmup,
            elements,
        })
    }
}

/// The task for a run of `compiled`'s model.
pub(crate) fn task(compiled: &Compiled, experiment: &Experiment) -> String {
    let task = Task {
        model: compiled.model().into(),
        seed: experiment.seed,
        replications: experiment.replications,
    };
    serde_json::to_string(&task).unwrap_or_default()
}
