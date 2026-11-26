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

pub mod agents;
pub mod balance;
pub mod calendar;
pub mod capacity;
pub mod dmn;
pub mod expr;
pub mod messages;
pub mod montecarlo;
pub mod path;
pub mod site;

pub use calendar::{Calendar, DateTime, Timed};
pub use montecarlo::{
    Control, McCorrelation, McInput, McOutput, McStop, McTarget, MonteCarlo, Sampling,
};
pub use path::{Frame, Path, Segment};
pub use site::{Junction, JunctionKind, Place, Point, Scenery, Site, Track};

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

/// The unit lengths are given in (a conveyor's length, speed and item size), set on the
/// experiment beside its [`TimeUnit`]. A speed is in this unit per the experiment's time unit.
/// A StochCraft vendor extension on the scenario (`stochcraft:lengthUnit`); BPSim has none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LengthUnit {
    Millimetres,
    Centimetres,
    #[default]
    Metres,
    Kilometres,
    Inches,
    Feet,
}

impl LengthUnit {
    /// The unit's short name, as it is written in files and shown (`m` for metres).
    pub fn name(self) -> &'static str {
        match self {
            LengthUnit::Millimetres => "mm",
            LengthUnit::Centimetres => "cm",
            LengthUnit::Metres => "m",
            LengthUnit::Kilometres => "km",
            LengthUnit::Inches => "in",
            LengthUnit::Feet => "ft",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "mm" => LengthUnit::Millimetres,
            "cm" => LengthUnit::Centimetres,
            "m" => LengthUnit::Metres,
            "km" => LengthUnit::Kilometres,
            "in" => LengthUnit::Inches,
            "ft" => LengthUnit::Feet,
            _ => return None,
        })
    }
}

/// What an element does: a source, a task with a resource (a server), a task with shared
/// resources, a delay, a sink, a gateway, a batch or an unbatch. Resource pools are separate
/// ([`ResourcePool`]).
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
        /// Processing times for entities with particular values of a property (a time by
        /// product type), `service` applying to the others; `None`: `service` for every entity.
        /// A StochCraft vendor extension (`stochcraft:processingTimeBy`). See [`ProcessingBy`].
        processing_by: Option<ProcessingBy>,
        /// A finite buffer: at most this many entities wait for a unit (not counting those in
        /// service); `None` is unlimited. 0 means no waiting room: an entity gets in only when a
        /// unit is free. When the buffer is full, an element sending an entity here waits
        /// holding it (a server keeps its unit busy: blocking after service), except a source,
        /// whose arrival is lost. A StochCraft vendor extension (`stochcraft:queueCapacity`).
        /// With a [`QueueLimit`] other than `Fixed`, the number means what that says.
        queue_capacity: Option<u32>,
        /// How the waiting room's size is set: fixed at `queue_capacity` (the default), by
        /// timetable, or per working server. See [`QueueLimit`]. Only with a finite
        /// `queue_capacity`.
        queue_limit: QueueLimit,
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
        /// A job under way when the units go off shift finishes first (unless `on_shift_end`
        /// says otherwise), and a job interrupted by a failure resumes after its repair even if
        /// the shift has ended meanwhile. Failures ([`Breakdown`]) keep coming in elapsed time
        /// through the breaks, and a repair goes on through them.
        availability: Vec<Timed<bool>>,
        /// Units while a calendar is active (BPSim `quantity` values `validFor` a calendar on the
        /// task's own resource), the first active one applying, as for a pool; `capacity` is the
        /// units outside them (0 when the file gives only shift values). Empty: always
        /// `capacity`. The units on shift at a time are the shift's units while the units are
        /// available, else none. [`ElementKind::most_units`] is how many units there are in all.
        shifts: Vec<Timed<u32>>,
        /// What happens to a job under way on a unit that goes off shift (fewer units on
        /// shift, or the availability closing): it finishes first (the default), is retried
        /// elsewhere, or resumes when the unit is on shift again. A StochCraft vendor extension
        /// (`stochcraft:onShiftEnd`). See [`OnShiftEnd`].
        on_shift_end: OnShiftEnd,
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
        /// Whether an arriving entity may not join the queue, with a chance that depends on its
        /// length (balking); `None`: every entity joins. See [`Balking`].
        balking: Option<Balking>,
        /// Each unit is checked now and then and taken out of service for a restart when too
        /// many entities wait (a StochCraft vendor extension, `stochcraft:healthCheck`); `None`:
        /// units are never taken out. See [`HealthCheck`].
        health_check: Option<HealthCheck>,
    },
    /// A task whose units come from shared resource pools ([`Model::resources`]): each entity
    /// takes what the first of `needs` that can be met right now asks for, all at once, holds it
    /// for its service (and while blocked after it), then gives it back. Entities wait here in
    /// one queue per task. At the end of each instant, once every event at it has happened
    /// (units given back, entities arrived), the free units go to the waiting entities across all
    /// tasks in order of their task's `priority` (higher first), then of arrival, each taking the
    /// first of its `needs` the free units can meet: units that come free together go to the
    /// first alternative, whichever job ended first. An entity that cannot be served now does
    /// not hold back one behind it that can. BPSim's `selection` with
    /// `getResource`, `getResourceByRoles` and `orResource`, and `priority`.
    SharedTask {
        service: Dist,
        /// Processing times for entities with particular values of a property, `service`
        /// applying to the others; `None`: `service` for every entity. See [`ProcessingBy`].
        processing_by: Option<ProcessingBy>,
        /// Service times by the pool whose units serve the entity, ahead of `processing_by` and
        /// `service` (a StochCraft vendor extension, `stochcraft:processingTimeByResource`);
        /// `None`: the pool does not change the time. See [`ProcessingByResource`].
        processing_by_resource: Option<ProcessingByResource>,
        /// The alternatives, tried in order (BPSim `orResource`); at least one.
        needs: Vec<Need>,
        /// Who gets freed units first, higher first (BPSim `priority`; default 0).
        priority: f64,
    },
    /// Holds each entity for a time with no capacity limit (a task with no resource).
    Delay {
        duration: Dist,
        /// Durations for entities with particular values of a property, `duration` applying
        /// to the others; `None`: `duration` for every entity. See [`ProcessingBy`].
        processing_by: Option<ProcessingBy>,
    },
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
    /// A conveyor that an entity rides from one end to the other (a StochCraft vendor extension on
    /// a task, `stochcraft:conveyor`). An entity enters at 0 and leaves at the `length`; every
    /// entity takes a slot of pitch `item_length + gap`. At most ⌊`length` / pitch⌋ are on it at
    /// once, and it carries at most `speed` / pitch entities per unit of time; how it moves them
    /// is in ROADMAP.md, M5.1. `shape` is used for drawing only; the simulation run still uses
    /// `length`.
    Conveyor {
        /// The conveyor's length, in the experiment's [`LengthUnit`]; above 0.
        length: f64,
        /// How far an entity travels in one unit of model time, in the experiment's length
        /// unit per its time unit; above 0.
        speed: f64,
        /// Whether entities slip on a stopped one ahead (the ones behind keep moving until each
        /// stops a pitch behind the one ahead), or the whole belt stops when the front entity
        /// cannot leave.
        accumulating: bool,
        /// The length of belt an entity takes, in the length unit; above 0.
        item_length: f64,
        /// The least free belt between one entity and the next, in the length unit; 0 or more.
        gap: f64,
        /// How the conveyor is drawn (straight, curved, or spiral); the simulation run still
        /// uses `length`.
        shape: ConveyorShape,
    },
    /// A transport: an item reaching it is carried from the element before it (the pickup) to
    /// the element after it (the drop-off) by a unit of a mobile pool (a StochCraft vendor
    /// extension on a task, `stochcraft:transport`). The unit travels to the pickup, loads for
    /// `load`, travels to the drop-off, and unloads for `unload`. It is not simulated yet
    /// (ROADMAP.md, M5.2).
    Transport {
        /// The mobile pool whose units carry the items ([`ResourcePool::mobility`]).
        pool: ElementId,
        /// The time to load an item at the pickup.
        load: Dist,
        /// The time to unload an item at the drop-off.
        unload: Dist,
    },
}

/// How a [`ElementKind::Conveyor`] is drawn in 3D; the simulation run still uses its `length`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ConveyorShape {
    /// A straight run of the conveyor's `length`.
    #[default]
    Straight,
    /// A horizontal arc of `radius` (in the length unit) sweeping `angle` degrees (positive
    /// turns left, negative turns right; nonzero).
    Curve { radius: f64, angle: f64 },
    /// A helical ramp of `radius` (in the length unit), climbing `rise` per turn for `turns`
    /// revolutions (all above 0).
    Spiral { radius: f64, rise: f64, turns: f64 },
}

impl ConveyorShape {
    /// This shape as a one-segment [`Path`] from the origin, heading along `+x`, in the same
    /// unit as the shape's own numbers: a straight run of `straight_length`, an arc, or a helix
    /// (a spiral's `rise` is per turn, the helix's over all its turns). The shape stays the
    /// stored form (the file format is unchanged), and the rest of the code reads geometry
    /// through this.
    pub fn path(&self, straight_length: f64) -> Path {
        let segment = match *self {
            ConveyorShape::Straight => Segment::Line {
                length: straight_length,
                rise: 0.0,
            },
            ConveyorShape::Curve { radius, angle } => Segment::Arc {
                radius,
                angle_deg: angle,
                rise: 0.0,
            },
            ConveyorShape::Spiral {
                radius,
                rise,
                turns,
            } => Segment::Helix {
                radius,
                rise: rise * turns,
                turns,
            },
        };
        Path::new(Frame::default(), vec![segment])
    }

    /// The drawn path length of this shape in the experiment's [`LengthUnit`], returning
    /// `straight_length` for [`ConveyorShape::Straight`].
    pub fn path_length(&self, straight_length: f64) -> f64 {
        match *self {
            ConveyorShape::Straight => straight_length,
            ConveyorShape::Curve { radius, angle } => radius * angle.abs().to_radians(),
            ConveyorShape::Spiral {
                radius,
                rise,
                turns,
            } => {
                let circumference = 2.0 * std::f64::consts::PI * radius;
                turns * (circumference * circumference + rise * rise).sqrt()
            }
        }
    }
}

impl ElementKind {
    /// A conveyor's slot for one entity: its item length plus its gap. `None` for any other
    /// element.
    pub fn conveyor_pitch(&self) -> Option<f64> {
        match self {
            ElementKind::Conveyor {
                item_length, gap, ..
            } => Some(item_length + gap),
            _ => None,
        }
    }

    /// The most entities a conveyor holds at once, ⌊length / pitch⌋ (see [`whole_pitches`]): 0
    /// when its numbers are not valid (see [`Problem::BadConveyorValue`]). `None` for any other
    /// element.
    pub fn conveyor_places(&self) -> Option<u64> {
        match self {
            ElementKind::Conveyor { length, .. } => {
                let pitch = self.conveyor_pitch()?;
                Some(if pitch > 0.0 && pitch.is_finite() {
                    whole_pitches(*length, pitch)
                } else {
                    0
                })
            }
            _ => None,
        }
    }
}

/// How many whole pitches fit in `length`: ⌊length / pitch⌋, where a quotient within rounding
/// of a whole number counts as that whole number.
///
/// A pitch is a sum of two decimals, which binary floating point holds only approximately:
/// 0.4 + 0.2 is 0.6000000000000001, so 30 / (0.4 + 0.2) is 49.99999999999999 and a plain floor
/// gives 49 places on a belt that holds 50. The sum and the division each round by at most half
/// a unit in the last place, so the quotient is nudged up by four units in its last place
/// before the floor: enough for those errors, and far too little to turn 49.99 into 50.
///
/// A negative or NaN quotient gives 0 and an infinite one `u64::MAX` (the cast saturates).
pub fn whole_pitches(length: f64, pitch: f64) -> u64 {
    let q = length / pitch;
    (q + q.abs() * (4.0 * f64::EPSILON)).floor() as u64
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

/// A task's processing time by the value of an entity property (a time by product type; a
/// StochCraft vendor extension, `stochcraft:processingTimeBy`, since BPSim 2.0 has none): an
/// entity whose value of `property` has an entry in `by_value` takes that entry's time; any
/// other entity (another value, or no value) takes the task's own processing time. Values are
/// compared as text, as a changeover compares them (`A`, `2`, `true`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessingBy {
    pub property: String,
    pub by_value: Vec<ByValue>,
}

/// The processing time of entities whose property value is `value`.
#[derive(Debug, Clone, PartialEq)]
pub struct ByValue {
    pub value: String,
    pub time: Dist,
}

impl ProcessingBy {
    /// The time of an entity whose value is `value` (`None`: it has none), if it has an entry.
    pub fn time_for(&self, value: Option<&str>) -> Option<&Dist> {
        let value = value?;
        self.by_value
            .iter()
            .find(|b| b.value == value)
            .map(|b| &b.time)
    }
}

/// A shared task's service time by the pool whose units serve an entity (a StochCraft vendor
/// extension, `stochcraft:processingTimeByResource`; Banks et al.'s servers that differ in
/// speed): an entity served by units of a pool that has an entry takes that entry's time. The
/// key is the pool, not the unit, since a pool's units are alike. When one take spans pools (a
/// quantity above 1 that no one pool can meet), the entry of the pool that supplied the first
/// unit, in the order the take was filled, applies. The entry comes first, then the task's time
/// by property ([`ProcessingBy`]), then its own `service`: an entity served by a pool without an
/// entry takes the time it would without these.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessingByResource {
    pub by_resource: Vec<ByResource>,
}

/// The service time of entities served by units of pool `resource`.
#[derive(Debug, Clone, PartialEq)]
pub struct ByResource {
    pub resource: ElementId,
    pub time: Dist,
}

impl ProcessingByResource {
    /// The time of an entity served by pool `pool`, if it has an entry.
    pub fn time_for(&self, pool: &ElementId) -> Option<&Dist> {
        self.by_resource
            .iter()
            .find(|b| &b.resource == pool)
            .map(|b| &b.time)
    }
}

