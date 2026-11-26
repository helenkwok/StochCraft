//! Control variates (ROADMAP M4.1; Law, *Simulation Modeling and Analysis*, 5th ed., §11.3).
//!
//! The answers are known exactly: for Y = X + Z with X and Z independent, controlling X leaves
//! the variance of Z, so the controlled estimate's variance is Var(Z) / Var(X + Z) of the plain
//! one. The seeds are fixed.

use stochcraft_mc::control::{exact_mean, is_constant};
use stochcraft_mc::{
    BATCH, BlockJob, Control, ControlledEstimate, Dist, McInput, McOutput, McResults, McTarget,
    MonteCarlo, RunOptions, Sampling, check, compile,
};

fn input(name: &str, dist: Dist) -> McInput {
    McInput {
        name: name.into(),
        dist,
        unit: String::new(),
    }
}

fn normal(name: &str, mean: f64, std_dev: f64) -> McInput {
    input(name, Dist::Normal { mean, std_dev })
}

fn uniform(name: &str, min: f64, max: f64) -> McInput {
    input(name, Dist::Uniform { min, max })
}

fn output(name: &str, expression: &str, controls: Vec<Control>) -> McOutput {
    McOutput {
        name: name.into(),
        expression: expression.into(),
        unit: String::new(),
        target: None,
        controls,
        role: Default::default(),
        summary: None,
    }
}

fn by_input(name: &str) -> Control {
    Control::Input { name: name.into() }
}

fn run(mc: &MonteCarlo, seed: u64, trials: u64) -> McResults {
    let c = compile(mc, seed).unwrap_or_else(|p| unreachable!("{p:?}"));
    c.run(&RunOptions {
        trials,
        stop: None,
        percentiles: vec![],
    })
}

fn estimate(r: &McResults, o: usize) -> &ControlledEstimate {
    r.outputs[o]
        .controlled
        .as_ref()
        .and_then(|c| c.estimate.as_ref())
        .unwrap_or_else(|| unreachable!("output {o} has no controlled estimate"))
}

fn error_codes(mc: &MonteCarlo) -> Vec<&'static str> {
    check(mc)
        .iter()
        .filter(|p| p.error)
        .map(|p| p.code)
        .collect()
}

/// The mean and the standard error of the mean of input `x`'s draws over `batches` batches.
fn drawn_mean(d: &Dist, sampling: Sampling, batches: u64) -> (f64, f64) {
    let mc = MonteCarlo {
        inputs: vec![input("x", d.clone())],
        outputs: vec![output("y", "$x", vec![])],
        sampling,
        ..MonteCarlo::default()
    };
    let c = compile(&mc, 99).unwrap_or_else(|p| unreachable!("{p:?}"));
    let values: Vec<f64> = (0..batches)
        .flat_map(|index| {
            c.block(BlockJob {
                index,
                trials: BATCH,
            })
            .inputs
            .remove(0)
        })
        .collect();
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (n - 1.0);
    (mean, (var / n).sqrt())
}

fn allowed() -> Vec<Dist> {
    vec![
        Dist::Exponential { mean: 3.0 },
        Dist::Uniform {
            min: -2.0,
            max: 5.0,
        },
        Dist::Triangular {
            min: 1.0,
            mode: 2.0,
            max: 6.0,
        },
        // Normal inputs can be negative: nothing is clamped.
        Dist::Normal {
            mean: -1.0,
            std_dev: 2.0,
        },
        Dist::TruncatedNormal {
            mean: 0.0,
            std_dev: 1.0,
            min: -1.0,
            max: 2.5,
        },
        Dist::LogNormal {
            mean: 5.0,
            std_dev: 3.0,
        },
        Dist::Gamma {
            shape: 2.5,
            scale: 1.5,
        },
        Dist::Gamma {
            shape: 0.4,
            scale: 2.0,
        },
        Dist::Beta {
            shape: 2.0,
            scale: 5.0,
        },
        Dist::Erlang { mean: 4.0, k: 3 },
        Dist::Weibull {
            shape: 1.7,
            scale: 3.0,
        },
        Dist::Poisson { mean: 3.5 },
        Dist::Poisson { mean: 40.0 },
        Dist::Poisson { mean: 1e7 },
        Dist::Binomial {
            probability: 0.3,
            trials: 20,
        },
        Dist::Binomial {
            probability: 0.9,
            trials: 1_000,
        },
        Dist::User {
            points: vec![(0.2, 1.0), (0.5, 4.0), (0.3, 10.0)],
            discrete: true,
        },
        Dist::User {
            points: vec![(0.0, 0.0), (0.5, 2.0), (0.5, 10.0)],
            discrete: false,
        },
    ]
}

