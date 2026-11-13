//! Patience, abandonment and retries (ROADMAP M2.10): servers whose entities give up.
//!
//! The statistical checks compare M/M/c+M (Poisson arrivals, exponential service, exponential
//! patience until service starts, unlimited room; the model of Garnett, Mandelbaum and Reiman,
//! M&SOM 4(3), 2002) with its birth–death chain, solved numerically here from first principles:
//! with n inside, entities arrive at rate λ and leave at rate min(n, c)·μ + max(n − c, 0)·θ
//! (each unit finishing, each waiting entity giving up). A share θ·E[Nq]/λ of arrivals gives up,
//! since entities give up at rate θ·E[Nq] in the long run.
#![allow(clippy::panic)]

use stochcraft_blocks::{
    Builder, ElementReport, ExperimentSummary, Metric, Patience, PatienceUntil, Report, Setup,
    compile, summarise,
};
use stochcraft_model::{Diagnostic, Experiment, Problem};
use stochcraft_random::{Dist, Rng};

const REPLICATIONS: u64 = 16;

fn patience(time: Dist, until: PatienceUntil) -> Patience {
    Patience { time, until }
}

/// The long-run state of M/M/c+M: (mean number waiting, share giving up, mean busy share).
fn mmc_m(lambda: f64, mu: f64, theta: f64, c: u32) -> (f64, f64, f64) {
    // Unnormalised stationary probabilities p(n) = Π λ / death(k), until they are negligible.
    let death = |k: u32| f64::from(k.min(c)) * mu + f64::from(k.saturating_sub(c)) * theta;
    let mut p = vec![1.0_f64];
    let mut n = 0u32;
    loop {
        n += 1;
        let next = p[p.len() - 1] * lambda / death(n);
        p.push(next);
        let total: f64 = p.iter().sum();
        if n > c && next < 1e-12 * total {
            break;
        }
        assert!(n < 100_000, "the chain does not settle");
    }
    let total: f64 = p.iter().sum();
    let mut queue = 0.0;
    let mut busy = 0.0;
    for (n, x) in p.iter().enumerate() {
        let n = n as u32;
        queue += f64::from(n.saturating_sub(c)) * x / total;
        busy += f64::from(n.min(c)) * x / total;
    }
    (queue, theta * queue / lambda, busy / f64::from(c))
}

/// Arrivals → a server of `c` units with exponential patience until service starts → exit.
/// With `unit_path`, the server sets up for 0 time before each job, which runs it as a server
/// whose units fail, follow a calendar or set up (its dispatcher holds the next entity while it
/// waits for a unit) without changing what happens.
fn mmc_m_model(lambda: f64, mu: f64, wait: Dist, c: u32, unit_path: bool) -> Builder {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(lambda), None);
    let desk = b.server("desk", c, Dist::exponential_rate(mu));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.patience(desk, patience(wait, PatienceUntil::Start));
    if unit_path {
        b.setup(desk, Setup::every_job(Dist::Constant(0.0)));
    }
    b
}

fn run(b: &Builder, warmup: f64, duration: f64) -> (ExperimentSummary, Vec<Report>) {
    let model = b.build(Experiment {
        warmup,
        duration,
        replications: REPLICATIONS as u32,
        ..Experiment::default()
    });
    let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
    let reports: Vec<Report> = (0..REPLICATIONS)
        .map(|r| {
            compiled
                .run(Rng::new(20_000 + r))
                .unwrap_or_else(|e| panic!("{e}"))
        })
        .collect();
    (summarise(&reports), reports)
}

fn covers(s: &ExperimentSummary, element: &str, metric: Metric, theory: f64) {
    let got = s.get(element, metric);
    eprintln!("{element} {}: {got:?} (theory {theory})", metric.name());
    assert!(
        got.is_some_and(|g| g.n == REPLICATIONS && g.contains(theory)),
        "{element} {}: {got:?} vs {theory}",
        metric.name()
    );
}

/// Every element balances: what entered left, was lost, gave up or is still inside. (Over a
/// run with no warm-up: what entered before a warm-up and left after counts only as leaving.)
/// A server's lost arrivals never entered it; anywhere else they entered and never left.
fn balanced(r: &Report) {
    for e in &r.elements {
        if e.kind == "resource" {
            continue;
        }
        let lost = if e.kind == "server" {
            0
        } else {
            e.lost.unwrap_or(0)
        };
        let out = e.left + lost + e.abandoned.unwrap_or(0) + e.work_in_progress.unwrap_or(0);
        assert_eq!(e.entered, out, "{} does not balance: {e:?}", e.id);
    }
}

