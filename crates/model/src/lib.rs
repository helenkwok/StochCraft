//! A StochCraft model in memory.
//!
//! This is not a file format. `stochcraft-io` reads and writes models as BPSim on BPMN or XPDL
//! (see ROADMAP.md); this crate is what those files map to, what the engine's commands change and
//! what `stochcraft-blocks` turns into a simulation.
//!
//! - Every element and connection has an [`ElementId`]: the id of its BPMN or XPDL element, kept
//!   stable so files diff cleanly, connections survive edits and agents can name things.
//! - Times are `f64` in the experiment's [`TimeUnit`] (BPSim's `baseTimeUnit`, minutes by default).
//! - [`Model::validate`] reports every problem at once as [`Diagnostic`]s, so a person or an agent
//!   can fix them all in one pass.
//! - Entities carry BPSim properties: each element may assign [`Property`] values to an entity
//!   as it enters, from a fixed value, a distribution or an [`expr`] expression.
//! - Gateways route entities: an exclusive gateway sends each one down one of its outgoing
//!   [`Connection`]s, by the connections' probabilities or by the first condition that holds; a
//!   parallel gateway copies each one down every outgoing connection and, with several incoming
//!   connections, waits for one entity on each before going on. Any element may have several
//!   incoming connections (a merge).

pub mod calendar;
pub mod expr;

pub use calendar::{Calendar, DateTime, Timed};

use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub use stochcraft_random::Dist;

/// The id of a model element or connection. Ids follow the XML `NCName` rule that BPMN and XPDL
/// ids obey: a letter or `_` first, then letters, digits, `.`, `-` or `_`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementId(String);

impl ElementId {
    /// The id, if `s` is a valid one.
    pub fn new(s: &str) -> Option<Self> {
        Self::is_valid(s).then(|| Self(s.to_owned()))
    }

    /// The id as given, valid or not. For tools that build models from names and let
    /// [`Model::validate`] report the invalid ones, instead of failing at the first.
    pub fn unchecked(s: &str) -> Self {
        Self(s.to_owned())
    }

    pub fn is_valid(s: &str) -> bool {
        let mut chars = s.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        (first.is_alphabetic() || first == '_')
            && chars.all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ElementId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// BPSim's time units (`TimeUnit`, section 6.1.4 of the specification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeUnit {
    Milliseconds,
    Seconds,
    #[default]
    Minutes,
    Hours,
    Days,
    /// BPSim's `year`, taken as 365 days.
    Years,
}

impl TimeUnit {
    /// The unit's BPSim spelling.
    pub fn bpsim_name(self) -> &'static str {
        match self {
            TimeUnit::Milliseconds => "ms",
            TimeUnit::Seconds => "s",
            TimeUnit::Minutes => "min",
            TimeUnit::Hours => "hour",
            TimeUnit::Days => "day",
            TimeUnit::Years => "year",
        }
    }

    pub fn from_bpsim_name(s: &str) -> Option<Self> {
        Some(match s {
            "ms" => TimeUnit::Milliseconds,
            "s" => TimeUnit::Seconds,
            "min" => TimeUnit::Minutes,
            "hour" => TimeUnit::Hours,
            "day" => TimeUnit::Days,
            "year" => TimeUnit::Years,
            _ => return None,
        })
    }

    /// Seconds in one unit.
    pub fn seconds(self) -> f64 {
        match self {
            TimeUnit::Milliseconds => 0.001,
            TimeUnit::Seconds => 1.0,
            TimeUnit::Minutes => 60.0,
            TimeUnit::Hours => 3_600.0,
            TimeUnit::Days => 86_400.0,
            TimeUnit::Years => 365.0 * 86_400.0,
        }
    }

    /// `value` in `from` units, expressed in `self` units.
    pub fn convert(self, value: f64, from: TimeUnit) -> f64 {
        value * from.seconds() / self.seconds()
    }
}

/// What an element does. M2 adds resources and the rest (ROADMAP.md).
#[derive(Debug, Clone, PartialEq)]
pub enum ElementKind {
    /// Creates entities (a BPMN start event): time between arrivals, and an optional limit
    /// (BPSim `interTriggerTimer` and `triggerCount`).
    Source {
        interarrival: Dist,
        limit: Option<u64>,
        /// Times between arrivals while a calendar is active (BPSim `validFor`), the first
        /// active one applying; `interarrival` applies outside them. When the value changes,
        /// the next arrival is drawn afresh from the new one at that moment.
        timed: Vec<Timed<Dist>>,
        /// No arrivals outside the calendars of `timed` (a file with no value without
        /// `validFor`); `interarrival` is then unused.
        idle_outside: bool,
    },
    /// `capacity` identical units serving one entity each, first come first served (a task with
    /// a resource): BPSim `processingTime`.
    Server {
        capacity: u32,
        service: Dist,
        /// A finite buffer: at most this many entities wait for a unit (not counting those in
        /// service); `None` is unlimited. 0 means no waiting room: an entity gets in only when a
        /// unit is free. When the buffer is full, an element sending an entity here waits
        /// holding it (a server keeps its unit busy: blocking after service), except a source,
        /// whose arrival is lost. A StochCraft vendor extension (`stochcraft:queueCapacity`).
        queue_capacity: Option<u32>,
        /// Each unit fails now and then and is repaired (a StochCraft vendor extension,
        /// `stochcraft:breakdown`, since BPSim has none); `None`: units never fail.
        breakdown: Option<Breakdown>,
    },
    /// A task whose units come from shared resource pools ([`Model::resources`]): each entity
    /// takes what the first of `needs` that can be met right now asks for, all at once, holds it
    /// for its service (and while blocked after it), then gives it back. Entities wait here in
    /// one queue per task; when units come free, waiting entities across all tasks get them in
    /// order of their task's `priority` (higher first), then of arrival. An entity that cannot be
    /// served now does not hold back one behind it that can. BPSim's `selection` with
    /// `getResource`, `getResourceByRoles` and `orResource`, and `priority`.
    SharedTask {
        service: Dist,
        /// The alternatives, tried in order (BPSim `orResource`); at least one.
        needs: Vec<Need>,
        /// Who gets freed units first, higher first (BPSim `priority`; default 0).
        priority: f64,
    },
    /// Holds each entity for a time with no capacity limit (a task with no resource).
    Delay { duration: Dist },
    /// Removes entities (a BPMN end event).
    Sink,
    /// Sends each entity down one outgoing connection, at once (a BPMN exclusive gateway). If any
    /// outgoing connection has a condition, the first in model order whose condition holds is
    /// taken, else `default`, else the entity leaves the model here; otherwise one is drawn by
    /// the connections' probabilities (those without one share evenly what the others leave).
    ExclusiveGateway {
        /// The connection taken when no condition holds (BPMN's `default` flow).
        default: Option<ElementId>,
    },
    /// Copies each entity down every outgoing connection, at once (a BPMN parallel gateway).
    /// With several incoming connections it first waits for an entity on each of them (a join)
    /// and sends one on: the one from the first incoming connection, in model order, with any
    /// property it lacks taken from the others in that order.
    ParallelGateway,
}

/// How a server's units fail: each unit independently, in elapsed time (busy or idle), after a
/// time drawn from `time_to_failure` since it was last repaired; then it is repaired for a time
/// drawn from `repair`.
#[derive(Debug, Clone, PartialEq)]
pub struct Breakdown {
    pub time_to_failure: Dist,
    pub repair: Dist,
    /// A failure interrupts the job under way, which resumes on the same unit after the repair
    /// with the service time it had left (BPSim `interruptible`); otherwise the unit fails only
    /// once its job is done, and the repair starts then.
    pub interruptible: bool,
}

/// What a [`ElementKind::SharedTask`] needs from the pools, for one entity.
#[derive(Debug, Clone, PartialEq)]
pub enum Need {
    /// `quantity` units of this pool (BPSim `getResource(id, qty)`).
    Resource { id: ElementId, quantity: u32 },
    /// `quantity` units, taken from the pools that have every one of `roles`, in model order
    /// (BPSim `getResourceByRoles([roles], qty)`).
    Roles { roles: Vec<String>, quantity: u32 },
}

impl Need {
    pub fn quantity(&self) -> u32 {
        match self {
            Need::Resource { quantity, .. } | Need::Roles { quantity, .. } => *quantity,
        }
    }
}

