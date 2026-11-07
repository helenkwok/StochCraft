//! The simulation behind a compiled model: one store per element input, processes per element,
//! and the statistics each element keeps.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use stochcraft_des::{Ctx, Resource, ResourceGuard, Sim, Store};
use stochcraft_model::calendar::{self, Calendar, DateTime};
use stochcraft_model::{Element, ElementId, ElementKind, Model, Need, ResourcePool, Timed};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::{Tally, TimeWeighted};

use crate::{Assign, Compiled, ElementReport, Entity, Held, Report, Source};

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
    pub fn run(&self, rng: Rng) -> Report {
        let x = &self.model.experiment;
        self.run_for(rng, x.warmup, x.duration)
    }

    /// As [`Compiled::run`], with an explicit warm-up and duration (negative or non-finite values
    /// count as 0).
    pub fn run_for(&self, mut rng: Rng, warmup: f64, duration: f64) -> Report {
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
                    capacity, service, ..
                } => spawn_server(&mut sim, env, capacity, service),
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
                    spawn_parallel(&mut sim, env, ins, outs, Rc::clone(&next_id))
                }
            }
        }

        // Discard everything before the warm-up. With none, there is nothing to discard (and a
        // reset at time 0 would discard an arrival at time 0).
        if warmup > 0.0 {
            let (c, st, pl) = (ctx.clone(), Rc::clone(&stats), Rc::clone(&pools));
            sim.spawn(async move {
                c.hold(warmup).await;
                let now = c.now();
                st.iter().for_each(|s| s.borrow_mut().reset(now));
                pl.reset(now);
            });
        }

        sim.run_until(end);
        let mut elements: Vec<ElementReport> = elements
            .iter()
            .zip(stats.iter())
            .zip(&self.assigns)
            .zip(&feeds_buffer)
            .map(|(((e, s), a), &blocks)| s.borrow().report(e, a, blocks, end))
            .collect();
        // Then each shared pool, after the elements.
        elements.extend(pools.reports(&self.model.resources, end));
        Report {
            warmup,
            duration,
            elements,
        }
    }
}

/// Statistics a block keeps while the model runs.
pub(crate) struct BlockStats {
    entered: u64,
    left: u64,
    /// Entities waiting in the block's input (servers only).
    waiting: TimeWeighted,
    /// Units busy (servers) or entities inside (delays).
    busy: TimeWeighted,
    /// Time from entering a server's input to the start of service.
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
    /// Arrivals lost because this element's buffer was full.
    lost: u64,
}

impl BlockStats {
    fn new(t: f64, properties: usize) -> Self {
        Self {
            entered: 0,
            left: 0,
            waiting: TimeWeighted::new(t, 0.0),
            busy: TimeWeighted::new(t, 0.0),
            wait: Tally::new(),
            time_in_system: Tally::new(),
            properties: (0..properties).map(|_| Tally::new()).collect(),
            blocked: TimeWeighted::new(t, 0.0),
            blocked_time: Tally::new(),
            lost: 0,
        }
    }

    fn reset(&mut self, t: f64) {
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
    }

    /// The element's report; `blocks`: whether it sends to a finite buffer.
    pub(crate) fn report(
        &self,
        element: &Element,
        assigns: &[Assign],
        blocks: bool,
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
                | ElementKind::Delay { .. } => self.busy.average(end),
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
            lost: finite.then_some(self.lost),
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

    /// Sends a new arrival if there is room (a place freed at this instant counts); otherwise
    /// it is lost, and counted as lost by the next element.
    async fn offer(&self, ctx: &Ctx, e: Entity) {
        if let Some(p) = &self.places {
            match p.pool.acquire_within(0.0).await {
                Ok(guard) => p.enter(e.id, guard),
                Err(_) => {
                    if let Some(s) = self.stats.get(self.index) {
                        s.borrow_mut().lost += 1;
                    }
                    return;
                }
            }
        }
        self.send(ctx.now(), e);
    }

    fn send(&self, now: f64, mut e: Entity) {
        e.entered = now;
        self.assigner.enter(self.index, &mut e, &self.stats);
        if let Some(s) = self.stats.get(self.index) {
            let mut s = s.borrow_mut();
            s.entered += 1;
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
    sim.spawn(async move {
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
            env.with_stats(|s| {
                s.entered += 1;
                s.left += 1;
            });
            let mut entity = Entity {
                id,
                created: now,
                entered: now,
                properties: Vec::new(),
            };
            // A source sets its properties on the entities it creates.
            env.assigner.enter(env.me, &mut entity, &env.stats);
            if let Some(out) = &env.out {
                out.offer(&env.ctx, entity).await;
            }
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
    let env = Rc::new(RefCell::new(env));
    for _ in 0..capacity {
        let env = Rc::clone(&env);
        let service = service.clone();
        sim.spawn(async move {
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
                    service.sample(&mut e.rng)
                };
                ctx.hold(duration).await;
                env.borrow().adjust(|s| &mut s.busy, -1.0);
                move_on(&env, &ctx, entity).await;
            }
        });
    }
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
                    mean_time_in_system: None,
                    mean_blocked_time: None,
                    blocking_probability: None,
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
    let env = Rc::new(RefCell::new(env));
    sim.spawn(async move {
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
    let env = Rc::new(RefCell::new(env));
    let mut sim_ctx = None;
    {
        let e = env.borrow();
        sim_ctx.get_or_insert(e.ctx.clone());
    }
    let Some(ctx) = sim_ctx else { return };
    sim.spawn(async move {
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
    sim.spawn(async move {
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
    sim.spawn(async move {
        loop {
            let entity = env.input.get().await;
            env.adjust(|s| &mut s.waiting, -1.0);
            env.with_stats(|s| s.left += 1);
            let to = choose(&outs, &entity, &env.assigner, &mut env.rng);
            // With no route (no condition holds and there is no default), the entity leaves
            // the model here.
            // A full finite buffer holds the gateway, and everything behind it, up.
            if let Some(out) = to.and_then(|i| outs.get(i)) {
                out.port.deliver(&env.ctx, entity).await;
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

fn spawn_parallel(
    sim: &mut Sim,
    env: BlockEnv,
    inputs: Vec<Store<Entity>>,
    outs: Vec<Out>,
    next_id: Rc<RefCell<u64>>,
) {
    sim.spawn(async move {
        loop {
            // One entity from each input (a join waits for all of them), in input order.
            let mut parts = Vec::with_capacity(inputs.len());
            for input in &inputs {
                parts.push(input.get().await);
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
