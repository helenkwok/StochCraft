//! Calendars against exact results: arrivals whose rate changes by time of day (a
//! non-homogeneous Poisson process, whose count over any interval is Poisson with the integral of
//! the rate as its mean), a rate change taking effect at once, and pools whose units follow
//! shifts, worked by hand.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, Event, Metric, Rule, compile, summarise};
use stochcraft_model::{DateTime, Dist, Experiment, Need};
use stochcraft_random::Rng;

const REPS: u64 = 16;
/// The seed is fixed so the tests are repeatable; each check is a 95% interval.
const SEED: u64 = 60_000;

fn at(day: u32, h: u32, m: u32) -> DateTime {
    DateTime::new(2016, 1, day, h, m, 0).unwrap_or(DateTime(0))
}

/// Every day from 1 January 2016, `from` to `to` (hours and minutes).
fn daily(from: (u32, u32), to: (u32, u32)) -> Event {
    Event {
        start: at(1, from.0, from.1),
        end: at(1, to.0, to.1),
        rule: Rule::parse("FREQ=DAILY").ok(),
    }
}

fn experiment(warmup: f64, duration: f64) -> Experiment {
    Experiment {
        warmup,
        duration,
        replications: REPS as u32,
        start: Some(at(1, 0, 0)),
        ..Experiment::default()
    }
}

/// Poisson arrivals at 0.5 a minute, and at 2 a minute from 8 to noon every day. Over a whole
/// day the count has mean 240 × 2 + 1200 × 0.5 = 1080; over the morning peak alone (8 to noon,
/// after a warm-up to 8), 480; over the afternoon and evening (noon to midnight), 360.
#[test]
fn arrivals_follow_the_rate_of_each_time_of_day() {
    let build = |warmup: f64, duration: f64| {
        let mut b = Builder::new();
        b.calendar("peak", vec![daily((8, 0), (12, 0))]);
        let src = b.source("calls", Dist::exponential_rate(0.5), None);
        b.timed_arrivals(src, vec![("peak", Dist::exponential_rate(2.0))], false);
        let out = b.sink("done");
        b.connect(src, out);
        b.build(experiment(warmup, duration))
    };
    for (warmup, duration, mean) in [
        (0.0, 1440.0, 1080.0),
        (480.0, 240.0, 480.0),
        (720.0, 720.0, 360.0),
    ] {
        let model = build(warmup, duration);
        let compiled = compile(&model);
        let Ok(compiled) = compiled else {
            panic!("{compiled:?}")
        };
        let reports: Vec<_> = (0..REPS)
            .map(|r| compiled.run(Rng::new(SEED + r)))
            .collect();
        let s = summarise(&reports);
        let got = s.get("calls", Metric::Entered);
        eprintln!("[{warmup}, {}): {got:?} (theory {mean})", warmup + duration);
        assert!(
            got.is_some_and(|g| g.contains(mean)),
            "[{warmup}, {}): {got:?} vs {mean}",
            warmup + duration
        );
    }
}

/// A new rate applies at once: with an arrival every 10 minutes, and every minute from 00:25 to
/// 00:50, arrivals come at 10 and 20, then (the one due at 30 dropped at the change) every minute
/// from 26 to 50, then every 10 from the change back at 50: at 60. 28 by 00:65.
#[test]
fn a_change_of_rate_takes_effect_at_once() {
    let mut b = Builder::new();
    b.calendar(
        "rush",
        vec![Event {
            start: at(1, 0, 25),
            end: at(1, 0, 50),
            rule: None,
        }],
    );
    let src = b.source("in", Dist::Constant(10.0), None);
    b.timed_arrivals(src, vec![("rush", Dist::Constant(1.0))], false);
    let out = b.sink("out");
    b.connect(src, out);
    let mut model = b.build(experiment(0.0, 65.0));
    model.experiment.replications = 1;
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    let r = c.run(Rng::new(1));
    assert_eq!(r.element("in").map(|e| e.entered), Some(28), "{r:?}");

    // With no value outside the calendar, there are arrivals only in it: 26 to 50.
    let mut b = Builder::new();
    b.calendar(
        "rush",
        vec![Event {
            start: at(1, 0, 25),
            end: at(1, 0, 50),
            rule: None,
        }],
    );
    let src = b.source("in", Dist::Constant(10.0), None);
    b.timed_arrivals(src, vec![("rush", Dist::Constant(1.0))], true);
    let out = b.sink("out");
    b.connect(src, out);
    let model = b.build(experiment(0.0, 120.0));
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    let r = c.run(Rng::new(1));
    assert_eq!(r.element("in").map(|e| e.entered), Some(25), "{r:?}");
}

