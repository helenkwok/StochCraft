//! The simulation behind a compiled model: one store per element input, processes per element,
//! and the statistics each element keeps.

use std::cell::RefCell;
use std::rc::Rc;

use stochcraft_des::{Ctx, Sim, Store};
use stochcraft_model::{Element, ElementKind};
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
                ElementKind::Server { capacity, service } => {
                    spawn_server(&mut sim, env, capacity, service)
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
            let (c, st) = (ctx.clone(), Rc::clone(&stats));
            sim.spawn(async move {
                c.hold(warmup).await;
                let now = c.now();
                st.iter().for_each(|s| s.borrow_mut().reset(now));
            });
        }

        let end = warmup + duration;
        sim.run_until(end);
        let elements = elements
            .iter()
            .zip(stats.iter())
            .zip(&self.assigns)
            .map(|((e, s), a)| s.borrow().report(e, a, end))
            .collect();
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
    }

    pub(crate) fn report(&self, element: &Element, assigns: &[Assign], end: f64) -> ElementReport {
        let kind = &element.kind;
        let server = matches!(kind, ElementKind::Server { .. });
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
                ElementKind::Server { .. } | ElementKind::Delay { .. } => self.busy.average(end),
                _ => None,
            },
            mean_time_in_system: if matches!(kind, ElementKind::Sink) {
                self.time_in_system.mean()
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
}

impl Port {
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
    sim.spawn(async move {
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
                out.send(now, entity);
            }
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
                let e = env.borrow();
                e.adjust(|s| &mut s.busy, -1.0);
                e.with_stats(|s| s.left += 1);
                if let Some(out) = &e.out {
                    out.send(ctx.now(), entity);
                }
            }
        });
    }
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
                let e = env2.borrow();
                e.adjust(|s| &mut s.busy, -1.0);
                e.with_stats(|s| s.left += 1);
                if let Some(out) = &e.out {
                    out.send(c.now(), entity);
                }
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
            let now = env.ctx.now();
            env.adjust(|s| &mut s.waiting, -1.0);
            env.with_stats(|s| s.left += 1);
            let to = choose(&outs, &entity, &env.assigner, &mut env.rng);
            // With no route (no condition holds and there is no default), the entity leaves
            // the model here.
            if let Some(out) = to.and_then(|i| outs.get(i)) {
                out.port.send(now, entity);
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
            let now = env.ctx.now();
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
                out.port.send(now, e);
            }
        }
    });
}
