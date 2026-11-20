//! Monte Carlo models whose answers are known exactly (ROADMAP M4.1's checks).
//!
//! Each check allows five standard errors of the estimate, so a check of a correct
//! implementation fails about once in 1.7 million runs; the seeds are fixed anyway.

use stochcraft_mc::{
    BATCH, Block, BlockJob, Dist, McCorrelation, McInput, McOutput, McResults, McStop, McTarget,
    MonteCarlo, RunOptions, Sampling, compile, rank_correlation,
};
use stochcraft_random::math::{exp, ln};
use stochcraft_random::{Family, ThreePoint, Upper};
use stochcraft_stats::spread::{normal_cdf, normal_quantile};

fn input(name: &str, dist: Dist) -> McInput {
    McInput {
        name: name.into(),
        dist,
        unit: String::new(),
    }
}

fn output(name: &str, expression: &str) -> McOutput {
    McOutput {
        name: name.into(),
        expression: expression.into(),
        unit: String::new(),
        target: None,
    }
}

fn run(mc: &MonteCarlo, seed: u64, trials: u64) -> McResults {
    let c = compile(mc, seed).unwrap_or_else(|p| unreachable!("{p:?}"));
    c.run(&RunOptions {
        trials,
        stop: None,
        percentiles: vec![],
    })
}

fn get(v: Option<f64>) -> f64 {
    v.unwrap_or(f64::NAN)
}

/// X + Y + Z for independent normals: exactly normal, mean 25 and variance 4 + 9 + 1 = 14.
#[test]
fn the_sum_of_independent_normals() {
    for sampling in [Sampling::Random, Sampling::LatinHypercube] {
        let mc = MonteCarlo {
            inputs: vec![
                input(
                    "x",
                    Dist::Normal {
                        mean: 10.0,
                        std_dev: 2.0,
                    },
                ),
                input(
                    "y",
                    Dist::Normal {
                        mean: 20.0,
                        std_dev: 3.0,
                    },
                ),
                input(
                    "z",
                    Dist::Normal {
                        mean: -5.0,
                        std_dev: 1.0,
                    },
                ),
            ],
            outputs: vec![output("total", "$x + $y + $z")],
            sampling,
            ..MonteCarlo::default()
        };
        let n = 100_000;
        let r = run(&mc, 11, n);
        let total = &r.outputs[0];
        assert_eq!(r.trials, n);
        assert_eq!((total.valid, total.invalid), (n, 0));
        let sd = 14.0_f64.sqrt();
        let mean = get(total.mean);
        assert!(
            (mean - 25.0).abs() < 5.0 * sd / (n as f64).sqrt(),
            "{sampling:?}: {mean}"
        );
        // The standard deviation's standard error is about sd / √(2n).
        let got = get(total.std_dev);
        assert!(
            (got - sd).abs() < 5.0 * sd / (2.0 * n as f64).sqrt(),
            "{sampling:?}: {got}"
        );
        // The 95% interval of the mean is the textbook one, 1.96 sd / √n.
        let hw = get(total.half_width);
        assert!(
            (hw / (1.96 * got / (n as f64).sqrt()) - 1.0).abs() < 0.01,
            "{hw}"
        );
        // P95 of N(25, 14): 25 + 1.645 √14.
        let p95 = total
            .percentiles
            .iter()
            .find(|(p, _)| *p == 95.0)
            .map_or(f64::NAN, |(_, v)| *v);
        let want = 25.0 + normal_quantile(0.95).unwrap_or(f64::NAN) * sd;
        assert!(
            (p95 - want).abs() < 0.05,
            "{sampling:?}: P95 {p95} vs {want}"
        );
    }
}

