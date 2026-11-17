//! The start page's examples (#149), run from their committed model files (written by
//! `stochcraft-cli new -t <name>`) and checked against exact answers, as ROADMAP's "Examples for
//! teaching" requires of every example:
//!
//! - the coffee counter against the M/M/1 and M/M/2 formulas (Erlang C);
//! - emergency triage against Cobham's non-preemptive priority waits on M/M/5;
//! - the call centre against the birth–death chain of M/M/c+M (Erlang-A);
//! - the weld and paint line, and the web service with health checks, against their
//!   continuous-time Markov chains, solved here from first principles.
//!
//! Each check allows √2 times the 95% half-width, since several checks read the same runs (as in
//! `crates/blocks/tests/queueing_cases.rs`).
#![allow(clippy::panic)]

use std::path::Path;

use serde_json::{Value, json};
use stochcraft_engine::Engine;

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

/// The engine with example `file` of `tests/models` open, with `experiment` set over its own.
fn open(file: &str, experiment: Value) -> Engine {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/models")
        .join(file);
    let mut e = Engine::new();
    let opened = ok(
        &mut e,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    assert_eq!(opened["notes"], json!([]), "{file}: {opened}");
    ok(&mut e, "experiment.set", experiment);
    e
}

/// The mean and 95% half-width of `metric` of `element` in run results `r`.
fn result(r: &Value, element: &str, metric: &str) -> (f64, f64) {
    let m = r["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|e| e["id"] == element))
        .map(|e| &e["metrics"][metric])
        .unwrap_or_else(|| panic!("no {element}"));
    match (m["mean"].as_f64(), m["half_width"].as_f64()) {
        (Some(mean), Some(hw)) => (mean, hw),
        _ => panic!("no {metric} for {element}: {m}"),
    }
}

/// The theory lies within √2 times the 95% half-width of the mean.
fn covers(r: &Value, element: &str, metric: &str, theory: f64) {
    let (mean, hw) = result(r, element, metric);
    let reach = std::f64::consts::SQRT_2 * hw;
    eprintln!("{element} {metric}: {mean} ± {hw}, theory {theory}");
    assert!(
        (mean - theory).abs() <= reach,
        "{element} {metric}: {mean} ± {hw}, theory {theory}"
    );
}

/// Erlang C: the chance an arrival waits, with `c` servers and offered load `a` < c.
fn erlang_c(c: u32, a: f64) -> f64 {
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..c {
        term *= a / f64::from(k);
        sum += term;
    }
    let last = term * a / f64::from(c) / (1.0 - a / f64::from(c));
    last / (sum + last)
}

/// The stationary distribution of a finite chain given each state's moves `(to, rate)`, by
/// uniformisation.
fn stationary(moves: &[Vec<(usize, f64)>]) -> Vec<f64> {
    let top = moves
        .iter()
        .map(|m| m.iter().map(|(_, q)| q).sum::<f64>())
        .fold(0.0, f64::max)
        * 1.05;
    let mut p = vec![1.0 / moves.len() as f64; moves.len()];
    for round in 0.. {
        let mut next = vec![0.0; moves.len()];
        for (s, ms) in moves.iter().enumerate() {
            let out: f64 = ms.iter().map(|(_, q)| q).sum();
            next[s] += p[s] * (1.0 - out / top);
            for (t, q) in ms {
                next[*t] += p[s] * q / top;
            }
        }
        let change: f64 = next.iter().zip(&p).map(|(a, b)| (a - b).abs()).sum();
        p = next;
        if change < 1e-14 {
            break;
        }
        assert!(round < 5_000_000, "the chain does not settle");
    }
    p
}

/// One barista at ρ = 0.8 waits 4 minutes on average (M/M/1: Wq = ρ / (μ − λ)); a second
/// barista brings it down to Erlang C's 0.1905 minutes.
#[test]
fn the_coffee_counter_matches_erlang_c() {
    let (lambda, mu) = (0.8, 1.0);
    let mut e = open(
        "coffee.bpmn",
        json!({ "replications": 20, "duration": 50_000.0 }),
    );
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "barista", "mean_wait", lambda / mu / (mu - lambda));
    covers(&r, "barista", "utilisation", lambda / mu);
    ok(
        &mut e,
        "block.set",
        json!({ "id": "barista", "capacity": 2 }),
    );
    let r = ok(&mut e, "run", json!({}));
    let wq = erlang_c(2, lambda / mu) / (2.0 * mu - lambda);
    assert!((wq - 0.190_476).abs() < 1e-5, "{wq}");
    covers(&r, "barista", "mean_wait", wq);
}

/// Three triage classes sharing five nurses, the higher priority first: Cobham's waits
/// W0 / ((1 − σₖ₋₁)(1 − σₖ)) with W0 = C(5, 3) / (cμ), as in queueing_cases.rs.
#[test]
fn triage_matches_cobham() {
    let w0 = erlang_c(5, 3.0) / (5.0 / 15.0);
    let mut e = open(
        "triage.bpmn",
        json!({ "replications": 32, "warmup": 10_000.0, "duration": 250_000.0 }),
    );
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "urgent", "mean_wait", w0 / (1.00 * 0.88));
    covers(&r, "standard", "mean_wait", w0 / (0.88 * 0.58));
    covers(&r, "minor", "mean_wait", w0 / (0.58 * 0.40));
    covers(&r, "nurses", "utilisation", 0.6);
}