/// A pool of identical units that tasks share (a BPMN `resource`, an XPDL `Participant` of type
/// RESOURCE): BPSim `quantity` and `role`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourcePool {
    pub id: ElementId,
    pub name: String,
    /// Units outside every shift of `shifts`.
    pub quantity: u32,
    pub roles: Vec<String>,
    /// Units while a calendar is active (BPSim `quantity` values `validFor` a calendar), the
    /// first active one applying. When the units go down, those in use finish their job first.
    pub shifts: Vec<Timed<u32>>,
    /// Whether the units are available outside the calendars of `availability` (BPSim
    /// `availability`, default true).
    pub available: bool,
    /// Availability while a calendar is active, the first active one applying.
    pub availability: Vec<Timed<bool>>,
}

impl ResourcePool {
    /// A pool of `quantity` units with roles and no shifts.
    pub fn new(id: ElementId, name: impl Into<String>, quantity: u32, roles: Vec<String>) -> Self {
        Self {
            id,
            name: name.into(),
            quantity,
            roles,
            shifts: Vec::new(),
            available: true,
            availability: Vec::new(),
        }
    }

    /// The most units the pool ever has.
    pub fn most_units(&self) -> u32 {
        let any_available = self.available || self.availability.iter().any(|a| a.value);
        if !any_available {
            return 0;
        }
        self.shifts
            .iter()
            .map(|s| s.value)
            .fold(self.quantity, u32::max)
    }

    /// The calendars the pool's values refer to.
    pub fn calendars(&self) -> impl Iterator<Item = &ElementId> {
        self.shifts
            .iter()
            .map(|t| &t.calendar)
            .chain(self.availability.iter().map(|t| &t.calendar))
    }

    /// Whether the pool plays every one of `roles`.
    pub fn has_roles(&self, roles: &[String]) -> bool {
        roles.iter().all(|r| self.roles.contains(r))
    }
}

impl ElementKind {
    pub fn name(&self) -> &'static str {
        match self {
            ElementKind::Source { .. } => "source",
            ElementKind::Server { .. } => "server",
            ElementKind::SharedTask { .. } => "shared_task",
            ElementKind::Delay { .. } => "delay",
            ElementKind::Sink => "sink",
            ElementKind::ExclusiveGateway { .. } => "exclusive_gateway",
            ElementKind::ParallelGateway => "parallel_gateway",
        }
    }

    /// Whether this is a gateway: it routes entities at once and holds none.
    pub fn is_gateway(&self) -> bool {
        matches!(
            self,
            ElementKind::ExclusiveGateway { .. } | ElementKind::ParallelGateway
        )
    }

    fn distributions(&self) -> Vec<(&'static str, &Dist)> {
        match self {
            ElementKind::Source {
                interarrival,
                timed,
                ..
            } => std::iter::once(("interarrival", interarrival))
                .chain(timed.iter().map(|t| ("interarrival", &t.value)))
                .collect(),
            ElementKind::Server {
                service, breakdown, ..
            } => {
                let mut d = vec![("service", service)];
                if let Some(b) = breakdown {
                    d.push(("time_to_failure", &b.time_to_failure));
                    d.push(("repair", &b.repair));
                }
                d
            }
            ElementKind::SharedTask { service, .. } => vec![("service", service)],
            ElementKind::Delay { duration } => vec![("duration", duration)],
            ElementKind::Sink
            | ElementKind::ExclusiveGateway { .. }
            | ElementKind::ParallelGateway => vec![],
        }
    }
}

/// BPSim's property types (`PropertyType`, section 6.1.5 of the specification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PropertyType {
    String,
    Boolean,
    /// A whole number (64 bits).
    Long,
    Double,
    /// A length of time, in the experiment's time unit.
    Duration,
    /// A date and time, kept as written.
    DateTime,
}

impl PropertyType {
    pub const ALL: [PropertyType; 6] = [
        PropertyType::String,
        PropertyType::Boolean,
        PropertyType::Long,
        PropertyType::Double,
        PropertyType::Duration,
        PropertyType::DateTime,
    ];

    /// The type's BPSim spelling.
    pub fn bpsim_name(self) -> &'static str {
        match self {
            PropertyType::String => "string",
            PropertyType::Boolean => "boolean",
            PropertyType::Long => "long",
            PropertyType::Double => "double",
            PropertyType::Duration => "duration",
            PropertyType::DateTime => "dateTime",
        }
    }

    pub fn from_bpsim_name(s: &str) -> Option<Self> {
        PropertyType::ALL.into_iter().find(|t| t.bpsim_name() == s)
    }

    /// Whether values of this type are numbers.
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            PropertyType::Long | PropertyType::Double | PropertyType::Duration
        )
    }
}

/// What a property is set to when an entity enters the element.
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// A fixed number (a duration in the experiment's time unit).
    Number(f64),
    Boolean(bool),
    /// Fixed text (a string or a date and time).
    Text(String),
    /// A value drawn for each entity.
    Random(Dist),
    /// A BPSim expression ([`expr`]), evaluated with the entity's properties as they were when it
    /// arrived.
    Expression(String),
}

impl PropertyValue {
    /// A short name for messages: "a number", "a distribution"…
    pub fn describe(&self) -> &'static str {
        match self {
            PropertyValue::Number(_) => "a number",
            PropertyValue::Boolean(_) => "true or false",
            PropertyValue::Text(_) => "text",
            PropertyValue::Random(_) => "a distribution",
            PropertyValue::Expression(_) => "an expression",
        }
    }
}

/// A property an element assigns to every entity that enters it (BPSim `PropertyParameters`).
///
/// All of an element's assignments are made together: each value is computed from the entity's
/// properties as they were when it arrived, so their order does not matter.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    /// The declared type; without one, the property takes the type declared elsewhere in the
    /// model, or else the type of its first value ([`Model::property_types`]).
    pub ty: Option<PropertyType>,
    pub value: PropertyValue,
}

/// A model element.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub id: ElementId,
    pub name: String,
    pub kind: ElementKind,
    /// Properties assigned to each entity that enters (for a source: each entity it creates).
    pub properties: Vec<Property>,
}

impl Element {
    /// An element with no properties.
    pub fn new(id: ElementId, name: impl Into<String>, kind: ElementKind) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            properties: Vec::new(),
        }
    }

    /// The property this element assigns under `name`.
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }
}

/// A connection from one element to another (a BPMN sequence flow).
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub id: ElementId,
    pub from: ElementId,
    pub to: ElementId,
    /// Out of an exclusive gateway: the chance an entity takes this connection (BPSim
    /// `Probability`).
    pub probability: Option<f64>,
    /// Out of an exclusive gateway: a BPSim expression ([`expr`]) that must hold for an entity
    /// to take this connection (BPSim `Condition`).
    pub condition: Option<String>,
}

impl Connection {
    /// A connection with no probability or condition.
    pub fn new(id: ElementId, from: ElementId, to: ElementId) -> Self {
        Self {
            id,
            from,
            to,
            probability: None,
            condition: None,
        }
    }
}

/// How to run the model (a BPSim scenario's parameters).
#[derive(Debug, Clone, PartialEq)]
pub struct Experiment {
    pub time_unit: TimeUnit,
    /// Simulated time before statistics are collected.
    pub warmup: f64,
    /// Simulated time over which statistics are collected, after the warm-up.
    pub duration: f64,
    pub replications: u32,
    pub seed: u64,
    /// The date and time model time 0 stands for (BPSim scenario `start`), which places the
    /// calendars; without one, midnight of the day the earliest calendar event starts.
    pub start: Option<DateTime>,
}

impl Default for Experiment {
    fn default() -> Self {
        Self {
            time_unit: TimeUnit::Minutes,
            warmup: 0.0,
            duration: 480.0,
            replications: 10,
            seed: 1,
            start: None,
        }
    }
}

/// A whole model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    pub name: String,
    pub elements: Vec<Element>,
    pub connections: Vec<Connection>,
    /// Pools of units that tasks share ([`ElementKind::SharedTask`]).
    pub resources: Vec<ResourcePool>,
    /// Calendars that parameter values can be valid for (shifts, opening hours).
    pub calendars: Vec<Calendar>,
    pub experiment: Experiment,
}

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The model runs, but probably not as intended.
    Warning,
    /// The model cannot run.
    Error,
}

