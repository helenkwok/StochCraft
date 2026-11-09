//! The simulation behind a compiled model: one store per element input, processes per element,
//! and the statistics each element keeps.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use stochcraft_des::{
    Ctx, Gate, Resource, ResourceGuard, Signal, Sim, Stopped, Store, Watch, Watching,
};
use stochcraft_model::calendar::{self, Calendar, DateTime};
use stochcraft_model::{
    Breakdown, Element, ElementId, ElementKind, Model, Need, ResourcePool, Timed,
};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::{Tally, TimeWeighted};

use crate::{Assign, Compiled, ElementReport, Entity, Held, Report, RunError, Source};

/// What an element assigns to entities as they enter, and the stream it draws values from.
struct Props {
    assigns: Vec<Assign>,
    rng: RefCell<Rng>,
}

/// The model's property slots and each element's assignments, shared by the ports.
struct Assigner {
    slots: Vec<String>,
    elements: Vec<Props>,
}

impl Assigner {
    /// The value of property `name` that `e` holds.
    fn read(&self, e: &Entity, name: &str) -> Option<stochcraft_model::expr::Value> {
        let slot = self.slots.binary_search_by(|s| s.as_str().cmp(name)).ok()?;
        e.properties.get(slot)?.as_ref().map(Held::value)
    }

    /// Makes element `index`'s assignments to `e`, all from the values it arrived with, and
    /// records the values in the element's statistics.
    fn enter(&self, index: usize, e: &mut Entity, stats: &[RefCell<BlockStats>]) {
        let Some(props) = self.elements.get(index) else {
            return;
        };
        if props.assigns.is_empty() {
            return;
        }
        if e.properties.len() < self.slots.len() {
            e.properties.resize(self.slots.len(), None);
        }
        let read = |name: &str| {
            let slot = self.slots.binary_search_by(|s| s.as_str().cmp(name)).ok()?;
            e.properties.get(slot)?.as_ref().map(Held::value)
        };
        let new: Vec<Option<Held>> = props
            .assigns
            .iter()
            .map(|a| match &a.source {
                Source::Fixed(v) => v.clone(),
                Source::Random(d) => {
                    let x = d.sample(&mut props.rng.borrow_mut());
                    Held::of(&stochcraft_model::expr::Value::Number(x), a.ty)
                }
                Source::Expr(x) => Held::of(&x.eval(&read), a.ty),
            })
            .collect();
        let mut s = stats.get(index).map(RefCell::borrow_mut);
        for (k, (a, v)) in props.assigns.iter().zip(new).enumerate() {
            if let (Some(s), Some(x)) = (&mut s, v.as_ref().and_then(Held::statistic))
                && let Some(t) = s.properties.get_mut(k)
            {
                t.add(x);
            }
            if let Some(slot) = e.properties.get_mut(a.slot) {
                *slot = v;
            }
        }
    }
}

impl Compiled {
    /// Runs one replication for the experiment's warm-up plus duration, with random numbers from
    /// `rng` (each element draws from its own forked stream, in model order; then each element's
    /// property values from another, so properties never change the other draws), and reports
    /// the statistics collected after the warm-up.
    ///
    /// A model whose entities can go round a loop without simulated time moving would never
    /// end; past [`Compiled::max_events_at_one_time`] events at one time the replication stops
    /// with [`RunError::TimeStopped`] instead.
    pub fn run(&self, rng: Rng) -> Result<Report, RunError> {
        self.run_watched(rng, &|| false)
    }

    /// As [`Compiled::run`], asking `cancelled` every so often (every thousand or so events)
    /// whether to stop, so that even a replication that would take very long, or never end,
    /// can be cancelled; it then returns [`RunError::Cancelled`]. Asking changes nothing else:
    /// a replication that is not cancelled gives exactly what [`Compiled::run`] gives.
    pub fn run_watched(&self, rng: Rng, cancelled: &dyn Fn() -> bool) -> Result<Report, RunError> {
        let x = &self.model.experiment;
        self.simulate(rng, x.warmup, x.duration, cancelled)
    }

    /// As [`Compiled::run`], with an explicit warm-up and duration (negative or non-finite values
    /// count as 0).
    pub fn run_for(&self, rng: Rng, warmup: f64, duration: f64) -> Result<Report, RunError> {
        self.simulate(rng, warmup, duration, &|| false)
    }

    fn simulate(
        &self,
        rng: Rng,
        warmup: f64,
        duration: f64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Report, RunError> {
        let mut live = self.start(rng, warmup, duration);
        let end = live.end;
        live.advance_watched(end, cancelled, u64::MAX)?;
        Ok(live.report())
    }

    /// One replication set up to run step by step (a [`Live`] run), exactly as
    /// [`Compiled::run`] would run it with `rng`.
    pub fn live(&self, rng: Rng) -> Live<'static> {
        let x = &self.model.experiment;
        let live = self.start(rng, x.warmup, x.duration);
        Live {
            compiled: Cow::Owned(self.clone()),
            ..live
        }
    }

    fn start(&self, mut rng: Rng, warmup: f64, duration: f64) -> Live<'_> {
        let clean = |t: f64| if t.is_finite() && t > 0.0 { t } else { 0.0 };
        let (warmup, duration) = (clean(warmup), clean(duration));
        let elements = &self.model.elements;
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let stats: Rc<Vec<RefCell<BlockStats>>> = Rc::new(
            self.assigns
                .iter()
                .map(|a| RefCell::new(BlockStats::new(0.0, a.len())))
                .collect(),
        );
        let inputs: Vec<Vec<Store<Entity>>> = self
            .inputs
            .iter()
            .map(|&n| (0..n.max(1)).map(|_| Store::new(&ctx)).collect())
            .collect();
        // A finite buffer's places: one per unit plus the waiting room. An entity holds one from
        // the moment it gets in (waiting, in service, or blocked on its way out) until it has
        // left, so a sender finding none free is blocked (or, for a source, loses its arrival).
        let places: Vec<Option<Rc<Places>>> = elements
            .iter()
            .map(|e| match e.kind {
                ElementKind::Server {
                    capacity,
                    queue_capacity: Some(q),
                    ..
                } => Some(Rc::new(Places {
                    pool: Resource::new(&ctx, u64::from(capacity) + u64::from(q)),
                    held: RefCell::new(BTreeMap::new()),
                })),
                _ => None,
            })
            .collect();
        let feeds_buffer: Vec<bool> = (0..elements.len())
            .map(|i| {
                self.routes
                    .get(i)
                    .into_iter()
                    .flatten()
                    .any(|r| places.get(r.to).is_some_and(Option::is_some))
            })
            .collect();
        let end = warmup + duration;
        let calendars = Calendars::new(&self.model, end);
        let units: Vec<Vec<(f64, u32)>> = self
            .model
            .resources
            .iter()
            .map(|r| calendars.units(r))
            .collect();
        let pools = Rc::new(Pools::new(
            &ctx,
            units
                .iter()
                .map(|u| u.first().map_or(0, |(_, n)| *n))
                .collect(),
        ));
        for (p, changes) in units.into_iter().enumerate() {
            if changes.len() > 1 {
                spawn_shifts(&mut sim, Rc::clone(&pools), p, changes);
            }
        }
        let next_id = Rc::new(RefCell::new(0u64));
        // The elements' streams first, in model order as always; then the properties'.
        let streams: Vec<Rng> = elements.iter().map(|_| rng.fork()).collect();
        let mut prop_root = rng.fork();
        let assigner = Rc::new(Assigner {
            slots: self.slots.clone(),
            elements: self
                .assigns
                .iter()
                .map(|a| Props {
                    assigns: a.clone(),
                    rng: RefCell::new(prop_root.fork()),
                })
                .collect(),
        });

