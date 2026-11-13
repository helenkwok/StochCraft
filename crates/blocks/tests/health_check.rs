//! Health checks (ROADMAP M2.10): servers whose units are taken out of service for a restart
//! when too many entities wait, as a load balancer takes out servers that fail its probes.
//!
//! The statistical check compares a server with exponential arrivals, service, checks and
//! restarts, retrying the job under way, with its continuous-time Markov chain, solved
//! numerically here from first principles. The state is (n inside, k units out); with w = c − k
//! units working, entities arrive at rate λ, jobs finish at rate min(n, w)·μ, each unit out comes
//! back at rate r, and each working unit is checked at rate γ, taken out when more than L·w wait
//! (n − min(n, w) > L·w) and at least `keep` would still work. A unit taken out while serving
//! sends its entity back to the queue, so n does not change; since service is exponential, the
//! retried job's fresh start changes nothing else.
#![allow(clippy::panic)]

use stochcraft_blocks::{
    Builder, ElementReport, ExperimentSummary, HealthCheck, Metric, OnTakeout, Patience,
    PatienceUntil, Report, compile, summarise,
};
use stochcraft_model::{Diagnostic, Experiment, Problem};
use stochcraft_random::{Dist, Rng};

const REPLICATIONS: u64 = 16;

fn check(
    interval: Dist,
    limit: f64,
    restart: Dist,
    keep: Option<u32>,
    on: OnTakeout,
) -> HealthCheck {
    HealthCheck {
        interval,
        limit,
        restart,
        keep,
        on_takeout: on,
    }
}

fn element<'a>(r: &'a Report, id: &str) -> &'a ElementReport {
    r.element(id).unwrap_or_else(|| panic!("no {id}"))
}