/// What a diagnostic is about. Stable, so the UI, the CLI and agents can act on it.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    InvalidId {
        id: String,
    },
    DuplicateId {
        id: ElementId,
    },
    UnknownElement {
        connection: ElementId,
        missing: ElementId,
    },
    IntoSource {
        connection: ElementId,
    },
    OutOfSink {
        connection: ElementId,
    },
    SelfLoop {
        connection: ElementId,
    },
    NoOutput,
    SeveralOutputs {
        count: usize,
    },
    BadDistribution {
        parameter: &'static str,
        error: stochcraft_random::DistError,
    },
    ZeroCapacity,
    /// A source with no limit whose time between arrivals is always 0: the run would make
    /// entities at one instant forever and never end.
    NoTimeBetweenArrivals,
    NoSource,
    NoSink,
    Unreachable,
    NoPathToSink,
    BadExperiment {
        field: &'static str,
        requirement: &'static str,
    },
    /// A property with no name, or one an element assigns twice.
    BadProperty {
        name: String,
        reason: &'static str,
    },
    /// Two declarations of a property with different types.
    PropertyTypeConflict {
        name: String,
        first: PropertyType,
        second: PropertyType,
    },
    /// A fixed value or distribution that a property of this type cannot hold.
    PropertyValueType {
        name: String,
        ty: PropertyType,
        value: &'static str,
    },
    /// An expression that cannot be used, with where and why.
    BadExpression {
        name: String,
        expression: String,
        error: expr::ExprError,
    },
    /// An expression reads a property that no element sets.
    UnknownProperty {
        name: String,
    },
    /// A probability outside 0 to 1.
    BadProbability {
        value: f64,
    },
    /// An exclusive gateway's probabilities do not add up to 1; they are used in proportion.
    ProbabilitySum {
        sum: f64,
    },
    /// A condition that cannot be used, with where and why.
    BadCondition {
        expression: String,
        error: expr::ExprError,
    },
    /// A probability or condition where it has no effect, and why.
    RoutingIgnored {
        what: &'static str,
        reason: &'static str,
    },
    /// An exclusive gateway's default connection is not one of its outgoing connections.
    BadDefault {
        connection: ElementId,
    },
    /// An exclusive gateway routes by condition and has no default connection.
    NoDefault,
    /// A join waits on an incoming connection that no entity can reach.
    JoinNeverCompletes {
        connection: ElementId,
    },
    /// A server with a finite buffer is fed through a gateway: while the buffer is full the
    /// gateway waits, holding up every entity behind it, and those wait before the gateway with
    /// no limit (rather than blocking the element that sent them).
    BufferBehindGateway {
        gateway: ElementId,
    },
    /// Servers with finite buffers that feed each other in a loop: when every buffer in the loop
    /// is full, each waits for the next and the loop deadlocks.
    BlockingCycle {
        through: Vec<ElementId>,
    },
    /// A resource pool with no units.
    EmptyPool,
    /// A task with shared resources that names nothing it needs.
    NoNeeds,
    /// A task needs a pool that does not exist.
    UnknownResource {
        id: ElementId,
    },
    /// A task needs units with roles that no pool plays.
    NoPoolWithRoles {
        roles: Vec<String>,
    },
    /// A task needs more units at once than the pools it can use have: that alternative can
    /// never be met (an error when no alternative can be).
    NeedNeverMet {
        needed: u32,
        available: u32,
    },
    /// A task's priority is not a finite number.
    BadPriority {
        value: f64,
    },
    /// A pool no task uses.
    UnusedPool,
    /// A value is valid for a calendar that does not exist.
    UnknownCalendar {
        id: ElementId,
    },
    /// A calendar with no events is never active.
    EmptyCalendar,
    /// Values valid for calendars on a block that does not use them.
    TimedIgnored,
}

impl Problem {
    /// A stable snake_case name for files, the command line and agents.
    pub fn code(&self) -> &'static str {
        match self {
            Problem::InvalidId { .. } => "invalid_id",
            Problem::DuplicateId { .. } => "duplicate_id",
            Problem::UnknownElement { .. } => "unknown_element",
            Problem::IntoSource { .. } => "into_source",
            Problem::OutOfSink { .. } => "out_of_sink",
            Problem::SelfLoop { .. } => "self_loop",
            Problem::NoOutput => "no_output",
            Problem::SeveralOutputs { .. } => "several_outputs",
            Problem::BadDistribution { .. } => "bad_distribution",
            Problem::ZeroCapacity => "zero_capacity",
            Problem::NoTimeBetweenArrivals => "no_time_between_arrivals",
            Problem::NoSource => "no_source",
            Problem::NoSink => "no_sink",
            Problem::Unreachable => "unreachable",
            Problem::NoPathToSink => "no_path_to_sink",
            Problem::BadExperiment { .. } => "bad_experiment",
            Problem::BadProperty { .. } => "bad_property",
            Problem::PropertyTypeConflict { .. } => "property_type_conflict",
            Problem::PropertyValueType { .. } => "property_value_type",
            Problem::BadExpression { .. } => "bad_expression",
            Problem::UnknownProperty { .. } => "unknown_property",
            Problem::BadProbability { .. } => "bad_probability",
            Problem::ProbabilitySum { .. } => "probability_sum",
            Problem::BadCondition { .. } => "bad_condition",
            Problem::RoutingIgnored { .. } => "routing_ignored",
            Problem::BadDefault { .. } => "bad_default",
            Problem::NoDefault => "no_default",
            Problem::JoinNeverCompletes { .. } => "join_never_completes",
            Problem::BufferBehindGateway { .. } => "buffer_behind_gateway",
            Problem::BlockingCycle { .. } => "blocking_cycle",
            Problem::EmptyPool => "empty_pool",
            Problem::NoNeeds => "no_needs",
            Problem::UnknownResource { .. } => "unknown_resource",
            Problem::NoPoolWithRoles { .. } => "no_pool_with_roles",
            Problem::NeedNeverMet { .. } => "need_never_met",
            Problem::BadPriority { .. } => "bad_priority",
            Problem::UnusedPool => "unused_pool",
            Problem::UnknownCalendar { .. } => "unknown_calendar",
            Problem::EmptyCalendar => "empty_calendar",
            Problem::TimedIgnored => "timed_ignored",
        }
    }
}

/// One problem found by [`Model::validate`].
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// The element or connection it concerns, if any.
    pub at: Option<ElementId>,
    pub problem: Problem,
}

impl Diagnostic {
    fn error(at: Option<&ElementId>, problem: Problem) -> Self {
        Self {
            severity: Severity::Error,
            at: at.cloned(),
            problem,
        }
    }

