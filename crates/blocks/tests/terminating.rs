//! Terminating runs (M2.8): replications that end on an event rather than after a fixed time,
//! against exact results. A deterministic activity network ends at its critical path's length;
//! the completion time of parallel exponential activities has the distribution of their maximum,
//! which the empirical distribution and its Dvoretzky–Kiefer–Wolfowitz band must contain.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, Compiled, Report, compile, run_warnings, summarise};
use stochcraft_model::{Dist, ElementId, Experiment, Model, Problem, Stop};
use stochcraft_random::Rng;
use stochcraft_stats::Ecdf;
use stochcraft_stats::spread::dkw_half_width;

fn compiled(model: &Model) -> Compiled {
    let c = compile(model);
    let Ok(c) = c else { panic!("{c:?}") };
    c
}

fn run(c: &Compiled, seed: u64) -> Report {
    c.run(Rng::new(seed)).unwrap_or_else(|e| panic!("{e}"))
}

fn stopping(stop: Stop, limit: f64) -> Experiment {
    Experiment {
        warmup: 0.0,
        duration: limit,
        replications: 1,
        stop: Some(stop),
        ..Experiment::default()
    }
}

fn count(element: &str, count: u64) -> Stop {
    Stop::Count {
        element: ElementId::unchecked(element),
        count,
    }
}

/// One project: a single entity, at time 0, through an activity network (activity on node):
/// A (3) then B (4) and E (5); C (6) from the start; D (2) after B and C; done after D and E.
/// Paths: A–B–D 9, C–D 8, A–E 8, so it completes at 9.
fn activity_network(limit: f64) -> Model {
    let mut b = Builder::new();
    let start = b.source("start", Dist::Constant(0.0), Some(1));
    let fork = b.parallel("fork");
    let a = b.delay("A", Dist::Constant(3.0));
    let after_a = b.parallel("after_A");
    let bb = b.delay("B", Dist::Constant(4.0));
    let c = b.delay("C", Dist::Constant(6.0));
    let e = b.delay("E", Dist::Constant(5.0));
    let before_d = b.parallel("before_D");
    let d = b.delay("D", Dist::Constant(2.0));
    let finish = b.parallel("finish");
    let done = b.sink("done");
    b.connect(start, fork);
    b.connect(fork, a);
    b.connect(fork, c);
    b.connect(a, after_a);
    b.connect(after_a, bb);
    b.connect(after_a, e);
    b.connect(bb, before_d);
    b.connect(c, before_d);
    b.connect(before_d, d);
    b.connect(d, finish);
    b.connect(e, finish);
    b.connect(finish, done);
    b.build(stopping(count("done", 1), limit))
}

#[test]
fn an_activity_network_ends_at_its_critical_path() {
    let c = compiled(&activity_network(1_000.0));
    let r = run(&c, 1);
    assert_eq!(r.stopped_at, Some(9.0));
    assert_eq!(r.duration, 9.0);
    let done = r.element("done").unwrap_or_else(|| panic!("no sink"));
    assert_eq!(done.entered, 1);
    assert_eq!(done.mean_time_in_system, Some(9.0));
    // Activity D was busy for 2 of the 9 time units.
    assert_eq!(
        r.element("D").and_then(|d| d.mean_contents),
        Some(2.0 / 9.0)
    );
}

#[test]
fn a_run_that_does_not_meet_its_stop_ends_at_its_limit() {
    // A limit before the critical path: censored there.
    let c = compiled(&activity_network(7.5));
    let r = run(&c, 1);
    assert_eq!(r.stopped_at, None);
    assert_eq!(r.duration, 7.5);
    assert_eq!(r.element("done").map(|d| d.entered), Some(0));
}