/// The product of independent log-normals is log-normal: mean m₁m₂ and variance
/// (m₁² + s₁²)(m₂² + s₂²) − m₁²m₂², and its median is e^(μ₁ + μ₂).
#[test]
fn the_product_of_log_normals() {
    let (m1, s1, m2, s2) = (2.0_f64, 0.5_f64, 3.0_f64, 0.6_f64);
    let median = {
        let mu = |m: f64, s: f64| ln(m) - 0.5 * ln(1.0 + (s / m) * (s / m));
        exp(mu(m1, s1) + mu(m2, s2))
    };
    let mut product = output("product", "$a * $b");
    product.target = Some(McTarget::AtMost(median));
    let mc = MonteCarlo {
        inputs: vec![
            input(
                "a",
                Dist::LogNormal {
                    mean: m1,
                    std_dev: s1,
                },
            ),
            input(
                "b",
                Dist::LogNormal {
                    mean: m2,
                    std_dev: s2,
                },
            ),
        ],
        outputs: vec![product],
        ..MonteCarlo::default()
    };
    let n = 200_000;
    let r = run(&mc, 5, n);
    let p = &r.outputs[0];
    let mean = m1 * m2;
    let variance = (m1 * m1 + s1 * s1) * (m2 * m2 + s2 * s2) - mean * mean;
    assert!(
        (get(p.mean) - mean).abs() < 5.0 * (variance / n as f64).sqrt(),
        "{:?}",
        p.mean
    );
    // The sample variance of this log-normal (excess kurtosis about 1.9) has a relative
    // standard error of about √(3.9 / n), 0.44%; 5 of them.
    let got = get(p.std_dev) * get(p.std_dev);
    assert!((got / variance - 1.0).abs() < 0.025, "{got} vs {variance}");
    // Half the trials are at most the median, within 5 standard errors of a proportion.
    let t = p.target.unwrap_or_else(|| unreachable!());
    let share = get(t.probability);
    assert!(
        (share - 0.5).abs() < 5.0 * (0.25 / n as f64).sqrt(),
        "{share}"
    );
    assert_eq!(t.of, n);
    let (lo, hi) = t.interval.unwrap_or((f64::NAN, f64::NAN));
    assert!(lo < share && share < hi);
}

/// A PERT schedule on a single path: the finish is the sum of the tasks' PERT durations, so its
/// mean is the sum of their means (exactly, for the distributions as written) and its variance
/// the sum of the beta-PERT variances (b − a)² αβ / ((α + β)²(α + β + 1)), with
/// α = 1 + 4(m − a)/(b − a), β = 1 + 4(b − m)/(b − a). With six tasks the finish is close to
/// normal, as PERT assumes: about 90% of finishes fall by the mean plus 1.2816 standard
/// deviations.
#[test]
fn a_pert_schedule_on_a_single_path() {
    let tasks = [
        (2.0, 4.0, 8.0),
        (3.0, 5.0, 9.0),
        (1.0, 2.0, 4.0),
        (4.0, 6.0, 12.0),
        (2.0, 3.0, 5.0),
        (5.0, 8.0, 14.0),
    ];
    let mut inputs = Vec::new();
    let (mut mean, mut variance) = (0.0, 0.0);
    for (k, (a, m, b)) in tasks.iter().enumerate() {
        let d = ThreePoint {
            family: Family::Pert,
            min: *a,
            mode: *m,
            upper: *b,
            upper_is: Upper::Max,
        }
        .fit()
        .unwrap_or_else(|e| unreachable!("{e}"));
        mean += d.mean().unwrap_or(f64::NAN);
        let (al, be) = (1.0 + 4.0 * (m - a) / (b - a), 1.0 + 4.0 * (b - m) / (b - a));
        variance += (b - a) * (b - a) * al * be / ((al + be) * (al + be) * (al + be + 1.0));
        inputs.push(input(&format!("task{k}"), d));
    }
    let expression = (0..tasks.len())
        .map(|k| format!("$task{k}"))
        .collect::<Vec<_>>()
        .join(" + ");
    let mut finish = output("finish", &expression);
    let sd = variance.sqrt();
    finish.target = Some(McTarget::AtMost(mean + 1.281_551_565_5 * sd));
    let mc = MonteCarlo {
        inputs,
        outputs: vec![finish],
        ..MonteCarlo::default()
    };
    let n = 100_000;
    let r = run(&mc, 3, n);
    let f = &r.outputs[0];
    assert!(
        (get(f.mean) - mean).abs() < 5.0 * sd / (n as f64).sqrt(),
        "{:?} vs {mean}",
        f.mean
    );
    // Each task's written distribution is its PERT beta through quantiles at every 1%, which
    // keeps its variance to well within 1%; the sample variance's own error is about 0.5%.
    let got = get(f.std_dev) * get(f.std_dev);
    assert!((got / variance - 1.0).abs() < 0.04, "{got} vs {variance}");
    let share = get(f.target.and_then(|t| t.probability));
    assert!((share - 0.90).abs() < 0.015, "{share}");
    // Against the normal PERT assumes, the median is within a few hundredths of the mean.
    let p50 = f
        .percentiles
        .iter()
        .find(|(p, _)| *p == 50.0)
        .map_or(f64::NAN, |(_, v)| *v);
    assert!((normal_cdf((p50 - mean) / sd) - 0.5).abs() < 0.02, "{p50}");
}

