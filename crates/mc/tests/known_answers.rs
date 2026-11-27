//! Monte Carlo models whose answers are known exactly (ROADMAP M4.1's checks).
//!
//! Each check allows five standard errors of the estimate, so a check of a correct
//! implementation fails about once in 1.7 million runs; the seeds are fixed anyway.

use stochcraft_mc::{
    BATCH, Block, BlockJob, Dist, IntervalBasis, McCorrelation, McInput, McOutput, McResults,
    McStop, McTarget, MonteCarlo, RunOptions, Sampling, compile, rank_correlation,
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
        controls: Vec::new(),
        role: Default::default(),
        summary: None,
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
        // With independent trials the 95% interval of the mean is the textbook one,
        // 1.96 sd / √n. (A Latin hypercube's comes from its batches, below.)
        let hw = get(total.half_width);
        if sampling == Sampling::Random {
            assert_eq!(r.interval, IntervalBasis::Trials);
            assert!(
                (hw / (1.96 * got / (n as f64).sqrt()) - 1.0).abs() < 0.01,
                "{hw}"
            );
        } else {
            assert_eq!(
                r.interval,
                IntervalBasis::Batches {
                    batches: 100,
                    size: BATCH
                }
            );
            // A sum is what Latin hypercube sampling estimates best: far narrower.
            assert!(hw < 0.1 * 1.96 * got / (n as f64).sqrt(), "{hw}");
        }
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
    // Latin hypercube runs are whole batches.
    assert_eq!((serial.trials, serial.requested_trials), (8_000, 7_500));
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

fn uniform(name: &str, min: f64, max: f64) -> McInput {
    input(name, Dist::Uniform { min, max })
}

/// A regression: under Latin hypercube sampling every batch of 1,000 draws of a uniform holds
/// exactly 500 at or below its median, so the share is judged across batches, not across the
/// trials as if they were independent (Wilson's interval would say 47% to 53%). The batches all
/// give the same share, so their spread gives no interval: none is claimed, rather than one of
/// no width.
#[test]
fn a_latin_hypercube_share_is_judged_across_batches() {
    let mut x = output("x", "$u");
    x.target = Some(McTarget::AtMost(0.5));
    let mc = MonteCarlo {
        inputs: vec![uniform("u", 0.0, 1.0)],
        outputs: vec![x],
        sampling: Sampling::LatinHypercube,
        ..MonteCarlo::default()
    };
    let r = run(&mc, 1, 3_000);
    assert_eq!(
        r.interval,
        IntervalBasis::Batches {
            batches: 3,
            size: BATCH
        }
    );
    let t = r.outputs[0].target.unwrap_or_else(|| unreachable!());
    assert_eq!(t.probability, Some(0.5));
    assert_eq!(t.interval, None);
    // Its mean's interval is across the three batch means too: narrow, but not none.
    let hw = get(r.outputs[0].half_width);
    assert!(hw > 0.0 && hw < 0.001, "{hw}");
}

/// Replicated Latin hypercube: the interval across batches covers the exact mean about as often
/// as it says. x y + z with x ~ U(0, 2), y ~ U(1, 3), z ~ U(-1, 1) has mean 1 × 2 + 0 = 2 and is
/// not additive, so Latin hypercube sampling does not remove all its variance. With four
/// batches the 95% interval should hold the mean in about 95 of 100 seeds; 88 is more than
/// three standard deviations of a binomial(100, 0.95) below that.
#[test]
fn latin_hypercube_intervals_cover_the_mean_at_their_rate() {
    let mc = MonteCarlo {
        inputs: vec![
            uniform("x", 0.0, 2.0),
            uniform("y", 1.0, 3.0),
            uniform("z", -1.0, 1.0),
        ],
        outputs: vec![output("s", "$x * $y + $z")],
        sampling: Sampling::LatinHypercube,
        ..MonteCarlo::default()
    };
    let covered = (1..=100_u64)
        .filter(|seed| {
            let r = run(&mc, *seed, 4 * BATCH);
            let o = &r.outputs[0];
            (get(o.mean) - 2.0).abs() <= get(o.half_width)
        })
        .count();
    assert!(covered >= 88, "{covered} of 100");
}

/// Correlated inputs reorder the trials within a batch, so plain random sampling with a
/// correlation is judged across batches too, and runs whole batches.
#[test]
fn correlated_random_sampling_is_judged_across_batches() {
    let r = run(&correlated(Sampling::Random, &[("a", "b", 0.5)]), 6, 2_500);
    assert_eq!((r.trials, r.requested_trials), (3_000, 2_500));
    assert_eq!(
        r.interval,
        IntervalBasis::Batches {
            batches: 3,
            size: BATCH
        }
    );
    assert!(r.outputs[0].half_width.is_some());
}

/// With fewer than two batches there is nothing to judge the batches' spread by: no interval.
#[test]
fn one_batch_gives_no_interval() {
    let mut x = output("x", "$u * 2");
    x.target = Some(McTarget::AtLeast(1.0));
    let mc = MonteCarlo {
        inputs: vec![uniform("u", 0.0, 1.0)],
        outputs: vec![x],
        sampling: Sampling::LatinHypercube,
        ..MonteCarlo::default()
    };
    let r = run(&mc, 1, 400);
    assert_eq!((r.trials, r.requested_trials), (1_000, 400));
    let o = &r.outputs[0];
    assert!(o.mean.is_some());
    assert_eq!(o.half_width, None);
    let t = o.target.unwrap_or_else(|| unreachable!());
    assert!(t.probability.is_some());
    assert_eq!(t.interval, None);
}

/// Plain random sampling with no correlations keeps its trials independent: the interval is
/// across trials, Wilson's for a share, and any number of trials runs as asked.
#[test]
fn independent_trials_keep_per_trial_intervals() {
    let mut x = output("x", "$u");
    x.target = Some(McTarget::AtMost(0.5));
    let mc = MonteCarlo {
        inputs: vec![uniform("u", 0.0, 1.0)],
        outputs: vec![x],
        ..MonteCarlo::default()
    };
    let r = run(&mc, 1, 2_500);
    assert_eq!((r.trials, r.interval), (2_500, IntervalBasis::Trials));
    let t = r.outputs[0].target.unwrap_or_else(|| unreachable!());
    let wilson = stochcraft_stats::Proportion::new(t.met, t.of).wilson(0.95);
    assert_eq!(t.interval, wilson);
}

fn first_influence(r: &McResults) -> &stochcraft_mc::Influence {
    r.outputs
        .first()
        .map(|o| &o.influence)
        .unwrap_or_else(|| unreachable!("no output"))
}

fn entry<'a>(i: &'a stochcraft_mc::Influence, name: &str) -> &'a stochcraft_mc::InputInfluence {
    i.inputs
        .iter()
        .find(|e| e.input == name)
        .unwrap_or_else(|| unreachable!("no input {name}"))
}

