//! Finite buffers and blocking against exact results: M/M/1/K, a two-machine line with a finite
//! intermediate buffer (docs/research.md, "Finite buffers and blocking"), and deterministic
//! cases worked by hand.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, ExperimentSummary, Metric, Report, compile, summarise};
use stochcraft_model::{Dist, Experiment, Model};
use stochcraft_random::Rng;
use stochcraft_stats::Summary;

const REPS: u64 = 16;
/// Each check is a 95% interval, so about one in twenty would miss by chance; the seeds are fixed
/// so the tests are repeatable.
const SEED: u64 = 30_000;

fn experiment() -> Experiment {
    Experiment {
        warmup: 1_000.0,
        duration: 20_000.0,
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

fn covers(s: &ExperimentSummary, element: &str, metric: Metric, theory: f64) {
    let got = s.get(element, metric);
    eprintln!("{element} {}: {got:?} (theory {theory})", metric.name());
    assert!(
        got.is_some_and(|g| g.n == REPS && g.contains(theory)),
        "{element} {}: {got:?} vs {theory}",
        metric.name()
    );
}

/// The stationary distribution of a birth-death chain on 0..=n with constant birth rate `up`
/// and death rate `down`: p_k ∝ (up/down)^k.
fn birth_death(up: f64, down: f64, n: usize) -> Vec<f64> {
    let rho = up / down;
    let mut p = vec![1.0];
    for k in 1..=n {
        p.push(p[k - 1] * rho);
    }
    let total: f64 = p.iter().sum();
    p.iter().map(|x| x / total).collect()
}

/// M/M/1/K: a source into one server whose system holds at most K entities (K − 1 waiting, so
/// `queue_capacity` is K − 1); an arrival that finds K there is lost. With p_n ∝ ρ^n on 0..=K:
/// loss probability p_K, L = Σ n p_n, Lq = L − (1 − p_0), utilisation 1 − p_0 and, by Little's
/// law on the entities that get in, Wq = Lq / (λ (1 − p_K)).
fn mm1k(lambda: f64, mu: f64, k: u32) {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(lambda), None);
    let server = b.buffered_server("server", 1, Dist::exponential_rate(mu), k - 1);
    let exit = b.sink("exit");
    b.connect(arrivals, server);
    b.connect(server, exit);
    let model = b.build(experiment());
    assert_eq!(model.validate(), vec![]);

    let p = birth_death(lambda, mu, k as usize);
    let loss = p[k as usize];
    let l: f64 = p.iter().enumerate().map(|(n, x)| n as f64 * x).sum();
    let busy = 1.0 - p[0];
    let lq = l - busy;
    let wq = lq / (lambda * (1.0 - loss));
    eprintln!("M/M/1/{k}, λ = {lambda}, μ = {mu}: P_K = {loss}, L = {l}, Lq = {lq}, Wq = {wq}");

    let reps = reports(&model);
    let s = summarise(&reps);
    covers(&s, "server", Metric::LossProbability, loss);
    covers(&s, "server", Metric::MeanQueueLength, lq);
    covers(&s, "server", Metric::Utilisation, busy);
    covers(&s, "server", Metric::MeanWait, wq);
    // L, the mean number in the system, per replication: waiting plus in service.
    let ls: Vec<f64> = reps
        .iter()
        .filter_map(|r| {
            let e = r.element("server")?;
            Some(e.mean_queue_length? + e.mean_contents?)
        })
        .collect();
    let got = Summary::of(&ls);
    eprintln!("server L: {got:?} (theory {l})");
    assert!(
        got.is_some_and(|g| g.n == REPS && g.contains(l)),
        "{got:?} vs {l}"
    );
    // The queue never holds more than its capacity, so nothing upstream is blocked.
    assert_eq!(s.get("arrivals", Metric::BlockingProbability), None);
    assert_eq!(s.get("server", Metric::BlockingProbability), None);
}

#[test]
fn mm1k_loss_probability_and_number_in_system() {
    // ρ = 0.9, K = 5: P_K ≈ 0.12602, L ≈ 2.19478, Lq ≈ 1.40820, Wq ≈ 1.79029.
    mm1k(0.9, 1.0, 5);
}

#[test]
fn overloaded_mm1k_loses_what_it_cannot_serve() {
    // ρ = 1.5, K = 3: P_K = 27/65 ≈ 0.41538, L = 129/65 ≈ 1.98462, Lq = 72/65 ≈ 1.10769.
    mm1k(1.5, 1.0, 3);
}

/// Two exponential machines in series with `b` waiting places between them, blocking after
/// service, and a first machine that is never starved. The state n (parts at the second machine,
/// in service or waiting, plus one if the first machine holds a finished part it cannot pass on)
/// is a birth-death chain on 0..=b+2 with birth rate μ1 and death rate μ2, so the line behaves as
/// M/M/1/(b+2) with λ = μ1: throughput μ2 (1 − p_0), the first machine blocked with probability
/// p_(b+2), and the second machine's queue n − 1 parts for 1 ≤ n ≤ b+1 and b at n = b+2.
fn two_machine_line(mu1: f64, mu2: f64, b: u32) {
    let mut m = Builder::new();
    // Arrivals twice as fast as the first machine, into a waiting room of 40: the first machine
    // is starved only when 40 arrivals in a row are lost, which (rate 2 against at most 1 out)
    // has probability below 2^-40; arrivals that find it full are lost.
    let raw = m.source("raw", Dist::exponential_rate(2.0 * mu1), None);
    let m1 = m.buffered_server("m1", 1, Dist::exponential_rate(mu1), 40);
    let m2 = m.buffered_server("m2", 1, Dist::exponential_rate(mu2), b);
    let done = m.sink("done");
    m.connect(raw, m1);
    m.connect(m1, m2);
    m.connect(m2, done);
    let model = m.build(experiment());
    assert_eq!(model.validate(), vec![]);

    let n = b as usize + 2;
    let p = birth_death(mu1, mu2, n);
    let throughput = mu2 * (1.0 - p[0]);
    let blocked = p[n];
    let lq2: f64 = p
        .iter()
        .enumerate()
        .skip(1)
        .map(|(k, x)| (k.min(n - 1) - 1) as f64 * x)
        .sum();
    // Each part the first machine finishes waits, on average, P(blocked) / throughput (Little's
    // law on the blocked state, which holds at most one part).
    let mean_blocked = blocked / throughput;
    eprintln!(
        "line μ1 = {mu1}, μ2 = {mu2}, b = {b}: throughput {throughput}, P(m1 blocked) {blocked}, Lq2 {lq2}"
    );

    let s = summarise(&reports(&model));
    let duration = experiment().duration;
    covers(&s, "done", Metric::Entered, throughput * duration);
    covers(&s, "m1", Metric::BlockingProbability, blocked);
    covers(&s, "m1", Metric::MeanBlockedTime, mean_blocked);
    covers(&s, "m1", Metric::Utilisation, 1.0 - blocked);
    covers(&s, "m2", Metric::Utilisation, 1.0 - p[0]);
    covers(&s, "m2", Metric::MeanQueueLength, lq2);
    covers(&s, "m2", Metric::LossProbability, 0.0);
}

#[test]
fn two_machine_line_matches_the_closed_form_throughput() {
    // μ1 = 1, μ2 = 1.25, b = 2: ρ = 0.8, p_0 = 0.2 / (1 − 0.8^5) ≈ 0.29748, throughput ≈ 0.87815,
    // P(m1 blocked) = p_0 0.8^4 ≈ 0.12185.
    two_machine_line(1.0, 1.25, 2);
}

#[test]
fn balanced_two_machine_line() {
    // μ1 = μ2 = 1, b = 1: p_n = 1/4 for n = 0..3, throughput 3/4, P(m1 blocked) 1/4.
    two_machine_line(1.0, 1.0, 1);
}

fn run_once(b: &Builder, duration: f64) -> Report {
    let r = b.run(Rng::new(1), 0.0, duration);
    let Ok(r) = r else { panic!("{r:?}") };
    r
}

fn get(r: &Report, element: &str, metric: Metric) -> Option<f64> {
    r.element(element)?.metric(metric)
}

/// Asserts that `element` reports `want` for `metric`, to rounding.
fn is(r: &Report, element: &str, metric: Metric, want: f64) {
    let got = get(r, element, metric);
    assert!(
        got.is_some_and(|g| (g - want).abs() < 1e-9),
        "{element} {}: {got:?} vs {want}",
        metric.name()
    );
}

/// Arrivals every 1 (at 1 to 5) into a machine taking 1, then a machine taking 3 with one
/// waiting place. Worked by hand:
///
/// - Part 1 goes through m1 at 1–2 and m2 at 2–5. Part 2: m1 2–3, waits in m2's buffer 3–5.
/// - Part 3: m1 3–4, then blocked 4–5 (the buffer holds part 2); part 4 waits in m1's queue 4–5.
/// - At 5 part 1 leaves: part 2 starts on m2, part 3 takes its place in the buffer, part 4
///   starts on m1 (5–6) and part 5 arrives and waits. Part 4 is blocked 6–8, part 5 runs on m1
///   8–9 and is blocked 9–11. m2 starts part 3 at 8, part 4 at 11 and part 5 at 14.
/// - Parts leave at 5, 8, 11, 14 and 17: times in system 4, 6, 8, 10 and 12.
#[test]
fn a_full_buffer_blocks_the_machine_before_it() {
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(1.0), Some(5));
    let m1 = b.server("m1", 1, Dist::Constant(1.0));
    let m2 = b.buffered_server("m2", 1, Dist::Constant(3.0), 1);
    let out = b.sink("out");
    b.connect(src, m1);
    b.connect(m1, m2);
    b.connect(m2, out);
    let r = run_once(&b, 100.0);
    is(&r, "out", Metric::Entered, 5.0);
    is(&r, "out", Metric::MeanTimeInSystem, 8.0);
    // Blocked 0, 0, 1, 2 and 2: 5 in all, over 100.
    is(&r, "m1", Metric::MeanBlockedTime, 1.0);
    is(&r, "m1", Metric::BlockingProbability, 0.05);
    // m1's queue: part 4 waits 4–5, part 5 waits 5–8.
    is(&r, "m1", Metric::MeanWait, 0.8);
    // m2's buffer: parts 2–5 wait 2, 3, 3 and 3.
    is(&r, "m2", Metric::MeanWait, 2.2);
    is(&r, "m2", Metric::Lost, 0.0);
    is(&r, "m2", Metric::LossProbability, 0.0);
    // m2 sends to a sink, which has room for everything: nothing to report about blocking.
    assert_eq!(get(&r, "m2", Metric::MeanBlockedTime), None);
    assert_eq!(get(&r, "m2", Metric::BlockingProbability), None);
    // Blocked time is not service: m1 served 5 parts of 1 each.
    is(&r, "m1", Metric::Utilisation, 0.05);
}