fn element<'a>(r: &'a Report, id: &str) -> &'a ElementReport {
    r.element(id).unwrap_or_else(|| panic!("no {id}"))
}

fn check_mmc_m(lambda: f64, mu: f64, theta: f64, c: u32, unit_path: bool) {
    let (queue, gives_up, busy) = mmc_m(lambda, mu, theta, c);
    eprintln!("λ {lambda} μ {mu} θ {theta} c {c}: Lq {queue}, P(abandon) {gives_up}, busy {busy}");
    let (s, _) = run(
        &mmc_m_model(lambda, mu, Dist::exponential_rate(theta), c, unit_path),
        1_000.0,
        20_000.0,
    );
    covers(&s, "desk", Metric::MeanQueueLength, queue);
    covers(&s, "desk", Metric::AbandonedShare, gives_up);
    covers(&s, "desk", Metric::Utilisation, busy);
}

#[test]
fn one_unit_with_impatient_callers_matches_the_chain() {
    // ρ = 0.8, θ/μ = 0.5.
    check_mmc_m(0.8, 1.0, 0.5, 1, false);
}

#[test]
fn three_units_nearly_full_match_the_chain() {
    // ρ = 0.95, θ/μ = 0.2.
    check_mmc_m(2.85, 1.0, 0.2, 3, false);
}

#[test]
fn an_overloaded_server_stays_stable_through_abandonment() {
    // ρ = 1.5: without patience the queue would grow without end.
    check_mmc_m(3.0, 1.0, 1.0, 2, false);
}

#[test]
fn a_server_whose_dispatcher_holds_the_next_entity_matches_the_chain_too() {
    check_mmc_m(2.85, 1.0, 0.2, 3, true);
}

/// Patience 0: an arrival is served only if a unit is free at once, which is Erlang's loss
/// system M/M/c/c. Erlang B with c = 3 and offered load a = 2: (a³/3!) / Σₖ aᵏ/k! = 4/19.
#[test]
fn patience_zero_is_erlang_b() {
    for unit_path in [false, true] {
        let b = mmc_m_model(2.0, 1.0, Dist::Constant(0.0), 3, unit_path);
        let (s, _) = run(&b, 1_000.0, 20_000.0);
        covers(&s, "desk", Metric::AbandonedShare, 4.0 / 19.0);
        covers(&s, "desk", Metric::MeanQueueLength, 0.0);
    }
}

/// Two arrivals at time 0, one unit, service 5, patience 5. With patience until start, the
/// second's deadline is the moment the unit comes free, and a hand-over at the deadline counts as
/// served. With patience until done, the first is done at its deadline, which counts as done in
/// time; the second starts at its deadline and so gives up in service, wasting the job.
/// (`(patience, until, abandoned, wasted)`.)
#[test]
fn a_unit_taking_an_entity_at_its_deadline_serves_it() {
    for (wait, until, gives_up, wasted) in [
        (5.0, PatienceUntil::Start, 0, None),
        (4.999, PatienceUntil::Start, 1, None),
        (5.0, PatienceUntil::Done, 1, Some(5.0)),
        // The first is not done in time, and the second gives up waiting.
        (4.999, PatienceUntil::Done, 2, Some(5.0)),
    ] {
        for unit_path in [false, true] {
            let mut b = Builder::new();
            let arrivals = b.source("arrivals", Dist::Constant(0.0), Some(2));
            let desk = b.server("desk", 1, Dist::Constant(5.0));
            let exit = b.sink("exit");
            b.connect(arrivals, desk);
            b.connect(desk, exit);
            b.patience(desk, patience(Dist::Constant(wait), until));
            if unit_path {
                b.setup(desk, Setup::every_job(Dist::Constant(0.0)));
            }
            let r = b
                .run(Rng::new(1), 0.0, 100.0)
                .unwrap_or_else(|e| panic!("{e:?}"));
            let d = element(&r, "desk");
            let label = format!("patience {wait}, {until:?}, unit path {unit_path}");
            assert_eq!(d.abandoned, Some(gives_up), "{label}: {d:?}");
            assert_eq!(d.wasted, wasted, "{label}: {d:?}");
            assert_eq!(d.left, 2 - gives_up, "{label}");
            assert_eq!(element(&r, "exit").left, 2 - gives_up, "{label}");
            balanced(&r);
        }
    }
}

