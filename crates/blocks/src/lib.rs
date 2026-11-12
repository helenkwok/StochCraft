//! Flow blocks: turning a [`Model`] into a simulation.
//!
//! [`compile`] checks a [`stochcraft_model::Model`] and returns a [`Compiled`] model, or every
//! problem found. [`Compiled::run`] runs one replication and returns a [`Report`] of statistics
//! per element, collected after the warm-up. [`Compiled`] is `Send + Sync`, so replications can
//! run in parallel (with `stochcraft-experiment`), and [`summarise`] turns their reports into a
//! 95% confidence interval per element and [`Metric`].
//!
//! Entities are created by a source, wait for and occupy a server with one or more identical
//! units, spend time in a delay with no capacity limit, and leave through a sink. Gateways route
//! them at once: an exclusive gateway down one outgoing connection (by probability, or the first
//! condition that holds), a parallel gateway down all of them as copies, after waiting for one on
//! each incoming connection when it has several (a join). Each copy is a new entity with the
//! original's creation time and properties. Each exclusive gateway draws from its own stream.
//!
//! Entities carry BPSim properties. Each element's [`stochcraft_model::Property`] assignments
//! are made as an entity enters it (at a source, as it is created), all together, from the
//! values the entity arrived with. Each element reports the mean of the values it assigned to
//! each property (numbers as they are, booleans as 0 or 1). Values drawn for properties come
//! from streams of their own, so adding a property never changes the other random numbers.
//!
//! [`Builder`] writes the same models from code, as in the tests and examples:
//!
//! ```
//! use stochcraft_blocks::Builder;
//! use stochcraft_random::{Dist, Rng};
//!
//! let mut m = Builder::new();
//! let arrivals = m.source("arrivals", Dist::exponential_rate(0.75), None);
//! let machine = m.server("machine", 1, Dist::exponential_rate(1.0));
//! let exit = m.sink("exit");
//! m.connect(arrivals, machine);
//! m.connect(machine, exit);
//! let report = m.run(Rng::new(1), 1_000.0, 10_000.0).expect("valid model");
//! assert!(report.element("machine").and_then(|e| e.utilisation).is_some());
//! ```

mod runtime;

pub use runtime::{Live, LiveState};

use std::collections::BTreeMap;

pub use stochcraft_model::calendar::{Calendar, Event, Rule};
use stochcraft_model::expr;
pub use stochcraft_model::{Breakdown, Change, Power, Routing, Setup, Transfer};
use stochcraft_model::{
    Connection, Diagnostic, Element, ElementId, ElementKind, Experiment, Model, Need, Problem,
    Property, PropertyType, PropertyValue, ResourcePool, Severity, Timed,
};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::Summary;

/// A property value an entity holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Held {
    Long(i64),
    Double(f64),
    Boolean(bool),
    Text(String),
}

impl Held {
    /// As an expression value.
    pub fn value(&self) -> expr::Value {
        match self {
            Held::Long(n) => expr::Value::Number(*n as f64),
            Held::Double(n) => expr::Value::Number(*n),
            Held::Boolean(b) => expr::Value::Boolean(*b),
            Held::Text(s) => expr::Value::Text(s.clone()),
        }
    }

    /// `v` as a property of type `ty`: a long is rounded to the nearest whole number (halves
    /// away from zero), and is not set at all when `v` is not a finite number.
    pub fn of(v: &expr::Value, ty: PropertyType) -> Option<Held> {
        Some(match ty {
            PropertyType::Long => {
                let n = v.number();
                if !n.is_finite() {
                    return None;
                }
                // `as` saturates at the ends of i64's range.
                Held::Long(n.round() as i64)
            }
            PropertyType::Double | PropertyType::Duration => Held::Double(v.number()),
            PropertyType::Boolean => Held::Boolean(v.boolean()),
            PropertyType::String | PropertyType::DateTime => Held::Text(v.text()),
        })
    }

    /// The number recorded in an element's statistics: numbers as they are, booleans as 0 or 1.
    fn statistic(&self) -> Option<f64> {
        match self {
            Held::Long(n) => Some(*n as f64),
            Held::Double(n) if n.is_finite() => Some(*n),
            Held::Boolean(b) => Some(f64::from(u8::from(*b))),
            Held::Double(_) | Held::Text(_) => None,
        }
    }
}

/// Something that flows through the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// Unique within a run, in creation order.
    pub id: u64,
    /// When its source created it.
    pub created: f64,
    /// When it entered the element it is in now.
    pub entered: f64,
    /// Its properties, by the slot [`Compiled::property_names`] gives each name (`None`: not
    /// set). Empty when the model has no properties.
    pub properties: Vec<Option<Held>>,
    /// A batch ([`ElementKind::Batch`]): the other entities it carries, until an
    /// [`ElementKind::Unbatch`] sends them on again. Empty for any other entity.
    pub members: Vec<Entity>,
}

/// Where an assigned value comes from.
#[derive(Debug, Clone)]
enum Source {
    Fixed(Option<Held>),
    Random(Dist),
    /// A mix: a value's index drawn from the shares, and the values.
    Choice(Dist, Vec<String>),
    Expr(expr::Expr),
}

/// One property assignment, ready to run.
#[derive(Debug, Clone)]
struct Assign {
    name: String,
    slot: usize,
    ty: PropertyType,
    source: Source,
}

/// One of the connections an element sends entities down.
#[derive(Debug, Clone)]
struct Route {
    /// The element it leads to.
    to: usize,
    /// Which of that element's inputs it arrives on: a join has one per incoming connection, in
    /// model order; every other element has one.
    input: usize,
    /// Out of an exclusive gateway routing by probability: this connection's share.
    weight: f64,
    /// Out of an exclusive gateway routing by condition: what must hold to take it.
    condition: Option<expr::Expr>,
    /// The gateway's default connection.
    default: bool,
}

