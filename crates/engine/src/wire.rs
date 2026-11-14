//! The JSON forms of models, distributions, diagnostics and results: what commands take and
//! return. They are the engine's own (the files on disk stay BPSim on BPMN or XPDL); the model
//! crates below stay free of serialisation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_blocks::{ExperimentSummary, Metric};
use stochcraft_model::calendar::{Calendar, DateTime, Event, Rule};
use stochcraft_model::{
    Bounds, Breakdown, ByValue, Change, Connection, Demand, DemandPer, Diagnostic, Dist, Element,
    ElementId, ElementKind, Estimate, Experiment, Family, HealthCheck, Model, Need, OnShiftEnd,
    OnTakeout, Patience, PatienceUntil, PlaceWhileOut, Power, ProcessingBy, Property, PropertyType,
    PropertyValue, ResourcePool, Routing, Scrap, Setup, Severity, Stop, ThreePoint, TimeUnit,
    Timed, Transfer, Upper,
};
use stochcraft_stats::split::{self, Split};

/// A probability distribution. Times are in the experiment's time unit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DistSpec {
    /// Always `value`.
    Constant { value: f64 },
    /// Exponential (BPSim `NegativeExponentialDistribution`) with this mean.
    Exponential { mean: f64 },
    /// Uniform on [min, max).
    Uniform { min: f64, max: f64 },
    /// Triangular on [min, max], peaking at `mode`.
    Triangular { min: f64, mode: f64, max: f64 },
    /// Normal.
    Normal { mean: f64, std_dev: f64 },
    /// Normal restricted to [min, max].
    TruncatedNormal {
        mean: f64,
        std_dev: f64,
        min: f64,
        max: f64,
    },
    /// Log-normal with this mean and standard deviation (of the values, not their logarithm).
    LogNormal { mean: f64, std_dev: f64 },
    /// Gamma with shape k and scale θ (mean kθ).
    Gamma { shape: f64, scale: f64 },
    /// Beta on [0, 1] with shape parameters α = `shape`, β = `scale` (BPSim's names).
    Beta { shape: f64, scale: f64 },
    /// The sum of `k` exponentials with overall mean `mean`.
    Erlang { mean: f64, k: u64 },
    /// Weibull with shape k and scale λ.
    Weibull { shape: f64, scale: f64 },
    /// Poisson count with this mean.
    Poisson { mean: f64 },
    /// Binomial count of successes in `trials`.
    Binomial { probability: f64, trials: u64 },
    /// Data points. `discrete`: one of the values with its probability; otherwise a cumulative
    /// curve through the points, interpolated linearly.
    User {
        points: Vec<UserPoint>,
        #[serde(default)]
        discrete: bool,
    },
}

/// One point of a user distribution.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserPoint {
    pub probability: f64,
    pub value: f64,
}

impl From<&Dist> for DistSpec {
    fn from(d: &Dist) -> Self {
        match d.clone() {
            Dist::Constant(value) => DistSpec::Constant { value },
            Dist::Exponential { mean } => DistSpec::Exponential { mean },
            Dist::Uniform { min, max } => DistSpec::Uniform { min, max },
            Dist::Triangular { min, mode, max } => DistSpec::Triangular { min, mode, max },
            Dist::Normal { mean, std_dev } => DistSpec::Normal { mean, std_dev },
            Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            } => DistSpec::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            },
            Dist::LogNormal { mean, std_dev } => DistSpec::LogNormal { mean, std_dev },
            Dist::Gamma { shape, scale } => DistSpec::Gamma { shape, scale },
            Dist::Beta { shape, scale } => DistSpec::Beta { shape, scale },
            Dist::Erlang { mean, k } => DistSpec::Erlang { mean, k },
            Dist::Weibull { shape, scale } => DistSpec::Weibull { shape, scale },
            Dist::Poisson { mean } => DistSpec::Poisson { mean },
            Dist::Binomial {
                probability,
                trials,
            } => DistSpec::Binomial {
                probability,
                trials,
            },
            Dist::User { points, discrete } => DistSpec::User {
                points: points
                    .into_iter()
                    .map(|(probability, value)| UserPoint { probability, value })
                    .collect(),
                discrete,
            },
        }
    }
}

impl From<DistSpec> for Dist {
    fn from(d: DistSpec) -> Self {
        match d {
            DistSpec::Constant { value } => Dist::Constant(value),
            DistSpec::Exponential { mean } => Dist::Exponential { mean },
            DistSpec::Uniform { min, max } => Dist::Uniform { min, max },
            DistSpec::Triangular { min, mode, max } => Dist::Triangular { min, mode, max },
            DistSpec::Normal { mean, std_dev } => Dist::Normal { mean, std_dev },
            DistSpec::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            } => Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            },
            DistSpec::LogNormal { mean, std_dev } => Dist::LogNormal { mean, std_dev },
            DistSpec::Gamma { shape, scale } => Dist::Gamma { shape, scale },
            DistSpec::Beta { shape, scale } => Dist::Beta { shape, scale },
            DistSpec::Erlang { mean, k } => Dist::Erlang { mean, k },
            DistSpec::Weibull { shape, scale } => Dist::Weibull { shape, scale },
            DistSpec::Poisson { mean } => Dist::Poisson { mean },
            DistSpec::Binomial {
                probability,
                trials,
            } => Dist::Binomial {
                probability,
                trials,
            },
            DistSpec::User { points, discrete } => Dist::User {
                points: points.iter().map(|p| (p.probability, p.value)).collect(),
                discrete,
            },
        }
    }
}

fn exponential_1() -> DistSpec {
    DistSpec::Exponential { mean: 1.0 }
}

fn constant_1() -> DistSpec {
    DistSpec::Constant { value: 1.0 }
}

fn one() -> u32 {
    1
}