    fn warning(at: Option<&ElementId>, problem: Problem) -> Self {
        Self {
            severity: Severity::Warning,
            at: at.cloned(),
            problem,
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{level}")?;
        if let Some(at) = &self.at {
            write!(f, " at `{at}`")?;
        }
        write!(f, ": ")?;
        match &self.problem {
            Problem::InvalidId { id } => write!(
                f,
                "`{id}` is not a valid id (a letter or _ first, then letters, digits, . - _)"
            ),
            Problem::DuplicateId { id } => write!(f, "the id `{id}` is used more than once"),
            Problem::UnknownElement { missing, .. } => {
                write!(f, "connects to `{missing}`, which does not exist")
            }
            Problem::IntoSource { .. } => write!(f, "connects into a source, which takes no input"),
            Problem::OutOfSink { .. } => write!(f, "leaves a sink, which has no output"),
            Problem::SelfLoop { .. } => write!(f, "connects an element to itself"),
            Problem::NoOutput => write!(f, "has nowhere to send entities"),
            Problem::SeveralOutputs { count } => {
                write!(
                    f,
                    "has {count} outgoing connections; choosing between them needs a gateway"
                )
            }
            Problem::BadDistribution { parameter, error } => write!(f, "{parameter}: {error}"),
            Problem::ZeroCapacity => write!(f, "a server needs at least 1 unit"),
            Problem::NoTimeBetweenArrivals => write!(
                f,
                "the time between arrivals is always 0, so the run would never end; give it a positive value or limit the arrivals"
            ),
            Problem::NoSource => write!(f, "the model has no source, so nothing will happen"),
            Problem::NoSink => write!(f, "the model has no sink, so no time in system is measured"),
            Problem::Unreachable => write!(f, "no entity can reach this element from a source"),
            Problem::NoPathToSink => write!(f, "entities here can never reach a sink"),
            Problem::BadExperiment { field, requirement } => {
                write!(f, "experiment {field} must be {requirement}")
            }
            Problem::BadProperty { name, reason } => write!(f, "property `{name}` {reason}"),
            Problem::PropertyTypeConflict {
                name,
                first,
                second,
            } => write!(
                f,
                "property `{name}` is declared as {} here and as {} elsewhere",
                second.bpsim_name(),
                first.bpsim_name()
            ),
            Problem::PropertyValueType { name, ty, value } => write!(
                f,
                "property `{name}` is a {}, which cannot hold {value}",
                ty.bpsim_name()
            ),
            Problem::BadExpression {
                name,
                expression,
                error,
            } => write!(f, "property `{name}`: `{expression}`: {error}"),
            Problem::UnknownProperty { name } => write!(
                f,
                "reads property `{name}`, which no element sets (it would read as empty)"
            ),
            Problem::BadProbability { value } => {
                write!(f, "probability {value} must be between 0 and 1")
            }
            Problem::ProbabilitySum { sum } => write!(
                f,
                "the outgoing probabilities add up to {sum}, not 1; they are used in proportion"
            ),
            Problem::BadCondition { expression, error } => {
                write!(f, "condition `{expression}`: {error}")
            }
            Problem::RoutingIgnored { what, reason } => write!(f, "{what} is ignored: {reason}"),
            Problem::BadDefault { connection } => write!(
                f,
                "the default connection `{connection}` is not one of this gateway's outgoing connections"
            ),
            Problem::NoDefault => write!(
                f,
                "routes by condition with no default connection: an entity for which no condition holds leaves the model here"
            ),
            Problem::JoinNeverCompletes { connection } => write!(
                f,
                "waits for an entity on `{connection}`, which none can reach, so it never sends one on"
            ),
            Problem::BufferBehindGateway { gateway } => write!(
                f,
                "has a finite buffer fed through gateway `{gateway}`: while the buffer is full the gateway waits and every entity behind it queues before the gateway, without limit"
            ),
            Problem::BlockingCycle { through } => {
                let names: Vec<String> = through.iter().map(|i| format!("`{i}`")).collect();
                write!(
                    f,
                    "finite buffers feed each other in a loop ({}); when all of them are full the loop deadlocks",
                    names.join(" → ")
                )
            }
            Problem::EmptyPool => write!(f, "a resource pool needs at least 1 unit"),
            Problem::NoNeeds => write!(f, "names no resource it needs"),
            Problem::UnknownResource { id } => {
                write!(f, "needs the resource `{id}`, which does not exist")
            }
            Problem::NoPoolWithRoles { roles } => {
                write!(
                    f,
                    "needs a resource with the roles {}, which no pool has",
                    roles.join(", ")
                )
            }
            Problem::NeedNeverMet { needed, available } => write!(
                f,
                "needs {needed} units at once where at most {available} exist, so this can never be met"
            ),
            Problem::BadPriority { value } => {
                write!(f, "priority must be a finite number, not {value}")
            }
            Problem::UnusedPool => write!(f, "no task uses this resource pool"),
            Problem::UnknownCalendar { id } => {
                write!(
                    f,
                    "has a value for the calendar `{id}`, which does not exist"
                )
            }
            Problem::EmptyCalendar => {
                write!(f, "the calendar has no events, so it is never active")
            }
            Problem::TimedIgnored => write!(
                f,
                "there are no arrivals outside the calendars, but none is given, so this start event never fires"
            ),
        }
    }
}

impl Model {
    pub fn element(&self, id: &ElementId) -> Option<&Element> {
        self.elements.iter().find(|e| &e.id == id)
    }

