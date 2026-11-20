//! The start page's examples (#149), run from their committed model files and checked against
//! exact answers, as ROADMAP's "Examples for teaching" requires of every example:
//!
//! - the coffee counter against the M/M/1 and M/M/2 formulas (Erlang C);
//! - emergency triage against Cobham's non-preemptive priority waits on M/M/5;
//! - the call centre against the birth–death chain of M/M/c+M (Erlang-A);
//! - the weld and paint line, and the web service with health checks, against their
//!   continuous-time Markov chains, solved here from first principles;
//! - the bank of Borshchev and Grigoryev's Example 2.2 against flow balance and, at its one ATM,
//!   Pollaczek–Khinchine;
//! - hospital beds against Erlang B, which holds for any length of stay;
//! - the machine repair crew against the finite-source chain M/M/R//6;
//! - inspection with rework against the product form of its Jackson network;
//! - the project's finish against the maximum of its activities' exponential durations;
//! - three desks with the same mean service time against Pollaczek–Khinchine.
//!
//! The first five files were written by `stochcraft-cli new -t <name>`; the others were built with
//! the same engine commands and saved, with their comparison scenarios (each inheriting the first)
//! added by hand, as in `mm1.bpmn`.
//!
//! Each check allows √2 times the 95% half-width, since several checks read the same runs (as in
//! `crates/blocks/tests/queueing_cases.rs`).
#![allow(clippy::panic)]

use std::f64::consts::SQRT_2;
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
    open_scenario(file, None, experiment)
}

/// As [`open`], reading the file's BPSim `scenario` (the first without one).
fn open_scenario(file: &str, scenario: Option<&str>, experiment: Value) -> Engine {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/models")
        .join(file);
    let mut e = Engine::new();
    let mut params = json!({ "path": path.to_string_lossy() });
    if let Some(s) = scenario {
        params["scenario"] = json!(s);
    }
    let opened = ok(&mut e, "file.open", params);
    assert_eq!(opened["notes"], json!([]), "{file} {scenario:?}: {opened}");
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

/// Erlang B: the chance an arrival finds all `c` servers busy with offered load `a` and no
/// waiting room, for any service-time distribution with that mean (M/G/c/c).
fn erlang_b(c: u32, a: f64) -> f64 {
    let mut b = 1.0;
    for k in 1..=c {
        b = a * b / (f64::from(k) + a * b);
    }
    b
}

/// The mean and second moment of a triangular distribution from `min` to `max` peaking at `mode`.
fn triangular(min: f64, mode: f64, max: f64) -> (f64, f64) {
    let mean = (min + mode + max) / 3.0;
    let variance =
        (min * min + mode * mode + max * max - min * mode - min * max - mode * max) / 18.0;
    (mean, variance + mean * mean)
}

/// Pollaczek–Khinchine: the mean wait in M/G/1 with arrival rate `lambda` and a service time of
/// mean `mean` and second moment `second`.
fn pollaczek_khinchine(lambda: f64, mean: f64, second: f64) -> f64 {
    lambda * second / (2.0 * (1.0 - lambda * mean))
}

/// Borshchev and Grigoryev, The Big Book of Simulation Modeling, ch. 2, Example 2.2: 45
/// customers an hour (0.75 a minute); half use the ATM (triangular 1, 2, 4 minutes), and 30% of
/// those then go to the cashiers; the other half go straight to five cashiers with one queue
/// (triangular 3, 5, 20). The book says "an ATM": the model has one.
/// - By flow balance the cashiers take 0.75 (0.5 + 0.5 × 0.3) = 0.4875 customers a minute, busy
///   0.4875 × 28/3 / 5 = 0.91 (0.758 with a sixth cashier), whatever the queueing.
/// - The ATM is M/G/1 at 0.375 a minute, busy 0.875, with Pollaczek–Khinchine's wait of
///   8.75 minutes and 3.28 waiting.
#[test]
fn the_bank_matches_flow_balance_and_pollaczek_khinchine() {
    let lambda = 0.75;
    let (to_atm, to_cashiers) = (lambda * 0.5, lambda * (0.5 + 0.5 * 0.3));
    let (atm, atm_second) = triangular(1.0, 2.0, 4.0);
    let (cashier, _) = triangular(3.0, 5.0, 20.0);
    let wait = pollaczek_khinchine(to_atm, atm, atm_second);
    assert!((wait - 8.75).abs() < 1e-9, "{wait}");
    let experiment = json!({ "replications": 20, "warmup": 5_000.0, "duration": 100_000.0 });
    let mut e = open("bank.bpmn", experiment.clone());
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "atm", "utilisation", to_atm * atm);
    covers(&r, "atm", "mean_wait", wait);
    covers(&r, "atm", "mean_queue_length", to_atm * wait);
    covers(&r, "cashiers", "utilisation", to_cashiers * cashier / 5.0);
    let mut e = open_scenario("bank.bpmn", Some("six-cashiers"), experiment);
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "cashiers", "utilisation", to_cashiers * cashier / 6.0);
}