/// What an element is and its parameters. Missing parameters take their defaults: exponential
/// times with mean 1, one server unit, a delay of 1, no arrival limit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
// The wire form of a block, one per element in a document, never in the event loop: a
// server's settings stay inline (see `ElementKind`).
#[allow(clippy::large_enum_variant)]
pub enum KindSpec {
    /// Creates entities (a BPMN start event).
    Source {
        /// Time between arrivals.
        #[serde(default = "exponential_1")]
        interarrival: DistSpec,
        /// Stop after this many arrivals; none means no limit.
        #[serde(default)]
        limit: Option<u64>,
        /// Time between arrivals while a calendar is active, the first active one applying;
        /// `interarrival` applies outside them. Left out when none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        timed: Vec<TimedDist>,
        /// No arrivals outside the calendars of `timed`. Left out when false.
        #[serde(default, skip_serializing_if = "is_false")]
        idle_outside: bool,
    },
    /// Identical units serving one entity each, first come first served (a task with a
    /// resource).
    Server {
        #[serde(default = "one")]
        capacity: u32,
        /// Time to serve one entity.
        #[serde(default = "exponential_1")]
        service: DistSpec,
        /// Processing times for entities with particular values of a property (a time by
        /// product type), `service` applying to the others; left out when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        processing_by: Option<ProcessingBySpec>,
        /// A finite buffer: at most this many entities wait for a unit (not counting those in
        /// service); left out, there is no limit. 0 means no waiting room. While it is full, a
        /// server or delay sending here holds its finished entity (blocking after service), a
        /// gateway waits, and a source's arrival is lost.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queue_capacity: Option<u32>,
        /// Each unit fails and is repaired; left out when units never fail.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        breakdown: Option<BreakdownSpec>,
        /// The time each part takes to get here from the element that sends it (a short
        /// conveyor, a hand transfer, a minimum time in the buffer); left out, parts arrive at
        /// once. The part takes its place in the buffer as the transfer starts and joins the
        /// queue as it ends.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transfer: Option<DistSpec>,
        /// Whether a sending server or task with shared resources keeps its unit, blocked,
        /// through the transfer (else it is free at once); left out when true.
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        transfer_holds_sender: bool,
        /// Whether the units are available (on shift) outside the calendars of
        /// `availability`; left out when true.
        #[serde(default = "yes", skip_serializing_if = "is_true")]
        available: bool,
        /// Availability while a calendar is active, the first active one applying; all the
        /// units follow it. A unit checks it as it takes its next entity, so a job under way
        /// finishes first; failures and repairs go on through the breaks. Left out when none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        availability: Vec<AvailabilityDoc>,
        /// Units while a calendar is active (a shift), the first active one applying; `capacity`
        /// applies outside them. The units on shift are the shift's while the units are
        /// available. Left out when none.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        shifts: Vec<ShiftDoc>,
        /// What happens to a job under way on a unit that goes off shift: `finish` (the default,
        /// left out) finishes it first; `retry` sends it back to the head of the queue at once,
        /// its work wasted; `resume` pauses it on the unit until the unit is on shift again.
        #[serde(default, skip_serializing_if = "OnShiftEndSpec::is_finish")]
        on_shift_end: OnShiftEndSpec,
        /// The power each unit draws in each state, in kilowatts, for energy results; left out
        /// when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        power: Option<PowerSpec>,
        /// A setup before a job: before every job, or with `on_change_of` only when that
        /// property's value changes from the last part's (a changeover); left out when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        setup: Option<SetupSpec>,
        /// The planned time to make one item (ISO 22400-2's planned runtime per item), for the
        /// OEE results; its mean is the plan. Not simulated. Left out when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        planned_run_time: Option<DistSpec>,
        /// How long an entity waits before it gives up (and takes the way out marked
        /// `abandoned`, or leaves the model); left out when entities wait for as long as it
        /// takes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        patience: Option<PatienceSpec>,
        /// Each unit is checked now and then and taken out for a restart when too many
        /// entities wait per working unit; left out when units are never taken out.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        health_check: Option<HealthCheckSpec>,
    },
    /// A task whose units come from the model's shared resource pools (`resources`): each
    /// entity takes what the first of `needs` that can be met now asks for, holds it for its
    /// service, then gives it back. When units come free, waiting entities across all tasks get
    /// them by their task's `priority` (higher first), then by arrival.
    SharedTask {
        /// Time to serve one entity.
        #[serde(default = "exponential_1")]
        service: DistSpec,
        /// Processing times for entities with particular values of a property (a time by
        /// product type), `service` applying to the others; left out when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        processing_by: Option<ProcessingBySpec>,
        /// The alternatives, tried in order (BPSim `orResource`).
        needs: Vec<NeedSpec>,
        /// Who gets freed units first, higher first; left out, 0.
        #[serde(default, skip_serializing_if = "is_zero")]
        priority: f64,
    },
    /// Holds each entity for a time, with no capacity limit (a task with no resource).
    Delay {
        #[serde(default = "constant_1")]
        duration: DistSpec,
        /// Processing times for entities with particular values of a property (a time by
        /// product type), `duration` applying to the others; left out when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        processing_by: Option<ProcessingBySpec>,
    },
    /// Removes entities (a BPMN end event). With `scrap`, what reaches it is scrap, for the OEE
    /// quality results; left out when it is not.
    Sink {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scrap: Option<ScrapSpec>,
        /// The customer demand it serves, for takt; left out when none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        demand: Option<DemandSpec>,
    },
    /// Sends each entity down one outgoing connection at once (a BPMN exclusive gateway): the
    /// first whose condition holds if any has one, else drawn by the connections'
    /// probabilities (`flow.set`); or, with `routing` `shortest_queue`, to the way with the
    /// shortest queue that has room.
    ExclusiveGateway {
        /// The outgoing connection taken when no condition holds; without one, such an entity
        /// leaves the model.
        #[serde(default)]
        default: Option<String>,
        /// How the way is chosen; left out when standard.
        #[serde(default, skip_serializing_if = "RoutingSpec::is_standard")]
        routing: RoutingSpec,
    },
    /// Copies each entity down every outgoing connection at once (a BPMN parallel gateway);
    /// with several incoming connections, first waits for an entity on each (a join). A
    /// connection's `quantity` makes it an assembly: that many entities from it go into each
    /// one sent on.
    ParallelGateway,
    /// Collects entities until there are `size`, then sends them on together as one batch.
    Batch {
        #[serde(default = "one")]
        size: u32,
    },
    /// Splits each batch back into its entities; anything else passes through.
    Unbatch,
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

/// What a task with shared resources needs for each entity: units of one pool (BPSim
/// `getResource`), or units from the pools that have all of `roles`, in model order
/// (`getResourceByRoles`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum NeedSpec {
    Resource {
        /// The pool's id.
        resource: String,
        #[serde(default = "one")]
        quantity: u32,
    },
    Roles {
        roles: Vec<String>,
        #[serde(default = "one")]
        quantity: u32,
    },
}

impl From<&Need> for NeedSpec {
    fn from(n: &Need) -> Self {
        match n {
            Need::Resource { id, quantity } => NeedSpec::Resource {
                resource: id.to_string(),
                quantity: *quantity,
            },
            Need::Roles { roles, quantity } => NeedSpec::Roles {
                roles: roles.clone(),
                quantity: *quantity,
            },
        }
    }
}

impl From<NeedSpec> for Need {
    fn from(n: NeedSpec) -> Self {
        match n {
            NeedSpec::Resource { resource, quantity } => Need::Resource {
                id: ElementId::unchecked(&resource),
                quantity,
            },
            NeedSpec::Roles { roles, quantity } => Need::Roles { roles, quantity },
        }
    }
}

/// A pool of identical units that tasks share (a BPMN `resource`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceDoc {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// How many units; 1 by default.
    #[serde(default = "one")]
    pub quantity: u32,
    /// The roles its units play, for `roles` needs; left out when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Units while a calendar is active, the first active one applying; `quantity` applies
    /// outside them. Left out when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shifts: Vec<ShiftDoc>,
    /// Whether the units are available outside the calendars of `availability`; left out
    /// when true.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub available: bool,
    /// Availability while a calendar is active. Left out when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub availability: Vec<AvailabilityDoc>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_true(b: &bool) -> bool {
    *b
}

fn yes() -> bool {
    true
}

/// How an exclusive gateway chooses the way each entity takes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutingSpec {
    /// By the first condition that holds, else the default; with no conditions, drawn by the
    /// connections' probabilities.
    #[default]
    Standard,
    /// To the way with the shortest queue (entities waiting or in transit) that has room; a
    /// tie goes to the way taken longest ago. With no room on any way, the element sending
    /// into the gateway waits (a server blocked after its job), or a source loses its arrival.
    ShortestQueue,
}

impl RoutingSpec {
    fn is_standard(&self) -> bool {
        *self == RoutingSpec::Standard
    }
}

impl From<Routing> for RoutingSpec {
    fn from(r: Routing) -> Self {
        match r {
            Routing::Standard => RoutingSpec::Standard,
            Routing::ShortestQueue => RoutingSpec::ShortestQueue,
        }
    }
}

impl From<RoutingSpec> for Routing {
    fn from(r: RoutingSpec) -> Self {
        match r {
            RoutingSpec::Standard => Routing::Standard,
            RoutingSpec::ShortestQueue => Routing::ShortestQueue,
        }
    }
}

/// A server's setup: time a unit spends preparing before a job, holding the part, in a state of
/// its own. Without `on_change_of`, before every job (BPSim `SetUpTime`); with it, a changeover:
/// only when the part's value of that property differs from that of the part the unit served
/// last (its first part needs none), for the time `by_change` gives for that change, else `time`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetupSpec {
    pub time: DistSpec,
    /// The property whose value changing makes a changeover; left out: before every job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_change_of: Option<String>,
    /// Times for particular changes, from one value to another (values as text).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_change: Vec<ChangeSpec>,
}

/// The time of a changeover from parts whose value is `from` to parts whose value is `to`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeSpec {
    pub from: String,
    pub to: String,
    pub time: DistSpec,
}

impl From<&Setup> for SetupSpec {
    fn from(s: &Setup) -> Self {
        Self {
            time: (&s.time).into(),
            on_change_of: s.on_change_of.clone(),
            by_change: s
                .by_change
                .iter()
                .map(|c| ChangeSpec {
                    from: c.from.clone(),
                    to: c.to.clone(),
                    time: (&c.time).into(),
                })
                .collect(),
        }
    }
}

impl From<SetupSpec> for Setup {
    fn from(s: SetupSpec) -> Self {
        Self {
            time: s.time.into(),
            on_change_of: s.on_change_of.filter(|p| !p.trim().is_empty()),
            by_change: s
                .by_change
                .into_iter()
                .map(|c| Change {
                    from: c.from,
                    to: c.to,
                    time: c.time.into(),
                })
                .collect(),
        }
    }
}

