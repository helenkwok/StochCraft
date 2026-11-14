//! A task's own units by shift (ROADMAP M2.10): the number of a server's units follows a
//! timetable, and a job under way on a unit that goes off shift finishes first, is retried
//! elsewhere, or resumes when the unit is back.
//!
//! The exact cases are worked by hand. The statistical ones compare an M/M/c server whose c
//! follows a timetable (10, then 4, then 10 again) with its continuous-time Markov chain, solved
//! numerically here (the transient distribution, by Runge-Kutta from an empty system):
//!
//! - retrying, the number inside N is a birth-death chain whose death rate is μ·min(N, c(t)):
//!   the entities whose units went off go back to the queue, and since service is exponential
//!   their fresh start changes nothing;
//! - resuming, the jobs paused at the drop (P = min(N, c) − 4 of them, at most 6) stay inside,
//!   unserved, until the units come back; the others, M = N − P, form the chain with 4 units
//!   meanwhile, and at the rise N = M + P carries on with 10. Exact again, by memorylessness;
//! - finishing is not a chain on N alone (busy units linger after the drop), so it is checked
//!   by hand only.
#![allow(clippy::panic)]

use stochcraft_blocks::{
    Builder, ElementReport, Event, ExperimentSummary, HealthCheck, Metric, OnShiftEnd, OnTakeout,
    Report, compile, summarise,
};
use stochcraft_model::{DateTime, Dist, Experiment, Problem, Severity};
use stochcraft_random::Rng;

fn at(h: u32, m: u32, s: u32) -> DateTime {
    DateTime::new(2016, 1, 1, h, m, s).unwrap_or(DateTime(0))
}

/// Once, from `from` to `to` (hours, minutes, seconds of 1 January 2016).
fn once(from: (u32, u32, u32), to: (u32, u32, u32)) -> Event {
    Event {
        start: at(from.0, from.1, from.2),
        end: at(to.0, to.1, to.2),
        rule: None,
    }
}

fn experiment(duration: f64, replications: u32) -> Experiment {
    Experiment {
        duration,
        replications,
        start: Some(at(0, 0, 0)),
        ..Experiment::default()
    }
}

fn element<'a>(r: &'a Report, id: &str) -> &'a ElementReport {
    r.element(id).unwrap_or_else(|| panic!("no {id} in {r:?}"))
}

fn close(what: &str, got: Option<f64>, want: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() < 1e-9),
        "{what}: {got:?} vs {want}"
    );
}

