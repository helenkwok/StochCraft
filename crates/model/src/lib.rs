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

pub use stochcraft_random::{Dist, Family, FitError, ThreePoint, Upper};

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
// The document form of a block: one per element, cloned with the model, never in the event
// loop, so a server's many settings stay inline rather than boxed.
#[allow(clippy::large_enum_variant)]
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
        /// The time a part takes to get here from the element that sends it (a short conveyor,
        /// a hand transfer, a minimum time in the buffer); `None`: parts arrive at once.
        /// See [`Transfer`].
        transfer: Option<Transfer>,
        /// Whether the units are available (on shift) outside the calendars of
        /// `availability`: BPSim `availability` on the task's own resource, default true.
        available: bool,
        /// Availability while a calendar is active, the first active one applying (BPSim
        /// `availability` values `validFor` a calendar), as for a pool. All the units follow it.
        ///
        /// A unit checks it only when it takes its next entity: a job under way when the units
        /// go off shift finishes first, and a job interrupted by a failure resumes after its
        /// repair even if the shift has ended meanwhile. Failures ([`Breakdown`]) keep coming
        /// in elapsed time through the breaks, and a repair goes on through them.
        availability: Vec<Timed<bool>>,
        /// The power each unit draws in each of its states, for energy results; `None`: no
        /// energy is reported. A StochCraft vendor extension (`stochcraft:power`).
        power: Option<Power>,
        /// A setup before a job: before every job (BPSim `SetUpTime`), or only when the part's
        /// type changes (a changeover, a StochCraft vendor extension); `None`: no setups. See
        /// [`Setup`].
        setup: Option<Setup>,
        /// The planned time to make one item (ISO 22400-2's planned runtime per item), for the
        /// OEE results: its mean is the plan. Not simulated; `None`: effectiveness and OEE are
        /// not reported. A StochCraft vendor extension (`stochcraft:plannedRunTime`).
        planned_run_time: Option<Dist>,
        /// How long an entity waits before it gives up (reneging); `None`: entities wait for as
        /// long as it takes. See [`Patience`].
        patience: Option<Patience>,
        /// Each unit is checked now and then and taken out of service for a restart when too
        /// many entities wait (a StochCraft vendor extension, `stochcraft:healthCheck`); `None`:
        /// units are never taken out. See [`HealthCheck`].
        health_check: Option<HealthCheck>,
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
    /// Removes entities (a BPMN end event). With `scrap`, what reaches it is scrap, for the
    /// OEE quality results; with `demand`, the customer demand it serves, for takt.
    Sink {
        scrap: Option<Scrap>,
        demand: Option<Demand>,
    },
    /// Sends each entity down one outgoing connection, at once (a BPMN exclusive gateway). With
    /// [`Routing::Standard`]: if any outgoing connection has a condition, the first in model
    /// order whose condition holds is taken, else `default`, else the entity leaves the model
    /// here; otherwise one is drawn by the connections' probabilities (those without one share
    /// evenly what the others leave). With [`Routing::ShortestQueue`], see there.
    ExclusiveGateway {
        /// The connection taken when no condition holds (BPMN's `default` flow).
        default: Option<ElementId>,
        /// How the way is chosen.
        routing: Routing,
    },
    /// Copies each entity down every outgoing connection, at once (a BPMN parallel gateway).
    /// With several incoming connections it first waits for an entity on each of them (a join)
    /// and sends one on: the one from the first incoming connection, in model order, with any
    /// property it lacks taken from the others in that order.
    ///
    /// An assembly is a join with a bill of materials: an incoming connection with a
    /// [`Connection::quantity`] gives that many entities to each one sent on, and the parts
    /// other than the first are used up.
    ParallelGateway,
    /// Collects entities until there are `size`, then sends them on together as one batch, at
    /// once (a StochCraft vendor extension on a task, `stochcraft:batchSize`). The batch is the
    /// first entity of the batch, carrying the others; an [`ElementKind::Unbatch`] later splits
    /// it up again. Entities waiting for a batch to fill at the end of a run are still inside.
    Batch { size: u32 },
    /// Splits each batch back into the entities it was made of, sending each on, at once (a
    /// StochCraft vendor extension on a task, `stochcraft:unbatch`). An entity that is not a
    /// batch passes through as it is.
    Unbatch,
}

/// The electrical power a server's unit draws in each state, in kilowatts. At every instant a
/// unit is in exactly one state, the first that applies of failed (or under repair), processing
/// (including a job running on into a break), blocked (holding a finished entity, waiting for
/// room or through a transfer), off shift (idle because the calendar has the units
/// unavailable) and starved (idle, on shift and up). Blocked, failed and off shift draw `idle` unless given.
#[derive(Debug, Clone, PartialEq)]
pub struct Power {
    pub processing: f64,
    pub idle: f64,
    pub blocked: Option<f64>,
    pub failed: Option<f64>,
    pub off_shift: Option<f64>,
}

impl Power {
    /// A unit's power while blocked: `blocked`, else `idle`.
    pub fn while_blocked(&self) -> f64 {
        self.blocked.unwrap_or(self.idle)
    }

    /// A unit's power while failed or under repair: `failed`, else `idle`.
    pub fn while_failed(&self) -> f64 {
        self.failed.unwrap_or(self.idle)
    }

    /// A unit's power while off shift: `off_shift`, else `idle`.
    pub fn while_off_shift(&self) -> f64 {
        self.off_shift.unwrap_or(self.idle)
    }

    /// Each value given, by state name, for checking.
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        let mut v = vec![("processing", self.processing), ("idle", self.idle)];
        for (name, x) in [
            ("blocked", self.blocked),
            ("failed", self.failed),
            ("off_shift", self.off_shift),
        ] {
            if let Some(x) = x {
                v.push((name, x));
            }
        }
        v
    }
}

/// The time a part takes to get into a server from the element that sends it, drawn for each
/// part (a StochCraft vendor extension, `stochcraft:transferTime`).
///
/// A part takes its place in the buffer (`queue_capacity`) when its transfer starts, so the
/// capacity counts parts in transit as well as those waiting, and it joins the queue when the
/// transfer ends. A part in transit is inside this server (its work in progress), not the
/// sender's. A server or a task with shared resources sending it waits after its job until there
/// is room (blocking after service); then, if `holds_sender`, its unit stays occupied, and counts
/// as blocked, for the whole transfer (a robot or a person carrying the part), and goes on with
/// it if the unit fails meanwhile; otherwise the unit is free at once and the time is a minimum
/// time in the buffer (`stochcraft:transferHoldsSender="false"`). Any other sender (a source, a
/// delay, a gateway, a batch or a split) holds nothing while the part travels.
/// A server's setup: time its unit spends preparing before a job, in a state of its own (neither
/// processing nor idle), during which it holds the part and takes no other.
///
/// Without `on_change_of`, every job has one: BPSim's `SetUpTime`, "the time expended prior to
/// performing the actual work" (BPSim 2.0, 6.2.4), part of each job's duration. With it, a
/// changeover: a unit sets up only when the part's value of that property differs from that of
/// the part it served last (its first part needs none), taking the time `by_change` gives for
/// that change, or else `time`. Failures come during a setup as during the job; an
/// interruptible one stops it, and it resumes after the repair.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    pub time: Dist,
    /// The property whose value changing makes a changeover.
    pub on_change_of: Option<String>,
    /// Times for particular changes, from one value to another (sequence-dependent setups).
    pub by_change: Vec<Change>,
}