/// Callers who hang up after an exponential patience: with n calls in, arrivals at λ and
/// departures at min(n, c)·μ + max(n − c, 0)·θ. A share θ·E[Nq]/λ hangs up.
#[test]
fn the_call_centre_matches_erlang_a() {
    let (lambda, mu, theta, c) = (1.0, 0.4, 0.2, 3usize);
    let n = 200;
    let moves: Vec<Vec<(usize, f64)>> = (0..=n)
        .map(|k| {
            let mut m = Vec::new();
            if k < n {
                m.push((k + 1, lambda));
            }
            if k > 0 {
                let down = k.min(c) as f64 * mu + k.saturating_sub(c) as f64 * theta;
                m.push((k - 1, down));
            }
            m
        })
        .collect();
    let p = stationary(&moves);
    let queue: f64 = p
        .iter()
        .enumerate()
        .map(|(k, x)| k.saturating_sub(c) as f64 * x)
        .sum();
    let busy: f64 = p
        .iter()
        .enumerate()
        .map(|(k, x)| k.min(c) as f64 * x)
        .sum::<f64>()
        / c as f64;
    let mut e = open(
        "call-centre.bpmn",
        json!({ "replications": 20, "duration": 50_000.0 }),
    );
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "agents", "mean_queue_length", queue);
    covers(&r, "agents", "abandoned_share", theta * queue / lambda);
    covers(&r, "agents", "utilisation", busy);
}