/// For every distribution a control may be, the mean of what a model draws is the mean it
/// states, within four standard errors over 200,000 trials; with the other samplings (whose
/// quantile functions draw it) over 10,000.
#[test]
fn the_means_of_the_inputs_allowed_as_controls_are_exact() {
    for d in allowed() {
        let stated = exact_mean(&d).unwrap_or_else(|why| unreachable!("{d:?}: {why}"));
        assert!(!is_constant(&d), "{d:?}");
        for (sampling, batches) in [
            (Sampling::Random, 200),
            (Sampling::LatinHypercube, 10),
            (Sampling::Antithetic, 10),
            (Sampling::Sobol, 10),
        ] {
            let (mean, se) = drawn_mean(&d, sampling, batches);
            assert!(
                (mean - stated).abs() <= 4.0 * se + 1e-9,
                "{d:?} under {sampling:?}: drew {mean}, stated {stated}, standard error {se}"
            );
        }
    }
}

/// A count with a large spread is drawn by a quantile expansion under the stratified samplings,
/// so it is refused as a control, and so is what has no spread.
#[test]
fn what_is_not_exact_or_has_no_spread_is_refused() {
    for d in [
        Dist::Poisson { mean: 1e9 },
        Dist::Binomial {
            probability: 0.5,
            trials: 1_000_000_000,
        },
    ] {
        let why = exact_mean(&d).err().unwrap_or_default();
        assert!(why.contains("standard deviation"), "{d:?}: {why:?}");
    }
    for d in [
        Dist::Constant(3.0),
        Dist::Normal {
            mean: 1.0,
            std_dev: 0.0,
        },
        Dist::Exponential { mean: 0.0 },
        Dist::Binomial {
            probability: 1.0,
            trials: 10,
        },
    ] {
        assert!(is_constant(&d), "{d:?}");
    }
}

/// Y = X + Z with independent normals, X the control: b is 1, the interval holds the true mean
/// about 95% of the time, and the variance of the estimate is Var(Z) / Var(X + Z) = 1/10 of the
/// plain one (the ratio, averaged over seeds, within 15%).
#[test]
fn controlling_one_of_two_independent_parts_leaves_the_other() {
    let mc = MonteCarlo {
        inputs: vec![normal("x", 10.0, 3.0), normal("z", 5.0, 1.0)],
        outputs: vec![output("y", "$x + $z", vec![by_input("x")])],
        ..MonteCarlo::default()
    };
    let mut ratios = Vec::new();
    for seed in 1..=40 {
        let r = run(&mc, seed, 20_000);
        let e = estimate(&r, 0);
        let c = r.outputs[0].controlled.as_ref();
        let b = c.map_or(f64::NAN, |c| c.controls[0].coefficient);
        assert!((b - 1.0).abs() < 0.06, "seed {seed}: b {b}");
        assert_eq!(c.map(|c| c.pilot_trials), Some(1_000));
        assert_eq!(e.trials, 19_000);
        ratios.push(e.variance_ratio.unwrap_or(f64::NAN));
    }
    let ratio = ratios.iter().sum::<f64>() / ratios.len() as f64;
    assert!(
        (ratio / 10.0 - 1.0).abs() < 0.15,
        "variance ratio {ratio}, expected 10"
    );
    // Coverage of the true mean, 15, over 200 seeds.
    let covered = (1..=200_u64)
        .filter(|seed| {
            let r = run(&mc, *seed, 4_000);
            let e = estimate(&r, 0);
            (e.mean - 15.0).abs() <= e.half_width.unwrap_or(0.0)
        })
        .count();
    assert!(covered >= 178, "{covered} of 200");
}