/// The time of a changeover from parts whose property value is `from` to parts whose value is
/// `to` (values as text: `A`, `2`, `true`).
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub from: String,
    pub to: String,
    pub time: Dist,
}

impl Setup {
    /// A setup of `time` before every job (BPSim `SetUpTime`).
    pub fn every_job(time: Dist) -> Self {
        Self {
            time,
            on_change_of: None,
            by_change: Vec::new(),
        }
    }

    /// The time of a changeover from `from` to `to`.
    pub fn time_for(&self, from: &str, to: &str) -> &Dist {
        self.by_change
            .iter()
            .find(|c| c.from == from && c.to == to)
            .map_or(&self.time, |c| &c.time)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transfer {
    pub time: Dist,
    pub holds_sender: bool,
}

impl Transfer {
    /// A transfer of `time` that holds the sending unit.
    pub fn holding(time: Dist) -> Self {
        Self {
            time,
            holds_sender: true,
        }
    }
}

/// How long an entity waits at a server before it gives up, drawn for each entity as it enters
/// (a StochCraft vendor extension, `stochcraft:patience`, since BPSim has no parameter for it).
/// The clock starts as it enters the server, before any transfer into it.
///
/// An entity that gives up is counted as abandoned (not as left) and takes the server's way out
/// marked [`Connection::abandoned`], or leaves the model if there is none. A unit taking it at
/// the deadline itself serves it. A patience of 0 gives up unless a unit is free at once
/// (Erlang's loss system); negative draws count as 0. The model is Garnett, Mandelbaum and
/// Reiman's M/M/N+M (Manufacturing & Service Operations Management 4(3), 2002) when patience,
/// arrivals and service are exponential.
#[derive(Debug, Clone, PartialEq)]
pub struct Patience {
    pub time: Dist,
    pub until: PatienceUntil,
}

/// What an entity's patience must see happen in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PatienceUntil {
    /// Its service starting (a caller who hangs up while on hold): once a unit takes it, it
    /// stays.
    #[default]
    Start,
    /// Its service being done (a client that times out): if the deadline passes while a unit
    /// serves it, or while the unit holds it blocked after service, it gives up then, and a
    /// stand-in (a new entity with its properties, creation time and last server) takes the
    /// abandoned way at once; the unit finishes the job anyway (wasted work) and discards it.
    Done,
}

impl PatienceUntil {
    /// The name in files and on the wire.
    pub fn name(self) -> &'static str {
        match self {
            PatienceUntil::Start => "start",
            PatienceUntil::Done => "done",
        }
    }

    /// The value named `s`.
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "start" => Some(PatienceUntil::Start),
            "done" => Some(PatienceUntil::Done),
            _ => None,
        }
    }
}

/// A health check on a server's units, as a load balancer makes on the servers behind it (a
/// StochCraft vendor extension, `stochcraft:healthCheck`, since BPSim has nothing for it).
///
/// Each unit is checked on its own, every `interval` (the first check one draw after the
/// start). A check fails when more entities wait than `limit` per working unit (up, not out and
/// on shift): a probe that would wait longer than its timeout, so `limit` is the timeout times
/// one unit's service rate. A unit that fails is taken out for `restart`, then comes back; one
/// off shift, failed or already out is not checked. Overload judged on the shared queue is
/// the signal a timed-out probe gives, since every unit serves the one queue. Taking units out
/// leaves fewer to serve the queue, which grows, so more fail: once tipped over, the server can
/// stay collapsed long after the cause has gone (a metastable failure).
///
/// Failures ([`Breakdown`]) keep coming in elapsed time while a unit is out. A server with a
/// health check needs an unlimited buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthCheck {
    /// The time between one unit's checks.
    pub interval: Dist,
    /// The most entities waiting per working unit that still passes (0 or more).
    pub limit: f64,
    /// How long a unit taken out is out.
    pub restart: Dist,
    /// Keep at least this many units in service: a failed check takes no unit out when that
    /// would leave fewer working. `None`: no floor.
    pub keep: Option<u32>,
    /// What happens to a job under way on a unit taken out.
    pub on_takeout: OnTakeout,
}

/// What happens to a job under way on a unit a health check takes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnTakeout {
    /// The connection drops and the client tries again: the entity goes back to the head of the
    /// queue at once, for any unit to start afresh (a new attempt, with a fresh patience if the
    /// server has one). The work done on it is wasted.
    #[default]
    Retry,
    /// The entity waits on its unit and the job resumes with what was left after the restart,
    /// as a job a failure interrupts does.
    Resume,
}

impl OnTakeout {
    /// The name in files and on the wire.
    pub fn name(self) -> &'static str {
        match self {
            OnTakeout::Retry => "retry",
            OnTakeout::Resume => "resume",
        }
    }

    /// The value named `s`.
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "retry" => Some(OnTakeout::Retry),
            "resume" => Some(OnTakeout::Resume),
            _ => None,
        }
    }
}