/// A model checked and ready to run.
#[derive(Debug, Clone)]
pub struct Compiled {
    model: Model,
    /// For each element, the connections it sends entities down, in model order.
    routes: Vec<Vec<Route>>,
    /// For each element, how many inputs it has (more than one only for a join).
    inputs: Vec<usize>,
    /// For each element, how many entities each of its inputs gives a parallel gateway's
    /// output (its bill of materials; 1 each unless a connection says otherwise).
    quantities: Vec<Vec<u32>>,
    warnings: Vec<Diagnostic>,
    /// Property names, sorted: an entity's slots.
    slots: Vec<String>,
    /// For each element, its property assignments.
    assigns: Vec<Vec<Assign>>,
    /// Sources create their first entity at time 0 rather than after the first interval.
    first_arrival_at_zero: bool,
    /// See [`Compiled::max_events_at_one_time`].
    max_events_at_one_time: u64,
}

/// The most events a replication may process at one simulated time before it stops with
/// [`RunError::TimeStopped`] (see [`Compiled::max_events_at_one_time`]).
///
/// Many events can legitimately happen at one time: a source whose time between arrivals is 0
/// may create up to its limit of entities at once (allowed since #36), and each entity costs a
/// few events at each element it passes. Ten million leaves room for a batch of about a million
/// entities through several elements at one instant, far beyond what a model needs, while a
/// model looping without time moving reaches it within seconds (the error path is the only one
/// that pays) instead of never ending.
pub const MAX_EVENTS_AT_ONE_TIME: u64 = 10_000_000;

/// Why a replication stopped without a [`Report`].
#[derive(Debug, Clone, PartialEq)]
pub enum RunError {
    /// It was cancelled ([`Compiled::run_watched`]).
    Cancelled,
    /// Simulated time stopped moving: more than `events` events happened at time `time`, the
    /// last of them in `element` (by id and name), which is part of a loop whose steps all take
    /// no time.
    TimeStopped {
        time: f64,
        events: u64,
        element: Option<(ElementId, String)>,
    },
    /// At time `time`, `element` (by id and name) drew a time to wait so long that it would end
    /// later than the largest time that can be represented.
    TimeOutOfRange {
        time: f64,
        element: Option<(ElementId, String)>,
    },
}

/// Writes `element` as `"name" (id)`, or just the id when the name adds nothing.
fn write_element(
    f: &mut std::fmt::Formatter<'_>,
    element: Option<&(ElementId, String)>,
) -> std::fmt::Result {
    match element {
        Some((id, name)) if !name.is_empty() && name != id.as_str() => {
            write!(f, "\"{name}\" ({id})")
        }
        Some((id, _)) => write!(f, "{id}"),
        None => f.write_str("an element"),
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Cancelled => f.write_str("the run was cancelled"),
            RunError::TimeStopped {
                time,
                events,
                element,
            } => {
                write!(
                    f,
                    "simulated time stopped moving at {time}: more than {events} events \
                     happened at that time"
                )?;
                if element.is_some() {
                    f.write_str(", the last at ")?;
                    write_element(f, element.as_ref())?;
                }
                f.write_str(
                    ". Entities seem to go round a loop whose steps all take no time; give a step \
                     in it a duration, or the loop a way out",
                )
            }
            RunError::TimeOutOfRange { time, element } => {
                write!(f, "at {time}, ")?;
                write_element(f, element.as_ref())?;
                f.write_str(
                    " drew a time so long that it would end later than the largest time that \
                     can be represented; check its durations",
                )
            }
        }
    }
}

impl std::error::Error for RunError {}

/// Why [`Builder::run`] gave no report.
#[derive(Debug, Clone, PartialEq)]
pub enum Failed {
    /// The model has errors: every diagnostic, as [`compile`] returns them.
    Invalid(Vec<Diagnostic>),
    /// The replication stopped.
    Run(RunError),
}

fn compile_assign(p: &Property, slots: &[String], ty: PropertyType) -> Option<Assign> {
    let source = match &p.value {
        PropertyValue::Number(n) => Source::Fixed(Held::of(&expr::Value::Number(*n), ty)),
        PropertyValue::Boolean(b) => Source::Fixed(Held::of(&expr::Value::Boolean(*b), ty)),
        PropertyValue::Text(s) => Source::Fixed(Held::of(&expr::Value::Text(s.clone()), ty)),
        PropertyValue::Random(d) => Source::Random(d.clone()),
        // The value's index drawn from a discrete distribution of the shares, so a mix draws as a
        // discrete user distribution does.
        PropertyValue::Choice(values) => Source::Choice(
            Dist::User {
                points: values
                    .iter()
                    .enumerate()
                    .map(|(i, (share, _))| (*share, i as f64))
                    .collect(),
                discrete: true,
            },
            values.iter().map(|(_, v)| v.clone()).collect(),
        ),
        PropertyValue::Expression(src) => Source::Expr(expr::parse(src).ok()?),
    };
    Some(Assign {
        name: p.name.clone(),
        slot: slots.binary_search(&p.name).ok()?,
        ty,
        source,
    })
}

