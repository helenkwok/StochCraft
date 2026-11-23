//! Racing and the clean-up on synthetic normal data, by macro-replications (ROADMAP M9, stage
//! P2). They are heavy, so they are ignored by default and run in CI's heavy tests job
//! (`cargo test --release -- --ignored`).
//!
//! **The criterion.** As for the selection procedures (`tests/select.rs`): a rate of correct
//! answers passes when the upper end of the 95% Wilson interval of the observed proportion is at
//! least 0.95, so the macro-replications do not show the chance that racing drops the best or the
//! clean-up does not select it to be above 5%. Each check prints its rate, the interval, how often
//! racing dropped the best and how often the best was not a finalist, and the mean runs of racing
//! against running every candidate to the end with the same clean-up.
#![allow(clippy::panic)]

use std::io::Write;
use std::ops::Range;

use stochcraft_experiment::race::{Race, RaceAnswer, Racing, Status};
use stochcraft_experiment::select::{Kn, Sense, select_best};
use stochcraft_random::math::{cos, ln};
use stochcraft_random::{Rng, SplitMix64};
use stochcraft_stats::Proportion;

const MACRO: u64 = 1000;

/// A standard normal keyed by (seed, a, b, c), independent of the order of calls.
fn normal(seed: u64, a: u64, b: u64, c: u64) -> f64 {
    let mut sm = SplitMix64::new(
        seed ^ a.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ b.wrapping_mul(0xd1b5_4a32_d192_ed03)
            ^ c.wrapping_mul(0x8cb9_2ba7_2f3d_8dd7),
    );
    let mut rng = Rng::new(sm.next_u64());
    let u1 = rng.uniform_open0();
    let u2 = rng.uniform();
    (-2.0 * ln(u1)).sqrt() * cos(2.0 * std::f64::consts::PI * u2)
}

/// Normal observations with variance 1 and correlation `rho` between candidates in the same
/// replication (a shared part √ρ Zⱼ, as common random numbers would give), for macro-replication
/// `m`.
fn correlated(
    seed: u64,
    m: u64,
    means: Vec<f64>,
    rho: f64,
) -> impl FnMut(usize, Range<u64>) -> Vec<f64> {
    move |s, reps| {
        reps.map(|j| {
            means[s]
                + rho.sqrt() * normal(seed, m, u64::MAX, j)
                + (1.0 - rho).sqrt() * normal(seed, m, s as u64, j)
        })
        .collect()
    }
}

/// Running every candidate to the end (every stage, no candidate dropped), then the same
/// clean-up: KN among the k best means on the replications after the last stage. Its runs.
fn without_racing<F>(settings: &Racing, candidates: usize, sample: &mut F) -> u64
where
    F: FnMut(usize, Range<u64>) -> Vec<f64>,
{
    let n = settings.max_replications();
    let s = match settings.sense {
        Sense::Maximise => 1.0,
        Sense::Minimise => -1.0,
    };
    let mut means: Vec<(usize, f64)> = (0..candidates)
        .map(|c| (c, s * sample(c, 0..n).iter().sum::<f64>() / n as f64))
        .collect();
    means.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let finalists: Vec<usize> = means
        .iter()
        .take(settings.finalists)
        .map(|(c, _)| *c)
        .collect();
    let kn = Kn {
        delta: settings.delta,
        confidence: settings.cleanup_confidence(),
        first_stage: settings.cleanup_first_stage,
        sense: settings.sense,
        sampling: settings.sampling,
        max_runs: None,
    };
    let selection = select_best(&kn, finalists.len(), |p, reps| {
        sample(finalists[p], reps.start + n..reps.end + n)
    });
    let Ok(selection) = selection else {
        panic!("{selection:?}")
    };
    candidates as u64 * n + selection.total_runs()
}

/// The counts of one check.
#[derive(Default)]
struct Tally {
    correct: u64,
    best_dropped: u64,
    best_not_final: u64,
    runs: u64,
    baseline: u64,
}

impl Tally {
    fn add(&mut self, a: &RaceAnswer, race: &Race, best: usize, baseline: u64) {
        self.correct += u64::from(a.chosen == Some(best));
        self.best_dropped += u64::from(matches!(race.status(best), Some(Status::Dropped { .. })));
        self.best_not_final += u64::from(!a.finalists.contains(&best));
        self.runs += a.total_runs();
        self.baseline += baseline;
    }