fn correlated(sampling: Sampling, pairs: &[(&str, &str, f64)]) -> MonteCarlo {
    MonteCarlo {
        inputs: vec![
            input(
                "a",
                Dist::LogNormal {
                    mean: 1.0,
                    std_dev: 0.5,
                },
            ),
            input(
                "b",
                Dist::Gamma {
                    shape: 2.0,
                    scale: 1.0,
                },
            ),
            input("c", Dist::Uniform { min: 0.0, max: 1.0 }),
        ],
        outputs: vec![output("s", "$a + $b + $c")],
        correlations: pairs
            .iter()
            .map(|(a, b, r)| McCorrelation {
                a: (*a).into(),
                b: (*b).into(),
                rank: *r,
            })
            .collect(),
        sampling,
        stop: None,
    }
}

/// Iman and Conover's method gives the rank correlations asked for: within 0.02 over 20,000
/// trials (the spread from batch to batch is about 0.002 here), and leaves each input's own
/// values as drawn.
#[test]
fn induced_rank_correlations_meet_their_targets() {
    let pairs = [("a", "b", 0.8), ("a", "c", -0.5), ("b", "c", -0.3)];
    for sampling in [Sampling::Random, Sampling::LatinHypercube] {
        let r = run(&correlated(sampling, &pairs), 9, 20_000);
        for c in &r.correlations {
            let got = get(c.achieved);
            assert!(
                (got - c.target).abs() < 0.02,
                "{sampling:?} {}~{}: {got} vs {}",
                c.a,
                c.b,
                c.target
            );
        }
        // Without the correlations the same inputs are near independent.
        let free = run(&correlated(sampling, &[]), 9, 20_000);
        assert!(free.correlations.is_empty());
        // Reordering keeps each input's sample: the means are the same, to rounding.
        for (x, y) in r.inputs.iter().zip(&free.inputs) {
            assert!((get(x.mean) - get(y.mean)).abs() < 1e-9, "{}", x.name);
        }
    }
}

/// Latin hypercube sampling puts exactly one of a batch's n values in each of n equally likely
/// strata of every input, with or without correlations (which only reorder them).
#[test]
fn latin_hypercube_strata_hold_one_value_each() {
    let n = 500_u64;
    let pairs = [("a", "b", 0.6)];
    for pairs in [&pairs[..], &[]] {
        let mut mc = correlated(Sampling::LatinHypercube, pairs);
        mc.inputs[0] = input(
            "a",
            Dist::Normal {
                mean: 3.0,
                std_dev: 2.0,
            },
        );
        let c = compile(&mc, 4).unwrap_or_else(|p| unreachable!("{p:?}"));
        let block = c.block(BlockJob {
            index: 2,
            trials: n,
        });
        let cdfs: [&dyn Fn(f64) -> f64; 3] = [
            &|x: f64| normal_cdf((x - 3.0) / 2.0),
            &|x: f64| {
                stochcraft_random::fit::cdf(
                    &Dist::Gamma {
                        shape: 2.0,
                        scale: 1.0,
                    },
                    x,
                )
                .unwrap_or(f64::NAN)
            },
            &|x: f64| x,
        ];
        for (k, cdf) in cdfs.iter().enumerate() {
            let mut strata: Vec<u64> = block.inputs[k]
                .iter()
                .map(|x| (cdf(*x) * n as f64).floor().clamp(0.0, (n - 1) as f64) as u64)
                .collect();
            strata.sort_unstable();
            assert_eq!(strata, (0..n).collect::<Vec<_>>(), "input {k}");
        }
    }
}

