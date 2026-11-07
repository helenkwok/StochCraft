//! Batching and assembly against exact results: deterministic flows worked by hand, an assembly
//! whose throughput is that of its slowest input, and batches that queue as an E₄/M/1 queue
//! (docs/research.md, "Batching and assembly").
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, ExperimentSummary, Metric, Report, compile, summarise};
use stochcraft_model::{Dist, Experiment, Model};
use stochcraft_random::Rng;

const REPS: u64 = 16;
/// Each check is a 95% interval, so about one in twenty would miss by chance; the seeds are fixed
/// so the tests are repeatable.
const SEED: u64 = 70_000;
const DURATION: f64 = 20_000.0;

fn experiment() -> Experiment {
    Experiment {
        warmup: 1_000.0,
        duration: DURATION,
        replications: REPS as u32,
        ..Experiment::default()
    }
}

fn reports(model: &Model) -> Vec<Report> {
    let compiled = compile(model);
    let Ok(compiled) = compiled else {
        panic!("{compiled:?}")
    };
    (0..REPS)
        .map(|r| {
            compiled
                .run(Rng::new(SEED + r))
                .unwrap_or_else(|e| panic!("{e}"))
        })
        .collect()
}

fn covers(s: &ExperimentSummary, element: &str, metric: Metric, theory: f64) {
    let got = s.get(element, metric);
    eprintln!("{element} {}: {got:?} (theory {theory})", metric.name());
    assert!(
        got.is_some_and(|g| g.n == REPS && g.contains(theory)),
        "{element} {}: {got:?} vs {theory}",
        metric.name()
    );
}

fn report(b: &Builder, duration: f64) -> Report {
    let r = b.run(Rng::new(1), 0.0, duration);
    let Ok(r) = r else { panic!("{r:?}") };
    r
}

fn counts(r: &Report, id: &str) -> (u64, u64) {
    r.element(id)
        .map(|e| (e.entered, e.left))
        .unwrap_or_else(|| panic!("no element {id}"))
}

/// Arrivals at 1, 2, …, 30 in batches of 3: a batch leaves at 3, 6, …, 30 (10 batches of 30
/// entities), and between batches 0, then 1, then 2 wait, so 1 waits on average. Split up again
/// before the sink, all 30 arrive, each after its own time in system.
#[test]
fn batches_collect_and_split_exactly() {
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(1.0), None);
    let batch = b.batch("pack", 3);
    let out = b.sink("out");
    b.connect(src, batch);
    b.connect(batch, out);
    let r = report(&b, 30.0);
    assert_eq!(counts(&r, "pack"), (30, 10), "{r:?}");
    assert_eq!(counts(&r, "out"), (10, 10), "{r:?}");
    let contents = r.element("pack").and_then(|e| e.mean_contents);
    assert!(
        contents.is_some_and(|c| (c - 1.0).abs() < 1e-9),
        "{contents:?}"
    );
    // A batch's time in system is its first entity's: it waited 2 for the other two.
    let t = r.element("out").and_then(|e| e.mean_time_in_system);
    assert_eq!(t, Some(2.0), "{r:?}");

    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(1.0), None);
    let batch = b.batch("pack", 3);
    let trip = b.delay("trip", Dist::Constant(5.0));
    let split = b.unbatch("unpack");
    let out = b.sink("out");
    b.connect(src, batch);
    b.connect(batch, trip);
    b.connect(trip, split);
    b.connect(split, out);
    let r = report(&b, 40.0);
    // Batches leave at 3, …, 39 (13), arrive 5 later: those leaving by 35 are unpacked (11).
    assert_eq!(counts(&r, "pack"), (40, 13), "{r:?}");
    assert_eq!(counts(&r, "unpack"), (11, 33), "{r:?}");
    assert_eq!(counts(&r, "out"), (33, 33), "{r:?}");
    // In each batch the entities waited 2, 1 and 0, then travelled 5: 7, 6, 5 on average 6.
    let t = r.element("out").and_then(|e| e.mean_time_in_system);
    assert!(t.is_some_and(|t| (t - 6.0).abs() < 1e-9), "{t:?}");
}

