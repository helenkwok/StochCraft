//! Populations checked against exact answers (ROADMAP M4.3, stage A2): finite populations whose
//! answers come from their Markov chains, worked out here, not from large-population curves.
//!
//! Each check compares the replications with the exact answer by a band that a correct model
//! leaves rarely: a mean within four standard errors (about once in 16,000 for a normal mean), a
//! share within four binomial standard errors, and a chi-squared statistic below its 0.1%
//! critical value (by the Wilson–Hilferty approximation, which for these degrees of freedom is
//! within 1% of the exact value and on the high side). The seeds are fixed, so the checks are
//! deterministic; the longest run only in the heavy tests (`--ignored`).
#![allow(clippy::panic)] // A model that fails to build or run fails the test.

use std::sync::Arc;

use stochcraft_agents::{
    Action, Chart, ChartSpec, Model, Population, Run, SendSpec, Simulation, StateSpec, Statistic,
    Stepping, TransitionSpec, bands,
};
use stochcraft_random::math::{exp, ln};
use stochcraft_stats::spread::normal_quantile;

fn build(spec: &ChartSpec) -> Arc<Chart> {
    match spec.build() {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("{e}"),
    }
}

fn run(model: &Model, seed: u64, end: f64) -> Run {
    match model.run(seed, end) {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    }
}

fn simulate(model: &Model, seed: u64, end: f64) -> Simulation {
    let mut sim = match Simulation::new(model, seed) {
        Ok(s) => s,
        Err(e) => panic!("{e}"),
    };
    if let Err(e) = sim.run_until(end) {
        panic!("{e}");
    }
    sim
}