/// A pool with 1 unit, and 3 from 8 to 4 pm, kept busy by more work than it can do (a job every
/// 0.1 minutes from time 0, each taking 1): in a day each unit finishes a job a minute while it is there,
/// 1 × 1440 + 2 × 480 = 2400 jobs, and the units are always busy.
#[test]
fn a_pool_has_the_units_of_its_shift() {
    let mut b = Builder::new();
    b.calendar("day", vec![daily((8, 0), (16, 0))]);
    b.pool("crew", 1, &[]);
    b.shifts("crew", &[("day", 3)]);
    let src = b.source("jobs", Dist::Constant(0.1), None);
    let t = b.shared_task(
        "work",
        Dist::Constant(1.0),
        vec![Need::Resource {
            id: stochcraft_model::ElementId::unchecked("crew"),
            quantity: 1,
        }],
        0.0,
    );
    let out = b.sink("out");
    b.connect(src, t);
    b.connect(t, out);
    let model = b.build(experiment(0.0, 1440.0));
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    // The first job at 0, so the unit is busy from the start.
    let r = c.first_arrival_at_zero(true).run(Rng::new(1));
    assert_eq!(r.element("work").map(|e| e.left), Some(2400), "{r:?}");
    let util = r.element("crew").and_then(|e| e.utilisation);
    assert!(util.is_some_and(|u| (u - 1.0).abs() < 1e-9), "{util:?}");
    // Mean units busy: 1 for 16 hours and 3 for 8, over the day.
    let busy = r.element("crew").and_then(|e| e.mean_contents);
    assert!(
        busy.is_some_and(|b| (b - 5.0 / 3.0).abs() < 1e-9),
        "{busy:?}"
    );
}

/// When a shift ends, a unit in use finishes its job before it goes; nobody else starts. With
/// one unit only from 00:00 to 00:10 and jobs of 30 minutes arriving at 5 and 10, the first is
/// done at 35 and the second never starts.
#[test]
fn a_unit_finishes_its_job_when_its_shift_ends() {
    let mut b = Builder::new();
    b.calendar(
        "early",
        vec![Event {
            start: at(1, 0, 0),
            end: at(1, 0, 10),
            rule: None,
        }],
    );
    b.pool("crew", 0, &[]);
    b.shifts("crew", &[("early", 1)]);
    let src = b.source("jobs", Dist::Constant(5.0), Some(2));
    // Arrivals at 5 and 10; the second arrives as the shift ends and so never starts.
    let t = b.shared_task(
        "work",
        Dist::Constant(30.0),
        vec![Need::Resource {
            id: stochcraft_model::ElementId::unchecked("crew"),
            quantity: 1,
        }],
        0.0,
    );
    let out = b.sink("out");
    b.connect(src, t);
    b.connect(t, out);
    let model = b.build(experiment(0.0, 100.0));
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    let r = c.run(Rng::new(1));
    assert_eq!(r.element("work").map(|e| e.left), Some(1), "{r:?}");
    assert_eq!(
        r.element("out").and_then(|e| e.mean_time_in_system),
        Some(30.0),
        "{r:?}"
    );
    // The unit was busy for 30 minutes and there for 35 (its shift, then the end of its job),
    // over the 100.
    let crew = r.element("crew");
    let busy = crew.and_then(|e| e.mean_contents);
    assert!(busy.is_some_and(|b| (b - 0.3).abs() < 1e-9), "{busy:?}");
    let util = crew.and_then(|e| e.utilisation);
    assert!(
        util.is_some_and(|u| (u - 30.0 / 35.0).abs() < 1e-9),
        "{util:?}"
    );
}