    /// Every problem with the model, errors first, each group in model order.
    pub fn validate(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        self.check_ids(&mut out);
        let elements: BTreeMap<&ElementId, &Element> =
            self.elements.iter().map(|e| (&e.id, e)).collect();
        let mut outgoing: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
        for c in &self.connections {
            let mut ok = true;
            for end in [&c.from, &c.to] {
                if !elements.contains_key(end) {
                    out.push(Diagnostic::error(
                        Some(&c.id),
                        Problem::UnknownElement {
                            connection: c.id.clone(),
                            missing: end.clone(),
                        },
                    ));
                    ok = false;
                }
            }
            if c.from == c.to {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::SelfLoop {
                        connection: c.id.clone(),
                    },
                ));
                ok = false;
            }
            if let Some(to) = elements.get(&c.to)
                && matches!(to.kind, ElementKind::Source { .. })
            {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::IntoSource {
                        connection: c.id.clone(),
                    },
                ));
                ok = false;
            }
            if let Some(from) = elements.get(&c.from)
                && matches!(from.kind, ElementKind::Sink)
            {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::OutOfSink {
                        connection: c.id.clone(),
                    },
                ));
                ok = false;
            }
            if ok {
                outgoing.entry(&c.from).or_default().push(&c.to);
            }
        }
        for e in &self.elements {
            let outs = outgoing.get(&e.id).map_or(0, Vec::len);
            match e.kind {
                ElementKind::Sink => {}
                _ if outs == 0 => out.push(Diagnostic::error(Some(&e.id), Problem::NoOutput)),
                _ if outs > 1 && !e.kind.is_gateway() => out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::SeveralOutputs { count: outs },
                )),
                _ => {}
            }
            if let ElementKind::Server { capacity: 0, .. } = e.kind {
                out.push(Diagnostic::error(Some(&e.id), Problem::ZeroCapacity));
            }
            // Every time between arrivals that can apply, with no limit on arrivals.
            if let ElementKind::Source {
                interarrival,
                limit: None,
                timed,
                idle_outside,
            } = &e.kind
                && (!*idle_outside)
                    .then_some(interarrival)
                    .into_iter()
                    .chain(timed.iter().map(|t| &t.value))
                    .any(|d| d.check().is_ok() && !d.can_be_positive())
            {
                out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::NoTimeBetweenArrivals,
                ));
            }
            for (parameter, d) in e.kind.distributions() {
                if let Err(error) = d.check() {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::BadDistribution { parameter, error },
                    ));
                }
            }
        }
        self.check_experiment(&mut out);
        self.check_properties(&mut out);
        self.check_routing(&mut out);
        self.check_reachability(&outgoing, &mut out);
        self.check_buffers(&outgoing, &mut out);
        self.check_resources(&mut out);
        // Stable order: errors first, then as found (model order).
        out.sort_by_key(|d| std::cmp::Reverse(d.severity));
        out
    }

    /// Whether the model has no errors (warnings allowed).
    pub fn is_runnable(&self) -> bool {
        self.validate().iter().all(|d| d.severity < Severity::Error)
    }

    /// The units that could ever meet `need`: its pool's, or those of every pool with its roles.
    pub fn units_for(&self, need: &Need) -> Option<u32> {
        match need {
            Need::Resource { id, .. } => self
                .resources
                .iter()
                .find(|r| &r.id == id)
                .map(ResourcePool::most_units),
            Need::Roles { roles, .. } => {
                let pools: Vec<&ResourcePool> = self
                    .resources
                    .iter()
                    .filter(|r| r.has_roles(roles))
                    .collect();
                (!pools.is_empty()).then(|| pools.iter().map(|r| r.most_units()).sum())
            }
        }
    }

    fn check_resources(&self, out: &mut Vec<Diagnostic>) {
        for r in &self.resources {
            if r.most_units() == 0 {
                out.push(Diagnostic::error(Some(&r.id), Problem::EmptyPool));
            }
        }
        self.check_calendars(out);
        let mut used: BTreeSet<&ElementId> = BTreeSet::new();
        for e in &self.elements {
            let ElementKind::SharedTask {
                needs, priority, ..
            } = &e.kind
            else {
                continue;
            };
            if !priority.is_finite() {
                out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::BadPriority { value: *priority },
                ));
            }
            if needs.is_empty() {
                out.push(Diagnostic::error(Some(&e.id), Problem::NoNeeds));
                continue;
            }
            let mut found = Vec::new();
            for need in needs {
                match need {
                    Need::Resource { id, .. } => {
                        used.insert(id);
                    }
                    Need::Roles { roles, .. } => used.extend(
                        self.resources
                            .iter()
                            .filter(|r| r.has_roles(roles))
                            .map(|r| &r.id),
                    ),
                }
                let problem = match (need, self.units_for(need)) {
                    (Need::Resource { id, .. }, None) => {
                        Some(Problem::UnknownResource { id: id.clone() })
                    }
                    (Need::Roles { roles, .. }, None) => Some(Problem::NoPoolWithRoles {
                        roles: roles.clone(),
                    }),
                    (_, Some(available)) if need.quantity() == 0 || need.quantity() > available => {
                        Some(Problem::NeedNeverMet {
                            needed: need.quantity(),
                            available,
                        })
                    }
                    _ => None,
                };
                found.push(problem);
            }
            // An alternative that can never be met is an error when it is the only kind there
            // is, else a warning (the others still serve).
            let all_fail = found.iter().all(Option::is_some);
            for problem in found.into_iter().flatten() {
                out.push(if all_fail {
                    Diagnostic::error(Some(&e.id), problem)
                } else {
                    Diagnostic::warning(Some(&e.id), problem)
                });
            }
        }
        for r in &self.resources {
            if !used.contains(&r.id) {
                out.push(Diagnostic::warning(Some(&r.id), Problem::UnusedPool));
            }
        }
    }

    /// Calendars: each referenced one exists, and each has an event.
    fn check_calendars(&self, out: &mut Vec<Diagnostic>) {
        let known: BTreeSet<&ElementId> = self.calendars.iter().map(|c| &c.id).collect();
        for c in &self.calendars {
            if c.events.is_empty() {
                out.push(Diagnostic::warning(Some(&c.id), Problem::EmptyCalendar));
            }
        }
        let mut refs: Vec<(&ElementId, &ElementId)> = Vec::new();
        for e in &self.elements {
            if let ElementKind::Source {
                timed,
                idle_outside,
                ..
            } = &e.kind
            {
                refs.extend(timed.iter().map(|t| (&e.id, &t.calendar)));
                if *idle_outside && timed.is_empty() {
                    out.push(Diagnostic::warning(Some(&e.id), Problem::TimedIgnored));
                }
            }
        }
        for r in &self.resources {
            refs.extend(r.calendars().map(|c| (&r.id, c)));
        }
        let mut reported = BTreeSet::new();
        for (at, c) in refs {
            if !known.contains(c) && reported.insert((at, c)) {
                out.push(Diagnostic::error(
                    Some(at),
                    Problem::UnknownCalendar { id: c.clone() },
                ));
            }
        }
    }

    fn check_ids(&self, out: &mut Vec<Diagnostic>) {
        let mut seen = BTreeSet::new();
        let ids = self
            .elements
            .iter()
            .map(|e| &e.id)
            .chain(self.connections.iter().map(|c| &c.id))
            .chain(self.resources.iter().map(|r| &r.id))
            .chain(self.calendars.iter().map(|c| &c.id));
        for id in ids {
            if !ElementId::is_valid(id.as_str()) {
                out.push(Diagnostic::error(
                    None,
                    Problem::InvalidId {
                        id: id.as_str().to_owned(),
                    },
                ));
            }
            if !seen.insert(id) {
                out.push(Diagnostic::error(
                    Some(id),
                    Problem::DuplicateId { id: id.clone() },
                ));
            }
        }
    }

    /// The type of every property the model sets, by name: its declared type, else the type of
    /// its first value in model order (a number or a distribution is a double, an expression the
    /// kind of value it gives).
    pub fn property_types(&self) -> BTreeMap<String, PropertyType> {
        let mut types: BTreeMap<String, PropertyType> = BTreeMap::new();
        let all = || self.elements.iter().flat_map(|e| &e.properties);
        for p in all() {
            if let Some(t) = p.ty {
                types.entry(p.name.clone()).or_insert(t);
            }
        }
        for p in all() {
            if types.contains_key(&p.name) {
                continue;
            }
            let t = match &p.value {
                PropertyValue::Number(_) | PropertyValue::Random(_) => PropertyType::Double,
                PropertyValue::Boolean(_) => PropertyType::Boolean,
                PropertyValue::Text(_) => PropertyType::String,
                PropertyValue::Expression(src) => match expr::parse(src).map(|e| e.kind()) {
                    Ok(expr::Kind::Boolean) => PropertyType::Boolean,
                    Ok(expr::Kind::Text) => PropertyType::String,
                    _ => PropertyType::Double,
                },
            };
            types.insert(p.name.clone(), t);
        }
        types
    }

    fn check_properties(&self, out: &mut Vec<Diagnostic>) {
        let types = self.property_types();
        let mut declared: BTreeMap<&str, PropertyType> = BTreeMap::new();
        for e in &self.elements {
            let at = Some(&e.id);
            let mut seen = BTreeSet::new();
            for p in &e.properties {
                let name = p.name.clone();
                if p.name.trim().is_empty() {
                    out.push(Diagnostic::error(
                        at,
                        Problem::BadProperty {
                            name,
                            reason: "needs a name",
                        },
                    ));
                    continue;
                }
                if !seen.insert(p.name.as_str()) {
                    out.push(Diagnostic::error(
                        at,
                        Problem::BadProperty {
                            name,
                            reason: "is set twice by this element",
                        },
                    ));
                    continue;
                }
                if let Some(t) = p.ty {
                    match declared.get(p.name.as_str()) {
                        Some(&first) if first != t => out.push(Diagnostic::error(
                            at,
                            Problem::PropertyTypeConflict {
                                name: name.clone(),
                                first,
                                second: t,
                            },
                        )),
                        Some(_) => {}
                        None => {
                            declared.insert(p.name.as_str(), t);
                        }
                    }
                }
                if let Some(ty) = p.ty.or_else(|| types.get(&p.name).copied()) {
                    let fits = match &p.value {
                        PropertyValue::Number(n) => {
                            ty.is_numeric() && (ty != PropertyType::Long || n.fract() == 0.0)
                        }
                        PropertyValue::Random(_) => ty.is_numeric(),
                        PropertyValue::Boolean(_) => ty == PropertyType::Boolean,
                        PropertyValue::Text(_) => {
                            matches!(ty, PropertyType::String | PropertyType::DateTime)
                        }
                        PropertyValue::Expression(_) => true,
                    };
                    if !fits {
                        let value = match (&p.value, ty) {
                            (PropertyValue::Number(_), PropertyType::Long) => "a fractional number",
                            (v, _) => v.describe(),
                        };
                        out.push(Diagnostic::error(
                            at,
                            Problem::PropertyValueType {
                                name: name.clone(),
                                ty,
                                value,
                            },
                        ));
                    }
                }
                match &p.value {
                    PropertyValue::Random(d) => {
                        if let Err(error) = d.check() {
                            out.push(Diagnostic::error(
                                at,
                                Problem::BadDistribution {
                                    parameter: "property value",
                                    error,
                                },
                            ));
                        }
                    }
                    PropertyValue::Expression(src) => match expr::parse(src) {
                        Ok(e) => {
                            let mut unknown = BTreeSet::new();
                            for read in e.properties() {
                                if !types.contains_key(read) && unknown.insert(read) {
                                    out.push(Diagnostic::error(
                                        at,
                                        Problem::UnknownProperty {
                                            name: read.to_owned(),
                                        },
                                    ));
                                }
                            }
                        }
                        Err(error) => out.push(Diagnostic::error(
                            at,
                            Problem::BadExpression {
                                name,
                                expression: src.clone(),
                                error,
                            },
                        )),
                    },
                    _ => {}
                }
            }
        }
    }

    /// Routing: probabilities, conditions and defaults on the connections out of exclusive
    /// gateways, and routing given where it has no effect.
    fn check_routing(&self, out: &mut Vec<Diagnostic>) {
        let types = self.property_types();
        for e in &self.elements {
            let outs: Vec<&Connection> =
                self.connections.iter().filter(|c| c.from == e.id).collect();
            let exclusive = matches!(e.kind, ElementKind::ExclusiveGateway { .. });
            for c in &outs {
                let at = Some(&c.id);
                if let Some(p) = c.probability
                    && !(p.is_finite() && (0.0..=1.0).contains(&p))
                {
                    out.push(Diagnostic::error(at, Problem::BadProbability { value: p }));
                }
                if let Some(src) = &c.condition {
                    match expr::parse(src) {
                        Ok(x) => {
                            let mut unknown = BTreeSet::new();
                            for read in x.properties() {
                                if !types.contains_key(read) && unknown.insert(read) {
                                    out.push(Diagnostic::error(
                                        at,
                                        Problem::UnknownProperty {
                                            name: read.to_owned(),
                                        },
                                    ));
                                }
                            }
                        }
                        Err(error) => out.push(Diagnostic::error(
                            at,
                            Problem::BadCondition {
                                expression: src.clone(),
                                error,
                            },
                        )),
                    }
                }
                if !exclusive && (c.probability.is_some() || c.condition.is_some()) {
                    let what = if c.condition.is_some() {
                        "this condition"
                    } else {
                        "this probability"
                    };
                    out.push(Diagnostic::warning(
                        at,
                        Problem::RoutingIgnored {
                            what,
                            reason: "only connections out of an exclusive gateway choose a route",
                        },
                    ));
                }
            }
            let ElementKind::ExclusiveGateway { default } = &e.kind else {
                continue;
            };
            if let Some(d) = default
                && !outs.iter().any(|c| &c.id == d)
            {
                out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::BadDefault {
                        connection: d.clone(),
                    },
                ));
            }
            if outs.iter().any(|c| c.condition.is_some()) {
                if default.is_none() {
                    out.push(Diagnostic::warning(Some(&e.id), Problem::NoDefault));
                }
                for c in outs.iter().filter(|c| c.probability.is_some()) {
                    out.push(Diagnostic::warning(
                        Some(&c.id),
                        Problem::RoutingIgnored {
                            what: "this probability",
                            reason: "the gateway routes by condition",
                        },
                    ));
                }
            } else if outs.len() > 1 {
                let given: Vec<f64> = outs.iter().filter_map(|c| c.probability).collect();
                let valid = given
                    .iter()
                    .all(|p| p.is_finite() && (0.0..=1.0).contains(p));
                let sum: f64 = given.iter().sum();
                let all = given.len() == outs.len();
                if valid && ((all && (sum - 1.0).abs() > 1e-9) || sum > 1.0 + 1e-9) {
                    out.push(Diagnostic::warning(
                        Some(&e.id),
                        Problem::ProbabilitySum { sum },
                    ));
                }
            }
        }
    }

    fn check_experiment(&self, out: &mut Vec<Diagnostic>) {
        let x = &self.experiment;
        if !(x.warmup.is_finite() && x.warmup >= 0.0) {
            out.push(Diagnostic::error(
                None,
                Problem::BadExperiment {
                    field: "warm-up",
                    requirement: "0 or more",
                },
            ));
        }
        if !(x.duration.is_finite() && x.duration > 0.0) {
            out.push(Diagnostic::error(
                None,
                Problem::BadExperiment {
                    field: "duration",
                    requirement: "greater than 0",
                },
            ));
        }
        if x.replications == 0 {
            out.push(Diagnostic::error(
                None,
                Problem::BadExperiment {
                    field: "replications",
                    requirement: "1 or more",
                },
            ));
        }
    }

    /// Finite buffers fed through gateways, and loops of finite buffers that can deadlock.
    fn check_buffers(
        &self,
        outgoing: &BTreeMap<&ElementId, Vec<&ElementId>>,
        out: &mut Vec<Diagnostic>,
    ) {
        let finite = |id: &ElementId| {
            self.element(id).is_some_and(|e| {
                matches!(
                    e.kind,
                    ElementKind::Server {
                        queue_capacity: Some(_),
                        ..
                    }
                )
            })
        };
        for e in &self.elements {
            if !finite(&e.id) {
                continue;
            }
            for (from, tos) in outgoing {
                if tos.contains(&&e.id) && self.element(from).is_some_and(|g| g.kind.is_gateway()) {
                    out.push(Diagnostic::warning(
                        Some(&e.id),
                        Problem::BufferBehindGateway {
                            gateway: (*from).clone(),
                        },
                    ));
                }
            }
        }
        // A loop among finite-buffer servers alone: anything else on the way has unlimited room.
        let next = |id: &ElementId| -> Vec<&ElementId> {
            outgoing
                .get(id)
                .map(|tos| tos.iter().copied().filter(|t| finite(t)).collect())
                .unwrap_or_default()
        };
        let mut reported: BTreeSet<&ElementId> = BTreeSet::new();
        for e in &self.elements {
            if !finite(&e.id) || reported.contains(&e.id) {
                continue;
            }
            // The shortest loop back to `e`, by breadth-first search.
            let mut parent: BTreeMap<&ElementId, &ElementId> = BTreeMap::new();
            let mut queue = VecDeque::from([&e.id]);
            let mut closed = None;
            while let Some(at) = queue.pop_front() {
                for to in next(at) {
                    if to == &e.id {
                        closed = Some(at);
                        break;
                    }
                    if !parent.contains_key(to) {
                        parent.insert(to, at);
                        queue.push_back(to);
                    }
                }
                if closed.is_some() {
                    break;
                }
            }
            let Some(mut at) = closed else {
                continue;
            };
            let mut through = vec![at];
            while at != &e.id {
                let Some(p) = parent.get(at) else {
                    break;
                };
                at = p;
                through.push(at);
            }
            through.reverse();
            reported.extend(through.iter().copied());
            out.push(Diagnostic::warning(
                Some(&e.id),
                Problem::BlockingCycle {
                    through: through.into_iter().cloned().collect(),
                },
            ));
        }
    }

    fn check_reachability(
        &self,
        outgoing: &BTreeMap<&ElementId, Vec<&ElementId>>,
        out: &mut Vec<Diagnostic>,
    ) {
        let sources: Vec<&ElementId> = self
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Source { .. }))
            .map(|e| &e.id)
            .collect();
        let sinks: Vec<&ElementId> = self
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Sink))
            .map(|e| &e.id)
            .collect();
        if sources.is_empty() && !self.elements.is_empty() {
            out.push(Diagnostic::warning(None, Problem::NoSource));
        }
        if sinks.is_empty() && !self.elements.is_empty() {
            out.push(Diagnostic::warning(None, Problem::NoSink));
        }
        let forward = reach(&sources, |id| outgoing.get(id).cloned().unwrap_or_default());
        let mut incoming: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
        for (from, tos) in outgoing {
            for to in tos {
                incoming.entry(to).or_default().push(from);
            }
        }
        let backward = reach(&sinks, |id| incoming.get(id).cloned().unwrap_or_default());
        if !sources.is_empty() {
            for e in &self.elements {
                if !matches!(e.kind, ElementKind::ParallelGateway) || !forward.contains(&e.id) {
                    continue;
                }
                let ins: Vec<&Connection> =
                    self.connections.iter().filter(|c| c.to == e.id).collect();
                if ins.len() > 1 {
                    for c in ins.iter().filter(|c| !forward.contains(&c.from)) {
                        out.push(Diagnostic::warning(
                            Some(&e.id),
                            Problem::JoinNeverCompletes {
                                connection: c.id.clone(),
                            },
                        ));
                    }
                }
            }
        }
        for e in &self.elements {
            if !sources.is_empty() && !forward.contains(&e.id) {
                out.push(Diagnostic::warning(Some(&e.id), Problem::Unreachable));
            } else if !sinks.is_empty() && !backward.contains(&e.id) {
                out.push(Diagnostic::warning(Some(&e.id), Problem::NoPathToSink));
            }
        }
    }
}