fn last(run: &Run, series: usize) -> f64 {
    run.series
        .get(series)
        .and_then(|s| s.values.last())
        .copied()
        .unwrap_or(f64::NAN)
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// The upper `alpha` critical value of chi-squared with `df` degrees of freedom
/// (Wilson–Hilferty).
fn chi2_critical(df: usize, alpha: f64) -> f64 {
    let k = df as f64;
    let z = normal_quantile(1.0 - alpha).unwrap_or(f64::NAN);
    let a = 2.0 / (9.0 * k);
    let c = 1.0 - a + z * a.sqrt();
    k * c * c * c
}

/// Pearson's chi-squared of observed counts against probabilities, merging neighbouring cells
/// until each expects at least 5. Returns the statistic and its degrees of freedom.
fn chi2(observed: &[u64], probs: &[f64]) -> (f64, usize) {
    let n: u64 = observed.iter().sum();
    let mut cells: Vec<(f64, f64)> = Vec::new();
    let (mut o, mut e) = (0.0, 0.0);
    for (k, &p) in probs.iter().enumerate() {
        o += observed.get(k).copied().unwrap_or(0) as f64;
        e += p * n as f64;
        if e >= 5.0 {
            cells.push((o, e));
            o = 0.0;
            e = 0.0;
        }
    }
    if let Some(cell) = cells.last_mut() {
        cell.0 += o;
        cell.1 += e;
    }
    let stat = cells.iter().map(|(o, e)| (o - e) * (o - e) / e).sum();
    (stat, cells.len() - 1)
}

/// 200 machines, each failing at rate 0.1 and never repaired: the count still working at time
/// t is binomial(200, e^(−0.1 t)).
#[test]
fn independent_failures_are_binomial() {
    let chart = build(
        &ChartSpec::new("machine")
            .data("failed_at", "-1")
            .state(
                StateSpec::state("working")
                    .on_entry(SendSpec::new("fail").at_rate("0.1"))
                    .transition(
                        TransitionSpec::on("fail")
                            .to("failed")
                            .then(Action::assign("failed_at", "time()")),
                    ),
            )
            .state(StateSpec::state("failed")),
    );
    let model = Model::new()
        .population(Population::new("machines", chart).count(200))
        .statistic(Statistic::count("working", "machines").when("In('working')"))
        .save_every(1.0);
    let reps = 1000;
    let runs: Vec<Run> = (1..=reps).map(|seed| run(&model, seed, 5.0)).collect();
    assert_eq!(runs[0].times, [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    let at5: Vec<f64> = runs.iter().map(|r| last(r, 0)).collect();
    let (n, p) = (200.0, exp(-0.5));
    let expected = n * p;
    let se = (n * p * (1.0 - p) / reps as f64).sqrt();
    let m = mean(&at5);
    assert!(
        (m - expected).abs() < 4.0 * se,
        "mean working at 5: {m}, binomial mean {expected}, standard error {se}"
    );
    // The binomial's probabilities, by the ratio of successive terms.
    let mut probs = vec![exp(200.0 * ln(1.0 - p))];
    for k in 0..200 {
        let prev = probs[k];
        probs.push(prev * (200 - k) as f64 / (k + 1) as f64 * p / (1.0 - p));
    }
    let mut observed = vec![0u64; 201];
    for &x in &at5 {
        observed[x as usize] += 1;
    }
    let (stat, df) = chi2(&observed, &probs);
    let crit = chi2_critical(df, 0.001);
    assert!(
        stat < crit,
        "chi-squared {stat} on {df} df, 0.1% critical value {crit}"
    );
    // The bands across the replications: the mean is the replications' mean.
    let b = bands(&runs);
    assert_eq!(b.times, runs[0].times);
    let band = &b.series[0];
    assert_eq!(band.name, "working");
    assert_eq!(band.mean[0], 200.0);
    assert!(
        (band.mean[5] - m).abs() < 1e-9,
        "{} against {m}",
        band.mean[5]
    );
    for k in 0..6 {
        assert!(band.low[k] <= band.median[k] && band.median[k] <= band.high[k]);
    }
    let expected1 = 200.0 * exp(-0.1);
    assert!(band.low[1] < expected1 && expected1 < band.high[1]);
}

/// The time of adoption of every agent, in order, for one replication of Bass as agents.
fn bass_times(n: u64, seed: u64, end: f64) -> (Vec<f64>, u64) {
    let chart = build(
        &ChartSpec::new("person")
            .data("adopted_at", "-1")
            .state(
                StateSpec::state("potential")
                    .on_entry(SendSpec::new("advert").id("ad").at_rate("0.011"))
                    .on_exit(Action::cancel("ad"))
                    .transition(TransitionSpec::on("advert wom").to("adopted")),
            )
            .state(
                StateSpec::state("adopted")
                    .on_entry(Action::assign("adopted_at", "time()"))
                    .on_entry(SendSpec::new("talk").at_rate("1.5"))
                    .transition(
                        TransitionSpec::on("talk")
                            .then(SendSpec::new("wom").to_random("people"))
                            .then(SendSpec::new("talk").at_rate("1.5")),
                    ),
            ),
    );
    let model = Model::new().population(Population::new("people", chart).count(n));
    let sim = simulate(&model, seed, end);
    let mut times: Vec<f64> = (0..n)
        .map(|s| {
            sim.agent("people", s)
                .and_then(|a| a.value("adopted_at"))
                .map_or(f64::NAN, |v| v.number())
        })
        .collect();
    assert!(
        times.iter().all(|&t| t >= 0.0),
        "seed {seed}: not every agent adopted by {end}"
    );
    times.sort_by(f64::total_cmp);
    let dropped = sim.counts()[0].dropped;
    (times, dropped)
}

/// With k adopters, adoptions happen at rate (N − k)(p + q k / (N − 1)): a pure-birth chain.
/// Returns the mean and standard deviation of the time to half and to all.
fn bass_chain(n: u64, p: f64, q: f64) -> ((f64, f64), (f64, f64)) {
    let rate = |k: u64| (n - k) as f64 * (p + q * k as f64 / (n - 1) as f64);
    let (mut m, mut v) = (0.0, 0.0);
    let mut half = (0.0, 0.0);
    for k in 0..n {
        if k == n / 2 {
            half = (m, v);
        }
        let r = rate(k);
        m += 1.0 / r;
        v += 1.0 / (r * r);
    }
    ((half.0, half.1.sqrt()), (m, v.sqrt()))
}

fn check_bass(n: u64, reps: u64, end: f64) {
    let ((half, half_sd), (all, all_sd)) = bass_chain(n, 0.011, 1.5);
    let (mut t_half, mut t_all) = (Vec::new(), Vec::new());
    let mut dropped = 0;
    for seed in 1..=reps {
        let (times, d) = bass_times(n, seed, end);
        t_half.push(times[(n / 2 - 1) as usize]);
        t_all.push(times[(n - 1) as usize]);
        dropped += d;
    }
    let r = reps as f64;
    let (mh, ma) = (mean(&t_half), mean(&t_all));
    assert!(
        (mh - half).abs() < 4.0 * half_sd / r.sqrt(),
        "N = {n}: mean time to half {mh}, chain {half} (sd {half_sd}, {reps} runs)"
    );
    assert!(
        (ma - all).abs() < 4.0 * all_sd / r.sqrt(),
        "N = {n}: mean time to all {ma}, chain {all} (sd {all_sd}, {reps} runs)"
    );
    assert!(
        dropped > 0,
        "word of mouth to adopters is dropped and counted"
    );
}

#[test]
fn bass_diffusion_as_agents_matches_its_chain_for_100() {
    check_bass(100, 200, 20.0);
}

/// ROADMAP's numbers: for N = 1,000, the mean time to all is 8.2502 (sd 0.8905), to half 3.3052.
#[test]
#[ignore = "heavy: 200 runs of 1,000 agents"]
fn bass_diffusion_as_agents_matches_its_chain_for_1000() {
    let ((half, _), (all, all_sd)) = bass_chain(1000, 0.011, 1.5);
    assert!((all - 8.2502).abs() < 5e-5, "{all}");
    assert!((all_sd - 0.8905).abs() < 5e-5, "{all_sd}");
    assert!((half - 3.3052).abs() < 5e-5, "{half}");
    check_bass(1000, 200, 16.0);
}

/// The final size of SIR in a well-mixed population of `n` with one infective: from the
/// embedded jump chain, where with s susceptible the next event is an infection with probability
/// r0 s / (r0 s + n − 1) (each infective contacts a random other agent at rate r0 γ and recovers at
/// rate γ). Returns the probability of each final number ever infected.
fn sir_final_size(n: usize, r0: f64) -> Vec<f64> {
    // prob[s][i]: the probability of passing through s susceptible and i infective.
    let mut prob = vec![vec![0.0; n + 2]; n];
    prob[n - 1][1] = 1.0;
    let mut final_size = vec![0.0; n + 1];
    for s in (0..n).rev() {
        for i in (1..=n).rev() {
            let w = prob[s][i];
            if w == 0.0 {
                continue;
            }
            let infect = r0 * s as f64 / (r0 * s as f64 + (n - 1) as f64);
            if s > 0 {
                prob[s - 1][i + 1] += w * infect;
            }
            if i == 1 {
                final_size[n - s] += w * (1.0 - infect);
            } else {
                prob[s][i - 1] += w * (1.0 - infect);
            }
        }
    }
    final_size
}

#[test]
fn sir_final_size_matches_the_embedded_chain() {
    let n = 100;
    let dist = sir_final_size(n, 1.5);
    let total: f64 = dist.iter().sum();
    assert!((total - 1.0).abs() < 1e-12, "{total}");
    assert!(
        (dist[1] - 0.4).abs() < 1e-12,
        "the first infective recovers first: {}",
        dist[1]
    );
    let exact_mean: f64 = dist.iter().enumerate().map(|(k, p)| k as f64 * p).sum();
    let exact_var: f64 = dist
        .iter()
        .enumerate()
        .map(|(k, p)| (k as f64 - exact_mean) * (k as f64 - exact_mean) * p)
        .sum();
    let early: f64 = dist[..=10].iter().sum();
    let chart = build(
        &ChartSpec::new("person")
            .state(
                StateSpec::state("start")
                    .transition(
                        TransitionSpec::eventless()
                            .when("serial() = 0")
                            .to("infected"),
                    )
                    .transition(TransitionSpec::eventless().to("susceptible")),
            )
            .state(
                StateSpec::state("susceptible")
                    .transition(TransitionSpec::on("contact").to("infected")),
            )
            .state(
                StateSpec::state("infected")
                    .on_entry(SendSpec::new("talk").id("t").at_rate("1.5"))
                    .on_entry(SendSpec::new("recover").at_rate("1"))
                    .on_exit(Action::cancel("t"))
                    .transition(
                        TransitionSpec::on("talk")
                            .then(SendSpec::new("contact").to_random("people"))
                            .then(SendSpec::new("talk").id("t").at_rate("1.5")),
                    )
                    .transition(TransitionSpec::on("recover").to("recovered")),
            )
            .state(StateSpec::state("recovered")),
    );
    let model = Model::new()
        .population(Population::new("people", chart).count(n as u64))
        .statistic(Statistic::count("susceptible", "people").when("In('susceptible')"));
    let reps = 2000;
    let sizes: Vec<f64> = (1..=reps)
        .map(|seed| n as f64 - last(&run(&model, seed, 1e6), 0))
        .collect();
    let r = reps as f64;
    let m = mean(&sizes);
    let se = (exact_var / r).sqrt();
    assert!(
        (m - exact_mean).abs() < 4.0 * se,
        "mean final size {m}, exact {exact_mean}, standard error {se}"
    );
    let share = sizes.iter().filter(|&&s| s <= 10.0).count() as f64 / r;
    let se = (early * (1.0 - early) / r).sqrt();
    assert!(
        (share - early).abs() < 4.0 * se,
        "share of runs with at most 10 ever infected {share}, exact {early}, standard error {se}"
    );
}

/// Pólya's urn in stepped mode: one agent of each colour; each step one new agent copies the
/// colour of an agent picked at random. After n steps the number of new agents of the first
/// colour is uniform on 0, 1, ..., n.
#[test]
fn polyas_urn_in_stepped_mode_is_uniform() {
    let ball = build(
        &ChartSpec::new("ball")
            .data(
                "colour",
                "if(serial() < 2, serial(), agent-value('balls', \
         floor(random() * (population-size('balls') - 1)), 'colour'))",
            )
            .state(StateSpec::state("here")),
    );
    let urn = build(
        &ChartSpec::new("urn").state(
            StateSpec::state("on")
                .transition(TransitionSpec::on("before-step").then(Action::create("balls"))),
        ),
    );
    let steps = 10;
    let model = Model::new()
        .population(Population::new("balls", ball).count(2))
        .environment(urn)
        .stepped(Stepping::every(1.0))
        .statistic(Statistic::count("first", "balls").when("$colour = 0"));
    let reps = 2200;
    let mut observed = vec![0u64; steps + 1];
    for seed in 1..=reps {
        let r = run(&model, seed, steps as f64);
        assert_eq!(r.counts[0].created, steps as u64 + 2, "seed {seed}");
        let new_first = last(&r, 0) - 1.0;
        observed[new_first as usize] += 1;
    }
    let probs = vec![1.0 / (steps + 1) as f64; steps + 1];
    let (stat, df) = chi2(&observed, &probs);
    assert_eq!(df, steps);
    let crit = chi2_critical(df, 0.001);
    assert!(
        stat < crit,
        "chi-squared {stat} on {df} df, 0.1% critical value {crit}: {observed:?}"
    );
}
