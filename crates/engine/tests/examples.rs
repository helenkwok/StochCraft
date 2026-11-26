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
//! - three desks with the same mean service time against Pollaczek–Khinchine;
//! - the project budget, a Monte Carlo model, against the exact mean and spread of a sum of
//!   independent triangular durations and the exact mean of its cost.
//! - the polling place against Erlang C, with its machines allocated by Allen's (2011) rule;
//! - the mass vaccination hub of Hanly et al. (2022) against flow balance at every station;
//! - the prison population of Brailsford, Churilov and Dangerfield (2014, ch. 8) against the
//!   mean numbers inside its network of infinite-server stations;
//! - Penny Fab (Hopp and Spearman, Factory Physics) against the practical worst case, exact for a
//!   balanced closed line with exponential times, and the best case for fixed times.
//! - repairmen with spares (Crane and Iglehart, WSC 1973) against the birth–death chain of the
//!   number of machines broken, whose stationary distribution gives the paper's values.
//! - arrivals at the gaps between zeros of the Riemann zeta function against exact G/M/1 theory,
//!   beside Poisson arrivals at the same rate (M/M/1).
//! - Able and Baker, two servers that differ in speed behind one queue (a service time by
//!   resource), against the exact long-run answer of their discrete-time Markov chain.
//! - delayed discharge (Boyle et al., OR Society Simulation Workshop 2023) against Erlang B for
//!   its two discharge-to-assess services, whose overflow waits in hospital.
//!
//! The first five files were written by `stochcraft-cli new -t <name>`; the others were built with
//! the same engine commands and saved, with their comparison scenarios (each inheriting the first)
//! added by hand, as in `mm1.bpmn`. The starting contents of the prison and of Penny Fab were
//! added by hand too, and the repairmen-with-spares file was written by hand in the same form. The
//! zeta queue was assembled from the output of `tools/zeta-gaps/generate.py`, and the delayed
//! discharge file was written by a short script in the same form (its stays have up to 197 points).
//!
//! Each check allows √2 times the 95% half-width, since several checks read the same runs (as in
//! `crates/blocks/tests/queueing_cases.rs`).
#![allow(clippy::panic)]

use std::f64::consts::SQRT_2;
use std::path::Path;

use serde_json::{Value, json};
use stochcraft_engine::Engine;
use stochcraft_model::{ElementId, ElementKind, Model};
use stochcraft_random::Dist;

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

/// The engine with example `file` of `tests/models` open, with `experiment` set over its own.
fn open(file: &str, experiment: Value) -> Engine {
    open_scenario(file, None, experiment)
}

/// The path of example `file` in `tests/models`.
fn model_path(file: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/models")
        .join(file)
}

/// The model of example `file` with its BPSim `scenario` (the first without one), as read from
/// the file.
fn model_of(file: &str, scenario: Option<&str>) -> Model {
    let path = model_path(file);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    stochcraft_io::read_scenario(&bytes, scenario)
        .unwrap_or_else(|e| panic!("{file} {scenario:?}: {e}"))
        .model
}

/// The mean service time and the units of server `id` in model `m`.
fn server(m: &Model, id: &str) -> (f64, u32) {
    match m.element(&ElementId::unchecked(id)).map(|e| &e.kind) {
        Some(ElementKind::Server {
            capacity, service, ..
        }) => (
            service
                .mean()
                .unwrap_or_else(|| panic!("{id}: no mean service time")),
            *capacity,
        ),
        _ => panic!("{id} is not a server"),
    }
}

/// The mean time between arrivals at source `id` in model `m`.
fn interarrival(m: &Model, id: &str) -> f64 {
    match m.element(&ElementId::unchecked(id)).map(|e| &e.kind) {
        Some(ElementKind::Source { interarrival, .. }) => interarrival
            .mean()
            .unwrap_or_else(|| panic!("{id}: no mean time between arrivals")),
        _ => panic!("{id} is not a source"),
    }
}

/// The warnings of the behaviour checks in run results `r`.
fn behaviour_warnings(r: &Value) -> Vec<&Value> {
    r["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|w| {
            w["parts"][0]["key"]
                .as_str()
                .is_some_and(|k| k.starts_with("behaviour_"))
        })
        .collect()
}

