//! Balking (ROADMAP M2.10): arrivals that do not join a server's queue.
//!
//! The statistical checks compare single- and multi-unit servers with Poisson arrivals and
//! exponential service against their birth–death chains, solved here from first principles:
//! with k inside, entities arrive at rate λ·b(n) (n the number the arrival finds: those waiting,
//! max(k − c, 0), or with `counts_in_service` all k) and leave at rate min(k, c)·μ. By PASTA a
//! share 1 − Σ p(k)·b(n(k)) of arrivals balks, and the throughput is Σ p(k)·min(k, c)·μ.
//!
//! Gross and Harris (*Fundamentals of Queueing Theory*, M/M/1 with balking) take b(n) = 1/(n + 1)
//! with n the number in the system: then λ(k) = λ/(k + 1), so p(k) = p(0)·(λ/μ)^k / k! and the
//! number in the system is Poisson with mean ρ = λ/μ, p(0) = e^(−ρ). That gives L = ρ, a share
//! balked of 1 − Σ e^(−ρ) ρ^k / (k + 1)! = 1 − (1 − e^(−ρ))/ρ and a throughput of μ(1 − e^(−ρ)).
#![allow(clippy::panic)]

use stochcraft_blocks::{
    BalkJoin, BalkRow, Balking, BlockId, Builder, ExperimentSummary, Metric, Report, compile,
    summarise,
};
use stochcraft_model::{Experiment, Problem};
use stochcraft_random::math::exp;
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::Summary;

// 64: the share balked is a ratio of counts whose intervals are narrow, and 16 replications
// leave a check of twenty quantities missing one now and then by chance.
const REPLICATIONS: u64 = 64;
const WARMUP: f64 = 1_000.0;
const DURATION: f64 = 20_000.0;

/// The long-run state of a c-unit server with Poisson arrivals at `lambda`, exponential service
/// at `mu` and join probability `b` of the number found: (mean number inside, mean number
/// waiting, share balked, throughput, utilisation).
fn chain(lambda: f64, mu: f64, c: u32, b: &dyn Fn(u32) -> f64) -> (f64, f64, f64, f64, f64) {
    let mut p = vec![1.0_f64];
    let mut k = 0u32;
    loop {
        let up = lambda * b(k);
        k += 1;
        let next = p[p.len() - 1] * up / (f64::from(k.min(c)) * mu);
        p.push(next);
        let total: f64 = p.iter().sum();
        if k > c && next < 1e-14 * total {
            break;
        }
        assert!(k < 100_000, "the chain does not settle");
    }
    let total: f64 = p.iter().sum();
    let (mut inside, mut waiting, mut joins, mut out, mut busy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (k, x) in p.iter().enumerate() {
        let k = k as u32;
        let x = x / total;
        inside += f64::from(k) * x;
        waiting += f64::from(k.saturating_sub(c)) * x;
        joins += b(k) * x;
        out += f64::from(k.min(c)) * mu * x;
        busy += f64::from(k.min(c)) * x;
    }
    (inside, waiting, 1.0 - joins, out, busy / f64::from(c))
}

fn table(rows: &[(u32, f64)]) -> Balking {
    Balking {
        join: BalkJoin::Table(
            rows.iter()
                .map(|&(waiting, probability)| BalkRow {
                    waiting,
                    probability,
                })
                .collect(),
        ),
        counts_in_service: false,
    }
}

/// Arrivals → a server of `c` units with `balking` (if any) → exit.
fn model(lambda: f64, mu: f64, c: u32, balking: Option<Balking>) -> Builder {
    model_desk(lambda, mu, c, balking).0
}

/// [`model`], and its desk.
fn model_desk(lambda: f64, mu: f64, c: u32, balking: Option<Balking>) -> (Builder, BlockId) {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(lambda), None);
    let desk = b.server("desk", c, Dist::exponential_rate(mu));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    if let Some(balking) = balking {
        b.balking(desk, balking);
    }
    (b, desk)
}