    /// Prints the rate with its Wilson interval and the runs, and checks it against 0.95.
    fn check(&self, name: &str) -> (f64, f64) {
        let p = Proportion::new(self.correct, MACRO);
        let Some((lo, hi)) = p.wilson(0.95) else {
            panic!("{name}: no trials")
        };
        let (runs, baseline) = (
            self.runs as f64 / MACRO as f64,
            self.baseline as f64 / MACRO as f64,
        );
        let _ = writeln!(
            std::io::stderr(),
            "race {name}: {}/{MACRO} correct = {:.4}, Wilson 95% [{lo:.4}, {hi:.4}]; best dropped by racing {}, best not a finalist {}; mean runs with racing {runs:.1}, every candidate to the end {baseline:.1}",
            self.correct,
            self.correct as f64 / MACRO as f64,
            self.best_dropped,
            self.best_not_final,
        );
        assert!(
            hi >= 0.95,
            "{name}: {}/{MACRO} correct, Wilson 95% [{lo:.4}, {hi:.4}] lies below 0.95",
            self.correct
        );
        (runs, baseline)
    }
}

/// The slippage configuration: ten candidates, the best (3) δ = 1/√10 above nine equal others,
/// variance 1, independent and with correlation 0.5; racing with the defaults (stages of 10, then
/// 5 up to 30 replications, M = 10), then KN among the five best. The others are exactly δ worse,
/// so racing, which drops only candidates worse by more than δ, should drop almost none: the
/// check is that the best is kept and selected at least 95% of the time.
#[test]
#[ignore = "heavy: 2,000 macro-replications"]
fn racing_in_the_slippage_configuration() {
    let delta = 1.0 / 10f64.sqrt();
    let settings = Racing::new(delta, Sense::Maximise, 10);
    let mut means = vec![0.0; 10];
    means[3] = delta;
    for (name, rho, seed) in [
        ("slippage, independent", 0.0, 111),
        ("slippage, rho 0.5", 0.5, 222),
    ] {
        let mut tally = Tally::default();
        for m in 0..MACRO {
            let mut sample = correlated(seed, m, means.clone(), rho);
            let Ok(mut race) = Race::new(settings) else {
                panic!("settings")
            };
            for _ in 0..10 {
                if let Err(e) = race.add() {
                    panic!("{e}")
                }
            }
            if let Err(e) = race.run(&mut sample) {
                panic!("{e}")
            }
            let a = race.clean_up(&mut sample);
            let Ok(a) = a else { panic!("{a:?}") };
            // The race asked only for indices below the clean-up's first.
            assert_eq!(a.fresh_from, settings.max_replications());
            let baseline = without_racing(&settings, 10, &mut sample);
            tally.add(&a, &race, 3, baseline);
        }
        tally.check(name);
    }
}

/// Candidates arriving one at a time: a feed of 50 candidates with known means (minimise), the
/// race run after each arrival. The best (mean 0) arrives 42nd (candidate 41); the others lie
/// from δ to 10δ above it, one exactly δ, spread by a fixed permutation. Stages of 10, then 5 up
/// to 40 replications (L = 7, M = 50), then KN among the five best. The best must be kept and
/// selected at least 95% of the time, and racing must use fewer runs on average than running
/// every candidate to the end with the same clean-up.
#[test]
#[ignore = "heavy: 1,000 macro-replications of 50 candidates"]
fn racing_a_feed_of_fifty_candidates() {
    let delta = 1.0 / 10f64.sqrt();
    let settings = Racing {
        stages: 7,
        ..Racing::new(delta, Sense::Minimise, 50)
    };
    let best = 41;
    let mut means = Vec::with_capacity(50);
    let mut j = 0;
    for c in 0..50 {
        if c == best {
            means.push(0.0);
        } else {
            let u = f64::from((j * 17) % 49) / 48.0;
            means.push(delta * (1.0 + 9.0 * u));
            j += 1;
        }
    }
    let mut tally = Tally::default();
    for m in 0..MACRO {
        let mut sample = correlated(333, m, means.clone(), 0.0);
        let Ok(mut race) = Race::new(settings) else {
            panic!("settings")
        };
        for c in 0..50 {
            assert_eq!(race.add(), Ok(c));
            if let Err(e) = race.run(&mut sample) {
                panic!("{e}")
            }
        }
        let a = race.clean_up(&mut sample);
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.tried, 50);
        let baseline = without_racing(&settings, 50, &mut sample);
        tally.add(&a, &race, best, baseline);
    }
    let (runs, baseline) = tally.check("feed of 50");
    assert!(
        runs < baseline,
        "racing used {runs:.1} runs on average, every candidate to the end {baseline:.1}"
    );
}
