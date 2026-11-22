//! Sobol indices (ROADMAP M4.1 part 3) against models whose indices are known exactly.
//!
//! The seeds are fixed. Each check allows a fixed tolerance of several standard errors of the
//! estimate at the rows used (stated at each test), so a correct estimator fails it about once
//! in tens of thousands of seeds.

use std::f64::consts::PI;

use stochcraft_mc::sobol::{DEFAULT_BOOTSTRAP, DEFAULT_ROWS};
use stochcraft_mc::{
    Compiled, Dist, McCorrelation, McInput, McOutput, MonteCarlo, Sampling, SobolError,
    SobolOptions, SobolOutput, SobolResults, compile,
};

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
    }
}

fn model(inputs: Vec<McInput>, outputs: Vec<McOutput>) -> MonteCarlo {
    MonteCarlo {
        inputs,
        outputs,
        correlations: vec![],
        sampling: Sampling::Random,
        stop: None,
    }
}

fn compiled(mc: &MonteCarlo) -> Compiled {
    compile(mc, 1).unwrap_or_else(|p| unreachable!("{p:?}"))
}

fn options(bootstrap: u32) -> SobolOptions {
    SobolOptions { bootstrap }
}

fn near(what: &str, got: f64, want: f64, tol: f64) {
    assert!(
        (got - want).abs() <= tol,
        "{what}: got {got}, want {want} within {tol}"
    );
}

/// The output `name`'s first-order and total index of input `x`.
fn indices(r: &SobolResults, name: &str, x: &str) -> (f64, f64) {
    let out: &SobolOutput = r
        .outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| unreachable!("no output {name}"));
    let i = out
        .indices
        .iter()
        .find(|i| i.input == x)
        .unwrap_or_else(|| unreachable!("no indices for {x}: {:?}", out.unavailable));
    (i.first.value, i.total.value)
}

/// The Ishigami function, sin x1 + a sin² x2 + b x3⁴ sin x1, with x_i uniform on (−π, π), a = 7
/// and b = 0.1 (Ishigami and Homma, 1990). Its variances in closed form (Homma and Saltelli,
/// 1996; Saltelli et al., *Global Sensitivity Analysis: The Primer*, 2008, §4.5):
///
/// - V = a²/8 + bπ⁴/5 + b²π⁸/18 + 1/2 (13.8446);
/// - V1 = (1 + bπ⁴/5)²/2, V2 = a²/8, V3 = 0, and the only interaction is V13 = 8b²π⁸/225;
/// - S_i = V_i/V, and S_Ti = (V_i + the interactions with i)/V.
///
/// So S1 = 0.3139, S2 = 0.4424, S3 = 0, ST1 = 0.5576, ST2 = 0.4424 and ST3 = 0.2437.
///
/// 10,000 rows (50,000 evaluations): the standard error of a first-order index here is about
/// 0.014 and of a total about 0.01, so the check allows 0.06.
#[test]
fn the_ishigami_function() {
    let pi2 = PI * PI;
    let pi4 = pi2 * pi2;
    let pi8 = pi4 * pi4;
    let (a, b) = (7.0, 0.1);
    let v = a * a / 8.0 + b * pi4 / 5.0 + b * b * pi8 / 18.0 + 0.5;
    let v1 = (1.0 + b * pi4 / 5.0) * (1.0 + b * pi4 / 5.0) / 2.0;
    let v2 = a * a / 8.0;
    let v13 = 8.0 * b * b * pi8 / 225.0;
    near("V", v, 13.8446, 1e-4);
    let want = [
        ("x1", v1 / v, (v1 + v13) / v),
        ("x2", v2 / v, v2 / v),
        ("x3", 0.0, v13 / v),
    ];
    near("S1", want[0].1, 0.3139, 1e-4);
    near("ST3", want[2].2, 0.2437, 1e-4);

    let uniform = || Dist::Uniform { min: -PI, max: PI };
    let mc = model(
        vec![
            input("x1", uniform()),
            input("x2", uniform()),
            input("x3", uniform()),
        ],
        vec![output(
            "y",
            "sin($x1) + 7 * pow(sin($x2), 2) + 0.1 * pow($x3, 4) * sin($x1)",
        )],
    );
    let r = compiled(&mc)
        .sobol(10_000, 20_261_121, &options(200))
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(r.rows, 10_000);
    assert_eq!(r.evaluations, 50_000);
    assert_eq!(r.inputs, ["x1", "x2", "x3"]);
    let out = r.outputs.first().unwrap_or_else(|| unreachable!());
    near("V estimate", out.variance.unwrap_or(f64::NAN), v, 1.0);
    assert_eq!(out.rows_dropped, 0);
    for (x, s, st) in want {
        let (first, total) = indices(&r, "y", x);
        near(&format!("S of {x}"), first, s, 0.06);
        near(&format!("ST of {x}"), total, st, 0.06);
    }
    // Each estimate has an interval of a plausible width.
    for i in &out.indices {
        for e in [i.first, i.total] {
            let (lo, hi) = e.interval.unwrap_or((f64::NAN, f64::NAN));
            assert!(lo < hi && hi - lo < 0.15, "{}: {lo}..{hi}", i.input);
        }
    }
}

