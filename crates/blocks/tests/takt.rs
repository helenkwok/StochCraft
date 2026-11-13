//! Takt at a sink with a customer demand, against results worked by hand: the time available
//! (the measured window, or a calendar's active time in it) over the demand over it, per run,
//! day, week or shift; and each server's cycle time to compare with it.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, Demand, DemandPer, ElementReport, Event, Report, Rule, compile};
use stochcraft_model::{DateTime, Dist, ElementId, Experiment};
use stochcraft_random::Rng;

fn element(r: &Report, id: &str) -> ElementReport {
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

fn demand(quantity: f64, per: DemandPer, calendar: Option<&str>) -> Demand {
    Demand {
        quantity,
        per,
        calendar: calendar.map(ElementId::unchecked),
    }
}

/// 1000 parts wanted over a run of 10 000 minutes: takt 10 minutes. A part every 2 minutes
/// arrives, five times the demand. The machine takes 1 minute a part: its cycle time.
#[test]
fn takt_per_run_is_the_window_over_the_demand() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(2.0), None);
    let m = b.server("machine", 1, Dist::Constant(1.0));
    let out = b.sink("out");
    b.demand(out, demand(1000.0, DemandPer::Run, None));
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(1), 0.0, 10_000.0) else {
        panic!("no report")
    };
    let o = element(&r, "out");
    close(o.takt, 10.0, 1e-12);
    close(o.demand, 1000.0, 1e-12);
    close(o.demand_met, 5.0, 0.01);
    close(element(&r, "machine").cycle_time, 1.0, 1e-12);
}

/// 144 parts a day, in minutes: takt 1440 / 144 = 10 minutes, whatever the run length; two
/// units of 4 minutes each give a cycle time of 2.
#[test]
fn takt_per_day_and_cycle_time_over_units() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(5.0), None);
    let m = b.server("machine", 2, Dist::Constant(4.0));
    let out = b.sink("out");
    b.demand(out, demand(144.0, DemandPer::Day, None));
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(1), 0.0, 3.5 * 1440.0) else {
        panic!("no report")
    };
    close(element(&r, "out").takt, 10.0, 1e-9);
    close(element(&r, "out").demand, 3.5 * 144.0, 1e-9);
    close(element(&r, "machine").cycle_time, 2.0, 1e-12);
}

fn at(day: u32, h: u32, m: u32) -> DateTime {
    DateTime::new(2016, 1, day, h, m, 0).unwrap_or(DateTime(0))
}

/// 48 parts a shift, the shift 8:00 to 16:00 every day, over three days: 1440 minutes on shift
/// for 144 parts, takt 10 minutes. Without the calendar's open time a day would count.
#[test]
fn takt_per_shift_uses_the_calendar() {
    let mut b = Builder::new();
    b.calendar(
        "day-shift",
        vec![Event {
            start: at(1, 8, 0),
            end: at(1, 16, 0),
            rule: Rule::parse("FREQ=DAILY").ok(),
        }],
    );
    let src = b.source("parts", Dist::Constant(5.0), None);
    let m = b.server("machine", 1, Dist::Constant(1.0));
    let out = b.sink("out");
    b.demand(out, demand(48.0, DemandPer::Shift, Some("day-shift")));
    b.connect(src, m);
    b.connect(m, out);
    let model = b.build(Experiment {
        duration: 3.0 * 1440.0,
        start: Some(at(1, 0, 0)),
        ..Experiment::default()
    });
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    let r = c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
    let o = element(&r, "out");
    close(o.demand, 144.0, 1e-9);
    close(o.takt, 10.0, 1e-9);
}

/// Without a demand anywhere, no takt and no cycle times.
#[test]
fn no_demand_no_takt() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(2.0), None);
    let m = b.server("machine", 1, Dist::Constant(1.0));
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(1), 0.0, 100.0) else {
        panic!("no report")
    };
    assert_eq!(element(&r, "out").takt, None);
    assert_eq!(element(&r, "machine").cycle_time, None);
}

/// A demand per shift needs a calendar.
#[test]
fn a_demand_per_shift_needs_a_calendar() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(2.0), None);
    let out = b.sink("out");
    b.demand(out, demand(10.0, DemandPer::Shift, None));
    b.connect(src, out);
    assert!(b.run(Rng::new(1), 0.0, 100.0).is_err());
}