fn names(i: &stochcraft_mc::Influence) -> Vec<&str> {
    i.inputs.iter().map(|e| e.input.as_str()).collect()
}

/// y = 2a + b + k with a uniform on 0 to 1, b on 0 to 10 and k a constant: the tornado moves one
/// input between its 10th and 90th percentiles with the other at its median, so each swing is
/// exactly the coefficient times the input's 10th-to-90th spread (a uniform's is 0.8 of its
/// range): 1.6 for a and 8 for b. The constant is left out, b is first, and the base has every
/// input at its median: 1 + 5 + 3. The rank correlations (y is mostly b) follow the same order.
#[test]
fn a_linear_models_swings_are_each_coefficient_times_the_spread() {
    let a = Dist::Uniform { min: 0.0, max: 1.0 };
    let b = Dist::Uniform {
        min: 0.0,
        max: 10.0,
    };
    let mc = MonteCarlo {
        inputs: vec![
            input("a", a.clone()),
            input("b", b.clone()),
            input("k", Dist::Constant(3.0)),
        ],
        outputs: vec![output("y", "2 * $a + $b + $k")],
        ..MonteCarlo::default()
    };
    let r = run(&mc, 1, 5_000);
    let inf = first_influence(&r);
    assert_eq!(names(inf), ["b", "a"]);
    assert!((get(inf.base) - 9.0).abs() < 1e-12, "{:?}", inf.base);
    let spread = |d: &Dist| {
        stochcraft_mc::quantile::quantile(d, 0.9) - stochcraft_mc::quantile::quantile(d, 0.1)
    };
    let (ea, eb) = (entry(inf, "a"), entry(inf, "b"));
    assert!((get(ea.swing()) - 2.0 * spread(&a)).abs() < 1e-12);
    assert!((get(eb.swing()) - spread(&b)).abs() < 1e-12);
    assert!((get(ea.swing()) - 1.6).abs() < 1e-12);
    assert!((get(eb.swing()) - 8.0).abs() < 1e-12);
    assert!((get(ea.low) - 8.2).abs() < 1e-12 && (get(ea.high) - 9.8).abs() < 1e-12);
    assert!((get(eb.low) - 5.0).abs() < 1e-12 && (get(eb.high) - 13.0).abs() < 1e-12);
    assert!((ea.p10 - 0.1).abs() < 1e-12 && (eb.p90 - 9.0).abs() < 1e-12);
    // Spearman: b carries almost all of y's variance (about 0.98); a about 0.2.
    assert!(get(eb.rank_correlation) > 0.9, "{eb:?}");
    assert!(
        (0.1..0.3).contains(&get(ea.rank_correlation)),
        "{:?}",
        ea.rank_correlation
    );
}