/// Y = 2A - 3B + noise with A and B the controls: b is (2, -3), and the controlled mean is the
/// true one with an interval far narrower than the plain one.
#[test]
fn several_controls_are_fitted_together() {
    let mc = MonteCarlo {
        inputs: vec![
            normal("a", 3.0, 1.0),
            normal("b", -2.0, 2.0),
            normal("n", 0.0, 1.0),
        ],
        outputs: vec![output(
            "y",
            "2 * $a - 3 * $b + $n",
            vec![by_input("a"), by_input("b")],
        )],
        ..MonteCarlo::default()
    };
    let r = run(&mc, 7, 20_000);
    let c = r.outputs[0]
        .controlled
        .as_ref()
        .unwrap_or_else(|| unreachable!("no controls"));
    assert_eq!(c.controls[0].name, "a");
    assert_eq!(c.controls[1].name, "b");
    assert!(
        (c.controls[0].coefficient - 2.0).abs() < 0.15,
        "{:?}",
        c.controls
    );
    assert!(
        (c.controls[1].coefficient + 3.0).abs() < 0.15,
        "{:?}",
        c.controls
    );
    let e = estimate(&r, 0);
    // 2 * 3 - 3 * (-2) = 12.
    let hw = e.half_width.unwrap_or(f64::NAN);
    assert!((e.mean - 12.0).abs() < 2.0 * hw, "{} ± {hw}", e.mean);
    // Var(Y) = 4 + 36 + 1 = 41 against a residual of 1.
    let ratio = e.variance_ratio.unwrap_or(f64::NAN);
    assert!((ratio / 41.0 - 1.0).abs() < 0.2, "ratio {ratio}");
    assert!(e.helped() && !e.worse());
    assert!(c.correlation.unwrap_or(0.0) > 0.98);
}

/// A control that has nothing to do with the output does no harm beyond the pilot's loss, and
/// the results say it did not help.
#[test]
fn an_unrelated_control_does_not_help() {
    let mc = MonteCarlo {
        inputs: vec![normal("x", 0.0, 1.0), normal("z", 10.0, 2.0)],
        outputs: vec![output("y", "$z", vec![by_input("x")])],
        ..MonteCarlo::default()
    };
    for seed in 1..=10 {
        let r = run(&mc, seed, 20_000);
        let e = estimate(&r, 0);
        let ratio = e.variance_ratio.unwrap_or(f64::NAN);
        assert!((ratio - 1.0).abs() < 0.05, "seed {seed}: ratio {ratio}");
        assert!(!e.helped(), "seed {seed}: {:?}", e.narrowing);
        let c = r.outputs[0].controlled.as_ref();
        assert!(c.and_then(|c| c.correlation).unwrap_or(1.0) < 0.15);
    }
}

/// Fitting b on a pilot that the estimate leaves out makes the controlled mean unbiased for a
/// nonlinear output: Y = exp(X) with X uniform on (0, 1) has the mean e - 1, and the controlled
/// means of 400 seeds average to it within four standard errors of their average.
#[test]
fn the_controlled_mean_of_a_nonlinear_output_is_unbiased() {
    let mc = MonteCarlo {
        inputs: vec![uniform("x", 0.0, 1.0)],
        outputs: vec![output("y", "exp($x)", vec![by_input("x")])],
        ..MonteCarlo::default()
    };
    let means: Vec<f64> = (1..=400_u64)
        .map(|seed| estimate(&run(&mc, seed, 1_500), 0).mean)
        .collect();
    let n = means.len() as f64;
    let avg = means.iter().sum::<f64>() / n;
    let var = means.iter().map(|m| (m - avg) * (m - avg)).sum::<f64>() / (n - 1.0);
    let se = (var / n).sqrt();
    let exact = std::f64::consts::E - 1.0;
    assert!(
        (avg - exact).abs() < 4.0 * se,
        "average {avg}, exact {exact}, standard error {se}"
    );
}

/// 2X + Z² with X uniform on (0, 1) and Z on (-1, 1) has the mean 1 + 1/3. Every sampling's
/// controlled interval holds it about 95% of the time, and the control helps under each.
#[test]
fn every_sampling_has_a_valid_controlled_interval() {
    for (sampling, trials) in [
        (Sampling::Random, 5_000),
        (Sampling::LatinHypercube, 6_000),
        (Sampling::Antithetic, 5_000),
        (Sampling::Sobol, 6_000),
    ] {
        let mc = MonteCarlo {
            inputs: vec![uniform("x", 0.0, 1.0), uniform("z", -1.0, 1.0)],
            outputs: vec![output("y", "2 * $x + $z * $z", vec![by_input("x")])],
            sampling,
            ..MonteCarlo::default()
        };
        let exact = 1.0 + 1.0 / 3.0;
        let covered = (1..=100_u64)
            .filter(|seed| {
                let r = run(&mc, *seed, trials);
                let e = estimate(&r, 0);
                (e.mean - exact).abs() <= e.half_width.unwrap_or(0.0)
            })
            .count();
        assert!(covered >= 86, "{sampling:?}: {covered} of 100");
        let r = run(&mc, 1, trials);
        let e = estimate(&r, 0);
        assert!(e.units >= 2, "{sampling:?}: {} units", e.units);
        assert!(e.half_width.is_some(), "{sampling:?}");
    }
}