        // Joins with a bill of materials: every part counts as entering.
        let assembly: Vec<bool> = self
            .quantities
            .iter()
            .zip(elements)
            .map(|(q, e)| {
                matches!(e.kind, ElementKind::ParallelGateway)
                    && self
                        .model
                        .connections
                        .iter()
                        .any(|c| c.to == e.id && c.quantity.is_some())
                    && !q.is_empty()
            })
            .collect();
        for ((i, element), rng) in elements.iter().enumerate().zip(streams) {
            let outs: Vec<Out> = self
                .routes
                .get(i)
                .into_iter()
                .flatten()
                .filter_map(|r| {
                    let store = inputs.get(r.to)?.get(r.input)?.clone();
                    Some(Out {
                        port: Port {
                            store,
                            stats: Rc::clone(&stats),
                            assigner: Rc::clone(&assigner),
                            index: r.to,
                            places: places.get(r.to).cloned().flatten(),
                            // A join's second and later inputs bring copies (`spawn_parallel`),
                            // unless it is an assembly, whose inputs bring parts.
                            counts: r.input == 0 || assembly.get(r.to).copied().unwrap_or(false),
                        },
                        weight: r.weight,
                        condition: r.condition.clone(),
                        default: r.default,
                    })
                })
                .collect();
            let Some(ins) = inputs.get(i).cloned() else {
                continue;
            };
            let Some(input) = ins.first().cloned() else {
                continue;
            };
            let env = BlockEnv {
                ctx: ctx.clone(),
                stats: Rc::clone(&stats),
                assigner: Rc::clone(&assigner),
                me: i,
                places: places.get(i).cloned().flatten(),
                input,
                out: outs.first().map(|o| o.port.clone()),
                rng,
            };
            match element.kind.clone() {
                ElementKind::Source {
                    interarrival,
                    limit,
                    timed,
                    idle_outside,
                } => {
                    let rates = calendars.arrivals(interarrival, &timed, idle_outside);
                    spawn_source(
                        &mut sim,
                        env,
                        rates,
                        limit,
                        Rc::clone(&next_id),
                        self.first_arrival_at_zero,
                    );
                }
                ElementKind::Server {
                    capacity,
                    service,
                    breakdown,
                    available,
                    availability,
                    ..
                } => {
                    let shifts = calendars.availability(available, &availability);
                    let always = matches!(shifts.as_slice(), [(_, true)]);
                    if breakdown.is_some() || !always {
                        spawn_unit_server(&mut sim, env, capacity, service, breakdown, shifts);
                    } else {
                        spawn_server(&mut sim, env, capacity, service);
                    }
                }
                ElementKind::SharedTask {
                    service,
                    needs,
                    priority,
                } => {
                    let alternatives = alternatives(&self.model, &needs);
                    let request = Rc::new(Request {
                        alternatives,
                        priority,
                    });
                    spawn_shared(&mut sim, env, service, request, Rc::clone(&pools));
                }
                ElementKind::Delay { duration } => spawn_delay(&mut sim, env, duration),
                ElementKind::Sink => spawn_sink(&mut sim, env),
                ElementKind::ExclusiveGateway { .. } => spawn_exclusive(&mut sim, env, outs),
                ElementKind::ParallelGateway => {
                    let quantities = self.quantities.get(i).cloned().unwrap_or_default();
                    spawn_parallel(&mut sim, env, ins, quantities, outs, Rc::clone(&next_id))
                }
                ElementKind::Batch { size } => spawn_batch(&mut sim, env, size),
                ElementKind::Unbatch => spawn_unbatch(&mut sim, env),
            }
        }

        // Discard everything before the warm-up. With none, there is nothing to discard (and a
        // reset at time 0 would discard an arrival at time 0). A run that ends first (its stop
        // condition met) never resets: `reset_at` says whether, and when, it did.
        let reset_at = Rc::new(Cell::new((warmup <= 0.0).then_some(0.0)));
        if warmup > 0.0 {
            let (c, st, pl, at) = (
                ctx.clone(),
                Rc::clone(&stats),
                Rc::clone(&pools),
                Rc::clone(&reset_at),
            );
            sim.spawn(async move {
                c.hold(warmup).await;
                let now = c.now();
                st.iter().for_each(|s| s.borrow_mut().reset(now));
                pl.reset(now);
                at.set(Some(now));
            });
        }

        let stop = self.model.experiment.stop.as_ref().and_then(|s| {
            let i = elements.iter().position(|e| &e.id == s.element())?;
            Some((i, s.clone()))
        });
        Live {
            compiled: Cow::Borrowed(self),
            sim,
            stats,
            pools,
            feeds_buffer,
            warmup,
            duration,
            end,
            stop,
            stopped_at: None,
            reset_at,
            watching: Watching::new(0.0),
        }
    }

    /// The error for a replication that stopped early. Element processes are tagged with the
    /// element's index ([`tag`]).
    fn run_error(&self, stopped: Stopped) -> RunError {
        match stopped {
            Stopped::Cancelled { .. } => RunError::Cancelled,
            Stopped::TimeStopped { time, events, tag } => RunError::TimeStopped {
                time,
                events,
                element: self.tagged(tag),
            },
            Stopped::Unschedulable { time, tag } => RunError::TimeOutOfRange {
                time,
                element: self.tagged(tag),
            },
        }
    }

    /// The id and name of the element whose processes have tag `tag`.
    fn tagged(&self, tag: Option<u64>) -> Option<(stochcraft_model::ElementId, String)> {
        tag.and_then(|t| usize::try_from(t).ok())
            .and_then(|i| self.model.elements.get(i))
            .map(|e| (e.id.clone(), e.name.clone()))
    }
}

/// One replication run step by step, for watching it: advance it to a time or by one event, and
/// read each block's state at that moment. Run to its end it gives exactly the [`Report`]
/// [`Compiled::run`] gives with the same random stream.
pub struct Live<'a> {
    compiled: Cow<'a, Compiled>,
    sim: Sim,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    pools: Rc<Pools>,
    feeds_buffer: Vec<bool>,
    warmup: f64,
    duration: f64,
    end: f64,
    /// The model's stop condition: the index of the element it watches, and the condition.
    stop: Option<(usize, stochcraft_model::Stop)>,
    /// When the stop condition was met, if it has been (the run then ends there).
    stopped_at: Option<f64>,
    /// When the statistics were last reset: 0 with no warm-up, the warm-up's end once it has
    /// ended, `None` while it lasts (and for good, when the run stopped during it).
    reset_at: Rc<Cell<Option<f64>>>,
    /// How far the simulation has got, kept between advances so that a run in slices
    /// ([`Live::advance_by`]) counts the events at one time across them.
    watching: Watching,
}

/// A block's state at one moment of a [`Live`] run. Counts are since the warm-up ended (since
/// the start while it lasts).
#[derive(Debug, Clone, PartialEq)]
pub struct LiveState {
    pub id: stochcraft_model::ElementId,
    pub entered: u64,
    pub left: u64,
    pub lost: u64,
    /// Entities waiting in the block's input now (tasks with a resource or shared resources).
    pub waiting: f64,
    /// Units busy (a task with a resource), entities inside (other tasks), or a pool's units
    /// in use, now.
    pub busy: f64,
    /// Units the block or pool has now: a task's capacity, a pool's units in its current
    /// shift; none for blocks without units.
    pub units: Option<f64>,
    /// Units down for repair now (servers with breakdowns).
    pub down: f64,
}