/// Patience until done, one arrival at 0, service 10, patience 5: it gives up at 5, when its
/// stand-in takes the abandoned way; the unit works on to 10 and discards the job.
#[test]
fn patience_until_done_gives_up_in_service_and_wastes_the_work() {
    for unit_path in [false, true] {
        let mut b = Builder::new();
        let arrivals = b.source("arrivals", Dist::Constant(0.0), Some(1));
        let desk = b.server("desk", 1, Dist::Constant(10.0));
        let exit = b.sink("exit");
        let gave_up = b.sink("gave_up");
        b.connect(arrivals, desk);
        b.connect(desk, exit);
        b.abandoned(desk, gave_up);
        b.patience(desk, patience(Dist::Constant(5.0), PatienceUntil::Done));
        b.planned_run_time(desk, Dist::Constant(10.0));
        if unit_path {
            b.setup(desk, Setup::every_job(Dist::Constant(0.0)));
        }
        let model = b.build(Experiment {
            warmup: 0.0,
            duration: 20.0,
            replications: 1,
            ..Experiment::default()
        });
        let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
        let mut live = compiled.live(Rng::new(1));
        let state = |live: &stochcraft_blocks::Live<'_>, id: &str| {
            live.states()
                .into_iter()
                .find(|s| s.id.as_str() == id)
                .unwrap_or_else(|| panic!("no {id}"))
        };
        live.advance_to(4.999).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(state(&live, "gave_up").left, 0);
        live.advance_to(5.0).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            state(&live, "gave_up").left,
            1,
            "the stand-in gives up at 5"
        );
        assert_eq!(state(&live, "desk").busy, 1.0, "the unit works on");
        live.advance_to(20.0).unwrap_or_else(|e| panic!("{e}"));
        let r = live.report();
        let d = element(&r, "desk");
        assert_eq!((d.entered, d.left, d.abandoned), (1, 0, Some(1)), "{d:?}");
        assert_eq!(d.work_in_progress, Some(0));
        assert_eq!(d.wasted, Some(10.0));
        assert_eq!(d.wasted_share, Some(1.0));
        assert_eq!(d.abandoned_wait, Some(5.0));
        assert_eq!(d.abandoned_share, Some(1.0));
        // The original was never produced: nothing to be effective about.
        assert_eq!(d.oee_effectiveness, Some(0.0));
        assert_eq!(d.utilisation, Some(0.5));
        assert_eq!(element(&r, "exit").left, 0);
        let g = element(&r, "gave_up");
        assert_eq!(g.left, 1);
        assert_eq!(
            g.mean_time_in_system,
            Some(5.0),
            "created at 0, gave up at 5"
        );
        balanced(&r);
    }
}

/// Patience until done also covers a finished job held blocked: one unit, service 1, patience
/// 3, sending to a server whose buffer is full until 6. The job is done at 1 and held; its
/// entity gives up at 3; when room appears at 6 the unit lets it go, discarded.
#[test]
fn patience_until_done_covers_a_job_held_after_service() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::Constant(0.0), Some(1));
    let desk = b.server("desk", 1, Dist::Constant(1.0));
    let gave_up = b.sink("gave_up");
    // The next server is busy until 6 with an entity of its own, and has no waiting room.
    let blocker = b.source("blocker", Dist::Constant(0.0), Some(1));
    let next = b.buffered_server("next", 1, Dist::Constant(6.0), 0);
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, next);
    b.abandoned(desk, gave_up);
    b.connect(blocker, next);
    b.connect(next, exit);
    b.patience(desk, patience(Dist::Constant(3.0), PatienceUntil::Done));
    let model = b.build(Experiment {
        warmup: 0.0,
        duration: 50.0,
        replications: 1,
        ..Experiment::default()
    });
    let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
    // The blocker's entity reaches `next` first (it is created after the desk's, at 0, but the
    // desk holds its own for 1).
    let r = compiled.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
    let d = element(&r, "desk");
    assert_eq!((d.entered, d.left, d.abandoned), (1, 0, Some(1)), "{d:?}");
    assert_eq!(d.wasted, Some(1.0));
    // Held blocked from 1 to 6.
    assert!(
        d.blocking_probability
            .is_some_and(|b| (b - 5.0 / 50.0).abs() < 1e-9),
        "{d:?}"
    );
    assert_eq!(element(&r, "gave_up").mean_time_in_system, Some(3.0));
    let n = element(&r, "next");
    assert_eq!(
        (n.entered, n.left),
        (1, 1),
        "only the blocker's entity: {n:?}"
    );
    balanced(&r);
}

