//! Servers whose units fail, against exact results: availability MTBF / (MTBF + MTTR), the
//! effective service rate of a saturated server, the time a job holds its unit when failures
//! interrupt it (Little's law), and deterministic cycles worked by hand for both kinds of
//! failure.
#![allow(clippy::panic)]

use stochcraft_blocks::{Breakdown, Builder, Metric, Report, compile, summarise};
use stochcraft_model::{Dist, Experiment, Model};
use stochcraft_random::Rng;

const REPS: u64 = 16;
/// The seed is fixed so the tests are repeatable; each check is a 95% interval.
const SEED: u64 = 70_000;

fn experiment(warmup: f64, duration: f64) -> Experiment {
    Experiment {
        warmup,
        duration,
        replications: REPS as u32,
        ..Experiment::default()
    }
}

fn reports(model: &Model) -> Vec<Report> {
    let compiled = compile(model);
    let Ok(compiled) = compiled else {
        panic!("{compiled:?}")
    };
    (0..REPS)
        .map(|r| compiled.run(Rng::new(SEED + r)))
        .collect()
}

/// A source at `lambda`, a server of one unit with service `service` that fails, and a sink.
fn failing(lambda: f64, service: Dist, b: Breakdown, x: Experiment) -> Model {
    let mut m = Builder::new();
    let src = m.source("in", Dist::exponential_rate(lambda), None);
    let srv = m.server("machine", 1, service);
    m.breakdown(srv, b);
    let out = m.sink("out");
    m.connect(src, srv);
    m.connect(srv, out);
    m.build(x)
}

/// Exponential time to failure (mean 100) and repair (mean 10): the unit is up
/// 100 / (100 + 10) of the time, whatever it does; saturated (arrivals at 2, service at rate 1)
/// and with failures interrupting the job, which resumes after the repair, it finishes jobs at
/// that fraction of its rate.
#[test]
fn availability_and_the_effective_service_rate_match() {
    let (mtbf, mttr) = (100.0, 10.0);
    let a = mtbf / (mtbf + mttr);
    let duration = 20_000.0;
    let b = Breakdown {
        time_to_failure: Dist::Exponential { mean: mtbf },
        repair: Dist::Exponential { mean: mttr },
        interruptible: true,
    };
    let model = failing(
        2.0,
        Dist::exponential_rate(1.0),
        b,
        experiment(0.0, duration),
    );
    let s = summarise(&reports(&model));
    let got = s.get("machine", Metric::Availability);
    eprintln!("availability {got:?} (theory {a})");
    assert!(got.is_some_and(|g| g.contains(a)), "{got:?} vs {a}");
    let left = s.get("machine", Metric::Left);
    eprintln!("jobs {left:?} (theory {})", a * duration);
    assert!(left.is_some_and(|g| g.contains(a * duration)), "{left:?}");
}

/// When failures (rate f = 1/50) interrupt a job and it resumes on its unit after a repair
/// (mean 5), a job holds the unit for its service plus the repairs of the failures during it:
/// E[S] (1 + f E[R]) = 1.1 for exponential service of mean 1, as failures in elapsed time are
/// Poisson. By Little's law the unit holds λ times that on average: 0.33 at λ = 0.3.
#[test]
fn an_interrupted_job_holds_its_unit_through_the_repairs() {
    let (lambda, mtbf, mttr) = (0.3, 50.0, 5.0);
    let b = Breakdown {
        time_to_failure: Dist::Exponential { mean: mtbf },
        repair: Dist::Exponential { mean: mttr },
        interruptible: true,
    };
    let model = failing(
        lambda,
        Dist::exponential_rate(1.0),
        b,
        experiment(1_000.0, 40_000.0),
    );
    let s = summarise(&reports(&model));
    let theory = lambda * (1.0 + mttr / mtbf);
    let got = s.get("machine", Metric::MeanContents);
    eprintln!("contents {got:?} (theory {theory})");
    assert!(
        got.is_some_and(|g| g.contains(theory)),
        "{got:?} vs {theory}"
    );
}

fn saturated(b: Breakdown, end: f64) -> Report {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(0.5), None);
    let srv = m.server("machine", 1, Dist::Constant(3.0));
    m.breakdown(srv, b);
    let out = m.sink("out");
    m.connect(src, srv);
    m.connect(srv, out);
    let model = m.build(experiment(0.0, end));
    let Ok(c) = compile(&model) else {
        panic!("does not compile")
    };
    c.first_arrival_at_zero(true).run(Rng::new(1))
}

/// Jobs of 3, a failure 10 after each repair, repairs of 2, the job under way finished first:
/// jobs end at 3, 6, 9, 12; the failure due at 10 waits to 12 and the repair lasts to 14; then
/// the same again every 14. By 70, five cycles of 4 jobs, and down 2 in every 14.
#[test]
fn a_failure_waits_for_the_job_when_not_interruptible() {
    let r = saturated(
        Breakdown {
            time_to_failure: Dist::Constant(10.0),
            repair: Dist::Constant(2.0),
            interruptible: false,
        },
        70.0,
    );
    let m = r.element("machine");
    assert_eq!(m.map(|e| e.left), Some(20), "{r:?}");
    let a = m.and_then(|e| e.availability);
    assert!(a.is_some_and(|a| (a - 60.0 / 70.0).abs() < 1e-9), "{a:?}");
}

/// The same, but a failure interrupts the job, which resumes after the repair: the unit works
/// [0, 10), [12, 22), [24, 34), … and a job ends at each 3 of work done: 19 by 69, when it has
/// been down for five repairs of 2.
#[test]
fn an_interrupted_job_resumes_where_it_stopped() {
    let r = saturated(
        Breakdown {
            time_to_failure: Dist::Constant(10.0),
            repair: Dist::Constant(2.0),
            interruptible: true,
        },
        69.0,
    );
    let m = r.element("machine");
    assert_eq!(m.map(|e| e.left), Some(19), "{r:?}");
    let a = m.and_then(|e| e.availability);
    assert!(a.is_some_and(|a| (a - 59.0 / 69.0).abs() < 1e-9), "{a:?}");
}