/// k activities in parallel with exponential durations of mean `mean`, from time 0.
fn parallel_exponentials(k: usize, mean: f64) -> Model {
    let mut b = Builder::new();
    let start = b.source("start", Dist::Constant(0.0), Some(1));
    let fork = b.parallel("fork");
    let join = b.parallel("join");
    let done = b.sink("done");
    b.connect(start, fork);
    for i in 0..k {
        let t = b.delay(&format!("task{i}"), Dist::Exponential { mean });
        b.connect(fork, t);
        b.connect(t, join);
    }
    b.connect(join, done);
    b.build(stopping(count("done", 1), 1e9))
}

/// The completion time of three exponential activities of mean 10 is their maximum:
/// P(T ≤ t) = (1 − e^(−t/10))³, mean 10 (1 + 1/2 + 1/3).
fn max_of_three(t: f64) -> f64 {
    let p = 1.0 - stochcraft_random::math::exp(-t / 10.0);
    p * p * p
}

#[test]
fn the_completion_of_parallel_exponentials_is_their_maximum() {
    let c = compiled(&parallel_exponentials(3, 10.0));
    let reports: Vec<Report> = (0..2_000).map(|i| run(&c, 80_000 + i)).collect();
    let s = summarise(&reports);
    let Some((met, time)) = s.stopped else {
        panic!("no stop times")
    };
    assert_eq!(met, 2_000);
    let mean = 10.0 * (1.0 + 0.5 + 1.0 / 3.0);
    assert!(time.contains(mean), "{time} vs {mean}");
    // The whole distribution: within the 95% DKW band at every step.
    let times: Vec<f64> = reports.iter().filter_map(|r| r.stopped_at).collect();
    let ecdf = Ecdf::new(&times);
    let eps = ecdf.dkw_half_width(0.95).unwrap_or(0.0);
    let worst = ecdf
        .finite()
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let below = i as f64 / 2_000.0;
            let at = (i + 1) as f64 / 2_000.0;
            (max_of_three(t) - below)
                .abs()
                .max((max_of_three(t) - at).abs())
        })
        .fold(0.0, f64::max);
    assert!(worst <= eps, "sup |F_n − F| = {worst} > ε = {eps}");
}

#[test]
fn the_dkw_band_covers_the_true_distribution_at_least_as_often_as_it_says() {
    // 200 experiments of 50 replications each: the 95% band should contain the true
    // distribution everywhere in at least 95% of them (the bound is conservative).
    let c = compiled(&parallel_exponentials(3, 10.0));
    let n = 50;
    let eps = dkw_half_width(n, 0.95).unwrap_or(0.0);
    let covered = (0..200u64)
        .filter(|x| {
            let mut times: Vec<f64> = (0..n as u64)
                .filter_map(|i| run(&c, 1_000_000 + x * 1_000 + i).stopped_at)
                .collect();
            times.sort_by(f64::total_cmp);
            times.iter().enumerate().all(|(i, &t)| {
                let f = max_of_three(t);
                (f - i as f64 / n as f64).abs() <= eps
                    && (f - (i + 1) as f64 / n as f64).abs() <= eps
            })
        })
        .count();
    assert!(covered >= 186, "covered in {covered} of 200");
}

#[test]
fn a_task_emptying_stops_the_run() {
    // 20 arrivals a time unit apart from time 1, served in 2 each: the queue builds, and the
    // server empties when the last finishes, at 1 + 20 × 2 = 41.
    let mut b = Builder::new();
    let src = b.source("orders", Dist::Constant(1.0), Some(20));
    let desk = b.server("desk", 1, Dist::Constant(2.0));
    let out = b.sink("out");
    b.connect(src, desk);
    b.connect(desk, out);
    let model = b.build(stopping(
        Stop::Empty {
            element: ElementId::unchecked("desk"),
        },
        1_000.0,
    ));
    let r = run(&compiled(&model), 3);
    assert_eq!(r.stopped_at, Some(41.0));
    assert_eq!(r.element("out").map(|o| o.entered), Some(20));
}

