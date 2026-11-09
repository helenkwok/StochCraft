//! Calendars against exact results: arrivals whose rate changes by time of day (a
//! non-homogeneous Poisson process, whose count over any interval is Poisson with the integral of
//! the rate as its mean), a rate change taking effect at once, pools whose units follow shifts,
//! and servers whose units follow a calendar and fail, worked by hand and against long-run
//! availability.
#![allow(clippy::panic)]

use stochcraft_blocks::{Breakdown, Builder, Event, Metric, Rule, compile, summarise};
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
            .map(|r| {
                compiled
                    .run(Rng::new(SEED + r))
                    .unwrap_or_else(|e| panic!("{e}"))
            })
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
    let r = c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
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
    let r = c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
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
    let r = c
        .first_arrival_at_zero(true)
        .run(Rng::new(1))
        .unwrap_or_else(|e| panic!("{e}"));
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
    let r = c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
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

fn close(got: Option<f64>, want: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() < 1e-9),
        "{got:?} vs {want}"
    );
}

/// A server of one unit with jobs of `service`, all waiting from time 0, whose unit is off
/// shift from 00:10 to 00:20, failing as `breakdown` says, run for `duration`.
fn server_with_a_break(
    service: f64,
    breakdown: Option<Breakdown>,
    duration: f64,
) -> stochcraft_blocks::Report {
    let mut b = Builder::new();
    b.calendar(
        "break",
        vec![Event {
            start: at(1, 0, 10),
            end: at(1, 0, 20),
            rule: None,
        }],
    );
    let src = b.source("jobs", Dist::Constant(0.0), Some(100));
    let m = b.server("machine", 1, Dist::Constant(service));
    b.availability(m, true, &[("break", false)]);
    if let Some(x) = breakdown {
        b.breakdown(m, x);
    }
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let model = b.build(experiment(0.0, duration));
    assert_eq!(model.validate(), vec![]);
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

/// Jobs of 1.5 and a break from 10 to 20: jobs end at 1.5, 3, …, 9; the one started at 9 is
/// under way at 10 and finishes at 10.5; none starts until 20; then they end at 21.5, …, 29 by
/// 29.5. 13 jobs. The unit is off shift for the whole break (finishing its job counts as off
/// shift: off shift comes before processing), 10 of 29.5, and busy 10.5 + 9.5.
#[test]
fn a_server_stops_taking_jobs_for_its_break() {
    let r = server_with_a_break(1.5, None, 29.5);
    let m = r.element("machine");
    assert_eq!(m.map(|e| e.left), Some(13), "{r:?}");
    close(m.and_then(|e| e.time_off_shift), 10.0 / 29.5);
    close(m.and_then(|e| e.utilisation), 20.0 / 29.5);
    // Units that never fail report no availability.
    assert_eq!(m.and_then(|e| e.availability), None);
}

/// The same break, jobs of 4, a failure 7 after each repair (elapsed time, whatever the unit
/// does), repairs of 5, and failures that interrupt the job:
/// - jobs end at 4; the second is interrupted at 7 with 1 left, and the repair runs 7 to 12,
///   through the start of the break;
/// - the interrupted job resumes at 12 although the break has begun, and ends at 13;
/// - the next failure comes at 19, during the break, and its repair runs to 24, past its end;
/// - the third job runs 24 to 28: three jobs by 30.
///
/// Failed 10 of 30 ([7, 12) and [19, 24)); off shift the rest of the break, 7 (including the
/// resumed job's last minute: off shift comes before processing).
#[test]
fn an_interrupted_job_resumes_after_its_repair_even_off_shift() {
    let r = server_with_a_break(
        4.0,
        Some(Breakdown {
            time_to_failure: Dist::Constant(7.0),
            repair: Dist::Constant(5.0),
            interruptible: true,
        }),
        30.0,
    );
    let m = r.element("machine");
    assert_eq!(m.map(|e| e.left), Some(3), "{r:?}");
    close(m.and_then(|e| e.availability), 1.0 - 10.0 / 30.0);
    close(m.and_then(|e| e.time_off_shift), 7.0 / 30.0);
    // An out at 4, 13 and 28.
    assert_eq!(r.element("out").map(|e| e.entered), Some(3));
}

/// A failing server off shift from midnight to 8 every day, saturated: failures (exponential,
/// mean 100) and repairs (exponential, mean 10) come and go in elapsed time, whatever the
/// calendar, so the unit is up A = 100 / 110 of the time, and up while off shift A / 3 of it;
/// on shift and up, A × 2/3 of the time, it does a job a minute (exponential service, mean 1),
/// failures interrupting them. Over 20 days after a day's warm-up: about 28,800 × 2/3 × A =
/// 17,454.5 jobs, plus the jobs under way at each 8 hours' close that finish after it (about
/// one a day, inside the interval).
#[test]
fn a_failing_server_on_a_calendar_matches_its_long_run_availability() {
    let mut b = Builder::new();
    b.calendar("night", vec![daily((0, 0), (8, 0))]);
    let src = b.source("jobs", Dist::exponential_rate(2.0), None);
    let m = b.server("machine", 1, Dist::exponential_rate(1.0));
    b.availability(m, true, &[("night", false)]);
    b.breakdown(
        m,
        Breakdown {
            time_to_failure: Dist::Exponential { mean: 100.0 },
            repair: Dist::Exponential { mean: 10.0 },
            interruptible: true,
        },
    );
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let duration = 20.0 * 1440.0;
    let model = b.build(experiment(1440.0, duration));
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    let reports: Vec<_> = (0..REPS)
        .map(|r| c.run(Rng::new(SEED + r)).unwrap_or_else(|e| panic!("{e}")))
        .collect();
    let s = summarise(&reports);
    let a = 100.0 / 110.0;
    for (metric, theory) in [
        (Metric::Availability, a),
        (Metric::TimeOffShift, a / 3.0),
        (Metric::Left, duration * 2.0 / 3.0 * a),
    ] {
        let got = s.get("machine", metric);
        eprintln!("{}: {got:?} (theory {theory})", metric.name());
        assert!(
            got.is_some_and(|g| g.n == REPS && g.contains(theory)),
            "{}: {got:?} vs {theory}",
            metric.name()
        );
    }
}