/// A task's processing times by the value of an entity property (a time by product type): an
/// entity whose value of `property`, as text, is the `value` of one of `by_value` takes that
/// `time`; any other entity (another value, or none) takes the task's own time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProcessingBySpec {
    pub property: String,
    /// One time per value (values as text: `A`, `2`, `true`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_value: Vec<ByValueSpec>,
}

/// The processing time of entities whose property value is `value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ByValueSpec {
    pub value: String,
    pub time: DistSpec,
}

impl From<&ProcessingBy> for ProcessingBySpec {
    fn from(b: &ProcessingBy) -> Self {
        Self {
            property: b.property.clone(),
            by_value: b
                .by_value
                .iter()
                .map(|v| ByValueSpec {
                    value: v.value.clone(),
                    time: (&v.time).into(),
                })
                .collect(),
        }
    }
}

/// Processing times by property value from their wire form: none when there is neither a
/// property nor a time.
pub fn processing_by(spec: Option<ProcessingBySpec>) -> Option<ProcessingBy> {
    let s = spec?;
    let property = s.property.trim().to_owned();
    (!property.is_empty() || !s.by_value.is_empty()).then(|| ProcessingBy {
        property,
        by_value: s
            .by_value
            .into_iter()
            .map(|v| ByValue {
                value: v.value,
                time: v.time.into(),
            })
            .collect(),
    })
}

/// How a server's units fail: each unit, independently, after a time to failure since its last
/// repair (busy or idle), then is repaired.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BreakdownSpec {
    pub time_to_failure: DistSpec,
    pub repair: DistSpec,
    /// A failure interrupts the job under way, which resumes on the same unit after the repair
    /// with the time it had left; otherwise the unit fails once its job is done.
    #[serde(default)]
    pub interruptible: bool,
    /// A unit that fails while holding a finished entity still hands it on when there is room
    /// (true), or keeps it until repaired (false); left out when true.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    // schemars leaves out a default that `skip_serializing_if` would; the forms need it.
    #[schemars(extend("default" = true))]
    pub hands_on_while_down: bool,
}

impl From<&Breakdown> for BreakdownSpec {
    fn from(b: &Breakdown) -> Self {
        Self {
            time_to_failure: (&b.time_to_failure).into(),
            repair: (&b.repair).into(),
            interruptible: b.interruptible,
            hands_on_while_down: b.hands_on_while_down,
        }
    }
}

impl From<BreakdownSpec> for Breakdown {
    fn from(b: BreakdownSpec) -> Self {
        Self {
            time_to_failure: b.time_to_failure.into(),
            repair: b.repair.into(),
            interruptible: b.interruptible,
            hands_on_while_down: b.hands_on_while_down,
        }
    }
}

/// The power a server's unit draws in each state, in kilowatts. A unit is in exactly one state
/// at a time, the first that applies of failed (or under repair), processing, blocked, off
/// shift (idle because the calendar is closed) and starved (idle); blocked, failed and off shift draw `idle` when left out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PowerSpec {
    pub processing: f64,
    pub idle: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_shift: Option<f64>,
}

impl From<&Power> for PowerSpec {
    fn from(p: &Power) -> Self {
        Self {
            processing: p.processing,
            idle: p.idle,
            blocked: p.blocked,
            failed: p.failed,
            off_shift: p.off_shift,
        }
    }
}

impl From<PowerSpec> for Power {
    fn from(p: PowerSpec) -> Self {
        Self {
            processing: p.processing,
            idle: p.idle,
            blocked: p.blocked,
            failed: p.failed,
            off_shift: p.off_shift,
        }
    }
}

/// A time between arrivals that applies while a calendar is active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimedDist {
    /// The calendar's id.
    pub calendar: String,
    pub value: DistSpec,
}

/// A pool's units while a calendar is active (a shift).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShiftDoc {
    /// The calendar's id.
    pub calendar: String,
    pub quantity: u32,
}

/// Whether a pool's or a server's units are available while a calendar is active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AvailabilityDoc {
    /// The calendar's id.
    pub calendar: String,
    pub available: bool,
}

fn timed<T, U>(v: &[Timed<T>], f: impl Fn(&Timed<T>) -> U) -> Vec<U> {
    v.iter().map(f).collect()
}

impl From<&ResourcePool> for ResourceDoc {
    fn from(r: &ResourcePool) -> Self {
        Self {
            id: r.id.to_string(),
            name: r.name.clone(),
            quantity: r.quantity,
            roles: r.roles.clone(),
            shifts: timed(&r.shifts, |t| ShiftDoc {
                calendar: t.calendar.to_string(),
                quantity: t.value,
            }),
            available: r.available,
            availability: timed(&r.availability, |t| AvailabilityDoc {
                calendar: t.calendar.to_string(),
                available: t.value,
            }),
        }
    }
}

impl From<ResourceDoc> for ResourcePool {
    fn from(r: ResourceDoc) -> Self {
        Self {
            id: ElementId::unchecked(&r.id),
            name: r.name,
            quantity: r.quantity,
            roles: r.roles,
            shifts: r.shifts.into_iter().map(Into::into).collect(),
            available: r.available,
            availability: r.availability.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ShiftDoc> for Timed<u32> {
    fn from(s: ShiftDoc) -> Self {
        Timed {
            calendar: ElementId::unchecked(&s.calendar),
            value: s.quantity,
        }
    }
}

impl From<AvailabilityDoc> for Timed<bool> {
    fn from(s: AvailabilityDoc) -> Self {
        Timed {
            calendar: ElementId::unchecked(&s.calendar),
            value: s.available,
        }
    }
}

impl From<TimedDist> for Timed<Dist> {
    fn from(t: TimedDist) -> Self {
        Timed {
            calendar: ElementId::unchecked(&t.calendar),
            value: t.value.into(),
        }
    }
}

/// A date and time, written `2016-01-01T06:00:00` (XML Schema's form, as BPSim files have it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp(pub DateTime);

impl Serialize for Stamp {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.iso())
    }
}

impl<'de> Deserialize<'de> for Stamp {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        DateTime::parse_iso(&text).map(Stamp).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "`{text}` is not a date and time such as 2016-01-01T06:00:00"
            ))
        })
    }
}

/// An iCalendar repeat rule, written as RFC 5545 does: `FREQ=WEEKLY;BYDAY=MO,TU;COUNT=10`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleSpec(pub Rule);

impl Serialize for RuleSpec {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.text())
    }
}

impl<'de> Deserialize<'de> for RuleSpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Rule::parse(&text)
            .map(RuleSpec)
            .map_err(|e| serde::de::Error::custom(format!("repeat `{text}`: {e}")))
    }
}

/// A period, which can repeat (an iCalendar `VEVENT`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventDoc {
    /// When the (first) period starts, such as `2016-01-01T09:00:00`.
    #[schemars(with = "String")]
    pub start: Stamp,
    /// When it ends; each repeat lasts as long.
    #[schemars(with = "String")]
    pub end: Stamp,
    /// How it repeats, as an iCalendar RRULE: FREQ=DAILY or WEEKLY, with INTERVAL, BYDAY
    /// (MO…SU), WKST, and UNTIL or COUNT. Left out when it does not repeat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub repeat: Option<RuleSpec>,
}

/// A calendar: values valid for it apply while any of its periods is under way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalendarDoc {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub events: Vec<EventDoc>,
}

impl From<&Event> for EventDoc {
    fn from(e: &Event) -> Self {
        Self {
            start: Stamp(e.start),
            end: Stamp(e.end),
            repeat: e.rule.clone().map(RuleSpec),
        }
    }
}

impl From<EventDoc> for Event {
    fn from(e: EventDoc) -> Self {
        Self {
            start: e.start.0,
            end: e.end.0,
            rule: e.repeat.map(|r| r.0),
        }
    }
}

impl From<&Calendar> for CalendarDoc {
    fn from(c: &Calendar) -> Self {
        Self {
            id: c.id.to_string(),
            name: c.name.clone(),
            events: c.events.iter().map(Into::into).collect(),
        }
    }
}