/// How an exclusive gateway chooses the way each entity takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Routing {
    /// By the first condition that holds, else the default; with no conditions, drawn by the
    /// connections' probabilities (BPMN and BPSim).
    #[default]
    Standard,
    /// To the way with the shortest queue that has room (a StochCraft vendor extension,
    /// `stochcraft:routing="shortestQueue"`): only ways whose next element has room in its
    /// buffer are candidates, and the entity takes the one with the fewest entities waiting or
    /// in transit there (not counting those in service); a tie goes to the way taken longest ago
    /// (the first in model order if none has been taken yet), so equal ways take turns. With no
    /// room on any way, the entity waits until a place frees on one, and the rule is applied
    /// then.
    ///
    /// The element sending into the gateway does the waiting, as if the ways were its own: a
    /// server stays blocked after its job until some way has room (and a transfer into the way
    /// chosen then holds its unit, if the transfer holds senders), and a source loses its
    /// arrival when no way has room. Probabilities, conditions and a default have no effect.
    /// A plain waiter for the same buffer (another sender with only that way) gets a freed
    /// place before an entity routed here.
    ShortestQueue,
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
    /// A unit that fails while holding a finished entity, waiting for room in the next buffer
    /// (or carrying it there), still hands it on as soon as there is room, under repair
    /// (`true`, the default); otherwise it keeps it until repaired
    /// (`stochcraft:breakdown handsOnWhileDown="false"`). Either way a unit under repair takes no
    /// new entity, and one left empty keeps its place in the buffer free until repaired.
    pub hands_on_while_down: bool,
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
            ElementKind::Sink { .. } => "sink",
            ElementKind::ExclusiveGateway { .. } => "exclusive_gateway",
            ElementKind::ParallelGateway => "parallel_gateway",
            ElementKind::Batch { .. } => "batch",
            ElementKind::Unbatch => "unbatch",
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
                service,
                breakdown,
                transfer,
                setup,
                planned_run_time,
                patience,
                health_check,
                ..
            } => {
                let mut d = vec![("service", service)];
                if let Some(s) = setup {
                    d.push(("setup", &s.time));
                    d.extend(s.by_change.iter().map(|c| ("setup", &c.time)));
                }
                if let Some(b) = breakdown {
                    d.push(("time_to_failure", &b.time_to_failure));
                    d.push(("repair", &b.repair));
                }
                if let Some(t) = transfer {
                    d.push(("transfer", &t.time));
                }
                if let Some(p) = planned_run_time {
                    d.push(("planned_run_time", p));
                }
                if let Some(p) = patience {
                    d.push(("patience", &p.time));
                }
                if let Some(h) = health_check {
                    d.push(("check_interval", &h.interval));
                    d.push(("restart", &h.restart));
                }
                d
            }
            ElementKind::SharedTask { service, .. } => vec![("service", service)],
            ElementKind::Delay { duration } => vec![("duration", duration)],
            ElementKind::Sink { .. }
            | ElementKind::ExclusiveGateway { .. }
            | ElementKind::ParallelGateway
            | ElementKind::Batch { .. }
            | ElementKind::Unbatch => vec![],
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
    /// Text drawn for each entity, each value with its share (a product mix: `A` half the time,
    /// `B` and `C` a quarter each). BPSim writes it as a discrete `UserDistribution` whose data
    /// points are `StringParameter`s (BPSim 2.0, 6.2.8 and 6.3.5.13).
    Choice(Vec<(f64, String)>),
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
            PropertyValue::Choice(_) => "a mix of text values",
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
    /// Into a parallel join (an assembly): how many entities from this connection go into each
    /// one the join sends on (a bill of materials, the StochCraft vendor extension
    /// `stochcraft:quantity`). `None` is 1.
    pub quantity: Option<u32>,
    /// Entities taking this connection are rework (ISO 22400-2's rework quantity): they failed
    /// their quality requirements but go back to be worked on again. Each counts against the
    /// last server it finished at, for the OEE quality results. A StochCraft vendor extension
    /// (`stochcraft:rework`).
    pub rework: bool,
    /// The way out of a server with a [`Patience`] that entities giving up take (a call back
    /// later, a retry), apart from its ordinary way out. A StochCraft vendor extension
    /// (`stochcraft:abandoned`).
    pub abandoned: bool,
}

/// The customer demand a sink serves, for takt: `quantity` parts each `per` period. Takt is the
/// available time divided by the demand over it (the Lean Enterprise Institute's Lean Lexicon:
/// "the available production time divided by customer demand"). The available time is the
/// measured window's, or with `calendar` only the time that calendar is active in it. A
/// StochCraft vendor extension (`stochcraft:demand`).
#[derive(Debug, Clone, PartialEq)]
pub struct Demand {
    pub quantity: f64,
    pub per: DemandPer,
    pub calendar: Option<ElementId>,
}

/// The period a [`Demand`]'s quantity is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemandPer {
    /// The whole measured window.
    Run,
    /// Each day (24 hours) of the measured window.
    Day,
    /// Each week (7 days) of the measured window.
    Week,
    /// Each active period of the demand's calendar (a shift); one partly in the measured window
    /// counts by the share of it inside.
    Shift,
}

impl DemandPer {
    /// The name in files and on the wire.
    pub fn name(self) -> &'static str {
        match self {
            DemandPer::Run => "run",
            DemandPer::Day => "day",
            DemandPer::Week => "week",
            DemandPer::Shift => "shift",
        }
    }

    /// The period named `s`.
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "run" => Some(DemandPer::Run),
            "day" => Some(DemandPer::Day),
            "week" => Some(DemandPer::Week),
            "shift" => Some(DemandPer::Shift),
            _ => None,
        }
    }
}

/// A sink that takes scrap (ISO 22400-2's scrap quantity: produced, failed its quality
/// requirements, scrapped or recycled), a StochCraft vendor extension (`stochcraft:scrap`). Each
/// entity reaching it counts against `charged_to` if given (an inspection that rejects work done
/// upstream), else the last server it finished at, which it carries as it does its properties
/// (a copy keeps its original's; a batch is its first part's, and its parts keep their own
/// once unbatched). One that has not finished at a server counts against none.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scrap {
    pub charged_to: Option<ElementId>,
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
            quantity: None,
            rework: false,
            abandoned: false,
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
    /// An event that ends each replication early (a terminating experiment); the warm-up plus
    /// the duration is then its time limit. A StochCraft vendor extension on the scenario
    /// (`stochcraft:stop`), since BPSim runs for a fixed duration.
    pub stop: Option<Stop>,
}

/// What ends a replication before its time limit. Checked after everything that happens at one
/// instant has happened, so the replication ends at that instant.
#[derive(Debug, Clone, PartialEq)]
pub enum Stop {
    /// Once `count` entities have left `element` since time 0, warm-up included (for a sink:
    /// arrived there): a target count.
    Count { element: ElementId, count: u64 },
    /// The first time `element`, a task, holds no entity (none waiting, none being served or
    /// delayed) after it has taken one in: a backlog cleared.
    Empty { element: ElementId },
}

impl Stop {
    /// The element whose state it watches.
    pub fn element(&self) -> &ElementId {
        match self {
            Stop::Count { element, .. } | Stop::Empty { element } => element,
        }
    }
}

/// An expert's three-point estimate that an element's time distribution was fitted from: the
/// parameter's pedigree (NASA-STD-7009B [M&S 24], docs/research.md), kept with the model as a
/// StochCraft vendor extension (`stochcraft:estimate`). The distribution is the one used; the
/// estimate records where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Estimate {
    pub three_point: ThreePoint,
    /// Who gave it or where it comes from (free text; may be empty).
    pub source: String,
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
            stop: None,
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
    /// Where elements are drawn: the shapes of the file's diagram (BPMN DI, XPDL graphics) and
    /// those placed since. An element without one is placed by [`Model::placed`].
    pub layout: BTreeMap<ElementId, Bounds>,
    /// The three-point estimates elements' time distributions were fitted from (see
    /// [`Model::estimated`] for which distribution).
    pub estimates: BTreeMap<ElementId, Estimate>,
}

/// Where an element is drawn: its shape's top-left corner and size, in the diagram's units
/// (BPMN DI's, with y growing downwards).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub fn centre(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }
}