impl Live<'_> {
    /// The simulated time now.
    pub fn now(&self) -> f64 {
        self.sim.now()
    }

    /// When the replication ends: its warm-up plus its duration, or the time its stop
    /// condition was met (see [`Live::stopped_at`]).
    pub fn end(&self) -> f64 {
        self.end
    }

    /// When the model's stop condition ([`stochcraft_model::Experiment::stop`]) was met, ending
    /// the run, if it has been.
    pub fn stopped_at(&self) -> Option<f64> {
        self.stopped_at
    }

    /// Whether the stop condition holds now.
    fn stop_met(&self) -> bool {
        let Some((i, stop)) = &self.stop else {
            return false;
        };
        let Some(s) = self.stats.get(*i) else {
            return false;
        };
        let s = s.borrow();
        let (entered, left) = (s.entered_before + s.entered, s.left_before + s.left);
        match stop {
            stochcraft_model::Stop::Count { count, .. } => left >= *count,
            stochcraft_model::Stop::Empty { .. } => entered > 0 && left >= entered,
        }
    }

    /// When the warm-up ends (statistics count from then).
    pub fn warmup(&self) -> f64 {
        self.warmup
    }

    /// Whether it has reached its end.
    pub fn is_done(&self) -> bool {
        self.sim.now() >= self.end
    }

    /// Runs every event up to time `until` (at most the end).
    pub fn advance_to(&mut self, until: f64) -> Result<(), RunError> {
        self.advance_watched(until, &|| false, u64::MAX).map(|_| ())
    }

    /// Runs at most `events` events towards the end; returns whether it has reached the end.
    /// A replication run this way, slice after slice, runs exactly the events of
    /// [`Compiled::run`] in the same order, whatever the slice size, and its stuck-time limit
    /// counts across slices.
    pub fn advance_by(&mut self, events: u64) -> Result<bool, RunError> {
        self.advance_watched(self.end, &|| false, events)
    }

    /// Runs the events up to time `until` (at most the end), at most `events` of them; returns
    /// whether it got to `until`. When it didn't, the next call carries on exactly where this
    /// one stopped: run slice after slice, the events and their order are those of
    /// [`Live::advance_to`], and the stuck-time limit counts across slices.
    pub fn advance_toward(&mut self, until: f64, events: u64) -> Result<bool, RunError> {
        self.advance_watched(until, &|| false, events)
    }

    /// When the next event happens, if it is no later than the end: the time [`Live::step`]
    /// goes to.
    pub fn next_event_time(&self) -> Option<f64> {
        self.sim.next_event_time().filter(|t| *t <= self.end)
    }

    /// The events run so far.
    pub fn events(&self) -> u64 {
        self.watching.events()
    }

    /// Runs the next event (and every other one at the same time); returns its time, or `None`
    /// when nothing more happens before the end (the run is then at its end).
    pub fn step(&mut self) -> Result<Option<f64>, RunError> {
        match self.sim.next_event_time() {
            Some(t) if t <= self.end => {
                self.advance_to(t)?;
                Ok(Some(t))
            }
            _ => {
                self.advance_to(self.end)?;
                Ok(None)
            }
        }
    }

    /// Runs the events up to `until` (at most the end), at most `budget` of them; returns
    /// whether it got to `until` (false: the budget ran out first, and the next call carries on
    /// exactly where this one stopped).
    fn advance_watched(
        &mut self,
        until: f64,
        cancelled: &dyn Fn() -> bool,
        budget: u64,
    ) -> Result<bool, RunError> {
        let until = if until.is_nan() {
            self.now()
        } else {
            until.min(self.end)
        };
        if until < self.now() {
            return Ok(true);
        }
        let watch = Watch {
            max_events_at_one_time: self.compiled.max_events_at_one_time,
            cancelled,
        };
        if self.stop.is_none() || self.stopped_at.is_some() {
            return self
                .sim
                .run_watched_slice(until, &watch, budget, &mut self.watching)
                .map_err(|stopped| self.compiled.run_error(stopped));
        }
        // With a stop condition: one instant at a time, checking it after each. The events are
        // the same, in the same order, as in one go. An instant the budget cuts short is
        // finished by the next call (its next event is still at that instant) before the
        // condition is checked.
        let mut left = budget;
        loop {
            let to = match self.sim.next_event_time() {
                Some(t) if t <= until => t,
                _ => until,
            };
            let before = self.watching.events();
            let finished = self
                .sim
                .run_watched_slice(to, &watch, left, &mut self.watching)
                .map_err(|stopped| self.compiled.run_error(stopped))?;
            if !finished {
                return Ok(false);
            }
            left = left.saturating_sub(self.watching.events().saturating_sub(before));
            if self.stop_met() {
                self.stopped_at = Some(self.sim.now());
                self.end = self.sim.now();
                return Ok(true);
            }
            if to >= until {
                return Ok(true);
            }
        }
    }

    /// Each element's state now, in model order, then each shared pool's.
    pub fn states(&self) -> Vec<LiveState> {
        let model = &self.compiled.model;
        let mut out: Vec<LiveState> = model
            .elements
            .iter()
            .zip(self.stats.iter())
            .map(|(e, s)| {
                let s = s.borrow();
                LiveState {
                    id: e.id.clone(),
                    entered: s.entered,
                    left: s.left,
                    lost: s.lost,
                    waiting: s.waiting.value(),
                    busy: s.busy.value(),
                    units: match &e.kind {
                        ElementKind::Server { capacity, .. } => Some(f64::from(*capacity)),
                        _ => None,
                    },
                    down: s.down.value(),
                }
            })
            .collect();
        out.extend(self.pools.states(&model.resources));
        out
    }

    /// The statistics so far, as a [`Report`] up to now (the full one once the run is done).
    pub fn report(&self) -> Report {
        let now = self.now().min(self.end);
        let model = &self.compiled.model;
        let mut elements: Vec<ElementReport> = model
            .elements
            .iter()
            .zip(self.stats.iter())
            .zip(&self.compiled.assigns)
            .zip(&self.feeds_buffer)
            .zip(&self.compiled.inputs)
            .map(|((((e, s), a), &blocks), &inputs)| {
                s.borrow().report(e, a, blocks, inputs > 1, now)
            })
            .collect();
        // Then each shared pool, after the elements.
        elements.extend(self.pools.reports(&model.resources, now));
        let after_warmup = self.reset_at.get();
        Report {
            warmup: self.warmup,
            duration: (now - self.warmup).max(0.0).min(self.duration),
            elements,
            stopped_at: self.stopped_at,
            measured_from: after_warmup.unwrap_or(0.0),
            measured_to: now,
            after_warmup: after_warmup.is_some(),
        }
    }
}

/// The tag of element `index`'s processes (see [`Sim::spawn_tagged`]).
fn tag(index: usize) -> u64 {
    u64::try_from(index).unwrap_or(u64::MAX)
}

/// Statistics a block keeps while the model runs. Every element balances: what entered either
/// left, was lost here, or is still inside.
pub(crate) struct BlockStats {
    /// Entities that entered (a source: created; a join: each entity once, see
    /// `spawn_parallel`).
    entered: u64,
    /// `entered` and `left` before the warm-up ended (for stop conditions, which count from 0).
    entered_before: u64,
    left_before: u64,
    /// Entities sent on (a sink: removed; a split or join: one per copy sent out).
    left: u64,
    /// Entities waiting in the block's input (servers only).
    waiting: TimeWeighted,
    /// Units busy (servers) or entities inside (delays).
    busy: TimeWeighted,
    /// Time from entering a server's input to the start of service. Only entities that started
    /// service count, so when the server is overloaded, those still waiting at the end (which
    /// have waited longest) are left out and the mean is biased low.
    wait: Tally,
    /// Time from creation to reaching a sink.
    time_in_system: Tally,
    /// The values assigned to each of the element's properties.
    properties: Vec<Tally>,
    /// Units (servers) or entities (delays) holding a finished entity while the next buffer is
    /// full.
    blocked: TimeWeighted,
    /// Time each entity that left waited for room in the next buffer (blocking after service).
    blocked_time: Tally,
    /// Entities lost here. A server with a finite buffer: arrivals from sources turned away
    /// because the buffer was full (they never entered). A source: its own arrivals lost that
    /// way (they entered and never left; the same arrival counts at the full buffer too). An
    /// exclusive gateway: entities with no connection to take (no condition held and there is
    /// no default), which leave the model there.
    lost: u64,
    /// Units failed or under repair (servers with breakdowns).
    down: TimeWeighted,
    /// A server's units in each state but starved, each unit in exactly one ([`UnitBook`]).
    states: UnitStates,
}

/// The state a server's unit is in: exactly one at every instant, the first that applies of
/// failed (or under repair), processing (an entity, including a job that runs on into a break
/// or resumes off shift after a repair), blocked (holding a finished entity), off shift (idle
/// because the calendar has the units unavailable) and starved (idle, on shift and up).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnitState {
    Failed,
    Processing,
    Blocked,
    OffShift,
    Starved,
}

/// How many of a server's units are in each state, over time. Starved is what is left of the
/// server's capacity.
pub(crate) struct UnitStates {
    failed: TimeWeighted,
    off_shift: TimeWeighted,
    blocked: TimeWeighted,
    processing: TimeWeighted,
}

impl UnitStates {
    fn new(t: f64) -> Self {
        Self {
            failed: TimeWeighted::new(t, 0.0),
            off_shift: TimeWeighted::new(t, 0.0),
            blocked: TimeWeighted::new(t, 0.0),
            processing: TimeWeighted::new(t, 0.0),
        }
    }

    fn reset(&mut self, t: f64) {
        self.failed.reset(t);
        self.off_shift.reset(t);
        self.blocked.reset(t);
        self.processing.reset(t);
    }

    fn get(&mut self, s: UnitState) -> Option<&mut TimeWeighted> {
        match s {
            UnitState::Failed => Some(&mut self.failed),
            UnitState::OffShift => Some(&mut self.off_shift),
            UnitState::Blocked => Some(&mut self.blocked),
            UnitState::Processing => Some(&mut self.processing),
            UnitState::Starved => None,
        }
    }
}

/// What a server's unit is doing, from which its [`UnitState`] follows.
#[derive(Debug, Clone, Copy, Default)]
struct Doing {
    /// Serving an entity, including one whose service a failure interrupted.
    processing: bool,
    /// Holding a finished entity, blocked.
    holding: bool,
    /// Failed or under repair.
    down: bool,
}

/// What each of a server's units is doing and whether they are on shift, kept in step with
/// the counts of units in each state ([`BlockStats`]'s `states`), which give the shares of time
/// in each state.
struct UnitBook {
    doing: RefCell<Vec<Doing>>,
    /// The units are on shift (their calendar's availability).
    open: Cell<bool>,
}

impl UnitBook {
    fn new(capacity: u32, open: bool) -> Rc<Self> {
        Rc::new(Self {
            doing: RefCell::new(vec![Doing::default(); capacity as usize]),
            open: Cell::new(open),
        })
    }

    fn state(&self, d: Doing) -> UnitState {
        if d.down {
            UnitState::Failed
        } else if d.processing {
            UnitState::Processing
        } else if d.holding {
            UnitState::Blocked
        } else if !self.open.get() {
            UnitState::OffShift
        } else {
            UnitState::Starved
        }
    }

