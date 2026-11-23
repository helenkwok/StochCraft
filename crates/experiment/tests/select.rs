//! The selection procedures' checks on synthetic normal data, by macro-replications (ROADMAP M8,
//! stage O1). They are heavy, so they are ignored by default and run in CI's heavy tests job
//! (`cargo test --release -- --ignored`).
//!
//! **The criterion.** A rate of correct decisions passes when the upper end of the 95% Wilson
//! interval of the observed proportion is at least 0.95: the macro-replications do not show the
//! probability of a correct decision to be below the nominal 0.95. Each check prints its rate,
//! the interval and the mean replications used to standard error, which CI's log keeps.
#![allow(clippy::panic)]

use std::io::Write;
use std::ops::Range;

use stochcraft_experiment::select::{
    Bound, Constraint, Decision, Feasibility, Kn, Outcome, Sense, check_feasibility,
    compare_with_standard, select_best, select_feasible,
};
use stochcraft_random::math::{cos, ln};
use stochcraft_random::{Rng, SplitMix64};
use stochcraft_stats::Proportion;

const MACRO: u64 = 2000;

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

/// Normal observations with variance 1 and correlation `rho` between systems in the same
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

/// Prints a rate with its Wilson interval and mean runs, and checks it against 0.95.
fn check_rate(name: &str, correct: u64, n: u64, runs: u64) {
    let p = Proportion::new(correct, n);
    let Some((lo, hi)) = p.wilson(0.95) else {
        panic!("{name}: no trials")
    };
    let mean_runs = runs as f64 / n as f64;
    let _ = writeln!(
        std::io::stderr(),
        "select {name}: {correct}/{n} correct = {:.4}, Wilson 95% [{lo:.4}, {hi:.4}], mean runs {mean_runs:.1}",
        correct as f64 / n as f64
    );
    assert!(
        hi >= 0.95,
        "{name}: {correct}/{n} correct, Wilson 95% [{lo:.4}, {hi:.4}] lies below 0.95"
    );
}

/// The slippage configuration: k = 10, n₀ = 10, 1 − α = 0.95, the best δ = 1/√n₀ above k − 1
/// equal others, variance 1. Correct selection at least 95% of the time, independent and with
/// correlation 0.5, and fewer replications with the correlation.
#[test]
#[ignore = "heavy: 4,000 macro-replications"]
fn kn_in_the_slippage_configuration() {
    let delta = 1.0 / 10f64.sqrt();
    let setup = Kn::new(delta, Sense::Maximise);
    let mut means = vec![0.0; 10];
    means[3] = delta;
    let mut mean_runs = Vec::new();
    for (name, rho, seed) in [
        ("slippage, independent", 0.0, 101),
        ("slippage, rho 0.5", 0.5, 202),
    ] {
        let (mut correct, mut runs) = (0, 0);
        for m in 0..MACRO {
            let s = select_best(&setup, 10, correlated(seed, m, means.clone(), rho));
            let Ok(s) = s else { panic!("{s:?}") };
            assert_eq!(s.outcome, Outcome::Selected);
            correct += u64::from(s.chosen == 3);
            runs += s.total_runs();
        }
        check_rate(name, correct, MACRO, runs);
        mean_runs.push(runs as f64 / MACRO as f64);
    }
    assert!(
        mean_runs[1] < mean_runs[0],
        "correlation 0.5 used {} replications on average, independent {}",
        mean_runs[1],
        mean_runs[0]
    );
}

/// Every mean within δ of the best: any choice is correct. Each macro-replication must stop, and
/// no system may take more than max(n₀, ⌊h²S²ᵢₗ/δ²⌋ + 1) replications over its pairs, where the
/// region closes; the replications used are recorded.
#[test]
#[ignore = "heavy: 2,000 macro-replications"]
fn kn_with_every_mean_within_delta() {
    let delta = 1.0 / 10f64.sqrt();
    let setup = Kn::new(delta, Sense::Minimise);
    let means: Vec<f64> = (0..10).map(|i| f64::from(i) * delta / 10.0).collect();
    let Some(h2) = setup.h2(10) else {
        panic!("no h²")
    };
    let n0 = setup.first_stage;
    let mut runs = 0;
    for m in 0..MACRO {
        let mut sample = correlated(303, m, means.clone(), 0.0);
        // The first stage the procedure will see, to bound its replications.
        let first: Vec<Vec<f64>> = (0..10).map(|s| sample(s, 0..n0)).collect();
        let mut bound = n0;
        for (i, a) in first.iter().enumerate() {
            for b in first.iter().skip(i + 1) {
                let d: Vec<f64> = a.iter().zip(b).map(|(x, y)| x - y).collect();
                let mean = d.iter().sum::<f64>() / n0 as f64;
                let s2 = d.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n0 - 1) as f64;
                bound = bound.max((h2 * s2 / (delta * delta)).floor() as u64 + 1);
            }
        }
        let s = select_best(&setup, 10, sample);
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(s.outcome, Outcome::Selected);
        assert!(
            s.runs.iter().all(|r| *r <= bound),
            "runs {:?} over the bound {bound}",
            s.runs
        );
        runs += s.total_runs();
    }
    let _ = writeln!(
        std::io::stderr(),
        "select every mean within delta: mean runs {:.1}",
        runs as f64 / MACRO as f64
    );
}

