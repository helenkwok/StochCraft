//! The capacity table: each station's arrival rate, service time, units and utilisation worked
//! out from the model's own numbers, before any run, and its mean wait where a queueing formula
//! holds exactly (the traffic equations of an open network; Gross, Shortle, Thompson and Harris,
//! *Fundamentals of Queueing Theory*, 4th ed., §§2.3, 2.5, 4.1, 5.1).
//!
//! Rates are passed on through the routing as the traffic equations pass them: a gateway drawing
//! by probability splits its rate, a parallel gateway copies it to every way and a join passes
//! on the least of its incoming rates (each over its quantity), a batch divides it by its size
//! and an unbatch multiplies it back, and a rework loop is solved as a whole. Where a rate
//! depends on what happens in the run (a condition on a property, the shortest queue, entities
//! giving up or turned away), it is not guessed: the table says what it depends on. What
//! follows entities lost, or a station sent more than it certainly carries (its capacity is not
//! known exactly, or its pool may be overloaded), is at most its rate.
//!
//! Utilisation is defined as in the results (busy is processing and setting up, failed time
//! not busy): the arrival rate times the mean service time over the units. The load, which
//! marks a station fine, congested or unstable, is that over the units' availability, since
//! failed time is capacity lost.
//!
//! A formula is given only where it is exact: M/M/c (Erlang C) where every station feeding this
//! one, through any rework loop, is exponential with an unlimited queue and routing by
//! probability (a Jackson network); with a finite queue, patience until service starts or
//! balking (M/M/c/K, Erlang A), and M/G/1, only where the arrivals are Poisson, so with no loop
//! and nothing before the station that loses entities, blocks or batches them.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{
    BalkJoin, Connection, Dist, Element, ElementId, ElementKind, Model, Need, PatienceUntil,
    Routing, never_negative,
};
use stochcraft_random::math::{exp, ln};

/// The default utilisation above which a station is congested.
pub const DEFAULT_TARGET: f64 = 0.85;

/// Whether a number is the value itself or a bound on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Exact,
    /// The value is at most this (entities lost before here, or an overloaded station before).
    AtMost,
    /// The value is at least this (the most units on shift, a pool's tasks with alternatives).
    AtLeast,
}

impl Bound {
    /// The name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Bound::Exact => "exact",
            Bound::AtMost => "at_most",
            Bound::AtLeast => "at_least",
        }
    }

    /// The bound of a product or quotient: `other` flipped when it divides.
    fn with(self, other: Bound, divides: bool) -> Option<Bound> {
        let other = match (other, divides) {
            (Bound::AtMost, true) => Bound::AtLeast,
            (Bound::AtLeast, true) => Bound::AtMost,
            (b, _) => b,
        };
        match (self, other) {
            (Bound::Exact, b) | (b, Bound::Exact) => Some(b),
            (a, b) if a == b => Some(a),
            _ => None,
        }
    }
}

/// A number in the table, or why it cannot be worked out before a run.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Known { value: f64, bound: Bound },
    Unknown(Why),
}

impl Value {
    fn exact(value: f64) -> Self {
        Value::Known {
            value,
            bound: Bound::Exact,
        }
    }

    /// The number, whatever its bound.
    pub fn value(&self) -> Option<f64> {
        match self {
            Value::Known { value, .. } => Some(*value),
            Value::Unknown(_) => None,
        }
    }

    /// The number if it is exact.
    pub fn exact_value(&self) -> Option<f64> {
        match self {
            Value::Known {
                value,
                bound: Bound::Exact,
            } => Some(*value),
            _ => None,
        }
    }

    /// `self` × `other` (or ÷ with `divides`), with the bound that follows, if any does.
    fn combine(&self, other: &Value, divides: bool) -> Value {
        match (self, other) {
            (Value::Unknown(w), _) | (_, Value::Unknown(w)) => Value::Unknown(w.clone()),
            (Value::Known { value: a, bound: x }, Value::Known { value: b, bound: y }) => {
                match x.with(*y, divides) {
                    Some(bound) => Value::Known {
                        value: if divides { a / b } else { a * b },
                        bound,
                    },
                    None => Value::Unknown(Why::Bounds),
                }
            }
        }
    }
}

/// Why a number, or a formula, is not given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// The way taken at this gateway depends on a condition on the entity.
    ByCondition(ElementId),
    /// This gateway sends each entity to the shortest queue.
    ShortestQueue(ElementId),
    /// Entities reach it by a way taken by those giving up, balking or turned away at this
    /// server, which depends on how congested it is.
    MarkedWay(ElementId),
    /// The batches this unbatch splits up cannot be traced to one batch size.
    UnbatchSize(ElementId),
    /// This source's time between arrivals has no mean, or can be negative.
    NoArrivalRate(ElementId),
    /// This element is on a loop that sends every entity back: they never leave.
    NoWayOut(ElementId),
    /// The times depend on a property of the entity, whose mix is not known before a run.
    ByProperty,
    /// The setups depend on the order of the entities' types.
    Changeover,
    /// The time can be negative (cut to 0 in a run), or has no mean.
    NoMean,
    /// The task's needs have alternatives, or roles that several pools have: which pool serves
    /// depends on the run.
    Alternatives,
    /// A bound that rises and one that falls make the result unknown.
    Bounds,
    /// At 100% or more there is no steady state: the queue grows without limit.
    Unstable,
    /// Several units with times that are not exponential: no exact formula.
    SeveralUnitsGeneral,
    /// The service time's variance is not known (a user distribution).
    NoVariance,
    /// Arrivals come from this source, whose times between arrivals are not exponential.
    NotPoisson(ElementId),
    /// Arrivals come from this source, whose rate follows a calendar or ends at a limit.
    SourceVaries(ElementId),
    /// The station is on, or after, a loop, so its arrivals are not Poisson.
    Loop,
    /// Arrivals come through this element, after which they are not Poisson (entities lost,
    /// blocked, batched or joined there, or times there not exponential).
    After(ElementId),
    /// The units follow a calendar.
    Calendar,
    /// The units fail.
    Breakdowns,
    /// Units are taken out for restarts.
    HealthCheck,
    /// Entities take time to get here.
    Transfer,
    /// Setups before jobs.
    Setups,
    /// Entities give up during service too.
    PatienceUntilDone,
    /// The patience is not exponential.
    PatienceNotExponential,
    /// The waiting room's size follows a calendar or the working units.
    QueueLimitVaries,
    /// A full waiting room makes the element sending here wait (blocking), so the senders and
    /// this station depend on each other.
    BlocksSender,
    /// The pool is shared with other tasks, or the task needs more than one unit.
    SharedPool,
    /// The service time is not exponential, with a finite queue, patience or balking.
    NotExponential,
    /// This element can hold up the station's units (blocking): a full waiting room, or a
    /// transfer into it that keeps the sending unit.
    HeldUp(ElementId),
    /// A conveyor has no queueing formula: its time on it is its length over its speed, and its
    /// waiting is before it, at what sends to it.
    ConveyorTime,
}