/// With no waiting room, an arrival gets in only when the unit is free, and one that arrives at
/// the instant the unit frees gets in.
#[test]
fn no_waiting_room_loses_arrivals_while_busy() {
    for (service, entered, lost) in [(2.5, 2.0, 4.0), (2.0, 3.0, 3.0)] {
        let mut b = Builder::new();
        let src = b.source("in", Dist::Constant(1.0), Some(6));
        let desk = b.buffered_server("desk", 1, Dist::Constant(service), 0);
        let out = b.sink("out");
        b.connect(src, desk);
        b.connect(desk, out);
        let r = run_once(&b, 100.0);
        // Service 2.5: 1 and 4 get in. Service 2: 1, 3 and 5 get in (each as the previous one
        // leaves).
        is(&r, "desk", Metric::Entered, entered);
        is(&r, "desk", Metric::Lost, lost);
        is(&r, "desk", Metric::LossProbability, lost / 6.0);
        is(&r, "desk", Metric::MeanQueueLength, 0.0);
        is(&r, "out", Metric::Entered, entered);
    }
}

/// A delay into a full buffer keeps its entity (and counts it blocked); an exclusive gateway
/// waits for room, holding up the entities behind it.
#[test]
fn delays_and_gateways_wait_for_room() {
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(1.0), Some(3));
    let walk = b.delay("walk", Dist::Constant(0.5));
    let pick = b.exclusive("pick");
    let desk = b.buffered_server("desk", 1, Dist::Constant(4.0), 0);
    let out = b.sink("out");
    b.connect(src, walk);
    b.connect(walk, pick);
    b.connect(pick, desk);
    b.connect(desk, out);
    let model = b.build(Experiment::default());
    let codes: Vec<&str> = model.validate().iter().map(|d| d.problem.code()).collect();
    assert_eq!(codes, ["buffer_behind_gateway"]);
    let r = run_once(&b, 100.0);
    // Arrivals at 1, 2, 3 reach the gateway at 1.5, 2.5, 3.5; the desk takes them at 1.5, 5.5
    // and 9.5, and they leave at 5.5, 9.5 and 13.5. The gateway holds the second from 2.5.
    is(&r, "out", Metric::Entered, 3.0);
    is(&r, "desk", Metric::Lost, 0.0);
    is(
        &r,
        "out",
        Metric::MeanTimeInSystem,
        (4.5 + 7.5 + 10.5) / 3.0,
    );
    // The delay sends to the gateway, which has unlimited room: it is never blocked.
    assert_eq!(get(&r, "walk", Metric::MeanBlockedTime), None);
}