    /// Moves a unit from state `from` to `to` in the counts.
    fn count(env: &BlockEnv, from: UnitState, to: UnitState) {
        if from == to {
            return;
        }
        let now = env.ctx.now();
        env.with_stats(|s| {
            for (st, delta) in [(from, -1.0), (to, 1.0)] {
                if let Some(tw) = s.states.get(st) {
                    let v = tw.value() + delta;
                    tw.set(now, v);
                }
            }
        });
    }

    /// Changes what unit `u` is doing (now), keeping the counts.
    fn change(&self, env: &BlockEnv, u: usize, f: impl FnOnce(&mut Doing)) {
        let (from, to) = {
            let mut doing = self.doing.borrow_mut();
            let Some(d) = doing.get_mut(u) else {
                return;
            };
            let from = self.state(*d);
            f(d);
            (from, self.state(*d))
        };
        Self::count(env, from, to);
    }

    /// The units go on or off shift (now).
    fn set_open(&self, env: &BlockEnv, open: bool) {
        let doing = self.doing.borrow().clone();
        let from: Vec<UnitState> = doing.iter().map(|d| self.state(*d)).collect();
        self.open.set(open);
        for (d, f) in doing.iter().zip(from) {
            Self::count(env, f, self.state(*d));
        }
    }
}

impl BlockStats {
    fn new(t: f64, properties: usize) -> Self {
        Self {
            entered: 0,
            entered_before: 0,
            left_before: 0,
            left: 0,
            waiting: TimeWeighted::new(t, 0.0),
            busy: TimeWeighted::new(t, 0.0),
            wait: Tally::new(),
            time_in_system: Tally::new(),
            properties: (0..properties).map(|_| Tally::new()).collect(),
            blocked: TimeWeighted::new(t, 0.0),
            blocked_time: Tally::new(),
            lost: 0,
            down: TimeWeighted::new(t, 0.0),
            states: UnitStates::new(t),
        }
    }

    fn reset(&mut self, t: f64) {
        self.entered_before += self.entered;
        self.left_before += self.left;
        self.entered = 0;
        self.left = 0;
        self.waiting.reset(t);
        self.busy.reset(t);
        self.wait = Tally::new();
        self.time_in_system = Tally::new();
        self.properties.iter_mut().for_each(|p| *p = Tally::new());
        self.blocked.reset(t);
        self.blocked_time = Tally::new();
        self.lost = 0;
        self.down.reset(t);
        self.states.reset(t);
    }

    /// Entities inside the element now: waiting in its input, being served or delayed (or
    /// collected into a batch), and held while blocked after service.
    fn inside(&self) -> u64 {
        let n = self.waiting.value() + self.busy.value() + self.blocked.value();
        // Sums of whole counts; `as` saturates (and a negative value cannot occur).
        n.round().max(0.0) as u64
    }

    /// The element's report; `blocks`: whether it sends to a finite buffer; `join`: whether it
    /// is a parallel gateway with several inputs (which holds entities until each input has
    /// one).
    pub(crate) fn report(
        &self,
        element: &Element,
        assigns: &[Assign],
        blocks: bool,
        join: bool,
        end: f64,
    ) -> ElementReport {
        let kind = &element.kind;
        let server = matches!(
            kind,
            ElementKind::Server { .. } | ElementKind::SharedTask { .. }
        );
        let finite = matches!(
            kind,
            ElementKind::Server {
                queue_capacity: Some(_),
                ..
            }
        );
        let holds = matches!(
            kind,
            ElementKind::Server { .. } | ElementKind::SharedTask { .. } | ElementKind::Delay { .. }
        );
        let offered = self.lost + self.entered;
        ElementReport {
            id: element.id.clone(),
            name: element.name.clone(),
            kind: kind.name(),
            entered: self.entered,
            left: self.left,
            mean_queue_length: if server {
                self.waiting.average(end)
            } else {
                None
            },
            mean_wait: if server { self.wait.mean() } else { None },
            utilisation: match kind {
                ElementKind::Server { capacity, .. } => {
                    self.busy.average(end).map(|b| b / f64::from(*capacity))
                }
                _ => None,
            },
            mean_contents: match kind {
                ElementKind::Server { .. }
                | ElementKind::SharedTask { .. }
                | ElementKind::Delay { .. }
                | ElementKind::Batch { .. } => self.busy.average(end),
                _ => None,
            },
            work_in_progress: match kind {
                ElementKind::Server { .. }
                | ElementKind::SharedTask { .. }
                | ElementKind::Delay { .. }
                | ElementKind::Batch { .. } => Some(self.inside()),
                ElementKind::ParallelGateway if join => Some(self.inside()),
                _ => None,
            },
            mean_time_in_system: if matches!(kind, ElementKind::Sink) {
                self.time_in_system.mean()
            } else {
                None
            },
            mean_blocked_time: if blocks && holds {
                self.blocked_time.mean()
            } else {
                None
            },
            blocking_probability: match kind {
                ElementKind::Server { capacity, .. } if blocks => {
                    self.blocked.average(end).map(|b| b / f64::from(*capacity))
                }
                _ => None,
            },
            availability: match kind {
                ElementKind::Server {
                    capacity,
                    breakdown: Some(_),
                    ..
                } => self
                    .down
                    .average(end)
                    .map(|d| 1.0 - d / f64::from(*capacity)),
                _ => None,
            },
            // Only for servers that follow a calendar, so others report as before.
            time_off_shift: match kind {
                ElementKind::Server {
                    capacity,
                    available,
                    availability,
                    ..
                } if !*available || !availability.is_empty() => self
                    .states
                    .off_shift
                    .average(end)
                    .map(|o| o / f64::from(*capacity)),
                _ => None,
            },
            // Only where losses can happen, so models without them report as before.
            lost: match kind {
                ElementKind::Server {
                    queue_capacity: Some(_),
                    ..
                }
                | ElementKind::ExclusiveGateway { .. } => Some(self.lost),
                ElementKind::Source { .. } if blocks => Some(self.lost),
                _ => None,
            },
            loss_probability: if finite && offered > 0 {
                Some(self.lost as f64 / offered as f64)
            } else {
                None
            },
            properties: assigns
                .iter()
                .zip(&self.properties)
                .map(|(a, t)| (a.name.clone(), t.mean()))
                .collect(),
        }
    }
}

/// Where a block sends entities: the next block's input, with its statistics.
#[derive(Clone)]
struct Port {
    store: Store<Entity>,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    assigner: Rc<Assigner>,
    index: usize,
    /// The next element's places, when its buffer is finite.
    places: Option<Rc<Places>>,
    /// Whether an entity sent here counts as entering the next element: always, except on a
    /// join's second and later inputs, whose entities are copies merged into the one on its
    /// first input.
    counts: bool,
}

/// A finite buffer's places (see [`Compiled::run_for`]), with the one each entity there holds.
struct Places {
    pool: Resource,
    held: RefCell<BTreeMap<u64, ResourceGuard>>,
}

impl Places {
    /// Takes the place `guard` for entity `id`, as it gets in.
    fn enter(&self, id: u64, guard: ResourceGuard) {
        self.held.borrow_mut().insert(id, guard);
    }

    /// Gives back the place entity `id` holds, as it leaves.
    fn leave(&self, id: u64) {
        drop(self.held.borrow_mut().remove(&id));
    }
}

impl Port {
    /// Waits for a place in the next element's finite buffer, if it has one, and takes it for
    /// `e`.
    async fn make_room(&self, e: &Entity) {
        if let Some(p) = &self.places {
            let guard = p.pool.acquire().await;
            p.enter(e.id, guard);
        }
    }

    /// Sends `e`, first waiting for room in a finite buffer.
    async fn deliver(&self, ctx: &Ctx, e: Entity) {
        self.make_room(&e).await;
        self.send(ctx.now(), e);
    }

    /// Sends a new arrival if there is room (a place freed at this instant counts) and returns
    /// `true`; otherwise the arrival is lost, counted as lost by the next element, and this
    /// returns `false`.
    async fn offer(&self, ctx: &Ctx, e: Entity) -> bool {
        if let Some(p) = &self.places {
            match p.pool.acquire_within(0.0).await {
                Ok(guard) => p.enter(e.id, guard),
                Err(_) => {
                    if let Some(s) = self.stats.get(self.index) {
                        s.borrow_mut().lost += 1;
                    }
                    return false;
                }
            }
        }
        self.send(ctx.now(), e);
        true
    }

    fn send(&self, now: f64, mut e: Entity) {
        e.entered = now;
        self.assigner.enter(self.index, &mut e, &self.stats);
        if let Some(s) = self.stats.get(self.index) {
            let mut s = s.borrow_mut();
            if self.counts {
                s.entered += 1;
            }
            let w = s.waiting.value() + 1.0;
            s.waiting.set(now, w);
        }
        self.store.put(e);
    }
}