impl Why {
    /// The code on the wire.
    pub fn code(&self) -> &'static str {
        match self {
            Why::ByCondition(_) => "by_condition",
            Why::ShortestQueue(_) => "shortest_queue",
            Why::MarkedWay(_) => "marked_way",
            Why::UnbatchSize(_) => "unbatch_size",
            Why::NoArrivalRate(_) => "no_arrival_rate",
            Why::NoWayOut(_) => "no_way_out",
            Why::ByProperty => "by_property",
            Why::Changeover => "changeover",
            Why::NoMean => "no_mean",
            Why::Alternatives => "alternatives",
            Why::Bounds => "bounds",
            Why::Unstable => "unstable",
            Why::SeveralUnitsGeneral => "several_units_general",
            Why::NoVariance => "no_variance",
            Why::NotPoisson(_) => "not_poisson",
            Why::SourceVaries(_) => "source_varies",
            Why::Loop => "loop",
            Why::After(_) => "after",
            Why::Calendar => "calendar",
            Why::Breakdowns => "breakdowns",
            Why::HealthCheck => "health_check",
            Why::Transfer => "transfer",
            Why::Setups => "setups",
            Why::PatienceUntilDone => "patience_until_done",
            Why::PatienceNotExponential => "patience_not_exponential",
            Why::QueueLimitVaries => "queue_limit_varies",
            Why::BlocksSender => "blocks_sender",
            Why::SharedPool => "shared_pool",
            Why::NotExponential => "not_exponential",
            Why::HeldUp(_) => "held_up",
            Why::ConveyorTime => "conveyor_time",
        }
    }

    /// The element it names, if any.
    pub fn at(&self) -> Option<&ElementId> {
        match self {
            Why::ByCondition(id)
            | Why::ShortestQueue(id)
            | Why::MarkedWay(id)
            | Why::UnbatchSize(id)
            | Why::NoArrivalRate(id)
            | Why::NoWayOut(id)
            | Why::NotPoisson(id)
            | Why::SourceVaries(id)
            | Why::After(id)
            | Why::HeldUp(id) => Some(id),
            _ => None,
        }
    }

    /// In words, with `name` giving an element's name.
    pub fn message(&self, name: impl Fn(&ElementId) -> String) -> String {
        match self {
            Why::ByCondition(id) => format!("depends on the conditions at {}", name(id)),
            Why::ShortestQueue(id) => format!("depends on the queues {} chooses between", name(id)),
            Why::MarkedWay(id) => format!(
                "depends on how many give up, balk or are turned away at {}",
                name(id)
            ),
            Why::UnbatchSize(id) => format!("the batch size {} splits is not known", name(id)),
            Why::NoArrivalRate(id) => format!(
                "the time between arrivals at {} has no mean, or can be negative",
                name(id)
            ),
            Why::NoWayOut(id) => format!("{} is on a loop that sends every entity back", name(id)),
            Why::ByProperty => "the times depend on a property whose mix is not known".into(),
            Why::Changeover => "the setups depend on the order of the types".into(),
            Why::NoMean => "the time has no mean, or can be negative".into(),
            Why::Alternatives => "which pool serves depends on the run".into(),
            Why::Bounds => {
                "arrivals are at most a rate and the units vary by shift: no bound follows".into()
            }
            Why::HeldUp(id) => format!(
                "the units can be held up by {} (a full waiting room, or a transfer that keeps \
                 them)",
                name(id)
            ),
            Why::Unstable => "at 100% or more the queue grows without limit".into(),
            Why::SeveralUnitsGeneral => {
                "no exact formula for several units with times that are not exponential".into()
            }
            Why::NoVariance => "the service time's variance is not known".into(),
            Why::NotPoisson(id) => format!(
                "the times between arrivals at {} are not exponential",
                name(id)
            ),
            Why::SourceVaries(id) => format!(
                "the arrivals at {} follow a calendar or stop at a limit",
                name(id)
            ),
            Why::Loop => "entities come back on a loop, so arrivals are not Poisson".into(),
            Why::After(id) => format!("arrivals come through {}, so are not Poisson", name(id)),
            Why::Calendar => "the units follow a calendar".into(),
            Why::Breakdowns => "the units fail".into(),
            Why::HealthCheck => "the units are taken out for restarts".into(),
            Why::Transfer => "entities take time to get here".into(),
            Why::Setups => "setups come before jobs".into(),
            Why::PatienceUntilDone => "entities give up during service too".into(),
            Why::PatienceNotExponential => "the patience is not exponential".into(),
            Why::QueueLimitVaries => "the waiting room's size varies".into(),
            Why::BlocksSender => {
                "a full waiting room holds up the element sending here (blocking)".into()
            }
            Why::SharedPool => {
                "the pool is shared with other tasks, or more than one unit is needed".into()
            }
            Why::ConveyorTime => "a conveyor's time is its length over its speed".into(),
            Why::NotExponential => {
                "the service time is not exponential, with a finite queue, patience or balking"
                    .into()
            }
        }
    }
}

/// What a row of the table is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StationKind {
    Server,
    SharedTask,
    Delay,
    Pool,
    /// A conveyor: it carries at most speed / pitch entities per unit of time.
    Conveyor,
}

impl StationKind {
    /// The name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            StationKind::Server => "server",
            StationKind::SharedTask => "shared_task",
            StationKind::Delay => "delay",
            StationKind::Pool => "pool",
            StationKind::Conveyor => "conveyor",
        }
    }
}

/// How busy a station is against the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Below the target.
    Fine,
    /// At or above the target, below 100%.
    Congested,
    /// At 100% or more.
    Unstable,
    /// Not known before a run.
    Unknown,
}

impl Status {
    /// The name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Status::Fine => "fine",
            Status::Congested => "congested",
            Status::Unstable => "unstable",
            Status::Unknown => "unknown",
        }
    }
}

/// A queueing formula's results at a station, in the model's time unit, each defined as the
/// result of the same name is, so the two can be set side by side.
#[derive(Debug, Clone, PartialEq)]
pub struct Formula {
    /// The model in Kendall's notation (`M/M/2`, `M/M/1/5+M`, `M/G/1`, `M/G/∞`).
    pub name: String,
    /// The mean time from arrival to the start of service, of the entities served
    /// (`mean_wait`).
    pub mean_wait: f64,
    /// The time-average number waiting (`mean_queue_length`).
    pub mean_queue: f64,
    /// The chance an entity that joins has to wait.
    pub wait_probability: Option<f64>,
    /// With a finite waiting room: the share of attempts to get in turned away
    /// (`loss_probability`).
    pub lost_share: Option<f64>,
    /// With balking: the share of arrivals that balk (`balked_share`).
    pub balked_share: Option<f64>,
    /// With patience: the share of those joining that give up (`abandoned_share`).
    pub abandoned_share: Option<f64>,
    /// The share of arrivals served.
    pub served_share: f64,
    /// The time-average number inside, waiting or served.
    pub mean_inside: f64,
}

/// One row of the table.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    /// The element, or the pool.
    pub id: ElementId,
    pub kind: StationKind,
    /// Entities arriving per time unit (for a pool: none).
    pub arrival_rate: Value,
    /// The mean time an entity holds a unit, setups before every job included (for a pool:
    /// none).
    pub mean_service: Value,
    /// The units (`None`: a delay, as many as needed).
    pub units: Option<u32>,
    /// The units follow a calendar: `units` is the most on shift.
    pub units_vary: bool,
    /// The share of time a unit is not failed (`None`: units never fail). At least the mean
    /// time to failure over that plus the mean repair when failures wait for the job to end,
    /// which keeps a unit up longer.
    pub availability: Option<Value>,
    /// Arrival rate × mean service ÷ units, as utilisation is in the results (for a delay: the
    /// mean number inside).
    pub utilisation: Value,
    /// Utilisation over the availability: what marks the station.
    pub load: Value,
    pub status: Status,
    /// The fewest units that bring the load under the target, if more are needed.
    pub units_for_target: Option<u32>,
    pub formula: Result<Formula, Why>,
    /// A conveyor's mean time on it, its length over its speed: exact unless something after
    /// it can hold it up (see `held_up_by`), then at least. `None` for any other row.
    pub time_on: Option<Value>,
}

/// The capacity table.
#[derive(Debug, Clone, PartialEq)]
pub struct Capacity {
    /// The utilisation above which a station is congested.
    pub target: f64,
    pub stations: Vec<Station>,
    /// The station with the highest known load, if any is known.
    pub bottleneck: Option<ElementId>,
}

impl Model {
    /// The capacity table, with `target` the utilisation above which a station is congested
    /// ([`DEFAULT_TARGET`] by default).
    pub fn capacity(&self, target: f64) -> Capacity {
        Net::new(self).table(target)
    }
}

/// The model as a graph, with each connection's rate.
struct Net<'a> {
    model: &'a Model,
    index: BTreeMap<&'a ElementId, usize>,
    /// Connection indices into and out of each element.
    ins: Vec<Vec<usize>>,
    outs: Vec<Vec<usize>>,
    /// Whether each element is on a loop.
    looped: Vec<bool>,
}

/// The one pool a shared task uses and how many units, or why it has no one pool.
type PoolUse = Result<(usize, u32), Why>;

/// A connection's rate, before bounds.
#[derive(Debug, Clone)]
enum Flow {
    Known(f64),
    Unknown(Why),
}

fn marked(c: &Connection) -> bool {
    c.abandoned || c.turned_away || c.balked
}

/// The mean of a time that cannot be negative.
fn mean_of(d: &Dist) -> Option<f64> {
    never_negative(d)
        .then(|| d.mean())
        .flatten()
        .filter(|m| m.is_finite() && *m >= 0.0)
}

fn exponential(d: &Dist) -> bool {
    matches!(d, Dist::Exponential { mean } if *mean > 0.0)
}

impl<'a> Net<'a> {
    fn new(model: &'a Model) -> Self {
        let index: BTreeMap<_, _> = model
            .elements
            .iter()
            .enumerate()
            .map(|(i, e)| (&e.id, i))
            .collect();
        let n = model.elements.len();
        let (mut ins, mut outs) = (vec![Vec::new(); n], vec![Vec::new(); n]);
        for (k, c) in model.connections.iter().enumerate() {
            if let (Some(&f), Some(&t)) = (index.get(&c.from), index.get(&c.to)) {
                outs[f].push(k);
                ins[t].push(k);
            }
        }
        let mut net = Self {
            model,
            index,
            ins,
            outs,
            looped: Vec::new(),
        };
        net.looped = (0..n).map(|i| net.before(i).contains(&i)).collect();
        net
    }

