//! Antithetic pairs and randomised Sobol points (ROADMAP M4.1 part 2b): models whose answers are
//! known exactly, the published first points of Joe and Kuo's Sobol sequence, and how often each
//! mode's 95% interval holds the exact mean.
//!
//! The checks that one run's interval holds the exact mean allow three half-widths, not one, so
//! that a correct implementation does not fail on a seed (a one-in-twenty event); how often the
//! interval holds the mean is checked over many seeds instead.

use std::collections::BTreeSet;

use stochcraft_mc::qmc;
use stochcraft_mc::{
    BATCH, BlockJob, Dist, IntervalBasis, McInput, McOutput, McResults, MonteCarlo, RunOptions,
    Sampling, check, compile,
};

fn input(name: &str, dist: Dist) -> McInput {
    McInput {
        name: name.into(),
        dist,
        unit: String::new(),
    }
}

fn uniform(name: &str, min: f64, max: f64) -> McInput {
    input(name, Dist::Uniform { min, max })
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

fn model(inputs: Vec<McInput>, expression: &str, sampling: Sampling) -> MonteCarlo {
    MonteCarlo {
        inputs,
        outputs: vec![output("y", expression)],
        sampling,
        ..MonteCarlo::default()
    }
}

/// The product of (1 + (u_i - 0.5) / i) over five uniform inputs: smooth, with mean exactly 1
/// (the factors are independent and each has mean 1) and a variance of about 0.126.
fn product() -> (Vec<McInput>, &'static str) {
    (
        (1..=5)
            .map(|i| uniform(&format!("u{i}"), 0.0, 1.0))
            .collect(),
        "(1 + ($u1 - 0.5) div 1) * (1 + ($u2 - 0.5) div 2) * (1 + ($u3 - 0.5) div 3) * (1 + ($u4 - 0.5) div 4) * (1 + ($u5 - 0.5) div 5)",
    )
}

/// The first ten points of the first eight dimensions of the Sobol sequence of Joe and Kuo's
/// "new-joe-kuo-6.21201" in Gray-code order. The first three dimensions are the ten points the
/// authors publish on their page (https://web.maths.unsw.edu.au/~fkuo/sobol/) for
/// `./sobol 10 3 new-joe-kuo-6.21201`; the others were worked out with a separate
/// implementation of the recurrence of their paper, written for this check.
const FIRST_POINTS: [[f64; 8]; 10] = [
    [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5],
    [0.75, 0.25, 0.25, 0.25, 0.75, 0.75, 0.25, 0.75],
    [0.25, 0.75, 0.75, 0.75, 0.25, 0.25, 0.75, 0.25],
    [0.375, 0.375, 0.625, 0.875, 0.375, 0.125, 0.375, 0.875],
    [0.875, 0.875, 0.125, 0.375, 0.875, 0.625, 0.875, 0.375],
    [0.625, 0.125, 0.875, 0.625, 0.625, 0.875, 0.125, 0.125],
    [0.125, 0.625, 0.375, 0.125, 0.125, 0.375, 0.625, 0.625],
    [
        0.1875, 0.3125, 0.9375, 0.4375, 0.5625, 0.3125, 0.4375, 0.9375,
    ],
    [
        0.6875, 0.8125, 0.4375, 0.9375, 0.0625, 0.8125, 0.9375, 0.4375,
    ],
];

#[test]
fn the_first_sobol_points_are_the_published_ones() {
    for dim in 0..8 {
        let points = qmc::points(dim, 10).unwrap_or_default();
        let want: Vec<f64> = FIRST_POINTS.iter().map(|p| p[dim]).collect();
        assert_eq!(points, want, "dimension {}", dim + 1);
    }
}

/// Every dimension of a Sobol sequence is a (0, 1)-sequence: its first 2^m points, cut into 2^m
/// equal intervals, have one in each. A wrong direction number breaks it for some dimension.
#[test]
fn every_dimension_has_one_point_in_each_interval() {
    for dim in 0..qmc::MAX_DIMENSIONS {
        let ints = qmc::integers(dim, 1024).unwrap_or_default();
        assert_eq!(ints.len(), 1024);
        let cells: BTreeSet<u32> = ints.iter().map(|x| x >> 22).collect();
        assert_eq!(cells.len(), 1024, "dimension {}", dim + 1);
    }
    assert!(qmc::integers(qmc::MAX_DIMENSIONS, 1).is_none());
    assert!(qmc::points(qmc::MAX_DIMENSIONS, 1).is_none());
}

/// A shifted point is strictly inside (0, 1), and the shift moves every point of a dimension the
/// same way (an exclusive or), so the points still fill the intervals evenly.
#[test]
fn a_shift_keeps_the_points_inside_and_even() {
    for shift in [0, 1, 0x8000_0000, u32::MAX, 0x1234_5678] {
        let ps = qmc::shifted(3, 1024, shift).unwrap_or_default();
        assert!(ps.iter().all(|p| *p > 0.0 && *p < 1.0), "{shift}");
        let cells: BTreeSet<usize> = ps.iter().map(|p| (p * 1024.0) as usize).collect();
        assert_eq!(cells.len(), 1024, "{shift}");
    }
}

/// exp(U) for U uniform on (0, 1) rises with U, so antithetic pairs err in opposite directions:
/// the interval across pair means holds the exact mean e - 1 and is far narrower than random
/// sampling's with the same trials (the pair mean has variance 0.0039 where two independent
/// trials' mean has 0.121).
#[test]
fn antithetic_pairs_narrow_the_interval_of_a_monotone_function() {
    let exact = 1.718_281_828_459_045;
    let inputs = || vec![uniform("u", 0.0, 1.0)];
    let random = run(&model(inputs(), "exp($u)", Sampling::Random), 7, 10_000);
    let anti = run(&model(inputs(), "exp($u)", Sampling::Antithetic), 7, 10_000);
    assert_eq!(anti.interval, IntervalBasis::Pairs { pairs: 5_000 });
    assert_eq!(anti.trials, 10_000);
    let (r, a) = (&random.outputs[0], &anti.outputs[0]);
    let (rh, ah) = (get(r.half_width), get(a.half_width));
    assert!((get(r.mean) - exact).abs() <= 3.0 * rh, "{}", get(r.mean));
    assert!((get(a.mean) - exact).abs() <= 3.0 * ah, "{}", get(a.mean));
    assert!(ah < 0.5 * rh, "antithetic {ah} against random {rh}");
    assert_eq!(a.interval_count, 5_000);
}

/// A smooth integrand of few inputs, where Sobol points do much better than random ones: the
/// interval across batches holds the exact mean 1 and is far narrower.
#[test]
fn sobol_points_narrow_the_interval_of_a_smooth_function() {
    let (inputs, expression) = product();
    let random = run(
        &model(inputs.clone(), expression, Sampling::Random),
        3,
        10_000,
    );
    let sobol = run(&model(inputs, expression, Sampling::Sobol), 3, 10_000);
    assert_eq!(
        sobol.interval,
        IntervalBasis::Batches {
            batches: 10,
            size: BATCH
        }
    );
    let (r, s) = (&random.outputs[0], &sobol.outputs[0]);
    let (rh, sh) = (get(r.half_width), get(s.half_width));
    assert!((get(r.mean) - 1.0).abs() <= 3.0 * rh, "{}", get(r.mean));
    assert!((get(s.mean) - 1.0).abs() <= 3.0 * sh, "{}", get(s.mean));
    assert!(sh < 0.25 * rh, "sobol {sh} against random {rh}");
}

/// Trial 2j + 1 of an antithetic run takes the opposite point of every input's range from trial
/// 2j: for U(0, 10) the pair's values add to 10, and for an exponential the two come from
/// probabilities u and 1 - u.
#[test]
fn antithetic_trials_come_in_pairs() {
    let mc = model(
        vec![
            uniform("a", 0.0, 10.0),
            input("b", Dist::Exponential { mean: 2.0 }),
        ],
        "$a + $b",
        Sampling::Antithetic,
    );
    let c = compile(&mc, 9).unwrap_or_else(|p| unreachable!("{p:?}"));
    assert!(c.paired() && !c.batched());
    let block = c.block(BlockJob {
        index: 0,
        trials: 1_000,
    });
    for pair in block.inputs[0].chunks(2) {
        assert!((pair[0] + pair[1] - 10.0).abs() < 1e-9, "{pair:?}");
    }
    // Exponential: x = -2 ln(1 - u), so (1 - e^(-x1 / 2)) + (1 - e^(-x2 / 2)) = 1.
    for pair in block.inputs[1].chunks(2) {
        let (u0, u1) = (
            1.0 - stochcraft_random::math::exp(-pair[0] / 2.0),
            1.0 - stochcraft_random::math::exp(-pair[1] / 2.0),
        );
        assert!((u0 + u1 - 1.0).abs() < 1e-9, "{pair:?}");
    }
    // An odd number of trials is made whole: pairs are never split.
    assert_eq!(c.trials_for(7), 8);
    assert_eq!(c.trials_for(1), 2);
}

/// The same seed gives the same run, a different seed a different one, in both modes (the
/// scramble itself is checked in `tests/scramble.rs`).
#[test]
fn runs_are_reproducible_by_seed() {
    let (inputs, expression) = product();
    for sampling in [Sampling::Antithetic, Sampling::Sobol] {
        let mc = model(inputs.clone(), expression, sampling);
        let a = run(&mc, 21, 2_000);
        assert_eq!(a, run(&mc, 21, 2_000), "{sampling:?}");
        assert_ne!(
            a.outputs[0].mean,
            run(&mc, 22, 2_000).outputs[0].mean,
            "{sampling:?}"
        );
    }
}

/// An input added after the others leaves the others' values as they were, in both modes: each
/// input's draws come from a stream of its own, and a Sobol input's dimension is its place in
/// the declaration order.
#[test]
fn adding_an_input_leaves_the_others_values_unchanged() {
    for sampling in [Sampling::Antithetic, Sampling::Sobol] {
        let two = vec![
            uniform("a", 0.0, 1.0),
            input(
                "b",
                Dist::Normal {
                    mean: 5.0,
                    std_dev: 2.0,
                },
            ),
        ];
        let mut three = two.clone();
        three.push(input("c", Dist::Exponential { mean: 3.0 }));
        let job = BlockJob {
            index: 2,
            trials: 1_000,
        };
        let few = compile(&model(two, "$a + $b", sampling), 5)
            .unwrap_or_else(|p| unreachable!("{p:?}"))
            .block(job);
        let more = compile(&model(three, "$a + $b + $c", sampling), 5)
            .unwrap_or_else(|p| unreachable!("{p:?}"))
            .block(job);
        assert_eq!(few.inputs[0], more.inputs[0], "{sampling:?}");
        assert_eq!(few.inputs[1], more.inputs[1], "{sampling:?}");
        assert_eq!(more.inputs.len(), 3);
        // Another batch of the same run has other draws.
        let other = compile(&model(vec![uniform("a", 0.0, 1.0)], "$a", sampling), 5)
            .unwrap_or_else(|p| unreachable!("{p:?}"))
            .block(BlockJob {
                index: 3,
                trials: 1_000,
            });
        assert_ne!(few.inputs[0], other.inputs[0], "{sampling:?}");
    }
}

/// Each Sobol batch is the same points under a scramble and shift of its own, so the batches
/// differ, yet each keeps the even spread: of a uniform input's 1,000 values, the first 512 of the
/// sequence fall one in each 512th of the range (scrambled or not) and the other 488, part of the
/// next 512, add at most one more to a cell, so every cell holds one or two.
#[test]
fn each_sobol_batch_is_shifted_on_its_own() {
    let mc = model(vec![uniform("u", 0.0, 1.0)], "$u", Sampling::Sobol);
    let c = compile(&mc, 4).unwrap_or_else(|p| unreachable!("{p:?}"));
    let a = c.block(BlockJob {
        index: 0,
        trials: 1_000,
    });
    let b = c.block(BlockJob {
        index: 1,
        trials: 1_000,
    });
    assert_ne!(a.inputs[0], b.inputs[0]);
    for block in [&a, &b] {
        let mut cells = vec![0_u32; 512];
        for x in &block.inputs[0] {
            cells[(x * 512.0) as usize] += 1;
        }
        assert!(cells.iter().all(|n| (1..=2).contains(n)), "{cells:?}");
    }
}

/// Antithetic sampling is refused with correlated inputs, which reorder the trials and so break
/// the pairs; Sobol sampling with more inputs than it has points for; a sequential stop needs a
/// most of whole batches (Sobol) or of a pair (antithetic).
#[test]
fn what_each_mode_refuses() {
    let codes =
        |mc: &MonteCarlo| -> Vec<&'static str> { check(mc).iter().map(|p| p.code).collect() };
    let mut mc = model(
        vec![uniform("a", 0.0, 1.0), uniform("b", 0.0, 1.0)],
        "$a + $b",
        Sampling::Antithetic,
    );
    mc.correlations.push(stochcraft_mc::McCorrelation {
        a: "a".into(),
        b: "b".into(),
        rank: 0.5,
    });
    assert!(codes(&mc).contains(&"antithetic_correlated"));
    assert!(compile(&mc, 1).is_err());
    for sampling in [Sampling::Random, Sampling::LatinHypercube, Sampling::Sobol] {
        mc.sampling = sampling;
        assert!(
            !codes(&mc).contains(&"antithetic_correlated"),
            "{sampling:?}"
        );
    }

    let many: Vec<McInput> = (0..=qmc::MAX_DIMENSIONS)
        .map(|i| uniform(&format!("x{i}"), 0.0, 1.0))
        .collect();
    let sum = (0..=qmc::MAX_DIMENSIONS)
        .map(|i| format!("$x{i}"))
        .collect::<Vec<_>>()
        .join(" + ");
    let mut big = model(many, &sum, Sampling::Sobol);
    assert!(codes(&big).contains(&"too_many_inputs"));
    assert!(compile(&big, 1).is_err());
    big.sampling = Sampling::Random;
    assert!(!codes(&big).contains(&"too_many_inputs"));
    big.inputs.pop();
    big.outputs = vec![output(
        "y",
        &(0..qmc::MAX_DIMENSIONS)
            .map(|i| format!("$x{i}"))
            .collect::<Vec<_>>()
            .join(" + "),
    )];
    big.sampling = Sampling::Sobol;
    assert!(compile(&big, 1).is_ok(), "exactly 64 inputs are allowed");

    let stop = |max| {
        Some(stochcraft_mc::McStop {
            relative: 0.01,
            max,
        })
    };
    let mut mc = model(vec![uniform("a", 0.0, 1.0)], "$a", Sampling::Sobol);
    mc.stop = stop(999);
    assert!(codes(&mc).contains(&"bad_stop"));
    mc.stop = stop(1_000);
    assert!(!codes(&mc).contains(&"bad_stop"));
    mc.sampling = Sampling::Antithetic;
    mc.stop = stop(1);
    assert!(codes(&mc).contains(&"bad_stop"));
    mc.stop = stop(2);
    assert!(!codes(&mc).contains(&"bad_stop"));
}

/// A sequential run with antithetic pairs or Sobol points ends, in whole pairs or batches, with
/// the precision asked for judged on the interval the results report.
#[test]
fn sequential_runs_end_in_whole_pairs_and_batches() {
    let (inputs, expression) = product();
    for (sampling, unit) in [(Sampling::Antithetic, 2), (Sampling::Sobol, BATCH)] {
        let mut mc = model(inputs.clone(), expression, sampling);
        mc.stop = Some(stochcraft_mc::McStop {
            relative: 0.001,
            max: 20_001,
        });
        let c = compile(&mc, 8).unwrap_or_else(|p| unreachable!("{p:?}"));
        let r = c.run(&RunOptions {
            trials: 1_000,
            stop: mc.stop,
            percentiles: vec![],
        });
        assert_eq!(r.trials % unit, 0, "{sampling:?}: {}", r.trials);
        assert!(r.trials <= 20_001, "{sampling:?}: {}", r.trials);
        let stop = r.stop.unwrap_or_else(|| unreachable!("a sequential run"));
        assert_eq!(stop.looks.first(), Some(&1_000), "{sampling:?}");
        let o = &r.outputs[0];
        let hw = get(o.half_width);
        // Stopped because the mean is within ±0.1% of itself, or at the most.
        assert!(
            hw <= 0.001 * get(o.mean) || r.trials >= 20_000,
            "{sampling:?}: ± {hw} after {}",
            r.trials
        );
    }
}

/// How often the 95% interval holds the exact mean, over many seeds: x y + z with x ~ U(0, 2),
/// y ~ U(1, 3), z ~ U(-1, 1) has mean 1 × 2 + 0 = 2 and is not monotone in every input, so
/// neither mode removes all its variance. A binomial(200, 0.95) count has a standard deviation
/// of 3.1 and a mean of 190; 176 is more than four standard deviations below it.
#[test]
fn the_intervals_of_both_modes_cover_the_mean_at_their_rate() {
    let inputs = vec![
        uniform("a", 0.0, 2.0),
        uniform("b", 1.0, 3.0),
        uniform("c", -1.0, 1.0),
    ];
    for (sampling, trials) in [(Sampling::Antithetic, 2_000), (Sampling::Sobol, 8 * BATCH)] {
        let mc = model(inputs.clone(), "$a * $b + $c", sampling);
        let covered = (1..=200_u64)
            .filter(|seed| {
                let r = run(&mc, *seed, trials);
                let o = &r.outputs[0];
                (get(o.mean) - 2.0).abs() <= get(o.half_width)
            })
            .count();
        assert!(covered >= 176, "{sampling:?}: {covered} of 200");
    }
}
