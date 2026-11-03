//! Flow blocks: the building blocks of process models.
//!
//! Entities (parts, customers, pallets) are created by a [`Model::source`], wait for and occupy a
//! [`Model::server`] with one or more identical units, spend time in a [`Model::delay`] with no
//! capacity limit, and leave through a [`Model::sink`]. Blocks are connected output to input with
//! [`Model::connect`]; [`Model::run`] builds the simulation, runs it and returns per-block
//! statistics, with everything before the warm-up time discarded.
//!
//! ```
//! use stochcraft_blocks::Model;
//! use stochcraft_random::{Dist, Rng};
//!
//! let mut m = Model::new();
//! let arrivals = m.source("arrivals", Dist::exponential_rate(0.75), None);
//! let machine = m.server("machine", 1, Dist::exponential_rate(1.0));
//! let exit = m.sink("exit");
//! m.connect(arrivals, machine);
//! m.connect(machine, exit);
//! let report = m.run(Rng::new(1), 1_000.0, 10_000.0).expect("valid model");
//! assert!(report.block("machine").and_then(|b| b.utilisation).is_some());
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use stochcraft_des::{Ctx, Sim, Store};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::{Tally, TimeWeighted};

/// A block in a [`Model`], returned when it is added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(u32);

/// Something that flows through the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entity {
    /// Unique within a run, in creation order.
    pub id: u64,
    /// When its source created it.
    pub created: f64,
    /// When it entered the block it is in now.
    pub entered: f64,
}

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Source {
        interarrival: Dist,
        limit: Option<u64>,
    },
    Server {
        capacity: u32,
        service: Dist,
    },
    Delay {
        duration: Dist,
    },
    Sink,
}

impl Kind {
    fn name(&self) -> &'static str {
        match self {
            Kind::Source { .. } => "source",
            Kind::Server { .. } => "server",
            Kind::Delay { .. } => "delay",
            Kind::Sink => "sink",
        }
    }
}

struct Block {
    name: String,
    kind: Kind,
    output: Option<BlockId>,
}

/// Why a model cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// A source, server or delay has nowhere to send entities.
    Unconnected { block: String },
    /// A connection leads into a source, which takes no input.
    IntoSource { from: String, to: String },
    /// Two blocks share a name, so results could not be told apart.
    DuplicateName { name: String },
    /// The warm-up or the run length is negative or not finite.
    BadTimes,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Unconnected { block } => {
                write!(f, "block `{block}` has no output connection")
            }
            ModelError::IntoSource { from, to } => {
                write!(f, "`{from}` is connected into source `{to}`")
            }
            ModelError::DuplicateName { name } => write!(f, "two blocks are named `{name}`"),
            ModelError::BadTimes => {
                write!(f, "warm-up and run length must be finite and not negative")
            }
        }
    }
}

impl std::error::Error for ModelError {}