    fn element(&self, i: usize) -> &'a Element {
        &self.model.elements[i]
    }

    fn end_of(&self, k: usize) -> Option<usize> {
        self.index.get(&self.model.connections[k].to).copied()
    }

    fn start_of(&self, k: usize) -> Option<usize> {
        self.index.get(&self.model.connections[k].from).copied()
    }

    /// The external arrival rate of a source.
    fn source_rate(e: &Element) -> Option<Result<f64, Why>> {
        let ElementKind::Source {
            interarrival,
            timed,
            idle_outside,
            ..
        } = &e.kind
        else {
            return None;
        };
        // With calendars, the highest rate of any period: an upper bound.
        let means = (!*idle_outside)
            .then_some(interarrival)
            .into_iter()
            .chain(timed.iter().map(|t| &t.value))
            .map(mean_of)
            .collect::<Option<Vec<f64>>>();
        Some(
            means
                .and_then(|m| m.into_iter().reduce(f64::min))
                .filter(|m| *m > 0.0)
                .map(|m| 1.0 / m)
                .ok_or_else(|| Why::NoArrivalRate(e.id.clone())),
        )
    }

    /// Whether this element's rate onwards is only an upper bound on what goes in.
    fn bounds_onward(&self, i: usize) -> bool {
        let e = self.element(i);
        match &e.kind {
            ElementKind::Source {
                limit,
                timed,
                idle_outside,
                ..
            } => limit.is_some() || !timed.is_empty() || *idle_outside,
            ElementKind::Server {
                queue_capacity,
                patience,
                balking,
                ..
            } => {
                // Entities lost here (arrivals from a source into a full room, or turned away
                // by a way of their own, those giving up or balking) leave by a marked way or
                // the model, not the ordinary way.
                let fed_by_source = self.ins[i].iter().any(|&k| {
                    self.start_of(k)
                        .is_some_and(|f| matches!(self.element(f).kind, ElementKind::Source { .. }))
                });
                let turns_away = self.outs[i]
                    .iter()
                    .any(|&k| self.model.connections[k].turned_away);
                (queue_capacity.is_some() && (fed_by_source || turns_away))
                    || patience.is_some()
                    || balking.is_some()
            }
            _ => false,
        }
    }

    /// The batch size an unbatch splits up: 1 if no batch is before it, the size if every batch
    /// before it has the same one and every way into it comes through one.
    fn unbatch_size(&self, i: usize) -> Option<u32> {
        let mut seen = BTreeSet::from([i]);
        let mut todo: VecDeque<usize> = self.ins[i]
            .iter()
            .filter_map(|&k| self.start_of(k))
            .collect();
        let (mut sizes, mut bare) = (BTreeSet::new(), false);
        while let Some(j) = todo.pop_front() {
            if !seen.insert(j) {
                continue;
            }
            match self.element(j).kind {
                ElementKind::Batch { size } => {
                    sizes.insert(size);
                }
                ElementKind::Source { .. } => bare = true,
                ElementKind::Unbatch => bare = true,
                _ => todo.extend(self.ins[j].iter().filter_map(|&k| self.start_of(k))),
            }
        }
        match (sizes.len(), bare) {
            (0, _) => Some(1),
            (1, false) => sizes.into_iter().next(),
            _ => None,
        }
    }

    /// Each connection's rate: the traffic equations, solved by passing rates on until they
    /// settle (they rise to the least solution; a loop sending everything back never settles).
    /// `caps[i]`: the most station `i` can pass on; `sure[i]`: what it certainly passes on, so
    /// that more coming in makes what follows at most.
    fn flows(&self, caps: &[Option<f64>], sure: &[Option<f64>]) -> (Vec<Flow>, Vec<bool>) {
        let (n, m) = (self.model.elements.len(), self.model.connections.len());
        let mut flow = vec![0.0_f64; m];
        // Connections whose rate is not known: fixed before the rates are passed on.
        let mut unknown: Vec<Option<Why>> = vec![None; m];
        let mut unbatch = vec![1.0; n];
        for (i, size) in unbatch.iter_mut().enumerate() {
            let e = self.element(i);
            let why = match &e.kind {
                ElementKind::ExclusiveGateway {
                    routing: Routing::ShortestQueue,
                    ..
                } => Some(Why::ShortestQueue(e.id.clone())),
                ElementKind::ExclusiveGateway { .. }
                    if self.outs[i]
                        .iter()
                        .any(|&k| self.model.connections[k].condition.is_some()) =>
                {
                    Some(Why::ByCondition(e.id.clone()))
                }
                ElementKind::Unbatch => match self.unbatch_size(i) {
                    Some(s) => {
                        *size = f64::from(s);
                        None
                    }
                    None => Some(Why::UnbatchSize(e.id.clone())),
                },
                ElementKind::Source { .. } => Self::source_rate(e).and_then(Result::err),
                _ => None,
            };
            for &k in &self.outs[i] {
                if marked(&self.model.connections[k]) {
                    unknown[k] = Some(Why::MarkedWay(e.id.clone()));
                } else if let Some(w) = &why {
                    unknown[k] = Some(w.clone());
                }
            }
        }
        let input = |flow: &[f64], i: usize| -> f64 {
            self.ins[i]
                .iter()
                .filter(|&&k| unknown[k].is_none())
                .map(|&k| flow[k])
                .sum()
        };
        let total: f64 = (0..n)
            .filter_map(|i| Self::source_rate(self.element(i)).and_then(Result::ok))
            .sum();
        let mut settled = false;
        let mut changed = vec![false; n];
        for _ in 0..200_000 {
            let mut most = 0.0_f64;
            for i in 0..n {
                let e = self.element(i);
                let through = match &e.kind {
                    ElementKind::Source { .. } => {
                        Self::source_rate(e).and_then(Result::ok).unwrap_or(0.0)
                    }
                    ElementKind::ParallelGateway if self.ins[i].len() > 1 => self.ins[i]
                        .iter()
                        .map(|&k| {
                            let q = self.model.connections[k].quantity.unwrap_or(1).max(1);
                            flow[k] / f64::from(q)
                        })
                        .fold(f64::INFINITY, f64::min),
                    ElementKind::Batch { size } => input(&flow, i) / f64::from((*size).max(1)),
                    ElementKind::Unbatch => input(&flow, i) * unbatch[i],
                    _ => input(&flow, i).min(caps[i].unwrap_or(f64::INFINITY)),
                };
                let ways: Vec<usize> = self.outs[i]
                    .iter()
                    .copied()
                    .filter(|&k| unknown[k].is_none())
                    .collect();
                let shares = self.shares(i, &ways);
                let mut moved = false;
                for (&k, share) in ways.iter().zip(shares) {
                    let new = through * share;
                    let d = (new - flow[k]).abs();
                    if d > 1e-13 * (1.0 + new.abs()) {
                        moved = true;
                    }
                    most = most.max(d / (1.0 + new.abs()));
                    flow[k] = new;
                }
                changed[i] = moved;
            }
            if most <= 1e-13 {
                settled = true;
                break;
            }
            if flow.iter().any(|f| *f > 1e12 * (1.0 + total)) {
                break;
            }
        }
        // Connections still moving are on a loop with no way out.
        let mut out: Vec<Flow> = unknown
            .iter()
            .zip(&flow)
            .map(|(w, &f)| w.clone().map_or(Flow::Known(f), Flow::Unknown))
            .collect();
        if !settled {
            for i in (0..n).filter(|&i| changed[i]) {
                for &k in &self.outs[i] {
                    if matches!(out[k], Flow::Known(_)) {
                        out[k] = Flow::Unknown(Why::NoWayOut(self.element(i).id.clone()));
                    }
                }
            }
        }
        // An unknown rate makes everything it reaches unknown.
        let mut queue: VecDeque<(usize, Why)> = out
            .iter()
            .enumerate()
            .filter_map(|(k, f)| match f {
                Flow::Unknown(w) => self.end_of(k).map(|t| (t, w.clone())),
                Flow::Known(_) => None,
            })
            .collect();
        let mut reached = vec![false; n];
        while let Some((i, why)) = queue.pop_front() {
            if reached[i] {
                continue;
            }
            reached[i] = true;
            for &k in &self.outs[i] {
                if matches!(out[k], Flow::Known(_)) {
                    out[k] = Flow::Unknown(why.clone());
                }
                if let Some(t) = self.end_of(k) {
                    queue.push_back((t, why.clone()));
                }
            }
        }
        // Bounds: what comes after a loss, or a station that may not pass on all it gets, is at
        // most its rate. A station is not bounded by itself through a loop for its capacity
        // alone: its arrivals are those it is offered, as its load is judged on them.
        let mut at_most = vec![false; n];
        for (j, certain) in sure.iter().enumerate() {
            let loses = self.bounds_onward(j);
            let short = certain.is_some_and(|c| input(&flow, j) > c * (1.0 - 1e-12));
            if !loses && !short {
                continue;
            }
            let mut seen = vec![false; n];
            let mut queue: VecDeque<usize> = self.outs[j]
                .iter()
                .filter_map(|&k| self.end_of(k))
                .collect();
            while let Some(i) = queue.pop_front() {
                if !seen[i] {
                    seen[i] = true;
                    at_most[i] |= i != j || loses;
                    queue.extend(self.outs[i].iter().filter_map(|&k| self.end_of(k)));
                }
            }
        }
        (out, at_most)
    }

    /// The shares of what passes through element `i` that each of `ways` takes.
    fn shares(&self, i: usize, ways: &[usize]) -> Vec<f64> {
        let e = self.element(i);
        match &e.kind {
            ElementKind::ExclusiveGateway {
                routing: Routing::Standard,
                ..
            } => {
                let given: f64 = ways
                    .iter()
                    .filter_map(|&k| self.model.connections[k].probability)
                    .sum();
                let without = ways
                    .iter()
                    .filter(|&&k| self.model.connections[k].probability.is_none())
                    .count();
                let rest = if without == 0 {
                    0.0
                } else {
                    (1.0 - given).max(0.0) / without as f64
                };
                ways.iter()
                    .map(|&k| self.model.connections[k].probability.unwrap_or(rest))
                    .collect()
            }
            // Every way gets each entity: a parallel gateway copies; any other element has one
            // ordinary way.
            _ => vec![1.0; ways.len()],
        }
    }

    /// Each element's arrival rate.
    fn arrivals(&self, flows: &[Flow], at_most: &[bool], i: usize) -> Value {
        let e = self.element(i);
        if let Some(r) = Self::source_rate(e) {
            // A source with a limit stops; one with calendars has its highest rate only at
            // times: either way, at most this rate in the long run.
            return match r {
                Ok(value) => Value::Known {
                    value,
                    bound: if self.bounds_onward(i) {
                        Bound::AtMost
                    } else {
                        Bound::Exact
                    },
                },
                Err(w) => Value::Unknown(w),
            };
        }
        let mut sum = 0.0;
        for &k in &self.ins[i] {
            match &flows[k] {
                Flow::Known(f) => sum += f,
                Flow::Unknown(w) => return Value::Unknown(w.clone()),
            }
        }
        Value::Known {
            value: sum,
            bound: if at_most[i] {
                Bound::AtMost
            } else {
                Bound::Exact
            },
        }
    }

    /// The elements that can reach `i`, not counting `i` unless it is on a loop.
    /// Nearest first.
    fn before(&self, i: usize) -> Vec<usize> {
        let mut seen = BTreeSet::new();
        let mut order = Vec::new();
        let mut todo: VecDeque<usize> = self.ins[i]
            .iter()
            .filter_map(|&k| self.start_of(k))
            .collect();
        while let Some(j) = todo.pop_front() {
            if seen.insert(j) {
                order.push(j);
                todo.extend(self.ins[j].iter().filter_map(|&k| self.start_of(k)));
            }
        }
        order
    }

    /// Why entities leaving element `i` would not keep a Jackson network's product form, if so.
    fn breaks_product_form(&self, i: usize, unstable: &[bool]) -> Option<Why> {
        let e = self.element(i);
        match &e.kind {
            ElementKind::Source {
                interarrival,
                limit,
                timed,
                idle_outside,
            } => {
                if limit.is_some() || !timed.is_empty() || *idle_outside {
                    Some(Why::SourceVaries(e.id.clone()))
                } else if !exponential(interarrival) {
                    Some(Why::NotPoisson(e.id.clone()))
                } else {
                    None
                }
            }
            ElementKind::ExclusiveGateway {
                routing: Routing::Standard,
                ..
            }
            | ElementKind::Delay { .. }
            | ElementKind::Sink { .. } => None,
            ElementKind::Server { .. }
                if plain_exponential(&e.kind) && !unstable[i] && self.held_up_by(i).is_none() =>
            {
                None
            }
            _ => Some(Why::After(e.id.clone())),
        }
    }

    /// Why a station's arrivals are not those of a Jackson network (with `poisson`, not
    /// Poisson), if so.
    fn arrivals_not(&self, i: usize, unstable: &[bool], poisson: bool) -> Option<Why> {
        let before = self.before(i);
        if poisson && (self.looped[i] || before.iter().any(|&j| self.looped[j])) {
            return Some(Why::Loop);
        }
        before
            .iter()
            .filter(|&&j| j != i)
            .find_map(|&j| self.breaks_product_form(j, unstable))
    }

    fn table(&self, target: f64) -> Capacity {
        let model = self.model;
        let n = model.elements.len();
        // First the stations' own numbers, then the rates (which a station over 100% caps).
        let service: Vec<Value> = (0..n).map(|i| mean_service(self.element(i))).collect();
        let units: Vec<Option<(u32, bool)>> = model.elements.iter().map(server_units).collect();
        let avail: Vec<Option<Value>> = model.elements.iter().map(availability).collect();
        // What a station can pass on at most: its units (a task: its pool's, as if alone on
        // it) over the service time, times the availability where that is exact (else 1, still
        // an upper bound).
        let caps: Vec<Option<f64>> = (0..n)
            .map(|i| {
                let s = service[i].value().filter(|s| *s > 0.0)?;
                let units = match (&units[i], self.pool_use(i)) {
                    (Some((c, _)), _) => f64::from(*c),
                    (None, Some(Ok((j, q)))) => {
                        f64::from(model.resources[j].most_units()) / f64::from(q.max(1))
                    }
                    _ => return None,
                };
                let a = avail[i]
                    .as_ref()
                    .and_then(Value::exact_value)
                    .unwrap_or(1.0);
                Some(units * a / s)
            })
            .collect();
        // What a station certainly passes on: its fixed units over the exact service time,
        // times the least availability. None is certain (0) with units on a calendar, a service
        // time only at least or not known, an availability not known, or a task with
        // alternatives.
        let sure: Vec<Option<f64>> = (0..n)
            .map(|i| {
                let units = match (&units[i], self.pool_use(i)) {
                    (Some((c, false)), _) => f64::from(*c),
                    (None, Some(Ok((j, q)))) => {
                        let p = &model.resources[j];
                        if !p.shifts.is_empty() || !p.availability.is_empty() || !p.available {
                            return Some(0.0);
                        }
                        f64::from(p.most_units()) / f64::from(q.max(1))
                    }
                    (None, None) => return None,
                    _ => return Some(0.0),
                };
                let Some(s) = service[i].exact_value() else {
                    return Some(0.0);
                };
                let a = match &avail[i] {
                    None => 1.0,
                    Some(Value::Known { value, .. }) => *value,
                    Some(Value::Unknown(_)) => 0.0,
                };
                Some(if s > 0.0 {
                    units * a / s
                } else {
                    f64::INFINITY
                })
            })
            .collect();
        // Cut to an upper bound of its capacity, a station on a loop would seem to get less
        // than it is offered, and not certainly so: there rates are cut only to a certain one.
        let caps: Vec<Option<f64>> = (0..n)
            .map(|i| match (caps[i], sure[i]) {
                (Some(c), Some(s)) if self.looped[i] && s < c * (1.0 - 1e-12) => None,
                (c, _) => c,
            })
            .collect();
        let (mut flows, mut at_most) = self.flows(&caps, &sure);
        let mut rates: Vec<Value> = (0..n).map(|i| self.arrivals(&flows, &at_most, i)).collect();
        // Tasks on a pool that may be loaded to 100% or more pass on at most what they get:
        // again, with what follows them bounded.
        // The pools' own rows use the rates offered, before that bound.
        let offered = rates.clone();
        let overloaded = self.overloaded_pool_users(&rates, &service);
        if overloaded.iter().any(|b| *b) {
            let sure: Vec<Option<f64>> = (0..n)
                .map(|i| if overloaded[i] { Some(0.0) } else { sure[i] })
                .collect();
            (flows, at_most) = self.flows(&caps, &sure);
            rates = (0..n).map(|i| self.arrivals(&flows, &at_most, i)).collect();
        }

        let mut stations = Vec::new();
        let mut unstable = vec![false; n];
        for (i, e) in model.elements.iter().enumerate() {
            let kind = match e.kind {
                ElementKind::Server { .. } => StationKind::Server,
                ElementKind::SharedTask { .. } => StationKind::SharedTask,
                ElementKind::Delay { .. } => StationKind::Delay,
                ElementKind::Conveyor { .. } => StationKind::Conveyor,
                _ => continue,
            };
            let mut st = Station {
                id: e.id.clone(),
                kind,
                arrival_rate: rates[i].clone(),
                mean_service: service[i].clone(),
                units: units[i].map(|(c, _)| c),
                units_vary: units[i].is_some_and(|(_, v)| v),
                availability: avail[i].clone(),
                utilisation: Value::Unknown(Why::SharedPool),
                load: Value::Unknown(Why::SharedPool),
                status: Status::Unknown,
                units_for_target: None,
                formula: Err(Why::SharedPool),
                time_on: None,
            };
            let work = rates[i].combine(&service[i], false);
            match kind {
                StationKind::Delay => {
                    st.utilisation = work.clone();
                    st.load = work.clone();
                    st.status = if work.value().is_some() {
                        Status::Fine
                    } else {
                        Status::Unknown
                    };
                    st.formula = match &work {
                        Value::Known {
                            value,
                            bound: Bound::Exact,
                        } => Ok(Formula {
                            name: "M/G/∞".into(),
                            mean_wait: 0.0,
                            mean_queue: 0.0,
                            wait_probability: Some(0.0),
                            lost_share: None,
                            balked_share: None,
                            abandoned_share: None,
                            served_share: 1.0,
                            mean_inside: *value,
                        }),
                        Value::Known { .. } if e.kind.processing_by().is_some() => {
                            Err(Why::ByProperty)
                        }
                        Value::Known { .. } => Err(Why::Bounds),
                        Value::Unknown(w) => Err(w.clone()),
                    };
                }
                StationKind::Server => {
                    let (c, vary) = units[i].unwrap_or((1, false));
                    let units_value = Value::Known {
                        value: f64::from(c.max(1)),
                        bound: if vary { Bound::AtMost } else { Bound::Exact },
                    };
                    st.utilisation = work.combine(&units_value, true);
                    st.load = st
                        .utilisation
                        .combine(avail[i].as_ref().unwrap_or(&Value::exact(1.0)), true);
                    // Held up, a unit is occupied longer than its service: at least this load.
                    if self.held_up_by(i).is_some() {
                        st.load = match st.load {
                            Value::Known {
                                value,
                                bound: Bound::Exact | Bound::AtLeast,
                            } => Value::Known {
                                value,
                                bound: Bound::AtLeast,
                            },
                            Value::Known { .. } => Value::Unknown(Why::Bounds),
                            unknown => unknown,
                        };
                    }
                    (st.status, st.units_for_target) = judge(
                        &st.load,
                        &work,
                        avail[i].as_ref(),
                        target,
                        tail_join(&e.kind),
                    );
                    unstable[i] = st.status == Status::Unstable;
                }
                StationKind::Conveyor => {
                    // One unit carrying a slot of pitch p at speed v: p / v per entity, so the
                    // utilisation (and load) is the arrival rate × p / v.
                    let one = Value::exact(1.0);
                    st.utilisation = work.combine(&one, true);
                    st.load = st.utilisation.clone();
                    // No count of units is given: a faster conveyor or a smaller pitch helps,
                    // not more units.
                    st.status = judge(&st.load, &work, None, target, 1.0).0;
                    unstable[i] = st.status == Status::Unstable;
                    st.formula = Err(Why::ConveyorTime);
                    st.time_on = Some(self.conveyor_time_on(i));
                }
                StationKind::SharedTask | StationKind::Pool => {}
            }
            stations.push(st);
        }
        // Formulas, now that every station's stability is known.
        for st in &mut stations {
            let Some(&i) = self.index.get(&st.id) else {
                continue;
            };
            if st.kind == StationKind::Server {
                st.formula = self.server_formula(i, &st.arrival_rate, &service[i], &unstable);
                // A formula gives the utilisation exactly, losses and abandonment included.
                if let Ok(f) = &st.formula
                    && let (Some(c), Some(s)) = (st.units, service[i].exact_value())
                {
                    let served = served_rate(f, st.arrival_rate.value().unwrap_or(0.0));
                    st.utilisation = Value::exact(served * s / f64::from(c.max(1)));
                }
            }
        }
        self.pools(&mut stations, &offered, &service, &unstable, target);

        let bottleneck = bottleneck(&stations);
        Capacity {
            target,
            stations,
            bottleneck,
        }
    }

    /// A server's queueing formula, or why none holds.
    fn server_formula(
        &self,
        i: usize,
        rate: &Value,
        service: &Value,
        unstable: &[bool],
    ) -> Result<Formula, Why> {
        let e = self.element(i);
        let ElementKind::Server {
            capacity,
            service: dist,
            queue_capacity,
            queue_limit,
            patience,
            balking,
            setup,
            ..
        } = &e.kind
        else {
            return Err(Why::SharedPool);
        };
        if let Some(w) = local_obstacle(&e.kind) {
            return Err(w);
        }
        if let Some(j) = self.held_up_by(i) {
            return Err(Why::HeldUp(self.element(j).id.clone()));
        }
        let lambda = match rate {
            Value::Known {
                value,
                bound: Bound::Exact,
            } => *value,
            // Only bounded: entities lost or held up before, so the arrivals say why.
            Value::Known { .. } => {
                return Err(self.arrivals_not(i, unstable, true).unwrap_or(Why::Bounds));
            }
            Value::Unknown(w) => return Err(w.clone()),
        };
        let s = match service {
            Value::Known { value, .. } => *value,
            Value::Unknown(w) => return Err(w.clone()),
        };
        let c = *capacity;
        let extras = queue_capacity.is_some() || patience.is_some() || balking.is_some();
        if extras {
            if !queue_limit.is_fixed() {
                return Err(Why::QueueLimitVaries);
            }
            if let Some(p) = patience {
                if p.until == PatienceUntil::Done {
                    return Err(Why::PatienceUntilDone);
                }
                if !exponential(&p.time) {
                    return Err(Why::PatienceNotExponential);
                }
            }
            if queue_capacity.is_some() && self.blocks_sender(i) {
                return Err(Why::BlocksSender);
            }
            if setup.is_some() || !exponential(dist) {
                return Err(Why::NotExponential);
            }
            if let Some(w) = self.arrivals_not(i, unstable, true) {
                return Err(w);
            }
            let theta = patience
                .as_ref()
                .and_then(|p| p.time.mean())
                .map_or(0.0, |m| 1.0 / m);
            return birth_death(lambda, 1.0 / s, c, *queue_capacity, theta, balking.as_ref())
                .ok_or(Why::Unstable);
        }
        if unstable[i] {
            return Err(Why::Unstable);
        }
        if setup.is_none() && exponential(dist) {
            if let Some(w) = self.arrivals_not(i, unstable, false) {
                return Err(w);
            }
            return erlang_c(lambda, 1.0 / s, c).ok_or(Why::Unstable);
        }
        if c > 1 {
            return Err(Why::SeveralUnitsGeneral);
        }
        if let Some(w) = self.arrivals_not(i, unstable, true) {
            return Err(w);
        }
        let variance = std::iter::once(dist)
            .chain(setup.as_ref().map(|s| &s.time))
            .map(Dist::variance)
            .sum::<Option<f64>>()
            .ok_or(Why::NoVariance)?;
        pollaczek_khinchine(lambda, s, variance).ok_or(Why::Unstable)
    }

    /// A conveyor's mean time on it: its length over its speed, exactly when nothing after it
    /// can hold it up, else at least (an item stops at the end while what follows is full).
    fn conveyor_time_on(&self, i: usize) -> Value {
        let ElementKind::Conveyor { length, speed, .. } = &self.element(i).kind else {
            return Value::Unknown(Why::NoMean);
        };
        if !(length.is_finite() && speed.is_finite() && *length > 0.0 && *speed > 0.0) {
            return Value::Unknown(Why::NoMean);
        }
        Value::Known {
            value: length / speed,
            bound: if self.held_up_by(i).is_some() {
                Bound::AtLeast
            } else {
                Bound::Exact
            },
        }
    }

    /// The server whose full waiting room can hold up element `i`'s units: the next server on
    /// its ordinary way (through gateways, batches and unbatches, which pass entities on at once)
    /// with a finite waiting room and no way for those it turns away.
    fn held_up_by(&self, i: usize) -> Option<usize> {
        let mut seen = BTreeSet::from([i]);
        let ordinary = |j: usize| {
            self.outs[j]
                .iter()
                .filter(|&&k| !marked(&self.model.connections[k]))
                .filter_map(|&k| self.end_of(k))
                .collect::<Vec<_>>()
        };
        let mut todo: VecDeque<usize> = ordinary(i).into();
        while let Some(j) = todo.pop_front() {
            if !seen.insert(j) {
                continue;
            }
            match &self.element(j).kind {
                ElementKind::Server {
                    transfer: Some(t), ..
                } if t.holds_sender => return Some(j),
                ElementKind::Server {
                    queue_capacity: Some(_),
                    ..
                } if !self.outs[j]
                    .iter()
                    .any(|&k| self.model.connections[k].turned_away) =>
                {
                    return Some(j);
                }
                ElementKind::ExclusiveGateway { .. }
                | ElementKind::ParallelGateway
                | ElementKind::Batch { .. }
                | ElementKind::Unbatch => todo.extend(ordinary(j)),
                _ => {}
            }
        }
        None
    }

    /// Whether a full waiting room at server `i` holds up an element sending to it: any sender
    /// other than a source, unless the server turns entities away by a way of their own.
    fn blocks_sender(&self, i: usize) -> bool {
        let turns_away = self.outs[i]
            .iter()
            .any(|&k| self.model.connections[k].turned_away);
        !turns_away
            && self.ins[i].iter().any(|&k| {
                self.start_of(k)
                    .is_some_and(|f| !matches!(self.element(f).kind, ElementKind::Source { .. }))
            })
    }

    /// The one pool shared task `i` uses and how many units, or why it has no one pool (`None`:
    /// not a shared task).
    fn pool_use(&self, i: usize) -> Option<PoolUse> {
        let ElementKind::SharedTask { needs, .. } = &self.element(i).kind else {
            return None;
        };
        let [need] = &needs[..] else {
            return Some(Err(Why::Alternatives));
        };
        let matching: Vec<usize> = self
            .model
            .resources
            .iter()
            .enumerate()
            .filter(|(_, p)| match need {
                Need::Resource { id, .. } => &p.id == id,
                Need::Roles { roles, .. } => roles.iter().all(|r| p.roles.contains(r)),
            })
            .map(|(j, _)| j)
            .collect();
        Some(match matching[..] {
            [j] => Ok((j, need.quantity())),
            _ => Err(Why::Alternatives),
        })
    }

    /// The shared tasks on a pool that may be loaded to 100% or more: its load is that or more,
    /// or not bounded above (a rate or service time not known or only at least, a task with
    /// alternatives that might use it).
    fn overloaded_pool_users(&self, rates: &[Value], service: &[Value]) -> Vec<bool> {
        let n = self.model.elements.len();
        let uses: Vec<Option<PoolUse>> = (0..n).map(|i| self.pool_use(i)).collect();
        let pools = self.model.resources.len();
        let mut work = vec![0.0_f64; pools];
        let mut unbounded = vec![false; pools];
        for (i, u) in uses.iter().enumerate() {
            match u {
                Some(Ok((j, q))) => {
                    let w = rates[i]
                        .combine(&service[i], false)
                        .combine(&Value::exact(f64::from(*q)), false);
                    match w {
                        Value::Known {
                            value,
                            bound: Bound::Exact | Bound::AtMost,
                        } => work[*j] += value,
                        _ => unbounded[*j] = true,
                    }
                }
                Some(Err(_)) => {
                    for (j, flag) in unbounded.iter_mut().enumerate() {
                        *flag |= self.might_use(i, j);
                    }
                }
                None => {}
            }
        }
        uses.iter()
            .map(|u| {
                matches!(u, Some(Ok((j, _))) if unbounded[*j]
                    || work[*j] >= f64::from(self.model.resources[*j].most_units().max(1)))
            })
            .collect()
    }

    /// Whether shared task `i` names pool `j` among its needs.
    fn might_use(&self, i: usize, j: usize) -> bool {
        let ElementKind::SharedTask { needs, .. } = &self.element(i).kind else {
            return false;
        };
        let p = &self.model.resources[j];
        needs.iter().any(|n| match n {
            Need::Resource { id, .. } => &p.id == id,
            Need::Roles { roles, .. } => roles.iter().all(|r| p.roles.contains(r)),
        })
    }

    /// The pools' rows, and the shared tasks' utilisation and formulas.
    fn pools(
        &self,
        stations: &mut Vec<Station>,
        rates: &[Value],
        service: &[Value],
        unstable: &[bool],
        target: f64,
    ) {
        let model = self.model;
        let tasks: Vec<(usize, PoolUse)> = (0..model.elements.len())
            .filter_map(|i| self.pool_use(i).map(|u| (i, u)))
            .collect();
        let mut pool_rows = Vec::new();
        for (j, pool) in model.resources.iter().enumerate() {
            let users: Vec<usize> = tasks
                .iter()
                .filter(|(i, r)| match r {
                    Ok((p, _)) => *p == j,
                    Err(_) => self.might_use(*i, j),
                })
                .map(|(i, _)| *i)
                .collect();
            if users.is_empty() {
                continue;
            }
            let units = pool.most_units();
            let vary = !pool.shifts.is_empty() || !pool.availability.is_empty() || !pool.available;
            let mut work = Value::exact(0.0);
            let mut partial = false;
            for &i in &users {
                match tasks.iter().find(|(t, _)| *t == i).map(|(_, r)| r) {
                    Some(Ok((_, q))) => {
                        let w = rates[i]
                            .combine(&service[i], false)
                            .combine(&Value::exact(f64::from(*q)), false);
                        work = add(&work, &w);
                    }
                    _ => partial = true,
                }
            }
            // Tasks that might use this pool add load not counted: what is counted is at least.
            if partial {
                work = match work {
                    Value::Known {
                        value,
                        bound: Bound::Exact | Bound::AtLeast,
                    } => Value::Known {
                        value,
                        bound: Bound::AtLeast,
                    },
                    Value::Known { .. } => Value::Unknown(Why::Bounds),
                    unknown => unknown,
                };
            }
            let units_value = Value::Known {
                value: f64::from(units.max(1)),
                bound: if vary { Bound::AtMost } else { Bound::Exact },
            };
            let utilisation = work.combine(&units_value, true);
            let (status, units_for_target) = judge(&utilisation, &work, None, target, 1.0);
            pool_rows.push(Station {
                id: pool.id.clone(),
                kind: StationKind::Pool,
                arrival_rate: Value::Unknown(Why::SharedPool),
                mean_service: Value::Unknown(Why::SharedPool),
                units: Some(units),
                units_vary: vary,
                availability: None,
                utilisation: utilisation.clone(),
                load: utilisation,
                status,
                units_for_target,
                formula: Err(Why::SharedPool),
                time_on: None,
            });
            // A pool serving one task, one unit at a time, at fixed units: that task is an
            // ordinary station with the pool's units.
            for st in stations.iter_mut() {
                let Some(&i) = self.index.get(&st.id) else {
                    continue;
                };
                if !users.contains(&i) {
                    continue;
                }
                let alone = users.len() == 1
                    && !partial
                    && !vary
                    && matches!(tasks.iter().find(|(t, _)| *t == i), Some((_, Ok((_, 1)))));
                st.units = alone.then_some(units);
                // The task's own share of the pool: its work over the pool's units.
                st.utilisation = match tasks.iter().find(|(t, _)| *t == i) {
                    Some((_, Ok((_, q)))) => rates[i]
                        .combine(&service[i], false)
                        .combine(&Value::exact(f64::from(*q)), false)
                        .combine(&units_value, true),
                    Some((_, Err(w))) => Value::Unknown(w.clone()),
                    None => Value::Unknown(Why::SharedPool),
                };
                st.load = st.utilisation.clone();
                st.status = pool_rows.last().map_or(Status::Unknown, |p| p.status);
                st.formula = if alone {
                    self.task_formula(i, &st.arrival_rate, &service[i], units, unstable)
                } else {
                    Err(Why::SharedPool)
                };
            }
        }
        stations.extend(pool_rows);
    }

    /// The formula for a task alone on a pool of `c` units.
    fn task_formula(
        &self,
        i: usize,
        rate: &Value,
        service: &Value,
        c: u32,
        unstable: &[bool],
    ) -> Result<Formula, Why> {
        let ElementKind::SharedTask { service: dist, .. } = &self.element(i).kind else {
            return Err(Why::SharedPool);
        };
        if let Some(j) = self.held_up_by(i) {
            return Err(Why::HeldUp(self.element(j).id.clone()));
        }
        let lambda = rate.exact_value().ok_or(Why::Bounds)?;
        let s = service.exact_value().ok_or(Why::NoMean)?;
        if lambda * s >= f64::from(c) {
            return Err(Why::Unstable);
        }
        // Tasks before this one sharing pools break the product form; they show as `After`.
        if exponential(dist) {
            if let Some(w) = self.arrivals_not(i, unstable, false) {
                return Err(w);
            }
            return erlang_c(lambda, 1.0 / s, c).ok_or(Why::Unstable);
        }
        if c > 1 {
            return Err(Why::SeveralUnitsGeneral);
        }
        if let Some(w) = self.arrivals_not(i, unstable, true) {
            return Err(w);
        }
        let variance = dist.variance().ok_or(Why::NoVariance)?;
        pollaczek_khinchine(lambda, s, variance).ok_or(Why::Unstable)
    }
}