/// Checks `model` and prepares it to run. On any error, returns every diagnostic (errors and
/// warnings) so all of them can be fixed at once.
pub fn compile(model: &Model) -> Result<Compiled, Vec<Diagnostic>> {
    let diagnostics = model.validate();
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(diagnostics);
    }
    // Validation guarantees every non-sink element has an output, only gateways more than one,
    // and every connection joins known elements.
    let index = |id: &ElementId| model.elements.iter().position(|e| &e.id == id);
    let incoming = |e: &Element| -> Vec<&Connection> {
        model.connections.iter().filter(|c| c.to == e.id).collect()
    };
    let is_join =
        |e: &Element| matches!(e.kind, ElementKind::ParallelGateway) && incoming(e).len() > 1;
    let inputs = model
        .elements
        .iter()
        .map(|e| if is_join(e) { incoming(e).len() } else { 1 })
        .collect();
    let quantities = model
        .elements
        .iter()
        .map(|e| {
            let ins = incoming(e);
            if !matches!(e.kind, ElementKind::ParallelGateway) || ins.is_empty() {
                return vec![1];
            }
            if is_join(e) {
                ins.iter().map(|c| c.quantity.unwrap_or(1)).collect()
            } else {
                // One input: every connection into it shares it.
                vec![ins.first().and_then(|c| c.quantity).unwrap_or(1)]
            }
        })
        .collect();
    let routes = model
        .elements
        .iter()
        .map(|e| {
            let outs: Vec<&Connection> = model
                .connections
                .iter()
                .filter(|c| c.from == e.id)
                .collect();
            let default = match &e.kind {
                ElementKind::ExclusiveGateway { default, .. } => default.as_ref(),
                _ => None,
            };
            let by_condition = outs.iter().any(|c| c.condition.is_some());
            // Probabilities: those not given share evenly what the given ones leave; if
            // nothing is left to share, every connection counts the same.
            let given: f64 = outs.iter().filter_map(|c| c.probability).sum();
            let unset = outs.iter().filter(|c| c.probability.is_none()).count();
            let share = if unset > 0 {
                (1.0 - given).max(0.0) / unset as f64
            } else {
                0.0
            };
            let mut weights: Vec<f64> = outs
                .iter()
                .map(|c| c.probability.unwrap_or(share))
                .collect();
            if weights.iter().sum::<f64>() <= 0.0 {
                weights.iter_mut().for_each(|w| *w = 1.0);
            }
            outs.iter()
                .zip(weights)
                .filter_map(|(c, weight)| {
                    let to = index(&c.to)?;
                    let target = model.elements.get(to)?;
                    let input = if is_join(target) {
                        incoming(target).iter().position(|x| x.id == c.id)?
                    } else {
                        0
                    };
                    Some(Route {
                        to,
                        input,
                        weight,
                        condition: if by_condition {
                            c.condition.as_deref().and_then(|s| expr::parse(s).ok())
                        } else {
                            None
                        },
                        default: default == Some(&c.id),
                    })
                })
                .collect()
        })
        .collect();
    let types = model.property_types();
    let slots: Vec<String> = types.keys().cloned().collect();
    // Validation guarantees every expression parses and every property has a type.
    let assigns = model
        .elements
        .iter()
        .map(|e| {
            e.properties
                .iter()
                .filter_map(|p| compile_assign(p, &slots, *types.get(&p.name)?))
                .collect()
        })
        .collect();
    Ok(Compiled {
        model: model.clone(),
        routes,
        inputs,
        quantities,
        warnings: diagnostics,
        slots,
        assigns,
        first_arrival_at_zero: false,
        max_events_at_one_time: MAX_EVENTS_AT_ONE_TIME,
    })
}

impl Compiled {
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Makes sources create their first entity at time 0 instead of after the first interval
    /// (decision D8 in ROADMAP.md is open; the default is after the first interval). For
    /// cross-checks against simulators that start at time 0.
    #[doc(hidden)]
    pub fn first_arrival_at_zero(mut self, yes: bool) -> Self {
        self.first_arrival_at_zero = yes;
        self
    }

    /// Sets the most events a replication may process at one simulated time before it stops
    /// with [`RunError::TimeStopped`] ([`MAX_EVENTS_AT_ONE_TIME`] unless set; at least 1). For
    /// tests that need the guard to act quickly.
    pub fn max_events_at_one_time(mut self, events: u64) -> Self {
        self.max_events_at_one_time = events.max(1);
        self
    }

    /// The names of the properties entities can carry, in slot order.
    pub fn property_names(&self) -> &[String] {
        &self.slots
    }

    /// Warnings found when compiling (the model runs, but perhaps not as intended).
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }
}

/// A statistic an element reports.
///
/// Counts balance on every element: what entered either left, was lost there ([`Metric::Lost`],
/// where reported), or was still inside at the end; except where entities are combined or split:
/// a batch of k reports k entered for each one left, an assembly counts every part it takes, and
/// a split sends on each entity of a batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Metric {
    /// Entities that entered (for a source: created). A join counts each entity once, as it
    /// arrives on the join's first input; what arrives on its other inputs are copies of it,
    /// merged into it.
    Entered,
    /// Entities that left, sent on to the next element (for a sink: removed). A parallel
    /// gateway counts one per copy it sends out, so a split into two reports twice what entered.
    /// Entities lost do not leave.
    Left,
    /// Servers: time-average number waiting for a unit.
    MeanQueueLength,
    /// Servers: mean time from arrival to start of service (a unit taking the entity; a setup
    /// is part of its service, as BPSim counts it in the activity's duration), over the
    /// entities that started service. Entities still waiting at the end are left out, so for an overloaded server the
    /// mean is biased low.
    MeanWait,
    /// Servers: time-average fraction of units busy.
    Utilisation,
    /// Servers and delays: time-average number of entities being served or delayed.
    MeanContents,
    /// Servers, shared tasks, delays, batches and joins: entities still inside at the end of
    /// the replication (waiting, being served or delayed, held while blocked, collected into a
    /// batch, or waiting at a join for their partners). Work that arrived and was never
    /// finished: large, or growing with the run length, when more work arrives than the
    /// element can do (see [`run_warnings`]).
    WorkInProgress,
    /// Servers, shared tasks, delays, batches and joins: time-average number of entities inside
    /// over the measured window, each counted once wherever it is in the element: on its way in
    /// (in transfer), waiting, being served or delayed (or collected into a batch, or waiting at
    /// a join), or held while blocked after service. An entity in transfer to the next element
    /// is that element's. Summed over the elements of a line, the line's time-average work in
    /// progress.
    MeanWorkInProgress,
    /// Sinks: mean time from creation to arrival, over the entities that arrived. Entities still
    /// inside the model at the end (see [`Metric::WorkInProgress`]) are left out, so when work
    /// piles up the mean is biased low.
    MeanTimeInSystem,
    /// Servers and delays sending to a finite buffer: mean time an entity that left waited,
    /// after its service or delay, for room in the next buffer (zero when there was room).
    MeanBlockedTime,
    /// Servers sending to a finite buffer: time-average fraction of units blocked (holding a
    /// served entity while the next buffer is full).
    BlockingProbability,
    /// Entities lost. Servers with a finite buffer: arrivals from sources turned away because
    /// the buffer was full (these never entered). Sources sending to a finite buffer: their
    /// arrivals lost that way (these entered, but never left), so one lost arrival counts at
    /// both. Exclusive gateways: entities for which no condition held and there was no default
    /// connection, which leave the model there.
    Lost,
    /// Servers with a finite buffer: the fraction of arrivals lost (lost over lost plus
    /// entered).
    LossProbability,
    /// Servers whose units fail: time-average fraction of units up (not failed or under
    /// repair).
    Availability,
    /// Servers whose units follow a calendar, or that have a power: time-average fraction of
    /// units off shift. A unit is in exactly one state at a time, the first that applies of
    /// failed (or under repair), processing, blocked, off shift and starved: off shift means
    /// only idle because the calendar has the units unavailable. A job that runs on into a
    /// break, or resumes off shift after a repair, is processing; a finished entity held
    /// during a break is blocked.
    TimeOffShift,
    /// Servers: time-average fraction of units processing (serving an entity, in a
    /// break too, and not failed).
    TimeProcessing,
    /// Servers: time-average fraction of units starved (idle, on shift and up).
    TimeStarved,
    /// Servers that send to a finite buffer or across a transfer (all with a power):
    /// time-average fraction of units blocked (holding a finished entity
    /// while the next buffer is full, or through a transfer to the next element, in a break too;
    /// not failed). Unlike [`Metric::BlockingProbability`], a failed unit holding an entity is
    /// not counted as blocked.
    TimeBlocked,
    /// Servers with breakdowns (all with a power): time-average fraction of units failed or under repair; the
    /// complement of [`Metric::Availability`].
    TimeFailed,
    /// Servers that set up (a setup before every job, or changeovers): time-average fraction of
    /// units setting up. Their utilisation includes it (a unit setting up holds its part).
    TimeSetup,
    /// Servers with a power: energy used in the measured window, in kilowatt-hours: each
    /// state's power times the exact time the units spent in it, cut at the end of the warm-up
    /// and at the end of the run.
    Energy,
    /// Sinks, when any server has a power: the energy of every server with a power over the
    /// measured window, divided by the entities that reached this sink in it, in kilowatt-hours
    /// each. A replication in which none reached it has no value (never 0), so the summary is
    /// the mean of the ratios of the replications that have one, and its `n` says how many.
    SpecificEnergy,
}