#[test]
fn a_stop_that_is_never_met_changes_nothing_else() {
    // The same replication with and without a stop condition it never meets.
    let model = |stop: Option<Stop>| {
        let mut b = Builder::new();
        let src = b.source("arrivals", Dist::Exponential { mean: 1.0 }, None);
        let desk = b.server("desk", 1, Dist::Exponential { mean: 0.8 });
        let out = b.sink("out");
        b.connect(src, desk);
        b.connect(desk, out);
        b.build(Experiment {
            warmup: 50.0,
            duration: 500.0,
            replications: 1,
            stop,
            ..Experiment::default()
        })
    };
    let plain = run(&compiled(&model(None)), 9);
    let stopping = run(&compiled(&model(Some(count("out", 1_000_000)))), 9);
    assert_eq!(plain, stopping);
    // Counted from time 0, warm-up included: the 100th entity out ends it.
    let r = run(&compiled(&model(Some(count("out", 100)))), 9);
    let at = r.stopped_at.unwrap_or_else(|| panic!("did not stop"));
    assert!(at > 50.0 && at < 550.0, "{at}");
    let out = r.element("out").map_or(0, |o| o.entered);
    assert!(out < 100, "{out} counted after the warm-up");
}

#[test]
fn a_live_run_ends_at_its_stop() {
    let c = compiled(&activity_network(1_000.0));
    let mut live = c.live(Rng::new(1));
    let mut steps = 0;
    while live.step().unwrap_or_else(|e| panic!("{e}")).is_some() {
        steps += 1;
    }
    assert!(live.is_done());
    assert_eq!(
        (live.now(), live.end(), live.stopped_at()),
        (9.0, 9.0, Some(9.0))
    );
    assert!(steps > 3);
    assert_eq!(live.report(), run(&c, 1));
}

/// A stop met before the warm-up ends (#67): warm-up 100, stop once 5 entities have left.
#[test]
fn a_stop_inside_the_warm_up_leaves_the_replication_out() {
    let model = |count_out: u64| {
        let mut b = Builder::new();
        let src = b.source("arrivals", Dist::Exponential { mean: 1.0 }, None);
        let desk = b.server("desk", 1, Dist::Exponential { mean: 0.8 });
        let out = b.sink("out");
        b.connect(src, desk);
        b.connect(desk, out);
        b.build(Experiment {
            warmup: 100.0,
            duration: 3000.0,
            replications: 1,
            stop: Some(count("out", count_out)),
            ..Experiment::default()
        })
    };
    let c = compiled(&model(5));
    assert!(
        c.warnings()
            .iter()
            .any(|d| d.problem == Problem::StopWithWarmup),
        "{:?}",
        c.warnings()
    );
    let reports: Vec<Report> = (0..8).map(|i| run(&c, 500 + i)).collect();
    for r in &reports {
        let at = r.stopped_at.unwrap_or_else(|| panic!("did not stop"));
        assert!(at < 100.0, "{at}");
        // Never reset: what it measured runs from 0 to the stop, and is not used.
        assert_eq!((r.measured_from, r.measured_to), (0.0, at));
        assert!(!r.after_warmup);
    }
    let s = summarise(&reports);
    assert!(s.elements.iter().all(|e| e.metrics.is_empty()), "{s:?}");
    assert_eq!(s.stopped.as_ref().map(|(n, _)| *n), Some(8));
    assert_eq!(s.mean_measured(), None);
    let problems: Vec<Problem> = run_warnings(c.model(), &s)
        .into_iter()
        .map(|d| d.problem)
        .collect();
    assert_eq!(
        problems,
        vec![Problem::EndedInWarmup {
            replications: 8,
            of: 8
        }]
    );
    // Stopping after the warm-up: measured from its end to the stop, and used.
    let c = compiled(&model(300));
    let r = run(&c, 1);
    let at = r.stopped_at.unwrap_or_else(|| panic!("did not stop"));
    assert_eq!((r.measured_from, r.measured_to), (100.0, at));
    assert!(r.after_warmup);
    let s = summarise(&[r]);
    assert!(run_warnings(c.model(), &s).is_empty());
}