/// An earlier output with a known mean is a control of a later one; a wrong mean is reported.
#[test]
fn an_output_with_a_known_mean_is_a_control() {
    let build = |expected: f64| MonteCarlo {
        inputs: vec![normal("x", 10.0, 2.0), normal("n", 0.0, 1.0)],
        outputs: vec![
            output("a", "$x * 2", vec![]),
            output(
                "b",
                "$a + $n",
                vec![Control::Output {
                    name: "a".into(),
                    expected,
                }],
            ),
        ],
        ..MonteCarlo::default()
    };
    let r = run(&build(20.0), 3, 20_000);
    let c = r.outputs[1]
        .controlled
        .as_ref()
        .unwrap_or_else(|| unreachable!("no controls"));
    assert!(!c.controls[0].input);
    assert!((c.controls[0].coefficient - 1.0).abs() < 0.05);
    assert!(!c.controls[0].mismatch);
    // Var(a) = 16 against a residual of 1.
    assert!(estimate(&r, 1).helped());
    assert!(r.outputs[0].controlled.is_none());
    // A mean that is not the output's is noticed.
    let r = run(&build(25.0), 3, 20_000);
    let c = r.outputs[1]
        .controlled
        .as_ref()
        .unwrap_or_else(|| unreachable!("no controls"));
    assert!(c.controls[0].mismatch, "{:?}", c.controls[0]);
}

/// The controlled probability of meeting a target: P(X + Z <= 0) = 1/2, and the indicator goes
/// with X (R² about 0.32), so the interval is about 17% narrower. The plain results are those of
/// a run without the control.
#[test]
fn a_targets_probability_is_controlled_too() {
    let with = MonteCarlo {
        inputs: vec![normal("x", 0.0, 1.0), normal("z", 0.0, 1.0)],
        outputs: vec![McOutput {
            target: Some(McTarget::AtMost(0.0)),
            ..output("y", "$x + $z", vec![by_input("x")])
        }],
        ..MonteCarlo::default()
    };
    let mut without = with.clone();
    without.outputs[0].controls.clear();
    let r = run(&with, 5, 40_000);
    let e = estimate(&r, 0);
    let t = e
        .target
        .as_ref()
        .unwrap_or_else(|| unreachable!("no controlled target"));
    assert!((t.probability - 0.5).abs() < 0.01, "{}", t.probability);
    let (lo, hi) = t.interval.unwrap_or((f64::NAN, f64::NAN));
    assert!(lo <= 0.5 && 0.5 <= hi, "{lo} to {hi}");
    let n = t.narrowing.unwrap_or(f64::NAN);
    assert!((0.05..0.35).contains(&n), "narrowing {n}");
    assert!(t.helped());
    // Everything but the controlled estimate is as without the control.
    let mut plain = r.outputs[0].clone();
    plain.controlled = None;
    assert_eq!(plain, run(&without, 5, 40_000).outputs[0]);
}

/// Controls with correlated inputs, and in a run of whole batches with a sequential stop, work
/// and leave the plain results alone.
#[test]
fn correlated_inputs_keep_their_means() {
    let mc = MonteCarlo {
        inputs: vec![normal("x", 10.0, 2.0), normal("z", 5.0, 1.0)],
        outputs: vec![output("y", "$x + $z", vec![by_input("x")])],
        correlations: vec![stochcraft_mc::McCorrelation {
            a: "x".into(),
            b: "z".into(),
            rank: 0.5,
        }],
        ..MonteCarlo::default()
    };
    assert!(error_codes(&mc).is_empty(), "{:?}", check(&mc));
    let r = run(&mc, 2, 10_000);
    let e = estimate(&r, 0);
    // Batches of 1,000: the first is the pilot, nine are left.
    assert_eq!(e.units, 9);
    assert_eq!(e.trials, 9_000);
    assert!((e.mean - 15.0).abs() < 4.0 * e.half_width.unwrap_or(f64::NAN));
}