/// Hospital beds as a loss system: 4 admissions a day, stays of 5 days on average (log-normal,
/// standard deviation 4), no waiting room: an offered load of 20 beds. Erlang B gives the share
/// turned away, 0.0661 with 24 beds, the same for exponential stays of the same mean (Erlang B
/// depends on the stay only through its mean), and 0.0188 with 28 beds; the beds are busy
/// a (1 − B) / c of the time.
#[test]
fn hospital_beds_match_erlang_b_for_any_length_of_stay() {
    let a = 4.0 * 5.0;
    for (scenario, beds) in [
        (None, 24),
        (Some("exponential-stays"), 24),
        (Some("28-beds"), 28),
    ] {
        let mut e = open_scenario(
            "beds.bpmn",
            scenario,
            json!({ "replications": 20, "warmup": 100.0, "duration": 20_000.0 }),
        );
        let r = ok(&mut e, "run", json!({}));
        let b = erlang_b(beds, a);
        covers(&r, "beds", "loss_probability", b);
        covers(&r, "beds", "utilisation", a * (1.0 - b) / f64::from(beds));
    }
    assert!(erlang_b(28, a) < 0.02 && erlang_b(27, a) > 0.02);
}

/// The machine repair crew, M/M/R//6: six machines each run for an exponential time of mean 4
/// hours, then wait for one of R fitters, who repair in an exponential time of mean 1 hour. The
/// number broken is a birth–death chain on 0..=6, up at (6 − n)/4 and down at min(n, R), as in
/// `crates/blocks/tests/resources.rs` (two fitters: busy 0.57642, 4.61138 machines running).
#[test]
fn the_machine_repair_crew_matches_the_finite_source_chain() {
    let (n, lambda, mu) = (6usize, 0.25, 1.0);
    for (scenario, r) in [
        (Some("one-fitter"), 1usize),
        (None, 2),
        (Some("three-fitters"), 3),
    ] {
        let moves: Vec<Vec<(usize, f64)>> = (0..=n)
            .map(|k| {
                let mut m = Vec::new();
                if k < n {
                    m.push((k + 1, (n - k) as f64 * lambda));
                }
                if k > 0 {
                    m.push((k - 1, k.min(r) as f64 * mu));
                }
                m
            })
            .collect();
        let p = stationary(&moves);
        let broken: f64 = p.iter().enumerate().map(|(k, x)| k as f64 * x).sum();
        let busy: f64 = p.iter().enumerate().map(|(k, x)| k.min(r) as f64 * x).sum();
        if r == 2 {
            assert!((busy / 2.0 - 0.57642).abs() < 1e-5, "{busy}");
            assert!((n as f64 - broken - 4.61138).abs() < 1e-5, "{broken}");
        }
        let mut e = open_scenario(
            "machine-repair.bpmn",
            scenario,
            json!({ "replications": 20, "warmup": 500.0, "duration": 60_000.0 }),
        );
        // A closed loop: no exit wanted, and the machines keep going round, so neither the
        // model's checks nor the behaviour checks warn.
        let checked = ok(&mut e, "model.validate", json!({}));
        assert_eq!(checked["diagnostics"], json!([]), "{scenario:?}: {checked}");
        let res = ok(&mut e, "run", json!({}));
        let behaviour: Vec<&Value> = res["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|w| {
                w["parts"][0]["key"]
                    .as_str()
                    .is_some_and(|k| k.starts_with("behaviour_"))
            })
            .collect();
        assert!(behaviour.is_empty(), "{scenario:?}: {behaviour:?}");
        covers(&res, "repair", "utilisation", busy / r as f64);
        covers(&res, "running", "mean_contents", n as f64 - broken);
        covers(&res, "repair", "mean_queue_length", broken - busy);
    }
}