/// Batur and Kim's check with every system ε from its limits: ten systems, two constraints
/// (one "at most", one "at least", ε = 0.1, variance 1), five systems ε inside both limits and
/// five ε beyond the second. Every system decided correctly at least 95% of the time.
#[test]
#[ignore = "heavy: 2,000 macro-replications"]
fn feasibility_with_every_system_at_its_tolerance() {
    let eps = 0.1;
    let constraints = [
        Constraint {
            bound: Bound::AtMost,
            limit: 0.0,
            tolerance: eps,
        },
        Constraint {
            bound: Bound::AtLeast,
            limit: 0.0,
            tolerance: eps,
        },
    ];
    let mean = |s: usize| if s < 5 { [-eps, eps] } else { [-eps, -eps] };
    let mut expected = vec![Decision::Feasible; 5];
    expected.extend([Decision::Infeasible; 5]);
    let (mut correct, mut runs) = (0, 0);
    for m in 0..MACRO {
        let check = check_feasibility(&Feasibility::default(), 10, &constraints, |s, reps| {
            reps.map(|j| {
                let [a, b] = mean(s);
                vec![
                    a + normal(404, m, s as u64, 2 * j),
                    b + normal(404, m, s as u64, 2 * j + 1),
                ]
            })
            .collect()
        });
        let Ok(check) = check else {
            panic!("{check:?}")
        };
        assert!(check.is_complete());
        correct += u64::from(check.decisions == expected);
        runs += check.total_runs();
    }
    check_rate("feasibility at the tolerance", correct, MACRO, runs);
}

/// The check, then KN among the feasible: six systems; 0 to 2 have the best objectives but are
/// ε beyond the constraint; of the feasible 3 to 5 (ε inside it), 5 is δ better than 3 and 4.
/// Objectives with correlation 0.5. Correct when the feasibility decisions are right and 5 is
/// chosen, with α split half and half.
#[test]
#[ignore = "heavy: 2,000 macro-replications"]
fn kn_among_the_feasible() {
    let delta = 1.0 / 10f64.sqrt();
    let eps = 0.2;
    let setup = Kn::new(delta, Sense::Maximise);
    let constraints = [Constraint {
        bound: Bound::AtMost,
        limit: 1.0,
        tolerance: eps,
    }];
    let objective = [2.0, 2.0, 2.0, 0.0, 0.0, delta];
    let constrained = |s: usize| if s < 3 { 1.0 + eps } else { 1.0 - eps };
    let (mut correct, mut runs) = (0, 0);
    for m in 0..MACRO {
        let mut obj = correlated(505, m, objective.to_vec(), 0.5);
        let s = select_feasible(&setup, &constraints, 0.5, 6, |s, reps: Range<u64>| {
            let values = obj(s, reps.clone());
            values
                .into_iter()
                .zip(reps)
                .map(|(x, j)| vec![x, constrained(s) + normal(606, m, s as u64, j)])
                .collect()
        });
        let Ok(s) = s else { panic!("{s:?}") };
        correct += u64::from(s.feasibility.feasible() == [3, 4, 5] && s.chosen() == Some(5));
        runs += s.total_runs();
    }
    check_rate("KN among the feasible", correct, MACRO, runs);
}

/// Kim's comparison with a standard, five alternatives with correlation 0.5 (common random
/// numbers, Kim's Case C): every alternative equal to the standard keeps the standard, and one
/// alternative δ better than the others and the standard is chosen, each at least 95% of the
/// time.
#[test]
#[ignore = "heavy: 4,000 macro-replications"]
fn comparison_with_a_standard() {
    let delta = 1.0 / 10f64.sqrt();
    let setup = Kn::new(delta, Sense::Maximise);
    let mut better = vec![0.0; 6];
    better[2] = delta;
    for (name, means, wanted, seed) in [
        ("standard, all equal", vec![0.0; 6], 0, 707),
        ("standard, one better", better, 2, 808),
    ] {
        let (mut correct, mut runs) = (0, 0);
        for m in 0..MACRO {
            let s = compare_with_standard(&setup, 5, correlated(seed, m, means.clone(), 0.5));
            let Ok(s) = s else { panic!("{s:?}") };
            correct += u64::from(s.chosen == wanted);
            runs += s.total_runs();
        }
        check_rate(name, correct, MACRO, runs);
    }
}
