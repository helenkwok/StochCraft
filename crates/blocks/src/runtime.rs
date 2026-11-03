//! The simulation behind a compiled model: one store per element input, processes per element,
//! and the statistics each element keeps.

use std::cell::RefCell;
use std::rc::Rc;

use stochcraft_des::{Ctx, Sim, Store};
use stochcraft_model::{Element, ElementKind};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::{Tally, TimeWeighted};

use crate::{Compiled, ElementReport, Entity, Report};

impl Compiled {
    /// Runs one replication for the experiment's warm-up plus duration, with random numbers from
    /// `rng` (each element draws from its own forked stream, in model order), and reports the
    /// statistics collected after the warm-up.
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
            elements
                .iter()
                .map(|_| RefCell::new(BlockStats::new(0.0)))
                .collect(),
        );
        let inputs: Vec<Store<Entity>> = elements.iter().map(|_| Store::new(&ctx)).collect();
        let next_id = Rc::new(RefCell::new(0u64));

        for (i, element) in elements.iter().enumerate() {
            let rng = rng.fork();
            let out = self.outputs.get(i).copied().flatten().and_then(|idx| {
                inputs.get(idx).map(|s| Port {
                    store: s.clone(),
                    stats: Rc::clone(&stats),
                    index: idx,
                })
            });
            let Some(input) = inputs.get(i).cloned() else {
                continue;
            };
            let env = BlockEnv {
                ctx: ctx.clone(),
                stats: Rc::clone(&stats),
                me: i,
                input,
                out,
                rng,
            };
            match element.kind.clone() {
                ElementKind::Source {
                    interarrival,
                    limit,
                } => spawn_source(&mut sim, env, interarrival, limit, Rc::clone(&next_id)),
                ElementKind::Server { capacity, service } => {
                    spawn_server(&mut sim, env, capacity, service)
                }
                ElementKind::Delay { duration } => spawn_delay(&mut sim, env, duration),
                ElementKind::Sink => spawn_sink(&mut sim, env),
            }
        }

        // Discard everything before the warm-up.
        let (c, st) = (ctx.clone(), Rc::clone(&stats));
        sim.spawn(async move {
            c.hold(warmup).await;
            let now = c.now();
            st.iter().for_each(|s| s.borrow_mut().reset(now));
        });

        let end = warmup + duration;
        sim.run_until(end);
        let elements = elements
            .iter()
            .zip(stats.iter())
            .map(|(e, s)| s.borrow().report(e, end))
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
}

impl BlockStats {
    fn new(t: f64) -> Self {
        Self {
            entered: 0,
            left: 0,
            waiting: TimeWeighted::new(t, 0.0),
            busy: TimeWeighted::new(t, 0.0),
            wait: Tally::new(),
            time_in_system: Tally::new(),
        }
    }

    fn reset(&mut self, t: f64) {
        self.entered = 0;
        self.left = 0;
        self.waiting.reset(t);
        self.busy.reset(t);
        self.wait = Tally::new();
        self.time_in_system = Tally::new();
    }

    pub(crate) fn report(&self, element: &Element, end: f64) -> ElementReport {
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
        }
    }
}

/// Where a block sends entities: the next block's input, with its statistics.
#[derive(Clone)]
struct Port {
    store: Store<Entity>,
    stats: Rc<Vec<RefCell<BlockStats>>>,
    index: usize,
}

impl Port {
    fn send(&self, now: f64, mut e: Entity) {
        e.entered = now;
        if let Some(s) = self.stats.get(self.index) {
            let mut s = s.borrow_mut();
            s.entered += 1;
            let w = s.waiting.value() + 1.0;
            s.waiting.set(now, w);
        }
        self.store.put(e);
    }
}

struct BlockEnv {
    ctx: Ctx,
    stats: Rc<Vec<RefCell<BlockStats>>>,
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
) {
    sim.spawn(async move {
        let mut made = 0u64;
        while limit.is_none_or(|l| made < l) {
            let dt = interarrival.sample(&mut env.rng);
            env.ctx.hold(dt).await;
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
            if let Some(out) = &env.out {
                out.send(
                    now,
                    Entity {
                        id,
                        created: now,
                        entered: now,
                    },
                );
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