impl Metric {
    pub const ALL: [Metric; 22] = [
        Metric::Entered,
        Metric::Left,
        Metric::MeanQueueLength,
        Metric::MeanWait,
        Metric::Utilisation,
        Metric::MeanContents,
        Metric::WorkInProgress,
        Metric::MeanWorkInProgress,
        Metric::MeanTimeInSystem,
        Metric::MeanBlockedTime,
        Metric::BlockingProbability,
        Metric::Lost,
        Metric::LossProbability,
        Metric::Availability,
        Metric::TimeOffShift,
        Metric::TimeProcessing,
        Metric::TimeStarved,
        Metric::TimeBlocked,
        Metric::TimeFailed,
        Metric::TimeSetup,
        Metric::Energy,
        Metric::SpecificEnergy,
    ];

    /// The metric with this [`Metric::name`].
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }

    /// A stable name for tables, files and agents.
    pub fn name(self) -> &'static str {
        match self {
            Metric::Entered => "entered",
            Metric::Left => "left",
            Metric::MeanQueueLength => "mean_queue_length",
            Metric::MeanWait => "mean_wait",
            Metric::Utilisation => "utilisation",
            Metric::MeanContents => "mean_contents",
            Metric::WorkInProgress => "work_in_progress",
            Metric::MeanWorkInProgress => "mean_work_in_progress",
            Metric::MeanTimeInSystem => "mean_time_in_system",
            Metric::MeanBlockedTime => "mean_blocked_time",
            Metric::BlockingProbability => "blocking_probability",
            Metric::Lost => "lost",
            Metric::LossProbability => "loss_probability",
            Metric::Availability => "availability",
            Metric::TimeOffShift => "time_off_shift",
            Metric::TimeProcessing => "time_processing",
            Metric::TimeStarved => "time_starved",
            Metric::TimeBlocked => "time_blocked",
            Metric::TimeFailed => "time_failed",
            Metric::TimeSetup => "time_setup",
            Metric::Energy => "energy",
            Metric::SpecificEnergy => "specific_energy",
        }
    }
}

/// Statistics for one element over the reporting period of one replication.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementReport {
    pub id: ElementId,
    pub name: String,
    /// "source", "server", "delay" or "sink".
    pub kind: &'static str,
    pub entered: u64,
    pub left: u64,
    pub mean_queue_length: Option<f64>,
    pub mean_wait: Option<f64>,
    pub utilisation: Option<f64>,
    pub mean_contents: Option<f64>,
    /// [`Metric::WorkInProgress`]: entities inside at the end.
    pub work_in_progress: Option<u64>,
    /// [`Metric::MeanWorkInProgress`]: the time-average number inside.
    pub mean_work_in_progress: Option<f64>,
    pub mean_time_in_system: Option<f64>,
    /// Only where a finite buffer is involved (`None` otherwise, so models without one report
    /// as before): see [`Metric::MeanBlockedTime`] and the metrics after it. `lost` is also
    /// reported by every exclusive gateway, buffer or not.
    pub mean_blocked_time: Option<f64>,
    pub blocking_probability: Option<f64>,
    /// Servers whose units fail: [`Metric::Availability`].
    pub availability: Option<f64>,
    /// Servers whose units follow a calendar: [`Metric::TimeOffShift`].
    pub time_off_shift: Option<f64>,
    /// Servers: [`Metric::TimeProcessing`] and the other states that can happen to them;
    /// servers with a power: all of them, and [`Metric::Energy`].
    pub time_processing: Option<f64>,
    pub time_starved: Option<f64>,
    pub time_blocked: Option<f64>,
    pub time_failed: Option<f64>,
    /// Servers that set up: [`Metric::TimeSetup`].
    pub time_setup: Option<f64>,
    pub energy: Option<f64>,
    /// Sinks, when any server has a power: [`Metric::SpecificEnergy`].
    pub specific_energy: Option<f64>,
    pub lost: Option<u64>,
    pub loss_probability: Option<f64>,
    /// For each property the element assigns, in the element's order: the mean of the values
    /// it assigned (`None` when it assigned none, or the property holds text).
    pub properties: Vec<(String, Option<f64>)>,
}