/// As [`open`], reading the file's BPSim `scenario` (the first without one).
fn open_scenario(file: &str, scenario: Option<&str>, experiment: Value) -> Engine {
    let path = model_path(file);
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

/// The project budget, a Monte Carlo model: the duration is the sum of two independent
/// triangular durations, so its mean (8 + 10 + 16)/3 + (15 + 20 + 32)/3 = 101/3 days and its
/// variance, each triangular's (a² + b² + c² − ab − ac − bc)/18, are exact; the cost is duration
/// × day rate + materials with the rate independent of the durations (it is correlated with the
/// materials only), so its mean is exact too: 101/3 × 900 + 12,000 = 42,300 USD.
#[test]
fn the_project_budget_matches_its_exact_means() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/budget.bpmn");
    let mut e = Engine::new();
    let opened = ok(
        &mut e,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    assert_eq!(opened["notes"], json!([]), "{opened}");
    assert_eq!(opened["kind"], "monte_carlo", "{opened}");
    assert_eq!(opened["diagnostics"], json!([]), "{opened}");
    let r = ok(&mut e, "mc.run", json!({}));
    assert_eq!(r["trials"], 10_000);
    let output = |name: &str| {
        r["outputs"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|o| o["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no output {name}: {r}"))
    };
    let variance = |a: f64, c: f64, b: f64| (a * a + b * b + c * c - a * b - a * c - b * c) / 18.0;
    for (name, theory) in [
        ("duration", 101.0 / 3.0),
        ("cost", 101.0 / 3.0 * 900.0 + 12_000.0),
    ] {
        let o = output(name);
        let (Some(mean), Some(hw)) = (o["mean"].as_f64(), o["half_width"].as_f64()) else {
            panic!("no mean: {o}")
        };
        eprintln!("{name}: {mean} ± {hw}, theory {theory}");
        assert!(
            (mean - theory).abs() <= SQRT_2 * hw,
            "{name}: {mean} ± {hw}, theory {theory}"
        );
        // A target, with its chance and interval.
        assert!(o["target"]["probability"].as_f64().is_some(), "{o}");
    }
    // The duration's spread: the sample standard deviation of 10,000 independent trials is
    // within about 2% of the true one (its relative standard error is about 1/√(2n) = 0.7%).
    let sd = output("duration")["std_dev"].as_f64().unwrap_or(f64::NAN);
    let theory = (variance(8.0, 10.0, 16.0) + variance(15.0, 20.0, 32.0)).sqrt();
    assert!(
        (sd - theory).abs() <= 0.02 * theory,
        "standard deviation {sd}, theory {theory}"
    );
    // The correlation asked for is the one the trials have, near enough.
    let achieved = r["correlations"][0]["achieved"]
        .as_f64()
        .unwrap_or(f64::NAN);
    assert!((achieved - 0.4).abs() < 0.05, "{achieved}");
}

/// Which inputs of the project budget matter: the tornado moves one input from its 10th to its
/// 90th percentile with the others at their medians, so its numbers are those of the inputs'
/// exact quantiles (a triangular's inverse distribution function, the uniform's and the
/// normal's z of 0.9, 1.2815515655446004). Building takes longer and is less certain than
/// designing, so it moves the duration more; the cost, which also has a day rate and materials,
/// is ordered by the same exact swings.
#[test]
fn the_project_budget_says_which_inputs_matter() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/budget.bpmn");
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    let r = ok(&mut e, "mc.run", json!({}));
    let influence = |name: &str| {
        r["outputs"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|o| o["name"] == name)
            .map(|o| o["influence"].clone())
            .unwrap_or_else(|| panic!("no output {name}: {r}"))
    };
    let triangular = |min: f64, mode: f64, max: f64, p: f64| {
        if p < (mode - min) / (max - min) {
            min + (p * (max - min) * (mode - min)).sqrt()
        } else {
            max - ((1.0 - p) * (max - min) * (max - mode)).sqrt()
        }
    };
    let design = |p: f64| triangular(8.0, 10.0, 16.0, p);
    let build = |p: f64| triangular(15.0, 20.0, 32.0, p);
    let near = |v: &Value, want: f64| {
        let got = v.as_f64().unwrap_or(f64::NAN);
        assert!(
            (got - want).abs() < 1e-6 * want.abs().max(1.0),
            "{got} not {want}"
        );
    };

    let d = influence("duration");
    near(&d["base"], design(0.5) + build(0.5));
    let inputs = d["inputs"].as_array().cloned().unwrap_or_default();
    let names: Vec<&str> = inputs.iter().filter_map(|i| i["input"].as_str()).collect();
    // Duration does not use the day rate or the materials: they come last, with no swing.
    assert_eq!(names.get(..2), Some(&["build", "design"][..]), "{d}");
    let mut rest = names.get(2..).unwrap_or_default().to_vec();
    rest.sort_unstable();
    assert_eq!(rest, ["day_rate", "materials"], "{d}");
    for i in inputs.get(2..).unwrap_or_default() {
        near(&i["swing"], 0.0);
    }
    let [b, des, ..] = inputs.as_slice() else {
        panic!("two inputs: {d}")
    };
    near(&b["low"], design(0.5) + build(0.1));
    near(&b["high"], design(0.5) + build(0.9));
    near(&b["swing"], build(0.9) - build(0.1));
    near(&des["low"], design(0.1) + build(0.5));
    near(&des["high"], design(0.9) + build(0.5));
    near(&des["swing"], design(0.9) - design(0.1));
    assert!(b["swing"].as_f64() > des["swing"].as_f64());
    // The rank correlations, from the trials, rank them alike.
    assert!(b["rank_correlation"].as_f64() > des["rank_correlation"].as_f64());
    assert!(des["rank_correlation"].as_f64().is_some_and(|x| x > 0.3));

    // The cost is duration × day rate + materials; every input at its median is the base.
    let c = influence("cost");
    let days = design(0.5) + build(0.5);
    near(&c["base"], days * 900.0 + 12_000.0);
    let z90 = 1.281_551_565_544_600_4;
    let mut want = [
        ("build", (build(0.9) - build(0.1)) * 900.0),
        ("design", (design(0.9) - design(0.1)) * 900.0),
        ("day_rate", days * 160.0),
        ("materials", 2.0 * z90 * 1_500.0),
    ];
    want.sort_by(|a, b| b.1.total_cmp(&a.1));
    let got: Vec<(&str, f64)> = c["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| {
            (
                i["input"].as_str().unwrap_or(""),
                i["swing"].as_f64().unwrap_or(f64::NAN),
            )
        })
        .collect();
    assert_eq!(
        got.iter().map(|g| g.0).collect::<Vec<_>>(),
        want.iter().map(|w| w.0).collect::<Vec<_>>(),
        "{c}"
    );
    for (g, w) in got.iter().zip(&want) {
        assert!((g.1 - w.1).abs() < 1e-6 * w.1, "{g:?} not {w:?}");
    }
}

