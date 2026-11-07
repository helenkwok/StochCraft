//! The simulation behind a compiled model: one store per element input, processes per element,
//! and the statistics each element keeps.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use stochcraft_des::{Ctx, Resource, ResourceGuard, Sim, Stopped, Store, Watch};
use stochcraft_model::{Element, ElementKind, Model, Need, ResourcePool};
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
        mut rng: Rng,
        warmup: f64,
        duration: f64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Report, RunError> {
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
        let pools = Rc::new(Pools::new(&ctx, &self.model.resources));
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
                            // A join's second and later inputs bring copies (`spawn_parallel`).
                            counts: r.input == 0,
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
                } => spawn_source(
                    &mut sim,
                    env,
                    interarrival,
                    limit,
                    Rc::clone(&next_id),
                    self.first_arrival_at_zero,
                ),
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

        let end = warmup + duration;
        let watch = Watch {
            max_events_at_one_time: self.max_events_at_one_time,
            cancelled,
        };
        sim.run_watched(end, &watch)
            .map_err(|stopped| self.run_error(stopped))?;
        let mut elements: Vec<ElementReport> = elements
            .iter()
            .zip(stats.iter())
            .zip(&self.assigns)
            .zip(&feeds_buffer)
            .map(|(((e, s), a), &blocks)| s.borrow().report(e, a, blocks, end))
            .collect();
        // Then each shared pool, after the elements.
        elements.extend(pools.reports(&self.model.resources, end));
        Ok(Report {
            warmup,
            duration,
            elements,
        })
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

fn spawn_source(
    sim: &mut Sim,
    mut env: BlockEnv,
    interarrival: Dist,
    limit: Option<u64>,
    next_id: Rc<RefCell<u64>>,
    first_at_zero: bool,
) {
    sim.spawn_tagged(tag(env.me), async move {
        let mut made = 0u64;
        while limit.is_none_or(|l| made < l) {
            if !(first_at_zero && made == 0) {
                let dt = interarrival.sample(&mut env.rng);
                env.ctx.hold(dt).await;
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

fn spawn_server(sim: &mut Sim, env: BlockEnv, capacity: u32, service: Dist) {
    let me = tag(env.me);
    let env = Rc::new(RefCell::new(env));
    for _ in 0..capacity {
        let env = Rc::clone(&env);
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
    free: RefCell<Vec<u32>>,
    quantity: Vec<u32>,
    busy: RefCell<Vec<TimeWeighted>>,
    /// How many times each pool's units were taken.
    taken: RefCell<Vec<u64>>,
    /// Highest priority first, then by arrival.
    waiting: RefCell<Vec<Waiting>>,
    seq: RefCell<u64>,
}

impl Pools {
    fn new(ctx: &Ctx, resources: &[ResourcePool]) -> Self {
        Self {
            ctx: ctx.clone(),
            free: RefCell::new(resources.iter().map(|r| r.quantity).collect()),
            quantity: resources.iter().map(|r| r.quantity).collect(),
            busy: RefCell::new(
                resources
                    .iter()
                    .map(|_| TimeWeighted::new(0.0, 0.0))
                    .collect(),
            ),
            taken: RefCell::new(vec![0; resources.len()]),
            waiting: RefCell::new(Vec::new()),
            seq: RefCell::new(0),
        }
    }

    /// Takes the first of `request`'s alternatives that the free units can meet, if any.
    fn try_take(&self, request: &Request) -> Option<Taken> {
        let mut free = self.free.borrow_mut();
        let alt = request.alternatives.iter().find(|a| {
            let have: u64 = a
                .pools
                .iter()
                .map(|&p| u64::from(free.get(p).copied().unwrap_or(0)))
                .sum();
            a.quantity > 0 && have >= u64::from(a.quantity)
        })?;
        let mut left = alt.quantity;
        let mut taken = Vec::new();
        for &p in &alt.pools {
            if left == 0 {
                break;
            }
            if let Some(f) = free.get_mut(p) {
                let n = (*f).min(left);
                if n > 0 {
                    *f -= n;
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
        let mut busy = self.busy.borrow_mut();
        let mut count = self.taken.borrow_mut();
        for &(p, _) in taken {
            if let (Some(b), Some(&q), Some(&f)) =
                (busy.get_mut(p), self.quantity.get(p), free.get(p))
            {
                b.set(now, f64::from(q - f));
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
                    *f += n;
                }
            }
        }
        self.record(&taken, false);
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
        self.taken.borrow_mut().iter_mut().for_each(|c| *c = 0);
    }

    /// Each pool's report: how often its units were taken, mean units busy and utilisation.
    fn reports(&self, resources: &[ResourcePool], end: f64) -> Vec<ElementReport> {
        let busy = self.busy.borrow();
        let taken = self.taken.borrow();
        resources
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mean = busy.get(i).and_then(|b| b.average(end));
                let n = taken.get(i).copied().unwrap_or(0);
                ElementReport {
                    id: r.id.clone(),
                    name: r.name.clone(),
                    kind: "resource",
                    entered: n,
                    left: n,
                    mean_queue_length: None,
                    mean_wait: None,
                    utilisation: mean.map(|b| b / f64::from(r.quantity.max(1))),
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
fn spawn_parallel(
    sim: &mut Sim,
    env: BlockEnv,
    inputs: Vec<Store<Entity>>,
    outs: Vec<Out>,
    next_id: Rc<RefCell<u64>>,
) {
    sim.spawn_tagged(tag(env.me), async move {
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