fn reports(b: &Builder, warmup: f64, duration: f64) -> Vec<Report> {
    let model = b.build(Experiment {
        warmup,
        duration,
        replications: REPLICATIONS as u32,
        ..Experiment::default()
    });
    let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
    (0..REPLICATIONS)
        .map(|r| {
            compiled
                .run(Rng::new(30_000 + r))
                .unwrap_or_else(|e| panic!("{e}"))
        })
        .collect()
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

/// The entities leaving the desk per time unit, over the replications.
fn throughput(rs: &[Report]) -> Summary {
    let x: Vec<f64> = rs
        .iter()
        .map(|r| {
            let desk = r.element("desk").unwrap_or_else(|| panic!("no desk"));
            desk.left as f64 / r.duration
        })
        .collect();
    Summary::of(&x).unwrap_or_else(|| panic!("no replications"))
}

/// Every element balances: a server's balked and lost arrivals never entered it, and a source's
/// balking arrivals left it. (Over a run with no warm-up.)
fn balanced(r: &Report) {
    for e in &r.elements {
        let lost = if e.kind == "server" {
            0
        } else {
            e.lost.unwrap_or(0)
        };
        let out = e.left + lost + e.abandoned.unwrap_or(0) + e.work_in_progress.unwrap_or(0);
        assert_eq!(e.entered, out, "{} does not balance: {e:?}", e.id);
    }
}

/// Runs the model and checks the number inside, the share balked and the throughput (and the
/// queue and utilisation) against the chain.
fn check(lambda: f64, mu: f64, c: u32, balking: Balking, b: &dyn Fn(u32) -> f64) {
    let (inside, waiting, balked, out, busy) = chain(lambda, mu, c, b);
    eprintln!(
        "λ {lambda} μ {mu} c {c}: L {inside}, Lq {waiting}, balked {balked}, throughput {out}, utilisation {busy}"
    );
    let rs = reports(&model(lambda, mu, c, Some(balking)), WARMUP, DURATION);
    let s = summarise(&rs);
    covers(&s, "desk", Metric::MeanWorkInProgress, inside);
    covers(&s, "desk", Metric::MeanQueueLength, waiting);
    covers(&s, "desk", Metric::BalkedShare, balked);
    covers(&s, "desk", Metric::Utilisation, busy);
    let t = throughput(&rs);
    eprintln!("desk throughput: {t:?} (theory {out})");
    assert!(t.contains(out), "throughput {t:?} vs {out}");
}

/// Gross and Harris's M/M/1 with balking, b(n) = 1/(n + 1) with n in the system: the number in
/// the system is Poisson with mean ρ, so the chain must agree with the closed forms.
fn gross_harris(lambda: f64, mu: f64) {
    let rho = lambda / mu;
    let closed = (rho, 1.0 - (1.0 - exp(-rho)) / rho, mu * (1.0 - exp(-rho)));
    let b = |k: u32| 1.0 / (f64::from(k) + 1.0);
    let (inside, _, balked, out, _) = chain(lambda, mu, 1, &b);
    assert!((inside - closed.0).abs() < 1e-9, "{inside} vs {}", closed.0);
    assert!((balked - closed.1).abs() < 1e-9, "{balked} vs {}", closed.1);
    assert!((out - closed.2).abs() < 1e-9, "{out} vs {}", closed.2);
    check(
        lambda,
        mu,
        1,
        Balking {
            join: BalkJoin::Reciprocal,
            counts_in_service: true,
        },
        &b,
    );
}

#[test]
#[ignore = "long runs against the chain; CI runs it in release with the ignored tests"]
fn reciprocal_balking_in_the_system_is_poisson() {
    // ρ = 0.8: L = 0.8, a share 0.30839 balked, throughput 0.55067.
    gross_harris(0.8, 1.0);
    // ρ = 2, stable only through balking: L = 2, a share 0.56767 balked, throughput 0.86466.
    gross_harris(2.0, 1.0);
}

#[test]
#[ignore = "long runs against the chain; CI runs it in release with the ignored tests"]
fn reciprocal_balking_on_those_waiting_matches_its_chain() {
    // n waiting = max(k − 1, 0): λ(0) = λ(1) = λ, λ(k) = λ/k, so p(k) = p(0) ρ^k / (k − 1)! for
    // k ≥ 1 and p(0) = 1/(1 + ρ e^ρ).
    let (lambda, mu) = (1.5, 1.0);
    let b = |k: u32| 1.0 / f64::from(k.max(1));
    let rho = lambda / mu;
    let (inside, ..) = chain(lambda, mu, 1, &b);
    // E[N] = Σ k p(k) = p(0) ρ e^ρ (1 + ρ).
    let closed = rho * exp(rho) * (1.0 + rho) / (1.0 + rho * exp(rho));
    assert!((inside - closed).abs() < 1e-9, "{inside} vs {closed}");
    check(
        lambda,
        mu,
        1,
        Balking {
            join: BalkJoin::Reciprocal,
            counts_in_service: false,
        },
        &b,
    );
}

#[test]
#[ignore = "long runs against the chain; CI runs it in release with the ignored tests"]
fn a_table_on_two_units_matches_its_chain() {
    // Two units; half join when 2 to 4 wait, a fifth from 5 on.
    let (lambda, mu, c) = (2.4, 1.0, 2);
    let b = move |k: u32| match k.saturating_sub(c) {
        0..=1 => 1.0,
        2..=4 => 0.5,
        _ => 0.2,
    };
    check(lambda, mu, c, table(&[(2, 0.5), (5, 0.2)]), &b);
}

/// M/M/1/K: loss probability and mean number inside.
fn mm1k(lambda: f64, mu: f64, k: u32) -> (f64, f64) {
    let rho = lambda / mu;
    // ρ^n for n = 0..=k, by multiplication.
    let powers: Vec<f64> = (0..=k)
        .scan(1.0, |x, _| {
            let now = *x;
            *x *= rho;
            Some(now)
        })
        .collect();
    let norm: f64 = powers.iter().sum();
    let loss = powers.last().copied().unwrap_or(1.0) / norm;
    let inside = powers
        .iter()
        .enumerate()
        .map(|(n, x)| n as f64 * x)
        .sum::<f64>()
        / norm;
    (loss, inside)
}

#[test]
#[ignore = "long runs against the chain; CI runs it in release with the ignored tests"]
fn joining_below_a_length_only_is_mm1k() {
    // b = 1 below L = 3 waiting, 0 from it: at most 3 waiting plus 1 in service, M/M/1/4.
    let (lambda, mu, l) = (0.9, 1.0, 3);
    let (loss, inside) = mm1k(lambda, mu, l + 1);
    eprintln!("M/M/1/{}: loss {loss}, L {inside}", l + 1);
    let b = move |k: u32| if k.saturating_sub(1) < l { 1.0 } else { 0.0 };
    let (chain_inside, _, chain_balked, ..) = chain(lambda, mu, 1, &b);
    assert!((chain_inside - inside).abs() < 1e-9);
    assert!((chain_balked - loss).abs() < 1e-9);
    check(lambda, mu, 1, table(&[(l, 0.0)]), &b);
}

/// Not joining at or above a length is a waiting room of that length with a way out for those
/// turned away: run for run, those balking are exactly those turned away, and everything else is
/// the same (a chance of 0 or 1 draws nothing).
#[test]
fn joining_below_a_length_is_the_same_as_a_waiting_room() {
    let (lambda, mu, l) = (0.9, 1.0, 3);
    let balking = reports(
        &model(lambda, mu, 1, Some(table(&[(l, 0.0)]))),
        50.0,
        2_000.0,
    );
    let mut room = Builder::new();
    let arrivals = room.source("arrivals", Dist::exponential_rate(lambda), None);
    let desk = room.buffered_server("desk", 1, Dist::exponential_rate(mu), l);
    let exit = room.sink("exit");
    let away = room.sink("away");
    room.connect(arrivals, desk);
    room.connect(desk, exit);
    room.turned_away(desk, away);
    let room = reports(&room, 50.0, 2_000.0);
    for (b, r) in balking.iter().zip(&room) {
        let (b, r) = (
            b.element("desk").unwrap_or_else(|| panic!("no desk")),
            r.element("desk").unwrap_or_else(|| panic!("no desk")),
        );
        assert!(b.balked.is_some_and(|n| n > 0), "{b:?}");
        assert_eq!(b.balked, r.lost);
        assert_eq!(b.entered, r.entered);
        assert_eq!(b.left, r.left);
        assert_eq!(b.mean_work_in_progress, r.mean_work_in_progress);
        assert_eq!(b.mean_queue_length, r.mean_queue_length);
        assert_eq!(b.mean_wait, r.mean_wait);
    }
}

/// A table that always joins changes nothing: the same results, to the last bit, as no
/// balking, apart from the balking rows themselves.
#[test]
fn always_joining_changes_nothing() {
    let plain = reports(&model(0.9, 1.0, 2, None), 50.0, 2_000.0);
    let ones = reports(
        &model(0.9, 1.0, 2, Some(table(&[(0, 1.0), (4, 1.0)]))),
        50.0,
        2_000.0,
    );
    for (p, o) in plain.iter().zip(&ones) {
        let mut o = o.clone();
        for e in &mut o.elements {
            if e.id.as_str() == "desk" {
                assert_eq!(e.balked, Some(0));
                assert_eq!(e.balked_share, Some(0.0));
                e.balked = None;
                e.balked_share = None;
            }
        }
        assert_eq!(p, &o);
    }
}

/// Balking draws from the server's own stream: adding it to one server leaves another server's
/// draws alone, and a different chance gives different results there.
#[test]
fn balking_draws_from_its_own_stream() {
    let line = |balking: Option<Balking>| {
        let mut b = Builder::new();
        let arrivals = b.source("arrivals", Dist::exponential_rate(0.9), None);
        let first = b.server("first", 1, Dist::exponential_rate(1.0));
        let desk = b.server("desk", 1, Dist::exponential_rate(1.5));
        let exit = b.sink("exit");
        b.connect(arrivals, first);
        b.connect(first, desk);
        b.connect(desk, exit);
        if let Some(balking) = balking {
            b.balking(desk, balking);
        }
        reports(&b, 0.0, 500.0)
    };
    let plain = line(None);
    let half = line(Some(table(&[(1, 0.5)])));
    for (p, h) in plain.iter().zip(&half) {
        let first = |r: &Report| r.element("first").cloned();
        // The first server, upstream, is untouched.
        assert_eq!(first(p), first(h));
        let d = h.element("desk").unwrap_or_else(|| panic!("no desk"));
        assert!(d.balked.is_some_and(|n| n > 0), "{d:?}");
        balanced(h);
    }
}

/// Those that balk take the way marked for them (here a delay back to the desk, a retry), each
/// balk counted, and every element balances.
#[test]
fn those_balking_take_their_way_and_try_again() {
    let (mut b, desk) = model_desk(0.7, 1.0, 1, Some(table(&[(2, 0.0)])));
    let later = b.delay("later", Dist::exponential_rate(0.5));
    b.balked(desk, later);
    b.connect(later, desk);
    let rs = reports(&b, 0.0, 1_000.0);
    for r in &rs {
        balanced(r);
        let d = r.element("desk").unwrap_or_else(|| panic!("no desk"));
        let l = r.element("later").unwrap_or_else(|| panic!("no later"));
        let balked = d.balked.unwrap_or(0);
        assert!(balked > 0);
        // Every balk went to the delay.
        assert_eq!(l.entered, balked);
        // At most 2 waiting plus 1 in service, ever: the room never grows past the table's 0.
        assert!(d.mean_queue_length.is_some_and(|q| q <= 2.0));
    }
}

/// The checks on a balking table and its ways.
#[test]
fn bad_tables_and_ways_are_reported() {
    let codes = |b: &Builder| -> Vec<&'static str> {
        b.build(Experiment::default())
            .validate()
            .into_iter()
            .map(|d| d.problem.code())
            .collect()
    };
    let b = model(1.0, 1.0, 1, Some(table(&[(3, 0.5), (1, 1.5)])));
    let c = codes(&b);
    assert!(c.contains(&"balking_rows_order"), "{c:?}");
    assert!(c.contains(&"balking_probability"), "{c:?}");

    let (mut b, desk) = model_desk(1.0, 1.0, 1, None);
    let later = b.delay("later", Dist::Constant(1.0));
    b.balked(desk, later);
    b.connect(later, desk);
    assert!(codes(&b).contains(&"balked_without_balking"));
    b.balking(desk, table(&[(1, 0.5)]));
    let other = b.delay("other", Dist::Constant(1.0));
    b.balked(desk, other);
    b.connect(other, desk);
    assert!(codes(&b).contains(&"several_balked_ways"));
    let _ = Problem::BalkingRowsOrder;
}