/// A polling place from T. T. Allen, *Introduction to Discrete Event Simulation and Agent-based
/// Modeling*, Springer 2011: ballots of 19 initiatives take 12.5 minutes on average and of 8
/// initiatives 6.5 (§7.3.3), over a 13-hour day (§7.3.4). The arrival rate is deduced, not
/// printed: about half a million voters at 532 locations (§7.3.1) is about 72 an hour. The
/// voting times are exponential, as Erlang C and the Kolesar–Green rule of §6.2 assume (the
/// study itself used truncated normal ones), so each scenario is M/M/c: Wq = C(c, a) S / (c − a).
/// The machines follow the book's utilisation-based rule (§7.3.5): roundup(ballot minutes ×
/// voters / (ρ₀ × 13 h × 60)) with its starting ρ₀ of 0.85 gives 18 for the long ballot and 10
/// for the short; 16 machines is the comparison.
#[test]
fn the_polling_place_matches_erlang_c() {
    let lambda = 72.0 / 60.0;
    let voters = 72.0 * 13.0;
    let allocate = |ballot: f64| (ballot * voters / (0.85 * 13.0 * 60.0)).ceil();
    assert!(
        (allocate(12.5) - 18.0).abs() < 1e-9 && (allocate(6.5) - 10.0).abs() < 1e-9,
        "{} {}",
        allocate(12.5),
        allocate(6.5)
    );
    let experiment = json!({ "replications": 20, "warmup": 1_000.0, "duration": 50_000.0 });
    for (scenario, ballot, c) in [
        (None, 12.5, 18),
        (Some("16-machines"), 12.5, 16),
        (Some("short-ballot"), 6.5, 10),
    ] {
        let a = lambda * ballot;
        let wait = erlang_c(c, a) * ballot / (f64::from(c) - a);
        let mut e = open_scenario("polling.bpmn", scenario, experiment.clone());
        let r = ok(&mut e, "run", json!({}));
        covers(&r, "machines", "utilisation", a / f64::from(c));
        covers(&r, "machines", "mean_wait", wait);
        covers(&r, "machines", "mean_queue_length", lambda * wait);
    }
}

/// The polling place's election day: 936 voters (72 an hour for 13 hours), the run stopping when
/// the last has voted. There is no exact answer here, only the direction: allocating by voters
/// alone (14 machines, half of 28 shared by two places of the same size) leaves the long-ballot
/// place's voters waiting far longer, and its last voter later, than the 18 the rule gives.
#[test]
fn the_polling_place_on_election_day_shows_the_direction() {
    let day = |scenario: &str| {
        let mut e = open_scenario(
            "polling.bpmn",
            Some(scenario),
            json!({ "replications": 200 }),
        );
        let r = ok(&mut e, "run", json!({}));
        let stop = &r["stop"];
        assert_eq!(stop["met"], json!(200), "{scenario}: {stop}");
        let Some(end) = stop["time"]["mean"].as_f64() else {
            panic!("{scenario}: no end time: {stop}")
        };
        (end, result(&r, "machines", "mean_wait").0)
    };
    let (end_18, wait_18) = day("election-day");
    let (end_14, wait_14) = day("election-day-14-machines");
    eprintln!(
        "18 machines: done at {end_18}, wait {wait_18}; 14: done at {end_14}, wait {wait_14}"
    );
    assert!(end_14 > end_18, "{end_14} vs {end_18}");
    assert!(wait_14 > 5.0 * wait_18, "{wait_14} vs {wait_18}");
}

/// The mass vaccination hub of Hanly, Churches, Fitzgerald, Caterson, MacIntyre and Jorm,
/// "Modelling vaccination capacity at mass vaccination hubs and general practice clinics: a
/// simulation study", BMC Health Services Research 22, 1059 (2022): Table 1's times (a minimum
/// plus an exponential, written as tables of their quantiles), Table 3's staff (low: 2, 4, 6, 4 and 5,
/// with 25 observation seats; high: three times as many), Table 2's 60 or 180 bookings an hour
/// and 2% who do not come. The paper's arrival noise and walking times are not published in it,
/// so arrivals here are Poisson and its times in the hub are not checked. What holds whatever
/// the queueing is flow balance: each station is busy λ E[S] / c of the time, from the means of
/// the times as the file gives them, which are within 0.2% of Table 1's; and the hub lets out
/// λ patients a unit of time, 470.4 in an 8-hour day for the small hub (the paper: about 500)
/// and 1,411.2 for the large (about 1,400).
#[test]
fn the_vaccination_hub_matches_flow_balance() {
    let stations = [
        "preparation",
        "entrance",
        "registration",
        "assessment",
        "vaccination",
        "observation",
    ];
    let table_1 = [
        1.0 + 1.0 / 3.0,
        2.0 + 1.0,
        3.0 + 1.0 / 0.7,
        2.0 + 1.0,
        3.0 + 1.0,
        0.98 * 20.0 + 0.02 * (20.0 + 10.0),
    ];
    let base = model_of("vaccination-hub.bpmn", None);
    for (id, paper) in stations.iter().zip(table_1) {
        let (mean, _) = server(&base, id);
        assert!(
            (mean - paper).abs() < 2e-3 * paper,
            "{id}: {mean} vs {paper}"
        );
    }
    // The large hub has three times the arrivals, so a shorter run.
    for (scenario, checked, per_day, duration) in [
        (None, &stations[..], 470.4, 20_000.0),
        (Some("large-hub"), &stations[..], 1_411.2, 6_000.0),
        (Some("ten-more"), &stations[4..], 548.8, 20_000.0),
    ] {
        let m = model_of("vaccination-hub.bpmn", scenario);
        let gap = interarrival(&m, "patients");
        assert!(
            (8.0 * 60.0 / gap - per_day).abs() < 1e-6,
            "{scenario:?}: {}",
            8.0 * 60.0 / gap
        );
        let mut e = open_scenario(
            "vaccination-hub.bpmn",
            scenario,
            json!({ "replications": 20, "warmup": 600.0, "duration": duration }),
        );
        let r = ok(&mut e, "run", json!({}));
        for id in checked {
            let (mean, units) = server(&m, id);
            covers(&r, id, "utilisation", mean / gap / f64::from(units));
        }
        covers(&r, "leave", "entered", duration / gap);
    }
}