/// A run too short to fit and estimate, and a single batch where intervals are across batches,
/// say why they have no controlled estimate.
#[test]
fn a_run_too_short_says_why_there_is_no_estimate() {
    let mc = |sampling| MonteCarlo {
        inputs: vec![normal("x", 0.0, 1.0)],
        outputs: vec![output("y", "$x * 2", vec![by_input("x")])],
        sampling,
        ..MonteCarlo::default()
    };
    let r = run(&mc(Sampling::Random), 1, 3);
    let c = r.outputs[0].controlled.as_ref();
    assert!(c.is_some_and(|c| c.estimate.is_none() && c.unavailable.is_some()));
    let r = run(&mc(Sampling::LatinHypercube), 1, BATCH);
    let c = r.outputs[0].controlled.as_ref();
    assert!(
        c.and_then(|c| c.unavailable.as_deref())
            .is_some_and(|why| why.contains("two batches")),
        "{c:?}"
    );
    // Fewer than 2,000 trials: the first half is the pilot.
    let r = run(&mc(Sampling::Random), 1, 1_500);
    assert_eq!(estimate(&r, 0).trials, 750);
}

/// What a model may name as a control.
#[test]
fn controls_are_checked() {
    let model = |controls: Vec<Control>| MonteCarlo {
        inputs: vec![
            normal("x", 0.0, 1.0),
            input("k", Dist::Constant(2.0)),
            input("big", Dist::Poisson { mean: 1e9 }),
        ],
        outputs: vec![
            output("first", "$x + $k + $big", vec![]),
            output("second", "$first + 1", controls),
            output("third", "5", vec![]),
        ],
        ..MonteCarlo::default()
    };
    let out = |name: &str, expected: f64| Control::Output {
        name: name.into(),
        expected,
    };
    assert!(error_codes(&model(vec![by_input("x")])).is_empty());
    assert!(error_codes(&model(vec![out("first", 3.0)])).is_empty());
    let cases: Vec<(Vec<Control>, &str)> = vec![
        (vec![by_input("nobody")], "unknown_control"),
        (vec![by_input("first")], "unknown_control"),
        (vec![out("x", 0.0)], "unknown_control"),
        (vec![out("second", 0.0)], "control_not_earlier"),
        (vec![out("third", 5.0)], "control_not_earlier"),
        (vec![by_input("big")], "inexact_control"),
        (vec![by_input("k")], "constant_control"),
        (vec![by_input("x"), by_input("x")], "duplicate_control"),
        (vec![out("first", f64::NAN)], "bad_control"),
    ];
    for (controls, code) in cases {
        let got = error_codes(&model(controls.clone()));
        assert!(
            got.contains(&code),
            "{controls:?}: wanted {code}, got {got:?}"
        );
    }
    // An earlier output that is always the same value cannot help either.
    let mut m = model(vec![out("third", 5.0)]);
    m.outputs.swap(1, 2);
    m.outputs[2].expression = "$first + $third".into();
    let got = error_codes(&m);
    assert!(got.contains(&"constant_control"), "{got:?}");
    // The problem is about the output that has the control.
    let problems = check(&model(vec![by_input("nobody")]));
    assert!(
        problems
            .iter()
            .any(|p| p.code == "unknown_control" && p.at.as_deref() == Some("second"))
    );
}

/// An input whose mean is not a finite number cannot be a control, in the check and in the picker.
#[test]
fn an_input_with_an_infinite_mean_is_no_control() {
    let d = Dist::Uniform {
        min: 1e308,
        max: 1.1e308,
    };
    assert!(exact_mean(&d).is_err());
    let mc = MonteCarlo {
        inputs: vec![input("x", d), normal("z", 0.0, 1.0)],
        outputs: vec![output("y", "$x + $z", vec![by_input("x")])],
        ..MonteCarlo::default()
    };
    assert!(
        error_codes(&mc).contains(&"inexact_control"),
        "{:?}",
        check(&mc)
    );
}