/// y = a − 2b + 0c with independent standard normals: the rank correlation of a normal output
/// with each input is (6/π) asin(r/2) for the Pearson r, so 0.43 for a, −0.89 for b and 0 for c,
/// with the sign and the order of the effects; the tornado agrees (b swings twice a, c none).
#[test]
fn rank_correlations_have_the_sign_and_order_of_the_effects() {
    let normal = Dist::Normal {
        mean: 0.0,
        std_dev: 1.0,
    };
    let mc = MonteCarlo {
        inputs: ["a", "b", "c"]
            .iter()
            .map(|n| input(n, normal.clone()))
            .collect(),
        outputs: vec![output("y", "$a - 2 * $b + 0 * $c")],
        ..MonteCarlo::default()
    };
    let r = run(&mc, 3, 20_000);
    let inf = first_influence(&r);
    assert_eq!(names(inf), ["b", "a", "c"]);
    let rho = |n: &str| get(entry(inf, n).rank_correlation);
    assert!((0.39..0.47).contains(&rho("a")), "{}", rho("a"));
    assert!((-0.92..-0.85).contains(&rho("b")), "{}", rho("b"));
    assert!(rho("c").abs() < 0.05, "{}", rho("c"));
    let swing = |n: &str| get(entry(inf, n).swing());
    assert!((swing("b") - 2.0 * swing("a")).abs() < 1e-12);
    assert!(swing("c").abs() < 1e-12);
}

/// The tornado is worked out from the distributions, with no sampling, so it is the same
/// however the trials were drawn.
#[test]
fn the_tornado_does_not_depend_on_the_sampling() {
    let make = |sampling| MonteCarlo {
        inputs: vec![
            input(
                "a",
                Dist::LogNormal {
                    mean: 1.0,
                    std_dev: 0.5,
                },
            ),
            uniform("b", 0.0, 3.0),
        ],
        outputs: vec![output("y", "$a * $b")],
        sampling,
        ..MonteCarlo::default()
    };
    let random = run(&make(Sampling::Random), 1, 2_000);
    let lhs = run(&make(Sampling::LatinHypercube), 9, 3_000);
    let tornado = |r: &McResults| {
        first_influence(r)
            .inputs
            .iter()
            .map(|e| (e.input.clone(), e.low, e.high, e.p10, e.p90))
            .collect::<Vec<_>>()
    };
    assert_eq!(tornado(&random), tornado(&lhs));
    assert_eq!(first_influence(&random).base, first_influence(&lhs).base);
}

/// The rank correlation of a correlated input includes the effect of the inputs it is correlated
/// with: b has no part in y = a, and its tornado bar is nothing, but it goes with a (rank
/// correlation 0.8), so it goes with y too.
#[test]
fn a_correlated_inputs_rank_correlation_includes_its_partners() {
    let mc = MonteCarlo {
        inputs: vec![uniform("a", 0.0, 1.0), uniform("b", 0.0, 1.0)],
        outputs: vec![output("y", "$a + 0 * $b")],
        correlations: vec![McCorrelation {
            a: "a".into(),
            b: "b".into(),
            rank: 0.8,
        }],
        ..MonteCarlo::default()
    };
    let r = run(&mc, 1, 5_000);
    let inf = first_influence(&r);
    let b = entry(inf, "b");
    assert!(get(b.swing()).abs() < 1e-12);
    assert!(
        (get(b.rank_correlation) - 0.8).abs() < 0.05,
        "{:?}",
        b.rank_correlation
    );
    assert!(get(entry(inf, "a").rank_correlation) > 0.99);
}

/// An output with no number in some trials is ranked over the trials where it has one, and a
/// swing with an end that is no number has no width and goes last: y is b where a is above
/// −0.5, and a division by zero below, so a moved to its 10th percentile (−0.8) gives no number.
#[test]
fn trials_with_no_number_are_left_out_of_the_ranks() {
    let mc = MonteCarlo {
        inputs: vec![uniform("a", -1.0, 1.0), uniform("b", 0.0, 1.0)],
        outputs: vec![output("y", "if($a > -0.5, $b, 1 div 0)")],
        ..MonteCarlo::default()
    };
    let r = run(&mc, 2, 10_000);
    let inf = first_influence(&r);
    let invalid = r.outputs.first().map_or(0, |o| o.invalid);
    assert!(invalid > 2_000 && invalid < 3_000, "{invalid}");
    assert!((get(inf.base) - 0.5).abs() < 1e-12);
    assert_eq!(names(inf), ["b", "a"]);
    let a = entry(inf, "a");
    assert_eq!((a.low, a.swing()), (None, None));
    assert!((get(a.high) - 0.5).abs() < 1e-12);
    // Over the trials with a number, y is b and says nothing of a.
    assert!(get(entry(inf, "b").rank_correlation) > 0.999);
    assert!(
        get(a.rank_correlation).abs() < 0.06,
        "{:?}",
        a.rank_correlation
    );
}