impl From<CalendarDoc> for Calendar {
    fn from(c: CalendarDoc) -> Self {
        Self {
            id: ElementId::unchecked(&c.id),
            name: c.name,
            events: c.events.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<&ElementKind> for KindSpec {
    fn from(k: &ElementKind) -> Self {
        match k {
            ElementKind::Source {
                interarrival,
                limit,
                timed: t,
                idle_outside,
            } => KindSpec::Source {
                interarrival: interarrival.into(),
                limit: *limit,
                timed: timed(t, |t| TimedDist {
                    calendar: t.calendar.to_string(),
                    value: (&t.value).into(),
                }),
                idle_outside: *idle_outside,
            },
            ElementKind::Server {
                capacity,
                service,
                processing_by,
                queue_capacity,
                breakdown,
                transfer,
                available,
                availability,
                power,
                setup,
                planned_run_time,
                patience,
                health_check,
                shifts,
                on_shift_end,
            } => KindSpec::Server {
                capacity: *capacity,
                service: service.into(),
                processing_by: processing_by.as_ref().map(Into::into),
                queue_capacity: *queue_capacity,
                breakdown: breakdown.as_ref().map(Into::into),
                transfer: transfer.as_ref().map(|t| (&t.time).into()),
                transfer_holds_sender: transfer.as_ref().is_none_or(|t| t.holds_sender),
                available: *available,
                availability: timed(availability, |t| AvailabilityDoc {
                    calendar: t.calendar.to_string(),
                    available: t.value,
                }),
                power: power.as_ref().map(Into::into),
                setup: setup.as_ref().map(Into::into),
                planned_run_time: planned_run_time.as_ref().map(Into::into),
                patience: patience.as_ref().map(Into::into),
                health_check: health_check.as_ref().map(Into::into),
                shifts: timed(shifts, |t| ShiftDoc {
                    calendar: t.calendar.to_string(),
                    quantity: t.value,
                }),
                on_shift_end: (*on_shift_end).into(),
            },
            ElementKind::SharedTask {
                service,
                processing_by,
                needs,
                priority,
            } => KindSpec::SharedTask {
                service: service.into(),
                processing_by: processing_by.as_ref().map(Into::into),
                needs: needs.iter().map(Into::into).collect(),
                priority: *priority,
            },
            ElementKind::Delay {
                duration,
                processing_by,
            } => KindSpec::Delay {
                duration: duration.into(),
                processing_by: processing_by.as_ref().map(Into::into),
            },
            ElementKind::Sink { scrap, demand } => KindSpec::Sink {
                scrap: scrap.as_ref().map(|s| ScrapSpec {
                    charged_to: s.charged_to.as_ref().map(ToString::to_string),
                }),
                demand: demand.as_ref().map(Into::into),
            },
            ElementKind::ExclusiveGateway { default, routing } => KindSpec::ExclusiveGateway {
                default: default.as_ref().map(ToString::to_string),
                routing: (*routing).into(),
            },
            ElementKind::ParallelGateway => KindSpec::ParallelGateway,
            ElementKind::Batch { size } => KindSpec::Batch { size: *size },
            ElementKind::Unbatch => KindSpec::Unbatch,
        }
    }
}

impl From<KindSpec> for ElementKind {
    fn from(k: KindSpec) -> Self {
        match k {
            KindSpec::Source {
                interarrival,
                limit,
                timed,
                idle_outside,
            } => ElementKind::Source {
                interarrival: interarrival.into(),
                limit,
                timed: timed.into_iter().map(Into::into).collect(),
                idle_outside,
            },
            KindSpec::Server {
                capacity,
                service,
                processing_by: by,
                queue_capacity,
                breakdown,
                transfer,
                transfer_holds_sender,
                available,
                availability,
                power,
                setup,
                planned_run_time,
                patience,
                health_check,
                shifts,
                on_shift_end,
            } => ElementKind::Server {
                capacity,
                service: service.into(),
                processing_by: processing_by(by),
                queue_capacity,
                breakdown: breakdown.map(Into::into),
                transfer: transfer.map(|t| Transfer {
                    time: t.into(),
                    holds_sender: transfer_holds_sender,
                }),
                available,
                availability: availability.into_iter().map(Into::into).collect(),
                power: power.map(Into::into),
                setup: setup.map(Into::into),
                planned_run_time: planned_run_time.map(Into::into),
                patience: patience.map(Into::into),
                health_check: health_check.map(Into::into),
                shifts: shifts.into_iter().map(Into::into).collect(),
                on_shift_end: on_shift_end.into(),
            },
            KindSpec::SharedTask {
                service,
                processing_by: by,
                needs,
                priority,
            } => ElementKind::SharedTask {
                service: service.into(),
                processing_by: processing_by(by),
                needs: needs.into_iter().map(Into::into).collect(),
                priority,
            },
            KindSpec::Delay {
                duration,
                processing_by: by,
            } => ElementKind::Delay {
                duration: duration.into(),
                processing_by: processing_by(by),
            },
            KindSpec::Sink { scrap, demand } => ElementKind::Sink {
                scrap: scrap.map(|s| Scrap {
                    charged_to: s.charged_to.as_deref().map(ElementId::unchecked),
                }),
                demand: demand.map(Into::into),
            },
            KindSpec::ExclusiveGateway { default, routing } => ElementKind::ExclusiveGateway {
                default: default.as_deref().map(ElementId::unchecked),
                routing: routing.into(),
            },
            KindSpec::ParallelGateway => ElementKind::ParallelGateway,
            KindSpec::Batch { size } => ElementKind::Batch { size },
            KindSpec::Unbatch => ElementKind::Unbatch,
        }
    }
}

/// What ends each replication before its time limit (a terminating experiment). Checked after
/// everything that happens at one instant, so the run ends at that instant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "when", rename_all = "snake_case", deny_unknown_fields)]
pub enum StopSpec {
    /// Once `count` entities have left `element` since time 0, warm-up included (for a sink:
    /// arrived there).
    Count { element: String, count: u64 },
    /// The first time task `element` holds no entity (none waiting, none in service) after it
    /// has taken one in.
    Empty { element: String },
}

impl StopSpec {
    pub fn element(&self) -> &str {
        match self {
            StopSpec::Count { element, .. } | StopSpec::Empty { element } => element,
        }
    }
}

impl From<&Stop> for StopSpec {
    fn from(s: &Stop) -> Self {
        match s {
            Stop::Count { element, count } => StopSpec::Count {
                element: element.to_string(),
                count: *count,
            },
            Stop::Empty { element } => StopSpec::Empty {
                element: element.to_string(),
            },
        }
    }
}

impl From<StopSpec> for Stop {
    fn from(s: StopSpec) -> Self {
        match s {
            StopSpec::Count { element, count } => Stop::Count {
                element: ElementId::unchecked(&element),
                count,
            },
            StopSpec::Empty { element } => Stop::Empty {
                element: ElementId::unchecked(&element),
            },
        }
    }
}

/// The distribution a three-point estimate is fitted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FamilySpec {
    /// Triangular on [min, max] peaking at the mode (BPSim's own).
    Triangular,
    /// The beta distribution on [min, max] with mean (min + 4 mode + max) / 6, written as a BPSim
    /// user distribution of its percentiles.
    Pert,
    /// Log-normal from the minimum, with the mode and 95th percentile given.
    Lognormal,
    /// Weibull from the minimum, with the mode and 95th percentile given.
    Weibull,
}

impl From<Family> for FamilySpec {
    fn from(f: Family) -> Self {
        match f {
            Family::Triangular => FamilySpec::Triangular,
            Family::Pert => FamilySpec::Pert,
            Family::LogNormal => FamilySpec::Lognormal,
            Family::Weibull => FamilySpec::Weibull,
        }
    }
}

impl From<FamilySpec> for Family {
    fn from(f: FamilySpec) -> Self {
        match f {
            FamilySpec::Triangular => Family::Triangular,
            FamilySpec::Pert => Family::Pert,
            FamilySpec::Lognormal => Family::LogNormal,
            FamilySpec::Weibull => Family::Weibull,
        }
    }
}

/// An expert's three-point estimate that the element's time distribution was fitted from (its
/// pedigree). Give `max` or `p95`, not both; log-normal and Weibull estimates take `p95`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EstimateDoc {
    pub family: FamilySpec,
    pub min: f64,
    /// The most likely value.
    pub mode: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// The value exceeded one time in twenty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95: Option<f64>,
    /// Who gave it or where it comes from.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

impl EstimateDoc {
    /// The estimate, or why not: exactly one of `max` and `p95` must be given.
    pub fn to_estimate(&self) -> Result<Estimate, &'static str> {
        let (upper, upper_is) = match (self.max, self.p95) {
            (Some(x), None) => (x, Upper::Max),
            (None, Some(x)) => (x, Upper::P95),
            (Some(_), Some(_)) => return Err("give `max` or `p95`, not both"),
            (None, None) => return Err("give `max` or `p95`"),
        };
        Ok(Estimate {
            three_point: ThreePoint {
                family: self.family.into(),
                min: self.min,
                mode: self.mode,
                upper,
                upper_is,
            },
            source: self.source.clone(),
        })
    }
}

impl From<&Estimate> for EstimateDoc {
    fn from(e: &Estimate) -> Self {
        let t = &e.three_point;
        let (max, p95) = match t.upper_is {
            Upper::Max => (Some(t.upper), None),
            Upper::P95 => (None, Some(t.upper)),
        };
        Self {
            family: t.family.into(),
            min: t.min,
            mode: t.mode,
            max,
            p95,
            source: e.source.clone(),
        }
    }
}

/// BPSim's time units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TimeUnitSpec {
    #[serde(rename = "ms")]
    Milliseconds,
    #[serde(rename = "s")]
    Seconds,
    #[serde(rename = "min")]
    Minutes,
    #[serde(rename = "hour")]
    Hours,
    #[serde(rename = "day")]
    Days,
    #[serde(rename = "year")]
    Years,
}