/// The clock starts as an entity enters, before its transfer: patience 5 runs out in a transfer
/// of 8, and the entity gives up as it arrives, at 8, having been there 5 by its patience.
#[test]
fn patience_runs_out_in_transfer_and_the_entity_gives_up_as_it_arrives() {
    for until in [PatienceUntil::Start, PatienceUntil::Done] {
        let mut b = Builder::new();
        let arrivals = b.source("arrivals", Dist::Constant(0.0), Some(1));
        let desk = b.server("desk", 1, Dist::Constant(1.0));
        let exit = b.sink("exit");
        let gave_up = b.sink("gave_up");
        b.connect(arrivals, desk);
        b.connect(desk, exit);
        b.abandoned(desk, gave_up);
        b.transfer(desk, Dist::Constant(8.0));
        b.patience(desk, patience(Dist::Constant(5.0), until));
        let r = b
            .run(Rng::new(1), 0.0, 20.0)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let d = element(&r, "desk");
        assert_eq!((d.entered, d.left, d.abandoned), (1, 0, Some(1)), "{d:?}");
        assert_eq!(d.abandoned_wait, Some(5.0));
        assert_eq!(d.mean_wait, None, "it never started service");
        let g = element(&r, "gave_up");
        assert_eq!(g.mean_time_in_system, Some(8.0), "it gives up on arrival");
        // In transfer from 0 to 8, then gone.
        assert!(
            d.mean_work_in_progress
                .is_some_and(|w| (w - 8.0 / 20.0).abs() < 1e-9),
            "{d:?}"
        );
        balanced(&r);
    }
}

/// Retries: those who give up wait a while and try again, or, at a gateway, give up for good.
/// Every element balances, and the flows between them agree.
#[test]
fn retries_balance_on_every_element() {
    for until in [PatienceUntil::Start, PatienceUntil::Done] {
        let mut b = Builder::new();
        let arrivals = b.source("arrivals", Dist::exponential_rate(1.2), Some(3_000));
        let desk = b.server("desk", 1, Dist::exponential_rate(1.0));
        let exit = b.sink("exit");
        let again = b.exclusive("again");
        let back_off = b.delay("back_off", Dist::Exponential { mean: 3.0 });
        let gave_up = b.sink("gave_up");
        b.connect(arrivals, desk);
        b.connect(desk, exit);
        b.abandoned(desk, again);
        b.route(again, back_off, Some(0.7), None);
        b.route(again, gave_up, Some(0.3), None);
        b.connect(back_off, desk);
        b.patience(desk, patience(Dist::Exponential { mean: 2.0 }, until));
        // Mid-run and once everything has drained.
        for duration in [500.0, 100_000.0] {
            let r = b
                .run(Rng::new(7), 0.0, duration)
                .unwrap_or_else(|e| panic!("{e:?}"));
            let label = format!("{until:?} to {duration}");
            let get = |id: &str| element(&r, id);
            let d = get("desk");
            assert!(d.abandoned.is_some_and(|a| a > 100), "{label}: {d:?}");
            assert_eq!(
                d.entered,
                get("arrivals").left + get("back_off").left,
                "{label}"
            );
            assert_eq!(d.abandoned, Some(get("again").entered), "{label}");
            assert_eq!(
                get("again").left,
                get("back_off").entered + get("gave_up").entered,
                "{label}"
            );
            if duration > 1_000.0 {
                // All gone: every arrival was served once or gave up for good.
                assert_eq!(get("exit").left + get("gave_up").left, 3_000, "{label}");
                balanced(&r);
                if until == PatienceUntil::Done {
                    assert!(d.wasted.is_some_and(|w| w > 0.0), "{label}");
                }
            } else if until == PatienceUntil::Start {
                // Patience until start has no entity both given up and inside, so the balance
                // holds at every moment.
                balanced(&r);
            }
        }
    }
}