/// The prison population of Brailsford, Churilov and Dangerfield, *Discrete-Event Simulation and
/// System Dynamics for Management Decision Making*, Wiley 2014, ch. 8 §8.3.1: 3,000 petty and
/// 650 serious first offenders a year, sentences of 5 and 20 years on average, and those who
/// reoffend back after 2 years on average; 50,000 petty and 26,000 serious offenders inside. The
/// share who reoffend is not given; if the stated numbers are the equilibrium, it is 0.70 for
/// petty and 0.50 for serious offenders (a deduction). Everything scaled down 100 times: 30 and
/// 6.5 a year. Each stay is a station without a queue, so the number inside is Poisson with mean
/// (arrivals including returns) × mean stay, whatever the stays' distribution: 500 petty and 260
/// serious offenders inside, 140 and 13 out before returning. The stays are exponential and the
/// run starts with those numbers inside, so the mean stays there from time 0 and no warm-up is
/// needed. With 60% of petty offenders reoffending, 375 and 90, reached after a warm-up.
#[test]
fn the_prison_population_matches_its_infinite_server_network() {
    let (petty, serious) = (30.0, 6.5);
    for (scenario, reoffend, warmup) in [(None, 0.7, 0.0), (Some("fewer-reoffend"), 0.6, 100.0)] {
        let mut e = open_scenario(
            "prison.bpmn",
            scenario,
            json!({ "replications": 20, "warmup": warmup, "duration": 200.0 }),
        );
        let r = ok(&mut e, "run", json!({}));
        let into_petty = petty / (1.0 - reoffend);
        let into_serious = serious / (1.0 - 0.5);
        covers(&r, "petty_prison", "mean_contents", into_petty * 5.0);
        covers(
            &r,
            "petty_outside",
            "mean_contents",
            into_petty * reoffend * 2.0,
        );
        // The serious offenders are the same in both scenarios: checked once.
        if scenario.is_none() {
            covers(&r, "serious_prison", "mean_contents", into_serious * 20.0);
            covers(
                &r,
                "serious_outside",
                "mean_contents",
                into_serious * 0.5 * 2.0,
            );
        }
    }
    // At full scale the deduced shares give the stated numbers inside.
    let inside = |first: f64, reoffend: f64, sentence: f64| first / (1.0 - reoffend) * sentence;
    assert!((inside(3_000.0, 0.7, 5.0) - 50_000.0).abs() < 1e-6);
    assert!((inside(650.0, 0.5, 20.0) - 26_000.0).abs() < 1e-6);
}

/// Penny Fab, from W. J. Hopp and M. L. Spearman, *Factory Physics*: four stations in a line, one
/// machine each, 2 hours a job, so the bottleneck rate is r_b = 0.5 a hour, the raw process time
/// T₀ = 8 hours and the critical work in process W₀ = r_b T₀ = 4. The w jobs start at the first
/// station and go round for ever (starting contents and a closed loop). With exponential times
/// the line is the practical worst case, whose throughput r_b w / (W₀ + w − 1) is exact for a
/// balanced closed line of exponential stations; each station is busy that × 2 hours, the cycle
/// time is T₀ + (w − 1) / r_b (Little's law), and by symmetry each station's wait is a quarter of
/// the cycle time beyond T₀. With fixed times the line is the best case: at w = W₀ each station
/// is always busy and 0.5 jobs an hour come out.
#[test]
fn penny_fab_matches_the_practical_worst_case_and_the_best_case() {
    let (rb, t0, w0): (f64, f64, f64) = (0.5, 8.0, 4.0);
    let duration = 50_000.0;
    for (scenario, w) in [
        (Some("1-job"), 1.0),
        (None, 4.0),
        (Some("8-jobs"), 8.0),
        (Some("16-jobs"), 16.0),
    ] {
        let mut e = open_scenario(
            "penny-fab.bpmn",
            scenario,
            json!({ "replications": 20, "warmup": 500.0, "duration": duration }),
        );
        let checked = ok(&mut e, "model.validate", json!({}));
        assert_eq!(checked["diagnostics"], json!([]), "{scenario:?}: {checked}");
        let r = ok(&mut e, "run", json!({}));
        let behaviour = behaviour_warnings(&r);
        assert!(behaviour.is_empty(), "{scenario:?}: {behaviour:?}");
        let throughput = rb * w / (w0 + w - 1.0);
        let cycle = t0 + (w - 1.0) / rb;
        assert!((w / throughput - cycle).abs() < 1e-9, "{w}");
        covers(&r, "deburr", "utilisation", throughput * 2.0);
        covers(&r, "rim", "mean_wait", (cycle - t0) / 4.0);
        covers(&r, "deburr", "entered", throughput * duration);
    }
    let duration = 10_000.0;
    let mut e = open_scenario(
        "penny-fab.bpmn",
        Some("fixed-times"),
        json!({ "replications": 4, "warmup": 500.0, "duration": duration }),
    );
    let r = ok(&mut e, "run", json!({}));
    for id in ["head", "tail", "rim", "deburr"] {
        let (busy, _) = result(&r, id, "utilisation");
        assert!((busy - 1.0).abs() < 1e-6, "{id}: {busy}");
    }
    let (out, _) = result(&r, "deburr", "entered");
    assert!((out - rb * duration).abs() <= 2.0, "{out}");
}