impl ElementKind {
    /// The size BPMN editors draw this kind's shape: events 36 across, gateways 50, tasks
    /// 100 by 80.
    pub fn shape_size(&self) -> (f64, f64) {
        match self {
            ElementKind::Source { .. } | ElementKind::Sink { .. } => (36.0, 36.0),
            ElementKind::ExclusiveGateway { .. } | ElementKind::ParallelGateway => (50.0, 50.0),
            ElementKind::Server { .. }
            | ElementKind::SharedTask { .. }
            | ElementKind::Delay { .. }
            | ElementKind::Batch { .. }
            | ElementKind::Unbatch => (100.0, 80.0),
        }
    }
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
    /// A server's power in a state that is negative or not a number.
    BadPower {
        state: &'static str,
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
    /// A setup has times for particular changes but no property whose changes it watches.
    ChangesWithoutProperty,
    /// A scrap sink counts its scrap against an element that is not a server.
    ScrapNotAgainstServer {
        id: String,
    },
    /// A planned run time per item whose mean is not above 0.
    PlannedRunTimeNotPositive,
    /// A patience whose mean is below 0.
    PatienceNegative,
    /// A connection marked for entities that give up leaves an element with no patience.
    AbandonedWithoutPatience {
        connection: ElementId,
    },
    /// Several ways out of one server are marked for entities that give up.
    SeveralAbandonedWays {
        count: usize,
    },
    /// A health check on a server with a finite buffer (not supported yet).
    HealthCheckWithFiniteBuffer,
    /// A health check whose limit is not a number of 0 or more.
    HealthCheckLimitNotValid,
    /// A health check whose time between checks has a mean that is not above 0.
    HealthCheckIntervalNotPositive,
    /// A demand whose quantity is not a number above 0.
    DemandNotPositive,
    /// A demand per shift with no calendar to say what a shift is.
    ShiftWithoutCalendar,
    /// A demand's calendar does not exist.
    DemandCalendarUnknown {
        id: ElementId,
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
    /// A probability, condition or default where it has no effect, and why.
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
    /// A batch of no entities.
    ZeroBatchSize,
    /// A bill of materials asking for no entities from a connection.
    ZeroQuantity,
    /// A quantity on a connection that does not go into a parallel gateway, where it does
    /// nothing.
    QuantityIgnored,
    /// The model splits batches but makes none.
    NothingToUnbatch,
    /// The experiment's stop condition cannot be used, and why.
    BadStop {
        reason: &'static str,
    },
    /// The experiment has both a stop condition and a warm-up: a replication whose stop comes
    /// before the warm-up ends has no statistics after the warm-up, and is left out of the
    /// results ([`Problem::EndedInWarmup`]).
    StopWithWarmup,
    /// Found from a run's results, not by [`Model::validate`]: `replications` of the `of` run
    /// stopped before the warm-up ended, so their statistics (all from the warm-up) are left out
    /// of every metric; their stop times still count.
    EndedInWarmup {
        replications: u64,
        of: u64,
    },
    /// A server fed straight from sources with steady arrivals is offered at least `load` times
    /// the work its units can do (arrival rate × mean service time ÷ units, at least 1): its
    /// queue grows without limit, so the run has no steady state.
    Overloaded {
        load: f64,
    },
    /// Found from a run's results, not by [`Model::validate`]: a server was busy nearly all the
    /// time it was up (`utilisation`, of the time up) and, on average, at least `backlog`
    /// entities were still waiting at the end against `left` that left during the run. The
    /// queue grows with the run, so its means describe no steady state.
    NoSteadyState {
        backlog: f64,
        left: f64,
        utilisation: f64,
    },
    /// A three-point estimate for an element with no time distribution to fit (or no such
    /// element).
    EstimateIgnored,
    /// A three-point estimate that cannot be fitted.
    BadEstimate {
        error: FitError,
    },
    /// The element's distribution is no longer the one fitted from its estimate (it was changed
    /// since): the estimate no longer says where the distribution came from.
    EstimateMismatch,
    /// A time (`parameter`) drawn from a normal distribution whose mean is less than
    /// [`NORMAL_1_PERCENT`] standard deviations: more than 1% of its draws are below 0, and each
    /// counts as 0, so the times are longer on average than the mean given.
    NormalOftenNegative {
        parameter: &'static str,
    },
}

/// Φ⁻¹(0.99): a normal distribution draws below 0 more than 1% of the time when its mean is less
/// than this many standard deviations ([`Problem::NormalOftenNegative`]).
pub const NORMAL_1_PERCENT: f64 = 2.326_347_874_040_840_8;

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
            Problem::BadPower { .. } => "bad_power",
            Problem::BadProperty { .. } => "bad_property",
            Problem::PropertyTypeConflict { .. } => "property_type_conflict",
            Problem::PropertyValueType { .. } => "property_value_type",
            Problem::BadExpression { .. } => "bad_expression",
            Problem::UnknownProperty { .. } => "unknown_property",
            Problem::ChangesWithoutProperty => "changes_without_property",
            Problem::ScrapNotAgainstServer { .. } => "scrap_not_against_server",
            Problem::PlannedRunTimeNotPositive => "planned_run_time_not_positive",
            Problem::PatienceNegative => "patience_negative",
            Problem::AbandonedWithoutPatience { .. } => "abandoned_without_patience",
            Problem::SeveralAbandonedWays { .. } => "several_abandoned_ways",
            Problem::HealthCheckWithFiniteBuffer => "health_check_with_finite_buffer",
            Problem::HealthCheckLimitNotValid => "health_check_limit_not_valid",
            Problem::HealthCheckIntervalNotPositive => "health_check_interval_not_positive",
            Problem::DemandNotPositive => "demand_not_positive",
            Problem::ShiftWithoutCalendar => "shift_without_calendar",
            Problem::DemandCalendarUnknown { .. } => "demand_calendar_unknown",
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
            Problem::ZeroBatchSize => "zero_batch_size",
            Problem::ZeroQuantity => "zero_quantity",
            Problem::QuantityIgnored => "quantity_ignored",
            Problem::NothingToUnbatch => "nothing_to_unbatch",
            Problem::BadStop { .. } => "bad_stop",
            Problem::StopWithWarmup => "stop_with_warmup",
            Problem::EndedInWarmup { .. } => "ended_in_warmup",
            Problem::Overloaded { .. } => "overloaded",
            Problem::NoSteadyState { .. } => "no_steady_state",
            Problem::EstimateIgnored => "estimate_ignored",
            Problem::BadEstimate { .. } => "bad_estimate",
            Problem::EstimateMismatch => "estimate_mismatch",
            Problem::NormalOftenNegative { .. } => "normal_often_negative",
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
            Problem::BadPower { state } => write!(
                f,
                "the power while {state} must be a number of kilowatts of at least 0"
            ),
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
            Problem::ChangesWithoutProperty => write!(
                f,
                "its setup has times for changes from one value to another but no property to watch"
            ),
            Problem::ScrapNotAgainstServer { id } => write!(
                f,
                "its scrap counts against `{id}`, which is not a task with a resource of its own"
            ),
            Problem::DemandNotPositive => {
                write!(f, "its demand must be a number of parts above 0")
            }
            Problem::ShiftWithoutCalendar => write!(
                f,
                "its demand is per shift but names no calendar to say when the shifts are"
            ),
            Problem::DemandCalendarUnknown { id } => {
                write!(
                    f,
                    "its demand names the calendar `{id}`, which does not exist"
                )
            }
            Problem::PlannedRunTimeNotPositive => {
                write!(f, "its planned run time per item must have a mean above 0")
            }
            Problem::PatienceNegative => {
                write!(f, "its patience must have a mean of 0 or more")
            }
            Problem::AbandonedWithoutPatience { connection } => write!(
                f,
                "connection `{connection}` is the way for entities that give up, but they come \
                 only from a task with a resource of its own and a patience"
            ),
            Problem::SeveralAbandonedWays { count } => write!(
                f,
                "{count} of its ways out are for entities that give up; at most one can be"
            ),
            Problem::HealthCheckWithFiniteBuffer => write!(
                f,
                "a health check needs an unlimited waiting room; remove the buffer's limit or the health check"
            ),
            Problem::HealthCheckLimitNotValid => write!(
                f,
                "its health check's limit (entities waiting per working unit) must be a number of 0 or more"
            ),
            Problem::HealthCheckIntervalNotPositive => write!(
                f,
                "its health check's time between checks must have a mean above 0"
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
            Problem::ZeroBatchSize => write!(f, "a batch needs at least 1 entity"),
            Problem::ZeroQuantity => {
                write!(
                    f,
                    "a bill of materials needs at least 1 entity from each connection"
                )
            }
            Problem::QuantityIgnored => write!(
                f,
                "this quantity is ignored: only a parallel gateway takes several entities from one connection"
            ),
            Problem::NothingToUnbatch => write!(
                f,
                "no element makes batches, so this one passes every entity through"
            ),
            Problem::BadStop { reason } => write!(f, "the run cannot stop here: {reason}"),
            Problem::StopWithWarmup => write!(
                f,
                "the run stops here, which can happen before the warm-up ends: a replication that stops during the warm-up has no statistics after it and is left out of the results (its stop time still counts)"
            ),
            Problem::EndedInWarmup { replications, of } if replications == of => write!(
                f,
                "every replication stopped before the warm-up ended, so none has statistics after it and no metric is reported (the stop times still are); shorten the warm-up or stop later"
            ),
            Problem::Overloaded { load } => write!(
                f,
                "its arrivals bring {:.0}% of the work its units can do (arrival rate × mean service time ÷ units), so its queue grows without limit and the results describe no steady state; add units or shorten the service",
                load * 100.0
            ),
            Problem::NoSteadyState {
                backlog,
                left,
                utilisation,
            } => write!(
                f,
                "busy {:.1}% of the time and, on average, at least {backlog:.0} entities still waiting at the end against {left:.0} that left: work arrives faster than it is done, so the queue grows with the run length and its means describe no steady state (or the run ends with a backlog it could not clear)",
                utilisation * 100.0
            ),
            Problem::EndedInWarmup { replications, of } => write!(
                f,
                "{replications} of {of} replications stopped before the warm-up ended; their statistics, all from the warm-up, are left out of every metric (their stop times still count)"
            ),
            Problem::EstimateIgnored => write!(
                f,
                "has a three-point estimate but no time distribution to fit it to; it is ignored"
            ),
            Problem::BadEstimate { error } => write!(f, "three-point estimate: {error}"),
            Problem::EstimateMismatch => write!(
                f,
                "the time distribution is no longer the one fitted from its three-point estimate"
            ),
            Problem::NormalOftenNegative { parameter } => write!(
                f,
                "{parameter}: this normal distribution draws below 0 more than 1 time in 100 (its mean is less than 2.33 standard deviations), and each such time counts as 0, so the times are longer on average than its mean; use a truncated normal with min 0, or a log-normal"
            ),
        }
    }
}