/// The times of `by`'s entries (none without it).
fn by_value_times(by: Option<&ProcessingBy>) -> impl Iterator<Item = &Dist> {
    by.into_iter()
        .flat_map(|b| b.by_value.iter().map(|v| &v.time))
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

/// Balking at a server: an entity arriving there looks at its queue and may not join it (a
/// StochCraft vendor extension, `stochcraft:balking`, since BPSim has no parameter for it). It
/// joins with a probability b(n) that depends on the number n it finds ([`Balking::join`]);
/// otherwise it balks: it is counted as balked (it never enters) and takes the server's way out
/// marked [`Connection::balked`] at once, or leaves the model if there is none.
///
/// The choice is made as the entity arrives, before it asks for room in a finite waiting room:
/// one that joins and finds no room is then turned away (or waits for room) as before. A
/// probability of 0 or 1 draws nothing; any other is drawn from the server's own stream.
///
/// Not joining at or above a length L is the same as a finite waiting room of L
/// (`queue_capacity`) with a way out for those turned away (they count as lost there, not
/// balked), and needs no balking. With `counts_in_service` and [`BalkJoin::Reciprocal`] it is
/// Gross and Harris's M/M/1 with balking (b(n) = 1/(n + 1), n in the system; *Fundamentals of
/// Queueing Theory*), whose number in the system is Poisson.
#[derive(Debug, Clone, PartialEq)]
pub struct Balking {
    pub join: BalkJoin,
    /// What n counts: those waiting (in the queue, or in transfer here), as a waiting room
    /// counts them, when false (the default); those in the system (also those in service, or
    /// held blocked after it) when true.
    pub counts_in_service: bool,
}

/// How the chance of joining depends on the number found, n.
#[derive(Debug, Clone, PartialEq)]
pub enum BalkJoin {
    /// Rows in increasing order of `waiting`: from each row's number on (until the next row's),
    /// the entity joins with the row's probability; below the first row it always joins.
    Table(Vec<BalkRow>),
    /// b(n) = 1/(n + 1): always when none are there, half the time when one is, and so on.
    Reciprocal,
}

/// One row of a [`BalkJoin::Table`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BalkRow {
    /// The number found from which the row applies.
    pub waiting: u32,
    /// The chance of joining, from 0 to 1.
    pub probability: f64,
}

impl Balking {
    /// The chance that an entity finding `n` joins.
    pub fn join_probability(&self, n: u64) -> f64 {
        match &self.join {
            BalkJoin::Table(rows) => rows
                .iter()
                .take_while(|r| u64::from(r.waiting) <= n)
                .last()
                .map_or(1.0, |r| r.probability),
            BalkJoin::Reciprocal => 1.0 / (n as f64 + 1.0),
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
/// Failures ([`Breakdown`]) keep coming in elapsed time while a unit is out. With a finite
/// buffer, an entity holds its place there throughout: a retried job keeps it back in the
/// queue, a resumed one keeps it waiting on its unit. `place_while_out` says whether a unit out
/// restarting also keeps a place of the buffer for itself.
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
    /// With a finite buffer, whether a unit out restarting keeps a place of it for itself (as
    /// a failed unit does) or holds none (as a unit off shift); with an unlimited buffer it
    /// changes nothing.
    pub place_while_out: PlaceWhileOut,
}

/// With a finite buffer, the place of a unit a health check takes out, while it restarts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaceWhileOut {
    /// As a failed unit's: at the takeout, once the job under way (if any) has gone back to the
    /// queue, the unit takes a place that is free at that moment (if one is), so no entity
    /// gets in for it, and gives it back when its restart ends. The buffer holds fewer while
    /// units are out. A unit with a job waiting on it (resumed) takes none: the job's place is
    /// the unit's; one holding a finished entity it cannot hand on yet keeps that entity's
    /// place once it leaves.
    #[default]
    Reserved,
    /// The unit holds no place while it is out, as a unit off shift: entities may fill the
    /// buffer up to every unit's place and the waiting room.
    Open,
}

impl PlaceWhileOut {
    /// The name in files and on the wire.
    pub fn name(self) -> &'static str {
        match self {
            PlaceWhileOut::Reserved => "reserved",
            PlaceWhileOut::Open => "open",
        }
    }

    /// The value named `s`.
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "reserved" => Some(PlaceWhileOut::Reserved),
            "open" => Some(PlaceWhileOut::Open),
            _ => None,
        }
    }
}

/// How the size of a server's finite waiting room is set (its `queue_limit`, with
/// `queue_capacity` giving the number).
///
/// `Fixed` is the waiting room as it always was: places for every unit plus `queue_capacity`,
/// an entity holding one from entering until it leaves, a failed unit (or one out restarting,
/// with [`PlaceWhileOut::Reserved`]) keeping one for itself.
///
/// The other two set a limit on the entities waiting that changes over time, and hold no
/// places: an entity is admitted when the number waiting (in the queue, or on its way in
/// across a transfer) less the idle working units ready to take them is below the limit now.
/// So it gets in when an idle working unit can take it at once, or when fewer than the limit
/// wait. Working is up, not out after a health check and on shift (as a health check counts
/// it); idle is working and holding no entity. An entity whose job a failure, a takeout or a
/// shift's end paused on its unit is in service, not waiting. When the limit drops, those
/// already waiting stay: only new arrivals are turned away until fewer wait than the limit.
/// No unit keeps a place for itself ([`PlaceWhileOut`] changes nothing): with
/// `PerWorkingServer` a unit failed or out already lowers the limit.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum QueueLimit {
    /// Places for every unit plus `queue_capacity` waiting.
    #[default]
    Fixed,
    /// At most this many waiting while a calendar is active, the first active one applying
    /// (as units by shift); `queue_capacity` outside them. A StochCraft vendor extension (values
    /// of `stochcraft:queueCapacity` `validFor` a calendar).
    ByShift(Vec<Timed<u32>>),
    /// At most `queue_capacity` waiting per working unit, so the limit follows failures,
    /// takeouts, shifts and returns by itself. A StochCraft vendor extension
    /// (`stochcraft:queueCapacity` with `perWorkingServer="true"`).
    PerWorkingServer,
}

impl QueueLimit {
    /// The calendars its values refer to.
    pub fn calendars(&self) -> impl Iterator<Item = &ElementId> {
        let timed: &[Timed<u32>] = match self {
            QueueLimit::ByShift(t) => t,
            _ => &[],
        };
        timed.iter().map(|t| &t.calendar)
    }

    /// Whether it holds places for every unit plus the waiting room (`Fixed`), rather than
    /// limiting the number waiting.
    pub fn is_fixed(&self) -> bool {
        matches!(self, QueueLimit::Fixed)
    }
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

/// What happens to a job under way on a server's unit that goes off shift: the units on shift
/// drop (a shift with fewer, [`ElementKind::Server`]'s `shifts`) or the availability closes.
///
/// Which units go off when the count drops by some number: first idle units that are up and
/// in service, the highest numbered first; then units failed or out restarting after a health
/// check (the highest numbered first); then busy units, those holding a finished entity first,
/// then the job started most recently first (the least work lost). When the count rises, units
/// still finishing a job before going off stay on first; then units come back, those holding a
/// job paused off shift first, then the lowest numbered first. A failed or out unit comes back
/// still failed or out until its repair or restart ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnShiftEnd {
    /// The unit finishes the job, then goes off; until then it counts as on shift. A job a
    /// failure interrupted resumes after the repair, shift or not.
    #[default]
    Finish,
    /// The job stops at once and the entity goes back to the head of the queue, for any unit
    /// to start afresh (with a fresh patience if the server has one). The work done on it is
    /// wasted.
    Retry,
    /// The job stops at once; the entity waits on its unit and the job carries on with what
    /// was left once that unit is on shift again.
    Resume,
}

impl OnShiftEnd {
    /// The name in files and on the wire.
    pub fn name(self) -> &'static str {
        match self {
            OnShiftEnd::Finish => "finish",
            OnShiftEnd::Retry => "retry",
            OnShiftEnd::Resume => "resume",
        }
    }

    /// The value named `s`.
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "finish" => Some(OnShiftEnd::Finish),
            "retry" => Some(OnShiftEnd::Retry),
            "resume" => Some(OnShiftEnd::Resume),
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
    /// A divert, as on a sorting conveyor (a StochCraft vendor extension,
    /// `stochcraft:routing="divert"`): the way is chosen as [`Routing::Standard`] chooses it (a
    /// condition, else the default; or a draw by the probabilities), but if that way has no room
    /// now the entity takes the default way instead (round a loop), and only if the default has
    /// none either does the element sending into the gateway wait, holding it. As with
    /// [`Routing::ShortestQueue`], the sender routes through the gateway itself, so an entity
    /// never leaves it before a way has room; the way first chosen stays chosen while it waits.
    /// It needs a default way.
    Divert,
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
    /// Whether the units move about (operators who walk or drive, [`ElementKind::Transport`]);
    /// `None` for a pool whose units stay where they are.
    pub mobility: Option<Mobility>,
}

/// How a mobile pool's units move. Speeds are in the experiment's length unit per its time
/// unit. A StochCraft vendor extension on the resource (`stochcraft:mobility`).
#[derive(Debug, Clone, PartialEq)]
pub struct Mobility {
    /// The speed of a unit that carries nothing; above 0.
    pub speed_empty: f64,
    /// The speed of a unit that carries an item; above 0.
    pub speed_loaded: f64,
    /// The element its units start at (their home); it needs a shape in [`Model::layout`].
    pub home: ElementId,
    /// What a unit does once it has put an item down.
    pub after_job: AfterJob,
}

/// What a mobile unit does once it has put an item down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AfterJob {
    /// It stays where it put the item down.
    #[default]
    Stay,
    /// It goes back to its home.
    GoHome,
}

impl AfterJob {
    /// The name written in files (`stay` or `goHome`).
    pub fn name(self) -> &'static str {
        match self {
            AfterJob::Stay => "stay",
            AfterJob::GoHome => "goHome",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "stay" => AfterJob::Stay,
            "goHome" => AfterJob::GoHome,
            _ => return None,
        })
    }
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
            mobility: None,
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
            ElementKind::Conveyor { .. } => "conveyor",
            ElementKind::Transport { .. } => "transport",
        }
    }

    /// Whether entities can be inside this kind of block as a run starts ([`Model::starting`]):
    /// a task (with a resource of its own or shared ones), a delay or a batch. Not sources,
    /// sinks, gateways, splits or conveyors.
    pub fn holds_at_start(&self) -> bool {
        matches!(
            self,
            ElementKind::Server { .. }
                | ElementKind::SharedTask { .. }
                | ElementKind::Delay { .. }
                | ElementKind::Batch { .. }
        )
    }

    /// Whether this kind of block can use a decision table ([`Model::decisions`]): a task, a
    /// delay, or an exclusive gateway that does not send each entity to the shortest queue.
    pub fn reads_decisions(&self) -> bool {
        match self {
            ElementKind::ExclusiveGateway { routing, .. } => {
                !matches!(routing, Routing::ShortestQueue)
            }
            ElementKind::Server { .. }
            | ElementKind::SharedTask { .. }
            | ElementKind::Delay { .. } => true,
            _ => false,
        }
    }

    /// A server's units in all: the most its timetable ever puts on shift (its `capacity`, or
    /// a shift's units if more). `None` for other elements.
    pub fn most_units(&self) -> Option<u32> {
        match self {
            ElementKind::Server {
                capacity, shifts, ..
            } => Some(shifts.iter().map(|s| s.value).fold(*capacity, u32::max)),
            _ => None,
        }
    }

    /// A task's processing times by property value (a server's, a task with shared resources'
    /// or a delay's), if it has them.
    pub fn processing_by(&self) -> Option<&ProcessingBy> {
        match self {
            ElementKind::Server { processing_by, .. }
            | ElementKind::SharedTask { processing_by, .. }
            | ElementKind::Delay { processing_by, .. } => processing_by.as_ref(),
            _ => None,
        }
    }

    /// The processing times by property value of a task, to change; `None` for an element
    /// that is not a task with a processing time.
    pub fn processing_by_mut(&mut self) -> Option<&mut Option<ProcessingBy>> {
        match self {
            ElementKind::Server { processing_by, .. }
            | ElementKind::SharedTask { processing_by, .. }
            | ElementKind::Delay { processing_by, .. } => Some(processing_by),
            _ => None,
        }
    }

    /// A shared task's service times by serving pool, if it has them.
    pub fn processing_by_resource(&self) -> Option<&ProcessingByResource> {
        match self {
            ElementKind::SharedTask {
                processing_by_resource,
                ..
            } => processing_by_resource.as_ref(),
            _ => None,
        }
    }

    /// A shared task's service times by serving pool, to change; `None` for any other element.
    pub fn processing_by_resource_mut(&mut self) -> Option<&mut Option<ProcessingByResource>> {
        match self {
            ElementKind::SharedTask {
                processing_by_resource,
                ..
            } => Some(processing_by_resource),
            _ => None,
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
                d.extend(by_value_times(self.processing_by()).map(|t| ("service", t)));
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
            ElementKind::SharedTask { service, .. } => std::iter::once(("service", service))
                .chain(by_value_times(self.processing_by()).map(|t| ("service", t)))
                .chain(
                    self.processing_by_resource()
                        .into_iter()
                        .flat_map(|b| b.by_resource.iter().map(|r| ("service", &r.time))),
                )
                .collect(),
            ElementKind::Delay { duration, .. } => std::iter::once(("duration", duration))
                .chain(by_value_times(self.processing_by()).map(|t| ("duration", t)))
                .collect(),
            ElementKind::Transport { load, unload, .. } => vec![("load", load), ("unload", unload)],
            ElementKind::Sink { .. }
            | ElementKind::ExclusiveGateway { .. }
            | ElementKind::ParallelGateway
            | ElementKind::Batch { .. }
            | ElementKind::Unbatch
            | ElementKind::Conveyor { .. } => vec![],
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
    /// The way out of a server with a finite waiting room that entities finding no room take
    /// at once (turned away), instead of being lost (from a source) or waiting before it (from
    /// any other element), apart from its ordinary way out. A StochCraft vendor extension
    /// (`stochcraft:turnedAway`).
    pub turned_away: bool,
    /// The way out of a server with a [`Balking`] that entities not joining its queue take at
    /// once, instead of leaving the model, apart from its ordinary way out. A StochCraft vendor
    /// extension (`stochcraft:balked`).
    pub balked: bool,
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
            turned_away: false,
            balked: false,
        }
    }
}

/// How to run the model (a BPSim scenario's parameters).
#[derive(Debug, Clone, PartialEq)]
pub struct Experiment {
    pub time_unit: TimeUnit,
    /// The unit lengths are in (a conveyor's); metres unless the scenario says otherwise.
    pub length_unit: LengthUnit,
    /// Length units per drawing unit of [`Model::layout`]: a distance between two elements is
    /// the straight line between their centres on the canvas, times this ([`Model::distance`]).
    /// 1 unless the scenario says otherwise (`stochcraft:layoutScale`).
    pub layout_scale: f64,
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
    /// How often results over time are recorded (BPSim scenario `baseResultFrequency`), in
    /// seconds: an `xs:duration` in the file, so it does not depend on the time unit. `None`
    /// when the file gives none (the run then records about 100 points over its length).
    pub result_frequency: Option<f64>,
    /// Sequential replications: start with [`Experiment::replications`], then add more until
    /// the results it watches are precise enough, or a maximum is reached. A StochCraft vendor
    /// extension on the scenario (`stochcraft:sequential`), since BPSim runs a fixed number.
    /// `None` runs exactly [`Experiment::replications`].
    pub sequential: Option<Sequential>,
    /// Real measurements the results are checked against (black-box validation), at most one
    /// for each result. A StochCraft vendor extension on the scenario (`stochcraft:observed`).
    pub observed: Vec<Observation>,
}

