//! Speed benchmarks.
//!
//! `cargo run --release --manifest-path bench/Cargo.toml -- [store|resource|parallel|all] [N] [REPS]`
//!
//! Model benchmarks, each checked against a known answer (see their modules); the run fails if
//! a check does:
//! - `tandem [CUSTOMERS]`: M/M/1 lines of 5, 50 and 200 stations, on `des` and through `blocks`.
//! - `hold [EVENTS]`: the hold model with 10, 1,000 and 100,000 processes.
//! - `devstone [MAX] [STEP]`: DEVStone LI, HI and HO for every width and depth in `STEP..=MAX`.
//! - `models`: all three with their defaults.
//!
//! Workloads (λ = 0.9, μ = 1, N customers, then the queue drains; theory W = 10):
//! - `store`: an arrival process puts timestamps into an unbounded FIFO store and one server
//!   process takes them out. The same model as Cimba's `benchmark/MM1_single.c` and its SimPy
//!   twin, so the numbers line up with Cimba's published ones.
//! - `resource`: the arrival process spawns one process per customer, which acquires a
//!   single-unit resource. simu-des has no store, so this is the workload it is compared on.
//! - `parallel`: REPS replications of `store` through `stochcraft-experiment`.
//!
//! Every engine draws from the same generator (`stochcraft-random`'s xoshiro256**), so the
//! comparison measures the engines, not their random number generators.

mod devstone;
mod hold;
mod tandem;

use std::cell::{Cell, RefCell};
use std::process::ExitCode;
use std::rc::Rc;
use std::time::Instant;

use stochcraft_des::{Resource, Sim, Store};
use stochcraft_experiment::Experiment;
use stochcraft_random::Rng;

const LAMBDA: f64 = 0.9;
const MU: f64 = 1.0;

struct Outcome {
    customers: u64,
    mean_time_in_system: f64,
}

#[derive(Default)]
struct Acc {
    n: Cell<u64>,
    sum: Cell<f64>,
}

impl Acc {
    fn add(&self, x: f64) {
        self.n.set(self.n.get() + 1);
        self.sum.set(self.sum.get() + x);
    }
    fn outcome(&self) -> Outcome {
        Outcome {
            customers: self.n.get(),
            mean_time_in_system: self.sum.get() / self.n.get().max(1) as f64,
        }
    }
}

fn stochcraft_store(n: u64, rng: Rng) -> Outcome {
    let mut sim = Sim::new();
    let ctx = sim.ctx();
    let store: Store<f64> = Store::new(&ctx);
    let rng = Rc::new(RefCell::new(rng));
    let acc = Rc::new(Acc::default());
    let (c, s, r) = (ctx.clone(), store.clone(), Rc::clone(&rng));
    sim.spawn(async move {
        for _ in 0..n {
            let t = r.borrow_mut().exponential(1.0 / LAMBDA);
            c.hold(t).await;
            s.put(c.now());
        }
    });
    let (c, a) = (ctx.clone(), Rc::clone(&acc));
    sim.spawn(async move {
        loop {
            let arrived = store.get().await;
            let t = rng.borrow_mut().exponential(1.0 / MU);
            c.hold(t).await;
            a.add(c.now() - arrived);
        }
    });
    sim.run();
    acc.outcome()
}

fn stochcraft_resource(n: u64, rng: Rng) -> Outcome {
    let mut sim = Sim::new();
    let ctx = sim.ctx();
    let server = Resource::new(&ctx, 1);
    let rng = Rc::new(RefCell::new(rng));
    let acc = Rc::new(Acc::default());
    let (c, a) = (ctx.clone(), Rc::clone(&acc));
    sim.spawn(async move {
        for _ in 0..n {
            let t = rng.borrow_mut().exponential(1.0 / LAMBDA);
            c.hold(t).await;
            let (cc, srv, aa, r) = (c.clone(), server.clone(), Rc::clone(&a), Rc::clone(&rng));
            c.spawn(async move {
                let arrived = cc.now();
                let _unit = srv.acquire().await;
                let t = r.borrow_mut().exponential(1.0 / MU);
                cc.hold(t).await;
                aa.add(cc.now() - arrived);
            });
        }
    });
    sim.run();
    acc.outcome()
}