/// A delay straight into a full buffer is blocked after its delay.
#[test]
fn a_delay_is_blocked_after_its_delay() {
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(1.0), Some(2));
    let walk = b.delay("walk", Dist::Constant(0.5));
    let desk = b.buffered_server("desk", 1, Dist::Constant(4.0), 0);
    let out = b.sink("out");
    b.connect(src, walk);
    b.connect(walk, desk);
    b.connect(desk, out);
    let r = run_once(&b, 100.0);
    // The first walks 1–1.5 and is served 1.5–5.5; the second walks 2–2.5 and is blocked
    // 2.5–5.5.
    is(&r, "walk", Metric::MeanBlockedTime, 1.5);
    is(&r, "walk", Metric::MeanContents, 0.01);
    is(&r, "out", Metric::MeanTimeInSystem, (4.5 + 7.5) / 2.0);
}

/// Without finite buffers, none of the buffer metrics appear (results stay as they were).
#[test]
fn models_without_buffers_report_no_buffer_metrics() {
    let mut b = Builder::new();
    let src = b.source("in", Dist::exponential_rate(0.5), None);
    let desk = b.server("desk", 1, Dist::exponential_rate(1.0));
    let out = b.sink("out");
    b.connect(src, desk);
    b.connect(desk, out);
    let r = run_once(&b, 1_000.0);
    for e in &r.elements {
        for m in [
            Metric::MeanBlockedTime,
            Metric::BlockingProbability,
            Metric::Lost,
            Metric::LossProbability,
        ] {
            assert_eq!(e.metric(m), None, "{} {m:?}", e.id);
        }
    }
}
