//! Time in system by percentile at a sink that asks for it (`stochcraft:timeInSystem`), against
//! results worked by hand: Hyndman and Fan's type 7 percentiles of each entity's exact time, the
//! share within a target, the warm-up's reset, and working time over a calendar's active periods.
#![allow(clippy::panic)]

use stochcraft_blocks::{
    Builder, ElementReport, Event, Metric, Report, Rule, TimeInSystem, compile, summarise,
};
use stochcraft_model::{DateTime, Dist, ElementId, Experiment};
use stochcraft_random::Rng;

fn element(r: &Report, id: &str) -> ElementReport {
    r.element(id)
        .cloned()
        .unwrap_or_else(|| panic!("no element {id}"))
}

fn close(got: Option<f64>, want: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() <= 1e-9),
        "{got:?} vs {want}"
    );
}

fn times(within: Option<f64>, calendar: Option<&str>) -> TimeInSystem {
    TimeInSystem {
        within,
        calendar: calendar.map(ElementId::unchecked),
    }
}

/// Four parts a minute apart through a machine of 2 minutes each: they leave after 2, 3, 4 and
/// 5 minutes in the system. Type 7 interpolates at h = (n − 1)p: the median halfway between 3
/// and 4, the 90th percentile 0.7 of the way from 4 to 5, the 95th 0.85. Within 3.5 minutes:
/// two of the four.
#[test]
fn percentiles_and_share_are_those_of_the_exact_times() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(1.0), Some(4));
    let m = b.server("machine", 1, Dist::Constant(2.0));
    let out = b.sink("out");
    b.time_in_system(out, times(Some(3.5), None));
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(1), 0.0, 100.0) else {
        panic!("no report")
    };
    let o = element(&r, "out");
    assert_eq!(o.left, 4);
    close(o.mean_time_in_system, 3.5);
    close(o.time_in_system_p50, 3.5);
    close(o.time_in_system_p90, 4.7);
    close(o.time_in_system_p95, 4.85);
    close(o.within_target_share, 0.5);
    // The summary carries them like any other metric.
    let s = summarise(&[r]);
    assert!(
        s.get("out", Metric::TimeInSystemP90)
            .is_some_and(|x| (x.mean - 4.7).abs() < 1e-9)
    );
    assert!(s.get("out", Metric::WithinTargetShare).is_some());
}

/// Without a target, the percentiles alone; without the extension, none of the four, so models
/// that do not ask report as before.
#[test]
fn only_sinks_that_ask_report_them() {
    let run = |x: Option<TimeInSystem>| {
        let mut b = Builder::new();
        let src = b.source("parts", Dist::Constant(1.0), Some(4));
        let m = b.server("machine", 1, Dist::Constant(2.0));
        let out = b.sink("out");
        if let Some(x) = x {
            b.time_in_system(out, x);
        }
        b.connect(src, m);
        b.connect(m, out);
        let Ok(r) = b.run(Rng::new(1), 0.0, 100.0) else {
            panic!("no report")
        };
        element(&r, "out")
    };
    let percentiles = run(Some(times(None, None)));
    close(percentiles.time_in_system_p50, 3.5);
    assert_eq!(percentiles.within_target_share, None);
    let plain = run(None);
    assert_eq!(plain.time_in_system_p50, None);
    assert_eq!(plain.time_in_system_p90, None);
    assert_eq!(plain.time_in_system_p95, None);
    assert_eq!(plain.within_target_share, None);
    close(plain.mean_time_in_system, 3.5);
}

/// Three parts that go straight out (0 minutes inside) leave by minute 3, before a warm-up of
/// 50; five more spend 100 minutes in a task and leave after it. With the warm-up, only the
/// five count: none is done within 50 minutes. Without it, three of the eight are.
#[test]
fn the_warm_up_clears_the_times() {
    let run = |warmup: f64| {
        let mut b = Builder::new();
        let quick = b.source("quick", Dist::Constant(1.0), Some(3));
        let slow = b.source("slow", Dist::Constant(1.0), Some(5));
        let task = b.delay("task", Dist::Constant(100.0));
        let out = b.sink("out");
        b.time_in_system(out, times(Some(50.0), None));
        b.connect(quick, out);
        b.connect(slow, task);
        b.connect(task, out);
        let Ok(r) = b.run(Rng::new(1), warmup, 200.0) else {
            panic!("no report")
        };
        element(&r, "out")
    };
    let after = run(50.0);
    assert_eq!(after.left, 5);
    close(after.within_target_share, 0.0);
    close(after.time_in_system_p50, 100.0);
    let all = run(0.0);
    assert_eq!(all.left, 8);
    close(all.within_target_share, 3.0 / 8.0);
}

fn at(day: u32, h: u32, m: u32) -> DateTime {
    DateTime::new(2016, 1, day, h, m, 0).unwrap_or(DateTime(0))
}

/// A part made on Friday 4 January 2016 at midnight (a Monday is model time 0, the calendar
/// active Monday to Friday) spends 3½ days in a task and leaves on Monday at noon. On the clock
/// that is 5040 minutes; in working time only Friday (1440) and Monday morning (720) count:
/// 2160, within a target of 2200 minutes. On the clock it is not.
#[test]
fn working_time_leaves_out_the_weekend() {
    let run = |calendar: Option<&str>| {
        let mut b = Builder::new();
        b.calendar(
            "weekdays",
            vec![Event {
                start: at(4, 0, 0),
                end: at(9, 0, 0),
                rule: Rule::parse("FREQ=WEEKLY").ok(),
            }],
        );
        let src = b.source("parts", Dist::Constant(4.0 * 1440.0), Some(1));
        let task = b.delay("task", Dist::Constant(3.5 * 1440.0));
        let out = b.sink("out");
        b.time_in_system(out, times(Some(2200.0), calendar));
        b.connect(src, task);
        b.connect(task, out);
        let model = b.build(Experiment {
            duration: 3.0 * 7.0 * 1440.0,
            start: Some(at(4, 0, 0)),
            ..Experiment::default()
        });
        let Ok(c) = compile(&model) else {
            panic!("does not compile")
        };
        let r = c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
        element(&r, "out")
    };
    let working = run(Some("weekdays"));
    assert_eq!(working.left, 1);
    close(working.time_in_system_p50, 2160.0);
    close(working.within_target_share, 1.0);
    // The mean stays on the clock.
    close(working.mean_time_in_system, 5040.0);
    let clock = run(None);
    close(clock.time_in_system_p50, 5040.0);
    close(clock.within_target_share, 0.0);
}

/// A target of 0 or less, and a calendar that does not exist, are refused.
#[test]
fn a_bad_target_or_calendar_is_refused() {
    for x in [times(Some(0.0), None), times(None, Some("nowhere"))] {
        let mut b = Builder::new();
        let src = b.source("parts", Dist::Constant(2.0), None);
        let out = b.sink("out");
        b.time_in_system(out, x);
        b.connect(src, out);
        assert!(b.run(Rng::new(1), 0.0, 100.0).is_err());
    }
}