/// Repairmen with spares, the example of M. A. Crane and D. L. Iglehart, *A New Approach to
/// Simulating Stable Stochastic Systems*, Proceedings of the 1973 Winter Simulation Conference
/// (the model on p. 269, its parameters and Table 1 on p. 270): N = 10 machines must run, M = 4
/// spares wait on cold standby (they cannot fail while they wait) and S = 3 repairmen repair the
/// broken ones; a running machine fails at rate λ = 1 and a repair takes an exponential time at
/// rate μ = 4. The number broken X is a birth–death chain on 0..=N + M, up at
/// λ min(N, N + M − X) and down at μ min(X, S). Its stationary distribution gives the paper's
/// theoretical values (Table 1): E(X) 3.471, E(X²) 17.278, P(X > M) .306, P(X > S) .438,
/// E[(S − X)⁺] .678, P(X > 0) .939 and P(X = 0) .061.
///
/// The run is checked on what the engine reports, each from the same chain: the machines broken
/// (the repair task's work in progress, waiting or being repaired), the repairmen's utilisation,
/// the machines waiting for a repairman and, by Little's law, their wait; the machines running,
/// the spares waiting and the repairs done (machines running × λ, the failures). The paper's
/// probabilities are shares of time, which no result reports, so they are checked on the chain
/// only. The scenarios (two or four repairmen, no spares or eight) are checked on the machines
/// broken and running.
#[test]
fn repairmen_with_spares_match_their_birth_death_chain() {
    let (n, lambda, mu): (usize, f64, f64) = (10, 1.0, 4.0);
    let duration = 10_000.0;
    for (scenario, spares, repairmen) in [
        (None, 4usize, 3usize),
        (Some("two-repairmen"), 4, 2),
        (Some("four-repairmen"), 4, 4),
        (Some("no-spares"), 0, 3),
        (Some("eight-spares"), 8, 3),
    ] {
        let m = model_of("repairmen-spares.bpmn", scenario);
        let (to_fail, machines) = server(&m, "running");
        assert!(
            (to_fail - 1.0 / lambda).abs() < 1e-12,
            "{scenario:?}: {to_fail}"
        );
        assert_eq!(machines as usize, n, "{scenario:?}");
        let (to_repair, crew) = server(&m, "repair");
        assert!(
            (to_repair - 1.0 / mu).abs() < 1e-12,
            "{scenario:?}: {to_repair}"
        );
        assert_eq!(crew as usize, repairmen, "{scenario:?}");
        let k = n + spares;
        let start = m.starting.get(&ElementId::unchecked("running")).copied();
        assert_eq!(start.map(|s| s as usize), Some(k), "{scenario:?}");

        let moves: Vec<Vec<(usize, f64)>> = (0..=k)
            .map(|x| {
                let mut out = Vec::new();
                if x < k {
                    out.push((x + 1, n.min(k - x) as f64 * lambda));
                }
                if x > 0 {
                    out.push((x - 1, x.min(repairmen) as f64 * mu));
                }
                out
            })
            .collect();
        let p = stationary(&moves);
        let mean =
            |f: &dyn Fn(usize) -> f64| -> f64 { p.iter().enumerate().map(|(x, q)| f(x) * q).sum() };
        let share = |f: &dyn Fn(usize) -> bool| mean(&|x: usize| f64::from(u8::from(f(x))));
        let broken = mean(&|x: usize| x as f64);
        let busy = mean(&|x: usize| x.min(repairmen) as f64);
        let waiting = mean(&|x: usize| x.saturating_sub(repairmen) as f64);
        let running = mean(&|x: usize| n.min(k - x) as f64);
        let spare = mean(&|x: usize| (k - x).saturating_sub(n) as f64);
        if scenario.is_none() {
            // The paper's theoretical values, printed to three decimals.
            for (what, chain, paper) in [
                ("E(X)", broken, 3.471),
                ("E(X²)", mean(&|x: usize| (x * x) as f64), 17.278),
                ("P(X > M)", share(&|x: usize| x > spares), 0.306),
                ("P(X > S)", share(&|x: usize| x > repairmen), 0.438),
                ("E[(S − X)⁺]", repairmen as f64 - busy, 0.678),
                ("P(X > 0)", share(&|x: usize| x > 0), 0.939),
                ("P(X = 0)", share(&|x: usize| x == 0), 0.061),
            ] {
                assert!(
                    (chain - paper).abs() < 5e-4,
                    "{what}: {chain} against {paper}"
                );
            }
            assert!((broken - 3.470_797).abs() < 1e-6, "{broken}");
            assert!((running - 9.289_369).abs() < 1e-6, "{running}");
        }
        // Repairs keep up with failures.
        assert!(
            (busy * mu - running * lambda).abs() < 1e-9,
            "{scenario:?}: {busy} {running}"
        );

        let mut e = open_scenario(
            "repairmen-spares.bpmn",
            scenario,
            json!({ "replications": 20, "warmup": 500.0, "duration": duration }),
        );
        // A closed loop: no exit wanted, so neither the model's checks nor the behaviour checks
        // warn.
        let checked = ok(&mut e, "model.validate", json!({}));
        assert_eq!(checked["diagnostics"], json!([]), "{scenario:?}: {checked}");
        let r = ok(&mut e, "run", json!({}));
        let behaviour = behaviour_warnings(&r);
        assert!(behaviour.is_empty(), "{scenario:?}: {behaviour:?}");
        covers(&r, "repair", "mean_work_in_progress", broken);
        covers(&r, "running", "mean_contents", running);
        // The paper's own case checks every result; the scenarios only the two above, so that
        // with fewer checks a miss by chance stays rare.
        if scenario.is_some() {
            continue;
        }
        covers(&r, "repair", "utilisation", busy / repairmen as f64);
        covers(&r, "repair", "mean_queue_length", waiting);
        covers(&r, "repair", "mean_wait", waiting / (running * lambda));
        covers(&r, "running", "utilisation", running / n as f64);
        covers(&r, "running", "mean_queue_length", spare);
        covers(&r, "repair", "entered", running * lambda * duration);
    }
}