#[cfg(feature = "simu")]
fn simu_resource(n: u64, rng: Rng) -> Outcome {
    use simu::{Resource as SimuResource, SimEnv};
    let mut env = SimEnv::with_seed(0);
    let h = env.handle();
    let server = SimuResource::new(1);
    let rng = Rc::new(RefCell::new(rng));
    let acc = Rc::new(Acc::default());
    let a = Rc::clone(&acc);
    env.spawn(async move {
        for _ in 0..n {
            let t = rng.borrow_mut().exponential(1.0 / LAMBDA);
            h.timeout(t).await;
            let (hh, srv, aa, r) = (h.clone(), server.clone(), Rc::clone(&a), Rc::clone(&rng));
            h.spawn(async move {
                let arrived = hh.now();
                let _unit = srv.request().await;
                let t = r.borrow_mut().exponential(1.0 / MU);
                hh.timeout(t).await;
                aa.add(hh.now() - arrived);
            });
        }
    });
    env.run();
    acc.outcome()
}

fn report(engine: &str, workload: &str, f: impl FnOnce() -> Outcome) {
    let started = Instant::now();
    let o = f();
    let secs = started.elapsed().as_secs_f64();
    println!(
        "{engine:<12} {workload:<9} {:>9} customers  W = {:>7.4}  {:>7.3} s  {:>6.2} M customers/s",
        o.customers,
        o.mean_time_in_system,
        secs,
        o.customers as f64 / secs / 1e6
    );
}

fn parallel(n: u64, reps: u64) {
    let started = Instant::now();
    let exp = Experiment::new(reps, 2026);
    let summary = exp.summarise(|r| stochcraft_store(n, r.rng).mean_time_in_system);
    let secs = started.elapsed().as_secs_f64();
    let threads = std::thread::available_parallelism().map_or(1, |t| t.get());
    match summary {
        Ok(Some(s)) => println!(
            "stochcraft   parallel  {reps} x {n} customers on {threads} threads: W = {s}  {secs:.3} s  {:.2} M customers/s",
            (reps * n) as f64 / secs / 1e6
        ),
        Ok(None) => println!("no replications"),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Defaults: customers per line in `tandem`, events per population in `hold`, and the largest
/// width and depth and the step between them in `devstone`.
const TANDEM_CUSTOMERS: u64 = 100_000;
const HOLD_EVENTS: u64 = 2_000_000;
const DEVSTONE_MAX: u64 = 200;
const DEVSTONE_STEP: u64 = 50;

/// Runs the model benchmarks; `models` runs all three with their defaults. Returns whether
/// every check passed.
fn models(which: &str, args: &[String]) -> bool {
    let arg = |i: usize, default: u64| {
        if which == "models" {
            return default;
        }
        args.get(i).and_then(|a| a.parse().ok()).unwrap_or(default)
    };
    let mut ok = true;
    if matches!(which, "tandem" | "models") {
        ok &= tandem::run(arg(1, TANDEM_CUSTOMERS));
    }
    if matches!(which, "hold" | "models") {
        ok &= hold::run(arg(1, HOLD_EVENTS));
    }
    if matches!(which, "devstone" | "models") {
        ok &= devstone::run(arg(1, DEVSTONE_MAX), arg(2, DEVSTONE_STEP));
    }
    ok
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let which = args.first().map_or("all", String::as_str);
    if matches!(which, "tandem" | "hold" | "devstone" | "models") {
        if models(which, &args) {
            return ExitCode::SUCCESS;
        }
        eprintln!("a check failed");
        return ExitCode::FAILURE;
    }
    let n = args
        .get(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1_000_000);
    let reps = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(64);
    println!(
        "M/M/1, λ = {LAMBDA}, μ = {MU}, theory W = {:.1}",
        1.0 / (MU - LAMBDA)
    );
    if matches!(which, "store" | "all") {
        report("stochcraft", "store", || stochcraft_store(n, Rng::new(1)));
    }
    if matches!(which, "resource" | "all") {
        report("stochcraft", "resource", || {
            stochcraft_resource(n, Rng::new(1))
        });
        #[cfg(feature = "simu")]
        report("simu-des", "resource", || simu_resource(n, Rng::new(1)));
    }
    if matches!(which, "parallel" | "all") {
        parallel(n, reps);
    }
    ExitCode::SUCCESS
}