/// y = x1 + 2 x2 + 0.5 x3 with x1 ~ U(0, 1) (variance 1/12), x2 ~ N(0, 1) and x3 ~ U(0, 4)
/// (variance 4/3): the variance is 1/12 + 4 + 1/3 = 53/12, and with no interactions
/// S_i = S_Ti = c_i² Var(x_i) / 53/12: 1/53, 48/53 and 4/53 (0.0189, 0.9057, 0.0755).
///
/// 20,000 rows: the standard error of S2 is about 0.01 and the others' less, so 0.05.
#[test]
fn a_linear_model_has_no_interactions() {
    let mc = model(
        vec![
            input("x1", Dist::Uniform { min: 0.0, max: 1.0 }),
            input(
                "x2",
                Dist::Normal {
                    mean: 0.0,
                    std_dev: 1.0,
                },
            ),
            input("x3", Dist::Uniform { min: 0.0, max: 4.0 }),
        ],
        vec![output("y", "$x1 + 2 * $x2 + 0.5 * $x3")],
    );
    let r = compiled(&mc)
        .sobol(20_000, 7, &options(100))
        .unwrap_or_else(|e| unreachable!("{e}"));
    for (x, want) in [("x1", 1.0 / 53.0), ("x2", 48.0 / 53.0), ("x3", 4.0 / 53.0)] {
        let (first, total) = indices(&r, "y", x);
        near(&format!("S of {x}"), first, want, 0.05);
        near(&format!("ST of {x}"), total, want, 0.05);
    }
}

/// y = x1 · x2 with independent U(0, 1): E y = 1/4, E y² = 1/9, so V = 1/9 − 1/16 = 7/144.
/// E[y | x1] = x1/2, so V1 = V2 = (1/12)/4 = 3/144 and S1 = S2 = 3/7 (0.4286); the interaction
/// is V12 = V − V1 − V2 = 1/144, so ST1 = ST2 = 4/7 (0.5714).
///
/// 20,000 rows, 0.05.
#[test]
fn a_product_has_an_interaction() {
    let u = || Dist::Uniform { min: 0.0, max: 1.0 };
    let mc = model(
        vec![input("x1", u()), input("x2", u())],
        vec![output("y", "$x1 * $x2")],
    );
    let r = compiled(&mc)
        .sobol(20_000, 3, &options(100))
        .unwrap_or_else(|e| unreachable!("{e}"));
    for x in ["x1", "x2"] {
        let (first, total) = indices(&r, "y", x);
        near(&format!("S of {x}"), first, 3.0 / 7.0, 0.05);
        near(&format!("ST of {x}"), total, 4.0 / 7.0, 0.05);
    }
}

#[test]
fn correlated_inputs_are_refused() {
    let u = || Dist::Uniform { min: 0.0, max: 1.0 };
    let mut mc = model(
        vec![input("x1", u()), input("x2", u()), input("x3", u())],
        vec![output("y", "$x1 + $x2 + $x3")],
    );
    mc.correlations.push(McCorrelation {
        a: "x1".into(),
        b: "x3".into(),
        rank: 0.5,
    });
    let e = compiled(&mc).sobol(100, 1, &options(0));
    assert_eq!(
        e,
        Err(SobolError::Correlated(vec!["x1".into(), "x3".into()]))
    );
    let said = e.err().map(|e| e.to_string()).unwrap_or_default();
    assert!(said.contains("independent"), "{said}");
}

#[test]
fn constant_inputs_are_left_out_and_an_output_that_does_not_vary_has_no_indices() {
    let mc = model(
        vec![
            input("x", Dist::Uniform { min: 0.0, max: 1.0 }),
            input("k", Dist::Constant(3.0)),
        ],
        vec![output("y", "$x + $k"), output("flat", "$x * 0")],
    );
    let r = compiled(&mc)
        .sobol(5_000, 5, &options(50))
        .unwrap_or_else(|e| unreachable!("{e}"));
    assert_eq!(r.inputs, ["x"]);
    assert_eq!(r.constants, ["k"]);
    assert_eq!(r.evaluations, 15_000);
    let (first, total) = indices(&r, "y", "x");
    near("S", first, 1.0, 0.1);
    near("ST", total, 1.0, 0.1);
    let flat = r.outputs.get(1).unwrap_or_else(|| unreachable!());
    assert!(flat.indices.is_empty());
    assert_eq!(flat.variance, Some(0.0));
    let why = flat.unavailable.clone().unwrap_or_default();
    assert!(why.contains("does not vary"), "{why}");

    let only_constants = model(
        vec![input("k", Dist::Constant(3.0))],
        vec![output("y", "$k")],
    );
    assert_eq!(
        compiled(&only_constants).sobol(100, 1, &options(0)),
        Err(SobolError::NoVariedInputs)
    );
    assert_eq!(
        compiled(&mc).sobol(5, 1, &options(0)),
        Err(SobolError::TooFewRows)
    );
}