/// The root σ in (0, 1) of σ = A*(μ(1 − σ)) for a G/M/1 queue whose times between arrivals take
/// the values `points` (probability, value) with service rate `mu`; A* is the Laplace–Stieltjes
/// transform of the time between arrivals, here the finite sum Σ pᵢ·exp(−s·tᵢ). Then an
/// arrival waits with chance σ and, on average, σ / (μ(1 − σ)) in the queue (Gross and Harris, Fundamentals of
/// Queueing Theory, on the G/M/1 queue; Kleinrock, Queueing Systems, vol. 1, on G/M/m).
fn gm1_sigma(points: &[(f64, f64)], mu: f64) -> f64 {
    let total: f64 = points.iter().map(|(p, _)| p).sum();
    let gap = |sigma: f64| {
        points
            .iter()
            .map(|(p, t)| p / total * stochcraft_random::math::exp(-mu * (1.0 - sigma) * t))
            .sum::<f64>()
            - sigma
    };
    // σ = 1 is always a root; with the utilisation below 1 the other one lies below it, where
    // the gap is positive at 0 and negative just under 1.
    let (mut lo, mut hi) = (0.0, 1.0 - 1e-6);
    assert!(gap(lo) > 0.0 && gap(hi) < 0.0, "no root in (0, 1)");
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if gap(mid) > 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// A server of exponential service with mean 1 minute, fed at 0.8 a minute on average (ρ = 0.8).
/// With Poisson arrivals the mean wait is 4 minutes (M/M/1). With the gaps between the zeros of
/// the Riemann zeta function, which repel one another (Wigner–Dyson spacing, few short gaps),
/// the same rate gives a G/M/1 queue with σ = 0.672 and a mean wait of about 2.05 minutes, read
/// here from the distribution in the model file.
#[test]
fn arrivals_that_keep_their_distance_match_g_m_1() {
    let experiment = json!({ "replications": 20, "duration": 50_000.0 });
    let m = model_of("zeta-queue.bpmn", Some("zeta"));
    let points = match m
        .element(&ElementId::unchecked("arrivals"))
        .map(|e| &e.kind)
    {
        Some(ElementKind::Source {
            interarrival: Dist::User { points, .. },
            ..
        }) => points.clone(),
        _ => panic!("the arrivals are not a user distribution"),
    };
    let (service, capacity) = server(&m, "server");
    assert_eq!(capacity, 1);
    let mu = 1.0 / service;
    let gap = interarrival(&m, "arrivals");
    assert!((gap - 1.25).abs() < 1e-6, "{gap}");
    let lambda = 1.0 / gap;
    let rho = lambda / mu;
    assert!((rho - 0.8).abs() < 1e-6, "{rho}");

    let sigma = gm1_sigma(&points, mu);
    let wait = sigma / (mu * (1.0 - sigma));
    // The gaps spread less than exponential ones (variance 0.15 of the mean squared, not 1), so
    // fewer arrivals find the server busy and they wait about half as long.
    assert!(sigma < rho && (sigma - 0.672).abs() < 1e-3, "{sigma}");
    assert!((wait - 2.05).abs() < 0.01, "{wait}");
    let mut e = open_scenario("zeta-queue.bpmn", Some("zeta"), experiment.clone());
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "server", "mean_wait", wait);
    covers(&r, "server", "mean_queue_length", lambda * wait);
    covers(&r, "server", "utilisation", rho);

    // The same mean gap with exponential gaps: σ = ρ, and Wq = ρ / (μ − λ) = 4 minutes.
    let m = model_of("zeta-queue.bpmn", Some("poisson"));
    let poisson_gap = interarrival(&m, "arrivals");
    assert!((poisson_gap - gap).abs() < 1e-6, "{poisson_gap}");
    let mut e = open_scenario("zeta-queue.bpmn", Some("poisson"), experiment);
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "server", "mean_wait", rho / (mu - lambda));
    covers(&r, "server", "utilisation", rho);
}

/// Able and Baker (Banks, Carson, Nelson and Nicol, Discrete-Event System Simulation, Example
/// 2.2): customers come 1 to 4 minutes apart (probabilities 0.25, 0.40, 0.20, 0.15) and queue
/// for two servers who differ in speed, one task that needs either of two pools. A customer
/// who finds both free goes to Able, who serves in 2 to 5 minutes (0.30, 0.28, 0.25, 0.17);
/// Baker serves in 3 to 6 (0.35, 0.25, 0.20, 0.20).
///
/// Every time is a whole number of minutes, so the system is a discrete-time Markov chain,
/// solved exactly by `tools/able-baker/chain.py` (the same to six digits with the queue cut at
/// 30 and at 50). At an instant, finished services free their servers, an arrival joins the
/// queue, then the queue takes the free servers, Able before Baker: the engine hands units that
/// come free together to the waiting entities, each trying its alternatives in the order of its
/// task's needs, so this holds whichever job happened to end first. Its answer:
///
/// - Able busy 0.885383 and Baker busy 0.745157 of the time. This split is the discriminating
///   check: it needs each server's own service time, where one time for both would leave them
///   about equally busy;
/// - a mean queue of 0.383956 and a mean wait of 0.863902 minutes, so that 39.13% of the
///   customers wait 2.2078 minutes each.
///
/// Cross-check, without the chain: the arrival rate is each server's busy share over its mean
/// service time, summed: 0.885383 / 3.29 + 0.745157 / 4.25 = 1 / 2.25.
#[test]
fn able_and_baker_match_their_markov_chain() {
    let m = model_of("able-baker.bpmn", None);
    let gap = interarrival(&m, "arrivals");
    assert!((gap - 2.25).abs() < 1e-9, "{gap}");
    let lambda = 1.0 / gap;
    let by = match m.element(&ElementId::unchecked("service")).map(|e| &e.kind) {
        Some(kind) => kind.processing_by_resource(),
        None => None,
    };
    let times: Vec<(String, f64)> = by
        .map(|b| {
            b.by_resource
                .iter()
                .map(|r| (r.resource.to_string(), r.time.mean().unwrap_or(f64::NAN)))
                .collect()
        })
        .unwrap_or_default();
    let (able, baker) = (0.885_383, 0.745_157);
    assert_eq!(times.len(), 2, "{times:?}");
    assert_eq!(times[0].0, "able");
    assert!((times[0].1 - 3.29).abs() < 1e-9, "{times:?}");
    assert_eq!(times[1].0, "baker");
    assert!((times[1].1 - 4.25).abs() < 1e-9, "{times:?}");
    assert!(
        (able / times[0].1 + baker / times[1].1 - lambda).abs() < 1e-5,
        "the chain's busy shares do not carry the arrival rate"
    );

    let experiment = json!({ "replications": 20, "duration": 50_000.0 });
    let mut e = open("able-baker.bpmn", experiment);
    let r = ok(&mut e, "run", json!({}));
    covers(&r, "able", "utilisation", able);
    covers(&r, "baker", "utilisation", baker);
    covers(&r, "service", "mean_queue_length", 0.383_956);
    covers(&r, "service", "mean_wait", 0.863_902);
    // Each customer is served once, by one of the two.
    let served = |id: &str| result(&r, id, "entered").0;
    let duration = 50_000.0;
    assert!(
        (served("able") + served("baker") - lambda * duration).abs() < 0.01 * lambda * duration,
        "{} + {}",
        served("able"),
        served("baker")
    );
}