impl ElementReport {
    /// The value of `metric`, if this element reports it.
    pub fn metric(&self, metric: Metric) -> Option<f64> {
        match metric {
            Metric::Entered => Some(self.entered as f64),
            Metric::Left => Some(self.left as f64),
            Metric::MeanQueueLength => self.mean_queue_length,
            Metric::MeanWait => self.mean_wait,
            Metric::Utilisation => self.utilisation,
            Metric::MeanContents => self.mean_contents,
            Metric::WorkInProgress => self.work_in_progress.map(|n| n as f64),
            Metric::MeanWorkInProgress => self.mean_work_in_progress,
            Metric::MeanTimeInSystem => self.mean_time_in_system,
            Metric::MeanBlockedTime => self.mean_blocked_time,
            Metric::BlockingProbability => self.blocking_probability,
            Metric::Lost => self.lost.map(|n| n as f64),
            Metric::LossProbability => self.loss_probability,
            Metric::Availability => self.availability,
            Metric::TimeOffShift => self.time_off_shift,
            Metric::TimeProcessing => self.time_processing,
            Metric::TimeStarved => self.time_starved,
            Metric::TimeBlocked => self.time_blocked,
            Metric::TimeFailed => self.time_failed,
            Metric::TimeSetup => self.time_setup,
            Metric::Energy => self.energy,
            Metric::SpecificEnergy => self.specific_energy,
        }
    }
}

/// The results of one replication.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub warmup: f64,
    /// The time statistics were collected over: from the warm-up to the time limit, or to the
    /// stop (0 if the run stopped during the warm-up, when the statistics are from time 0).
    pub duration: f64,
    /// In model order.
    pub elements: Vec<ElementReport>,
    /// When the model's stop condition was met, ending the run; `None` if it ran to its time
    /// limit (or has none). A time to an event, from time 0.
    pub stopped_at: Option<f64>,
    /// The span the element statistics actually cover: from the warm-up's end (0 with no
    /// warm-up, or when the run stopped before the warm-up ended) to the run's end.
    pub measured_from: f64,
    pub measured_to: f64,
    /// Whether the statistics come from after the warm-up: true unless the run stopped before
    /// the warm-up ended, when they come from the warm-up alone and [`summarise`] leaves them
    /// out.
    pub after_warmup: bool,
}

impl Report {
    /// The element with this id.
    pub fn element(&self, id: &str) -> Option<&ElementReport> {
        self.elements.iter().find(|e| e.id.as_str() == id)
    }
}

/// One element's results across replications.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementSummary {
    pub id: ElementId,
    pub name: String,
    pub kind: &'static str,
    /// Each metric the element reports, in [`Metric::ALL`] order.
    pub metrics: Vec<(Metric, Summary)>,
    /// The mean value assigned to each property, as in [`ElementReport::properties`].
    pub properties: Vec<(String, Summary)>,
    /// Each metric's value in every replication, in replication order (`None` where a
    /// replication had none), in the order of `metrics`: the distribution behind the summary.
    pub samples: Vec<(Metric, Vec<Option<f64>>)>,
}

impl ElementSummary {
    /// `metric`'s value in each replication, in replication order.
    pub fn samples(&self, metric: Metric) -> Option<&[Option<f64>]> {
        self.samples
            .iter()
            .find(|(m, _)| *m == metric)
            .map(|(_, v)| v.as_slice())
    }

    pub fn metric(&self, metric: Metric) -> Option<&Summary> {
        self.metrics
            .iter()
            .find(|(m, _)| *m == metric)
            .map(|(_, s)| s)
    }
}

/// The results of an experiment: a confidence interval per element and metric.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperimentSummary {
    pub replications: u64,
    pub elements: Vec<ElementSummary>,
    /// How many replications met the model's stop condition before their time limit, and the
    /// time they met it (over those only, so biased low when some did not: the distribution of
    /// [`Report::stopped_at`], with those censored at the limit, is the full picture). `None`
    /// when none met it, or there is none.
    pub stopped: Option<(u64, Summary)>,
    /// When each replication met the stop condition, in replication order (`None`: it did
    /// not, or there is none).
    pub stop_times: Vec<Option<f64>>,
    /// What each replication measured, in replication order.
    pub measured: Vec<Measured>,
}

/// The span one replication's statistics cover, and whether they are used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measured {
    pub from: f64,
    pub to: f64,
    /// False when the replication stopped before its warm-up ended: its statistics are all
    /// from the warm-up, so [`summarise`] leaves them out (its stop time still counts).
    pub used: bool,
}

impl ExperimentSummary {
    /// The mean time measured over the replications whose statistics are used; `None` when
    /// none is.
    pub fn mean_measured(&self) -> Option<f64> {
        let used: Vec<f64> = self
            .measured
            .iter()
            .filter(|m| m.used)
            .map(|m| m.to - m.from)
            .collect();
        (!used.is_empty()).then(|| used.iter().sum::<f64>() / used.len() as f64)
    }
}

impl ExperimentSummary {
    pub fn get(&self, id: &str, metric: Metric) -> Option<&Summary> {
        self.elements
            .iter()
            .find(|e| e.id.as_str() == id)?
            .metric(metric)
    }
}

/// Summarises replication reports of the same model (as [`Compiled::run`] returns them, in any
/// order). A metric is left out of an element when no replication reported a value for it, for
/// example the mean wait of a server that no entity reached. Replications without a value are
/// left out of that metric's summary, so its `n` can be less than the number of replications,
/// and differ from metric to metric.
///
/// A replication that stopped before its warm-up ended ([`Report::after_warmup`] false) has
/// statistics from the warm-up alone, which describe the start-up rather than the system: they
/// are left out of every metric and property (as `None` in the samples), and only its stop time
/// counts. [`run_warnings`] says so.
pub fn summarise(reports: &[Report]) -> ExperimentSummary {
    let elements = reports.first().map_or_else(Vec::new, |first| {
        first
            .elements
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let mut samples = Vec::new();
                let metrics = Metric::ALL
                    .iter()
                    .filter_map(|&m| {
                        let each: Vec<Option<f64>> = reports
                            .iter()
                            .map(|r| r.elements.get(i).filter(|_| r.after_warmup)?.metric(m))
                            .collect();
                        let values: Vec<f64> = each.iter().flatten().copied().collect();
                        let s = Summary::of(&values)?;
                        samples.push((m, each));
                        Some((m, s))
                    })
                    .collect();
                let properties = e
                    .properties
                    .iter()
                    .enumerate()
                    .filter_map(|(j, (name, _))| {
                        let values: Vec<f64> = reports
                            .iter()
                            .filter(|r| r.after_warmup)
                            .filter_map(|r| r.elements.get(i)?.properties.get(j)?.1)
                            .collect();
                        Summary::of(&values).map(|s| (name.clone(), s))
                    })
                    .collect();
                ElementSummary {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    kind: e.kind,
                    metrics,
                    properties,
                    samples,
                }
            })
            .collect()
    });
    let times: Vec<f64> = reports.iter().filter_map(|r| r.stopped_at).collect();
    ExperimentSummary {
        replications: reports.len() as u64,
        elements,
        stopped: Summary::of(&times).map(|s| (times.len() as u64, s)),
        stop_times: reports.iter().map(|r| r.stopped_at).collect(),
        measured: reports
            .iter()
            .map(|r| Measured {
                from: r.measured_from,
                to: r.measured_to,
                used: r.after_warmup,
            })
            .collect(),
    }
}