/// The station whose load is surely the highest: at least as high as every other server's and
/// pool's could be. None when a load that is not known, or only bounded below, could be higher.
fn bottleneck(stations: &[Station]) -> Option<ElementId> {
    let candidates: Vec<&Station> = stations
        .iter()
        .filter(|s| {
            matches!(
                s.kind,
                StationKind::Server | StationKind::Pool | StationKind::Conveyor
            )
        })
        .collect();
    let low = |s: &Station| match s.load {
        Value::Known {
            value,
            bound: Bound::Exact | Bound::AtLeast,
        } => Some(value),
        _ => None,
    };
    let high = |s: &Station| match s.load {
        Value::Known {
            value,
            bound: Bound::Exact | Bound::AtMost,
        } => Some(value),
        _ => None,
    };
    let (best, value) = candidates
        .iter()
        .filter_map(|s| low(s).map(|v| (s, v)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    candidates
        .iter()
        .filter(|s| s.id != best.id)
        .all(|s| high(s).is_some_and(|h| h <= value))
        .then(|| best.id.clone())
}

fn add(a: &Value, b: &Value) -> Value {
    match (a, b) {
        (Value::Unknown(w), _) | (_, Value::Unknown(w)) => Value::Unknown(w.clone()),
        (Value::Known { value: x, bound: p }, Value::Known { value: y, bound: q }) => {
            match p.with(*q, false) {
                Some(bound) => Value::Known {
                    value: x + y,
                    bound,
                },
                None => Value::Unknown(Why::Bounds),
            }
        }
    }
}

/// The rate of entities a formula's station serves, out of `lambda` arriving.
fn served_rate(f: &Formula, lambda: f64) -> f64 {
    lambda * f.served_share
}

/// The share of arrivals a server still takes in when its queue is very long: 0 with a finite
/// room, patience or balking that falls to 0 (the queue cannot grow without limit), the last
/// balking row's chance, else 1. A load times this of 100% or more is unstable.
fn tail_join(kind: &ElementKind) -> f64 {
    let ElementKind::Server {
        queue_capacity,
        patience,
        balking,
        ..
    } = kind
    else {
        return 1.0;
    };
    if queue_capacity.is_some() || patience.is_some() {
        return 0.0;
    }
    match balking.as_ref().map(|b| &b.join) {
        None => 1.0,
        Some(BalkJoin::Reciprocal) => 0.0,
        Some(BalkJoin::Table(rows)) => rows.last().map_or(1.0, |r| r.probability.clamp(0.0, 1.0)),
    }
}

/// The station's status, and the units that would bring it under the target.
fn judge(
    load: &Value,
    work: &Value,
    availability: Option<&Value>,
    target: f64,
    tail: f64,
) -> (Status, Option<u32>) {
    let status = match load {
        Value::Known { value, bound } => {
            let level = |v: f64| {
                if v * tail >= 1.0 {
                    Status::Unstable
                } else if v >= target {
                    Status::Congested
                } else {
                    Status::Fine
                }
            };
            match bound {
                Bound::Exact => level(*value),
                // A lower bound shows the station is at least that busy; an upper bound that it
                // is at most.
                Bound::AtLeast if level(*value) != Status::Fine => level(*value),
                Bound::AtMost if level(*value) == Status::Fine => Status::Fine,
                _ => Status::Unknown,
            }
        }
        Value::Unknown(_) => Status::Unknown,
    };
    // Only from exact numbers: a bounded work or availability gives no number of units.
    let a = match availability {
        None => Some(1.0),
        Some(v) => v.exact_value(),
    };
    let needed = match (status, work, a) {
        (
            Status::Congested | Status::Unstable,
            Value::Known {
                value,
                bound: Bound::Exact,
            },
            Some(a),
        ) => {
            let per_unit = target * a;
            // The fewest units whose load is under the target (strictly, so 85% is not fine).
            let mut c = (value / per_unit).ceil().max(1.0);
            if value / (c * a) >= target {
                c += 1.0;
            }
            Some(c as u32)
        }
        _ => None,
    };
    (status, needed)
}

/// The mean time an entity holds a unit at a station, with setups before every job.
fn mean_service(e: &Element) -> Value {
    // A conveyor's slot takes pitch over speed to pass: the most it carries is its inverse.
    if let ElementKind::Conveyor { speed, .. } = &e.kind {
        return match e.kind.conveyor_pitch() {
            Some(p) if p.is_finite() && p > 0.0 && speed.is_finite() && *speed > 0.0 => {
                Value::exact(p / speed)
            }
            _ => Value::Unknown(Why::NoMean),
        };
    }
    let (time, setup) = match &e.kind {
        ElementKind::Server { service, setup, .. } => (service, setup.as_ref()),
        ElementKind::SharedTask { service, .. } => (service, None),
        ElementKind::Delay { duration, .. } => (duration, None),
        _ => return Value::Unknown(Why::NoMean),
    };
    // With times by property value, the least of the means; with changeovers, none: what the
    // entities' mix or order adds is not known, so the mean is at least this.
    let mut bound = Bound::Exact;
    let mut least = mean_of(time);
    if let Some(by) = e.kind.processing_by() {
        bound = Bound::AtLeast;
        for v in &by.by_value {
            least = match (least, mean_of(&v.time)) {
                (Some(a), Some(b)) => Some(a.min(b)),
                _ => None,
            };
        }
    }
    let setup_mean = match setup {
        None => Some(0.0),
        Some(s) if s.on_change_of.is_some() || !s.by_change.is_empty() => {
            bound = Bound::AtLeast;
            Some(0.0)
        }
        Some(s) => mean_of(&s.time),
    };
    match (least, setup_mean) {
        (Some(m), Some(su)) => Value::Known {
            value: m + su,
            bound,
        },
        _ => Value::Unknown(Why::NoMean),
    }
}

/// A server's units, and whether they follow a calendar (then the most on shift).
fn server_units(e: &Element) -> Option<(u32, bool)> {
    // A conveyor is one station: its slots are not units.
    if matches!(e.kind, ElementKind::Conveyor { .. }) {
        return Some((1, false));
    }
    let ElementKind::Server {
        shifts,
        available,
        availability,
        ..
    } = &e.kind
    else {
        return None;
    };
    let vary = !shifts.is_empty() || !availability.is_empty() || !*available;
    Some((e.kind.most_units().unwrap_or(1), vary))
}

/// The share of time a server's unit is not failed: the mean time to failure over that plus
/// the mean repair, the failure clock running in elapsed time from the end of each repair. For
/// failures that wait for the job to end the unit stays up longer, so it is at least that.
fn availability(e: &Element) -> Option<Value> {
    let ElementKind::Server {
        breakdown: Some(b), ..
    } = &e.kind
    else {
        return None;
    };
    Some(match (mean_of(&b.time_to_failure), mean_of(&b.repair)) {
        (Some(f), Some(r)) if f + r > 0.0 => Value::Known {
            value: f / (f + r),
            bound: if b.interruptible {
                Bound::Exact
            } else {
                Bound::AtLeast
            },
        },
        _ => Value::Unknown(Why::NoMean),
    })
}

/// Whether a server is plain M/M/c: exponential times and none of the features that change
/// how it serves.
fn plain_exponential(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Server {
            service,
            queue_capacity: None,
            patience: None,
            balking: None,
            setup: None,
            ..
        } if exponential(service)
    ) && local_obstacle(kind).is_none()
}

/// What in a server itself rules out every formula, if anything.
fn local_obstacle(kind: &ElementKind) -> Option<Why> {
    let ElementKind::Server {
        processing_by,
        breakdown,
        transfer,
        available,
        availability,
        shifts,
        setup,
        health_check,
        ..
    } = kind
    else {
        return None;
    };
    if !shifts.is_empty() || !availability.is_empty() || !*available {
        Some(Why::Calendar)
    } else if breakdown.is_some() {
        Some(Why::Breakdowns)
    } else if health_check.is_some() {
        Some(Why::HealthCheck)
    } else if transfer.is_some() {
        Some(Why::Transfer)
    } else if processing_by.is_some() {
        Some(Why::ByProperty)
    } else if setup
        .as_ref()
        .is_some_and(|s| s.on_change_of.is_some() || !s.by_change.is_empty())
    {
        Some(Why::Changeover)
    } else {
        None
    }
}

/// M/M/c: Erlang C by the Erlang B recursion (stable for many units).
fn erlang_c(lambda: f64, mu: f64, c: u32) -> Option<Formula> {
    let (a, cf) = (lambda / mu, f64::from(c.max(1)));
    let rho = a / cf;
    if rho.is_nan() || rho >= 1.0 || lambda <= 0.0 {
        return (lambda == 0.0).then(|| Formula {
            name: format!("M/M/{c}"),
            mean_wait: 0.0,
            mean_queue: 0.0,
            wait_probability: Some(0.0),
            lost_share: None,
            balked_share: None,
            abandoned_share: None,
            served_share: 1.0,
            mean_inside: 0.0,
        });
    }
    let mut b = 1.0;
    for k in 1..=c.max(1) {
        b = a * b / (f64::from(k) + a * b);
    }
    let wait = b / (1.0 - rho * (1.0 - b));
    let mean_wait = wait / (cf * mu - lambda);
    let mean_queue = lambda * mean_wait;
    Some(Formula {
        name: format!("M/M/{c}"),
        mean_wait,
        mean_queue,
        wait_probability: Some(wait),
        lost_share: None,
        balked_share: None,
        abandoned_share: None,
        served_share: 1.0,
        mean_inside: mean_queue + a,
    })
}

/// M/G/1: the Pollaczek–Khinchine formula, Wq = λ E[S²] / (2 (1 − ρ)).
fn pollaczek_khinchine(lambda: f64, s: f64, variance: f64) -> Option<Formula> {
    let rho = lambda * s;
    if rho.is_nan() || rho >= 1.0 {
        return None;
    }
    let mean_wait = lambda * (variance + s * s) / (2.0 * (1.0 - rho));
    let mean_queue = lambda * mean_wait;
    Some(Formula {
        name: "M/G/1".into(),
        mean_wait,
        mean_queue,
        wait_probability: Some(rho),
        lost_share: None,
        balked_share: None,
        abandoned_share: None,
        served_share: 1.0,
        mean_inside: mean_queue + rho,
    })
}

/// A birth–death queue: c exponential units at rate `mu`, Poisson arrivals at `lambda` joining
/// with the balking chance, at most `queue` waiting (`None`: unlimited), each waiting one
/// giving up at rate `theta` (exponential patience until service starts). M/M/c/K, Erlang A
/// (M/M/c+M) and balking, alone or together. `None` if it has no steady state.
fn birth_death(
    lambda: f64,
    mu: f64,
    c: u32,
    queue: Option<u32>,
    theta: f64,
    balking: Option<&crate::Balking>,
) -> Option<Formula> {
    let cf = f64::from(c.max(1));
    let join = |n: u64| -> f64 {
        let Some(b) = balking else { return 1.0 };
        let waiting = n.saturating_sub(u64::from(c));
        let counted = if b.counts_in_service { n } else { waiting };
        match &b.join {
            crate::BalkJoin::Reciprocal => 1.0 / (counted as f64 + 1.0),
            crate::BalkJoin::Table(rows) => rows
                .iter()
                .rev()
                .find(|r| u64::from(r.waiting) <= counted)
                .map_or(1.0, |r| r.probability.clamp(0.0, 1.0)),
        }
    };
    let last = queue.map(|q| u64::from(c) + u64::from(q));
    // The most any state from n on takes in: a later balking row can take in more.
    let most_from = |n: u64| -> f64 {
        let now = join(n);
        match balking.map(|b| (&b.join, b.counts_in_service)) {
            Some((crate::BalkJoin::Table(rows), in_service)) => {
                let counted = if in_service {
                    n
                } else {
                    n.saturating_sub(u64::from(c))
                };
                rows.iter()
                    .filter(|r| u64::from(r.waiting) > counted)
                    .map(|r| r.probability.clamp(0.0, 1.0))
                    .fold(now, f64::max)
            }
            _ => now,
        }
    };
    // With nothing to bound the queue, what it takes in at length must be less than the units
    // serve.
    if last.is_none() && theta <= 0.0 {
        let tail = match balking.map(|b| &b.join) {
            None => 1.0,
            Some(crate::BalkJoin::Reciprocal) => 0.0,
            Some(crate::BalkJoin::Table(rows)) => {
                rows.last().map_or(1.0, |r| r.probability.clamp(0.0, 1.0))
            }
        };
        if lambda * tail >= cf * mu {
            return None;
        }
    }
    // Log weights of the states n = 0, 1, …, until they are negligible.
    let mut logs = vec![0.0_f64];
    let mut peak = 0.0_f64;
    let mut n: u64 = 0;
    loop {
        if last.is_some_and(|l| n >= l) {
            break;
        }
        let up = lambda * join(n);
        if up <= 0.0 {
            break;
        }
        let k = n + 1;
        let down =
            (k.min(u64::from(c)) as f64) * mu + (k.saturating_sub(u64::from(c)) as f64) * theta;
        let next = logs[logs.len() - 1] + ln(up / down);
        logs.push(next);
        peak = peak.max(next);
        n = k;
        if last.is_none() && k > u64::from(c) && lambda * most_from(k) < down && next < peak - 50.0
        {
            break;
        }
        if logs.len() > 5_000_000 {
            return None;
        }
    }
    let weights: Vec<f64> = logs.iter().map(|l| exp(l - peak)).collect();
    let total: f64 = weights.iter().sum();
    let p: Vec<f64> = weights.iter().map(|w| w / total).collect();
    // An entity joining at place q in the queue (q − 1 ahead of it, all units busy) moves up at
    // rate cμ + (q − 1)θ and gives up at rate θ: the time to the next event does not depend on
    // which it is (competing exponentials), so it is served with chance Π a_i and, if so, after
    // Σ 1/(cμ + iθ) (Garnett, Mandelbaum and Reiman 2002 for M/M/N+M).
    let rate_c = cf * mu;
    let (mut reach, mut time) = (1.0_f64, 0.0_f64);
    let mut served_at = Vec::with_capacity(p.len());
    for q in 1..=p.len().saturating_sub(c as usize) {
        let qf = q as f64;
        reach *= (rate_c + (qf - 1.0) * theta) / (rate_c + qf * theta);
        time += 1.0 / (rate_c + qf * theta);
        served_at.push((reach, time));
    }
    let (mut joining, mut full, mut balked) = (0.0, 0.0, 0.0);
    let (mut served, mut served_wait, mut waiting_join, mut lq, mut inside) =
        (0.0, 0.0, 0.0, 0.0, 0.0);
    for (n, &pn) in p.iter().enumerate() {
        let is_full = last.is_some_and(|l| n as u64 >= l);
        if is_full {
            full += pn;
        } else {
            let j = join(n as u64);
            joining += pn * j;
            balked += pn * (1.0 - j);
            if n < c as usize {
                served += pn * j;
            } else {
                waiting_join += pn * j;
                if let Some(&(chance, wait)) = served_at.get(n - c as usize) {
                    served += pn * j * chance;
                    served_wait += pn * j * chance * wait;
                }
            }
        }
        lq += pn * n.saturating_sub(c as usize) as f64;
        inside += pn * n as f64;
    }
    let share = |a: f64, b: f64| if b > 0.0 { a / b } else { 0.0 };
    let mut name = format!("M/M/{c}");
    if let Some(l) = last {
        name.push_str(&format!("/{l}"));
    }
    if theta > 0.0 {
        name.push_str("+M");
    }
    if balking.is_some() {
        name.push_str(" with balking");
    }
    Some(Formula {
        name,
        mean_wait: share(served_wait, served),
        mean_queue: lq,
        wait_probability: Some(share(waiting_join, joining)),
        lost_share: last.map(|_| share(full, full + joining)),
        balked_share: balking.map(|_| balked),
        abandoned_share: (theta > 0.0).then(|| share(theta * lq, lambda * joining)),
        served_share: served,
        mean_inside: inside,
    })
}

#[cfg(test)]
mod tests;
