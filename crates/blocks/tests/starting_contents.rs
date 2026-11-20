//! Starting contents (ROADMAP M2.9, #225): runs that start with entities already inside blocks
//! instead of empty, against results worked by hand. Entities placed in a task at the start
//! join its queue at time 0 in order: as many as there are free units start a fresh service at
//! once, the rest wait.
#![allow(clippy::panic)]

use stochcraft_blocks::checks::check_report;
use stochcraft_blocks::{Builder, Compiled, Metric, Report, compile, summarise};
use stochcraft_model::{Dist, ElementId, Experiment, Model, Stop};
use stochcraft_random::Rng;

fn compiled(model: &Model) -> Compiled {
    let c = compile(model);
    let Ok(c) = c else { panic!("{c:?}") };
    c
}

fn run(model: &Model, seed: u64) -> Report {
    compiled(model)
        .run(Rng::new(seed))
        .unwrap_or_else(|e| panic!("{e}"))
}

fn experiment(warmup: f64, duration: f64, stop: Option<Stop>) -> Experiment {
    Experiment {
        warmup,
        duration,
        replications: 1,
        stop,
        ..Experiment::default()
    }
}

fn start(model: &mut Model, id: &str, n: u32) {
    model.starting.insert(ElementId::unchecked(id), n);
}

/// A task with `units` units serving in exactly 2 time units, with `n` entities inside at the
/// start and no arrivals: it serves them in waves and is empty after ⌈n / units⌉ × 2.
fn emptying(units: u32, n: u32, x: Experiment) -> Model {
    let mut b = Builder::new();
    let desk = b.server("desk", units, Dist::Constant(2.0));
    let out = b.sink("out");
    b.connect(desk, out);
    let mut m = b.build(x);
    start(&mut m, "desk", n);
    m
}

#[test]
fn a_task_started_full_empties_when_the_last_is_served() {
    let stop = || {
        Some(Stop::Empty {
            element: ElementId::unchecked("desk"),
        })
    };
    // No source: the block with starting contents is where entities come from.
    let m = emptying(1, 5, experiment(0.0, 100.0, stop()));
    assert_eq!(m.validate(), vec![]);
    let r = run(&m, 1);
    // M/D/1 started with 5 and no arrivals: 5 × 2.
    assert_eq!(r.stopped_at, Some(10.0));
    let desk = r.element("desk").unwrap_or_else(|| panic!("no desk"));
    assert_eq!((desk.started, desk.entered, desk.left), (Some(5), 5, 5));
    assert_eq!(r.element("out").map(|o| o.entered), Some(5));
    // Two units take two each at once: ⌈5 / 2⌉ × 2.
    let r = run(&emptying(2, 5, experiment(0.0, 100.0, stop())), 1);
    assert_eq!(r.stopped_at, Some(6.0));
}

#[test]
fn entities_started_inside_wait_and_are_served_in_order() {
    // Over 100 time units with one unit: waits 0, 2, 4, 6 and 8 (mean 4), busy 10 of 100, and
    // the entities count from time 0 in their time in the system (2, 4, …, 10: mean 6).
    let m = emptying(1, 5, experiment(0.0, 100.0, None));
    let r = run(&m, 1);
    let close = |got: Option<f64>, want: f64| {
        assert!(
            got.is_some_and(|g| (g - want).abs() < 1e-12),
            "{got:?} vs {want}"
        );
    };
    let desk = r.element("desk").unwrap_or_else(|| panic!("no desk"));
    close(desk.mean_wait, 4.0);
    close(desk.utilisation, 0.1);
    assert_eq!(desk.work_in_progress, Some(0));
    let out = r.element("out").unwrap_or_else(|| panic!("no out"));
    close(out.mean_time_in_system, 6.0);
    // Every count balances, in the block and in the model.
    assert_eq!(check_report(&m, &r), vec![]);
    let s = summarise(&[r]);
    assert_eq!(s.get("desk", Metric::Started).map(|x| x.mean), Some(5.0));
    // Blocks with none report no such result.
    assert_eq!(s.get("out", Metric::Started), None);
}