/// A bill of materials: 2 parts from A (one every 1) and 1 from B (one every 2) make each
/// assembly. Assembly k uses A's arrivals at 2k − 1 and 2k and B's at 2k, so one is made every 2
/// (15 in 30), each 1 after its first part arrived; every part counts as entering.
#[test]
fn an_assembly_takes_its_bill_of_materials() {
    let mut b = Builder::new();
    let a = b.source("a", Dist::Constant(1.0), None);
    let c = b.source("b", Dist::Constant(2.0), None);
    let join = b.parallel("assemble");
    let out = b.sink("out");
    b.connect(a, join);
    b.connect(c, join);
    b.connect(join, out);
    b.quantity(a, join, 2);
    let r = report(&b, 30.0);
    assert_eq!(counts(&r, "assemble"), (45, 15), "{r:?}");
    assert_eq!(counts(&r, "out"), (15, 15), "{r:?}");
    let t = r.element("out").and_then(|e| e.mean_time_in_system);
    assert_eq!(t, Some(1.0), "{r:?}");
}

/// An assembly makes λ_k / q_k a time from each input k, at most, so its throughput is the least
/// of those: with Poisson parts at rates 1 (2 per assembly) and 0.4 (1 per assembly) it is 0.4,
/// set by the second; at rates 0.6 and 0.4 it is 0.3, set by the first.
#[test]
fn an_assembly_runs_at_the_pace_of_its_slowest_input() {
    for (rate_a, rate_b, theory) in [(1.0, 0.4, 0.4), (0.6, 0.4, 0.3)] {
        let mut b = Builder::new();
        let a = b.source("a", Dist::exponential_rate(rate_a), None);
        let c = b.source("b", Dist::exponential_rate(rate_b), None);
        let join = b.parallel("assemble");
        let out = b.sink("out");
        b.connect(a, join);
        b.connect(c, join);
        b.connect(join, out);
        b.quantity(a, join, 2);
        let s = summarise(&reports(&b.build(experiment())));
        covers(&s, "out", Metric::Left, theory * DURATION);
    }
}

/// GI/M/1: with interarrival times of Laplace transform A*(s) and exponential service at rate μ,
/// the wait in queue is σ / (μ(1 − σ)), σ the root in (0, 1) of σ = A*(μ(1 − σ)).
fn gi_m_1_wait(transform: impl Fn(f64) -> f64, mu: f64) -> f64 {
    let mut sigma = 0.5;
    for _ in 0..10_000 {
        sigma = transform(mu * (1.0 - sigma));
    }
    sigma / (mu * (1.0 - sigma))
}

/// Poisson arrivals at rate 1 packed into batches of 4: the batches arrive with Erlang-4 times
/// between them (mean 4) at one machine that takes an exponential time of mean 3 per batch, so
/// they queue as E₄/M/1, A*(s) = (1/(1+s))⁴. Unpacked again, everything arrives (rate 1).
#[test]
fn batches_queue_as_erlang_arrivals() {
    let mu = 1.0 / 3.0;
    let mut b = Builder::new();
    let src = b.source("in", Dist::exponential_rate(1.0), None);
    let pack = b.batch("pack", 4);
    let machine = b.server("machine", 1, Dist::exponential_rate(mu));
    let unpack = b.unbatch("unpack");
    let out = b.sink("out");
    b.connect(src, pack);
    b.connect(pack, machine);
    b.connect(machine, unpack);
    b.connect(unpack, out);
    let s = summarise(&reports(&b.build(experiment())));
    let wait = gi_m_1_wait(|x| (1.0 / (1.0 + x)).powi(4), mu);
    covers(&s, "machine", Metric::MeanWait, wait);
    covers(&s, "machine", Metric::Utilisation, 0.75);
    // On average 1.5 entities wait for their batch to fill (0, 1, 2 or 3 alike).
    covers(&s, "pack", Metric::MeanContents, 1.5);
    covers(&s, "out", Metric::Left, DURATION);
}