/// The exact mean numbers waiting in hospital in the delayed-discharge model `m`, for a nursing
/// home and for care at home, each λ B(c, λT) T, as read from the file: the arrival rate λ is the
/// patients' rate times the gateway's share, c the discharge-to-assess (D2A) places and T the
/// mean stay of the distribution as written, which the hospital wait has too. Those finding
/// every D2A place taken (Erlang B's share, which holds whatever the stay's distribution) wait
/// in hospital, where there is no limit, for a time T on average (Little's law).
fn delayed_in_hospital(m: &Model) -> [f64; 2] {
    let rate = 1.0 / interarrival(m, "patients");
    let share = |flow: &str| {
        m.connections
            .iter()
            .find(|c| c.id == ElementId::unchecked(flow))
            .and_then(|c| c.probability)
            .unwrap_or_else(|| panic!("{flow}: no probability"))
    };
    [
        ("flow-2", "d2a_inpatient", "hospital_nursing_home"),
        ("flow-3", "d2a_home", "hospital_care_at_home"),
    ]
    .map(|(flow, d2a, hospital)| {
        let lambda = rate * share(flow);
        let (stay, places) = server(m, d2a);
        let (wait, _) = server(m, hospital);
        assert!(
            (wait - stay).abs() < 1e-12,
            "{d2a} {stay}, {hospital} {wait}"
        );
        lambda * erlang_b(places, lambda * stay) * stay
    })
}

/// Delayed discharge, from L. M. Boyle, C. S. M. Currie, C. Lamas Fernandez, L. Nguyen and
/// C. Halpenny, *A Discrete Event Simulation Model of a Hospital for Prediction of the Impact of
/// Delayed Discharge*, Proceedings of the Operational Research Society Simulation Workshop 2023
/// (SW23), pp. 240–249, doi 10.36819/SW23.029 (Table 1 on p. 244, Tables 2–5 on pp. 245–246).
/// Patients fit to leave hospital arrive at 114 a day (Poisson); 18.52% need a nursing home and
/// go to a D2A inpatient bed (75), the rest need care at home and go to a D2A home place (500).
/// Those who find none free wait in hospital, with no limit. A nursing home accepts a patient on
/// a given day with chance 1/7, care at home with chance 1/3. The model is two independent loss
/// systems: the stay, in D2A or in hospital, is 1 + 2(N − 1) days with N geometric on 1, 2, …
/// (each refusal costs 2 days), mean 2/p − 1: 13 days and 5 days. The paper's text gives each
/// wait 1 day, which would put 74 in hospital rather than its 205; 2 days per refusal is the
/// reading that gives its tables. The file writes the stays as discrete user distributions on
/// 1, 3, 5, …, cut where the rest of the tail is below 1e-9 and that rest put on the last point,
/// and the check uses the mean as written.
///
/// Exact, by Erlang B: 202.14 patients wait in hospital on average (199.84 for a nursing home,
/// 2.31 for care at home); the paper's Table 2 has 205 (203, 207) and the NHS data 208. The run
/// uses the paper's 8 weeks of warm-up and a year, with 20 replications rather than its 5.
#[test]
fn delayed_discharge_matches_erlang_b() {
    let m = model_of("delayed-discharge.bpmn", None);
    assert!(
        (interarrival(&m, "patients") * 114.0 - 1.0).abs() < 1e-12,
        "{}",
        interarrival(&m, "patients")
    );
    // The stays' means as written are within 1e-7 of 2/p − 1.
    for (id, p) in [("d2a_inpatient", 1.0 / 7.0), ("d2a_home", 1.0 / 3.0)] {
        let (stay, _) = server(&m, id);
        assert!((stay - (2.0 / p - 1.0)).abs() < 1e-7, "{id}: {stay}");
    }
    let [nursing_home, care_at_home] = delayed_in_hospital(&m);
    assert!(
        (nursing_home + care_at_home - 202.14).abs() < 0.005,
        "{nursing_home} + {care_at_home}"
    );

    let mut e = open(
        "delayed-discharge.bpmn",
        json!({ "replications": 20, "warmup": 56.0, "duration": 365.0 }),
    );
    let r = ok(&mut e, "run", json!({}));
    let behaviour = behaviour_warnings(&r);
    assert!(behaviour.is_empty(), "{behaviour:?}");
    covers(&r, "hospital_nursing_home", "mean_contents", nursing_home);
    covers(&r, "hospital_care_at_home", "mean_contents", care_at_home);
    let rate = 114.0;
    for (id, share, places) in [("d2a_inpatient", 0.1852, 75u32), ("d2a_home", 0.8148, 500)] {
        let a = rate * share * server(&m, id).0;
        let b = erlang_b(places, a);
        covers(&r, id, "loss_probability", b);
        covers(&r, id, "utilisation", a * (1.0 - b) / f64::from(places));
    }
}