fn run_once(b: &Builder, duration: f64) -> Report {
    let model = b.build(experiment(duration, 1));
    // Some of these are overloaded on purpose (a warning); none has an error.
    let errors: Vec<_> = model
        .validate()
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert_eq!(errors, vec![]);
    let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
    compiled.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

/// Three units with jobs of 6, a job arriving every minute from 1, and only one unit on shift
/// from 10.5 to 20.5, run to 27. At 10.5 every unit is busy: unit 0 with the job it started at
/// 7, unit 1 at 8, unit 2 at 9. The two to go off are those that started last, units 2 and 1.
fn three_units(rule: OnShiftEnd) -> Report {
    let mut b = Builder::new();
    b.calendar("low", vec![once((0, 10, 30), (0, 20, 30))]);
    let src = b.source("jobs", Dist::Constant(1.0), None);
    let m = b.server("machine", 3, Dist::Constant(6.0));
    b.server_shifts(m, 3, &[("low", 1)], rule);
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    run_once(&b, 27.0)
}

/// Finish: units 1 and 2 finish their jobs at 14 and 15, then go off; unit 0 carries on (jobs
/// ending 13, 19, 25); at 20.5 units 1 and 2 come back and start jobs that end at 26.5. Jobs out
/// by 27: 7, 8, 9, 13, 14, 15, 19, 25, 26.5, 26.5, ten. (Had the units that started first gone
/// off, units 0 and 1, unit 2 would have ended a job at 21 and only 9 would be out.)
///
/// Busy: unit 0 from 1 to 27, 26; units 1 and 2 to 14 and 15, and from 20.5: 18.5 each; 63. On
/// shift, counting the units finishing: 3 to 14, 2 to 15, 1 to 20.5, then 3: 69. Utilisation
/// 63/69. Off shift: unit 1 from 14 to 20.5, unit 2 from 15: 12 of 3 × 27 unit-minutes.
#[test]
fn finishing_the_job_then_going_off() {
    let r = three_units(OnShiftEnd::Finish);
    let m = element(&r, "machine");
    assert_eq!(m.left, 10, "{m:?}");
    close("utilisation", m.utilisation, 63.0 / 69.0);
    close("off shift", m.time_off_shift, 12.0 / 81.0);
    // Nothing is retried, so no work wasted is reported.
    assert_eq!(m.wasted, None);
}

/// Retry: at 10.5 the jobs of units 2 and 1 stop; they had run 1.5 and 2.5, wasted (4 in all;
/// had units 0 and 1 gone off, 6). Both entities go back to the head of the queue; unit 0 starts
/// them afresh after its job ends at 13 (ending 19 and 25); at 20.5 units 1 and 2 start the next
/// two, ending 26.5. Out by 27: 7, 8, 9, 13, 19, 25, 26.5, 26.5, eight.
///
/// Busy: unit 0 26, unit 1 8.5 + 6.5, unit 2 7.5 + 6.5: 55; on shift 3 × 10.5 + 10 + 3 × 6.5 =
/// 61. Off shift: units 1 and 2 from 10.5 to 20.5, 20 of 81. Wasted 4 of the 55 working.
#[test]
fn retrying_the_job_elsewhere() {
    let r = three_units(OnShiftEnd::Retry);
    let m = element(&r, "machine");
    assert_eq!(m.left, 8, "{m:?}");
    close("wasted", m.wasted, 4.0);
    close("wasted share", m.wasted_share, 4.0 / 55.0);
    close("utilisation", m.utilisation, 55.0 / 61.0);
    close("off shift", m.time_off_shift, 20.0 / 81.0);
    // Every entity is accounted for: 27 arrived, 8 out, the rest inside.
    assert_eq!(m.entered, m.left + m.work_in_progress.unwrap_or(0));
}

/// Resume: at 10.5 the jobs of units 2 and 1 pause with 4.5 and 3.5 left; unit 0 ends jobs at
/// 13, 19 and 25; at 20.5 units 1 and 2 come back to their paused jobs, ending them at 24 and
/// 25. Out by 27: 7, 8, 9, 13, 19, 24, 25, 25, eight.
///
/// The paused jobs are off shift, not busy working: busy less paused is 26 + 15 + 14 = 55, over
/// 61 on shift; off shift 20 of 81, as for a retry, with nothing wasted.
#[test]
fn resuming_the_job_when_back_on_shift() {
    let r = three_units(OnShiftEnd::Resume);
    let m = element(&r, "machine");
    assert_eq!(m.left, 8, "{m:?}");
    close("utilisation", m.utilisation, 55.0 / 61.0);
    close("off shift", m.time_off_shift, 20.0 / 81.0);
    assert_eq!(m.wasted, None);
    // The paused jobs' entities stayed inside: busy (in the work in progress) throughout.
    assert_eq!(m.entered, m.left + m.work_in_progress.unwrap_or(0));
}

/// Idle units go off first: one job of 30 from 1 on three units, one on shift from 10.5 to
/// 20.5. The two idle units go off, the busy one carries on, and nothing is wasted.
#[test]
fn idle_units_go_off_first() {
    let mut b = Builder::new();
    b.calendar("low", vec![once((0, 10, 30), (0, 20, 30))]);
    let src = b.source("jobs", Dist::Constant(1.0), Some(1));
    let m = b.server("machine", 3, Dist::Constant(30.0));
    b.server_shifts(m, 3, &[("low", 1)], OnShiftEnd::Retry);
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let r = run_once(&b, 32.0);
    let m = element(&r, "machine");
    assert_eq!(m.left, 1, "{m:?}");
    close("wasted", m.wasted, 0.0);
    // Busy 30 over on shift 3 × 32 − 2 × 10.
    close("utilisation", m.utilisation, 30.0 / 76.0);
}

/// Units only in a shift: none outside it (a quantity of 0), 2 from 10 to 20. Jobs of 4 waiting
/// from time 0 start at 10: two end at 14 and 18 on each unit, and the two started at 18 finish
/// at 22 (finishing after the shift): 6 out by 30, and the units on shift for 20 minutes plus 2 ×
/// 2 finishing.
#[test]
fn units_only_in_a_shift() {
    let mut b = Builder::new();
    b.calendar("day", vec![once((0, 10, 0), (0, 20, 0))]);
    let src = b.source("jobs", Dist::Constant(0.0), Some(100));
    let m = b.server("machine", 1, Dist::Constant(4.0));
    b.server_shifts(m, 0, &[("day", 2)], OnShiftEnd::Finish);
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let r = run_once(&b, 30.0);
    let m = element(&r, "machine");
    assert_eq!(m.left, 6, "{m:?}");
    close("utilisation", m.utilisation, 1.0);
    // Off shift: 2 units for 10 before, and for 8 after the jobs end at 22.
    close("off shift", m.time_off_shift, (20.0 + 16.0) / 60.0);
}

/// A server with no units outside its shifts and none in them never has one: a model problem.
#[test]
fn a_server_with_no_units_ever_is_refused() {
    let mut b = Builder::new();
    b.calendar("day", vec![once((0, 10, 0), (0, 20, 0))]);
    let src = b.source("jobs", Dist::Constant(1.0), None);
    let m = b.server("machine", 1, Dist::Constant(4.0));
    b.server_shifts(m, 0, &[("day", 0)], OnShiftEnd::Finish);
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let model = b.build(experiment(30.0, 1));
    assert!(
        model
            .validate()
            .iter()
            .any(|d| d.problem == Problem::ZeroCapacity),
        "{:?}",
        model.validate()
    );
    // A shift on a calendar the model does not have is one too.
    let mut b2 = b.clone();
    b2.server_shifts(m, 1, &[("night", 3)], OnShiftEnd::Finish);
    let model = b2.build(experiment(30.0, 1));
    assert!(
        model.validate().iter().any(
            |d| matches!(&d.problem, Problem::UnknownCalendar { id } if id.as_str() == "night")
        ),
        "{:?}",
        model.validate()
    );
}

/// A health check counts the units working as those on shift, up and not out, and checks no
/// unit off shift. Four units, saturated with jobs of 1, two on shift from 10.5 to 40.5
/// (retrying, so no unit lingers); each unit checked at 15, 30 and 45, with a limit of 0, a
/// restart of 5 and at least 2 kept in service. At 15 and 30 the two on shift are checked and
/// neither is taken out: 2 working is not more than the 2 kept (counting the 4 units, both would
/// go). At 45 all four are back on shift: units 0 and 1 are taken out, then 2 working are kept.
///
/// Units in service: 4 to 10.5, 2 to 40.5, 4 to 45, 2 to 46: 122 over 46.
#[test]
fn a_health_check_counts_the_units_on_shift() {
    let mut b = Builder::new();
    b.calendar("low", vec![once((0, 10, 30), (0, 40, 30))]);
    let src = b.source("jobs", Dist::Constant(0.0), Some(1_000));
    let m = b.server("desk", 4, Dist::Constant(1.0));
    b.server_shifts(m, 4, &[("low", 2)], OnShiftEnd::Retry);
    b.health_check(
        m,
        HealthCheck {
            interval: Dist::Constant(15.0),
            limit: 0.0,
            restart: Dist::Constant(5.0),
            keep: Some(2),
            on_takeout: OnTakeout::Retry,
        },
    );
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let r = run_once(&b, 46.0);
    let m = element(&r, "desk");
    assert_eq!(m.taken_out, Some(2), "{m:?}");
    close("in service", m.mean_in_service, 122.0 / 46.0);
    close("restarting", m.time_restarting, 2.0 / (4.0 * 46.0));
}

/// A server with shifts that never change the number of units (the shift's count is the
/// quantity) runs exactly as the same server without them.
#[test]
fn shifts_of_the_same_count_change_nothing() {
    let build = |shifts: bool| {
        let mut b = Builder::new();
        b.calendar("day", vec![once((0, 10, 0), (0, 20, 0))]);
        let src = b.source("jobs", Dist::exponential_rate(2.0), None);
        let m = b.server("desk", 3, Dist::exponential_rate(1.0));
        if shifts {
            b.server_shifts(m, 3, &[("day", 3)], OnShiftEnd::Retry);
        }
        let out = b.sink("out");
        b.connect(src, m);
        b.connect(m, out);
        b
    };
    for seed in 0..4 {
        let model = |s: bool| build(s).build(experiment(500.0, 1));
        let run = |s: bool| {
            compile(&model(s))
                .unwrap_or_else(|d| panic!("{d:?}"))
                .run(Rng::new(seed))
                .unwrap_or_else(|e| panic!("{e}"))
        };
        let (a, b) = (run(false), run(true));
        let (x, y) = (element(&a, "desk"), element(&b, "desk"));
        assert_eq!((x.entered, x.left), (y.entered, y.left));
        for metric in [Metric::MeanQueueLength, Metric::MeanWait] {
            assert_eq!(x.metric(metric), y.metric(metric), "{}", metric.name());
        }
        // Over the units on shift, which are always 3 (averaged, to the last bit or so).
        let (u, v) = (x.utilisation.unwrap_or(-1.0), y.utilisation.unwrap_or(1.0));
        assert!((u - v).abs() < 1e-12, "{u} vs {v}");
    }
}

// The M/M/c server whose c follows a timetable, against its Markov chain.

const LAMBDA: f64 = 7.0;
const MU: f64 = 1.0;
/// The units outside the low shift, and in it.
const HIGH: usize = 10;
const LOW: usize = 4;
/// The low shift, from 60 to 80, in a run from an empty system measured from the drop to 90 (a
/// warm-up of 60): the time the rules differ, and the drain after it.
const DROP: f64 = 60.0;
const RISE: f64 = 80.0;
const END: f64 = 90.0;
/// The chain's states, 0 to `MAX` inside (arrivals blocked there: far beyond any reached).
const MAX: usize = 400;
/// The Runge-Kutta step.
const DT: f64 = 0.005;
const REPLICATIONS: u32 = 200;

/// dp/dt for the birth-death chain with `c` units: arrivals at λ, departures at μ·min(n, c).
fn derivative(p: &[f64], c: usize, out: &mut [f64]) {
    for (n, o) in out.iter_mut().enumerate() {
        let gone = |k: usize| MU * k.min(c) as f64;
        let mut d = -(gone(n) + if n < MAX { LAMBDA } else { 0.0 }) * p[n];
        if n > 0 {
            d += LAMBDA * p[n - 1];
        }
        if n < MAX {
            d += gone(n + 1) * p[n + 1];
        }
        *o = d;
    }
}

/// Evolves `p` with `c` units for `time`, by fourth-order Runge-Kutta; returns the integrals over
/// that time of the mean waiting (n − min(n, c)) and of the departure rate (μ·min(n, c)).
fn evolve(p: &mut [f64], c: usize, time: f64) -> (f64, f64) {
    let steps = (time / DT).round() as usize;
    let measure = |p: &[f64]| {
        p.iter().enumerate().fold((0.0, 0.0), |(w, d), (n, x)| {
            (w + x * (n - n.min(c)) as f64, d + x * MU * n.min(c) as f64)
        })
    };
    let (mut waiting, mut departing) = (0.0, 0.0);
    let n = p.len();
    let (mut k1, mut k2, mut k3, mut k4, mut tmp) = (
        vec![0.0; n],
        vec![0.0; n],
        vec![0.0; n],
        vec![0.0; n],
        vec![0.0; n],
    );
    for _ in 0..steps {
        let before = measure(p);
        derivative(p, c, &mut k1);
        for i in 0..n {
            tmp[i] = p[i] + 0.5 * DT * k1[i];
        }
        derivative(&tmp, c, &mut k2);
        for i in 0..n {
            tmp[i] = p[i] + 0.5 * DT * k2[i];
        }
        derivative(&tmp, c, &mut k3);
        for i in 0..n {
            tmp[i] = p[i] + DT * k3[i];
        }
        derivative(&tmp, c, &mut k4);
        for i in 0..n {
            p[i] += DT / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
        }
        let after = measure(p);
        // The trapezoid rule over the step.
        waiting += 0.5 * DT * (before.0 + after.0);
        departing += 0.5 * DT * (before.1 + after.1);
    }
    (waiting, departing)
}

/// The theory's mean queue and mean entities held by units (time-averaged from the drop to the
/// end: those in service, and paused ones) and mean departures in that time, retrying or
/// resuming.
fn chain(resume: bool) -> (f64, f64, f64) {
    let mut p = vec![0.0; MAX + 1];
    p[0] = 1.0;
    // Up to the drop: the warm-up, not measured.
    evolve(&mut p, HIGH, DROP);
    let mut held_paused = 0.0;
    let (w2, d2) = if resume {
        // The paused jobs, P of them, sit apart; M = N − P is served by the units on shift.
        let mut slices: Vec<Vec<f64>> = vec![vec![0.0; MAX + 1]; HIGH - LOW + 1];
        for (n, x) in p.iter().enumerate() {
            let paused = n.min(HIGH).saturating_sub(LOW);
            slices[paused][n - paused] += x;
        }
        let (mut w, mut d) = (0.0, 0.0);
        for s in &mut slices {
            let (a, b) = evolve(s, LOW, RISE - DROP);
            w += a;
            d += b;
        }
        p = vec![0.0; MAX + 1];
        for (paused, s) in slices.iter().enumerate() {
            for (m, x) in s.iter().enumerate() {
                p[(m + paused).min(MAX)] += x;
            }
            // Held, unserved, through the low shift (each slice keeps its probability).
            held_paused += paused as f64 * s.iter().sum::<f64>() * (RISE - DROP);
        }
        (w, d)
    } else {
        evolve(&mut p, LOW, RISE - DROP)
    };
    let (w3, d3) = evolve(&mut p, HIGH, END - RISE);
    let span = END - DROP;
    // Units serving: the departure rate over μ.
    let held = ((d2 + d3) / MU + held_paused) / span;
    ((w2 + w3) / span, held, d2 + d3)
}

fn simulate(rule: OnShiftEnd) -> ExperimentSummary {
    let mut b = Builder::new();
    b.calendar("low", vec![once((1, 0, 0), (1, 20, 0))]);
    let src = b.source("arrivals", Dist::exponential_rate(LAMBDA), None);
    let desk = b.server("desk", HIGH as u32, Dist::exponential_rate(MU));
    b.server_shifts(desk, HIGH as u32, &[("low", LOW as u32)], rule);
    let out = b.sink("exit");
    b.connect(src, desk);
    b.connect(desk, out);
    let mut model = b.build(experiment(END - DROP, REPLICATIONS));
    model.experiment.warmup = DROP;
    let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
    let reports: Vec<Report> = (0..u64::from(REPLICATIONS))
        .map(|r| {
            compiled
                .run(Rng::new(70_000 + r))
                .unwrap_or_else(|e| panic!("{e}"))
        })
        .collect();
    summarise(&reports)
}

fn covers(s: &ExperimentSummary, metric: Metric, theory: f64) {
    let got = s.get("desk", metric);
    eprintln!("desk {}: {got:?} (theory {theory})", metric.name());
    assert!(
        got.is_some_and(|g| g.n == u64::from(REPLICATIONS) && g.contains(theory)),
        "desk {}: {got:?} vs {theory}",
        metric.name()
    );
}

/// Retrying: the queue and the departures of M/M/c with c = 10, 4 from 60 to 80, then 10 again,
/// from 60 to 90: mean queue, mean entities held by units, and departures.
#[test]
fn retrying_matches_the_chain_whose_units_follow_the_timetable() {
    let (queue, held, served) = chain(false);
    let s = simulate(OnShiftEnd::Retry);
    covers(&s, Metric::MeanQueueLength, queue);
    covers(&s, Metric::MeanContents, held);
    covers(&s, Metric::Left, served);
    // The resuming chain's is far from it: the check tells the rules apart.
    let (_, other, _) = chain(true);
    assert!(
        s.get("desk", Metric::MeanContents)
            .is_some_and(|g| !g.contains(other))
    );
}

/// Resuming: the paused jobs wait on their units, unserved, until the shift ends; they are held
/// (in the mean contents) but not served.
#[test]
fn resuming_matches_the_chain_with_the_paused_jobs_set_apart() {
    let (queue, held, served) = chain(true);
    let s = simulate(OnShiftEnd::Resume);
    covers(&s, Metric::MeanQueueLength, queue);
    covers(&s, Metric::MeanContents, held);
    covers(&s, Metric::Left, served);
    let (_, other, _) = chain(false);
    assert!(
        s.get("desk", Metric::MeanContents)
            .is_some_and(|g| !g.contains(other))
    );
}
