//! Energy per server state and energy per part, worked by hand: power integrated over the exact
//! time each unit spends in each state, cut at the end of the warm-up and of the run, in
//! kilowatt-hours whatever the model's time unit; and energy per part as the mean of the
//! replications' ratios, leaving out (never zeroing) a replication with no output.
#![allow(clippy::panic)]

use stochcraft_blocks::{Breakdown, Builder, Event, Metric, Power, Report, compile, summarise};
use stochcraft_model::{DateTime, Dist, Experiment, TimeUnit};
use stochcraft_random::Rng;

fn close(got: Option<f64>, want: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() < 1e-9 * want.abs().max(1.0)),
        "{got:?} vs {want}"
    );
}

fn power(processing: f64, idle: f64) -> Power {
    Power {
        processing,
        idle,
        blocked: None,
        failed: None,
        off_shift: None,
    }
}

/// One machine (processing 10 kW, idle 2 kW) taking a job of 1 minute every 4 from time 0,
/// measured over [32.5, 92.5) minutes: it processes half of the job [32, 33), the jobs from 36
/// to 88 (14) and half of [92, 93), 15 minutes in all, and is idle 45: (10 × 15 + 2 × 45) / 60 =
/// 4 kWh. 15 parts leave in the window (at 33, 37, …, 89): 4/15 kWh each. With every time in
/// `unit` instead of minutes, the same.
fn one_machine(unit: TimeUnit) -> Report {
    let k = TimeUnit::Minutes.seconds() / unit.seconds();
    let mut b = Builder::new();
    let src = b.source("jobs", Dist::Constant(4.0 * k), None);
    let m = b.server("machine", 1, Dist::Constant(k));
    b.power(m, power(10.0, 2.0));
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let model = b.build(Experiment {
        time_unit: unit,
        warmup: 32.5 * k,
        duration: 60.0 * k,
        ..Experiment::default()
    });
    assert_eq!(model.validate(), vec![]);
    let c = match compile(&model) {
        Ok(c) => c.first_arrival_at_zero(true),
        Err(e) => panic!("{e:?}"),
    };
    c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn a_machine_uses_its_power_times_its_time_in_each_state() {
    let r = one_machine(TimeUnit::Minutes);
    let m = r.element("machine");
    close(m.and_then(|e| e.energy), 4.0);
    close(m.and_then(|e| e.time_processing), 0.25);
    close(m.and_then(|e| e.time_starved), 0.75);
    close(m.and_then(|e| e.time_blocked), 0.0);
    close(m.and_then(|e| e.time_failed), 0.0);
    close(m.and_then(|e| e.time_off_shift), 0.0);
    assert_eq!(r.element("out").map(|e| e.left), Some(15));
    close(r.element("out").and_then(|e| e.specific_energy), 4.0 / 15.0);
    // Only sinks report energy per part, and only servers energy.
    assert_eq!(r.element("jobs").and_then(|e| e.energy), None);
    assert_eq!(m.and_then(|e| e.specific_energy), None);
}

#[test]
fn energy_is_in_kilowatt_hours_whatever_the_time_unit() {
    for unit in [TimeUnit::Seconds, TimeUnit::Hours, TimeUnit::Days] {
        let r = one_machine(unit);
        close(r.element("machine").and_then(|e| e.energy), 4.0);
        close(r.element("out").and_then(|e| e.specific_energy), 4.0 / 15.0);
    }
}

/// A server with no power reports no energy and no state shares, and then no sink reports
/// energy per part.
#[test]
fn without_power_there_is_no_energy() {
    let mut b = Builder::new();
    let src = b.source("jobs", Dist::Constant(4.0), None);
    let m = b.server("machine", 1, Dist::Constant(1.0));
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let model = b.build(Experiment {
        duration: 60.0,
        ..Experiment::default()
    });
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    let r = c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
    let m = r.element("machine");
    assert_eq!(m.and_then(|e| e.energy), None);
    assert_eq!(m.and_then(|e| e.time_processing), None);
    assert_eq!(m.and_then(|e| e.time_off_shift), None);
    assert_eq!(r.element("out").and_then(|e| e.specific_energy), None);
}

fn at(h: u32, m: u32) -> DateTime {
    DateTime::new(2016, 1, 1, h, m, 0).unwrap_or(DateTime(0))
}

/// The machine of `calendars.rs`'s interrupted job: jobs of 4 waiting from 0, a failure 7 after
/// each repair, repairs of 5, failures interrupting the job, off shift from 10 to 20, over 30
/// minutes. Processing [0, 7), the resumed job [12, 13) in the break, and [24, 30): 14; failed
/// [7, 12) and [19, 24): 10; off shift (idle in the break) [13, 19): 6.
fn failing_on_a_calendar(p: Power) -> Report {
    let mut b = Builder::new();
    b.calendar(
        "break",
        vec![Event {
            start: at(0, 10),
            end: at(0, 20),
            rule: None,
        }],
    );
    let src = b.source("jobs", Dist::Constant(0.0), Some(100));
    let m = b.server("machine", 1, Dist::Constant(4.0));
    b.availability(m, true, &[("break", false)]);
    b.breakdown(
        m,
        Breakdown {
            time_to_failure: Dist::Constant(7.0),
            repair: Dist::Constant(5.0),
            interruptible: true,
            hands_on_while_down: true,
        },
    );
    b.power(m, p);
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let model = b.build(Experiment {
        duration: 30.0,
        start: Some(at(0, 0)),
        ..Experiment::default()
    });
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

/// Each state at its own power: (10 × 14 + 0.5 × 10 + 0.2 × 6) / 60 kWh. The states share the
/// time out exactly; failed is the complement of availability.
#[test]
fn failed_and_off_shift_have_their_own_power() {
    let r = failing_on_a_calendar(Power {
        processing: 10.0,
        idle: 2.0,
        blocked: Some(3.0),
        failed: Some(0.5),
        off_shift: Some(0.2),
    });
    let m = r.element("machine");
    close(m.and_then(|e| e.energy), (140.0 + 5.0 + 1.2) / 60.0);
    close(m.and_then(|e| e.time_processing), 14.0 / 30.0);
    close(m.and_then(|e| e.time_failed), 10.0 / 30.0);
    close(m.and_then(|e| e.time_off_shift), 6.0 / 30.0);
    close(m.and_then(|e| e.time_starved), 0.0);
    close(m.and_then(|e| e.time_blocked), 0.0);
    close(m.and_then(|e| e.availability), 20.0 / 30.0);
    // Three parts out (at 4, 13 and 28).
    close(
        r.element("out").and_then(|e| e.specific_energy),
        146.2 / 60.0 / 3.0,
    );
}

/// Left out, blocked, failed and off shift draw the idle power: (10 × 14 + 2 × 16) / 60 kWh.
#[test]
fn states_without_a_power_draw_idle_power() {
    let r = failing_on_a_calendar(power(10.0, 2.0));
    close(
        r.element("machine").and_then(|e| e.energy),
        (140.0 + 32.0) / 60.0,
    );
}

/// The line of `transfers.rs` whose first machine holds each part through its transfer: the
/// first machine processes [5k, 5k + 3) and is blocked [5k + 3, 5k + 5); over the 80 minutes
/// after a warm-up of 21 it processes 48 and is blocked 32, at 5 and 3 kW: (240 + 96) / 60 kWh.
/// The second processes 32 at 4 kW and is idle 48 at 0.5 kW: (128 + 24) / 60 kWh. 16 parts
/// leave in the window: (336 + 152) / 60 / 16 kWh each.
#[test]
fn a_machine_holding_a_part_in_transfer_is_blocked() {
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(0.0), Some(40));
    let m1 = b.server("m1", 1, Dist::Constant(3.0));
    let m2 = b.buffered_server("m2", 1, Dist::Constant(2.0), 2);
    b.transfer(m2, Dist::Constant(2.0));
    b.power(
        m1,
        Power {
            blocked: Some(3.0),
            ..power(5.0, 1.0)
        },
    );
    b.power(m2, power(4.0, 0.5));
    let out = b.sink("out");
    b.connect(src, m1);
    b.connect(m1, m2);
    b.connect(m2, out);
    let model = b.build(Experiment {
        warmup: 21.0,
        duration: 80.0,
        ..Experiment::default()
    });
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    let r = c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
    let first = r.element("m1");
    close(first.and_then(|e| e.time_blocked), 32.0 / 80.0);
    close(first.and_then(|e| e.time_processing), 48.0 / 80.0);
    close(first.and_then(|e| e.energy), 336.0 / 60.0);
    close(r.element("m2").and_then(|e| e.energy), 152.0 / 60.0);
    assert_eq!(r.element("out").map(|e| e.left), Some(16));
    close(
        r.element("out").and_then(|e| e.specific_energy),
        488.0 / 60.0 / 16.0,
    );
}

/// Rare arrivals over a short run: some replications have no part out. They have no energy per
/// part (not 0), and the summary is the mean of the others' ratios, over that many.
#[test]
fn a_replication_with_no_output_gives_no_ratio() {
    let mut b = Builder::new();
    let src = b.source("jobs", Dist::Exponential { mean: 30.0 }, None);
    let m = b.server("machine", 1, Dist::Constant(1.0));
    b.power(m, power(10.0, 2.0));
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let model = b.build(Experiment {
        duration: 20.0,
        ..Experiment::default()
    });
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    let reports: Vec<Report> = (0..20)
        .map(|r| {
            c.run(Rng::new(80_000 + r))
                .unwrap_or_else(|e| panic!("{e}"))
        })
        .collect();
    let ratios: Vec<f64> = reports
        .iter()
        .filter_map(|r| r.element("out")?.specific_energy)
        .collect();
    let none = reports
        .iter()
        .filter(|r| r.element("out").is_some_and(|e| e.left == 0))
        .count();
    assert!(none > 0 && !ratios.is_empty(), "{none} without output");
    assert_eq!(ratios.len() + none, reports.len());
    // Every replication used energy, idle or not.
    assert!(
        reports
            .iter()
            .all(|r| r.element("machine").and_then(|e| e.energy).is_some())
    );
    let s = summarise(&reports);
    let got = s.get("out", Metric::SpecificEnergy);
    let mean = ratios.iter().sum::<f64>() / ratios.len() as f64;
    assert!(
        got.is_some_and(|g| g.n == ratios.len() as u64 && (g.mean - mean).abs() < 1e-12),
        "{got:?} vs {mean} over {}",
        ratios.len()
    );
}