/// A control in small units is fitted beside one with a spread of millions: y = x exactly, so
/// the controlled interval has nothing left (the coefficient of x is 1).
#[test]
fn a_control_is_fitted_whatever_its_units() {
    let mc = MonteCarlo {
        inputs: vec![uniform("w", 0.0, 1e6), uniform("x", 0.0, 1.0)],
        outputs: vec![output("y", "$x", vec![by_input("w"), by_input("x")])],
        ..MonteCarlo::default()
    };
    let r = run(&mc, 3, 4_000);
    let c = r.outputs[0]
        .controlled
        .as_ref()
        .unwrap_or_else(|| unreachable!("no controls"));
    assert!((c.controls[1].coefficient - 1.0).abs() < 1e-3, "{c:?}");
    let e = estimate(&r, 0);
    assert!(e.narrowing.is_none_or(|n| n > 0.99), "{e:?}");
    assert!((e.mean - 0.5).abs() < 1e-3, "{}", e.mean);
}

/// Trials with no number are dropped from the plain estimate, but the controls' known means are
/// those of every trial, so there is no controlled estimate: x uniform on [-1, 1] is a control of
/// y = x where it is above 0, and the retained x average 0.5, not 0.
#[test]
fn trials_with_no_number_give_no_controlled_estimate() {
    let mc = MonteCarlo {
        inputs: vec![uniform("x", -1.0, 1.0)],
        outputs: vec![output("y", "if($x > 0, $x, sqrt(-1))", vec![by_input("x")])],
        ..MonteCarlo::default()
    };
    let r = run(&mc, 7, 4_000);
    let c = r.outputs[0]
        .controlled
        .as_ref()
        .unwrap_or_else(|| unreachable!("no controls"));
    assert!(c.estimate.is_none(), "{c:?}");
    assert!(
        c.unavailable
            .as_deref()
            .is_some_and(|why| why.contains("no number")),
        "{c:?}"
    );
}

/// The wrong stated mean of an output control is found from the units the estimate is across: for
/// antithetic pairs the pair means barely move, so a mean that is 0.0092 off is far outside five
/// standard errors of them, though it is inside five standard errors of the single trials.
#[test]
fn the_mismatch_is_judged_on_the_pairs_of_an_antithetic_run() {
    let model = |sampling| MonteCarlo {
        inputs: vec![uniform("x", 0.0, 1.0)],
        outputs: vec![
            output("c", "$x + 0.01 * ($x - 0.5) * ($x - 0.5)", vec![]),
            output(
                "y",
                "$c",
                vec![Control::Output {
                    name: "c".into(),
                    expected: 0.51,
                }],
            ),
        ],
        sampling,
        ..MonteCarlo::default()
    };
    let flagged = |sampling| {
        let r = run(&model(sampling), 4, 2_000);
        r.outputs[1]
            .controlled
            .as_ref()
            .is_some_and(|c| c.controls.iter().any(|u| u.mismatch))
    };
    assert!(flagged(Sampling::Antithetic));
    assert!(!flagged(Sampling::Random));
}

/// Every pair of an antithetic run alike (no spread across pairs): a wrong stated mean is still
/// flagged, and the right one is not.
#[test]
fn a_wrong_mean_is_flagged_when_the_pairs_have_no_spread() {
    let model = |expected| MonteCarlo {
        inputs: vec![uniform("x", 0.0, 1.0)],
        outputs: vec![
            output("c", "if($x < 0.5, 0, 1)", vec![]),
            output(
                "y",
                "$c",
                vec![Control::Output {
                    name: "c".into(),
                    expected,
                }],
            ),
        ],
        sampling: Sampling::Antithetic,
        ..MonteCarlo::default()
    };
    let flagged = |expected| {
        let r = run(&model(expected), 4, 2_000);
        r.outputs[1]
            .controlled
            .as_ref()
            .is_some_and(|c| c.controls.iter().any(|u| u.mismatch))
    };
    assert!(flagged(0.75));
    assert!(!flagged(0.5));
}

/// An input used only as a control is used: it is not reported as unused.
#[test]
fn an_input_used_only_as_a_control_is_not_unused() {
    let mc = |controls: Vec<Control>| MonteCarlo {
        inputs: vec![normal("a", 0.0, 1.0), normal("x", 0.0, 1.0)],
        outputs: vec![output("y", "$a", controls)],
        ..MonteCarlo::default()
    };
    let unused = |m: &MonteCarlo| -> Vec<Option<String>> {
        check(m)
            .iter()
            .filter(|p| p.code == "unused_input")
            .map(|p| p.at.clone())
            .collect()
    };
    assert!(unused(&mc(vec![by_input("x")])).is_empty());
    assert_eq!(unused(&mc(vec![])), vec![Some("x".to_owned())]);
}