/// An output with a row that is not a finite number in some matrix gets no indices (the rows
/// left out depend on the inputs, so what is left does not estimate the shares), with the
/// counts; an output with a number in every row keeps its indices.
#[test]
fn an_output_with_rows_that_are_not_a_number_gets_no_indices_but_the_others_keep_theirs() {
    let mc = model(
        vec![input("x", Dist::Uniform { min: 0.0, max: 1.0 })],
        vec![output("y", "if($x > 0.9, 1 div 0, $x)"), output("z", "$x")],
    );
    let r = compiled(&mc)
        .sobol(2_000, 11, &options(20))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let y = r.outputs.first().unwrap_or_else(|| unreachable!());
    // A row survives when x is at most 0.9 in A and in B: 81% of them.
    assert!(y.rows_dropped > 200 && y.rows_dropped < 550, "{y:?}");
    assert_eq!(y.rows_used + y.rows_dropped, 2_000);
    assert!(y.indices.is_empty(), "{y:?}");
    let why = y.unavailable.as_deref().unwrap_or("");
    assert!(
        why.contains(&format!("{} of 2000 rows", y.rows_dropped)),
        "{why}"
    );
    let z = r.outputs.get(1).unwrap_or_else(|| unreachable!());
    assert_eq!(z.rows_dropped, 0);
    assert!(z.unavailable.is_none() && !z.indices.is_empty(), "{z:?}");
}

#[test]
fn the_same_seed_gives_the_same_results_and_a_stepped_run_agrees() {
    let u = || Dist::Uniform { min: 0.0, max: 1.0 };
    let mc = model(
        vec![input("x1", u()), input("x2", u())],
        vec![output("y", "$x1 * $x2 + $x1")],
    );
    let c = compiled(&mc);
    let a = c.sobol(2_500, 99, &options(30));
    let b = c.sobol(2_500, 99, &options(30));
    assert!(a.is_ok());
    assert_eq!(a, b);
    assert_ne!(a, c.sobol(2_500, 100, &options(30)));

    // Stepped.
    let mut run = c
        .sobol_run(2_500, 99, &options(30))
        .unwrap_or_else(|e| unreachable!("{e}"));
    let mut steps = 0;
    let stepped = loop {
        if let Some(r) = run.take_results() {
            break r;
        }
        run.step(&c);
        steps += 1;
    };
    // 3 batches, then each output a step to start and as many as its bootstrap needs, and the end.
    assert!(steps > 3 + 1, "{steps}");
    assert_eq!(Ok(stepped), a);
}

#[test]
fn the_defaults_and_the_cost() {
    assert_eq!(DEFAULT_ROWS, 1_000);
    assert_eq!(DEFAULT_BOOTSTRAP, 1_000);
    assert_eq!(SobolOptions::default().bootstrap, 1_000);
    let u = || Dist::Uniform { min: 0.0, max: 1.0 };
    let mc = model(
        vec![input("a", u()), input("b", u()), input("c", u())],
        vec![output("y", "$a + $b * $c")],
    );
    let r = compiled(&mc)
        .sobol(DEFAULT_ROWS, 1, &options(10))
        .unwrap_or_else(|e| unreachable!("{e}"));
    // N (k + 2) evaluations.
    assert_eq!(r.evaluations, 5_000);
    assert_eq!(
        compiled(&mc).sobol(3_000_000, 1, &options(0)),
        Err(SobolError::TooMany {
            evaluations: 15_000_000
        })
    );
}

/// The bootstrap is made a few resamples a step, from the same keyed stream in the same order:
/// a step of one resample at a time gives bit-identical results, and takes more steps.
#[test]
fn a_small_step_size_gives_the_same_results() {
    let u = || Dist::Uniform { min: 0.0, max: 1.0 };
    let mc = model(
        vec![input("x1", u()), input("x2", u())],
        vec![output("y", "$x1 * $x2 + $x1"), output("z", "$x2")],
    );
    let c = compiled(&mc);
    let whole = c.sobol(1_200, 5, &options(40));
    let drive = |ops: usize| {
        let mut run = c
            .sobol_run(1_200, 5, &options(40))
            .unwrap_or_else(|e| unreachable!("{e}"))
            .with_step_operations(ops);
        let mut steps = 0;
        loop {
            if let Some(r) = run.take_results() {
                break (r, steps);
            }
            run.step(&c);
            steps += 1;
        }
    };
    let (small, small_steps) = drive(1);
    let (default, default_steps) = drive(stochcraft_mc::sobol::STEP_OPERATIONS);
    assert_eq!(Ok(small), whole);
    assert_eq!(Ok(default), whole);
    // One resample a step: 40 per output instead of one step.
    assert!(
        small_steps >= default_steps + 2 * 39,
        "{small_steps} {default_steps}"
    );
}