impl From<TimeUnit> for TimeUnitSpec {
    fn from(t: TimeUnit) -> Self {
        match t {
            TimeUnit::Milliseconds => TimeUnitSpec::Milliseconds,
            TimeUnit::Seconds => TimeUnitSpec::Seconds,
            TimeUnit::Minutes => TimeUnitSpec::Minutes,
            TimeUnit::Hours => TimeUnitSpec::Hours,
            TimeUnit::Days => TimeUnitSpec::Days,
            TimeUnit::Years => TimeUnitSpec::Years,
        }
    }
}

impl From<TimeUnitSpec> for TimeUnit {
    fn from(t: TimeUnitSpec) -> Self {
        match t {
            TimeUnitSpec::Milliseconds => TimeUnit::Milliseconds,
            TimeUnitSpec::Seconds => TimeUnit::Seconds,
            TimeUnitSpec::Minutes => TimeUnit::Minutes,
            TimeUnitSpec::Hours => TimeUnit::Hours,
            TimeUnitSpec::Days => TimeUnit::Days,
            TimeUnitSpec::Years => TimeUnit::Years,
        }
    }
}

/// BPSim's property types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum PropertyTypeSpec {
    #[serde(rename = "string")]
    String,
    #[serde(rename = "boolean")]
    Boolean,
    /// A whole number.
    #[serde(rename = "long")]
    Long,
    #[serde(rename = "double")]
    Double,
    /// A length of time in the experiment's time unit.
    #[serde(rename = "duration")]
    Duration,
    /// A date and time, as text.
    #[serde(rename = "dateTime")]
    DateTime,
}

impl From<PropertyType> for PropertyTypeSpec {
    fn from(t: PropertyType) -> Self {
        match t {
            PropertyType::String => PropertyTypeSpec::String,
            PropertyType::Boolean => PropertyTypeSpec::Boolean,
            PropertyType::Long => PropertyTypeSpec::Long,
            PropertyType::Double => PropertyTypeSpec::Double,
            PropertyType::Duration => PropertyTypeSpec::Duration,
            PropertyType::DateTime => PropertyTypeSpec::DateTime,
        }
    }
}

impl From<PropertyTypeSpec> for PropertyType {
    fn from(t: PropertyTypeSpec) -> Self {
        match t {
            PropertyTypeSpec::String => PropertyType::String,
            PropertyTypeSpec::Boolean => PropertyType::Boolean,
            PropertyTypeSpec::Long => PropertyType::Long,
            PropertyTypeSpec::Double => PropertyType::Double,
            PropertyTypeSpec::Duration => PropertyType::Duration,
            PropertyTypeSpec::DateTime => PropertyType::DateTime,
        }
    }
}

/// What a property is set to, one of: `{"number": 3}`, `{"boolean": true}`, `{"text": "gold"}`,
/// `{"random": <distribution>}` (drawn for each entity), `{"mix": [{"value": "A", "share": 0.6},
/// {"value": "B", "share": 0.4}]}` (text drawn for each entity by its share, such as a product mix)
/// or `{"expression": "getProperty('n') - 1"}` (BPSim's XPath, from the entity's properties as it
/// arrived).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PropertyValueSpec {
    Number(f64),
    Boolean(bool),
    Text(String),
    Random(DistSpec),
    Mix(Vec<MixShare>),
    Expression(String),
}

/// One value of a mix and its share. The shares need not add up to 1: each value is drawn in
/// proportion to its share.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MixShare {
    pub value: String,
    pub share: f64,
}

impl From<&PropertyValue> for PropertyValueSpec {
    fn from(v: &PropertyValue) -> Self {
        match v {
            PropertyValue::Number(n) => PropertyValueSpec::Number(*n),
            PropertyValue::Boolean(b) => PropertyValueSpec::Boolean(*b),
            PropertyValue::Text(t) => PropertyValueSpec::Text(t.clone()),
            PropertyValue::Random(d) => PropertyValueSpec::Random(d.into()),
            PropertyValue::Choice(c) => PropertyValueSpec::Mix(
                c.iter()
                    .map(|(share, value)| MixShare {
                        value: value.clone(),
                        share: *share,
                    })
                    .collect(),
            ),
            PropertyValue::Expression(x) => PropertyValueSpec::Expression(x.clone()),
        }
    }
}

impl From<PropertyValueSpec> for PropertyValue {
    /// A constant "distribution" is a fixed number, as model files hold it.
    fn from(v: PropertyValueSpec) -> Self {
        match v {
            PropertyValueSpec::Number(n)
            | PropertyValueSpec::Random(DistSpec::Constant { value: n }) => {
                PropertyValue::Number(n)
            }
            PropertyValueSpec::Boolean(b) => PropertyValue::Boolean(b),
            PropertyValueSpec::Text(t) => PropertyValue::Text(t),
            PropertyValueSpec::Random(d) => PropertyValue::Random(d.into()),
            PropertyValueSpec::Mix(m) => {
                PropertyValue::Choice(m.into_iter().map(|s| (s.share, s.value)).collect())
            }
            PropertyValueSpec::Expression(x) => PropertyValue::Expression(x),
        }
    }
}

/// A property an element sets on every entity that enters it (for a source: that it creates).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PropertyDoc {
    pub name: String,
    /// The declared type; without one, the type declared elsewhere in the model or else the type
    /// of the property's first value.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub ty: Option<PropertyTypeSpec>,
    pub value: PropertyValueSpec,
}

impl From<&Property> for PropertyDoc {
    fn from(p: &Property) -> Self {
        Self {
            name: p.name.clone(),
            ty: p.ty.map(Into::into),
            value: (&p.value).into(),
        }
    }
}

impl From<PropertyDoc> for Property {
    fn from(p: PropertyDoc) -> Self {
        Self {
            name: p.name,
            ty: p.ty.map(Into::into),
            value: p.value.into(),
        }
    }
}

/// Properties in name order: the order model files read them in (an element's assignments are
/// made together, so the order means nothing).
pub fn sorted_properties(props: impl IntoIterator<Item = Property>) -> Vec<Property> {
    let mut v: Vec<Property> = props.into_iter().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// An element of a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ElementDoc {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(flatten)]
    pub kind: KindSpec,
    /// Properties the element sets on entities; left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyDoc>,
    /// Where the element is drawn; left out when it has no shape yet (it is then placed
    /// automatically).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<BoundsDoc>,
    /// The three-point estimate the element's time distribution was fitted from; left out when
    /// there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<EstimateDoc>,
}

/// A shape's top-left corner and size, in the diagram's units (BPMN DI's, y downwards).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundsDoc {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl From<Bounds> for BoundsDoc {
    fn from(b: Bounds) -> Self {
        Self {
            x: b.x,
            y: b.y,
            width: b.width,
            height: b.height,
        }
    }
}

impl From<BoundsDoc> for Bounds {
    fn from(b: BoundsDoc) -> Self {
        Self {
            x: b.x,
            y: b.y,
            width: b.width,
            height: b.height,
        }
    }
}