/// The weld and paint line as a chain. Weld (one unit, three waiting places) feeds Paint (one
/// unit, two places), which breaks down in elapsed time and resumes an interrupted job after its
/// repair. The state is (n₁ at Weld, Weld blocked, n₂ at Paint, Paint up, a place kept by Paint
/// down without a part):
/// - an arrival finding Weld full (n₁ = 4) is lost;
/// - Weld finishing passes the part on if Paint has room (n₂ + kept < 3), else holds it: blocked;
/// - Paint finishing (up, n₂ > 0) frees a place, which a blocked Weld fills at once;
/// - Paint failing while empty keeps one of its places until repaired; failing while working
///   holds its part;
/// - a repair frees the kept place, which a blocked Weld fills at once.
#[test]
fn the_weld_and_paint_line_matches_its_chain() {
    let (lambda, mu1, mu2, fail, repair) =
        (1.0 / 6.5, 1.0 / 4.0, 1.0 / 5.0, 1.0 / 120.0, 1.0 / 15.0);
    let (cap1, cap2) = (4usize, 3usize);
    // States (n1, blocked, n2, up, kept) and their indices.
    let mut states = Vec::new();
    for n1 in 0..=cap1 {
        for blocked in [false, true] {
            for n2 in 0..=cap2 {
                for up in [true, false] {
                    for kept in [false, true] {
                        // Weld blocked holds a part; only Paint down keeps a place, and never
                        // more places than it has.
                        if (blocked && n1 == 0) || (kept && up) || n2 + usize::from(kept) > cap2 {
                            continue;
                        }
                        states.push((n1, blocked, n2, up, kept));
                    }
                }
            }
        }
    }
    let index = |s: (usize, bool, usize, bool, bool)| {
        states
            .iter()
            .position(|x| *x == s)
            .unwrap_or_else(|| panic!("no state {s:?}"))
    };
    let room = |n2: usize, kept: bool| cap2 - n2 - usize::from(kept);
    // A blocked Weld hands its part to Paint once there is room.
    let settle = |(n1, blocked, n2, up, kept): (usize, bool, usize, bool, bool)| {
        if blocked && room(n2, kept) > 0 {
            (n1 - 1, false, n2 + 1, up, kept)
        } else {
            (n1, blocked, n2, up, kept)
        }
    };
    let moves: Vec<Vec<(usize, f64)>> = states
        .iter()
        .map(|&(n1, blocked, n2, up, kept)| {
            let mut m = Vec::new();
            if n1 < cap1 {
                m.push((index((n1 + 1, blocked, n2, up, kept)), lambda));
            }
            if n1 > 0 && !blocked {
                let next = if room(n2, kept) > 0 {
                    (n1 - 1, false, n2 + 1, up, kept)
                } else {
                    (n1, true, n2, up, kept)
                };
                m.push((index(next), mu1));
            }
            if up && n2 > 0 {
                m.push((index(settle((n1, blocked, n2 - 1, up, kept))), mu2));
            }
            if up {
                m.push((index((n1, blocked, n2, false, n2 == 0)), fail));
            } else {
                m.push((index(settle((n1, blocked, n2, true, false))), repair));
            }
            m
        })
        .collect();
    let p = stationary(&moves);
    let (mut lost, mut shipped, mut paint_queue) = (0.0, 0.0, 0.0);
    for (x, &(n1, _, n2, up, kept)) in p.iter().zip(&states) {
        if n1 == cap1 {
            lost += x;
        }
        if up && n2 > 0 {
            shipped += x * mu2;
        }
        // Paint holds a part unless it is down keeping an empty place.
        let holds = n2 > 0 && !kept;
        paint_queue += x * (n2 - usize::from(holds)) as f64;
    }
    let duration = 200_000.0;
    let mut e = open(
        "weld-paint.bpmn",
        json!({ "replications": 20, "warmup": 5_000.0, "duration": duration }),
    );
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "weld", "loss_probability", lost);
    covers(&r, "paint", "mean_queue_length", paint_queue);
    covers(&r, "shipped", "entered", shipped * duration);
}

/// The web service: three app servers checked once a second on average, a server taken out for a
/// restart (mean 2 s) when more than half a request waits per working server, two always kept,
/// the request under way retried. The chain of `crates/blocks/tests/health_check.rs`:
/// (n inside, k servers out).
#[test]
fn the_web_service_matches_its_chain() {
    let (lambda, mu, c, gamma, limit, restart, keep) = (1.5, 1.0, 3usize, 1.0, 0.5, 0.5, 2usize);
    let n_max = 150;
    let at = |n: usize, k: usize| n * (c + 1) + k;
    let fails = |n: usize, k: usize| {
        let w = c - k;
        w > 0 && (n - n.min(w)) as f64 > limit * w as f64 && w > keep
    };
    let mut moves: Vec<Vec<(usize, f64)>> = vec![Vec::new(); (n_max + 1) * (c + 1)];
    for n in 0..=n_max {
        for k in 0..=c {
            let w = c - k;
            let m = &mut moves[at(n, k)];
            if n < n_max {
                m.push((at(n + 1, k), lambda));
            }
            if n.min(w) > 0 {
                m.push((at(n - 1, k), n.min(w) as f64 * mu));
            }
            if k > 0 {
                m.push((at(n, k - 1), k as f64 * restart));
            }
            if fails(n, k) {
                m.push((at(n, k + 1), w as f64 * gamma));
            }
        }
    }
    let p = stationary(&moves);
    let (mut queue, mut in_service) = (0.0, 0.0);
    for n in 0..=n_max {
        for k in 0..=c {
            let x = p[at(n, k)];
            let w = c - k;
            queue += (n - n.min(w)) as f64 * x;
            in_service += w as f64 * x;
        }
    }
    let mut e = open(
        "it-service.bpmn",
        json!({ "replications": 16, "duration": 20_000.0 }),
    );
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "servers", "mean_queue_length", queue);
    covers(&r, "servers", "mean_in_service", in_service);
}