/// Results do not depend on how the batches are made: one after another, in reverse order, or
/// on three threads.
#[test]
fn results_do_not_depend_on_threads() {
    let mc = correlated(Sampling::LatinHypercube, &[("a", "b", 0.7)]);
    let c = compile(&mc, 21).unwrap_or_else(|p| unreachable!("{p:?}"));
    let options = RunOptions {
        trials: 7_500,
        stop: None,
        percentiles: vec![1.0],
    };
    let serial = c.run(&options);
    let mut reversed = |jobs: &[BlockJob]| -> Result<Vec<Block>, ()> {
        let mut out: Vec<Block> = jobs.iter().rev().map(|j| c.block(*j)).collect();
        out.reverse();
        Ok(out)
    };
    assert_eq!(c.run_with(&options, &mut reversed), Ok(serial.clone()));
    let mut threads = |jobs: &[BlockJob]| -> Result<Vec<Block>, ()> {
        let parts: Vec<Vec<BlockJob>> = jobs.chunks(3).map(<[BlockJob]>::to_vec).collect();
        std::thread::scope(|s| {
            let handles: Vec<_> = parts
                .iter()
                .map(|part| s.spawn(|| part.iter().map(|j| c.block(*j)).collect::<Vec<_>>()))
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().map_err(|_| ()))
                .collect::<Result<Vec<_>, ()>>()
                .map(|v| v.into_iter().flatten().collect())
        })
    };
    assert_eq!(c.run_with(&options, &mut threads), Ok(serial.clone()));
    assert_eq!(serial.trials, 7_500);
    // Another seed gives other numbers.
    let other = compile(&mc, 22).map(|c| c.run(&options));
    assert_ne!(
        other.ok().map(|r| r.outputs[0].mean),
        Some(serial.outputs[0].mean)
    );
}

/// A sequential run stops once the mean is within the share asked for, after whole batches, and
/// gives exactly what a fixed run of that many trials gives.
#[test]
fn a_sequential_run_stops_when_the_mean_is_precise_enough() {
    let mc = MonteCarlo {
        inputs: vec![input("x", Dist::Exponential { mean: 10.0 })],
        outputs: vec![output("y", "$x * 2")],
        stop: Some(McStop {
            relative: 0.01,
            max: 1_000_000,
        }),
        ..MonteCarlo::default()
    };
    let c = compile(&mc, 2).unwrap_or_else(|p| unreachable!("{p:?}"));
    let r = c.run(&RunOptions {
        trials: 1_000,
        stop: mc.stop,
        percentiles: vec![],
    });
    let stop = r.stop.clone().unwrap_or_else(|| unreachable!());
    assert_eq!(stop.reason, stochcraft_stats::precision::Reason::TargetMet);
    // An exponential's half-width within 1% of its mean needs about (1.96 / 0.01)² = 38,416
    // trials.
    assert!(
        r.trials.is_multiple_of(BATCH) && r.trials > 30_000 && r.trials < 60_000,
        "{}",
        r.trials
    );
    let y = &r.outputs[0];
    assert!(get(y.half_width) <= 0.01 * get(y.mean));
    assert_eq!(stop.looks.last(), Some(&r.trials));
    let fixed = c.run(&RunOptions {
        trials: r.trials,
        stop: None,
        percentiles: vec![],
    });
    assert_eq!(fixed.outputs, r.outputs);
    // At its maximum it stops and says so.
    let capped = c.run(&RunOptions {
        trials: 1_000,
        stop: Some(McStop {
            relative: 0.0001,
            max: 2_500,
        }),
        percentiles: vec![],
    });
    assert_eq!(capped.trials, 2_500);
    assert_eq!(
        capped.stop.map(|s| s.reason),
        Some(stochcraft_stats::precision::Reason::MaxReached)
    );
}

/// The achieved correlation is measured the way the target is meant: Spearman's, on ranks.
#[test]
fn rank_correlation_is_spearmans() {
    let x = [1.0, 2.0, 3.0, 4.0, 5.0];
    let y = [1.0, 4.0, 9.0, 16.0, 100.0];
    assert!(rank_correlation(&x, &y).is_some_and(|r| (r - 1.0).abs() < 1e-12));
    let z = [5.0, 4.0, 3.0, 2.0, 1.0];
    assert!(rank_correlation(&x, &z).is_some_and(|r| (r + 1.0).abs() < 1e-12));
}