/// A connection from one element to another (a BPMN sequence flow).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionDoc {
    pub id: String,
    pub from: String,
    pub to: String,
    /// Out of an exclusive gateway: the chance an entity takes this connection; left out when
    /// not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability: Option<f64>,
    /// Out of an exclusive gateway: a BPSim expression that must hold to take it; left out
    /// when not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Into a parallel gateway: how many entities from this connection go into each one it
    /// sends on (an assembly's bill of materials); left out, 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<u32>,
    /// Entities taking it are rework, counted against the last server they finished at, for
    /// the OEE quality results; left out when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rework: bool,
    /// Out of a server with a patience: the way entities that give up take, besides its
    /// ordinary way out; left out when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub abandoned: bool,
}

/// How long an entity waits at a server before it gives up, drawn as it enters, and what must
/// happen in that time: its service start (`start`, the default, left out) or its service being
/// done (`done`; a unit serving it finishes the job anyway, and the work is wasted).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatienceSpec {
    pub time: DistSpec,
    #[serde(default, skip_serializing_if = "PatienceUntilSpec::is_start")]
    pub until: PatienceUntilSpec,
}

/// What an entity's patience must see happen in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatienceUntilSpec {
    /// Its service starting.
    #[default]
    Start,
    /// Its service being done.
    Done,
}

impl PatienceUntilSpec {
    fn is_start(&self) -> bool {
        *self == PatienceUntilSpec::Start
    }
}

impl From<&Patience> for PatienceSpec {
    fn from(p: &Patience) -> Self {
        PatienceSpec {
            time: (&p.time).into(),
            until: match p.until {
                PatienceUntil::Start => PatienceUntilSpec::Start,
                PatienceUntil::Done => PatienceUntilSpec::Done,
            },
        }
    }
}

/// What happens to a job under way on a server's unit that goes off shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnShiftEndSpec {
    /// The unit finishes it, then goes off.
    #[default]
    Finish,
    /// Back to the head of the queue at once, for any unit to start afresh; its work is wasted.
    Retry,
    /// It waits on the unit and carries on with what was left when the unit is on shift again.
    Resume,
}

impl OnShiftEndSpec {
    fn is_finish(&self) -> bool {
        *self == OnShiftEndSpec::Finish
    }
}

impl From<OnShiftEnd> for OnShiftEndSpec {
    fn from(o: OnShiftEnd) -> Self {
        match o {
            OnShiftEnd::Finish => OnShiftEndSpec::Finish,
            OnShiftEnd::Retry => OnShiftEndSpec::Retry,
            OnShiftEnd::Resume => OnShiftEndSpec::Resume,
        }
    }
}

impl From<OnShiftEndSpec> for OnShiftEnd {
    fn from(o: OnShiftEndSpec) -> Self {
        match o {
            OnShiftEndSpec::Finish => OnShiftEnd::Finish,
            OnShiftEndSpec::Retry => OnShiftEnd::Retry,
            OnShiftEndSpec::Resume => OnShiftEnd::Resume,
        }
    }
}

/// A health check on a server's units: each unit is checked every `interval` (on its own; the
/// first check one interval after the start) and is taken out of service for `restart` when
/// more entities wait than `limit` per working unit (up, not out and on shift). A unit off
/// shift, failed or already out is not checked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HealthCheckSpec {
    pub interval: DistSpec,
    /// The most entities waiting per working unit that still passes (a probe's timeout times
    /// one unit's service rate); left out, there is no limit and no check fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
    pub restart: DistSpec,
    /// Keep at least this many units in service: a failed check takes no unit out when that
    /// would leave fewer working. Left out when there is no floor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<u32>,
    /// What happens to a job under way on a unit taken out: `retry` (the default, left out),
    /// back to the head of the queue at once, its work wasted; or `resume`, it waits on the
    /// unit and resumes after the restart.
    #[serde(default, skip_serializing_if = "OnTakeoutSpec::is_retry")]
    pub on_takeout: OnTakeoutSpec,
    /// With a finite buffer, the place of a unit taken out while it restarts: `reserved` (the
    /// default, left out), it keeps one for itself as a failed unit does, so the buffer holds
    /// fewer while units are out; or `open`, it holds none. With an unlimited buffer it changes
    /// nothing.
    #[serde(default, skip_serializing_if = "PlaceWhileOutSpec::is_reserved")]
    pub place_while_out: PlaceWhileOutSpec,
}

/// With a finite buffer, the place of a unit a health check takes out, while it restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlaceWhileOutSpec {
    /// It keeps a place for itself (one free at the takeout, once the job under way is back in
    /// the queue) until its restart ends, as a failed unit does.
    #[default]
    Reserved,
    /// It holds none, as a unit off shift.
    Open,
}

impl PlaceWhileOutSpec {
    fn is_reserved(&self) -> bool {
        *self == PlaceWhileOutSpec::Reserved
    }
}

/// What happens to a job under way on a unit a health check takes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OnTakeoutSpec {
    /// Back to the head of the queue at once, for any unit to start afresh.
    #[default]
    Retry,
    /// It waits on the unit and resumes with what was left after the restart.
    Resume,
}

impl OnTakeoutSpec {
    fn is_retry(&self) -> bool {
        *self == OnTakeoutSpec::Retry
    }
}

impl From<&HealthCheck> for HealthCheckSpec {
    fn from(h: &HealthCheck) -> Self {
        HealthCheckSpec {
            interval: (&h.interval).into(),
            limit: h.limit.is_finite().then_some(h.limit),
            restart: (&h.restart).into(),
            keep: h.keep,
            on_takeout: match h.on_takeout {
                OnTakeout::Retry => OnTakeoutSpec::Retry,
                OnTakeout::Resume => OnTakeoutSpec::Resume,
            },
            place_while_out: match h.place_while_out {
                PlaceWhileOut::Reserved => PlaceWhileOutSpec::Reserved,
                PlaceWhileOut::Open => PlaceWhileOutSpec::Open,
            },
        }
    }
}

impl From<HealthCheckSpec> for HealthCheck {
    fn from(h: HealthCheckSpec) -> Self {
        HealthCheck {
            interval: h.interval.into(),
            limit: h.limit.unwrap_or(f64::INFINITY),
            restart: h.restart.into(),
            keep: h.keep,
            on_takeout: match h.on_takeout {
                OnTakeoutSpec::Retry => OnTakeout::Retry,
                OnTakeoutSpec::Resume => OnTakeout::Resume,
            },
            place_while_out: match h.place_while_out {
                PlaceWhileOutSpec::Reserved => PlaceWhileOut::Reserved,
                PlaceWhileOutSpec::Open => PlaceWhileOut::Open,
            },
        }
    }
}

impl From<PatienceSpec> for Patience {
    fn from(p: PatienceSpec) -> Self {
        Patience {
            time: p.time.into(),
            until: match p.until {
                PatienceUntilSpec::Start => PatienceUntil::Start,
                PatienceUntilSpec::Done => PatienceUntil::Done,
            },
        }
    }
}

/// The customer demand a sink serves, for takt: `quantity` parts each `per` period, with the
/// time available that of the measured window, or of `calendar` only (needed per shift: each
/// active period of it is a shift).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DemandSpec {
    pub quantity: f64,
    pub per: DemandPerSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<String>,
}

/// The period a demand is for: the whole measured window, each day, each week, or each shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DemandPerSpec {
    Run,
    Day,
    Week,
    Shift,
}

impl From<&Demand> for DemandSpec {
    fn from(d: &Demand) -> Self {
        DemandSpec {
            quantity: d.quantity,
            per: match d.per {
                DemandPer::Run => DemandPerSpec::Run,
                DemandPer::Day => DemandPerSpec::Day,
                DemandPer::Week => DemandPerSpec::Week,
                DemandPer::Shift => DemandPerSpec::Shift,
            },
            calendar: d.calendar.as_ref().map(ToString::to_string),
        }
    }
}

impl From<DemandSpec> for Demand {
    fn from(d: DemandSpec) -> Self {
        Demand {
            quantity: d.quantity,
            per: match d.per {
                DemandPerSpec::Run => DemandPer::Run,
                DemandPerSpec::Day => DemandPer::Day,
                DemandPerSpec::Week => DemandPer::Week,
                DemandPerSpec::Shift => DemandPer::Shift,
            },
            calendar: d.calendar.as_deref().map(ElementId::unchecked),
        }
    }
}

/// A sink that takes scrap: each entity reaching it counts against `charged_to`, a server (an
/// inspection rejecting work done upstream), or, left out, the last server it finished at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScrapSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charged_to: Option<String>,
}