/// Arrivals → a server of `c` units → exit, with `health` on the server if given.
fn mmc(lambda: f64, mu: f64, c: u32, health: Option<HealthCheck>) -> Builder {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(lambda), None);
    let desk = b.server("desk", c, Dist::exponential_rate(mu));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    if let Some(h) = health {
        b.health_check(desk, h);
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
                .run(Rng::new(30_000 + r))
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

/// Every element balances: what entered left, gave up or is still inside.
fn balanced(r: &Report) {
    for e in &r.elements {
        let out = e.left + e.abandoned.unwrap_or(0) + e.work_in_progress.unwrap_or(0);
        assert_eq!(e.entered, out, "{} does not balance: {e:?}", e.id);
    }
}

/// A limit never reached takes no unit out and changes nothing, to the last bit: the checks draw
/// from a stream of their own, so the arrivals and services are those of the server without a
/// health check.
#[test]
fn a_limit_never_reached_changes_nothing() {
    let plain = mmc(2.4, 1.0, 3, None);
    for limit in [f64::INFINITY, 1e12] {
        let checked = mmc(
            2.4,
            1.0,
            3,
            Some(check(
                Dist::Exponential { mean: 0.5 },
                limit,
                Dist::Constant(5.0),
                None,
                OnTakeout::Retry,
            )),
        );
        for seed in 0..4 {
            let a = plain
                .run(Rng::new(seed), 100.0, 2_000.0)
                .unwrap_or_else(|e| panic!("{e:?}"));
            let b = checked
                .run(Rng::new(seed), 100.0, 2_000.0)
                .unwrap_or_else(|e| panic!("{e:?}"));
            let (x, y) = (element(&a, "desk"), element(&b, "desk"));
            assert_eq!(y.taken_out, Some(0), "{y:?}");
            assert_eq!(y.time_restarting, Some(0.0));
            assert_eq!(y.wasted, Some(0.0));
            assert!(
                y.mean_in_service.is_some_and(|m| (m - 3.0).abs() < 1e-9),
                "{y:?}"
            );
            assert_eq!(x.taken_out, None);
            for id in ["arrivals", "desk", "exit"] {
                let (x, y) = (element(&a, id), element(&b, id));
                assert_eq!((x.entered, x.left), (y.entered, y.left), "{id}");
                for m in [
                    Metric::MeanQueueLength,
                    Metric::MeanWait,
                    Metric::Utilisation,
                    Metric::MeanTimeInSystem,
                    Metric::TimeProcessing,
                    Metric::TimeStarved,
                ] {
                    assert_eq!(x.metric(m), y.metric(m), "{id} {}", m.name());
                }
            }
        }
    }
}

/// The chain of the module comment: (mean number waiting, takeouts per time unit, mean units
/// in service, mean busy share).
fn chain(
    lambda: f64,
    mu: f64,
    c: u32,
    gamma: f64,
    limit: f64,
    r: f64,
    keep: u32,
) -> (f64, f64, f64, f64) {
    const N: usize = 150;
    let c = c as usize;
    let at = |n: usize, k: usize| n * (c + 1) + k;
    let size = (N + 1) * (c + 1);
    // Each state's moves: (to, rate).
    let mut moves: Vec<Vec<(usize, f64)>> = vec![Vec::new(); size];
    let fails = |n: usize, k: usize| {
        let w = c - k;
        let waiting = n - n.min(w);
        w > 0 && waiting as f64 > limit * w as f64 && w > keep as usize
    };
    for n in 0..=N {
        for k in 0..=c {
            let w = c - k;
            let m = &mut moves[at(n, k)];
            if n < N {
                m.push((at(n + 1, k), lambda));
            }
            if n > 0 && n.min(w) > 0 {
                m.push((at(n - 1, k), n.min(w) as f64 * mu));
            }
            if k > 0 {
                m.push((at(n, k - 1), k as f64 * r));
            }
            if fails(n, k) {
                m.push((at(n, k + 1), w as f64 * gamma));
            }
        }
    }
    // Uniformise and iterate to the stationary distribution.
    let top = lambda + c as f64 * (mu + gamma + r);
    let mut p = vec![0.0_f64; size];
    p[0] = 1.0;
    for round in 0.. {
        let mut next = vec![0.0_f64; size];
        for (s, ms) in moves.iter().enumerate() {
            let out: f64 = ms.iter().map(|(_, q)| q).sum();
            next[s] += p[s] * (1.0 - out / top);
            for (t, q) in ms {
                next[*t] += p[s] * q / top;
            }
        }
        let change: f64 = next.iter().zip(&p).map(|(a, b)| (a - b).abs()).sum();
        p = next;
        if change < 1e-13 {
            break;
        }
        assert!(round < 2_000_000, "the chain does not settle");
    }
    assert!(
        p[at(N, 0)..].iter().sum::<f64>() < 1e-12,
        "truncated too soon"
    );
    let (mut queue, mut takeouts, mut in_service, mut busy) = (0.0, 0.0, 0.0, 0.0);
    for n in 0..=N {
        for k in 0..=c {
            let x = p[at(n, k)];
            let w = c - k;
            queue += (n - n.min(w)) as f64 * x;
            if fails(n, k) {
                takeouts += w as f64 * gamma * x;
            }
            in_service += w as f64 * x;
            busy += n.min(w) as f64 * x;
        }
    }
    (queue, takeouts, in_service, busy / c as f64)
}

/// Exponential checks, restarts and service, retrying the job under way: the server's mean
/// queue, takeouts, units in service and utilisation match the chain. Three units, two always
/// kept, so the server cannot collapse; a unit is taken out when more than half an entity waits
/// per working unit.
#[test]
fn retrying_takeouts_match_the_chain() {
    let (lambda, mu, c, gamma, limit, r, keep) = (1.5, 1.0, 3, 1.0, 0.5, 0.5, 2);
    let (queue, rate, in_service, busy) = chain(lambda, mu, c, gamma, limit, r, keep);
    eprintln!("Lq {queue}, takeouts/t {rate}, in service {in_service}, busy {busy}");
    let duration = 20_000.0;
    let b = mmc(
        lambda,
        mu,
        c,
        Some(check(
            Dist::exponential_rate(gamma),
            limit,
            Dist::exponential_rate(r),
            Some(keep),
            OnTakeout::Retry,
        )),
    );
    let (s, reports) = run(&b, 1_000.0, duration);
    covers(&s, "desk", Metric::MeanQueueLength, queue);
    covers(&s, "desk", Metric::TakenOut, rate * duration);
    covers(&s, "desk", Metric::MeanInService, in_service);
    covers(&s, "desk", Metric::Utilisation, busy);
    covers(
        &s,
        "desk",
        Metric::TimeRestarting,
        1.0 - in_service / f64::from(c),
    );
    for r in &reports {
        assert!(element(r, "desk").wasted.is_some_and(|w| w > 0.0));
    }
}

/// Ten units, a restart of 20, checks every 1 on average, failing at more than 2 waiting per
/// working unit. At half load the queue never gets near 20, and no unit is taken out; near full
/// load the queue now and then passes it, units are taken out, the queue grows for want of them
/// and more are taken out: the server collapses.
#[test]
fn a_server_collapses_under_high_load_and_not_under_low() {
    let health = check(
        Dist::Exponential { mean: 1.0 },
        2.0,
        Dist::Constant(20.0),
        None,
        OnTakeout::Retry,
    );
    let (low, _) = run(&mmc(5.0, 1.0, 10, Some(health.clone())), 100.0, 2_000.0);
    let taken = |s: &ExperimentSummary| s.get("desk", Metric::TakenOut).map(|x| x.mean);
    let in_service = |s: &ExperimentSummary| s.get("desk", Metric::MeanInService).map(|x| x.mean);
    assert_eq!(taken(&low), Some(0.0));
    assert!(in_service(&low).is_some_and(|m| (m - 10.0).abs() < 1e-9));
    let (high, _) = run(&mmc(9.8, 1.0, 10, Some(health)), 100.0, 2_000.0);
    eprintln!(
        "taken out {:?}, in service {:?}",
        taken(&high),
        in_service(&high)
    );
    assert!(taken(&high).is_some_and(|t| t > 50.0));
    assert!(in_service(&high).is_some_and(|m| m < 8.0));
}

/// Keeping at least 6 of 10 in service: however overloaded, no check takes out a unit that
/// would leave fewer than 6 working, so at least 6 always are; without it, far fewer are.
#[test]
fn keep_at_least_is_respected() {
    let with = |keep| {
        mmc(
            20.0,
            1.0,
            10,
            Some(check(
                Dist::Exponential { mean: 1.0 },
                0.0,
                Dist::Exponential { mean: 10.0 },
                keep,
                OnTakeout::Retry,
            )),
        )
    };
    let (_, kept) = run(&with(Some(6)), 0.0, 500.0);
    for r in &kept {
        let d = element(r, "desk");
        assert!(d.mean_in_service.is_some_and(|m| m >= 6.0 - 1e-9), "{d:?}");
        assert!(d.time_restarting.is_some_and(|t| t <= 0.4 + 1e-9), "{d:?}");
    }
    let (s, _) = run(&with(None), 0.0, 500.0);
    assert!(
        s.get("desk", Metric::MeanInService)
            .is_some_and(|m| m.mean < 3.0),
        "{s:?}"
    );
}

/// One unit, service 3, three arrivals at 0 with a patience of 7 until service starts, and a
/// check every 4 that fails when anyone waits, with a restart of 2. A is served from 0 to 3, then
/// B starts. At 4 the check fails (C waits): B's job goes back to the head of the queue, its
/// 1 of work wasted, with a fresh patience to 11. The unit is back at 6 and takes B, first in
/// line, which is done at 9; C's patience runs out at 7. Had B gone to the back, C would have
/// started at 6, in time, and no one would have given up.
#[test]
fn a_retried_job_goes_back_first_in_line() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::Constant(0.0), Some(3));
    let desk = b.server("desk", 1, Dist::Constant(3.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.patience(
        desk,
        Patience {
            time: Dist::Constant(7.0),
            until: PatienceUntil::Start,
        },
    );
    b.health_check(
        desk,
        check(
            Dist::Constant(4.0),
            0.0,
            Dist::Constant(2.0),
            None,
            OnTakeout::Retry,
        ),
    );
    let r = b
        .run(Rng::new(1), 0.0, 100.0)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let d = element(&r, "desk");
    assert_eq!(d.taken_out, Some(1), "{d:?}");
    assert_eq!(d.wasted, Some(1.0), "{d:?}");
    assert_eq!(d.abandoned, Some(1), "{d:?}");
    assert_eq!(d.left, 2);
    // Waits from arrival to the start of the service that finished: A 0, B 6.
    assert_eq!(d.mean_wait, Some(3.0), "{d:?}");
    // A done at 3, B at 9.
    assert_eq!(element(&r, "exit").mean_time_in_system, Some(6.0));
    assert!(
        d.time_restarting.is_some_and(|t| (t - 0.02).abs() < 1e-12),
        "{d:?}"
    );
    balanced(&r);
}

/// One unit, service 10, two arrivals at 0 with a patience of 5 until service starts, a check
/// every 2 that fails when anyone waits, and a restart of 4. At 2 A's job is retried, with a
/// fresh patience to 7; B's runs out at 5, while the unit is out. The unit is back at 6 and
/// serves A, within its new patience (its first would have run out at 5 too).
#[test]
fn a_retried_entity_has_a_fresh_patience() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::Constant(0.0), Some(2));
    let desk = b.server("desk", 1, Dist::Constant(10.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.patience(
        desk,
        Patience {
            time: Dist::Constant(5.0),
            until: PatienceUntil::Start,
        },
    );
    b.health_check(
        desk,
        check(
            Dist::Constant(2.0),
            0.0,
            Dist::Constant(4.0),
            None,
            OnTakeout::Retry,
        ),
    );
    let r = b
        .run(Rng::new(1), 0.0, 100.0)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let d = element(&r, "desk");
    assert_eq!(d.taken_out, Some(1), "{d:?}");
    assert_eq!(d.abandoned, Some(1), "{d:?}");
    assert_eq!(d.left, 1);
    assert_eq!(d.wasted, Some(2.0));
    assert_eq!(d.mean_wait, Some(6.0));
    assert_eq!(element(&r, "exit").mean_time_in_system, Some(16.0));
    balanced(&r);
}

/// One unit, service 10, two arrivals at 0, a check every 4 that fails when anyone waits, a
/// restart of 3, and the job resuming after it. A works 0–4, waits out 4–7, works 7–11, waits out
/// 11–14 and is done at 16 with nothing wasted; B, alone then, is served 16–26.
#[test]
fn a_resumed_job_continues_with_what_was_left() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::Constant(0.0), Some(2));
    let desk = b.server("desk", 1, Dist::Constant(10.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.health_check(
        desk,
        check(
            Dist::Constant(4.0),
            0.0,
            Dist::Constant(3.0),
            None,
            OnTakeout::Resume,
        ),
    );
    let r = b
        .run(Rng::new(1), 0.0, 100.0)
        .unwrap_or_else(|e| panic!("{e:?}"));
    let d = element(&r, "desk");
    assert_eq!(d.taken_out, Some(2), "{d:?}");
    assert_eq!(d.wasted, None, "nothing is wasted: {d:?}");
    assert_eq!(d.left, 2);
    assert_eq!(element(&r, "exit").mean_time_in_system, Some(21.0));
    assert!(
        d.time_restarting.is_some_and(|t| (t - 0.06).abs() < 1e-12),
        "{d:?}"
    );
    // Processing 20 of the 100, restarting 6 (a unit out with a job is restarting).
    assert!(
        d.time_processing.is_some_and(|t| (t - 0.2).abs() < 1e-12),
        "{d:?}"
    );
    balanced(&r);
}

/// A health check needs an unlimited waiting room, a limit of 0 or more and a positive time
/// between checks.
#[test]
fn a_health_check_on_a_finite_buffer_or_with_bad_values_is_refused() {
    let has = |ds: &[Diagnostic], f: fn(&Problem) -> bool| ds.iter().any(|d| f(&d.problem));
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::Exponential { mean: 1.0 }, None);
    let desk = b.buffered_server("desk", 1, Dist::Exponential { mean: 0.5 }, 5);
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.health_check(
        desk,
        check(
            Dist::Constant(0.0),
            -1.0,
            Dist::Constant(1.0),
            None,
            OnTakeout::Retry,
        ),
    );
    let ds = compile(&b.build(Experiment::default()))
        .err()
        .unwrap_or_default();
    assert!(
        has(&ds, |p| matches!(p, Problem::HealthCheckWithFiniteBuffer)),
        "{ds:?}"
    );
    assert!(
        has(&ds, |p| matches!(p, Problem::HealthCheckLimitNotValid)),
        "{ds:?}"
    );
    assert!(
        has(&ds, |p| matches!(
            p,
            Problem::HealthCheckIntervalNotPositive
        )),
        "{ds:?}"
    );
}