#[test]
fn after_a_warm_up_those_placed_at_the_start_count_as_inside() {
    // At 5, two have left (at 2 and 4), one is in service and two wait: three inside as the
    // statistics start, none placed after it, and the three leave at 6, 8 and 10.
    let m = emptying(1, 5, experiment(5.0, 95.0, None));
    let r = run(&m, 1);
    let desk = r.element("desk").unwrap_or_else(|| panic!("no desk"));
    assert_eq!(desk.started, None);
    assert_eq!(desk.inside_at_start, 3);
    assert_eq!((desk.entered, desk.left), (0, 3));
    assert_eq!(check_report(&m, &r), vec![]);
}

#[test]
fn starting_contents_balance_with_arrivals_delays_and_batches() {
    // A random line with entities inside its task, its delay and its batch at the start, and
    // arrivals: every block and the model balance.
    let mut b = Builder::new();
    let src = b.source("arrivals", Dist::Exponential { mean: 1.0 }, None);
    let desk = b.server("desk", 2, Dist::Exponential { mean: 1.5 });
    let walk = b.delay("walk", Dist::Exponential { mean: 3.0 });
    let pack = b.batch("pack", 4);
    let out = b.sink("out");
    b.connect(src, desk);
    b.connect(desk, walk);
    b.connect(walk, pack);
    b.connect(pack, out);
    let mut m = b.build(experiment(0.0, 200.0, None));
    start(&mut m, "desk", 7);
    start(&mut m, "walk", 3);
    start(&mut m, "pack", 6);
    assert_eq!(m.validate(), vec![]);
    for seed in 1..=5 {
        let r = run(&m, seed);
        assert_eq!(check_report(&m, &r), vec![], "seed {seed}");
        for (id, n) in [("desk", 7), ("walk", 3), ("pack", 6)] {
            let e = r.element(id).unwrap_or_else(|| panic!("no {id}"));
            assert_eq!(e.started, Some(n), "{id}");
            assert!(e.entered >= n, "{id}: {}", e.entered);
        }
    }
    // The batch of 4 goes at once with 4 of the 6, at time 0.
    m.starting.clear();
    start(&mut m, "pack", 6);
    m.experiment.duration = 1e-9;
    let r = run(&m, 1);
    assert_eq!(r.element("out").map(|o| o.entered), Some(1));
    assert_eq!(r.element("pack").map(|p| p.inside_at_end), Some(2));
}

#[test]
fn a_full_waiting_room_at_the_start_turns_arrivals_away() {
    // One unit and 2 places waiting, all three taken at the start; service 2, an arrival
    // every 0.5 from 0.5: those at 0.5, 1 and 1.5 find no room and are lost.
    let mut b = Builder::new();
    let src = b.source("arrivals", Dist::Constant(0.5), None);
    let desk = b.buffered_server("desk", 1, Dist::Constant(2.0), 2);
    let out = b.sink("out");
    b.connect(src, desk);
    b.connect(desk, out);
    let mut m = b.build(experiment(0.0, 1.9, None));
    start(&mut m, "desk", 3);
    assert_eq!(m.validate(), vec![]);
    let r = run(&m, 1);
    let desk = r.element("desk").unwrap_or_else(|| panic!("no desk"));
    assert_eq!(desk.lost, Some(3));
    assert_eq!((desk.started, desk.entered), (Some(3), 3));
    assert_eq!(check_report(&m, &r), vec![]);
    // More than the room holds is refused.
    start(&mut m, "desk", 4);
    assert!(compile(&m).is_err());
}

#[test]
fn starting_contents_leave_every_other_blocks_draws_as_they_were() {
    // Two lines side by side: entities inside the first line's task at the start change
    // nothing on the second line, nor the first line's arrivals (each block draws from its own
    // streams).
    let model = |starting: bool| {
        let mut b = Builder::new();
        for line in ["a", "b"] {
            let src = b.source(&format!("in-{line}"), Dist::Exponential { mean: 1.0 }, None);
            let desk = b.server(&format!("desk-{line}"), 1, Dist::Exponential { mean: 0.8 });
            let out = b.sink(&format!("out-{line}"));
            b.connect(src, desk);
            b.connect(desk, out);
        }
        let mut m = b.build(experiment(0.0, 500.0, None));
        if starting {
            start(&mut m, "desk-a", 4);
        }
        m
    };
    for seed in [3, 11] {
        let (plain, started) = (run(&model(false), seed), run(&model(true), seed));
        for id in ["in-a", "in-b", "desk-b", "out-b"] {
            assert_eq!(plain.element(id), started.element(id), "{id}");
        }
        assert_ne!(plain.element("desk-a"), started.element("desk-a"));
    }
}
