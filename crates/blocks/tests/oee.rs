//! ISO 22400-2's OEE elements against results worked by hand: availability is the time producing
//! over the time planned to be busy (off shift is planned downtime, so it does not lower it, but
//! setups, starving and failures do); effectiveness compares the planned run time per item with
//! the time taken; the setup ratio is the time setting up over the time setting up and
//! producing.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, Event, Report, Rule, Setup, compile};
use stochcraft_model::{DateTime, Dist, Experiment};
use stochcraft_random::Rng;

fn element(r: &Report, id: &str) -> stochcraft_blocks::ElementReport {
    r.element(id)
        .cloned()
        .unwrap_or_else(|| panic!("no element {id}"))
}

fn close(got: Option<f64>, want: f64, tol: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() <= tol),
        "{got:?} vs {want} ± {tol}"
    );
}

/// A part every 10 minutes; the machine sets up for 2 then works 3, against a plan of 2.5 a
/// part. It produces 30% of the time (availability 0.3), takes 3 where 2.5 was planned
/// (effectiveness 2.5 / 3) and sets up for 2 of every 5 minutes it is at work (setup ratio 0.4).
#[test]
fn the_oee_elements_match_a_hand_worked_machine() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    let m = b.server("machine", 1, Dist::Constant(3.0));
    b.setup(m, Setup::every_job(Dist::Constant(2.0)));
    b.planned_run_time(m, Dist::Constant(2.5));
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(1), 0.0, 100_000.0) else {
        panic!("no report")
    };
    let e = element(&r, "machine");
    close(e.oee_availability, 0.3, 1e-3);
    close(e.oee_effectiveness, 2.5 / 3.0, 1e-3);
    close(e.setup_ratio, 0.4, 1e-9);
}

/// Without a plan there is no effectiveness, and without setups no setup ratio; availability is
/// always there.
#[test]
fn effectiveness_needs_a_plan() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    let m = b.server("machine", 1, Dist::Constant(4.0));
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(1), 0.0, 100_000.0) else {
        panic!("no report")
    };
    let e = element(&r, "machine");
    close(e.oee_availability, 0.4, 1e-3);
    assert_eq!(e.oee_effectiveness, None);
    assert_eq!(e.setup_ratio, None);
}

fn at(day: u32, h: u32, m: u32) -> DateTime {
    DateTime::new(2016, 1, day, h, m, 0).unwrap_or(DateTime(0))
}

/// The machine is on shift 8:00 to 16:00 only, and parts are always waiting. It works the whole
/// shift and nothing else: a third of the day producing, two thirds off shift. Off shift is
/// planned downtime, so availability is 1; 480 jobs of 1 minute against a plan of 0.8 give
/// effectiveness 0.8.
#[test]
fn off_shift_is_planned_downtime() {
    let mut b = Builder::new();
    b.calendar(
        "day",
        vec![Event {
            start: at(1, 8, 0),
            end: at(1, 16, 0),
            rule: Rule::parse("FREQ=DAILY").ok(),
        }],
    );
    let src = b.source("parts", Dist::Constant(0.5), None);
    let m = b.server("machine", 1, Dist::Constant(1.0));
    b.availability(m, false, &[("day", true)]);
    b.planned_run_time(m, Dist::Constant(0.8));
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let model = b.build(Experiment {
        duration: 1440.0,
        start: Some(at(1, 0, 0)),
        ..Experiment::default()
    });
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    let r = c
        .first_arrival_at_zero(true)
        .run(Rng::new(1))
        .unwrap_or_else(|e| panic!("{e}"));
    let e = element(&r, "machine");
    assert_eq!(e.left, 480, "{e:?}");
    close(e.time_processing, 1.0 / 3.0, 1e-9);
    close(e.oee_availability, 1.0, 1e-9);
    close(e.oee_effectiveness, 0.8, 1e-9);
}

/// A job done but held at the end, blocked by a full next element, counts as produced: the
/// machine finishes two one-minute jobs against a plan of 1, sends the first on and holds the
/// second, so effectiveness is 1.
#[test]
fn a_finished_job_held_while_blocked_counts_as_produced() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(0.5), Some(2));
    let m = b.server("machine", 1, Dist::Constant(1.0));
    b.planned_run_time(m, Dist::Constant(1.0));
    let next = b.buffered_server("next", 1, Dist::Constant(1.0e9), 0);
    b.connect(src, m);
    b.connect(m, next);
    let out = b.sink("out");
    b.connect(next, out);
    let Ok(r) = b.run(Rng::new(1), 0.0, 10.0) else {
        panic!("no report")
    };
    let e = element(&r, "machine");
    assert_eq!(e.left, 1, "{e:?}");
    close(e.oee_effectiveness, 1.0, 1e-12);
}

/// A plan whose mean is not above 0 is refused before the run.
#[test]
fn a_plan_must_have_a_positive_mean() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    let m = b.server("machine", 1, Dist::Constant(4.0));
    b.planned_run_time(
        m,
        Dist::Normal {
            mean: -1.0,
            std_dev: 0.5,
        },
    );
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    assert!(b.run(Rng::new(1), 0.0, 100.0).is_err());
}