fn reach<'a>(
    start: &[&'a ElementId],
    next: impl Fn(&'a ElementId) -> Vec<&'a ElementId>,
) -> BTreeSet<&'a ElementId> {
    let mut seen: BTreeSet<&ElementId> = start.iter().copied().collect();
    let mut queue: VecDeque<&ElementId> = start.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        for n in next(id) {
            if seen.insert(n) {
                queue.push_back(n);
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> ElementId {
        ElementId(s.to_owned())
    }

    fn el(i: &str, kind: ElementKind) -> Element {
        Element::new(id(i), i, kind)
    }

    fn prop(name: &str, ty: Option<PropertyType>, value: PropertyValue) -> Property {
        Property {
            name: name.into(),
            ty,
            value,
        }
    }

    #[test]
    fn properties_are_typed_and_checked() {
        let mut m = mm1();
        m.elements[0].properties = vec![
            prop(
                "issues",
                Some(PropertyType::Long),
                PropertyValue::Random(Dist::Poisson { mean: 2.0 }),
            ),
            prop("tier", None, PropertyValue::Text("gold".into())),
        ];
        m.elements[1].properties = vec![
            prop(
                "issues",
                None,
                PropertyValue::Expression("getProperty('issues') - 1".into()),
            ),
            prop(
                "done",
                None,
                PropertyValue::Expression("getProperty('issues') <= 0".into()),
            ),
        ];
        assert_eq!(m.validate(), vec![]);
        let types = m.property_types();
        assert_eq!(types.get("issues"), Some(&PropertyType::Long));
        assert_eq!(types.get("tier"), Some(&PropertyType::String));
        assert_eq!(types.get("done"), Some(&PropertyType::Boolean));

        m.elements[1].properties.push(prop(
            "late",
            None,
            PropertyValue::Expression("getProperty('missing') +".into()),
        ));
        m.elements[1].properties.push(prop(
            "score",
            None,
            PropertyValue::Expression("getProperty('missing') * 2".into()),
        ));
        m.elements[2].properties = vec![
            prop(
                "issues",
                Some(PropertyType::Double),
                PropertyValue::Number(1.0),
            ),
            prop("tier", None, PropertyValue::Number(3.0)),
            prop("tier", None, PropertyValue::Text("x".into())),
            prop("", None, PropertyValue::Boolean(true)),
        ];
        m.elements[0].properties.push(prop(
            "count",
            Some(PropertyType::Long),
            PropertyValue::Number(1.5),
        ));
        let codes: Vec<(String, &str)> = m
            .validate()
            .iter()
            .map(|d| {
                (
                    d.at.as_ref().map(ToString::to_string).unwrap_or_default(),
                    d.problem.code(),
                )
            })
            .collect();
        for want in [
            ("machine", "bad_expression"),
            ("machine", "unknown_property"),
            ("exit", "property_type_conflict"),
            ("exit", "property_value_type"),
            ("exit", "bad_property"),
            ("arrivals", "property_value_type"),
        ] {
            assert!(
                codes.contains(&(want.0.into(), want.1)),
                "missing {want:?} in {codes:?}"
            );
        }
        let text: Vec<String> = m.validate().iter().map(ToString::to_string).collect();
        assert!(
            text.iter().any(|t| t
                == "error at `machine`: property `late`: `getProperty('missing') +`: the expression ends too early (at character 25)"),
            "{text:#?}"
        );
    }

    fn link(i: &str, from: &str, to: &str) -> Connection {
        Connection {
            probability: None,
            condition: None,
            id: id(i),
            from: id(from),
            to: id(to),
        }
    }

    fn routed(mut link: impl FnMut(&str, &str, &str) -> Connection) -> Model {
        let mut m = mm1();
        m.elements.insert(
            1,
            el(
                "check",
                ElementKind::ExclusiveGateway {
                    default: Some(id("nowhere")),
                },
            ),
        );
        m.elements.push(el("scrap", ElementKind::Sink));
        m.elements.push(el("join", ElementKind::ParallelGateway));
        m.elements.push(el(
            "orphan",
            ElementKind::Delay {
                duration: Dist::Constant(1.0),
            },
        ));
        let mut to_machine = link("c1", "check", "machine");
        to_machine.probability = Some(0.7);
        to_machine.condition = Some("getProperty('missing') >".into());
        let mut to_scrap = link("c2", "check", "scrap");
        to_scrap.probability = Some(1.5);
        let mut wasted = link("c3", "machine", "join");
        wasted.condition = Some("true()".into());
        m.connections = vec![
            link("c0", "arrivals", "check"),
            to_machine,
            to_scrap,
            wasted,
            link("c4", "orphan", "join"),
            link("c5", "join", "exit"),
        ];
        m
    }

    #[test]
    fn routing_is_checked() {
        let m = routed(|i, f, t| Connection::new(id(i), id(f), id(t)));
        let found: Vec<(String, &str)> = m
            .validate()
            .iter()
            .map(|d| {
                (
                    d.at.as_ref().map(ToString::to_string).unwrap_or_default(),
                    d.problem.code(),
                )
            })
            .collect();
        for want in [
            ("c1", "bad_condition"),
            ("c2", "bad_probability"),
            ("c3", "routing_ignored"),
            ("check", "bad_default"),
            ("c1", "routing_ignored"),
            ("join", "join_never_completes"),
        ] {
            assert!(
                found.contains(&(want.0.into(), want.1)),
                "missing {want:?} in {found:?}"
            );
        }
        // A gateway may have several outputs; nothing else may.
        assert!(
            !found
                .iter()
                .any(|(at, c)| at == "check" && *c == "several_outputs")
        );

        // Probabilities that do not add up to 1 are used in proportion, with a warning.
        let mut m = mm1();
        m.elements.insert(
            1,
            el("pick", ElementKind::ExclusiveGateway { default: None }),
        );
        m.elements.push(el("scrap", ElementKind::Sink));
        let mut a = Connection::new(id("a"), id("pick"), id("machine"));
        a.probability = Some(0.66);
        let mut b = Connection::new(id("b"), id("pick"), id("scrap"));
        b.probability = Some(0.33);
        m.connections = vec![
            Connection::new(id("in"), id("arrivals"), id("pick")),
            a,
            b,
            Connection::new(id("out"), id("machine"), id("exit")),
        ];
        let ds = m.validate();
        assert_eq!(ds.len(), 1, "{ds:?}");
        assert_eq!(ds[0].problem.code(), "probability_sum");
        assert_eq!(ds[0].severity, Severity::Warning);
        assert!(ds[0].to_string().contains("0.99"), "{}", ds[0]);
    }

    fn mm1() -> Model {
        Model {
            name: "M/M/1".into(),
            elements: vec![
                el(
                    "arrivals",
                    ElementKind::Source {
                        interarrival: Dist::exponential_rate(0.75),
                        limit: None,
                        idle_outside: false,
                        timed: Vec::new(),
                    },
                ),
                el(
                    "machine",
                    ElementKind::Server {
                        capacity: 1,
                        service: Dist::exponential_rate(1.0),
                        queue_capacity: None,
                        breakdown: None,
                    },
                ),
                el("exit", ElementKind::Sink),
            ],
            connections: vec![
                link("f1", "arrivals", "machine"),
                link("f2", "machine", "exit"),
            ],
            resources: Vec::new(),
            experiment: Experiment::default(),
            calendars: Vec::new(),
        }
    }

    fn problems(m: &Model) -> Vec<(Severity, Option<String>, String)> {
        m.validate()
            .into_iter()
            .map(|d| {
                (
                    d.severity,
                    d.at.as_ref().map(|a| a.to_string()),
                    format!("{:?}", d.problem),
                )
            })
            .collect()
    }

    #[test]
    fn a_valid_model_has_no_diagnostics() {
        assert_eq!(mm1().validate(), vec![]);
        assert!(mm1().is_runnable());
    }

    fn buffered(m: &mut Model, id: &str, q: u32) {
        if let Some(ElementKind::Server { queue_capacity, .. }) = m
            .elements
            .iter_mut()
            .find(|e| e.id.as_str() == id)
            .map(|e| &mut e.kind)
        {
            *queue_capacity = Some(q);
        }
    }

    #[test]
    fn finite_buffers_are_valid_even_with_no_waiting_room() {
        let mut m = mm1();
        buffered(&mut m, "machine", 0);
        assert_eq!(m.validate(), vec![]);
    }

    #[test]
    fn a_buffer_fed_through_a_gateway_is_flagged() {
        let mut m = mm1();
        buffered(&mut m, "machine", 2);
        m.elements
            .push(el("pick", ElementKind::ExclusiveGateway { default: None }));
        m.connections[0].to = ElementId::unchecked("pick");
        m.connections.push(link("f3", "pick", "machine"));
        let ds = m.validate();
        assert_eq!(ds.len(), 1, "{ds:?}");
        assert_eq!(ds[0].problem.code(), "buffer_behind_gateway");
        assert_eq!(ds[0].severity, Severity::Warning);
        assert_eq!(ds[0].at.as_ref().map(ElementId::as_str), Some("machine"));
        assert!(ds[0].to_string().contains("`pick`"), "{}", ds[0]);
    }

    #[test]
    fn a_loop_of_finite_buffers_may_deadlock() {
        let server = |q| ElementKind::Server {
            capacity: 1,
            service: Dist::Constant(1.0),
            queue_capacity: q,
            breakdown: None,
        };
        let mut m = mm1();
        m.elements.push(el("a", server(Some(1))));
        m.elements.push(el("b", server(Some(1))));
        m.elements
            .push(el("split", ElementKind::ExclusiveGateway { default: None }));
        // machine → a → b → split → (a | exit): a and b are a loop if the way back skips the
        // gateway, so first check that a gateway on the way breaks it.
        m.connections[1].to = ElementId::unchecked("a");
        m.connections.push(link("f3", "a", "b"));
        m.connections.push(link("f4", "b", "split"));
        m.connections.push(link("f5", "split", "a"));
        m.connections.push(link("f6", "split", "exit"));
        let codes = |m: &Model| -> Vec<&'static str> {
            m.validate().iter().map(|d| d.problem.code()).collect()
        };
        assert_eq!(codes(&m), ["buffer_behind_gateway"]);
        // b straight back to a: a loop of finite buffers only.
        m.connections.retain(|c| c.id.as_str() != "f4");
        m.connections.push(link("f7", "b", "a"));
        m.connections.retain(|c| c.from.as_str() != "split");
        m.elements.retain(|e| e.id.as_str() != "split");
        let ds = m.validate();
        let cycle: Vec<&Diagnostic> = ds
            .iter()
            .filter(|d| d.problem.code() == "blocking_cycle")
            .collect();
        assert_eq!(cycle.len(), 1, "{ds:?}");
        assert!(cycle[0].to_string().contains("`a` → `b`"), "{}", cycle[0]);
    }

    #[test]
    fn ids_follow_ncname() {
        for ok in [
            "a",
            "_x",
            "machine-1",
            "Task_1.2",
            "_FA77D7D8-9C02-4006-942A-B003AEDA5E3C",
            "工位",
        ] {
            assert!(ElementId::is_valid(ok), "{ok}");
        }
        for bad in ["", "1abc", "-a", "a b", "a:b", ".x"] {
            assert!(!ElementId::is_valid(bad), "{bad}");
            assert_eq!(ElementId::new(bad), None);
        }
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let mut m = mm1();
        m.elements.push(el(
            "machine",
            ElementKind::Delay {
                duration: Dist::Constant(1.0),
            },
        ));
        m.elements.push(el(
            "idle",
            ElementKind::Server {
                capacity: 0,
                service: Dist::Uniform { min: 2.0, max: 1.0 },
                queue_capacity: None,
                breakdown: None,
            },
        ));
        m.connections.push(link("f3", "exit", "arrivals"));
        m.connections.push(link("f4", "ghost", "exit"));
        m.experiment.replications = 0;
        let p = problems(&m);
        let has = |frag: &str| p.iter().any(|(_, _, s)| s.contains(frag));
        for frag in [
            "DuplicateId",
            "IntoSource",
            "OutOfSink",
            "UnknownElement",
            "ZeroCapacity",
            "BadDistribution",
            "NoOutput",
            "BadExperiment",
        ] {
            assert!(has(frag), "missing {frag} in {p:#?}");
        }
        // Errors come before warnings.
        let first_warning = p
            .iter()
            .position(|(s, _, _)| *s == Severity::Warning)
            .unwrap_or(p.len());
        assert!(
            p[first_warning..]
                .iter()
                .all(|(s, _, _)| *s == Severity::Warning)
        );
        assert!(!m.is_runnable());
    }

    #[test]
    fn branching_needs_a_gateway() {
        let mut m = mm1();
        m.elements.push(el("scrap", ElementKind::Sink));
        m.connections.push(link("f3", "machine", "scrap"));
        assert_eq!(
            problems(&m),
            vec![(
                Severity::Error,
                Some("machine".into()),
                "SeveralOutputs { count: 2 }".into()
            )]
        );
    }

    #[test]
    fn reachability_warnings() {
        let mut m = mm1();
        // An island that no source feeds, and a loop entities can never leave.
        m.elements.push(el(
            "orphan",
            ElementKind::Delay {
                duration: Dist::Constant(1.0),
            },
        ));
        m.connections.push(link("f3", "orphan", "exit"));
        m.elements.push(el(
            "a",
            ElementKind::Delay {
                duration: Dist::Constant(1.0),
            },
        ));
        m.elements.push(el(
            "b",
            ElementKind::Delay {
                duration: Dist::Constant(1.0),
            },
        ));
        m.elements.push(el(
            "src2",
            ElementKind::Source {
                interarrival: Dist::Constant(1.0),
                limit: Some(3),
                idle_outside: false,
                timed: Vec::new(),
            },
        ));
        m.connections.push(link("f4", "src2", "a"));
        m.connections.push(link("f5", "a", "b"));
        m.connections.push(link("f6", "b", "a"));
        let p = problems(&m);
        assert!(
            p.contains(&(
                Severity::Warning,
                Some("orphan".into()),
                "Unreachable".into()
            )),
            "{p:#?}"
        );
        for x in ["src2", "a", "b"] {
            assert!(
                p.contains(&(Severity::Warning, Some(x.into()), "NoPathToSink".into())),
                "{x}: {p:#?}"
            );
        }
        assert!(m.is_runnable(), "warnings alone do not stop a run");
    }

    #[test]
    fn arrivals_need_time_between_them_unless_limited() {
        for (interarrival, limit, stuck) in [
            (Dist::Constant(0.0), None, true),
            (Dist::exponential_rate(0.0), None, true),
            (Dist::Constant(0.0), Some(5), false),
            (
                Dist::Normal {
                    mean: 0.0,
                    std_dev: 1.0,
                },
                None,
                false,
            ),
        ] {
            let mut m = mm1();
            for e in &mut m.elements {
                if let ElementKind::Source { .. } = e.kind {
                    e.kind = ElementKind::Source {
                        interarrival: interarrival.clone(),
                        limit,
                        idle_outside: false,
                        timed: Vec::new(),
                    };
                }
            }
            let found = problems(&m)
                .iter()
                .any(|(s, _, p)| *s == Severity::Error && p == "NoTimeBetweenArrivals");
            assert_eq!(found, stuck, "{interarrival:?}, limit {limit:?}");
            assert_eq!(m.is_runnable(), !stuck);
        }
    }

    #[test]
    fn self_loops_and_empty_models() {
        let mut m = mm1();
        m.connections.push(link("f3", "machine", "machine"));
        assert!(
            problems(&m)
                .iter()
                .any(|(_, at, s)| at.as_deref() == Some("f3") && s.contains("SelfLoop"))
        );
        let empty = Model::default();
        assert_eq!(empty.validate(), vec![]);
        let lonely = Model {
            elements: vec![el("x", ElementKind::Sink)],
            ..Model::default()
        };
        assert!(problems(&lonely).iter().any(|(_, _, s)| s == "NoSource"));
    }

    #[test]
    fn time_units_match_bpsim() {
        for u in [
            TimeUnit::Milliseconds,
            TimeUnit::Seconds,
            TimeUnit::Minutes,
            TimeUnit::Hours,
            TimeUnit::Days,
            TimeUnit::Years,
        ] {
            assert_eq!(TimeUnit::from_bpsim_name(u.bpsim_name()), Some(u));
        }
        assert_eq!(
            TimeUnit::default(),
            TimeUnit::Minutes,
            "BPSim's default base time unit"
        );
        assert_eq!(TimeUnit::Minutes.convert(2.0, TimeUnit::Hours), 120.0);
        assert_eq!(TimeUnit::Hours.convert(90.0, TimeUnit::Minutes), 1.5);
        assert_eq!(TimeUnit::from_bpsim_name("minutes"), None);
    }

    #[test]
    fn diagnostics_read_well() {
        let mut m = mm1();
        m.connections.retain(|c| c.id.as_str() != "f2");
        let text: Vec<String> = m.validate().iter().map(ToString::to_string).collect();
        assert!(
            text.contains(&"error at `machine`: has nowhere to send entities".to_string()),
            "{text:#?}"
        );
    }
}
