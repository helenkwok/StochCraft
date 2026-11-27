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
use stochcraft_blocks::{Compiled, ElementReport, Recording, Report, compile};
use stochcraft_experiment::Experiment;

use crate::run::run_failed;
use crate::{CommandError, ErrorCode, wire};

/// A run as its workers get it.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Task {
    pub model: wire::ModelDoc,
    pub seed: u64,
    pub replications: u64,
    /// The interval to record results over time at, when the run records them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<f64>,
    /// A run on a file of random bytes: the part of the file its replications read, from the
    /// first replication's segment to the last one's that the run may have, with each
    /// replication's segment length. Left out for pseudo-random numbers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub random: Option<TaskBytes>,
}

/// The bytes of a run on a file of random bytes, as its workers get them.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TaskBytes {
    /// Each replication's segment length, in bytes.
    pub segment_bytes: u64,
    /// The bytes, from the first replication's segment on, as lowercase hexadecimal.
    pub hex: String,
}

/// One worker's side of a run: the model compiled, ready to run any of its replications.
pub struct Worker {
    compiled: Compiled,
    experiment: Experiment,
    random: Option<crate::random::ByteFeed>,
}

impl Worker {
    /// A worker for the run described by `task` (from [`crate::Engine::worker_task`]).
    pub fn new(task: &str) -> Result<Self, CommandError> {
        let task: Task = serde_json::from_str(task)
            .map_err(|e| CommandError::new(ErrorCode::InvalidParams, format!("task: {e}")))?;
        let random = match &task.random {
            Some(b) => {
                let data = crate::random::from_hex(&b.hex).ok_or_else(|| {
                    CommandError::new(ErrorCode::InvalidParams, "task: `random.hex` is not hex")
                })?;
                Some(crate::random::ByteFeed::new(
                    data.into(),
                    0,
                    b.segment_bytes,
                ))
            }
            None => None,
        };
        let model: stochcraft_model::Model = task.model.into();
        let compiled = compile(&model)
            .map_err(|ds| CommandError {
                code: ErrorCode::InvalidModel,
                message: "the model has errors and cannot run".into(),
                details: Some(wire::diagnostics(&model, &ds)),
            })?
            .record_every(task.record);
        Ok(Self {
            compiled,
            experiment: Experiment::new(task.replications, task.seed),
            random,
        })
    }