/// A finite buffer: an entity giving up frees its place, and arrivals lost to a full buffer are
/// not abandonments.
#[test]
fn giving_up_frees_a_place_in_a_finite_buffer() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(1.0), None);
    let desk = b.buffered_server("desk", 1, Dist::Exponential { mean: 1.5 }, 3);
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.patience(
        desk,
        patience(Dist::Exponential { mean: 2.0 }, PatienceUntil::Start),
    );
    let r = b
        .run(Rng::new(3), 0.0, 5_000.0)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let d = element(&r, "desk");
    assert!(d.abandoned.is_some_and(|a| a > 100), "{d:?}");
    assert!(d.lost.is_some_and(|l| l > 0), "{d:?}");
    // At most 1 in service and 3 waiting, all the time.
    assert!(d.mean_work_in_progress.is_some_and(|w| w <= 4.0));
    assert_eq!(d.lost, element(&r, "arrivals").lost);
    balanced(&r);
}

/// Models without patience report none of the new results.
#[test]
fn servers_without_patience_report_no_abandonment() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(0.5), None);
    let desk = b.server("desk", 1, Dist::exponential_rate(1.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    let r = b
        .run(Rng::new(1), 0.0, 100.0)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let d = element(&r, "desk");
    assert_eq!(
        (d.abandoned, d.abandoned_share, d.abandoned_wait, d.wasted),
        (None, None, None, None)
    );
    // Patience until start wastes nothing, so it reports no waste.
    b.patience(
        desk,
        patience(Dist::exponential_rate(1.0), PatienceUntil::Start),
    );
    let r = b
        .run(Rng::new(1), 0.0, 100.0)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let d = element(&r, "desk");
    assert!(d.abandoned.is_some() && d.wasted.is_none(), "{d:?}");
}

fn problems(b: &Builder) -> Vec<Diagnostic> {
    b.build(Experiment::default()).validate()
}

fn has(ds: &[Diagnostic], f: impl Fn(&Problem) -> bool) -> bool {
    ds.iter().any(|d| f(&d.problem))
}

#[test]
fn the_abandoned_way_is_checked() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(0.5), None);
    let desk = b.server("desk", 1, Dist::exponential_rate(1.0));
    let exit = b.sink("exit");
    let gave_up = b.sink("gave_up");
    let other = b.sink("other");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.abandoned(desk, gave_up);
    // Without a patience: the way is refused.
    let ds = problems(&b);
    assert!(
        has(&ds, |p| matches!(
            p,
            Problem::AbandonedWithoutPatience { .. }
        )),
        "{ds:?}"
    );
    // With one, the server may have two ways out.
    b.patience(desk, patience(Dist::Constant(0.0), PatienceUntil::Start));
    let ds = problems(&b);
    assert!(
        !ds.iter()
            .any(|d| d.severity == stochcraft_model::Severity::Error),
        "{ds:?}"
    );
    // A third way out, as a second abandoned way, is refused.
    b.abandoned(desk, other);
    let ds = problems(&b);
    assert!(
        has(&ds, |p| matches!(
            p,
            Problem::SeveralAbandonedWays { count: 2 }
        )),
        "{ds:?}"
    );
    // Only an abandoned way is not a way out for served entities.
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(0.5), None);
    let desk = b.server("desk", 1, Dist::exponential_rate(1.0));
    let gave_up = b.sink("gave_up");
    b.connect(arrivals, desk);
    b.abandoned(desk, gave_up);
    b.patience(
        desk,
        patience(Dist::exponential_rate(1.0), PatienceUntil::Start),
    );
    assert!(has(&problems(&b), |p| matches!(p, Problem::NoOutput)));
}

#[test]
fn a_negative_patience_is_refused_and_an_overloaded_patient_server_is_not_warned_about() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(2.0), None);
    let desk = b.server("desk", 1, Dist::exponential_rate(1.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    assert!(has(&problems(&b), |p| matches!(
        p,
        Problem::Overloaded { .. }
    )));
    b.patience(
        desk,
        patience(
            Dist::Uniform {
                min: -3.0,
                max: -1.0,
            },
            PatienceUntil::Start,
        ),
    );
    let ds = problems(&b);
    assert!(
        has(&ds, |p| matches!(p, Problem::PatienceNegative)),
        "{ds:?}"
    );
    assert!(
        !has(&ds, |p| matches!(p, Problem::Overloaded { .. })),
        "{ds:?}"
    );
}