/// A measurement of the real system for one result of the model to be checked against: an
/// element's metric (by its name in the results, such as `mean_wait`), in the metric's own unit
/// (a share as a fraction: 0.85 is 85%).
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// A block or a pool of the model.
    pub element: ElementId,
    pub metric: String,
    pub target: Observed,
}

/// What a result is checked against ([`Observation::target`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Observed {
    /// The value seen in the real system, with the half-width of its own 95% interval where its
    /// data give one, and a `tolerance` (above 0, in the metric's unit) that the modeller sets in
    /// advance: the difference the model may have and still count as agreeing. The simulated mean
    /// differs from it when the 95% interval of the difference excludes 0 (with a tolerance:
    /// lies wholly outside it), agrees when the whole interval is inside the tolerance, and
    /// otherwise no difference is detected or the check is inconclusive.
    Value {
        value: f64,
        half_width: Option<f64>,
        tolerance: Option<f64>,
    },
    /// The result should be at most this; decided by the simulated 95% interval, not by the mean
    /// alone.
    AtMost(f64),
    /// The result should be at least this; decided as an upper bound is.
    AtLeast(f64),
}

impl Observed {
    /// The number it is given, whatever it is checked as.
    pub fn value(self) -> f64 {
        match self {
            Observed::Value { value, .. } | Observed::AtMost(value) | Observed::AtLeast(value) => {
                value
            }
        }
    }
}

/// How a sequential experiment decides it has run enough replications: each result it watches
/// must have a 95% half-width within its target, a share of its mean (`relative`) or, for a
/// share such as a utilisation, percentage points (`points`). Replication i is the same in a
/// sequential run as in a fixed one, so one that stops at n replications gives exactly the
/// results of a fixed run of n.
#[derive(Debug, Clone, PartialEq)]
pub struct Sequential {
    /// The one result to watch; `None` watches every block's key measures (the results the
    /// window shows first, up to four per block).
    pub measure: Option<SequentialMeasure>,
    /// The half-width as a share of the mean at which a result is precise enough (0.05 is
    /// ± 5%).
    pub relative: f64,
    /// The half-width in percentage points at which a share is precise enough (1 is ± 1 point).
    pub points: f64,
    /// The most replications to run, the first ones included.
    pub max: u32,
}

impl Default for Sequential {
    fn default() -> Self {
        Self {
            measure: None,
            relative: 0.05,
            points: 1.0,
            max: 200,
        }
    }
}

/// One result a sequential experiment watches: an element's metric (by its name in the
/// results, such as `mean_wait`), with an absolute half-width to reach in the metric's own unit
/// (a share as a fraction: 0.01 is one point), or, without one, the experiment's relative or
/// points target.
#[derive(Debug, Clone, PartialEq)]
pub struct SequentialMeasure {
    pub element: ElementId,
    pub metric: String,
    pub half_width: Option<f64>,
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
            length_unit: LengthUnit::Metres,
            layout_scale: 1.0,
            warmup: 0.0,
            duration: 480.0,
            replications: 10,
            seed: 1,
            start: None,
            stop: None,
            result_frequency: None,
            sequential: None,
            observed: Vec::new(),
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
    /// Starting contents: how many entities are inside each block as every replication starts,
    /// instead of an empty start (tasks, delays and batches only, see
    /// [`ElementKind::holds_at_start`]). They are put into the block's queue at time 0 in
    /// order, as arrivals would be, so as many as there are free units start a fresh service
    /// at once and the rest wait. A block left out (or 0) starts empty.
    pub starting: BTreeMap<ElementId, u32>,
    /// The Monte Carlo inputs and outputs, for a model of a calculation rather than a process
    /// ([`montecarlo`]): `None` for a process model. A model with one and no elements is a Monte
    /// Carlo model, which [`Model::validate`] leaves to `stochcraft-mc` to check.
    pub monte_carlo: Option<MonteCarlo>,
    /// The physical layout: conveyors' tracks, blocks' places and scenery ([`site`]). Empty for a
    /// model without one, whose blocks are all unplaced.
    pub site: Site,
    /// The populations, environment and statistics of an agent-based model, a model with no
    /// process ([`agents`]): `None` for any other. A model with one and no elements is an
    /// agent-based model, which [`Model::validate`] leaves to `stochcraft-agents` to check.
    pub agents: Option<agents::Agents>,
    /// The decision table each element uses ([`dmn`], the `stochcraft:decision` vendor
    /// extension): evaluated as an entity enters, its outputs written as properties.
    pub decisions: BTreeMap<ElementId, dmn::DecisionUse>,
    /// The `.dmn` files those decisions are in, by the reference [`dmn::DecisionUse::file`]
    /// gives. Read beside the model's file by the engine (not by `stochcraft-io`'s reading of
    /// the model), so a file not read is missing here.
    pub dmn: BTreeMap<String, dmn::Definitions>,
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
            | ElementKind::Unbatch
            | ElementKind::Conveyor { .. }
            | ElementKind::Transport { .. } => (100.0, 80.0),
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
        field: ExperimentField,
    },
    /// A server's power in a state that is negative or not a number.
    BadPower {
        state: &'static str,
    },
    /// A property with no name, or one an element assigns twice.
    BadProperty {
        name: String,
        reason: PropertyFault,
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
    /// Processing times by property value with no property to read the value of.
    TimesByValueWithoutProperty,
    /// Two processing times by property value for one value.
    DuplicateTimeByValue {
        value: String,
    },
    /// A processing time by property value whose mean is below 0.
    TimeByValueNegative {
        value: String,
    },
    /// A service time by pool for a pool the model does not have.
    TimeForUnknownPool {
        id: ElementId,
    },
    /// Two service times by pool for one pool.
    DuplicateTimeByPool {
        id: ElementId,
    },
    /// A service time by pool for a pool the task does not need.
    TimeForPoolNotNeeded {
        id: ElementId,
    },
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
    /// A connection marked for entities turned away leaves an element that is not a task with
    /// a resource of its own and a finite waiting room.
    TurnedAwayWithoutRoom {
        connection: ElementId,
    },
    /// Several ways out of one server are marked for entities turned away.
    SeveralTurnedAwayWays {
        count: usize,
    },
    /// A connection is marked both for entities that give up and for those turned away.
    TurnedAwayAndAbandoned {
        connection: ElementId,
    },
    /// A balking table's chance of joining outside 0 to 1.
    BalkingProbability,
    /// A balking table whose rows are not in increasing order of the number found.
    BalkingRowsOrder,
    /// A connection marked for entities that balk leaves an element with no balking.
    BalkedWithoutBalking {
        connection: ElementId,
    },
    /// Several ways out of one server are marked for entities that balk.
    SeveralBalkedWays {
        count: usize,
    },
    /// A connection is marked both for entities that balk and for those that give up or are
    /// turned away.
    BalkedAndOtherWay {
        connection: ElementId,
    },
    /// A waiting room by timetable or per working server with no number of places: it is
    /// unlimited, and the setting is ignored.
    QueueLimitWithoutRoom,
    /// A waiting room by timetable with no calendar's value: the file cannot hold it (it would
    /// read back as a fixed room, which holds places).
    QueueByShiftEmpty,
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
        what: IgnoredWhat,
        reason: IgnoredWhy,
    },
    /// An exclusive gateway's default connection is not one of its outgoing connections.
    BadDefault {
        connection: ElementId,
    },
    /// An exclusive gateway routes by condition and has no default connection.
    NoDefault,
    /// A divert gateway has no default connection, the way taken when the one chosen is full.
    DivertNoDefault,
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
    /// A conveyor's `field` (`length`, `speed`, `item_length`, `radius`, `rise` or `turns`) is
    /// not above 0 and finite, its `gap` is not 0 or more and finite, or its curve `angle` is 0
    /// or not finite.
    BadConveyorValue {
        field: &'static str,
    },
    /// A conveyor too short for even one entity: its `length` is under its `pitch` (item length
    /// plus gap), so ⌊length / pitch⌋ is 0.
    ConveyorTooShort {
        length: f64,
        pitch: f64,
    },
    /// A curved or spiral conveyor's drawn path length differs from its `length` by more than
    /// 1%, so the drawing and the run would not agree.
    ConveyorLengthMismatch {
        length: f64,
        path_length: f64,
    },
    /// A block is placed `s` along `conveyor`'s track, outside it: below 0 or beyond its
    /// `length`. An error for a divert, whose `s` the run reads; a warning for any other block,
    /// which is only drawn there.
    PlaceOffPath {
        conveyor: ElementId,
        s: f64,
        length: f64,
    },
    /// Items on `conveyor` meet the divert `before` (at `before_s` along it) before this one
    /// (at `s`), by the flows, but this one is placed nearer the start, so the order of the
    /// flows and the order along the track disagree.
    PointsOutOfOrder {
        conveyor: ElementId,
        s: f64,
        before: ElementId,
        before_s: f64,
    },
    /// The junction on this flow has a transfer time that is not 0 or more and finite.
    BadJunctionTime {
        time: f64,
    },
    /// Found when the model is prepared to run, not by [`Model::validate`] (a transport can be
    /// drawn, saved and sized meanwhile): transports are not simulated yet.
    TransportNotSimulated,
    /// A block names a decision table it cannot use: the file was not read, has no such
    /// decision, or the table cannot be used here (`reason`, a sentence in English).
    DecisionUnusable {
        file: String,
        decision: String,
        reason: String,
    },
    /// Two rules of a UNIQUE decision table (numbered from 1) both match `example`.
    DecisionOverlap {
        decision: String,
        first: usize,
        second: usize,
        example: String,
    },
    /// No rule of a decision table matches `example`, and it has no default output.
    DecisionGap {
        decision: String,
        example: String,
    },
    /// A gateway routed by a decision table can be given `value`, which names none of its ways
    /// out.
    DecisionRoute {
        decision: String,
        value: String,
    },
    /// A decision table too large for the check of its rules to finish: whether rules overlap,
    /// or an input is matched by no rule, is not known.
    DecisionUnchecked {
        decision: String,
    },
    /// A transport's pool does not exist, or is not mobile.
    TransportPool {
        pool: ElementId,
        /// Whether the pool exists (it is then not mobile).
        exists: bool,
    },
    /// A mobile pool's speed (`field` is `speed_empty` or `speed_loaded`) is not above 0 and
    /// finite.
    BadMobilitySpeed {
        field: &'static str,
    },
    /// A mobile pool's home is not an element of the model.
    UnknownHome {
        home: ElementId,
    },
    /// A transport has other than exactly one element before it and one after it.
    TransportEnds {
        inputs: usize,
        outputs: usize,
    },
    /// A distance needs `element` to have a shape on the canvas (a transport's pickup or
    /// drop-off, a mobile pool's home), and it has none.
    NoShape {
        element: ElementId,
    },
    /// The experiment's layout scale is not above 0 and finite.
    BadLayoutScale,
    /// A bill of materials asking for no entities from a connection.
    ZeroQuantity,
    /// A quantity on a connection that does not go into a parallel gateway, where it does
    /// nothing.
    QuantityIgnored,
    /// The model splits batches but makes none.
    NothingToUnbatch,
    /// The experiment's stop condition cannot be used, and why.
    BadStop {
        reason: StopFault,
    },
    /// An observed value ([`Experiment::observed`]) is for a block or pool that the model does
    /// not have, so there is no result to check it against.
    BadObservation,
    /// The starting contents of a block ([`Model::starting`]) cannot be used, and why.
    BadStarting {
        reason: StartingFault,
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
    /// A server, or a pool, is offered at least `load` times the work its units can do (arrival
    /// rate × mean service time ÷ units, over the share of time they are up; for a pool, the
    /// work of every task on it), at least 1, as the capacity table ([`Model::capacity`]) passes
    /// rates on through routing, joins and pools: its queue grows without limit, so the run has
    /// no steady state.
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
    /// Found from a run's results, not by [`Model::validate`]: the run had one replication, so
    /// its results have no confidence interval and cannot be told from chance.
    OneReplication,
    /// Found from a run's results, not by [`Model::validate`]: a run with no warm-up and no stop
    /// condition, in which the number inside a task settles only after about `warmup` (in
    /// `unit`; `None`: later than half the run), by MSER-5 on its mean across replications, and
    /// leaving that start-up out would move the task's mean number inside by `shift`, more than
    /// chance. Its results are biased by the start from empty.
    NoWarmup {
        warmup: Option<f64>,
        shift: f64,
        unit: &'static str,
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
    /// Found from a run's results, not by [`Model::validate`]: a behaviour check (a check that
    /// needs no reference, made on each replication's results) failed in `replications` of the
    /// `of` run; `found` is what it found in the replication where it was furthest off.
    Behaviour {
        found: Behaviour,
        replications: u64,
        of: u64,
    },
}

/// What a behaviour check found in one replication's results ([`Problem::Behaviour`]). Each
/// holds the numbers that should agree, or the one out of its range.
#[derive(Debug, Clone, PartialEq)]
pub enum Behaviour {
    /// An element's entities do not add up: `came` came in, or were inside as the statistics
    /// started, against `went`, that left, were lost, gave up or are still inside.
    Unbalanced { came: u64, went: u64 },
    /// The model's entities do not add up: `came` created, or inside as the statistics started,
    /// against `went`, that reached an exit, left the model elsewhere (lost, or giving up or
    /// balking with nowhere to go) or are still inside.
    Unaccounted { came: u64, went: u64 },
    /// Of `came` entities created or inside as the statistics started, none left the model: none
    /// reached an exit, nor gave up, balked, was turned away or found no way with nowhere to go.
    NothingOut { came: u64 },
    /// In a closed model ([`Model::is_closed`]), whose entities go round and never leave, the
    /// `came` entities created or inside at the start are stuck: no block holding entities at
    /// the end finished any in the run ([`Behaviour::NothingOut`] for an open model).
    NothingMoved { came: u64 },
    /// Little's law does not hold at an element: the time-average number inside was `mean`, but
    /// the entities' own times in it (from when each came in and went out) give `from_stays`.
    LittlesLaw { mean: f64, from_stays: f64 },
    /// A server's units counted busy in its utilisation (time-average, `busy`) are not those
    /// working (`working`: processing or setting up). A unit idle, blocked, off shift, failed or
    /// restarting is not busy, even holding a job.
    BusyNotWorking { busy: f64, working: f64 },
    /// The shares of a server's units' time in each state add up to `total`, not 1.
    StatesTotal { total: f64 },
    /// A share (`metric`, a fraction of time) is `value`, outside 0 to 1.
    OutOfRange { metric: &'static str, value: f64 },
}

impl Behaviour {
    /// A stable snake_case name for files, the command line and agents.
    pub fn code(&self) -> &'static str {
        match self {
            Behaviour::Unbalanced { .. } => "unbalanced",
            Behaviour::Unaccounted { .. } => "unaccounted",
            Behaviour::NothingOut { .. } => "nothing_out",
            Behaviour::NothingMoved { .. } => "nothing_moved",
            Behaviour::LittlesLaw { .. } => "littles_law",
            Behaviour::BusyNotWorking { .. } => "busy_not_working",
            Behaviour::StatesTotal { .. } => "states_total",
            Behaviour::OutOfRange { .. } => "out_of_range",
        }
    }

    /// How far off it is, to pick the worst of several replications.
    pub fn gap(&self) -> f64 {
        match self {
            Behaviour::Unbalanced { came, went } | Behaviour::Unaccounted { came, went } => {
                came.abs_diff(*went) as f64
            }
            Behaviour::NothingOut { came } | Behaviour::NothingMoved { came } => *came as f64,
            Behaviour::LittlesLaw { mean, from_stays } => (mean - from_stays).abs(),
            Behaviour::BusyNotWorking { busy, working } => (busy - working).abs(),
            Behaviour::StatesTotal { total } => (total - 1.0).abs(),
            Behaviour::OutOfRange { value, .. } => (value - value.clamp(0.0, 1.0)).abs(),
        }
    }
}

impl std::fmt::Display for Behaviour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // All but two (nothing out, nothing moved) can only fail through a fault in the
        // simulation itself, not the model.
        const FAULT: &str = "this points to a fault in StochCraft rather than in the model; please report it with the model";
        match self {
            Behaviour::Unbalanced { came, went } => write!(
                f,
                "its entities do not add up: {came} came in or were inside at the start, but {went} left, were lost, gave up or are still inside; {FAULT}"
            ),
            Behaviour::Unaccounted { came, went } => write!(
                f,
                "the model's entities do not add up: {came} were created or inside at the start, but {went} reached an exit, left the model elsewhere (lost, or giving up or balking with nowhere to go) or are still inside; {FAULT}"
            ),
            Behaviour::NothingOut { came } => write!(
                f,
                "no entity left the model, of {came} created or inside at the start: check that the routing leads to an exit and that the run is long enough for an entity to get through"
            ),
            Behaviour::NothingMoved { came } => write!(
                f,
                "the entities going round the closed model are stuck, of {came} created or inside at the start: no block where they were at the end finished any during the run; check that every block of the loop can serve them and that the run is long enough for an entity to get round"
            ),
            Behaviour::LittlesLaw { mean, from_stays } => write!(
                f,
                "Little's law does not hold: the time-average number inside was {}, but the entities' own times inside (from when each came in and went out, within the run) give {}; {FAULT}",
                short(*mean),
                short(*from_stays)
            ),
            Behaviour::BusyNotWorking { busy, working } => write!(
                f,
                "its utilisation counts {} units busy on average, but {} were processing or setting up: {}; {FAULT}",
                short(*busy),
                short(*working),
                if busy > working {
                    "units idle, blocked, off shift, failed or restarting were counted busy"
                } else {
                    "units at work were not counted busy"
                }
            ),
            Behaviour::StatesTotal { total } => write!(
                f,
                "the shares of its units' time in each state add up to {}, not 1; {FAULT}",
                short(*total)
            ),
            Behaviour::OutOfRange { metric, value } => write!(
                f,
                "{metric} is {}, outside 0 to 1 (0% to 100%); {FAULT}",
                short(*value)
            ),
        }
    }
}