/// A server busy at least this share of the time its units were up is saturated: with a queue
/// that does not empty, it is working flat out ([`run_warnings`]).
///
/// Chosen with the backlog thresholds below so that stable queues stay quiet. Measured
/// (4 to 10 replications each): the committed models' servers are busy at most 0.75 (mm1.bpmn;
/// the others 0.11 to 0.21; the BPSim reference models have no servers), and stable M/M/1
/// queues at 0.75 and 0.9 over 20,000 time units stay below it (one at 0.98 reaches it, and the
/// backlog test keeps it quiet); an M/M/1 with 12.5% or 50% more work than capacity is busy
/// more than 0.999.
pub const SATURATED_UTILISATION: f64 = 0.98;

/// The fewest entities that must still be waiting at a saturated server at the end, on
/// average, for [`run_warnings`] to call its queue growing: a handful waiting says nothing.
pub const BACKLOG_MIN: f64 = 10.0;

/// The backlog, as a share of the entities that left the server during the run, from which
/// [`run_warnings`] calls its queue growing. A stable queue holds what arrives in about one
/// mean time through it (Little's law), a small share of a run many times longer: a share of
/// 0.05 means entities wait a twentieth of the whole run, which a stable queue in a run long
/// enough to measure it does not do. Measured: the committed models end with at most 0.03% of
/// their departures still waiting (mm1.bpmn: 3.7 of 14,970); an M/M/1 with 12.5% more work
/// than capacity, over 3,000 time units after a warm-up of 1,000, ends with about 19%. A queue
/// at exactly its capacity (load 1, about 3% here) is not called growing: it has no steady
/// state either, but its backlog grows only as the square root of the run length.
pub const BACKLOG_SHARE: f64 = 0.05;

/// Warnings about a run's results (`model`'s, summarised in `summary`) that only the run can
/// show, in the same form as [`Model::validate`]'s:
///
/// - Replications that stopped before the warm-up ended
///   ([`stochcraft_model::Problem::EndedInWarmup`]).
/// - A server with no steady state ([`stochcraft_model::Problem::NoSteadyState`]): busy at least
///   [`SATURATED_UTILISATION`] of the time its units were up, with at least [`BACKLOG_MIN`]
///   entities, and at least [`BACKLOG_SHARE`] of those that left, still waiting at the end
///   (means across replications). The backlog is what was inside beyond the server's units,
///   so it never counts entities in service. Only servers with an unlimited queue are judged: a
///   finite buffer loses arrivals instead of growing, and tasks with shared resources report no
///   utilisation of their own.
pub fn run_warnings(model: &Model, summary: &ExperimentSummary) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let unused = summary.measured.iter().filter(|m| !m.used).count() as u64;
    if unused > 0 {
        out.push(Diagnostic {
            severity: Severity::Warning,
            at: None,
            problem: Problem::EndedInWarmup {
                replications: unused,
                of: summary.measured.len() as u64,
            },
        });
    }
    for e in &summary.elements {
        let Some(ElementKind::Server {
            capacity,
            queue_capacity: None,
            ..
        }) = model.element(&e.id).map(|x| &x.kind)
        else {
            continue;
        };
        let mean = |m: Metric| e.metric(m).map(|s| s.mean);
        let (Some(busy), Some(inside), Some(left)) = (
            mean(Metric::Utilisation),
            mean(Metric::WorkInProgress),
            mean(Metric::Left),
        ) else {
            continue;
        };
        // Busy as a share of the time up and on shift, for servers whose units fail or follow
        // a calendar.
        let up =
            mean(Metric::Availability).unwrap_or(1.0) - mean(Metric::TimeOffShift).unwrap_or(0.0);
        let utilisation = if up > 0.0 && up < 1.0 {
            busy / up
        } else {
            busy
        };
        let backlog = inside - f64::from(*capacity);
        if utilisation >= SATURATED_UTILISATION
            && backlog >= BACKLOG_MIN
            && backlog >= BACKLOG_SHARE * left
        {
            out.push(Diagnostic {
                severity: Severity::Warning,
                at: Some(e.id.clone()),
                problem: Problem::NoSteadyState {
                    backlog,
                    left,
                    utilisation,
                },
            });
        }
    }
    out
}

/// An element added to a [`Builder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(usize);

