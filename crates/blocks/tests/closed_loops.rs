//! Closed loops (#235): models whose entities go round for ever, a fixed number of them, as in a
//! closed queueing network or a finite-source model (machines that fail and wait for a fitter,
//! M/M/c//N). Such a model needs no exit, so the model's checks do not ask for one
//! ([`Model::is_closed`]), and its entities never leave, so the behaviour checks look for
//! entities that stopped moving instead of for none leaving.
#![allow(clippy::panic)]

use stochcraft_blocks::checks::check_report;
use stochcraft_blocks::{Builder, Report, compile};
use stochcraft_model::{Dist, ElementId, Experiment, Model};
use stochcraft_random::Rng;

fn experiment() -> Experiment {
    Experiment {
        duration: 100.0,
        replications: 1,
        ..Experiment::default()
    }
}

fn run(model: &Model, seed: u64) -> Report {
    let c = compile(model);
    let Ok(c) = c else { panic!("{c:?}") };
    c.run(Rng::new(seed)).unwrap_or_else(|e| panic!("{e}"))
}

fn codes(model: &Model) -> Vec<&'static str> {
    model.validate().iter().map(|d| d.problem.code()).collect()
}

fn found(model: &Model, report: &Report) -> Vec<(Option<String>, &'static str)> {
    check_report(model, report)
        .iter()
        .map(|d| (d.at.as_ref().map(|a| a.to_string()), d.problem.code()))
        .collect()
}

/// Machines (`limit` of them, all made at time 0) that run for `running`, are then repaired by
/// one fitter in `repair`, and run again.
fn crew(b: &mut Builder, limit: Option<u64>, running: Dist, repair: Dist) {
    let machines = b.source("machines", Dist::Constant(0.0), limit);
    let up = b.delay("running", running);
    let fix = b.server("repair", 1, repair);
    b.connect(machines, up);
    b.connect(up, fix);
    b.connect(fix, up);
}

#[test]
fn a_closed_loop_needs_no_exit() {
    let mut b = Builder::new();
    crew(&mut b, Some(3), Dist::Constant(1.0), Dist::Constant(0.5));
    let m = b.build(experiment());
    assert!(m.is_closed());
    assert_eq!(m.validate(), vec![]);

    // Entities placed inside at the start, with no source, close a loop too.
    let mut b = Builder::new();
    let up = b.delay("running", Dist::Constant(1.0));
    let fix = b.server("repair", 1, Dist::Constant(0.5));
    b.connect(up, fix);
    b.connect(fix, up);
    let mut m = b.build(experiment());
    m.starting.insert(ElementId::unchecked("running"), 3);
    assert!(m.is_closed());
    assert_eq!(m.validate(), vec![]);
}

#[test]
fn a_model_with_no_exit_that_is_not_a_closed_loop_still_lacks_one() {
    // A source with no limit: the number inside grows without end.
    let mut b = Builder::new();
    crew(&mut b, None, Dist::Constant(1.0), Dist::Constant(0.5));
    let m = b.build(experiment());
    assert!(!m.is_closed());
    assert!(codes(&m).contains(&"no_sink"), "{:?}", codes(&m));

    // Entities that pass a block once and go round elsewhere: `first` is never returned to.
    let mut b = Builder::new();
    let src = b.source("machines", Dist::Constant(0.0), Some(3));
    let first = b.delay("first", Dist::Constant(1.0));
    let up = b.delay("running", Dist::Constant(1.0));
    let fix = b.server("repair", 1, Dist::Constant(0.5));
    b.connect(src, first);
    b.connect(first, up);
    b.connect(up, fix);
    b.connect(fix, up);
    let m = b.build(experiment());
    assert!(!m.is_closed());
    assert!(codes(&m).contains(&"no_sink"), "{:?}", codes(&m));

    // A loop that drains into a second loop and never comes back.
    let mut b = Builder::new();
    let src = b.source("machines", Dist::Constant(0.0), Some(3));
    let a = b.delay("a", Dist::Constant(1.0));
    let fork = b.exclusive("fork");
    let c = b.delay("c", Dist::Constant(1.0));
    let d = b.delay("d", Dist::Constant(1.0));
    b.connect(src, a);
    b.connect(a, fork);
    b.route(fork, a, Some(0.5), None);
    b.route(fork, c, Some(0.5), None);
    b.connect(c, d);
    b.connect(d, c);
    let m = b.build(experiment());
    assert!(!m.is_closed());
    assert!(codes(&m).contains(&"no_sink"), "{:?}", codes(&m));

    // An open line with no exit at all.
    let mut b = Builder::new();
    let src = b.source("arrivals", Dist::Constant(1.0), None);
    let desk = b.server("desk", 1, Dist::Constant(0.5));
    let next = b.delay("next", Dist::Constant(0.5));
    b.connect(src, desk);
    b.connect(desk, next);
    let m = b.build(experiment());
    assert!(!m.is_closed());
    assert!(codes(&m).contains(&"no_sink"), "{:?}", codes(&m));

    // A model with an exit is never closed, and asks for no other.
    let mut b = Builder::new();
    let src = b.source("arrivals", Dist::Constant(1.0), Some(3));
    let desk = b.server("desk", 1, Dist::Constant(0.5));
    let out = b.sink("out");
    b.connect(src, desk);
    b.connect(desk, out);
    let m = b.build(experiment());
    assert!(!m.is_closed());
    assert_eq!(m.validate(), vec![]);
}

#[test]
fn a_closed_loop_that_keeps_moving_raises_no_behaviour_warning() {
    // Fixed times: all three run until 1, are repaired at 1.5, 2 and 2.5, and so on.
    let mut b = Builder::new();
    crew(&mut b, Some(3), Dist::Constant(1.0), Dist::Constant(0.5));
    let m = b.build(experiment());
    assert_eq!(found(&m, &run(&m, 1)), vec![]);
    // Random times, with a warm-up.
    let mut b = Builder::new();
    crew(
        &mut b,
        Some(6),
        Dist::Exponential { mean: 4.0 },
        Dist::Exponential { mean: 1.0 },
    );
    let mut m = b.build(experiment());
    m.experiment.warmup = 20.0;
    for seed in 1..=10 {
        let r = run(&m, seed);
        assert_eq!(found(&m, &r), vec![], "seed {seed}");
    }
}

#[test]
fn a_closed_loop_that_jams_is_found() {
    // The fitter takes longer than the run: each machine runs once (until 1) and then all
    // three wait at the fitter to the end. `running` finished all three, so counting any block's
    // finished entities would miss the jam; `repair`, where they all are, finished none.
    let mut b = Builder::new();
    crew(
        &mut b,
        Some(3),
        Dist::Constant(1.0),
        Dist::Constant(1_000.0),
    );
    let m = b.build(experiment());
    let r = run(&m, 1);
    let at = |id: &str| {
        r.element(id)
            .map(|e| (e.left, e.inside_at_end))
            .unwrap_or_else(|| panic!("no {id}"))
    };
    assert_eq!(at("running"), (3, 0));
    assert_eq!(at("repair"), (0, 3));
    assert_eq!(found(&m, &r), vec![(None, "nothing_moved")]);
    // Long enough, the loop moves on.
    let mut longer = m.clone();
    longer.experiment.duration = 5_000.0;
    assert_eq!(found(&longer, &run(&longer, 1)), vec![]);
}