/// Φ⁻¹(0.99): a normal distribution draws below 0 more than 1% of the time when its mean is less
/// than this many standard deviations ([`Problem::NormalOftenNegative`]).
pub const NORMAL_1_PERCENT: f64 = 2.326_347_874_040_840_8;

/// The experiment's setting a problem is about ([`Problem::BadExperiment`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperimentField {
    /// The warm-up is not 0 or more.
    Warmup,
    /// The duration is not above 0.
    Duration,
    /// There are no replications.
    Replications,
}

/// What is wrong with a property ([`Problem::BadProperty`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyFault {
    /// It has no name.
    NoName,
    /// An element sets it twice.
    SetTwice,
    /// Its shares are negative or add up to 0.
    BadShares,
}

/// What a routing setting that has no effect is ([`Problem::RoutingIgnored`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoredWhat {
    Condition,
    Probability,
    Default,
}

/// Why a routing setting has no effect ([`Problem::RoutingIgnored`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoredWhy {
    /// The connection is not out of an exclusive gateway.
    NotDecision,
    /// The gateway routes to the shortest queue.
    ShortestQueue,
    /// The gateway routes by condition.
    ByCondition,
}

/// Why a stop condition cannot be used ([`Problem::BadStop`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopFault {
    /// No element has the id it names.
    NoSuchBlock,
    /// Its count is 0.
    ZeroCount,
    /// It waits for an element that is not a task to empty.
    NotATask,
}

/// Why a block's starting contents cannot be used ([`Problem::BadStarting`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartingFault {
    /// No element has the id they are for.
    NoSuchBlock,
    /// The block cannot hold entities at the start ([`ElementKind::holds_at_start`]).
    CannotHold,
    /// More than the block takes in at time 0 ([`Model::most_at_start`]): `most`.
    TooMany { count: u32, most: u32 },
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
            Problem::BadPower { .. } => "bad_power",
            Problem::BadProperty { .. } => "bad_property",
            Problem::PropertyTypeConflict { .. } => "property_type_conflict",
            Problem::PropertyValueType { .. } => "property_value_type",
            Problem::BadExpression { .. } => "bad_expression",
            Problem::UnknownProperty { .. } => "unknown_property",
            Problem::ChangesWithoutProperty => "changes_without_property",
            Problem::TimesByValueWithoutProperty => "times_by_value_without_property",
            Problem::DuplicateTimeByValue { .. } => "duplicate_time_by_value",
            Problem::TimeByValueNegative { .. } => "time_by_value_negative",
            Problem::TimeForUnknownPool { .. } => "time_for_unknown_pool",
            Problem::DuplicateTimeByPool { .. } => "duplicate_time_by_pool",
            Problem::TimeForPoolNotNeeded { .. } => "time_for_pool_not_needed",
            Problem::ScrapNotAgainstServer { .. } => "scrap_not_against_server",
            Problem::PlannedRunTimeNotPositive => "planned_run_time_not_positive",
            Problem::PatienceNegative => "patience_negative",
            Problem::AbandonedWithoutPatience { .. } => "abandoned_without_patience",
            Problem::SeveralAbandonedWays { .. } => "several_abandoned_ways",
            Problem::TurnedAwayWithoutRoom { .. } => "turned_away_without_room",
            Problem::SeveralTurnedAwayWays { .. } => "several_turned_away_ways",
            Problem::TurnedAwayAndAbandoned { .. } => "turned_away_and_abandoned",
            Problem::BalkingProbability => "balking_probability",
            Problem::BalkingRowsOrder => "balking_rows_order",
            Problem::BalkedWithoutBalking { .. } => "balked_without_balking",
            Problem::SeveralBalkedWays { .. } => "several_balked_ways",
            Problem::BalkedAndOtherWay { .. } => "balked_and_other_way",
            Problem::QueueLimitWithoutRoom => "queue_limit_without_room",
            Problem::QueueByShiftEmpty => "queue_by_shift_empty",
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
            Problem::DivertNoDefault => "divert_no_default",
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
            Problem::BadConveyorValue { .. } => "bad_conveyor_value",
            Problem::ConveyorTooShort { .. } => "conveyor_too_short",
            Problem::ConveyorLengthMismatch { .. } => "conveyor_length_mismatch",
            Problem::PlaceOffPath { .. } => "place_off_path",
            Problem::PointsOutOfOrder { .. } => "points_out_of_order",
            Problem::BadJunctionTime { .. } => "bad_junction_time",
            Problem::TransportNotSimulated => "transport_not_simulated",
            Problem::DecisionUnusable { .. } => "decision_unusable",
            Problem::DecisionOverlap { .. } => "decision_overlap",
            Problem::DecisionGap { .. } => "decision_gap",
            Problem::DecisionRoute { .. } => "decision_route",
            Problem::DecisionUnchecked { .. } => "decision_unchecked",
            Problem::TransportPool { .. } => "transport_pool",
            Problem::BadMobilitySpeed { .. } => "bad_mobility_speed",
            Problem::UnknownHome { .. } => "unknown_home",
            Problem::TransportEnds { .. } => "transport_ends",
            Problem::NoShape { .. } => "no_shape",
            Problem::BadLayoutScale => "bad_layout_scale",
            Problem::ZeroQuantity => "zero_quantity",
            Problem::QuantityIgnored => "quantity_ignored",
            Problem::NothingToUnbatch => "nothing_to_unbatch",
            Problem::BadStop { .. } => "bad_stop",
            Problem::BadObservation => "bad_observation",
            Problem::BadStarting { .. } => "bad_starting",
            Problem::StopWithWarmup => "stop_with_warmup",
            Problem::EndedInWarmup { .. } => "ended_in_warmup",
            Problem::Overloaded { .. } => "overloaded",
            Problem::NoSteadyState { .. } => "no_steady_state",
            Problem::OneReplication => "one_replication",
            Problem::NoWarmup { .. } => "no_warmup",
            Problem::EstimateIgnored => "estimate_ignored",
            Problem::BadEstimate { .. } => "bad_estimate",
            Problem::EstimateMismatch => "estimate_mismatch",
            Problem::NormalOftenNegative { .. } => "normal_often_negative",
            Problem::Behaviour { found, .. } => found.code(),
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
    /// The diagnostic in English, naming blocks by id (see [`Diagnostic::plain`]).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.plain())
    }
}