    /// Runs replication `index` (counted from 0) and answers with its report, or why it
    /// stopped, for [`crate::Engine::deliver`].
    pub fn run(&self, index: u64) -> String {
        let rng = match &self.random {
            Some(feed) => feed.stream(index),
            None => self.experiment.stream(index),
        };
        let answer = match self.compiled.run(rng) {
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
    /// Results over time: the interval, the times, and each element's values in the order of
    /// its quantities (which both sides know from the model), all as bits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recording: Option<EncodedRecording>,
    /// The bytes read from a file of random bytes; left out when none.
    #[serde(default, skip_serializing_if = "is_zero")]
    random_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedRecording {
    interval: u64,
    times: Vec<u64>,
    elements: Vec<Vec<Vec<u64>>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedElement {
    entered: u64,
    left: u64,
    /// The optional metrics, in [`ElementReport`]'s order.
    metrics: [Option<u64>; 30],
    /// Servers with a patience: the abandoned share and wait, the work wasted and its share
    /// (apart, since serde takes arrays of at most 32).
    patience: [Option<u64>; 4],
    /// Servers with a health check: the mean units in service and the share of time
    /// restarting.
    health: [Option<u64>; 2],
    /// Servers with balking: the share balked.
    balking: [Option<u64>; 1],
    balked: Option<u64>,
    lost: Option<u64>,
    abandoned: Option<u64>,
    taken_out: Option<u64>,
    work_in_progress: Option<u64>,
    properties: Vec<Option<u64>>,
    /// What the behaviour checks need: the entities inside at the start and the end, the time
    /// inside and the units on shift, and at the start and the end those inside that had given
    /// up and those leaving by a marked way.
    #[serde(default)]
    inside: [u64; 2],
    #[serde(default)]
    checks: [Option<u64>; 2],
    #[serde(default)]
    given_up_inside: [u64; 2],
    #[serde(default)]
    leaving: [u64; 2],
    /// A conveyor's histogram of the times on it: start and width (as bits) and the counts.
    #[serde(default)]
    time_on_histogram: Option<(u64, u64, Vec<u64>)>,
    /// Blocks with starting contents: the entities placed inside as the run started.
    #[serde(default)]
    started: Option<u64>,
    /// Sinks that report time in system: its median, 90th and 95th percentiles and the share
    /// within the target.
    #[serde(default)]
    time_in_system: [Option<u64>; 4],
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
            random_bytes: r.random_bytes,
            recording: r.recording.as_ref().map(|rec| EncodedRecording {
                interval: rec.interval.to_bits(),
                times: rec.times.iter().map(|t| t.to_bits()).collect(),
                elements: rec
                    .elements
                    .iter()
                    .map(|qs| {
                        qs.iter()
                            .map(|(_, v)| v.iter().map(|x| x.to_bits()).collect())
                            .collect()
                    })
                    .collect(),
            }),
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
                        bits(e.mean_time_on),
                    ],
                    patience: [
                        bits(e.abandoned_share),
                        bits(e.abandoned_wait),
                        bits(e.wasted),
                        bits(e.wasted_share),
                    ],
                    health: [bits(e.mean_in_service), bits(e.time_restarting)],
                    balking: [bits(e.balked_share)],
                    balked: e.balked,
                    lost: e.lost,
                    abandoned: e.abandoned,
                    taken_out: e.taken_out,
                    work_in_progress: e.work_in_progress,
                    properties: e.properties.iter().map(|(_, v)| bits(*v)).collect(),
                    inside: [e.inside_at_start, e.inside_at_end],
                    checks: [bits(e.time_inside), bits(e.on_shift)],
                    given_up_inside: e.given_up_inside,
                    leaving: e.leaving,
                    time_on_histogram: e
                        .time_on_histogram
                        .as_ref()
                        .map(|h| (h.start.to_bits(), h.width.to_bits(), h.counts.clone())),
                    started: e.started,
                    time_in_system: [
                        bits(e.time_in_system_p50),
                        bits(e.time_in_system_p90),
                        bits(e.time_in_system_p95),
                        bits(e.within_target_share),
                    ],
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
                    on,
                ] = e.metrics;
                let [ash, aw, wa, wsh] = e.patience;
                let [mis, rs] = e.health;
                let [bsh] = e.balking;
                let [inside_at_start, inside_at_end] = e.inside;
                let [time_inside, on_shift] = e.checks;
                let [p50, p90, p95, within] = e.time_in_system;
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
                    time_in_system_p50: float(p50),
                    time_in_system_p90: float(p90),
                    time_in_system_p95: float(p95),
                    within_target_share: float(within),
                    mean_blocked_time: float(b),
                    mean_time_on: float(on),
                    time_on_histogram: e.time_on_histogram.map(|(start, width, counts)| {
                        stochcraft_blocks::Histogram {
                            start: f64::from_bits(start),
                            width: f64::from_bits(width),
                            counts,
                        }
                    }),
                    blocking_probability: float(bp),
                    availability: float(a),
                    lost: e.lost,
                    loss_probability: float(lp),
                    abandoned: e.abandoned,
                    abandoned_share: float(ash),
                    abandoned_wait: float(aw),
                    balked: e.balked,
                    balked_share: float(bsh),
                    wasted: float(wa),
                    wasted_share: float(wsh),
                    taken_out: e.taken_out,
                    mean_in_service: float(mis),
                    time_restarting: float(rs),
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
                    inside_at_start,
                    inside_at_end,
                    given_up_inside: e.given_up_inside,
                    leaving: e.leaving,
                    time_inside: float(time_inside),
                    on_shift: float(on_shift),
                    started: e.started,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        // The quantities from the template's recording (one with no time run), the values from
        // the answer; both or neither.
        let recording = match (self.recording, &template.recording) {
            (None, None) => None,
            (Some(rec), Some(t)) if rec.elements.len() == t.elements.len() => {
                let elements = rec
                    .elements
                    .into_iter()
                    .zip(&t.elements)
                    .map(|(values, qs)| {
                        (values.len() == qs.len()).then(|| {
                            qs.iter()
                                .zip(values)
                                .map(|((q, _), v)| {
                                    (*q, v.into_iter().map(f64::from_bits).collect())
                                })
                                .collect()
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(Recording {
                    interval: f64::from_bits(rec.interval),
                    times: rec.times.into_iter().map(f64::from_bits).collect(),
                    elements,
                })
            }
            _ => return None,
        };
        Some(Report {
            warmup: f64::from_bits(self.warmup),
            duration: f64::from_bits(self.duration),
            stopped_at: float(self.stopped_at),
            measured_from: f64::from_bits(self.measured_from),
            measured_to: f64::from_bits(self.measured_to),
            after_warmup: self.after_warmup,
            elements,
            recording,
            random_bytes: self.random_bytes,
        })
    }
}

/// The task for a run of `plan`.
pub(crate) fn task(plan: &crate::run::Plan) -> String {
    let (compiled, experiment) = (&plan.compiled, &plan.experiment);
    let task = Task {
        model: compiled.model().into(),
        seed: experiment.seed,
        replications: experiment.replications,
        record: compiled.recorded_every(),
        random: plan.random.as_ref().map(|feed| TaskBytes {
            segment_bytes: feed.segment(),
            hex: crate::random::to_hex(feed.window(plan.max_replications())),
        }),
    };
    serde_json::to_string(&task).unwrap_or_default()
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}