/// A process model: blocks and the connections between them.
#[derive(Default)]
pub struct Model {
    blocks: Vec<Block>,
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, name: &str, kind: Kind) -> BlockId {
        let id = BlockId(u32::try_from(self.blocks.len()).unwrap_or(u32::MAX));
        self.blocks.push(Block {
            name: name.to_owned(),
            kind,
            output: None,
        });
        id
    }

    /// Creates entities with the given time between arrivals, up to `limit` if one is given.
    pub fn source(&mut self, name: &str, interarrival: Dist, limit: Option<u64>) -> BlockId {
        self.add(
            name,
            Kind::Source {
                interarrival,
                limit,
            },
        )
    }

    /// `capacity` identical units (at least 1) that each serve one entity at a time, first come
    /// first served, for a time drawn from `service`.
    pub fn server(&mut self, name: &str, capacity: u32, service: Dist) -> BlockId {
        self.add(
            name,
            Kind::Server {
                capacity: capacity.max(1),
                service,
            },
        )
    }

    /// Holds every entity for a time drawn from `duration`, with no capacity limit (transport,
    /// curing, travel).
    pub fn delay(&mut self, name: &str, duration: Dist) -> BlockId {
        self.add(name, Kind::Delay { duration })
    }

    /// Removes entities from the model and records their time in system.
    pub fn sink(&mut self, name: &str) -> BlockId {
        self.add(name, Kind::Sink)
    }

    /// Sends `from`'s entities to `to`. A later call for the same `from` replaces the earlier one.
    pub fn connect(&mut self, from: BlockId, to: BlockId) {
        if let Some(b) = self.blocks.get_mut(from.0 as usize) {
            b.output = Some(to);
        }
    }

    fn validate(&self, warmup: f64, duration: f64) -> Result<(), ModelError> {
        if !(warmup.is_finite() && duration.is_finite() && warmup >= 0.0 && duration >= 0.0) {
            return Err(ModelError::BadTimes);
        }
        let mut names: Vec<&str> = self.blocks.iter().map(|b| b.name.as_str()).collect();
        names.sort_unstable();
        if let Some(w) = names.windows(2).find(|w| w[0] == w[1]) {
            return Err(ModelError::DuplicateName {
                name: w[0].to_owned(),
            });
        }
        for b in &self.blocks {
            match (&b.kind, b.output) {
                (Kind::Sink, _) => {}
                (_, None) => {
                    return Err(ModelError::Unconnected {
                        block: b.name.clone(),
                    });
                }
                (_, Some(to)) => match self.blocks.get(to.0 as usize) {
                    None => {
                        return Err(ModelError::Unconnected {
                            block: b.name.clone(),
                        });
                    }
                    Some(t) if matches!(t.kind, Kind::Source { .. }) => {
                        return Err(ModelError::IntoSource {
                            from: b.name.clone(),
                            to: t.name.clone(),
                        });
                    }
                    Some(_) => {}
                },
            }
        }
        Ok(())
    }

    /// Runs the model for `warmup + duration` time units with random numbers from `rng` (each
    /// block draws from its own forked stream) and reports statistics for the `duration` after
    /// the warm-up.
    pub fn run(&self, mut rng: Rng, warmup: f64, duration: f64) -> Result<Report, ModelError> {
        self.validate(warmup, duration)?;
        let mut sim = Sim::new();
        let ctx = sim.ctx();
        let stats: Rc<Vec<RefCell<BlockStats>>> = Rc::new(
            self.blocks
                .iter()
                .map(|_| RefCell::new(BlockStats::new(0.0)))
                .collect(),
        );
        let inputs: Vec<Store<Entity>> = self.blocks.iter().map(|_| Store::new(&ctx)).collect();
        let next_id = Rc::new(RefCell::new(0u64));

        for (i, block) in self.blocks.iter().enumerate() {
            let rng = rng.fork();
            let out = block.output.and_then(|o| {
                let idx = o.0 as usize;
                inputs.get(idx).map(|s| Port {
                    store: s.clone(),
                    stats: Rc::clone(&stats),
                    index: idx,
                })
            });
            let input = inputs.get(i).cloned();
            let (Some(input), me) = (input, i) else {
                continue;
            };
            let env = BlockEnv {
                ctx: ctx.clone(),
                stats: Rc::clone(&stats),
                me,
                input,
                out,
                rng,
            };
            match block.kind.clone() {
                Kind::Source {
                    interarrival,
                    limit,
                } => spawn_source(&mut sim, env, interarrival, limit, Rc::clone(&next_id)),
                Kind::Server { capacity, service } => {
                    spawn_server(&mut sim, env, capacity, service)
                }
                Kind::Delay { duration } => spawn_delay(&mut sim, env, duration),
                Kind::Sink => spawn_sink(&mut sim, env),
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
        let blocks = self
            .blocks
            .iter()
            .zip(stats.iter())
            .map(|(b, s)| s.borrow().report(&b.name, &b.kind, end))
            .collect();
        Ok(Report {
            warmup,
            duration,
            blocks,
        })
    }
}

/// Statistics a block keeps while the model runs.
struct BlockStats {
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

    fn report(&self, name: &str, kind: &Kind, end: f64) -> BlockReport {
        let server = matches!(kind, Kind::Server { .. });
        BlockReport {
            name: name.to_owned(),
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
                Kind::Server { capacity, .. } => {
                    self.busy.average(end).map(|b| b / f64::from(*capacity))
                }
                _ => None,
            },
            mean_contents: match kind {
                Kind::Server { .. } | Kind::Delay { .. } => self.busy.average(end),
                _ => None,
            },
            mean_time_in_system: if matches!(kind, Kind::Sink) {
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

/// Statistics for one block over the reporting period.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockReport {
    pub name: String,
    /// "source", "server", "delay" or "sink".
    pub kind: &'static str,
    /// Entities that entered (for a source: created).
    pub entered: u64,
    /// Entities that left (for a sink: removed).
    pub left: u64,
    /// Servers: time-average number waiting for a unit.
    pub mean_queue_length: Option<f64>,
    /// Servers: mean time from arrival to start of service.
    pub mean_wait: Option<f64>,
    /// Servers: time-average fraction of units busy.
    pub utilisation: Option<f64>,
    /// Servers and delays: time-average number of entities being served or delayed.
    pub mean_contents: Option<f64>,
    /// Sinks: mean time from creation to arrival here.
    pub mean_time_in_system: Option<f64>,
}

/// The results of a run.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub warmup: f64,
    pub duration: f64,
    pub blocks: Vec<BlockReport>,
}

impl Report {
    pub fn block(&self, name: &str) -> Option<&BlockReport> {
        self.blocks.iter().find(|b| b.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_line_has_exact_times() {
        // Arrivals every 1.0, a 0.5 machine, 2.0 of transport, a 0.5 machine: no queueing, so
        // every entity takes exactly 3.0. The warm-up ends at 10.25 so that no event falls on
        // the boundary: exits at 11, 12, …, 110 are the 100 counted.
        let mut m = Model::new();
        let src = m.source("in", Dist::Constant(1.0), None);
        let a = m.server("cut", 1, Dist::Constant(0.5));
        let t = m.delay("conveyor", Dist::Constant(2.0));
        let b = m.server("weld", 1, Dist::Constant(0.5));
        let out = m.sink("out");
        m.connect(src, a);
        m.connect(a, t);
        m.connect(t, b);
        m.connect(b, out);
        let r = m.run(Rng::new(1), 10.25, 100.0);
        let r = r.ok();
        let sink = r.as_ref().and_then(|r| r.block("out"));
        assert!(
            sink.and_then(|s| s.mean_time_in_system)
                .is_some_and(|w| (w - 3.0).abs() < 1e-9),
            "{sink:?}"
        );
        let cut = r.as_ref().and_then(|r| r.block("cut"));
        assert!(
            cut.and_then(|c| c.utilisation)
                .is_some_and(|u| (u - 0.5).abs() < 1e-9),
            "{cut:?}"
        );
        assert!(
            cut.and_then(|c| c.mean_wait)
                .is_some_and(|w| w.abs() < 1e-9)
        );
        let conv = r.as_ref().and_then(|r| r.block("conveyor"));
        // Little's law: 1 entity per unit time for 2.0 each.
        assert!(
            conv.and_then(|c| c.mean_contents)
                .is_some_and(|n| (n - 2.0).abs() < 1e-9),
            "{conv:?}"
        );
        assert_eq!(sink.map(|s| s.left), Some(100));
    }

    #[test]
    fn source_limit_stops_arrivals() {
        let mut m = Model::new();
        let src = m.source("in", Dist::Constant(1.0), Some(5));
        let out = m.sink("out");
        m.connect(src, out);
        let r = m.run(Rng::new(1), 0.0, 100.0).ok();
        assert_eq!(
            r.as_ref().and_then(|r| r.block("out")).map(|b| b.left),
            Some(5)
        );
    }

    #[test]
    fn invalid_models_are_refused() {
        let mut m = Model::new();
        let src = m.source("in", Dist::Constant(1.0), None);
        let s = m.server("m", 1, Dist::Constant(1.0));
        m.connect(src, s);
        assert_eq!(
            m.run(Rng::new(1), 0.0, 1.0),
            Err(ModelError::Unconnected { block: "m".into() })
        );
        m.connect(s, src);
        assert_eq!(
            m.run(Rng::new(1), 0.0, 1.0),
            Err(ModelError::IntoSource {
                from: "m".into(),
                to: "in".into()
            })
        );
        let out = m.sink("in");
        m.connect(s, out);
        assert_eq!(
            m.run(Rng::new(1), 0.0, 1.0),
            Err(ModelError::DuplicateName { name: "in".into() })
        );
        let mut ok = Model::new();
        let a = ok.source("a", Dist::Constant(1.0), None);
        let z = ok.sink("z");
        ok.connect(a, z);
        assert_eq!(
            ok.run(Rng::new(1), f64::NAN, 1.0),
            Err(ModelError::BadTimes)
        );
        assert_eq!(ok.run(Rng::new(1), 0.0, -1.0), Err(ModelError::BadTimes));
    }

    #[test]
    fn same_seed_same_report() {
        let build = || {
            let mut m = Model::new();
            let src = m.source("in", Dist::exponential_rate(0.8), None);
            let s = m.server(
                "m",
                2,
                Dist::Triangular {
                    min: 0.5,
                    mode: 1.5,
                    max: 3.0,
                },
            );
            let out = m.sink("out");
            m.connect(src, s);
            m.connect(s, out);
            m.run(Rng::new(77), 50.0, 2_000.0)
        };
        assert!(build().is_ok());
        assert_eq!(build(), build());
    }
}