/// `v` to at most four decimals, without trailing zeros: a time for a message.
fn short(v: f64) -> String {
    let s = format!("{v:.4}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// Whether no draw from `d` is negative, so its mean is the mean time it gives (durations
/// count a negative draw as 0, which would make the true mean larger).
fn never_negative(d: &Dist) -> bool {
    match d {
        Dist::Constant(v) => *v >= 0.0,
        Dist::Uniform { min, .. }
        | Dist::Triangular { min, .. }
        | Dist::TruncatedNormal { min, .. }
        | Dist::Beta { min, .. } => *min >= 0.0,
        Dist::Normal { .. } => false,
        Dist::User { points, .. } => points.iter().all(|(_, v)| *v >= 0.0),
        Dist::Exponential { .. }
        | Dist::Poisson { .. }
        | Dist::Erlang { .. }
        | Dist::LogNormal { .. }
        | Dist::Gamma { .. }
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

    /// Whether this is a Monte Carlo model: a calculation with no process ([`montecarlo`]).
    pub fn is_monte_carlo(&self) -> bool {
        self.monte_carlo.is_some() && self.elements.is_empty()
    }

    /// Whether this is an agent-based model: populations of agents with no process
    /// ([`agents`]).
    pub fn is_agents(&self) -> bool {
        self.agents.is_some() && self.elements.is_empty()
    }

    /// Every problem with the model, errors first, each group in model order. A Monte Carlo
    /// model ([`Model::is_monte_carlo`]) has none here: it has no process, and its inputs and
    /// outputs are checked by `stochcraft-mc`. An agent-based model ([`Model::is_agents`]) has
    /// only its warm-up and duration checked here, as a process model's are; its charts and
    /// populations `stochcraft-agents` checks.
    pub fn validate(&self) -> Vec<Diagnostic> {
        if self.is_monte_carlo() {
            return Vec::new();
        }
        if self.is_agents() {
            let mut out = Vec::new();
            self.check_run_length(&mut out);
            return out;
        }
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
        // A task with a resource of its own and a finite waiting room, whose entities finding
        // no room can be turned away.
        let roomy = |id: &ElementId| {
            elements.get(id).is_some_and(|e| {
                matches!(
                    e.kind,
                    ElementKind::Server {
                        queue_capacity: Some(_),
                        ..
                    }
                )
            })
        };
        // A task with a resource of its own and balking, whose arrivals may not join.
        let balky = |id: &ElementId| {
            elements.get(id).is_some_and(|e| {
                matches!(
                    e.kind,
                    ElementKind::Server {
                        balking: Some(_),
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
            if c.turned_away && c.abandoned {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::TurnedAwayAndAbandoned {
                        connection: c.id.clone(),
                    },
                ));
            } else if c.turned_away && !roomy(&c.from) {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::TurnedAwayWithoutRoom {
                        connection: c.id.clone(),
                    },
                ));
            }
            if c.balked && (c.abandoned || c.turned_away) {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::BalkedAndOtherWay {
                        connection: c.id.clone(),
                    },
                ));
            } else if c.balked && !balky(&c.from) {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::BalkedWithoutBalking {
                        connection: c.id.clone(),
                    },
                ));
            }
            if ok {
                outgoing.entry(&c.from).or_default().push(&c.to);
                if !((c.abandoned && patient(&c.from))
                    || (c.turned_away && roomy(&c.from))
                    || (c.balked && balky(&c.from)))
                {
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
            let turned_away = self
                .connections
                .iter()
                .filter(|c| c.turned_away && c.from == e.id)
                .count();
            if turned_away > 1 && roomy(&e.id) {
                out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::SeveralTurnedAwayWays { count: turned_away },
                ));
            }
            let balked = self
                .connections
                .iter()
                .filter(|c| c.balked && c.from == e.id)
                .count();
            if balked > 1 && balky(&e.id) {
                out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::SeveralBalkedWays { count: balked },
                ));
            }
            if let ElementKind::Server {
                balking: Some(b), ..
            } = &e.kind
                && let BalkJoin::Table(rows) = &b.join
            {
                if rows.iter().any(|r| !(0.0..=1.0).contains(&r.probability)) {
                    out.push(Diagnostic::error(Some(&e.id), Problem::BalkingProbability));
                }
                if rows.windows(2).any(|w| w[0].waiting >= w[1].waiting) {
                    out.push(Diagnostic::error(Some(&e.id), Problem::BalkingRowsOrder));
                }
            }
            if let ElementKind::Server {
                queue_capacity: None,
                queue_limit,
                ..
            } = &e.kind
                && !queue_limit.is_fixed()
            {
                out.push(Diagnostic::warning(
                    Some(&e.id),
                    Problem::QueueLimitWithoutRoom,
                ));
            }
            // Only for a finite room: an unlimited one ignores the setting (warned of above).
            if let ElementKind::Server {
                queue_capacity: Some(_),
                queue_limit: QueueLimit::ByShift(timed),
                ..
            } = &e.kind
                && timed.is_empty()
            {
                out.push(Diagnostic::error(Some(&e.id), Problem::QueueByShiftEmpty));
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
            // No units ever: none outside the shifts and none in any of them.
            if e.kind.most_units() == Some(0) {
                out.push(Diagnostic::error(Some(&e.id), Problem::ZeroCapacity));
            }
            if let ElementKind::Batch { size: 0 } = e.kind {
                out.push(Diagnostic::error(Some(&e.id), Problem::ZeroBatchSize));
            }
            if let ElementKind::Conveyor {
                length,
                speed,
                item_length,
                gap,
                shape,
                ..
            } = e.kind
            {
                let above_zero = |x: f64| x.is_finite() && x > 0.0;
                let mut bad = false;
                for (field, ok) in [
                    ("length", above_zero(length)),
                    ("speed", above_zero(speed)),
                    ("item_length", above_zero(item_length)),
                    ("gap", gap.is_finite() && gap >= 0.0),
                ] {
                    if !ok {
                        bad = true;
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::BadConveyorValue { field },
                        ));
                    }
                }
                let mut bad_shape = false;
                let shape_checks: &[(&'static str, bool)] = match shape {
                    ConveyorShape::Straight => &[],
                    ConveyorShape::Curve { radius, angle } => &[
                        ("radius", above_zero(radius)),
                        ("angle", angle.is_finite() && angle != 0.0),
                    ],
                    ConveyorShape::Spiral {
                        radius,
                        rise,
                        turns,
                    } => &[
                        ("radius", above_zero(radius)),
                        ("rise", above_zero(rise)),
                        ("turns", above_zero(turns)),
                    ],
                };
                for &(field, ok) in shape_checks {
                    if !ok {
                        bad_shape = true;
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::BadConveyorValue { field },
                        ));
                    }
                }
                if !bad && e.kind.conveyor_places() == Some(0) {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::ConveyorTooShort {
                            length,
                            pitch: item_length + gap,
                        },
                    ));
                }
                // A track's path gives the length (D23): a typed length that differs from it by
                // more than the step a file keeps is an override, and is flagged. Without a
                // track, the drawing shape's length is checked to within 1%.
                let track = self.site.paths.get(&e.id);
                if let Some(t) = track.filter(|_| !bad) {
                    let path_length = t.length(self.experiment.length_unit);
                    if path_length.is_finite()
                        && (path_length - length).abs()
                            > site::length_step(self.experiment.length_unit)
                    {
                        out.push(Diagnostic::warning(
                            Some(&e.id),
                            Problem::ConveyorLengthMismatch {
                                length,
                                path_length,
                            },
                        ));
                    }
                } else if !bad && !bad_shape {
                    let path_length = shape.path_length(length);
                    if path_length.is_finite() && (path_length - length).abs() > 0.01 * length {
                        out.push(Diagnostic::warning(
                            Some(&e.id),
                            Problem::ConveyorLengthMismatch {
                                length,
                                path_length,
                            },
                        ));
                    }
                }
            }
            if let ElementKind::Transport { pool, .. } = &e.kind {
                let mobility = self
                    .resources
                    .iter()
                    .find(|r| &r.id == pool)
                    .map(|r| r.mobility.as_ref());
                if !matches!(mobility, Some(Some(_))) {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::TransportPool {
                            pool: pool.clone(),
                            exists: mobility.is_some(),
                        },
                    ));
                }
                let before: Vec<&ElementId> = self
                    .connections
                    .iter()
                    .filter(|c| c.to == e.id)
                    .map(|c| &c.from)
                    .collect();
                let after: Vec<&ElementId> = self
                    .connections
                    .iter()
                    .filter(|c| c.from == e.id)
                    .map(|c| &c.to)
                    .collect();
                if before.len() != 1 || after.len() != 1 {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::TransportEnds {
                            inputs: before.len(),
                            outputs: after.len(),
                        },
                    ));
                } else {
                    // The pickup, the drop-off and the pool's home need places, else the
                    // distances between them are not known.
                    let mut places = vec![before[0], after[0]];
                    if let Some(Some(m)) = mobility {
                        places.push(&m.home);
                    }
                    let mut seen: Vec<&ElementId> = Vec::new();
                    for place in places {
                        if !seen.contains(&place) && !self.layout.contains_key(place) {
                            out.push(Diagnostic::error(
                                Some(&e.id),
                                Problem::NoShape {
                                    element: place.clone(),
                                },
                            ));
                        }
                        seen.push(place);
                    }
                }
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
                ..
            } = &e.kind
            {
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
            if let Some(by) = e.kind.processing_by() {
                let name = by.property.trim();
                if name.is_empty() {
                    if !by.by_value.is_empty() {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::TimesByValueWithoutProperty,
                        ));
                    }
                } else if !self.property_types().contains_key(name) {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::UnknownProperty {
                            name: name.to_owned(),
                        },
                    ));
                }
                let mut seen = BTreeSet::new();
                for b in &by.by_value {
                    if !seen.insert(b.value.as_str()) {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::DuplicateTimeByValue {
                                value: b.value.clone(),
                            },
                        ));
                    }
                    if b.time.check().is_ok() && b.time.mean().is_some_and(|m| m < 0.0) {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::TimeByValueNegative {
                                value: b.value.clone(),
                            },
                        ));
                    }
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
        self.check_observations(&mut out);
        self.check_estimates(&mut out);
        self.check_starting(&mut out);
        self.check_properties(&mut out);
        self.check_decisions(&mut out);
        self.check_routing(&mut out);
        self.check_site(&mut out);
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

    /// The straight-line distance between the centres of the shapes of elements `a` and `b` in
    /// [`Model::layout`], times the experiment's `layout_scale`, in its length unit. `None` if
    /// either has no shape.
    pub fn distance(&self, a: &ElementId, b: &ElementId) -> Option<f64> {
        let (ax, ay) = self.layout.get(a)?.centre();
        let (bx, by) = self.layout.get(b)?.centre();
        Some((bx - ax).hypot(by - ay) * self.experiment.layout_scale)
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
            if let Some(m) = &r.mobility {
                for (field, speed) in [
                    ("speed_empty", m.speed_empty),
                    ("speed_loaded", m.speed_loaded),
                ] {
                    if !(speed.is_finite() && speed > 0.0) {
                        out.push(Diagnostic::error(
                            Some(&r.id),
                            Problem::BadMobilitySpeed { field },
                        ));
                    }
                }
                if !self.elements.iter().any(|e| e.id == m.home) {
                    out.push(Diagnostic::error(
                        Some(&r.id),
                        Problem::UnknownHome {
                            home: m.home.clone(),
                        },
                    ));
                }
            }
        }
        self.check_calendars(out);
        let mut used: BTreeSet<&ElementId> = BTreeSet::new();
        for e in &self.elements {
            if let ElementKind::Transport { pool, .. } = &e.kind {
                used.insert(pool);
                continue;
            }
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
            // Service times by pool: each names a pool the model has and the task can draw on,
            // once. An entry the task could never use is an error, not a silent fallback.
            if let Some(by) = e.kind.processing_by_resource() {
                let mut seen = BTreeSet::new();
                for b in &by.by_resource {
                    if !seen.insert(&b.resource) {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::DuplicateTimeByPool {
                                id: b.resource.clone(),
                            },
                        ));
                    }
                    let Some(pool) = self.resources.iter().find(|r| r.id == b.resource) else {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::TimeForUnknownPool {
                                id: b.resource.clone(),
                            },
                        ));
                        continue;
                    };
                    let needed = needs.iter().any(|n| match n {
                        Need::Resource { id, .. } => *id == pool.id,
                        Need::Roles { roles, .. } => pool.has_roles(roles),
                    });
                    if !needed {
                        out.push(Diagnostic::error(
                            Some(&e.id),
                            Problem::TimeForPoolNotNeeded {
                                id: b.resource.clone(),
                            },
                        ));
                    }
                }
            }
        }
        for r in &self.resources {
            if !used.contains(&r.id) {
                out.push(Diagnostic::warning(Some(&r.id), Problem::UnusedPool));
            }
        }
    }

    /// Calendars: each referenced one exists, and each has an event.
    /// The layout's own problems (ROADMAP.md, M5.5): places along a track that fall off it,
    /// divert points whose order along a track disagrees with the order of the flows, and
    /// junctions with a transfer time below 0.
    fn check_site(&self, out: &mut Vec<Diagnostic>) {
        let unit = self.experiment.length_unit;
        for (id, place) in &self.site.places {
            let Place::Along { path, s } = place else {
                continue;
            };
            let (Some(track), Some(e)) = (self.site.paths.get(path), self.element(id)) else {
                continue;
            };
            let length = track.length(unit);
            if s.is_finite() && *s >= 0.0 && *s <= length + site::length_step(unit) {
                continue;
            }
            let problem = Problem::PlaceOffPath {
                conveyor: path.clone(),
                s: *s,
                length,
            };
            let divert = matches!(
                e.kind,
                ElementKind::ExclusiveGateway {
                    routing: Routing::Divert,
                    ..
                }
            );
            out.push(if divert {
                Diagnostic::error(Some(id), problem)
            } else {
                Diagnostic::warning(Some(id), problem)
            });
        }
        for conveyor in self.site.paths.keys() {
            let points = site::placed_in_turn(self, conveyor);
            for pair in points.windows(2) {
                if let [a, b] = pair
                    && b.s < a.s
                {
                    out.push(Diagnostic::error(
                        Some(&b.gateway),
                        Problem::PointsOutOfOrder {
                            conveyor: conveyor.clone(),
                            s: b.s,
                            before: a.gateway.clone(),
                            before_s: a.s,
                        },
                    ));
                }
            }
        }
        let kept = self.site.kept(self);
        for (id, j) in &kept.junctions {
            if !(j.transfer.is_finite() && j.transfer >= 0.0) {
                out.push(Diagnostic::error(
                    Some(id),
                    Problem::BadJunctionTime { time: j.transfer },
                ));
            }
        }
    }

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
            if let ElementKind::Server {
                availability,
                shifts,
                ..
            } = &e.kind
            {
                refs.extend(availability.iter().map(|t| (&e.id, &t.calendar)));
                refs.extend(shifts.iter().map(|t| (&e.id, &t.calendar)));
            }
            if let ElementKind::Server { queue_limit, .. } = &e.kind {
                refs.extend(queue_limit.calendars().map(|c| (&e.id, c)));
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

    /// The product of an entity, for drawing it: the first property (in the order elements and
    /// their properties are declared) that is text, with its values, those of every assignment
    /// of it that gives text (a mix's values in order, and fixed text). `None` without one.
    pub fn product_values(&self) -> Option<(String, Vec<String>)> {
        let types = self.property_types();
        let name = self
            .elements
            .iter()
            .flat_map(|e| &e.properties)
            .map(|p| &p.name)
            .find(|n| matches!(types.get(*n), Some(PropertyType::String)))?;
        let mut values: Vec<String> = Vec::new();
        for p in self
            .elements
            .iter()
            .flat_map(|e| &e.properties)
            .filter(|p| p.name == *name)
        {
            let texts: Vec<&String> = match &p.value {
                PropertyValue::Choice(v) => v.iter().map(|(_, s)| s).collect(),
                PropertyValue::Text(s) => vec![s],
                _ => Vec::new(),
            };
            for s in texts {
                if !values.contains(s) {
                    values.push(s.clone());
                }
            }
        }
        Some((name.clone(), values))
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
        // What decision tables write: a number, a string or a boolean, as the outputs give. A use
        // left behind by a block that is gone writes nothing.
        for (id, u) in &self.decisions {
            if self.element(id).is_none() {
                continue;
            }
            let Ok((_, table)) = self.decision_table(u) else {
                continue;
            };
            for (name, kind) in table.outputs() {
                types.entry(name.to_owned()).or_insert(match kind {
                    Some(dmn::Kind::Text) => PropertyType::String,
                    Some(dmn::Kind::Boolean) => PropertyType::Boolean,
                    Some(dmn::Kind::Number) | None => PropertyType::Double,
                });
            }
        }
        types
    }

    /// The decision `u` names, with its table compiled, or why it cannot be had (a sentence).
    pub fn decision_table(
        &self,
        u: &dmn::DecisionUse,
    ) -> Result<(&dmn::Decision, dmn::Compiled), String> {
        let file = self
            .dmn
            .get(&u.file)
            .ok_or_else(|| "the file was not read.".to_owned())?;
        let d = file
            .decision(&u.decision)
            .ok_or_else(|| format!("the file has no decision with the id “{}”.", u.decision))?;
        let table = d.compile().map_err(|e| format!("{e}."))?;
        Ok((d, table))
    }

    /// The ways out of `gateway`, if it is an exclusive gateway routed directly by its decision
    /// table (it has one, and none of its ways out has a condition), each with the output values
    /// that take it: its connection's id, the id of the block it leads to, and that block's name.
    pub fn decision_ways(&self, gateway: &ElementId) -> Option<Vec<(&Connection, Vec<String>)>> {
        let e = self.element(gateway)?;
        if !matches!(e.kind, ElementKind::ExclusiveGateway { .. }) {
            return None;
        }
        self.decisions.get(gateway)?;
        let ways: Vec<&Connection> = self
            .connections
            .iter()
            .filter(|c| &c.from == gateway)
            .collect();
        if ways.iter().any(|c| c.condition.is_some()) {
            return None;
        }
        Some(
            ways.into_iter()
                .map(|c| {
                    let mut keys = vec![c.id.to_string(), c.to.to_string()];
                    if let Some(name) = self
                        .element(&c.to)
                        .map(|t| t.name.trim())
                        .filter(|n| !n.is_empty())
                    {
                        keys.push(name.to_owned());
                    }
                    (c, keys)
                })
                .collect(),
        )
    }

    /// The ways out among `ways` ([`Model::decision_ways`]) that the output value `key` names: by
    /// a way's flow id, the id of the block it leads to, or that block's name, compared exactly.
    /// A valid model has each value its table can give name exactly one.
    pub fn ways_named<'a>(
        ways: &[(&'a Connection, Vec<String>)],
        key: &str,
    ) -> Vec<&'a Connection> {
        ways.iter()
            .filter(|(_, keys)| keys.iter().any(|k| k == key))
            .map(|(c, _)| *c)
            .collect()
    }

    /// The numbers among the names of `ways` ([`Model::decision_ways`]), for a table whose
    /// aggregation gives a number that is not among its cells: each number once, with the name
    /// it was first written as and the ways out a name of which reads as it. Names are compared
    /// as numbers, so `1` and `1.0` name the same number; a valid model has each name one way.
    pub fn ways_by_number<'a>(
        ways: &[(&'a Connection, Vec<String>)],
    ) -> Vec<(f64, String, Vec<&'a Connection>)> {
        let mut out: Vec<(f64, String, Vec<&'a Connection>)> = Vec::new();
        for (c, keys) in ways {
            for k in keys {
                let Some(x) = k.trim().parse::<f64>().ok().filter(|x| x.is_finite()) else {
                    continue;
                };
                match out.iter_mut().find(|(y, _, _)| *y == x) {
                    Some((_, _, named)) => {
                        if !named.iter().any(|d| d.id == c.id) {
                            named.push(c);
                        }
                    }
                    None => out.push((x, k.clone(), vec![*c])),
                }
            }
        }
        out
    }

    /// The output a gateway routed directly by its decision table routes by: the one its
    /// decision use names, else the table's only output; `Err` with the reason otherwise.
    pub fn route_output(u: &dmn::DecisionUse, table: &dmn::Compiled) -> Result<usize, String> {
        let outputs = table.outputs();
        match &u.route {
            Some(r) => outputs
                .iter()
                .position(|(n, _)| n == r)
                .ok_or_else(|| format!("the table has no output named “{r}” to route by.")),
            None if outputs.len() == 1 => Ok(0),
            None => Err(
                "the table has several outputs, and the gateway does not say which to route by."
                    .to_owned(),
            ),
        }
    }

    fn check_decisions(&self, out: &mut Vec<Diagnostic>) {
        let types = self.property_types();
        for (id, u) in &self.decisions {
            // A use left behind by a block that is gone is not part of the model.
            let Some(e) = self.element(id) else {
                continue;
            };
            let unusable = |reason: String| {
                Diagnostic::error(
                    Some(id),
                    Problem::DecisionUnusable {
                        file: u.file.clone(),
                        decision: u.decision.clone(),
                        reason,
                    },
                )
            };
            if !e.kind.reads_decisions() {
                let reason = match &e.kind {
                    ElementKind::ExclusiveGateway { .. } => {
                        "a gateway that sends each entity to the shortest queue reads no decision table."
                    }
                    _ => "only a task or an exclusive gateway can use a decision table.",
                };
                out.push(unusable(reason.to_owned()));
                continue;
            }
            let (d, table) = match self.decision_table(u) {
                Ok(x) => x,
                Err(reason) => {
                    out.push(unusable(reason));
                    continue;
                }
            };
            if table.gives_list() {
                out.push(unusable(
                    "under the hit policy Collect without an aggregation it gives a list, which an entity property cannot hold; choose an aggregation or another hit policy."
                        .to_owned(),
                ));
                continue;
            }
            let label = d.label().to_owned();
            for name in table.input_names() {
                if !types.contains_key(name) {
                    out.push(Diagnostic::error(
                        Some(id),
                        Problem::UnknownProperty {
                            name: name.to_owned(),
                        },
                    ));
                }
            }
            // Each output must fit the property it writes: a string written into a boolean
            // would read as true, and no rule testing for that string would match it again.
            let whole = |vs: &[dmn::Literal]| {
                vs.iter().all(|v| match v {
                    dmn::Literal::Number(x) => x.fract() == 0.0,
                    _ => true,
                })
            };
            // A long holds -2⁶³ up to 2⁶³ - 1; a number beyond that would be held as the nearest
            // end of it, not as the table gave it.
            let long = |vs: &[f64]| {
                vs.iter().all(|x| {
                    (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(x)
                })
            };
            let numbers = |vs: &[dmn::Literal]| -> Vec<f64> {
                vs.iter()
                    .filter_map(|v| match v {
                        dmn::Literal::Number(x) => Some(*x),
                        _ => None,
                    })
                    .collect()
            };
            // What values of `kind` (all whole numbers, or not; all within a long, or not) give
            // that the property's type `ty` cannot hold, if anything.
            let misfit =
                |kind: Option<dmn::Kind>, whole: bool, fits: bool, ty: PropertyType| match kind {
                    Some(dmn::Kind::Number) if !ty.is_numeric() => Some("numbers"),
                    Some(dmn::Kind::Number) if ty == PropertyType::Long && !whole => {
                        Some("fractional numbers")
                    }
                    Some(dmn::Kind::Number) if ty == PropertyType::Long && !fits => {
                        Some("numbers beyond -9223372036854775808 to 9223372036854775807")
                    }
                    Some(dmn::Kind::Text)
                        if !matches!(ty, PropertyType::String | PropertyType::DateTime) =>
                    {
                        Some("text")
                    }
                    Some(dmn::Kind::Boolean) if ty != PropertyType::Boolean => {
                        Some("true or false")
                    }
                    _ => None,
                };
            // Each input must compare values of its property's type: a number test never
            // matches a boolean, so every entity would fall to the default (or to none) unseen.
            for (name, kind) in table.inputs() {
                let Some(&ty) = types.get(name) else {
                    continue;
                };
                if let Some(compares) = misfit(kind, true, true, ty) {
                    out.push(unusable(format!(
                        "its input “{name}” compares {compares}, but the property “{name}” is of type {}.",
                        ty.bpsim_name()
                    )));
                }
            }
            for (j, (name, kind)) in table.outputs().into_iter().enumerate() {
                let Some(&ty) = types.get(name) else {
                    continue;
                };
                // An aggregation's result: COUNT is always whole; SUM, MIN and MAX of the cells.
                let cells = table.aggregation() == Some(dmn::Aggregation::Count)
                    || whole(&table.output_values(j));
                // And within a long: a SUM's least and greatest sums, of every rule giving less
                // than 0 and of every rule giving more (two rules giving one number add it
                // twice); else the cells themselves.
                let given = numbers(&table.output_values(j));
                let fits = match table.aggregation() {
                    Some(dmn::Aggregation::Count) => true,
                    Some(dmn::Aggregation::Sum) => {
                        let each = numbers(&table.rule_outputs(j));
                        let low: f64 = each.iter().filter(|x| **x < 0.0).sum();
                        let high: f64 = each.iter().filter(|x| **x > 0.0).sum();
                        long(&[low, high])
                    }
                    _ => long(&given),
                };
                if let Some(gives) = misfit(kind, cells, fits, ty) {
                    out.push(unusable(format!(
                        "its output “{name}” gives {gives}, which the property “{name}”, of type {}, cannot hold.",
                        ty.bpsim_name()
                    )));
                    continue;
                }
                // With an aggregation, the default is given as it is when no rule matches, not
                // aggregated, so it is checked on its own.
                if table.aggregation().is_some()
                    && let Some(d) = table.default_value(j)
                    && let Some(gives) = misfit(
                        Some(d.kind()),
                        whole(std::slice::from_ref(d)),
                        long(&numbers(std::slice::from_ref(d))),
                        ty,
                    )
                {
                    out.push(unusable(format!(
                        "its output “{name}” gives {gives} by default, when no rule matches, which the property “{name}”, of type {}, cannot hold.",
                        ty.bpsim_name()
                    )));
                }
            }
            if let Some(ways) = self.decision_ways(id) {
                match Self::route_output(u, &table) {
                    // An aggregation's value is not among the cells: the numbers the ways are
                    // named by are what it can route to, compared as routing compares them.
                    Ok(j) if table.aggregation().is_some() => {
                        let by_number = Self::ways_by_number(&ways);
                        for (_, key, named) in &by_number {
                            if named.len() > 1 {
                                out.push(unusable(format!(
                                    "its aggregation can give {key}, which names more than one of the gateway's ways out (a way's id, or the id or name of the block it leads to, read as a number: 1 and 1.0 are the same); make each number name one."
                                )));
                            }
                        }
                        // Without a default way, an entity whose number names no way out is
                        // lost here: each number the aggregation can give, and its default,
                        // must name one. Where those are too many to list, a default way is
                        // needed.
                        let has_default = matches!(
                            &e.kind,
                            ElementKind::ExclusiveGateway {
                                default: Some(_),
                                ..
                            }
                        );
                        if !has_default {
                            let route = |value: String| {
                                Diagnostic::error(
                                    Some(id),
                                    Problem::DecisionRoute {
                                        decision: label.clone(),
                                        value,
                                    },
                                )
                            };
                            match table.aggregates() {
                                Some(results) => {
                                    for x in results {
                                        if !by_number.iter().any(|(y, _, _)| *y == x) {
                                            out.push(route(dmn::Literal::Number(x).text()));
                                        }
                                    }
                                }
                                None => out.push(unusable(format!(
                                    "its aggregation SUM can give the sum of any of its rules, more than {} of them, too many to check that each names a way out; give the gateway a default way.",
                                    dmn::MAX_SUMMED
                                ))),
                            }
                            match table.default_value(j) {
                                Some(dmn::Literal::Number(x))
                                    if !by_number.iter().any(|(y, _, _)| y == x) =>
                                {
                                    out.push(route(dmn::Literal::Number(*x).text()));
                                }
                                Some(d @ (dmn::Literal::Text(_) | dmn::Literal::Boolean(_))) => {
                                    out.push(route(d.text()));
                                }
                                _ => {}
                            }
                        }
                    }
                    Ok(j) => {
                        for v in table.output_values(j) {
                            let key = v.text();
                            match Self::ways_named(&ways, &key).len() {
                                1 => {}
                                0 => out.push(Diagnostic::error(
                                    Some(id),
                                    Problem::DecisionRoute {
                                        decision: label.clone(),
                                        value: key,
                                    },
                                )),
                                _ => out.push(unusable(format!(
                                    "the output it routes by can give “{key}”, which names more than one of the gateway's ways out (a way's id, or the id or name of the block it leads to); make it name one."
                                ))),
                            }
                        }
                    }
                    Err(reason) => out.push(unusable(reason)),
                }
            }
            let integer: Vec<bool> = table
                .input_names()
                .iter()
                .map(|n| types.get(*n) == Some(&PropertyType::Long))
                .collect();
            for f in table.check_bound(&integer) {
                out.push(match f {
                    dmn::Finding::Overlap {
                        first,
                        second,
                        example,
                    } => Diagnostic::error(
                        Some(id),
                        Problem::DecisionOverlap {
                            decision: label.clone(),
                            first,
                            second,
                            example,
                        },
                    ),
                    dmn::Finding::Gap { example } => Diagnostic::warning(
                        Some(id),
                        Problem::DecisionGap {
                            decision: label.clone(),
                            example,
                        },
                    ),
                    dmn::Finding::Unchecked => Diagnostic::warning(
                        Some(id),
                        Problem::DecisionUnchecked {
                            decision: label.clone(),
                        },
                    ),
                });
            }
        }
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
                            reason: PropertyFault::NoName,
                        },
                    ));
                    continue;
                }
                if !seen.insert(p.name.as_str()) {
                    out.push(Diagnostic::error(
                        at,
                        Problem::BadProperty {
                            name,
                            reason: PropertyFault::SetTwice,
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
                                    reason: PropertyFault::BadShares,
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
                        IgnoredWhat::Condition
                    } else {
                        IgnoredWhat::Probability
                    };
                    out.push(Diagnostic::warning(
                        at,
                        Problem::RoutingIgnored {
                            what,
                            reason: IgnoredWhy::NotDecision,
                        },
                    ));
                }
            }
            let ElementKind::ExclusiveGateway { default, routing } = &e.kind else {
                continue;
            };
            if *routing == Routing::Divert && default.is_none() {
                out.push(Diagnostic::error(Some(&e.id), Problem::DivertNoDefault));
            }
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
                for c in &outs {
                    let what = if c.condition.is_some() {
                        IgnoredWhat::Condition
                    } else if c.probability.is_some() {
                        IgnoredWhat::Probability
                    } else if default.as_ref() == Some(&c.id) {
                        IgnoredWhat::Default
                    } else {
                        continue;
                    };
                    out.push(Diagnostic::warning(
                        Some(&c.id),
                        Problem::RoutingIgnored {
                            what,
                            reason: IgnoredWhy::ShortestQueue,
                        },
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
                            what: IgnoredWhat::Probability,
                            reason: IgnoredWhy::ByCondition,
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
        self.check_run_length(out);
        if !(x.layout_scale.is_finite() && x.layout_scale > 0.0) {
            out.push(Diagnostic::error(None, Problem::BadLayoutScale));
        }
        if x.replications == 0 {
            out.push(Diagnostic::error(
                None,
                Problem::BadExperiment {
                    field: ExperimentField::Replications,
                },
            ));
        }
    }

    /// The warm-up, finite and 0 or more, and the duration, finite and above 0.
    fn check_run_length(&self, out: &mut Vec<Diagnostic>) {
        let x = &self.experiment;
        if !(x.warmup.is_finite() && x.warmup >= 0.0) {
            out.push(Diagnostic::error(
                None,
                Problem::BadExperiment {
                    field: ExperimentField::Warmup,
                },
            ));
        }
        if !(x.duration.is_finite() && x.duration > 0.0) {
            out.push(Diagnostic::error(
                None,
                Problem::BadExperiment {
                    field: ExperimentField::Duration,
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
        // The capacity table passes the load on through routing, joins and shared pools; a
        // station is warned about only when it is surely at 100% or more (a lower bound, such
        // as the least of times by property or the most units on shift, will do).
        for st in self.capacity(capacity::DEFAULT_TARGET).stations {
            // A shared task shares its pool's status: the pool is warned about, once.
            if st.status == capacity::Status::Unstable
                && st.kind != capacity::StationKind::SharedTask
                && st.kind != capacity::StationKind::Transport
                && let Some(load) = st.load.value()
            {
                out.push(Diagnostic::warning(
                    Some(&st.id),
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
            (_, None) => Some(StopFault::NoSuchBlock),
            (Stop::Count { count: 0, .. }, _) => Some(StopFault::ZeroCount),
            (Stop::Count { .. }, Some(_)) => None,
            (
                Stop::Empty { .. },
                Some(
                    ElementKind::Server { .. }
                    | ElementKind::SharedTask { .. }
                    | ElementKind::Delay { .. },
                ),
            ) => None,
            (Stop::Empty { .. }, Some(_)) => Some(StopFault::NotATask),
        };
        if let Some(reason) = reason {
            out.push(Diagnostic::error(Some(id), Problem::BadStop { reason }));
        } else if self.experiment.warmup > 0.0 {
            out.push(Diagnostic::warning(Some(id), Problem::StopWithWarmup));
        }
    }

    /// Each observed value is for a block or pool the model has. A warning, not an error: the
    /// run still goes, and the results say there is nothing to check that observation against.
    fn check_observations(&self, out: &mut Vec<Diagnostic>) {
        for o in &self.experiment.observed {
            let known = self.element(&o.element).is_some()
                || self.resources.iter().any(|r| r.id == o.element);
            if !known {
                out.push(Diagnostic::warning(
                    Some(&o.element),
                    Problem::BadObservation,
                ));
            }
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

    /// Each block's starting contents are for a block that exists, can hold entities at the
    /// start, and fit in its places. A count of 0 is the same as none.
    fn check_starting(&self, out: &mut Vec<Diagnostic>) {
        for (id, &count) in &self.starting {
            if count == 0 {
                continue;
            }
            let reason = match self.element(id).map(|e| &e.kind) {
                None => Some(StartingFault::NoSuchBlock),
                Some(kind) if !kind.holds_at_start() => Some(StartingFault::CannotHold),
                Some(kind) => self
                    .most_at_start(kind)
                    .filter(|most| count > *most)
                    .map(|most| StartingFault::TooMany { count, most }),
            };
            if let Some(reason) = reason {
                out.push(Diagnostic::error(Some(id), Problem::BadStarting { reason }));
            }
        }
    }

    /// The most entities a block of kind `kind` can take in as a run starts, as arrivals at
    /// time 0 would be let in; `None`: no limit (no finite waiting room).
    ///
    /// - A fixed waiting room has a place for every unit the server ever has on shift plus the
    ///   room: the places it starts with.
    /// - A limit by timetable or per working server lets an entity in while those waiting, less
    ///   the idle working units, are fewer than the limit. At time 0 every unit on shift is idle
    ///   and working, so it lets in the units on shift then (by the server's timetable and
    ///   availability) plus the limit then: the timetable's value at time 0, or the number per
    ///   working unit times those units.
    pub fn most_at_start(&self, kind: &ElementKind) -> Option<u32> {
        let ElementKind::Server {
            capacity,
            queue_capacity: Some(q),
            queue_limit,
            shifts,
            available,
            availability,
            ..
        } = kind
        else {
            return None;
        };
        if queue_limit.is_fixed() {
            return Some(kind.most_units().unwrap_or(0).saturating_add(*q));
        }
        let units = self
            .timed_at_start(shifts)
            .and_then(|i| shifts.get(i))
            .map_or(*capacity, |s| s.value);
        let on = self
            .timed_at_start(availability)
            .and_then(|i| availability.get(i))
            .map_or(*available, |a| a.value);
        let working = if on { units } else { 0 };
        let limit = match queue_limit {
            QueueLimit::ByShift(timed) => self
                .timed_at_start(timed)
                .and_then(|i| timed.get(i))
                .map_or(*q, |t| t.value),
            QueueLimit::PerWorkingServer => q.saturating_mul(working),
            QueueLimit::Fixed => *q,
        };
        Some(working.saturating_add(limit))
    }

    /// Which of `timed`'s values applies at time 0 (the first whose calendar is active then), as
    /// a run reads them: calendars from the experiment's start, or else from midnight of the
    /// earliest calendar event. `None`: none applies (the value outside them does).
    fn timed_at_start<T>(&self, timed: &[Timed<T>]) -> Option<usize> {
        if timed.is_empty() {
            return None;
        }
        let x = &self.experiment;
        let origin = x
            .start
            .or_else(|| Calendar::default_origin(&self.calendars))
            .unwrap_or(DateTime(0));
        let end = x.warmup + x.duration;
        let end = if end.is_finite() && end > 0.0 {
            end
        } else {
            0.0
        };
        let periods = |c: &ElementId| -> Vec<(f64, f64)> {
            self.calendars
                .iter()
                .find(|k| &k.id == c)
                .map(|k| k.periods(origin, x.time_unit, end))
                .unwrap_or_default()
        };
        calendar::timeline(timed, &periods, end)
            .first()
            .and_then(|(_, i)| *i)
    }

    /// The blocks with entities inside as a run starts ([`Model::starting`]), in model order.
    pub fn started_blocks(&self) -> impl Iterator<Item = (usize, &Element, u32)> {
        self.elements.iter().enumerate().filter_map(|(i, e)| {
            let n = self.starting.get(&e.id).copied().filter(|n| *n > 0)?;
            e.kind.holds_at_start().then_some((i, e, n))
        })
    }

    /// Whether the model is closed: a fixed number of entities go round and none leave, as in
    /// a closed queueing network (Gordon and Newell, 1967) or a finite-source model such as
    /// machines that fail and wait for a fitter (M/M/c//N). Such a model needs no exit.
    ///
    /// Closed means: entities start somewhere (a source or a block with starting contents);
    /// every source makes a limited number; the model has no exit; no block copies, combines or
    /// splits entities (a parallel gateway, a batch, an unbatch), so their number stays fixed;
    /// and from every block where entities enter (those the sources send to, and those with
    /// starting contents) every block they can reach leads back to it. A loop that drains into
    /// another loop and never comes back, or a way that ends nowhere, is therefore not closed.
    /// Entities that give up, balk or find no way at a gateway leave without an exit; a model
    /// with such ways is closed only as far as its blocks and connections go.
    pub fn is_closed(&self) -> bool {
        let mut outgoing: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
        let mut incoming: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
        for c in &self.connections {
            outgoing.entry(&c.from).or_default().push(&c.to);
            incoming.entry(&c.to).or_default().push(&c.from);
        }
        let mut entries: Vec<&ElementId> = self.started_blocks().map(|(_, e, _)| &e.id).collect();
        for e in &self.elements {
            match &e.kind {
                ElementKind::Source { limit: None, .. } | ElementKind::Sink { .. } => return false,
                ElementKind::Source { .. } => {
                    for &to in outgoing.get(&e.id).into_iter().flatten() {
                        if !entries.contains(&to) {
                            entries.push(to);
                        }
                    }
                }
                _ => {}
            }
        }
        if entries.is_empty() {
            return false;
        }
        entries.iter().all(|&entry| {
            let forward = reach(&[entry], |id| outgoing.get(id).cloned().unwrap_or_default());
            let back = reach(&[entry], |id| incoming.get(id).cloned().unwrap_or_default());
            forward.iter().all(|id| {
                back.contains(id)
                    && self.element(id).is_some_and(|e| {
                        !matches!(
                            e.kind,
                            ElementKind::Source { .. }
                                | ElementKind::ParallelGateway
                                | ElementKind::Batch { .. }
                                | ElementKind::Unbatch
                        )
                    })
            })
        })
    }

    /// The time distribution of element `id` that a three-point estimate is for: a source's time
    /// between arrivals, a task's service time, a delay's duration.
    pub fn estimated(&self, id: &ElementId) -> Option<&Dist> {
        match &self.element(id)?.kind {
            ElementKind::Source { interarrival, .. } => Some(interarrival),
            ElementKind::Server { service, .. } | ElementKind::SharedTask { service, .. } => {
                Some(service)
            }
            ElementKind::Delay { duration, .. } => Some(duration),
            _ => None,
        }
    }

    /// Finite buffers fed through gateways, and loops of finite buffers that can deadlock.
    fn check_buffers<'a>(
        &'a self,
        outgoing: &BTreeMap<&'a ElementId, Vec<&'a ElementId>>,
        out: &mut Vec<Diagnostic>,
    ) {
        // A finite waiting room that holds its senders up: not one whose entities finding no
        // room are turned away, which never wait before it.
        let finite = |id: &ElementId| {
            self.element(id).is_some_and(|e| {
                matches!(
                    e.kind,
                    ElementKind::Server {
                        queue_capacity: Some(_),
                        ..
                    }
                )
            }) && !self
                .connections
                .iter()
                .any(|c| c.turned_away && &c.from == id)
        };
        for e in &self.elements {
            if !finite(&e.id) {
                continue;
            }
            for (from, tos) in outgoing {
                // A shortest-queue or divert gateway leaves the waiting to its sender.
                let holds_up = self.element(from).is_some_and(|g| {
                    g.kind.is_gateway()
                        && !matches!(
                            g.kind,
                            ElementKind::ExclusiveGateway {
                                routing: Routing::ShortestQueue | Routing::Divert,
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
        // A sender into a shortest-queue gateway routes through it and waits only when every
        // way is full, so it waits for the gateway's ways when all of them are finite buffers.
        // A way into another gateway is not: that gateway takes the entity in itself, without
        // limit, and does its own waiting. The buffers an entity sent to `to` may wait for.
        let shortest_queue = |id: &ElementId| {
            self.element(id).is_some_and(|g| {
                matches!(
                    g.kind,
                    ElementKind::ExclusiveGateway {
                        routing: Routing::ShortestQueue,
                        ..
                    }
                )
            })
        };
        let waits_for = |to: &'a ElementId| -> Vec<&'a ElementId> {
            if finite(to) {
                return vec![to];
            }
            match outgoing.get(to) {
                Some(ways)
                    if shortest_queue(to) && !ways.is_empty() && ways.iter().all(|w| finite(w)) =>
                {
                    ways.clone()
                }
                _ => Vec::new(),
            }
        };
        let next = |id: &ElementId| -> Vec<&'a ElementId> {
            let mut tos: Vec<&ElementId> = Vec::new();
            for &to in outgoing.get(id).into_iter().flatten() {
                for t in waits_for(to) {
                    if !tos.contains(&t) {
                        tos.push(t);
                    }
                }
            }
            tos
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
        // Where entities come from: sources, and blocks with entities inside at the start.
        let sources: Vec<&ElementId> = self
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Source { .. }))
            .map(|e| &e.id)
            .chain(self.started_blocks().map(|(_, e, _)| &e.id))
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
        // A closed model's entities go round for ever and need no exit ([`Model::is_closed`]).
        if sinks.is_empty() && !self.elements.is_empty() && !self.is_closed() {
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

    #[test]
    fn an_agent_based_models_warm_up_and_duration_are_checked() {
        let fields = |warmup: f64, duration: f64| -> Vec<Problem> {
            let mut m = Model {
                agents: Some(agents::Agents::default()),
                ..Model::default()
            };
            m.experiment.warmup = warmup;
            m.experiment.duration = duration;
            assert!(m.is_agents());
            m.validate().into_iter().map(|d| d.problem).collect()
        };
        let warmup = Problem::BadExperiment {
            field: ExperimentField::Warmup,
        };
        let duration = Problem::BadExperiment {
            field: ExperimentField::Duration,
        };
        assert_eq!(fields(0.0, 60.0), vec![]);
        assert_eq!(fields(10.0, 60.0), vec![]);
        // A negative duration that a warm-up would hide in their sum is still refused.
        assert_eq!(fields(2.0, -1.0), vec![duration.clone()]);
        assert_eq!(fields(-1.0, 60.0), vec![warmup.clone()]);
        assert_eq!(fields(f64::NAN, 0.0), vec![warmup, duration.clone()]);
        assert_eq!(fields(0.0, f64::INFINITY), vec![duration]);
    }

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
                == "error: The expression for the property “late” on machine cannot be used: the expression ends too early (at character 25). Correct “getProperty('missing') +”."),
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
            turned_away: false,
            balked: false,
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
                processing_by: None,
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
                        processing_by: None,
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
                        balking: None,
                        health_check: None,
                        shifts: Vec::new(),
                        on_shift_end: OnShiftEnd::Finish,
                        queue_limit: QueueLimit::Fixed,
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
            starting: BTreeMap::new(),
            monte_carlo: None,
            site: Default::default(),
            agents: None,
            decisions: BTreeMap::new(),
            dmn: BTreeMap::new(),
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
        assert!(ds[0].to_string().contains("through pick"), "{}", ds[0]);
    }

    #[test]
    fn a_loop_of_finite_buffers_may_deadlock() {
        let server = |q| ElementKind::Server {
            processing_by: None,
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
            balking: None,
            health_check: None,
            shifts: Vec::new(),
            on_shift_end: OnShiftEnd::Finish,
            queue_limit: QueueLimit::Fixed,
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
        assert!(cycle[0].to_string().contains("a → b"), "{}", cycle[0]);
    }

    #[test]
    fn a_loop_through_a_shortest_queue_gateway_may_deadlock() {
        let server = |q| ElementKind::Server {
            processing_by: None,
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
            balking: None,
            health_check: None,
            shifts: Vec::new(),
            on_shift_end: OnShiftEnd::Finish,
            queue_limit: QueueLimit::Fixed,
        };
        let shortest = || ElementKind::ExclusiveGateway {
            default: None,
            routing: Routing::ShortestQueue,
        };
        let cycles = |m: &Model| -> Vec<String> {
            m.validate()
                .iter()
                .filter(|d| d.problem.code() == "blocking_cycle")
                .map(ToString::to_string)
                .collect()
        };
        // machine → a → pick → b → a: the gateway waits while b is full, so a waits for b and b
        // for a.
        let mut m = mm1();
        m.elements.push(el("a", server(Some(1))));
        m.elements.push(el("b", server(Some(1))));
        m.elements.push(el("pick", shortest()));
        m.connections[1].to = ElementId::unchecked("a");
        m.connections.push(link("f3", "a", "pick"));
        m.connections.push(link("f4", "pick", "b"));
        m.connections.push(link("f5", "b", "a"));
        let found = cycles(&m);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("a → b"), "{found:?}");
        // A way into another gateway: that one takes entities in without limit and waits
        // itself, so a → pick → again → b → a holds nothing up in a loop.
        m.elements.push(el("again", shortest()));
        m.connections.retain(|c| c.id.as_str() != "f4");
        m.connections.push(link("f6", "pick", "again"));
        m.connections.push(link("f7", "again", "b"));
        assert_eq!(cycles(&m), Vec::<String>::new());
        // A way out with unlimited room means the gateway never waits: no loop.
        m.connections
            .retain(|c| c.id.as_str() != "f6" && c.id.as_str() != "f7");
        m.connections.push(link("f8", "pick", "b"));
        m.connections.push(link("f9", "pick", "exit"));
        assert_eq!(cycles(&m), Vec::<String>::new());
        m.connections.retain(|c| c.id.as_str() != "f9");
        let found = cycles(&m);
        assert_eq!(found.len(), 1, "{found:?}");
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
                processing_by: None,
                duration: Dist::Constant(1.0),
            },
        ));
        m.elements.push(el(
            "idle",
            ElementKind::Server {
                processing_by: None,
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
                balking: None,
                health_check: None,
                shifts: Vec::new(),
                on_shift_end: OnShiftEnd::Finish,
                queue_limit: QueueLimit::Fixed,
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
                processing_by: None,
                duration: Dist::Constant(1.0),
            },
        ));
        m.connections.push(link("f3", "orphan", "exit"));
        m.elements.push(el(
            "a",
            ElementKind::Delay {
                processing_by: None,
                duration: Dist::Constant(1.0),
            },
        ));
        m.elements.push(el(
            "b",
            ElementKind::Delay {
                processing_by: None,
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
        assert!(message.contains("Service time"), "{message}");
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
    fn transports_and_mobile_pools_are_checked() {
        let shape = |x: f64| Bounds {
            x,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let mut m = mm1();
        m.elements[1].kind = ElementKind::Transport {
            pool: id("ops"),
            load: Dist::Constant(1.0),
            unload: Dist::Constant(1.0),
        };
        assert_eq!(m.elements[1].kind.name(), "transport");
        let has = |m: &Model, what: &str| {
            problems(m)
                .iter()
                .any(|(s, _, x)| *s == Severity::Error && x.contains(what))
        };
        // No such pool, and no places on the canvas.
        assert!(has(&m, "exists: false"));
        assert!(has(&m, "NoShape"));
        assert_eq!(m.distance(&id("arrivals"), &id("exit")), None);
        // A pool that is not mobile.
        m.resources = vec![ResourcePool::new(id("ops"), "Operators", 1, Vec::new())];
        assert!(has(&m, "exists: true"));
        // A mobile one, with places for the pickup, the drop-off and the home.
        m.resources[0].mobility = Some(Mobility {
            speed_empty: 1.0,
            speed_loaded: 2.0,
            home: id("arrivals"),
            after_job: AfterJob::Stay,
        });
        m.layout.insert(id("arrivals"), shape(0.0));
        m.layout.insert(id("exit"), shape(30.0));
        assert!(
            m.validate().iter().all(|d| d.severity != Severity::Error),
            "{:?}",
            m.validate()
        );
        // The straight line between the centres, times the scale.
        let (a, b) = (id("arrivals"), id("exit"));
        assert_eq!(m.distance(&a, &b), Some(30.0));
        m.experiment.layout_scale = 0.5;
        assert_eq!(m.distance(&a, &b), Some(15.0));
        m.experiment.layout_scale = 0.0;
        assert!(has(&m, "BadLayoutScale"));
        m.experiment.layout_scale = 1.0;
        // A drop-off without a place.
        m.layout.remove(&b);
        assert!(has(&m, "NoShape"));
        m.layout.insert(b, shape(30.0));
        // Speeds above 0, and a home that is an element.
        m.resources[0].mobility = Some(Mobility {
            speed_empty: 0.0,
            speed_loaded: f64::NAN,
            home: id("nowhere"),
            after_job: AfterJob::GoHome,
        });
        assert!(has(&m, "BadMobilitySpeed { field: \"speed_empty\" }"));
        assert!(has(&m, "BadMobilitySpeed { field: \"speed_loaded\" }"));
        assert!(has(&m, "UnknownHome"));
        // Exactly one way out.
        m.resources[0].mobility = Some(Mobility {
            speed_empty: 1.0,
            speed_loaded: 2.0,
            home: id("arrivals"),
            after_job: AfterJob::Stay,
        });
        let mut second = m.connections[1].clone();
        second.id = id("out2");
        m.connections.push(second);
        assert!(has(&m, "TransportEnds { inputs: 1, outputs: 2 }"));
        assert_eq!(
            AfterJob::from_name(AfterJob::GoHome.name()),
            Some(AfterJob::GoHome)
        );
        assert_eq!(AfterJob::from_name("stay"), Some(AfterJob::Stay));
        assert_eq!(AfterJob::from_name("go"), None);
    }

    #[test]
    fn length_units_read_back_by_name() {
        for u in [
            LengthUnit::Millimetres,
            LengthUnit::Centimetres,
            LengthUnit::Metres,
            LengthUnit::Kilometres,
            LengthUnit::Inches,
            LengthUnit::Feet,
        ] {
            assert_eq!(LengthUnit::from_name(u.name()), Some(u));
        }
        assert_eq!(LengthUnit::default(), LengthUnit::Metres);
        assert_eq!(LengthUnit::from_name("metres"), None);
    }

    /// A length that is a whole number of pitches holds that many, although the pitch (a sum of
    /// decimals) and the quotient are rounded; a length just short of one holds one fewer.
    #[test]
    fn a_whole_number_of_pitches_holds_that_many() {
        let places = |length, item_length, gap| {
            ElementKind::Conveyor {
                length,
                speed: 0.5,
                accumulating: true,
                item_length,
                gap,
                shape: ConveyorShape::Straight,
            }
            .conveyor_places()
        };
        // 0.4 + 0.2 is 0.6000000000000001: a plain floor gives 49, 19 and 0.
        assert_eq!(places(30.0, 0.4, 0.2), Some(50));
        assert_eq!(places(12.0, 0.4, 0.2), Some(20));
        assert_eq!(places(4.0, 0.4, 0.2), Some(6));
        assert_eq!(places(0.6, 0.4, 0.2), Some(1));
        assert_eq!(places(29.99, 0.4, 0.2), Some(49));
        assert_eq!(places(0.59, 0.4, 0.2), Some(0));
        // Exact in binary.
        assert_eq!(places(3.0, 0.5, 0.0), Some(6));
        assert_eq!(places(2.9, 0.5, 0.0), Some(5));
        // Invalid numbers hold none.
        assert_eq!(places(f64::NAN, 0.4, 0.2), Some(0));
        assert_eq!(places(-3.0, 0.4, 0.2), Some(0));
        assert_eq!(places(3.0, f64::NAN, 0.2), Some(0));
        assert_eq!(places(3.0, 0.0, 0.0), Some(0));
        assert_eq!(whole_pitches(30.0, 0.4 + 0.2), 50);
        assert_eq!(whole_pitches(f64::NEG_INFINITY, 0.6), 0);
        assert_eq!(ElementKind::ParallelGateway.conveyor_places(), None);
    }

    #[test]
    fn conveyors_are_checked() {
        let belt = |length, speed, item_length, gap| ElementKind::Conveyor {
            length,
            speed,
            accumulating: true,
            item_length,
            gap,
            shape: ConveyorShape::Straight,
        };
        let shaped = |length, shape| ElementKind::Conveyor {
            length,
            speed: 1.0,
            accumulating: true,
            item_length: 0.5,
            gap: 0.0,
            shape,
        };
        let mut m = mm1();
        m.elements[1].kind = belt(10.0, 5.0, 0.8, 0.2);
        assert_eq!(m.elements[1].kind.conveyor_places(), Some(10));
        assert!(
            m.validate().iter().all(|d| d.severity != Severity::Error),
            "{:?}",
            m.validate()
        );
        // Each number: not above 0, or not finite; the gap may be 0; angle must be nonzero.
        for (kind, field) in [
            (belt(0.0, 0.5, 0.8, 0.2), "length"),
            (belt(10.0, -1.0, 0.8, 0.2), "speed"),
            (belt(10.0, 5.0, f64::NAN, 0.2), "item_length"),
            (belt(10.0, 5.0, 0.8, -0.1), "gap"),
            (belt(f64::INFINITY, 5.0, 0.8, 0.2), "length"),
            (
                shaped(
                    5.0,
                    ConveyorShape::Curve {
                        radius: 0.0,
                        angle: 90.0,
                    },
                ),
                "radius",
            ),
            (
                shaped(
                    5.0,
                    ConveyorShape::Curve {
                        radius: 2.0,
                        angle: 0.0,
                    },
                ),
                "angle",
            ),
            (
                shaped(
                    5.0,
                    ConveyorShape::Curve {
                        radius: 2.0,
                        angle: f64::NAN,
                    },
                ),
                "angle",
            ),
            (
                shaped(
                    5.0,
                    ConveyorShape::Spiral {
                        radius: -1.0,
                        rise: 1.5,
                        turns: 2.0,
                    },
                ),
                "radius",
            ),
            (
                shaped(
                    5.0,
                    ConveyorShape::Spiral {
                        radius: 1.2,
                        rise: 0.0,
                        turns: 2.0,
                    },
                ),
                "rise",
            ),
            (
                shaped(
                    5.0,
                    ConveyorShape::Spiral {
                        radius: 1.2,
                        rise: 1.5,
                        turns: f64::INFINITY,
                    },
                ),
                "turns",
            ),
        ] {
            m.elements[1].kind = kind;
            let p = problems(&m);
            assert!(
                p.iter().any(|(s, at, x)| *s == Severity::Error
                    && at.as_deref() == Some("machine")
                    && x == &format!("BadConveyorValue {{ field: {field:?} }}")),
                "{field}: {p:?}"
            );
        }
        m.elements[1].kind = belt(10.0, 5.0, 0.8, 0.0);
        assert!(
            m.validate().iter().all(|d| d.severity != Severity::Error),
            "a gap of 0 is allowed"
        );
        // Shorter than one pitch: nothing fits.
        m.elements[1].kind = belt(1.0, 5.0, 0.8, 0.5);
        assert!(
            problems(&m)
                .iter()
                .any(|(s, _, x)| *s == Severity::Error && x.starts_with("ConveyorTooShort")),
            "{:?}",
            problems(&m)
        );
        // Exactly one output, as for a server.
        m.elements[1].kind = belt(10.0, 5.0, 0.8, 0.2);
        let out = m.connections[1].clone();
        let mut second = out.clone();
        second.id = ElementId::unchecked("out2");
        m.connections.push(second);
        assert!(
            problems(&m)
                .iter()
                .any(|(_, at, x)| at.as_deref() == Some("machine")
                    && x.starts_with("SeveralOutputs")),
            "{:?}",
            problems(&m)
        );
    }

    #[test]
    fn conveyor_shape_path_length_matches_geometry() {
        assert!((ConveyorShape::Straight.path_length(7.5) - 7.5).abs() < 1e-12);
        let curve = ConveyorShape::Curve {
            radius: 2.0,
            angle: 90.0,
        };
        assert!((curve.path_length(10.0) - std::f64::consts::PI).abs() < 1e-12);
        let right_curve = ConveyorShape::Curve {
            radius: 2.0,
            angle: -90.0,
        };
        assert!((right_curve.path_length(10.0) - std::f64::consts::PI).abs() < 1e-12);
        let spiral = ConveyorShape::Spiral {
            radius: 1.2,
            rise: 1.5,
            turns: 2.0,
        };
        let circumference = 2.4 * std::f64::consts::PI;
        let expected_spiral = 2.0 * (circumference * circumference + 1.5 * 1.5).sqrt();
        assert!((spiral.path_length(10.0) - expected_spiral).abs() < 1e-12);
    }

    #[test]
    fn conveyor_shapes_as_paths_have_the_shapes_path_length() {
        let spiral = ConveyorShape::Spiral {
            radius: 1.2,
            rise: 1.5,
            turns: 2.0,
        };
        let curve = ConveyorShape::Curve {
            radius: 2.0,
            angle: -90.0,
        };
        for shape in [ConveyorShape::Straight, curve, spiral] {
            let path = shape.path(7.5);
            assert_eq!(path.segments.len(), 1);
            assert!((path.length() - shape.path_length(7.5)).abs() < 1e-12);
        }
        // A spiral of 1.5 m a turn for two turns climbs 3 m.
        assert!((spiral.path(0.0).end().z - 3.0).abs() < 1e-12);
        // A right curve ends to the right.
        let end = curve.path(0.0).end();
        assert!((end.x - 2.0).abs() < 1e-12 && (end.y + 2.0).abs() < 1e-12);
    }

    #[test]
    fn conveyor_whose_shape_path_differs_from_length_warns() {
        let mut m = mm1();
        let shape = ConveyorShape::Curve {
            radius: 2.0,
            angle: 90.0,
        };
        let path_length = shape.path_length(0.0);
        // Matching path length (and within 1%): no mismatch warning.
        m.elements[1].kind = ElementKind::Conveyor {
            length: path_length,
            speed: 1.0,
            accumulating: true,
            item_length: 0.5,
            gap: 0.0,
            shape,
        };
        assert!(
            !problems(&m)
                .iter()
                .any(|(_, _, x)| x.starts_with("ConveyorLengthMismatch")),
            "{:?}",
            problems(&m)
        );
        // 5% longer than drawn path: warns, naming both lengths.
        let length = path_length * 1.05;
        m.elements[1].kind = ElementKind::Conveyor {
            length,
            speed: 1.0,
            accumulating: true,
            item_length: 0.5,
            gap: 0.0,
            shape,
        };
        let ds = m.validate();
        let warn = ds
            .iter()
            .find(|d| d.problem.code() == "conveyor_length_mismatch");
        assert!(
            warn.is_some_and(|d| d.severity == Severity::Warning),
            "{ds:?}"
        );
        if let Some(d) = warn {
            let msg = d.to_string();
            assert!(
                msg.contains(&short(path_length)) && msg.contains(&short(length)),
                "{msg}"
            );
        }
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
            text.contains(
                &"error: machine has nowhere to send entities. Add a flow from it to the next block."
                    .to_string()
            ),
            "{text:#?}"
        );
    }

    #[test]
    fn an_observation_of_a_block_the_model_lacks_is_a_warning_not_an_error() {
        let found = |element: &str| {
            let mut m = mm1();
            m.experiment.observed = vec![Observation {
                element: ElementId::unchecked(element),
                metric: "mean_wait".into(),
                target: Observed::AtMost(2.0),
            }];
            m.validate()
                .into_iter()
                .filter(|d| d.problem.code() == "bad_observation")
                .map(|d| (d.severity, d.at.map(|a| a.to_string())))
                .collect::<Vec<_>>()
        };
        // A block the model has is fine.
        assert!(found("machine").is_empty());
        // One it no longer has is reported at that id, and does not stop a run.
        assert_eq!(
            found("removed"),
            vec![(Severity::Warning, Some("removed".to_owned()))]
        );
    }

    #[test]
    fn an_observation_of_a_pool_is_fine() {
        let mut m = mm1();
        m.resources.push(ResourcePool::new(
            ElementId::unchecked("crew"),
            "Crew",
            2,
            Vec::new(),
        ));
        m.experiment.observed = vec![Observation {
            element: ElementId::unchecked("crew"),
            metric: "utilisation".into(),
            target: Observed::Value {
                value: 0.8,
                half_width: Some(0.05),
                tolerance: None,
            },
        }];
        assert!(
            !m.validate()
                .iter()
                .any(|d| d.problem.code() == "bad_observation")
        );
        assert_eq!(m.experiment.observed[0].target.value(), 0.8);
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

    #[test]
    fn starting_contents_must_fit_a_block_that_holds_them() {
        let codes = |m: &Model| -> Vec<(Option<String>, String)> {
            m.validate()
                .into_iter()
                .map(|d| (d.at.map(|a| a.to_string()), d.problem.code().to_owned()))
                .collect()
        };
        let mut m = mm1();
        // Any number inside a task without a waiting-room limit; 0 is the same as none.
        m.starting.insert(ElementId::unchecked("machine"), 40);
        m.starting.insert(ElementId::unchecked("exit"), 0);
        assert_eq!(codes(&m), vec![]);
        // Not in a block that cannot hold entities, nor one that does not exist.
        m.starting.insert(ElementId::unchecked("exit"), 2);
        m.starting.insert(ElementId::unchecked("ghost"), 1);
        assert_eq!(
            codes(&m),
            vec![
                (Some("exit".into()), "bad_starting".into()),
                (Some("ghost".into()), "bad_starting".into()),
            ]
        );
        m.starting.remove(&ElementId::unchecked("exit"));
        m.starting.remove(&ElementId::unchecked("ghost"));
        // A waiting room of 3 behind 1 unit holds 4.
        if let Some(e) = m.elements.iter_mut().find(|e| e.id.as_str() == "machine")
            && let ElementKind::Server { queue_capacity, .. } = &mut e.kind
        {
            *queue_capacity = Some(3);
        }
        let too_many = m
            .validate()
            .into_iter()
            .find(|d| d.problem.code() == "bad_starting")
            .map(|d| d.problem);
        assert_eq!(
            too_many,
            Some(Problem::BadStarting {
                reason: StartingFault::TooMany { count: 40, most: 4 }
            })
        );
        m.starting.insert(ElementId::unchecked("machine"), 4);
        assert_eq!(codes(&m), vec![]);
    }

    #[test]
    fn a_block_with_starting_contents_is_where_entities_come_from() {
        // No source: entities inside the task at the start still reach the exit, so the model
        // has somewhere entities come from and every block is reached.
        let mut m = mm1();
        m.elements.retain(|e| e.id.as_str() != "arrivals");
        m.connections.retain(|c| c.id.as_str() != "f1");
        let codes = |m: &Model| -> Vec<String> {
            m.validate()
                .into_iter()
                .map(|d| d.problem.code().to_owned())
                .collect()
        };
        assert!(
            codes(&m).contains(&"no_source".to_owned()),
            "{:?}",
            codes(&m)
        );
        m.starting.insert(ElementId::unchecked("machine"), 3);
        assert_eq!(codes(&m), Vec::<String>::new());
    }
}