/// One of a gateway's outgoing connections, ready to route.
struct Out {
    port: Port,
    weight: f64,
    condition: Option<stochcraft_model::expr::Expr>,
    default: bool,
}

struct BlockEnv {
    ctx: Ctx,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    assigner: Rc<Assigner>,
    me: usize,
    /// This element's places, when its buffer is finite.
    places: Option<Rc<Places>>,
    input: Store<Entity>,
    out: Option<Port>,
    rng: Rng,
}

impl BlockEnv {
    fn with_stats(&self, f: impl FnOnce(&mut BlockStats)) {
        if let Some(s) = self.stats.get(self.me) {
            f(&mut s.borrow_mut());
        }
    }

    fn adjust(&self, field: fn(&mut BlockStats) -> &mut TimeWeighted, delta: f64) {
        let now = self.ctx.now();
        self.with_stats(|s| {
            let tw = field(s);
            let v = tw.value() + delta;
            tw.set(now, v);
        });
    }
}

/// The time between arrivals over a run: from each time on, until the next, the distribution
/// that applies (`None`: no arrivals).
type Rates = Vec<(f64, Option<Dist>)>;

fn spawn_source(
    sim: &mut Sim,
    mut env: BlockEnv,
    rates: Rates,
    limit: Option<u64>,
    next_id: Rc<RefCell<u64>>,
    first_at_zero: bool,
) {
    sim.spawn_tagged(tag(env.me), async move {
        let mut made = 0u64;
        // The period of `rates` now under way.
        let mut k = 0usize;
        while limit.is_none_or(|l| made < l) {
            let arrive_now =
                first_at_zero && made == 0 && rates.first().is_some_and(|(_, d)| d.is_some());
            if !arrive_now {
                // When the rate changes before the next arrival, that arrival is dropped and the
                // next is drawn from the new rate at the change (BPSim: a calendar value takes
                // effect at once).
                loop {
                    let now = env.ctx.now();
                    let change = rates.get(k + 1).map(|(t, _)| *t);
                    match rates.get(k).and_then(|(_, d)| d.as_ref()) {
                        None => match change {
                            Some(c) => {
                                env.ctx.hold(c - now).await;
                                k += 1;
                            }
                            None => return,
                        },
                        Some(d) => {
                            let dt = d.sample(&mut env.rng);
                            match change {
                                Some(c) if now + dt > c => {
                                    env.ctx.hold(c - now).await;
                                    k += 1;
                                }
                                _ => {
                                    env.ctx.hold(dt).await;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            let now = env.ctx.now();
            let id = {
                let mut n = next_id.borrow_mut();
                let id = *n;
                *n += 1;
                id
            };
            made += 1;
            env.with_stats(|s| s.entered += 1);
            let mut entity = Entity {
                id,
                created: now,
                entered: now,
                properties: Vec::new(),
                members: Vec::new(),
            };
            // A source sets its properties on the entities it creates.
            env.assigner.enter(env.me, &mut entity, &env.stats);
            // It leaves, unless the next element's finite buffer is full and it is lost.
            let sent = match &env.out {
                Some(out) => out.offer(&env.ctx, entity).await,
                None => true,
            };
            env.with_stats(|s| {
                if sent {
                    s.left += 1;
                } else {
                    s.lost += 1;
                }
            });
        }
    });
}

/// The model's calendars over one run: when each is active, in model time.
struct Calendars {
    periods: BTreeMap<ElementId, Vec<(f64, f64)>>,
    end: f64,
}

impl Calendars {
    fn new(model: &Model, end: f64) -> Self {
        let origin = model
            .experiment
            .start
            .or_else(|| Calendar::default_origin(&model.calendars))
            .unwrap_or(DateTime(0));
        let unit = model.experiment.time_unit;
        Self {
            periods: model
                .calendars
                .iter()
                .map(|c| (c.id.clone(), c.periods(origin, unit, end)))
                .collect(),
            end,
        }
    }

    fn timeline<T>(&self, timed: &[Timed<T>]) -> Vec<(f64, Option<usize>)> {
        if timed.is_empty() {
            return vec![(0.0, None)];
        }
        let periods = |c: &ElementId| self.periods.get(c).cloned().unwrap_or_default();
        calendar::timeline(timed, &periods, self.end)
    }

    /// A source's time between arrivals over the run.
    fn arrivals(&self, default: Dist, timed: &[Timed<Dist>], idle_outside: bool) -> Rates {
        self.timeline(timed)
            .into_iter()
            .map(|(t, i)| {
                let d = match i {
                    Some(i) => timed.get(i).map(|x| x.value.clone()),
                    None => (!idle_outside).then(|| default.clone()),
                };
                (t, d)
            })
            .collect()
    }

    /// Whether a server's units are on shift over the run: from each time on, until the next.
    fn availability(&self, available: bool, timed: &[Timed<bool>]) -> Vec<(f64, bool)> {
        let mut out: Vec<(f64, bool)> = Vec::new();
        for (t, i) in self.timeline(timed) {
            let open = i.and_then(|i| timed.get(i)).map_or(available, |x| x.value);
            if out.last().is_none_or(|(_, o)| *o != open) {
                out.push((t, open));
            }
        }
        out
    }

    /// A pool's units over the run: its shift's quantity while it is available, else none.
    fn units(&self, r: &ResourcePool) -> Vec<(f64, u32)> {
        let q = self.timeline(&r.shifts);
        let a = self.timeline(&r.availability);
        let mut cuts: Vec<f64> = q.iter().chain(&a).map(|(t, _)| *t).collect();
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        let at = |line: &[(f64, Option<usize>)], t: f64| {
            line.iter()
                .take_while(|(s, _)| *s <= t)
                .last()
                .and_then(|(_, i)| *i)
        };
        let mut out: Vec<(f64, u32)> = Vec::new();
        for t in cuts {
            let quantity = at(&q, t)
                .and_then(|i| r.shifts.get(i))
                .map_or(r.quantity, |s| s.value);
            let available = at(&a, t)
                .and_then(|i| r.availability.get(i))
                .map_or(r.available, |s| s.value);
            let units = if available { quantity } else { 0 };
            if out.last().is_none_or(|(_, u)| *u != units) {
                out.push((t, units));
            }
        }
        out
    }
}

/// Changes pool `pool`'s units at each time of `changes` (the first, at 0, is already set).
fn spawn_shifts(sim: &mut Sim, pools: Rc<Pools>, pool: usize, changes: Vec<(f64, u32)>) {
    let ctx = pools.ctx.clone();
    sim.spawn(async move {
        for (t, units) in changes.into_iter().skip(1) {
            let now = ctx.now();
            ctx.hold(t - now).await;
            pools.set_units(pool, units);
        }
    });
}

fn spawn_server(sim: &mut Sim, env: BlockEnv, capacity: u32, service: Dist) {
    let me = tag(env.me);
    let env = Rc::new(RefCell::new(env));
    let book = UnitBook::new(capacity, true);
    for u in 0..capacity as usize {
        let env = Rc::clone(&env);
        let book = Rc::clone(&book);
        let service = service.clone();
        sim.spawn_tagged(me, async move {
            loop {
                let (input, ctx) = {
                    let e = env.borrow();
                    (e.input.clone(), e.ctx.clone())
                };
                let entity = input.get().await;
                let duration = {
                    let mut e = env.borrow_mut();
                    let now = e.ctx.now();
                    e.with_stats(|s| s.wait.add(now - entity.entered));
                    e.adjust(|s| &mut s.waiting, -1.0);
                    e.adjust(|s| &mut s.busy, 1.0);
                    book.change(&e, u, |d| d.processing = true);
                    service.sample(&mut e.rng)
                };
                ctx.hold(duration).await;
                {
                    let e = env.borrow();
                    e.adjust(|s| &mut s.busy, -1.0);
                    book.change(&e, u, |d| {
                        d.processing = false;
                        d.holding = true;
                    });
                }
                move_on(&env, &ctx, entity).await;
                book.change(&env.borrow(), u, |d| d.holding = false);
            }
        });
    }
}

/// One unit of a server whose units fail or follow a calendar.
#[derive(Default)]
struct Unit {
    /// Serving an entity (possibly interrupted by a failure), or blocked after service.
    busy: bool,
    /// Failed or under repair.
    down: bool,
    /// The job process while it is in service (so a failure can interrupt it).
    serving: Option<u64>,
    /// A failure is due and waits for the job to finish (non-interruptible).
    due: bool,
}

/// A server whose units fail and are repaired ([`Breakdown`]), or follow a calendar (`shifts`:
/// from each time on, whether they are on shift), or both. One process hands each waiting
/// entity, first come first served, to the first unit that is up, idle and on shift; a process
/// per unit makes it fail and repairs it; and one process takes the units on and off shift.
///
/// A unit checks the calendar only as it takes an entity: a job under way when the shift ends
/// finishes, and one a failure interrupted resumes after the repair, shift or not. Failures
/// come in elapsed time, and repairs go on, through the breaks.
fn spawn_unit_server(
    sim: &mut Sim,
    mut env: BlockEnv,
    capacity: u32,
    service: Dist,
    breakdown: Option<Breakdown>,
    shifts: Vec<(f64, bool)>,
) {
    let ctx = env.ctx.clone();
    let me = tag(env.me);
    // The failures draw from their own stream, so they do not change the service times.
    let mut fail_rng = breakdown.as_ref().map(|_| env.rng.fork());
    let units: Rc<RefCell<Vec<Unit>>> = Rc::new(RefCell::new(
        (0..capacity).map(|_| Unit::default()).collect(),
    ));
    let book = UnitBook::new(capacity, true);
    // A unit came free, was repaired, or came on shift.
    let changed = Signal::new(&ctx);
    // Each unit's repair, which an interrupted job waits for; and its job ending, which a due
    // failure waits for.
    let up: Rc<Vec<Gate>> = Rc::new((0..capacity).map(|_| Gate::new(&ctx, true)).collect());
    let done: Rc<Vec<Signal>> = Rc::new((0..capacity).map(|_| Signal::new(&ctx)).collect());
    let env = Rc::new(RefCell::new(env));
    // Off shift from the start: every unit is (counted from time 0).
    if shifts.first().is_some_and(|(_, open)| !*open) {
        book.set_open(&env.borrow(), false);
    }

    if shifts.len() > 1 {
        let (c, book, changed, env) = (
            ctx.clone(),
            Rc::clone(&book),
            changed.clone(),
            Rc::clone(&env),
        );
        sim.spawn_tagged(me, async move {
            for (t, open) in shifts.into_iter().skip(1) {
                let now = c.now();
                c.hold(t - now).await;
                book.set_open(&env.borrow(), open);
                if open {
                    changed.notify_all();
                }
            }
        });
    }

    for u in 0..capacity as usize {
        let (Some(b), Some(fail_rng)) = (breakdown.clone(), fail_rng.as_mut()) else {
            break;
        };
        let (c, units, changed, up, done, env, book) = (
            ctx.clone(),
            Rc::clone(&units),
            changed.clone(),
            Rc::clone(&up),
            Rc::clone(&done),
            Rc::clone(&env),
            Rc::clone(&book),
        );
        let mut rng = fail_rng.fork();
        sim.spawn_tagged(me, async move {
            loop {
                c.hold(b.time_to_failure.sample(&mut rng)).await;
                let busy = units.borrow().get(u).is_some_and(|x| x.busy);
                if busy && !b.interruptible {
                    if let Some(x) = units.borrow_mut().get_mut(u) {
                        x.due = true;
                    }
                    if let Some(d) = done.get(u) {
                        d.wait().await;
                    }
                }
                let serving = units.borrow_mut().get_mut(u).and_then(|x| {
                    x.down = true;
                    x.due = false;
                    x.serving
                });
                if let Some(g) = up.get(u) {
                    g.close();
                }
                {
                    let e = env.borrow();
                    e.adjust(|s| &mut s.down, 1.0);
                    book.change(&e, u, |d| d.down = true);
                }
                if let Some(pid) = serving {
                    c.interrupt(pid, 0);
                }
                c.hold(b.repair.sample(&mut rng)).await;
                if let Some(x) = units.borrow_mut().get_mut(u) {
                    x.down = false;
                }
                {
                    let e = env.borrow();
                    e.adjust(|s| &mut s.down, -1.0);
                    book.change(&e, u, |d| d.down = false);
                }
                if let Some(g) = up.get(u) {
                    g.open();
                }
                changed.notify_all();
            }
        });
    }

    sim.spawn_tagged(me, async move {
        loop {
            let input = env.borrow().input.clone();
            let entity = input.get().await;
            // The first unit that is up, idle and on shift; else wait for one.
            let unit = loop {
                let free = book
                    .open
                    .get()
                    .then(|| {
                        units
                            .borrow()
                            .iter()
                            .position(|x| !x.busy && !x.down && !x.due)
                    })
                    .flatten();
                match free {
                    Some(u) => break u,
                    None => changed.wait().await,
                }
            };
            let duration = {
                let mut e = env.borrow_mut();
                let now = e.ctx.now();
                e.with_stats(|s| s.wait.add(now - entity.entered));
                e.adjust(|s| &mut s.waiting, -1.0);
                e.adjust(|s| &mut s.busy, 1.0);
                book.change(&e, unit, |d| d.processing = true);
                service.sample(&mut e.rng)
            };
            if let Some(x) = units.borrow_mut().get_mut(unit) {
                x.busy = true;
            }
            let (c, units2, changed2, up2, done2, env2, book2) = (
                ctx.clone(),
                Rc::clone(&units),
                changed.clone(),
                Rc::clone(&up),
                Rc::clone(&done),
                Rc::clone(&env),
                Rc::clone(&book),
            );
            let interruptible = breakdown.as_ref().is_some_and(|b| b.interruptible);
            let pid = ctx.spawn(async move {
                let mut left = duration;
                loop {
                    if !interruptible {
                        c.hold(left).await;
                        break;
                    }
                    match c.hold_interruptible(left).await {
                        Ok(()) => break,
                        Err(i) => {
                            // Failed mid-job: wait for the repair, then carry on.
                            left = i.remaining;
                            if let Some(x) = units2.borrow_mut().get_mut(unit) {
                                x.serving = None;
                            }
                            if let Some(g) = up2.get(unit) {
                                g.wait().await;
                            }
                            if let (Some(x), Some(pid)) =
                                (units2.borrow_mut().get_mut(unit), c.current())
                            {
                                x.serving = Some(pid);
                            }
                        }
                    }
                }
                if let Some(x) = units2.borrow_mut().get_mut(unit) {
                    x.serving = None;
                }
                {
                    let e = env2.borrow();
                    e.adjust(|s| &mut s.busy, -1.0);
                    book2.change(&e, unit, |d| {
                        d.processing = false;
                        d.holding = true;
                    });
                }
                move_on(&env2, &c, entity).await;
                book2.change(&env2.borrow(), unit, |d| d.holding = false);
                if let Some(x) = units2.borrow_mut().get_mut(unit) {
                    x.busy = false;
                }
                if let Some(d) = done2.get(unit) {
                    d.notify_all();
                }
                changed2.notify_all();
            });
            if interruptible && let Some(x) = units.borrow_mut().get_mut(unit) {
                x.serving = Some(pid);
            }
        }
    });
}

/// An entity whose service or delay is over leaves for the next element. Into a full finite
/// buffer it waits here first, blocking (blocking after service): the element keeps it, and its
/// place here, until there is room, and the time is recorded.
async fn move_on(env: &Rc<RefCell<BlockEnv>>, ctx: &Ctx, entity: Entity) {
    let out = env.borrow().out.clone();
    if let Some(out) = out.as_ref().filter(|o| o.places.is_some()) {
        let start = ctx.now();
        env.borrow().adjust(|s| &mut s.blocked, 1.0);
        out.make_room(&entity).await;
        let now = ctx.now();
        let e = env.borrow();
        e.adjust(|s| &mut s.blocked, -1.0);
        e.with_stats(|s| s.blocked_time.add(now - start));
    }
    {
        let e = env.borrow();
        e.with_stats(|s| s.left += 1);
        if let Some(p) = &e.places {
            p.leave(entity.id);
        }
    }
    if let Some(out) = out {
        out.send(ctx.now(), entity);
    }
}

/// One way to meet a shared task's need: `quantity` units from any of `pools` (indices into the
/// model's resources), taken in that order.
struct Alternative {
    pools: Vec<usize>,
    quantity: u32,
}

/// What a shared task asks of the pools for each entity.
struct Request {
    /// Tried in order; the first that can be met now is taken.
    alternatives: Vec<Alternative>,
    priority: f64,
}

/// The pools each of `needs` can draw on.
fn alternatives(model: &Model, needs: &[Need]) -> Vec<Alternative> {
    needs
        .iter()
        .map(|need| {
            let pools = model
                .resources
                .iter()
                .enumerate()
                .filter(|(_, r)| match need {
                    Need::Resource { id, .. } => &r.id == id,
                    Need::Roles { roles, .. } => r.has_roles(roles),
                })
                .map(|(i, _)| i)
                .collect();
            Alternative {
                pools,
                quantity: need.quantity(),
            }
        })
        .collect()
}

/// Units taken from the pools: (pool, how many).
type Taken = Vec<(usize, u32)>;

/// An entity waiting for units.
struct Waiting {
    request: Rc<Request>,
    /// Arrival order, for ties of priority.
    seq: u64,
    /// Where the units are handed over.
    grant: Store<Taken>,
}

/// The shared resource pools of one run: free units, the entities waiting for some, and each
/// pool's busy units over time.
struct Pools {
    ctx: Ctx,
    /// Units free now; below 0 while a pool's shift has ended with more units still busy
    /// finishing their jobs than it now has.
    free: RefCell<Vec<i64>>,
    /// Units each pool has now (its shift's quantity while available).
    units: RefCell<Vec<u32>>,
    busy: RefCell<Vec<TimeWeighted>>,
    /// Units each pool has, over time, for utilisation: its shift's, or more while units whose
    /// shift has ended are finishing their jobs.
    capacity: RefCell<Vec<TimeWeighted>>,
    /// How many times each pool's units were taken.
    taken: RefCell<Vec<u64>>,
    /// Highest priority first, then by arrival.
    waiting: RefCell<Vec<Waiting>>,
    seq: RefCell<u64>,
}

impl Pools {
    fn new(ctx: &Ctx, units: Vec<u32>) -> Self {
        Self {
            ctx: ctx.clone(),
            free: RefCell::new(units.iter().map(|&u| i64::from(u)).collect()),
            busy: RefCell::new(units.iter().map(|_| TimeWeighted::new(0.0, 0.0)).collect()),
            capacity: RefCell::new(
                units
                    .iter()
                    .map(|&u| TimeWeighted::new(0.0, f64::from(u)))
                    .collect(),
            ),
            taken: RefCell::new(vec![0; units.len()]),
            units: RefCell::new(units),
            waiting: RefCell::new(Vec::new()),
            seq: RefCell::new(0),
        }
    }

    /// Sets pool `p`'s units (a shift change). Units taken away that are busy finish their job
    /// first; units added go to waiting entities at once.
    fn set_units(&self, p: usize, n: u32) {
        let now = self.ctx.now();
        let more = {
            let mut units = self.units.borrow_mut();
            let mut free = self.free.borrow_mut();
            let (Some(u), Some(f)) = (units.get_mut(p), free.get_mut(p)) else {
                return;
            };
            *f += i64::from(n) - i64::from(*u);
            let more = n > *u;
            *u = n;
            let present = (i64::from(n) - (*f).min(0)) as f64;
            if let Some(c) = self.capacity.borrow_mut().get_mut(p) {
                c.set(now, present);
            }
            more
        };
        if more {
            self.dispatch();
        }
    }

    /// Takes the first of `request`'s alternatives that the free units can meet, if any.
    fn try_take(&self, request: &Request) -> Option<Taken> {
        let mut free = self.free.borrow_mut();
        let alt = request.alternatives.iter().find(|a| {
            let have: i64 = a
                .pools
                .iter()
                .map(|&p| free.get(p).copied().unwrap_or(0).max(0))
                .sum();
            a.quantity > 0 && have >= i64::from(a.quantity)
        })?;
        let mut left = alt.quantity;
        let mut taken = Vec::new();
        for &p in &alt.pools {
            if left == 0 {
                break;
            }
            if let Some(f) = free.get_mut(p) {
                let n = (*f).clamp(0, i64::from(left)) as u32;
                if n > 0 {
                    *f -= i64::from(n);
                    left -= n;
                    taken.push((p, n));
                }
            }
        }
        drop(free);
        self.record(&taken, true);
        Some(taken)
    }

    /// Updates the busy units of the pools in `taken`, as they are taken or given back.
    fn record(&self, taken: &Taken, take: bool) {
        let now = self.ctx.now();
        let free = self.free.borrow();
        let units = self.units.borrow();
        let mut busy = self.busy.borrow_mut();
        let mut count = self.taken.borrow_mut();
        for &(p, _) in taken {
            if let (Some(b), Some(&q), Some(&f)) = (busy.get_mut(p), units.get(p), free.get(p)) {
                b.set(now, (i64::from(q) - f) as f64);
                // Units still finishing a job after their shift are there until it ends.
                let mut capacity = self.capacity.borrow_mut();
                if let Some(c) = capacity.get_mut(p)
                    && (f < 0 || c.value() > f64::from(q))
                {
                    c.set(now, (i64::from(q) - f.min(0)) as f64);
                }
            }
            if take && let Some(c) = count.get_mut(p) {
                *c += 1;
            }
        }
    }

    /// Waits until units for `request` are free, then takes them.
    async fn acquire(&self, request: &Rc<Request>) -> Taken {
        if let Some(taken) = self.try_take(request) {
            return taken;
        }
        let grant = Store::new(&self.ctx);
        let seq = {
            let mut s = self.seq.borrow_mut();
            *s += 1;
            *s
        };
        {
            let mut w = self.waiting.borrow_mut();
            // After every waiter of the same or higher priority.
            let at = w
                .iter()
                .position(|x| x.request.priority < request.priority)
                .unwrap_or(w.len());
            w.insert(
                at,
                Waiting {
                    request: Rc::clone(request),
                    seq,
                    grant: grant.clone(),
                },
            );
        }
        grant.get().await
    }

    /// Gives back `taken`, then hands free units to waiting entities: in priority order, each
    /// that can be served now (one that cannot does not hold back the others).
    fn release(&self, taken: Taken) {
        {
            let mut free = self.free.borrow_mut();
            for &(p, n) in &taken {
                if let Some(f) = free.get_mut(p) {
                    *f += i64::from(n);
                }
            }
        }
        self.record(&taken, false);
        self.dispatch();
    }

    /// Hands free units to waiting entities: in priority order, each that can be served now
    /// (one that cannot does not hold back the others).
    fn dispatch(&self) {
        let mut i = 0;
        loop {
            let next = {
                let w = self.waiting.borrow();
                match w.get(i) {
                    Some(x) => Rc::clone(&x.request),
                    None => break,
                }
            };
            match self.try_take(&next) {
                Some(t) => {
                    let x = self.waiting.borrow_mut().remove(i);
                    let _ = x.seq;
                    x.grant.put(t);
                }
                None => i += 1,
            }
        }
    }

    /// Discards the statistics so far (the end of the warm-up).
    fn reset(&self, t: f64) {
        self.busy.borrow_mut().iter_mut().for_each(|b| b.reset(t));
        self.capacity
            .borrow_mut()
            .iter_mut()
            .for_each(|b| b.reset(t));
        self.taken.borrow_mut().iter_mut().for_each(|c| *c = 0);
    }

    /// Each pool's report: how often its units were taken, mean units busy and utilisation.
    /// Each pool's state now.
    fn states(&self, resources: &[ResourcePool]) -> Vec<LiveState> {
        let busy = self.busy.borrow();
        let units = self.units.borrow();
        let taken = self.taken.borrow();
        resources
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let n = taken.get(i).copied().unwrap_or(0);
                LiveState {
                    id: r.id.clone(),
                    entered: n,
                    left: n,
                    lost: 0,
                    waiting: 0.0,
                    busy: busy.get(i).map_or(0.0, TimeWeighted::value),
                    units: units.get(i).map(|u| f64::from(*u)),
                    down: 0.0,
                }
            })
            .collect()
    }

    fn reports(&self, resources: &[ResourcePool], end: f64) -> Vec<ElementReport> {
        let busy = self.busy.borrow();
        let capacity = self.capacity.borrow();
        let taken = self.taken.borrow();
        resources
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mean = busy.get(i).and_then(|b| b.average(end));
                // Busy units over the units the pool had, both averaged over time.
                let constant = r.shifts.is_empty() && r.availability.is_empty();
                let units = if constant {
                    Some(f64::from(if r.available { r.quantity } else { 0 }))
                } else {
                    capacity.get(i).and_then(|c| c.average(end))
                };
                let n = taken.get(i).copied().unwrap_or(0);
                ElementReport {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    kind: "resource",
                    entered: n,
                    left: n,
                    mean_queue_length: None,
                    mean_wait: None,
                    utilisation: match (mean, units) {
                        (Some(b), Some(u)) if u > 0.0 => Some(b / u),
                        (Some(_), _) => Some(0.0),
                        _ => None,
                    },
                    mean_contents: mean,
                    work_in_progress: None,
                    mean_time_in_system: None,
                    mean_blocked_time: None,
                    blocking_probability: None,
                    availability: None,
                    time_off_shift: None,
                    lost: None,
                    loss_probability: None,
                    properties: Vec::new(),
                }
            })
            .collect()
    }
}

/// A task whose units come from shared pools: each entity waits in this task's queue for the
/// units its request needs, is served, and gives them back once it has left (holding them while
/// blocked after service).
fn spawn_shared(
    sim: &mut Sim,
    env: BlockEnv,
    service: Dist,
    request: Rc<Request>,
    pools: Rc<Pools>,
) {
    let ctx = env.ctx.clone();
    let me = tag(env.me);
    let env = Rc::new(RefCell::new(env));
    // Each entity's service is a process of its own, started from this one, so tagged alike.
    sim.spawn_tagged(me, async move {
        loop {
            let input = env.borrow().input.clone();
            let entity = input.get().await;
            let (c, env2, req, pl) = (
                ctx.clone(),
                Rc::clone(&env),
                Rc::clone(&request),
                Rc::clone(&pools),
            );
            let service = service.clone();
            ctx.spawn(async move {
                let taken = pl.acquire(&req).await;
                let duration = {
                    let mut e = env2.borrow_mut();
                    let now = c.now();
                    e.with_stats(|s| s.wait.add(now - entity.entered));
                    e.adjust(|s| &mut s.waiting, -1.0);
                    e.adjust(|s| &mut s.busy, 1.0);
                    service.sample(&mut e.rng)
                };
                c.hold(duration).await;
                env2.borrow().adjust(|s| &mut s.busy, -1.0);
                move_on(&env2, &c, entity).await;
                pl.release(taken);
            });
        }
    });
}

fn spawn_delay(sim: &mut Sim, env: BlockEnv, duration: Dist) {
    let me = tag(env.me);
    let env = Rc::new(RefCell::new(env));
    let mut sim_ctx = None;
    {
        let e = env.borrow();
        sim_ctx.get_or_insert(e.ctx.clone());
    }
    let Some(ctx) = sim_ctx else { return };
    sim.spawn_tagged(me, async move {
        loop {
            let input = env.borrow().input.clone();
            let entity = input.get().await;
            let dt = {
                let mut e = env.borrow_mut();
                e.adjust(|s| &mut s.waiting, -1.0);
                e.adjust(|s| &mut s.busy, 1.0);
                duration.sample(&mut e.rng)
            };
            let (c, env2) = (ctx.clone(), Rc::clone(&env));
            ctx.spawn(async move {
                c.hold(dt).await;
                env2.borrow().adjust(|s| &mut s.busy, -1.0);
                move_on(&env2, &c, entity).await;
            });
        }
    });
}

fn spawn_sink(sim: &mut Sim, env: BlockEnv) {
    sim.spawn_tagged(tag(env.me), async move {
        loop {
            let entity = env.input.get().await;
            let now = env.ctx.now();
            env.adjust(|s| &mut s.waiting, -1.0);
            env.with_stats(|s| {
                s.left += 1;
                s.time_in_system.add(now - entity.created);
            });
        }
    });
}

/// Which of `outs` an entity takes: the first whose condition holds, else the default, if any
/// connection has a condition; otherwise one drawn by weight. With one connection, nothing is
/// drawn, so a gateway with a single way out never changes the random numbers.
fn choose(outs: &[Out], e: &Entity, assigner: &Assigner, rng: &mut Rng) -> Option<usize> {
    if outs.iter().any(|o| o.condition.is_some()) {
        let read = |name: &str| assigner.read(e, name);
        return outs
            .iter()
            .position(|o| {
                !o.default
                    && o.condition
                        .as_ref()
                        .is_some_and(|c| c.eval(&read).boolean())
            })
            .or_else(|| outs.iter().position(|o| o.default));
    }
    if outs.len() <= 1 {
        return if outs.is_empty() { None } else { Some(0) };
    }
    let total: f64 = outs.iter().map(|o| o.weight).sum();
    let mut u = rng.uniform() * total;
    for (i, o) in outs.iter().enumerate() {
        if u < o.weight {
            return Some(i);
        }
        u -= o.weight;
    }
    // Rounding: the last connection with any weight.
    outs.iter().rposition(|o| o.weight > 0.0)
}

fn spawn_exclusive(sim: &mut Sim, mut env: BlockEnv, outs: Vec<Out>) {
    sim.spawn_tagged(tag(env.me), async move {
        loop {
            let entity = env.input.get().await;
            env.adjust(|s| &mut s.waiting, -1.0);
            let to = choose(&outs, &entity, &env.assigner, &mut env.rng);
            // With no route (no condition holds and there is no default), the entity leaves
            // the model here, and is counted as lost.
            // A full finite buffer holds the gateway, and everything behind it, up.
            match to.and_then(|i| outs.get(i)) {
                Some(out) => {
                    env.with_stats(|s| s.left += 1);
                    out.port.deliver(&env.ctx, entity).await;
                }
                None => env.with_stats(|s| s.lost += 1),
            }
        }
    });
}

/// The entity a join sends on: the one from its first input, with any property it lacks taken
/// from the others, in input order.
fn combine(mut parts: Vec<Entity>) -> Option<Entity> {
    let mut first = if parts.is_empty() {
        return None;
    } else {
        parts.remove(0)
    };
    for other in parts {
        if first.properties.len() < other.properties.len() {
            first.properties.resize(other.properties.len(), None);
        }
        for (mine, theirs) in first.properties.iter_mut().zip(other.properties) {
            if mine.is_none() {
                *mine = theirs;
            }
        }
    }
    Some(first)
}

/// A parallel gateway. A split sends a copy of each entity down every outgoing connection; a
/// join waits for one entity on each input and merges them into one ([`combine`]), then sends
/// it on (copied, if it also splits).
///
/// A join pairs entities by arrival order: the next one waiting on each input, whichever split
/// made them. With several entities' copies in flight, copies of different entities can be
/// merged with each other, if one branch overtakes another.
///
/// Counting: a join's `entered` counts each entity once, as it arrives on the first input (the
/// entities arriving on the others are its copies, merged into it); `left` counts each entity
/// sent out, one per outgoing connection. So a 2-in/1-out join reports entered = left (less
/// any still waiting for their copies at the end), and a 1-in/2-out split left = 2 x entered.
///
/// An assembly (a join with a bill of materials) takes `quantities[k]` entities from input `k`
/// for each one it sends on; the parts other than the first are used up. Every part counts as
/// entering, so an assembly reports entered = left × the parts per assembly (less those still
/// waiting at the end).
fn spawn_parallel(
    sim: &mut Sim,
    env: BlockEnv,
    inputs: Vec<Store<Entity>>,
    quantities: Vec<u32>,
    outs: Vec<Out>,
    next_id: Rc<RefCell<u64>>,
) {
    sim.spawn_tagged(tag(env.me), async move {
        loop {
            // The parts from each input (a join waits for all of them), in input order.
            let mut parts = Vec::with_capacity(inputs.len());
            for (k, input) in inputs.iter().enumerate() {
                for _ in 0..quantities.get(k).copied().unwrap_or(1).max(1) {
                    parts.push(input.get().await);
                }
            }
            env.adjust(|s| &mut s.waiting, -(parts.len() as f64));
            let Some(entity) = combine(parts) else {
                continue;
            };
            // The first connection takes the entity itself; the others take copies, each a new
            // entity with the same creation time and properties.
            for (k, out) in outs.iter().enumerate() {
                let mut e = entity.clone();
                if k > 0 {
                    let mut n = next_id.borrow_mut();
                    e.id = *n;
                    *n += 1;
                }
                env.with_stats(|s| s.left += 1);
                out.port.deliver(&env.ctx, e).await;
            }
        }
    });
}

/// A batch: entities wait here until there are `size` of them, then go on together as one, the
/// first carrying the others ([`Entity::members`]). `entered` counts each entity, `left` each
/// batch, and the mean contents is the number waiting for their batch to fill.
fn spawn_batch(sim: &mut Sim, env: BlockEnv, size: u32) {
    let size = usize::try_from(size.max(1)).unwrap_or(usize::MAX);
    sim.spawn_tagged(tag(env.me), async move {
        let mut held: Vec<Entity> = Vec::with_capacity(size);
        loop {
            let entity = env.input.get().await;
            env.adjust(|s| &mut s.waiting, -1.0);
            env.adjust(|s| &mut s.busy, 1.0);
            held.push(entity);
            if held.len() < size {
                continue;
            }
            let mut members = std::mem::take(&mut held);
            env.adjust(|s| &mut s.busy, -(members.len() as f64));
            let mut batch = members.remove(0);
            batch.members.extend(members);
            env.with_stats(|s| s.left += 1);
            if let Some(out) = &env.out {
                out.deliver(&env.ctx, batch).await;
            }
        }
    });
}

/// Splits each batch into its entities and sends each on, the carrier first; anything else
/// passes through. `entered` counts batches as they arrive, `left` each entity sent on.
fn spawn_unbatch(sim: &mut Sim, env: BlockEnv) {
    sim.spawn_tagged(tag(env.me), async move {
        loop {
            let mut entity = env.input.get().await;
            env.adjust(|s| &mut s.waiting, -1.0);
            let members = std::mem::take(&mut entity.members);
            for e in std::iter::once(entity).chain(members) {
                env.with_stats(|s| s.left += 1);
                if let Some(out) = &env.out {
                    out.deliver(&env.ctx, e).await;
                }
            }
        }
    });
}