/// Writes a [`Model`] from code. Each element's name is also its id, so names must be valid ids
/// (a letter or `_` first, then letters, digits, `.`, `-` or `_`) and unique; problems are
/// reported when the model is built or run.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    elements: Vec<(String, ElementKind, Vec<Property>)>,
    /// `from` → `to`, with a probability and a condition; a later connection from the same
    /// element replaces the earlier one unless `from` is a gateway.
    connections: Vec<(BlockId, BlockId, Option<f64>, Option<String>)>,
    /// Bills of materials: how many entities a connection gives a parallel gateway.
    quantities: BTreeMap<(BlockId, BlockId), u32>,
    /// Shared resource pools.
    pools: Vec<ResourcePool>,
    calendars: Vec<Calendar>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, name: &str, kind: ElementKind) -> BlockId {
        self.elements.push((name.to_owned(), kind, Vec::new()));
        BlockId(self.elements.len() - 1)
    }

    /// Makes `block` set property `name` on every entity that enters it (or that it creates).
    pub fn property(
        &mut self,
        block: BlockId,
        name: &str,
        ty: Option<PropertyType>,
        value: PropertyValue,
    ) {
        if let Some((_, _, props)) = self.elements.get_mut(block.0) {
            props.push(Property {
                name: name.to_owned(),
                ty,
                value,
            });
        }
    }

    /// Creates entities with the given time between arrivals, up to `limit` if one is given.
    pub fn source(&mut self, name: &str, interarrival: Dist, limit: Option<u64>) -> BlockId {
        self.add(
            name,
            ElementKind::Source {
                interarrival,
                limit,
                idle_outside: false,
                timed: Vec::new(),
            },
        )
    }

    /// `capacity` identical units that each serve one entity at a time, first come first served,
    /// for a time drawn from `service`.
    pub fn server(&mut self, name: &str, capacity: u32, service: Dist) -> BlockId {
        self.add(
            name,
            ElementKind::Server {
                capacity,
                service,
                queue_capacity: None,
                breakdown: None,
                transfer: None,
                available: true,
                availability: Vec::new(),
                power: None,
                setup: None,
            },
        )
    }

    /// As [`Builder::server`], with a finite buffer: at most `queue_capacity` entities wait for
    /// a unit. When it is full, a server or delay sending here waits holding its entity
    /// (blocking after service) and a source loses its arrival.
    pub fn buffered_server(
        &mut self,
        name: &str,
        capacity: u32,
        service: Dist,
        queue_capacity: u32,
    ) -> BlockId {
        self.add(
            name,
            ElementKind::Server {
                capacity,
                service,
                queue_capacity: Some(queue_capacity),
                breakdown: None,
                transfer: None,
                available: true,
                availability: Vec::new(),
                power: None,
                setup: None,
            },
        )
    }

    /// A pool of `quantity` identical units, playing `roles`, that shared tasks draw on. Its
    /// name is its id.
    pub fn pool(&mut self, name: &str, quantity: u32, roles: &[&str]) {
        self.pools.push(ResourcePool::new(
            ElementId::new(name).unwrap_or_else(|| ElementId::unchecked(name)),
            name,
            quantity,
            roles.iter().map(|r| (*r).to_owned()).collect(),
        ));
    }

    /// Makes server `block`'s units fail and be repaired.
    pub fn breakdown(&mut self, block: BlockId, breakdown: Breakdown) {
        if let Some((_, ElementKind::Server { breakdown: b, .. }, _)) =
            self.elements.get_mut(block.0)
        {
            *b = Some(breakdown);
        }
    }

    /// Gives server `block` a transfer time: each part takes a time drawn from `transfer` to
    /// get there, holding a sending server's unit meanwhile.
    pub fn transfer(&mut self, block: BlockId, transfer: Dist) {
        self.set_transfer(block, Transfer::holding(transfer));
    }

    /// Gives server `block` a minimum time in its buffer: each part takes a time drawn from
    /// `time` to get there, and a sending server's unit is free as soon as the part is in.
    pub fn min_stay(&mut self, block: BlockId, time: Dist) {
        self.set_transfer(
            block,
            Transfer {
                time,
                holds_sender: false,
            },
        );
    }

    fn set_transfer(&mut self, block: BlockId, transfer: Transfer) {
        if let Some((_, ElementKind::Server { transfer: t, .. }, _)) =
            self.elements.get_mut(block.0)
        {
            *t = Some(transfer);
        }
    }

    /// Makes server `block`'s units follow calendars: on shift while each calendar is active
    /// as `availability` says (the first active one applying), and as `available` says outside
    /// them.
    pub fn availability(&mut self, block: BlockId, available: bool, availability: &[(&str, bool)]) {
        if let Some((
            _,
            ElementKind::Server {
                available: a,
                availability: t,
                ..
            },
            _,
        )) = self.elements.get_mut(block.0)
        {
            *a = available;
            *t = availability
                .iter()
                .map(|(c, v)| Timed {
                    calendar: ElementId::unchecked(c),
                    value: *v,
                })
                .collect();
        }
    }

    /// Gives server `block` the power its units draw in each state, for energy results.
    pub fn power(&mut self, block: BlockId, power: Power) {
        if let Some((_, ElementKind::Server { power: p, .. }, _)) = self.elements.get_mut(block.0) {
            *p = Some(power);
        }
    }

    /// Gives server `block` a setup: before every job, or a changeover ([`Setup`]).
    pub fn setup(&mut self, block: BlockId, setup: Setup) {
        if let Some((_, ElementKind::Server { setup: s, .. }, _)) = self.elements.get_mut(block.0) {
            *s = Some(setup);
        }
    }

    /// A calendar of `events`; its name is its id.
    pub fn calendar(&mut self, name: &str, events: Vec<Event>) {
        self.calendars.push(Calendar {
            id: ElementId::unchecked(name),
            name: name.to_owned(),
            events,
        });
    }

    /// Pool `pool`'s units while each calendar is active (the first active one applying); its
    /// quantity applies outside them.
    pub fn shifts(&mut self, pool: &str, shifts: &[(&str, u32)]) {
        if let Some(p) = self.pools.iter_mut().find(|p| p.name == pool) {
            p.shifts = shifts
                .iter()
                .map(|(c, q)| Timed {
                    calendar: ElementId::unchecked(c),
                    value: *q,
                })
                .collect();
        }
    }

    /// Source `block`'s time between arrivals while each calendar is active (the first active
    /// one applying); outside them, its own interarrival time, or none with `idle_outside`.
    pub fn timed_arrivals(&mut self, block: BlockId, timed: Vec<(&str, Dist)>, idle_outside: bool) {
        if let Some((
            _,
            ElementKind::Source {
                timed: t,
                idle_outside: i,
                ..
            },
            _,
        )) = self.elements.get_mut(block.0)
        {
            *t = timed
                .into_iter()
                .map(|(c, d)| Timed {
                    calendar: ElementId::unchecked(c),
                    value: d,
                })
                .collect();
            *i = idle_outside;
        }
    }

    /// A task whose units come from the shared pools: each entity takes what the first of
    /// `needs` that can be met asks for, ahead of entities of tasks with a lower `priority`.
    pub fn shared_task(
        &mut self,
        name: &str,
        service: Dist,
        needs: Vec<Need>,
        priority: f64,
    ) -> BlockId {
        self.add(
            name,
            ElementKind::SharedTask {
                service,
                needs,
                priority,
            },
        )
    }

    /// Holds every entity for a time drawn from `duration`, with no capacity limit.
    pub fn delay(&mut self, name: &str, duration: Dist) -> BlockId {
        self.add(name, ElementKind::Delay { duration })
    }

    /// Removes entities from the model and records their time in system.
    pub fn sink(&mut self, name: &str) -> BlockId {
        self.add(name, ElementKind::Sink)
    }

    /// Sends each entity down one outgoing connection: by condition if any connection has one
    /// ([`Builder::route`]), else by the connections' probabilities.
    pub fn exclusive(&mut self, name: &str) -> BlockId {
        self.add(
            name,
            ElementKind::ExclusiveGateway {
                default: None,
                routing: Routing::Standard,
            },
        )
    }

    /// Makes the connection from exclusive gateway `gateway` to `to` its default.
    pub fn default_route(&mut self, gateway: BlockId, to: BlockId) {
        // Connection ids are only known when the model is built: keep the target's name.
        let target = self
            .elements
            .get(to.0)
            .map(|(n, _, _)| n.clone())
            .unwrap_or_default();
        if let Some((_, ElementKind::ExclusiveGateway { default, .. }, _)) =
            self.elements.get_mut(gateway.0)
        {
            *default = Some(ElementId::unchecked(&format!("->{target}")));
        }
    }

    /// Sends each entity to the way with the shortest queue that has room
    /// ([`Routing::ShortestQueue`]), the sender waiting while no way has any.
    pub fn shortest_queue(&mut self, name: &str) -> BlockId {
        self.add(
            name,
            ElementKind::ExclusiveGateway {
                default: None,
                routing: Routing::ShortestQueue,
            },
        )
    }

    /// Copies each entity down every outgoing connection; with several incoming connections,
    /// waits for one entity on each first.
    pub fn parallel(&mut self, name: &str) -> BlockId {
        self.add(name, ElementKind::ParallelGateway)
    }

    /// Collects `size` entities and sends them on as one batch.
    pub fn batch(&mut self, name: &str, size: u32) -> BlockId {
        self.add(name, ElementKind::Batch { size })
    }

    /// Splits batches back into their entities.
    pub fn unbatch(&mut self, name: &str) -> BlockId {
        self.add(name, ElementKind::Unbatch)
    }

    /// Makes the connection from `from` into parallel gateway `to` give it `quantity` entities
    /// for each one it sends on (a bill of materials). Connect them first.
    pub fn quantity(&mut self, from: BlockId, to: BlockId, quantity: u32) {
        self.quantities.insert((from, to), quantity);
    }

    fn is_gateway(&self, b: BlockId) -> bool {
        self.elements
            .get(b.0)
            .is_some_and(|(_, k, _)| k.is_gateway())
    }

    /// Sends `from`'s entities to `to`. A later call for the same `from` replaces the earlier one,
    /// unless `from` is a gateway.
    pub fn connect(&mut self, from: BlockId, to: BlockId) {
        self.route(from, to, None, None);
    }

    /// Connects exclusive gateway `from` to `to`, taken with `probability`, or when `condition`
    /// (a BPSim expression) holds.
    pub fn route(
        &mut self,
        from: BlockId,
        to: BlockId,
        probability: Option<f64>,
        condition: Option<&str>,
    ) {
        if !self.is_gateway(from) {
            self.connections.retain(|(f, ..)| *f != from);
        }
        self.connections
            .push((from, to, probability, condition.map(str::to_owned)));
    }

    /// The model, with the given experiment settings. Names that are not valid ids are kept as
    /// written in the model's diagnostics.
    pub fn build(&self, experiment: Experiment) -> Model {
        let id = |i: usize| {
            let name = self.elements.get(i).map_or("", |(n, _, _)| n.as_str());
            ElementId::new(name).unwrap_or_else(|| ElementId::unchecked(name))
        };
        let mut connections: Vec<&(BlockId, BlockId, Option<f64>, Option<String>)> =
            self.connections.iter().collect();
        connections.sort_by_key(|(from, to, ..)| (*from, *to));
        let connections: Vec<Connection> = connections
            .into_iter()
            .enumerate()
            .map(|(n, (from, to, probability, condition))| Connection {
                id: ElementId::unchecked(&format!("flow-{}", n + 1)),
                from: id(from.0),
                to: id(to.0),
                probability: *probability,
                condition: condition.clone(),
                quantity: self.quantities.get(&(*from, *to)).copied(),
            })
            .collect();
        let elements = self
            .elements
            .iter()
            .enumerate()
            .map(|(i, (name, kind, properties))| {
                // A default route names its target (`->name`) until connection ids exist.
                let kind = match kind {
                    ElementKind::ExclusiveGateway {
                        default: Some(d),
                        routing,
                    } => ElementKind::ExclusiveGateway {
                        default: d.as_str().strip_prefix("->").and_then(|t| {
                            connections
                                .iter()
                                .find(|c| c.from == id(i) && c.to.as_str() == t)
                                .map(|c| c.id.clone())
                        }),
                        routing: *routing,
                    },
                    other => other.clone(),
                };
                Element {
                    id: id(i),
                    name: name.clone(),
                    kind,
                    properties: properties.clone(),
                }
            })
            .collect();
        Model {
            name: String::new(),
            elements,
            connections,
            resources: self.pools.clone(),
            calendars: self.calendars.clone(),
            experiment,
            ..Model::default()
        }
    }

    /// Builds, compiles and runs one replication for `warmup + duration` time units.
    pub fn run(&self, rng: Rng, warmup: f64, duration: f64) -> Result<Report, Failed> {
        let experiment = Experiment {
            warmup,
            duration,
            replications: 1,
            ..Experiment::default()
        };
        compile(&self.build(experiment))
            .map_err(Failed::Invalid)?
            .run(rng)
            .map_err(Failed::Run)
    }
}

#[cfg(test)]
mod tests;