/// The paper's published means of the patients waiting in hospital (Tables 2–5), against the
/// exact values of the file's scenarios. Every exact value sits below the published one, by 0.3
/// (nursing home 1 in 10) to 3.5 (D2A home 400 places). The published 95% intervals, over 5
/// runs, hold the exact value in 13 of the 16 cells; the baseline's (203, 207), 450 places'
/// (227, 233) and a nursing home 1 in 5's (118, 121) do not. Table 2's rows for 550 and 650 D2A
/// home places are identical, and match Table 4's row for care at home 1 in 2 but for one digit;
/// the exact values agree, since with that many places, or care at home accepting every other
/// day, hardly anyone waits in hospital for care at home (199.84 in all three). The paper derives
/// p from the expected wait by p Σ i(1 − p)^i = 1/p, but that sum is (1 − p)/p.
#[test]
fn delayed_discharge_sits_just_below_the_published_tables() {
    for (scenario, published) in [
        (None, 205.0),
        (Some("home-350"), 320.0),
        (Some("home-400"), 273.0),
        (Some("home-450"), 230.0),
        (Some("home-550"), 202.0),
        (Some("home-650"), 202.0),
        (Some("nursing-home-1-in-5"), 119.0),
        (Some("nursing-home-1-in-6"), 162.0),
        (Some("nursing-home-1-in-8"), 246.0),
        (Some("nursing-home-1-in-9"), 288.0),
        (Some("nursing-home-1-in-10"), 329.0),
        (Some("care-at-home-1-in-2"), 202.0),
        (Some("care-at-home-1-in-4"), 356.0),
        (Some("care-at-home-1-in-5"), 539.0),
        (Some("inpatient-85"), 195.0),
        (Some("inpatient-95"), 185.0),
    ] {
        let [nursing_home, care_at_home] =
            delayed_in_hospital(&model_of("delayed-discharge.bpmn", scenario));
        let exact = nursing_home + care_at_home;
        eprintln!("{scenario:?}: exact {exact:.2}, published {published}");
        assert!(
            published - exact > 0.25 && published - exact < 3.6,
            "{scenario:?}: exact {exact}, published {published}"
        );
    }
    // The sum the paper writes as 1/p.
    let p: f64 = 1.0 / 7.0;
    let (mut sum, mut power) = (0.0, 1.0);
    for i in 1..2_000 {
        power *= 1.0 - p;
        sum += p * f64::from(i) * power;
    }
    assert!((sum - (1.0 - p) / p).abs() < 1e-9, "{sum}");
}

/// The delayed-discharge scenarios, each checked on the hospital wait it changes against
/// [`delayed_in_hospital`]: D2A home places 350, 400, 450, 550 and 650; a nursing home accepting
/// 1 day in 5, 6, 8, 9 or 10; care at home accepting 1 day in 2, 4 or 5; D2A inpatient beds 85
/// and 95. Exact totals: 317.1, 269.5, 226.6, 199.8, 199.8; 118.0, 160.0, 244.3, 286.5, 328.7;
/// 199.8, 353.2, 537.3; 192.2, 182.3. The runs warm up for 200 days: with stays of 19 days on
/// average, 8 weeks would leave the hospital wait about 0.3% low. Where the exact wait is all
/// but 0 (care at home with 550 or 650 places, or accepted 1 day in 2), the run is bounded
/// instead, since its interval is then 0 wide.
#[test]
#[ignore = "heavy: 15 scenarios of a year each"]
fn delayed_discharge_scenarios_match_erlang_b() {
    for (scenario, changes) in [
        ("home-350", "hospital_care_at_home"),
        ("home-400", "hospital_care_at_home"),
        ("home-450", "hospital_care_at_home"),
        ("home-550", "hospital_care_at_home"),
        ("home-650", "hospital_care_at_home"),
        ("nursing-home-1-in-5", "hospital_nursing_home"),
        ("nursing-home-1-in-6", "hospital_nursing_home"),
        ("nursing-home-1-in-8", "hospital_nursing_home"),
        ("nursing-home-1-in-9", "hospital_nursing_home"),
        ("nursing-home-1-in-10", "hospital_nursing_home"),
        ("care-at-home-1-in-2", "hospital_care_at_home"),
        ("care-at-home-1-in-4", "hospital_care_at_home"),
        ("care-at-home-1-in-5", "hospital_care_at_home"),
        ("inpatient-85", "hospital_nursing_home"),
        ("inpatient-95", "hospital_nursing_home"),
    ] {
        let [nursing_home, care_at_home] =
            delayed_in_hospital(&model_of("delayed-discharge.bpmn", Some(scenario)));
        let exact = if changes == "hospital_nursing_home" {
            nursing_home
        } else {
            care_at_home
        };
        let mut e = open_scenario(
            "delayed-discharge.bpmn",
            Some(scenario),
            json!({ "replications": 20, "warmup": 200.0, "duration": 365.0 }),
        );
        let r = ok(&mut e, "run", json!({}));
        if exact < 0.01 {
            let (mean, _) = result(&r, changes, "mean_contents");
            assert!(mean < 0.5, "{scenario}: {mean}, exact {exact}");
        } else {
            covers(&r, changes, "mean_contents", exact);
        }
    }
}