/// How to run a model. `duration` excludes the warm-up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExperimentDoc {
    pub time_unit: TimeUnitSpec,
    pub warmup: f64,
    pub duration: f64,
    pub replications: u32,
    pub seed: u64,
    /// The date and time model time 0 stands for, which places the calendars; left out when
    /// not set (then midnight of the day the earliest calendar event starts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<String>")]
    pub start: Option<Stamp>,
    /// What ends each replication early; left out when it runs for its duration. The warm-up
    /// plus the duration is then its time limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<StopSpec>,
}

impl From<&Experiment> for ExperimentDoc {
    fn from(x: &Experiment) -> Self {
        Self {
            time_unit: x.time_unit.into(),
            warmup: x.warmup,
            duration: x.duration,
            replications: x.replications,
            seed: x.seed,
            start: x.start.map(Stamp),
            stop: x.stop.as_ref().map(Into::into),
        }
    }
}

impl From<ExperimentDoc> for Experiment {
    fn from(x: ExperimentDoc) -> Self {
        Self {
            time_unit: x.time_unit.into(),
            warmup: x.warmup,
            duration: x.duration,
            replications: x.replications,
            seed: x.seed,
            start: x.start.map(|s| s.0),
            stop: x.stop.map(Into::into),
        }
    }
}

fn default_experiment() -> ExperimentDoc {
    (&Experiment::default()).into()
}

/// A whole model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelDoc {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub elements: Vec<ElementDoc>,
    #[serde(default)]
    pub connections: Vec<ConnectionDoc>,
    /// Pools of units that tasks share; left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<ResourceDoc>,
    /// Calendars that values can be valid for; left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<CalendarDoc>,
    #[serde(default = "default_experiment")]
    pub experiment: ExperimentDoc,
}

impl From<&Model> for ModelDoc {
    fn from(m: &Model) -> Self {
        Self {
            name: m.name.clone(),
            elements: m
                .elements
                .iter()
                .map(|e| ElementDoc {
                    id: e.id.to_string(),
                    name: e.name.clone(),
                    kind: (&e.kind).into(),
                    properties: e.properties.iter().map(Into::into).collect(),
                    bounds: m.layout.get(&e.id).copied().map(Into::into),
                    estimate: m.estimates.get(&e.id).map(Into::into),
                })
                .collect(),
            connections: m
                .connections
                .iter()
                .map(|c| ConnectionDoc {
                    id: c.id.to_string(),
                    from: c.from.to_string(),
                    to: c.to.to_string(),
                    probability: c.probability,
                    condition: c.condition.clone(),
                    quantity: c.quantity,
                    rework: c.rework,
                    abandoned: c.abandoned,
                })
                .collect(),
            resources: m.resources.iter().map(Into::into).collect(),
            calendars: m.calendars.iter().map(Into::into).collect(),
            experiment: (&m.experiment).into(),
        }
    }
}

impl From<ModelDoc> for Model {
    /// Ids are kept as written; [`Model::validate`] reports invalid ones.
    fn from(m: ModelDoc) -> Self {
        let layout = m
            .elements
            .iter()
            .filter_map(|e| Some((ElementId::unchecked(&e.id), e.bounds?.into())))
            .collect();
        // An estimate without exactly one of max and p95 cannot be kept (estimate.set refuses
        // one).
        let estimates = m
            .elements
            .iter()
            .filter_map(|e| {
                let x = e.estimate.as_ref()?.to_estimate().ok()?;
                Some((ElementId::unchecked(&e.id), x))
            })
            .collect();
        Self {
            layout,
            estimates,
            name: m.name,
            elements: m
                .elements
                .into_iter()
                .map(|e| Element {
                    id: ElementId::unchecked(&e.id),
                    name: e.name,
                    kind: e.kind.into(),
                    properties: sorted_properties(e.properties.into_iter().map(Into::into)),
                })
                .collect(),
            connections: m
                .connections
                .into_iter()
                .map(|c| Connection {
                    id: ElementId::unchecked(&c.id),
                    from: ElementId::unchecked(&c.from),
                    to: ElementId::unchecked(&c.to),
                    probability: c.probability,
                    condition: c.condition,
                    quantity: c.quantity,
                    rework: c.rework,
                    abandoned: c.abandoned,
                })
                .collect(),
            resources: m.resources.into_iter().map(Into::into).collect(),
            calendars: m.calendars.into_iter().map(Into::into).collect(),
            experiment: m.experiment.into(),
        }
    }
}

/// A model as JSON.
pub fn model(m: &Model) -> Value {
    serde_json::to_value(ModelDoc::from(m)).unwrap_or(Value::Null)
}

/// Diagnostics as JSON: `severity`, `at`, a stable `code` and a `message`.
pub fn diagnostics(ds: &[Diagnostic]) -> Value {
    ds.iter()
        .map(|d| {
            json!({
                "severity": match d.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                },
                "at": d.at.as_ref().map(ElementId::as_str),
                "code": d.problem.code(),
                "message": d.to_string(),
            })
        })
        .collect()
}