/// Whether no draw from `d` is negative, so its mean is the mean time it gives (durations
/// count a negative draw as 0, which would make the true mean larger).
fn never_negative(d: &Dist) -> bool {
    match d {
        Dist::Constant(v) => *v >= 0.0,
        Dist::Uniform { min, .. }
        | Dist::Triangular { min, .. }
        | Dist::TruncatedNormal { min, .. } => *min >= 0.0,
        Dist::Normal { .. } => false,
        Dist::User { points, .. } => points.iter().all(|(_, v)| *v >= 0.0),
        Dist::Exponential { .. }
        | Dist::Poisson { .. }
        | Dist::Erlang { .. }
        | Dist::LogNormal { .. }
        | Dist::Gamma { .. }
        | Dist::Beta { .. }
        | Dist::Weibull { .. }
        | Dist::Binomial { .. } => true,
    }
}

/// Whether `a` and `b` are the same distribution up to the rounding of a file's numbers (a
/// fitted distribution read back from a file, say).
fn same_fit(a: &Dist, b: &Dist) -> bool {
    let near = |x: f64, y: f64| (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0);
    match (a, b) {
        (
            Dist::Triangular { min, mode, max },
            Dist::Triangular {
                min: m2,
                mode: o2,
                max: x2,
            },
        ) => near(*min, *m2) && near(*mode, *o2) && near(*max, *x2),
        (
            Dist::LogNormal { mean, std_dev },
            Dist::LogNormal {
                mean: m2,
                std_dev: s2,
            },
        ) => near(*mean, *m2) && near(*std_dev, *s2),
        (
            Dist::Weibull { shape, scale },
            Dist::Weibull {
                shape: k2,
                scale: l2,
            },
        ) => near(*shape, *k2) && near(*scale, *l2),
        (
            Dist::User { points, discrete },
            Dist::User {
                points: p2,
                discrete: d2,
            },
        ) => {
            discrete == d2
                && points.len() == p2.len()
                && points
                    .iter()
                    .zip(p2)
                    .all(|((p, v), (q, w))| near(*p, *q) && near(*v, *w))
        }
        _ => false,
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
        // The ways entities take after their service: all but a patient server's way for those
        // who give up, which entities leave by on their own (not a unit holding them).
        let mut served: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
        let patient = |id: &ElementId| {
            elements.get(id).is_some_and(|e| {
                matches!(
                    e.kind,
                    ElementKind::Server {
                        patience: Some(_),
                        ..
                    }
                )
            })
        };
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
                && matches!(from.kind, ElementKind::Sink { .. })
            {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::OutOfSink {
                        connection: c.id.clone(),
                    },
                ));
                ok = false;
            }
            if c.abandoned && !patient(&c.from) {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::AbandonedWithoutPatience {
                        connection: c.id.clone(),
                    },
                ));
            }
            if ok {
                outgoing.entry(&c.from).or_default().push(&c.to);
                if !(c.abandoned && patient(&c.from)) {
                    served.entry(&c.from).or_default().push(&c.to);
                }
            }
        }
        for e in &self.elements {
            let outs = served.get(&e.id).map_or(0, Vec::len);
            let abandoned = self
                .connections
                .iter()
                .filter(|c| c.abandoned && c.from == e.id)
                .count();
            if abandoned > 1 && patient(&e.id) {
                out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::SeveralAbandonedWays { count: abandoned },
                ));
            }
            match e.kind {
                ElementKind::Sink { .. } => {}
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
            if let ElementKind::Batch { size: 0 } = e.kind {
                out.push(Diagnostic::error(Some(&e.id), Problem::ZeroBatchSize));
            }
            if matches!(e.kind, ElementKind::Unbatch)
                && !self
                    .elements
                    .iter()
                    .any(|b| matches!(b.kind, ElementKind::Batch { .. }))
            {
                out.push(Diagnostic::warning(Some(&e.id), Problem::NothingToUnbatch));
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
            if let ElementKind::Sink {
                scrap: Some(Scrap {
                    charged_to: Some(c),
                }),
                ..
            } = &e.kind
                && !self
                    .elements
                    .iter()
                    .any(|x| x.id == *c && matches!(x.kind, ElementKind::Server { .. }))
            {
                out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::ScrapNotAgainstServer { id: c.to_string() },
                ));
            }
            if let ElementKind::Sink {
                demand: Some(d), ..
            } = &e.kind
            {
                if !(d.quantity.is_finite() && d.quantity > 0.0) {
                    out.push(Diagnostic::error(Some(&e.id), Problem::DemandNotPositive));
                }
                match &d.calendar {
                    None if d.per == DemandPer::Shift => {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::ShiftWithoutCalendar,
                        ));
                    }
                    Some(c) if !self.calendars.iter().any(|x| x.id == *c) => {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::DemandCalendarUnknown { id: c.clone() },
                        ));
                    }
                    _ => {}
                }
            }
            if let ElementKind::Server {
                planned_run_time: Some(d),
                ..
            } = &e.kind
                && d.check().is_ok()
                && !d.mean().is_some_and(|m| m > 0.0)
            {
                out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::PlannedRunTimeNotPositive,
                ));
            }
            if let ElementKind::Server {
                patience: Some(p), ..
            } = &e.kind
                && p.time.check().is_ok()
                && p.time.mean().is_some_and(|m| m < 0.0)
            {
                out.push(Diagnostic::error(Some(&e.id), Problem::PatienceNegative));
            }
            if let ElementKind::Server {
                health_check: Some(h),
                queue_capacity,
                ..
            } = &e.kind
            {
                if queue_capacity.is_some() {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::HealthCheckWithFiniteBuffer,
                    ));
                }
                if h.limit.is_nan() || h.limit < 0.0 {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::HealthCheckLimitNotValid,
                    ));
                }
                if h.interval.check().is_ok() && h.interval.mean().is_some_and(|m| m <= 0.0) {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::HealthCheckIntervalNotPositive,
                    ));
                }
            }
            if let ElementKind::Server { setup: Some(s), .. } = &e.kind {
                match &s.on_change_of {
                    Some(name) if !self.property_types().contains_key(name) => {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::UnknownProperty { name: name.clone() },
                        ));
                    }
                    None if !s.by_change.is_empty() => {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::ChangesWithoutProperty,
                        ));
                    }
                    _ => {}
                }
            }
            if let ElementKind::Server { power: Some(p), .. } = &e.kind {
                for (state, x) in p.values() {
                    if !(x.is_finite() && x >= 0.0) {
                        out.push(Diagnostic::error(Some(&e.id), Problem::BadPower { state }));
                    }
                }
            }
            for (parameter, d) in e.kind.distributions() {
                if let Err(error) = d.check() {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::BadDistribution { parameter, error },
                    ));
                } else if let Dist::Normal { mean, std_dev } = d
                    && *mean < NORMAL_1_PERCENT * std_dev
                {
                    out.push(Diagnostic::warning(
                        Some(&e.id),
                        Problem::NormalOftenNegative { parameter },
                    ));
                }
            }
        }
        self.check_experiment(&mut out);
        self.check_stop(&mut out);
        self.check_estimates(&mut out);
        self.check_properties(&mut out);
        self.check_routing(&mut out);
        self.check_reachability(&outgoing, &mut out);
        self.check_buffers(&served, &mut out);
        self.check_resources(&mut out);
        self.check_load(&mut out);
        // Stable order: errors first, then as found (model order).
        out.sort_by_key(|d| std::cmp::Reverse(d.severity));
        out
    }

    /// Where every element is drawn: its shape in [`Model::layout`], or else a place of its own.
    /// With no layout at all, elements go in columns by their distance from the sources
    /// (breadth first), in model order within a column; elements added to a laid-out model
    /// without a shape go in a row below the rest.
    pub fn placed(&self) -> BTreeMap<ElementId, Bounds> {
        let mut out = BTreeMap::new();
        let unplaced: Vec<&Element> = self
            .elements
            .iter()
            .filter(|e| !self.layout.contains_key(&e.id))
            .collect();
        if unplaced.is_empty() {
            for e in &self.elements {
                if let Some(b) = self.layout.get(&e.id) {
                    out.insert(e.id.clone(), *b);
                }
            }
            return out;
        }
        let shaped: Vec<Bounds> = self
            .elements
            .iter()
            .filter_map(|e| self.layout.get(&e.id).copied())
            .collect();
        if shaped.is_empty() {
            return self.columns();
        }
        for e in &self.elements {
            if let Some(b) = self.layout.get(&e.id) {
                out.insert(e.id.clone(), *b);
            }
        }
        let left = shaped.iter().map(|b| b.x).fold(f64::INFINITY, f64::min);
        let bottom = shaped
            .iter()
            .map(|b| b.y + b.height)
            .fold(f64::NEG_INFINITY, f64::max);
        for (i, e) in unplaced.iter().enumerate() {
            let (w, h) = e.kind.shape_size();
            let (cx, cy) = (left + 50.0 + 150.0 * i as f64, bottom + 100.0);
            out.insert(
                e.id.clone(),
                Bounds {
                    x: cx - w / 2.0,
                    y: cy - h / 2.0,
                    width: w,
                    height: h,
                },
            );
        }
        out
    }

    /// Columns by distance from the sources, rows in model order within a column.
    fn columns(&self) -> BTreeMap<ElementId, Bounds> {
        let mut next: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
        for c in &self.connections {
            next.entry(&c.from).or_default().push(&c.to);
        }
        let mut depth: BTreeMap<&ElementId, usize> = BTreeMap::new();
        let mut queue: VecDeque<&ElementId> = VecDeque::new();
        for e in &self.elements {
            if matches!(e.kind, ElementKind::Source { .. }) {
                depth.insert(&e.id, 0);
                queue.push_back(&e.id);
            }
        }
        while let Some(id) = queue.pop_front() {
            let d = depth.get(id).copied().unwrap_or(0);
            for n in next.get(id).into_iter().flatten() {
                if !depth.contains_key(n) {
                    depth.insert(n, d + 1);
                    queue.push_back(n);
                }
            }
        }
        let last = depth.values().copied().max().map_or(0, |m| m + 1);
        let mut rows: BTreeMap<usize, usize> = BTreeMap::new();
        let mut out = BTreeMap::new();
        for e in &self.elements {
            let col = depth.get(&e.id).copied().unwrap_or(last);
            let row = rows.entry(col).or_insert(0);
            let (cx, cy) = (100.0 + 180.0 * col as f64, 100.0 + 140.0 * *row as f64);
            let (w, h) = e.kind.shape_size();
            out.insert(
                e.id.clone(),
                Bounds {
                    x: cx - w / 2.0,
                    y: cy - h / 2.0,
                    width: w,
                    height: h,
                },
            );
            *row += 1;
        }
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
            if let ElementKind::Server { availability, .. } = &e.kind {
                refs.extend(availability.iter().map(|t| (&e.id, &t.calendar)));
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
                PropertyValue::Text(_) | PropertyValue::Choice(_) => PropertyType::String,
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
                        PropertyValue::Choice(_) => ty == PropertyType::String,
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
                    PropertyValue::Choice(values) => {
                        let shares_ok = values.iter().all(|(s, _)| s.is_finite() && *s >= 0.0) && {
                            let total = values.iter().map(|(s, _)| s).sum::<f64>();
                            total > 0.0 && total.is_finite()
                        };
                        if !shares_ok {
                            out.push(Diagnostic::error(
                                at,
                                Problem::BadProperty {
                                    name: name.clone(),
                                    reason: "needs shares that are not negative and add up to more than 0",
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
                match c.quantity {
                    Some(0) => out.push(Diagnostic::error(at, Problem::ZeroQuantity)),
                    Some(_)
                        if !self
                            .element(&c.to)
                            .is_some_and(|t| matches!(t.kind, ElementKind::ParallelGateway)) =>
                    {
                        out.push(Diagnostic::warning(at, Problem::QuantityIgnored));
                    }
                    _ => {}
                }
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
            let ElementKind::ExclusiveGateway { default, routing } = &e.kind else {
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
            if *routing == Routing::ShortestQueue {
                let reason = "the gateway routes to the shortest queue";
                for c in &outs {
                    let what = if c.condition.is_some() {
                        "this condition"
                    } else if c.probability.is_some() {
                        "this probability"
                    } else if default.as_ref() == Some(&c.id) {
                        "this default"
                    } else {
                        continue;
                    };
                    out.push(Diagnostic::warning(
                        Some(&c.id),
                        Problem::RoutingIgnored { what, reason },
                    ));
                }
            } else if outs.iter().any(|c| c.condition.is_some()) {
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

    /// Servers offered more work than they can do, where that is plain from the model: arrivals
    /// straight from sources whose time between arrivals never changes (no limit, no
    /// calendars) and is never negative, into a server with an unlimited queue. Arrivals by any
    /// other way are left out, so the load found is a lower bound and the warning conservative.
    /// (Negative service draws count as 0, which only makes the true load higher.)
    fn check_load(&self, out: &mut Vec<Diagnostic>) {
        for e in &self.elements {
            // Entities that give up keep a patient server's queue from growing without limit.
            let ElementKind::Server {
                capacity,
                service,
                queue_capacity: None,
                patience: None,
                health_check: None,
                ..
            } = &e.kind
            else {
                continue;
            };
            let Some(mean_service) = service.mean().filter(|m| m.is_finite() && *m > 0.0) else {
                continue;
            };
            let rate: f64 = self
                .connections
                .iter()
                .filter(|c| c.to == e.id)
                .filter_map(|c| match &self.element(&c.from)?.kind {
                    ElementKind::Source {
                        interarrival,
                        limit: None,
                        timed,
                        idle_outside: false,
                    } if timed.is_empty() && never_negative(interarrival) => interarrival
                        .mean()
                        .filter(|m| m.is_finite() && *m > 0.0)
                        .map(|m| 1.0 / m),
                    _ => None,
                })
                .sum();
            let load = rate * mean_service / f64::from((*capacity).max(1));
            if load >= 1.0 {
                out.push(Diagnostic::warning(
                    Some(&e.id),
                    Problem::Overloaded { load },
                ));
            }
        }
    }

    /// The stop condition names an element that can meet it.
    fn check_stop(&self, out: &mut Vec<Diagnostic>) {
        let Some(stop) = &self.experiment.stop else {
            return;
        };
        let id = stop.element();
        let reason = match (stop, self.element(id).map(|e| &e.kind)) {
            (_, None) => Some("no element has this id"),
            (Stop::Count { count: 0, .. }, _) => Some("the count must be 1 or more"),
            (Stop::Count { .. }, Some(_)) => None,
            (
                Stop::Empty { .. },
                Some(
                    ElementKind::Server { .. }
                    | ElementKind::SharedTask { .. }
                    | ElementKind::Delay { .. },
                ),
            ) => None,
            (Stop::Empty { .. }, Some(_)) => Some("only a task can empty"),
        };
        if let Some(reason) = reason {
            out.push(Diagnostic::error(Some(id), Problem::BadStop { reason }));
        } else if self.experiment.warmup > 0.0 {
            out.push(Diagnostic::warning(Some(id), Problem::StopWithWarmup));
        }
    }

    /// Each estimate is for an element with a time distribution, can be fitted, and is what the
    /// element uses.
    fn check_estimates(&self, out: &mut Vec<Diagnostic>) {
        for (id, estimate) in &self.estimates {
            let Some(dist) = self.estimated(id) else {
                out.push(Diagnostic::warning(Some(id), Problem::EstimateIgnored));
                continue;
            };
            match estimate.three_point.fit() {
                Err(error) => out.push(Diagnostic::warning(
                    Some(id),
                    Problem::BadEstimate { error },
                )),
                Ok(fitted) if !same_fit(&fitted, dist) => {
                    out.push(Diagnostic::warning(Some(id), Problem::EstimateMismatch));
                }
                Ok(_) => {}
            }
        }
    }

    /// The time distribution of element `id` that a three-point estimate is for: a source's time
    /// between arrivals, a task's service time, a delay's duration.
    pub fn estimated(&self, id: &ElementId) -> Option<&Dist> {
        match &self.element(id)?.kind {
            ElementKind::Source { interarrival, .. } => Some(interarrival),
            ElementKind::Server { service, .. } | ElementKind::SharedTask { service, .. } => {
                Some(service)
            }
            ElementKind::Delay { duration } => Some(duration),
            _ => None,
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
                // A shortest-queue gateway leaves the waiting to its sender.
                let holds_up = self.element(from).is_some_and(|g| {
                    g.kind.is_gateway()
                        && !matches!(
                            g.kind,
                            ElementKind::ExclusiveGateway {
                                routing: Routing::ShortestQueue,
                                ..
                            }
                        )
                });
                if tos.contains(&&e.id) && holds_up {
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
            .filter(|e| matches!(e.kind, ElementKind::Sink { .. }))
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
            quantity: None,
            rework: false,
            abandoned: false,
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
                    routing: Routing::Standard,
                },
            ),
        );
        m.elements.push(el(
            "scrap",
            ElementKind::Sink {
                scrap: None,
                demand: None,
            },
        ));
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
            el(
                "pick",
                ElementKind::ExclusiveGateway {
                    default: None,
                    routing: Routing::Standard,
                },
            ),
        );
        m.elements.push(el(
            "scrap",
            ElementKind::Sink {
                scrap: None,
                demand: None,
            },
        ));
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

    #[test]
    fn batches_and_bills_of_materials_are_checked() {
        let mut m = mm1();
        m.elements[1].kind = ElementKind::Batch { size: 0 };
        m.connections[0].quantity = Some(2);
        let p = problems(&m);
        assert!(
            p.iter().any(|(s, at, x)| *s == Severity::Error
                && at.as_deref() == Some("machine")
                && x == "ZeroBatchSize"),
            "{p:?}"
        );
        assert!(
            p.iter().any(|(s, at, x)| *s == Severity::Warning
                && at.as_deref() == Some("f1")
                && x == "QuantityIgnored"),
            "{p:?}"
        );
        m.elements[1].kind = ElementKind::Unbatch;
        m.connections[0].quantity = None;
        assert!(problems(&m).iter().any(|(_, _, x)| x == "NothingToUnbatch"));
        m.elements[1].kind = ElementKind::ParallelGateway;
        m.connections[0].quantity = Some(0);
        assert!(
            problems(&m)
                .iter()
                .any(|(s, _, x)| *s == Severity::Error && x == "ZeroQuantity")
        );
        m.connections[0].quantity = Some(3);
        assert_eq!(m.validate(), vec![]);
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
                        transfer: None,
                        available: true,
                        availability: Vec::new(),
                        power: None,
                        setup: None,
                        planned_run_time: None,
                        patience: None,
                        health_check: None,
                    },
                ),
                el(
                    "exit",
                    ElementKind::Sink {
                        scrap: None,
                        demand: None,
                    },
                ),
            ],
            connections: vec![
                link("f1", "arrivals", "machine"),
                link("f2", "machine", "exit"),
            ],
            resources: Vec::new(),
            experiment: Experiment::default(),
            layout: BTreeMap::new(),
            estimates: BTreeMap::new(),
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
        m.elements.push(el(
            "pick",
            ElementKind::ExclusiveGateway {
                default: None,
                routing: Routing::Standard,
            },
        ));
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
            transfer: None,
            available: true,
            availability: Vec::new(),
            power: None,
            setup: None,
            planned_run_time: None,
            patience: None,
            health_check: None,
        };
        let mut m = mm1();
        m.elements.push(el("a", server(Some(1))));
        m.elements.push(el("b", server(Some(1))));
        m.elements.push(el(
            "split",
            ElementKind::ExclusiveGateway {
                default: None,
                routing: Routing::Standard,
            },
        ));
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
                transfer: None,
                available: true,
                availability: Vec::new(),
                power: None,
                setup: None,
                planned_run_time: None,
                patience: None,
                health_check: None,
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
        m.elements.push(el(
            "scrap",
            ElementKind::Sink {
                scrap: None,
                demand: None,
            },
        ));
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
    fn a_normal_time_that_is_often_negative_is_warned_about() {
        let warned = |service: Dist| {
            let mut m = mm1();
            if let ElementKind::Server { service: s, .. } = &mut m.elements[1].kind {
                *s = service;
            }
            let p = problems(&m);
            assert!(m.is_runnable(), "a warning only: {p:#?}");
            p.iter().any(|(s, at, p)| {
                *s == Severity::Warning
                    && at.as_deref() == Some("machine")
                    && p.starts_with("NormalOftenNegative")
            })
        };
        let normal = |mean, std_dev| Dist::Normal { mean, std_dev };
        // P(X < 0) is 16% at mean 1, sd 1, and about 0 at mean 10, sd 1.
        assert!(warned(normal(1.0, 1.0)));
        assert!(!warned(normal(10.0, 1.0)));
        // The edge: 1% below 0 at 2.326 standard deviations.
        assert!(warned(normal(2.32, 1.0)));
        assert!(!warned(normal(2.33, 1.0)));
        // A truncated normal or a log-normal never draws below 0.
        assert!(!warned(Dist::TruncatedNormal {
            mean: 1.0,
            std_dev: 1.0,
            min: 0.0,
            max: 10.0
        }));
        assert!(!warned(Dist::LogNormal {
            mean: 1.0,
            std_dev: 1.0
        }));
        let mut m = mm1();
        if let ElementKind::Server { service, .. } = &mut m.elements[1].kind {
            *service = normal(1.0, 1.0);
        }
        let message = m.validate()[0].to_string();
        assert!(message.contains("service"), "{message}");
        assert!(message.contains("truncated normal"), "{message}");
        assert!(message.contains("log-normal"), "{message}");
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
            elements: vec![el(
                "x",
                ElementKind::Sink {
                    scrap: None,
                    demand: None,
                },
            )],
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

    #[test]
    fn stops_need_an_element_that_can_meet_them() {
        let codes = |stop: Stop| {
            let mut m = mm1();
            m.experiment.stop = Some(stop);
            m.validate()
                .into_iter()
                .filter(|d| d.problem.code() == "bad_stop")
                .map(|d| (d.severity, d.at.map(|a| a.to_string())))
                .collect::<Vec<_>>()
        };
        let id = ElementId::unchecked;
        assert!(
            codes(Stop::Count {
                element: id("exit"),
                count: 10
            })
            .is_empty()
        );
        assert!(
            codes(Stop::Empty {
                element: id("machine")
            })
            .is_empty()
        );
        let bad = vec![(Severity::Error, Some("exit".to_owned()))];
        assert_eq!(
            codes(Stop::Empty {
                element: id("exit")
            }),
            bad
        );
        assert_eq!(
            codes(Stop::Count {
                element: id("exit"),
                count: 0
            }),
            bad
        );
        assert_eq!(
            codes(Stop::Count {
                element: id("gone"),
                count: 1
            }),
            vec![(Severity::Error, Some("gone".to_owned()))]
        );
    }

    #[test]
    fn estimates_must_fit_what_the_element_uses() {
        let three_point = ThreePoint {
            family: Family::Triangular,
            min: 0.5,
            mode: 1.0,
            upper: 2.0,
            upper_is: Upper::Max,
        };
        let mut m = mm1();
        let estimate = Estimate {
            three_point,
            source: String::new(),
        };
        m.estimates
            .insert(ElementId::unchecked("machine"), estimate.clone());
        m.estimates
            .insert(ElementId::unchecked("exit"), estimate.clone());
        let codes = |m: &Model| -> Vec<(Option<String>, String)> {
            m.validate()
                .into_iter()
                .map(|d| (d.at.map(|a| a.to_string()), d.problem.code().to_owned()))
                .collect()
        };
        assert_eq!(
            codes(&m),
            vec![
                (Some("exit".into()), "estimate_ignored".into()),
                (Some("machine".into()), "estimate_mismatch".into()),
            ]
        );
        if let Some(e) = m.elements.iter_mut().find(|e| e.id.as_str() == "machine")
            && let ElementKind::Server { service, .. } = &mut e.kind
        {
            *service = Dist::Triangular {
                min: 0.5,
                mode: 1.0,
                max: 2.0,
            };
        }
        m.estimates.remove(&ElementId::unchecked("exit"));
        assert_eq!(codes(&m), vec![]);
        let mut bad = estimate;
        bad.three_point.mode = 5.0;
        m.estimates.insert(ElementId::unchecked("machine"), bad);
        assert_eq!(
            codes(&m),
            vec![(Some("machine".into()), "bad_estimate".into())]
        );
    }
}