/// Inspection with rework, the Jackson network of `crates/blocks/tests/routing.rs`: parts arrive
/// at rate 1, inspection takes a mean 0.25 minutes, half the parts fail and go to rework (mean
/// 0.5 minutes) and back to inspection. The traffic equations give inspection 1 / (1 − p) parts a
/// minute and rework p / (1 − p); in product form each is M/M/1, so the waits are ρ / (μ − λ) and
/// the time in the system is the sum of ρ / (1 − ρ) over both. Half reworked: 0.25 and 0.5
/// minutes of waiting, 2 minutes inside; a quarter reworked: 0.125, 0.1 and 0.7.
#[test]
fn inspection_with_rework_matches_the_jackson_network() {
    let (mu_a, mu_b) = (4.0, 2.0);
    for (scenario, p) in [(None, 0.5), (Some("quarter-reworked"), 0.25)] {
        let lambda_a = 1.0 / (1.0 - p);
        let lambda_b = p * lambda_a;
        let (rho_a, rho_b) = (lambda_a / mu_a, lambda_b / mu_b);
        let duration = 50_000.0;
        let mut e = open_scenario(
            "rework.bpmn",
            scenario,
            json!({ "replications": 20, "warmup": 1_000.0, "duration": duration }),
        );
        let r = ok(&mut e, "run", json!({}));
        covers(&r, "inspect", "entered", lambda_a * duration);
        covers(&r, "inspect", "utilisation", rho_a);
        covers(&r, "rework", "utilisation", rho_b);
        covers(&r, "inspect", "mean_wait", rho_a / (mu_a - lambda_a));
        covers(&r, "rework", "mean_wait", rho_b / (mu_b - lambda_b));
        covers(
            &r,
            "good",
            "mean_time_in_system",
            rho_a / (1.0 - rho_a) + rho_b / (1.0 - rho_b),
        );
    }
}

/// The project: three activities in parallel from day 0, each exponential with a mean of 10
/// days, the run stopping when the project finishes. The finish is their maximum, as in
/// `crates/blocks/tests/terminating.rs`: P(T ≤ t) = (1 − e^(−t/10))³, mean 10 (1 + 1/2 + 1/3)
/// = 18.33 days; finished by day 30 with chance 0.858.
#[test]
fn the_project_finishes_at_the_maximum_of_its_activities() {
    let n = 2_000;
    let mut e = open("project.bpmn", json!({ "replications": n }));
    let r = ok(&mut e, "run", json!({}));
    let stop = &r["stop"];
    assert_eq!(stop["met"], json!(n), "{stop}");
    let (Some(mean), Some(hw)) = (
        stop["time"]["mean"].as_f64(),
        stop["time"]["half_width"].as_f64(),
    ) else {
        panic!("no finish time: {stop}")
    };
    let theory = 10.0 * (1.0 + 1.0 / 2.0 + 1.0 / 3.0);
    eprintln!("finish: {mean} ± {hw}, theory {theory}");
    assert!(
        (mean - theory).abs() <= SQRT_2 * hw,
        "{mean} ± {hw}, theory {theory}"
    );
    let times: Vec<f64> = stop["times"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    assert_eq!(times.len(), n, "{stop}");
    let by_30 = times.iter().filter(|t| **t <= 30.0).count() as f64 / n as f64;
    let q = 1.0 - stochcraft_random::math::exp(-3.0);
    let chance = q * q * q;
    let hw = 1.96 * (chance * (1.0 - chance) / n as f64).sqrt();
    eprintln!("finished by day 30: {by_30}, theory {chance} ± {hw}");
    assert!(
        (by_30 - chance).abs() <= SQRT_2 * hw,
        "{by_30} vs {chance} ± {hw}"
    );
}

/// Three desks, each M/G/1 at ρ = 0.8 with a mean service time of 1 minute, fed separately: a
/// fixed time (second moment 1), an exponential one (2) and a gamma of shape 0.25 (standard
/// deviation 2, second moment 5). Pollaczek–Khinchine's waits are λE[S²] / (2(1 − ρ)): 2, 4 and
/// 10 minutes.
#[test]
fn the_same_mean_with_more_spread_waits_longer_by_pollaczek_khinchine() {
    let lambda = 0.8;
    let mut e = open(
        "spread.bpmn",
        json!({ "replications": 20, "warmup": 2_000.0, "duration": 100_000.0 }),
    );
    let r = ok(&mut e, "run", json!({}));
    for (desk, second) in [
        ("fixed", 1.0),
        ("random", 2.0),
        ("uneven", 1.0 + 0.25 * 4.0 * 4.0),
    ] {
        covers(&r, desk, "utilisation", lambda);
        covers(
            &r,
            desk,
            "mean_wait",
            pollaczek_khinchine(lambda, 1.0, second),
        );
    }
}