/// An experiment's results as JSON: per element, each metric's mean, 95% half-width, standard
/// deviation and number of replications, and, where the replications split into two outcomes
/// ([`stochcraft_stats::split::two_outcomes`]), the two groups as `split`. Results whose splits
/// put the same replications together share a `group` (from 1), and `splits` lists each group
/// once: its replications on either side (`sizes`, the lead's low side first) and its results
/// in table order, the lead first.
pub fn results(model: &Model, replications: u64, seed: u64, summary: &ExperimentSummary) -> Value {
    let x = &model.experiment;
    // Every result whose runs split, in table order, with the side of each replication.
    let found: Vec<(usize, Metric, Split, Vec<Option<bool>>)> = summary
        .elements
        .iter()
        .enumerate()
        .flat_map(|(i, e)| {
            e.samples.iter().filter_map(move |(m, samples)| {
                let values: Vec<f64> = samples.iter().flatten().copied().collect();
                let s = split::two_outcomes(&values)?;
                Some((i, *m, s, split::sides(samples, s.cut)))
            })
        })
        .collect();
    let groups = split::group_by_runs(&found.iter().map(|f| f.3.clone()).collect::<Vec<_>>());
    let group_of = |k: usize| {
        groups
            .iter()
            .position(|g| g.members.contains(&k))
            .map(|g| g + 1)
    };
    let elements: Vec<Value> = summary
        .elements
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let metrics: serde_json::Map<String, Value> = e
                .metrics
                .iter()
                .map(|(m, s)| {
                    let mut v = json!({
                        "mean": s.mean,
                        "half_width": s.half_width,
                        "std_dev": s.std_dev,
                        "n": s.n,
                    });
                    // Only where the runs split, so other results are as before.
                    if let Some(k) = found.iter().position(|f| f.0 == i && f.1 == *m) {
                        let s = &found[k].2;
                        v["split"] = json!({
                            "cut": s.cut,
                            "low": { "n": s.low.n, "mean": s.low.mean },
                            "high": { "n": s.high.n, "mean": s.high.mean },
                            "group": group_of(k),
                        });
                    }
                    (m.name().to_owned(), v)
                })
                .collect();
            let mut v =
                json!({ "id": e.id.as_str(), "name": e.name, "kind": e.kind, "metrics": metrics });
            // The mean value the element assigned to each property; left out when it assigns
            // none, so results of models without properties are as before.
            if !e.properties.is_empty() {
                let props: serde_json::Map<String, Value> = e
                    .properties
                    .iter()
                    .map(|(name, s)| {
                        (
                            name.clone(),
                            json!({
                                "mean": s.mean,
                                "half_width": s.half_width,
                                "std_dev": s.std_dev,
                                "n": s.n,
                            }),
                        )
                    })
                    .collect();
                v["properties"] = Value::Object(props);
            }
            v
        })
        .collect();
    // Each metric's value in every replication, in replication order (null where one had
    // none): the distributions behind the means, for percentiles and `results.distribution`.
    let values: serde_json::Map<String, Value> = summary
        .elements
        .iter()
        .map(|e| {
            let per: serde_json::Map<String, Value> = e
                .samples
                .iter()
                .map(|(m, v)| (m.name().to_owned(), json!(v)))
                .collect();
            (e.id.to_string(), Value::Object(per))
        })
        .collect();
    let mut v = json!({
        "replications": replications,
        "seed": seed,
        "time_unit": TimeUnitSpec::from(x.time_unit),
        "warmup": x.warmup,
        "duration": x.duration,
        "metrics": Metric::ALL.iter().map(|m| m.name()).collect::<Vec<_>>(),
        "elements": elements,
        "values": values,
        // What each replication's statistics cover; with no stop condition, every one covers
        // the duration after the warm-up.
        "measured": {
            "spans": summary
                .measured
                .iter()
                .map(|m| json!({ "from": m.from, "to": m.to, "used": m.used }))
                .collect::<Vec<_>>(),
            "mean_duration": summary.mean_measured(),
        },
        // What the results show that the model could not: as the run record's warnings.
        "warnings": diagnostics(&stochcraft_blocks::run_warnings(model, summary)),
    });
    if let Some(stop) = &x.stop {
        let met = summary.stop_times.iter().flatten().count();
        let mut s = serde_json::to_value(StopSpec::from(stop)).unwrap_or(Value::Null);
        s["met"] = json!(met);
        // The time to the event, over the replications that met it before the time limit.
        s["time"] = summary.stopped.as_ref().map_or(Value::Null, |(_, t)| {
            json!({ "mean": t.mean, "half_width": t.half_width, "std_dev": t.std_dev, "n": t.n })
        });
        s["times"] = json!(summary.stop_times);
        v["stop"] = s;
    }
    // Left out when nothing splits, so other results are as before.
    if !groups.is_empty() {
        v["splits"] = groups
            .iter()
            .enumerate()
            .map(|(g, run)| {
                let results: Vec<Value> = run
                    .members
                    .iter()
                    .map(|k| {
                        let (i, m, _, _) = &found[*k];
                        json!([summary.elements[*i].id.as_str(), m.name()])
                    })
                    .collect();
                json!({
                    "id": g + 1,
                    "n": run.low + run.high,
                    "sizes": [run.low, run.high],
                    "results": results,
                })
            })
            .collect();
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_give_the_two_outcomes_where_the_runs_split() {
        use stochcraft_blocks::ElementSummary;
        use stochcraft_stats::Summary;
        // Units in service: about 1.7 in 9 runs, about 9.9 in 3, and a metric that does not
        // split.
        let split: Vec<f64> = (0..12)
            .map(|i| {
                if i % 4 == 3 {
                    9.9 + 0.01 * f64::from(i)
                } else {
                    1.7 - 0.01 * f64::from(i)
                }
            })
            .collect();
        let even: Vec<f64> = (0..12).map(f64::from).collect();
        // The queue splits the same runs the other way round (long where few were in
        // service), with no value in the first run; units left split other runs.
        let queue: Vec<Option<f64>> = (0..12)
            .map(|i| {
                (i > 0).then(|| {
                    if i % 4 == 3 {
                        0.5 + 0.01 * f64::from(i)
                    } else {
                        50.0 + f64::from(i)
                    }
                })
            })
            .collect();
        let left: Vec<f64> = (0..12)
            .map(|i| {
                if i < 4 {
                    100.0 + f64::from(i)
                } else {
                    10.0 + 0.1 * f64::from(i)
                }
            })
            .collect();
        let summary = |v: &[f64]| {
            Summary::of(v).unwrap_or(Summary {
                n: 0,
                mean: 0.0,
                std_dev: 0.0,
                half_width: 0.0,
            })
        };
        let queued: Vec<f64> = queue.iter().flatten().copied().collect();
        let element = ElementSummary {
            id: ElementId::unchecked("desk"),
            name: "Desk".into(),
            kind: "server",
            metrics: vec![
                (Metric::Entered, summary(&even)),
                (Metric::Left, summary(&left)),
                (Metric::MeanQueueLength, summary(&queued)),
                (Metric::MeanInService, summary(&split)),
            ],
            properties: vec![],
            samples: vec![
                (Metric::Entered, even.iter().copied().map(Some).collect()),
                (Metric::Left, left.iter().copied().map(Some).collect()),
                (Metric::MeanQueueLength, queue),
                (
                    Metric::MeanInService,
                    split.iter().copied().map(Some).chain([None]).collect(),
                ),
            ],
        };
        let s = ExperimentSummary {
            replications: 13,
            elements: vec![element],
            stopped: None,
            stop_times: vec![],
            measured: vec![],
        };
        let r = results(&Model::default(), 13, 1, &s);
        let metrics = &r["elements"][0]["metrics"];
        assert!(metrics["entered"].get("split").is_none(), "{metrics}");
        let got = &metrics["mean_in_service"]["split"];
        assert_eq!(
            (&got["low"]["n"], &got["high"]["n"]),
            (&json!(9), &json!(3))
        );
        assert!(
            got["cut"].as_f64().is_some_and(|c| c > 1.7 && c < 9.9),
            "{got}"
        );
        assert!(
            got["high"]["mean"]
                .as_f64()
                .is_some_and(|m| (m - 9.97).abs() < 1e-9),
            "{got}"
        );
        // Units left split other runs: a group of their own. The queue and the units in
        // service split the same runs: one group, led by the queue (first in table order), its
        // low side (3 runs) first, over the 12 runs either has.
        assert_eq!(metrics["left"]["split"]["group"], json!(1));
        assert_eq!(metrics["mean_queue_length"]["split"]["group"], json!(2));
        assert_eq!(got["group"], json!(2));
        assert_eq!(
            r["splits"],
            json!([
                { "id": 1, "n": 12, "sizes": [8, 4], "results": [["desk", "left"]] },
                {
                    "id": 2, "n": 12, "sizes": [3, 9],
                    "results": [["desk", "mean_queue_length"], ["desk", "mean_in_service"]],
                },
            ])
        );
        // Nothing splits: no `splits`.
        let mut calm = s.clone();
        calm.elements[0]
            .samples
            .retain(|(m, _)| *m == Metric::Entered);
        assert!(
            results(&Model::default(), 13, 1, &calm)
                .get("splits")
                .is_none()
        );
    }

    #[test]
    fn every_distribution_round_trips() {
        let all = [
            Dist::Constant(2.0),
            Dist::Exponential { mean: 1.5 },
            Dist::Uniform { min: 1.0, max: 2.0 },
            Dist::Triangular {
                min: 1.0,
                mode: 2.0,
                max: 4.0,
            },
            Dist::Normal {
                mean: 3.0,
                std_dev: 0.5,
            },
            Dist::TruncatedNormal {
                mean: 3.0,
                std_dev: 0.5,
                min: 2.0,
                max: 4.0,
            },
            Dist::LogNormal {
                mean: 3.0,
                std_dev: 1.0,
            },
            Dist::Gamma {
                shape: 2.0,
                scale: 0.5,
            },
            Dist::Beta {
                shape: 2.0,
                scale: 3.0,
            },
            Dist::Erlang { mean: 4.0, k: 3 },
            Dist::Weibull {
                shape: 1.5,
                scale: 2.0,
            },
            Dist::Poisson { mean: 4.0 },
            Dist::Binomial {
                probability: 0.3,
                trials: 10,
            },
            Dist::User {
                points: vec![(0.25, 1.0), (0.75, 2.0)],
                discrete: true,
            },
        ];
        for d in all {
            let v = serde_json::to_value(DistSpec::from(&d)).unwrap_or(Value::Null);
            let back: Option<DistSpec> = serde_json::from_value(v.clone()).ok();
            assert_eq!(back.map(Dist::from), Some(d), "{v}");
        }
        let e: Result<DistSpec, _> =
            serde_json::from_value(json!({"type": "exponential", "mean": 1, "rate": 2}));
        assert!(e.is_err(), "unknown fields are refused");
    }

    #[test]
    fn kinds_take_defaults() {
        let k: Option<KindSpec> = serde_json::from_value(json!({"kind": "server"})).ok();
        assert_eq!(
            k,
            Some(KindSpec::Server {
                processing_by: None,
                capacity: 1,
                service: DistSpec::Exponential { mean: 1.0 },
                queue_capacity: None,
                breakdown: None,
                transfer: None,
                transfer_holds_sender: true,
                available: true,
                availability: Vec::new(),
                power: None,
                setup: None,
                planned_run_time: None,
                patience: None,
                health_check: None,
                shifts: Vec::new(),
                on_shift_end: OnShiftEndSpec::Finish,
            })
        );
    }
}
