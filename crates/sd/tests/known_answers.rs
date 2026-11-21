//! System dynamics models whose answers are known (ROADMAP M4.2's checks).
//!
//! The tolerances come from running the same stepping in a separate script (Euler or
//! fourth-order Runge-Kutta, the same pulse and the same order of updates) and leaving a margin
//! of about two over what it gave; each says which it is.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use stochcraft_random::math::{exp, powf};
use stochcraft_sd::{
    ErrorKind, Gf, GfKind, GfX, Layout, Method, RunOptions, SdModel, SdResults, SimSpecs, Step,
    VarKind, Variable, compile, read_xmile, run, write_xmile,
};

fn specs(stop: f64, dt: f64, method: Method) -> SimSpecs {
    SimSpecs {
        start: 0.0,
        stop,
        dt: Step::Value(dt),
        method,
        time_units: String::new(),
        save_step: None,
    }
}

fn model(stop: f64, dt: f64, method: Method, vars: Vec<Variable>) -> SdModel {
    SdModel {
        specs: specs(stop, dt, method),
        vars,
        ..SdModel::default()
    }
}

fn go(m: &SdModel, opts: &RunOptions) -> SdResults {
    let p = compile(m).unwrap();
    run(&p, opts).unwrap()
}

fn once(m: &SdModel) -> SdResults {
    go(m, &RunOptions::default())
}

fn mean(r: &SdResults, name: &str) -> Vec<f64> {
    r.series(name)
        .map(|s| s.mean.clone())
        .unwrap_or_else(|| panic!("no series {name}"))
}

fn last(r: &SdResults, name: &str) -> f64 {
    mean(r, name).last().copied().unwrap()
}

fn non_negative(mut v: Variable) -> Variable {
    if let VarKind::Stock { non_negative, .. } = &mut v.kind {
        *non_negative = true;
    }
    v
}

fn growth(dt: f64, method: Method) -> SdModel {
    model(
        10.0,
        dt,
        method,
        vec![
            Variable::stock("population", "100", &["growth"], &[]),
            Variable::flow("growth", "rate * population"),
            Variable::aux("rate", "0.1"),
        ],
    )
}

#[test]
fn exponential_growth_euler_error_halves_with_the_step() {
    let exact = 100.0 * exp(1.0);
    let a = last(&once(&growth(1.0 / 16.0, Method::Euler)), "population");
    let b = last(&once(&growth(1.0 / 32.0, Method::Euler)), "population");
    let ratio = (a - exact).abs() / (b - exact).abs();
    // The script gave 1.994.
    assert!((1.95..2.05).contains(&ratio), "ratio {ratio}");
    // And Euler's answer is the closed form of the recurrence, (1 + r dt)^n.
    let discrete = 100.0 * powf(1.0 + 0.1 / 16.0, 160.0);
    assert!((a - discrete).abs() < 1e-9 * a, "{a} vs {discrete}");
}

#[test]
fn exponential_growth_with_rk4_matches_the_exponential() {
    let exact = 100.0 * exp(1.0);
    let r = once(&growth(1.0 / 16.0, Method::Rk4));
    // The script gave 1.3e-11.
    assert!((last(&r, "population") - exact).abs() < 1e-9 * exact);
    assert_eq!(r.method, "rk4");
}

#[test]
fn time_is_start_plus_steps_times_dt() {
    let r = once(&growth(1.0 / 16.0, Method::Euler));
    assert_eq!(r.times.len(), 161);
    assert_eq!(r.times.get(160).copied(), Some(10.0));
    assert_eq!(r.times.get(16).copied(), Some(1.0));
    assert_eq!(r.steps, 160);
    // A step override is the same model at another step.
    let b = go(
        &growth(1.0 / 16.0, Method::Euler),
        &RunOptions {
            dt: Some(1.0 / 32.0),
            ..RunOptions::default()
        },
    );
    let mut other = growth(1.0 / 32.0, Method::Euler);
    other.specs.save_step = Some(1.0 / 16.0);
    let direct = once(&other);
    assert_eq!(mean(&b, "population"), mean(&direct, "population"));
}

#[test]
fn the_halved_step_check_reports_the_difference_of_each_stock() {
    let m = growth(0.25, Method::Euler);
    let r = go(
        &m,
        &RunOptions {
            dt_check: true,
            ..RunOptions::default()
        },
    );
    let check = r.dt_check.clone().unwrap();
    assert_eq!(check.stocks.len(), 1);
    let s = check.stocks.first().unwrap();
    let a = 100.0 * powf(1.0 + 0.1 * 0.25, 40.0);
    let b = 100.0 * powf(1.0 + 0.1 * 0.125, 80.0);
    let want = (a - b).abs() / b;
    assert!(
        (s.max_relative_difference - want).abs() < 1e-9,
        "{} vs {want}",
        s.max_relative_difference
    );
    assert_eq!(s.at_time, 10.0);
    assert!(check.within_threshold, "{want}");
    // A coarse step fails it.
    let coarse = go(
        &growth(2.0, Method::Euler),
        &RunOptions {
            dt_check: true,
            ..RunOptions::default()
        },
    );
    assert!(!coarse.dt_check.unwrap().within_threshold);
}

#[test]
fn logistic_growth_with_rk4_matches_its_closed_form() {
    let mut m = model(
        20.0,
        0.125,
        Method::Rk4,
        vec![
            Variable::stock("pop", "10", &["g"], &[]),
            Variable::flow("g", "r * pop * (1 - pop / k)"),
            Variable::aux("r", "0.5"),
            Variable::aux("k", "1000"),
        ],
    );
    m.specs.save_step = Some(1.0);
    let r = once(&m);
    let pop = mean(&r, "pop");
    assert_eq!(pop.len(), 21);
    for (k, p) in pop.iter().enumerate() {
        let t = k as f64;
        let exact = 1000.0 / (1.0 + 99.0 * exp(-0.5 * t));
        // The script gave a largest relative error of 2.3e-7.
        assert!((p - exact).abs() / exact < 1e-6, "t={t}: {p} vs {exact}");
    }
}

#[test]
fn a_third_order_delay_answers_a_pulse_with_the_erlang_density() {
    // A one-step pulse of volume 1 into DELAY3 with delay 3: three stages of time 1 each, so the
    // output is the Erlang density with shape 3 and rate 1, t^2 e^-t / 2.
    let mut m = model(
        30.0,
        0.01,
        Method::Euler,
        vec![
            Variable::aux("input", "PULSE(1, 0)"),
            Variable::aux("out", "DELAY3(input, 3, 0)"),
            Variable::flow("emit", "out"),
            Variable::stock("total", "0", &["emit"], &[]),
        ],
    );
    m.specs.save_step = Some(0.5);
    let r = once(&m);
    let out = mean(&r, "out");
    for (k, v) in out.iter().enumerate() {
        let t = r.times.get(k).copied().unwrap();
        let density = t * t * exp(-t) / 2.0;
        // The script gave a largest difference of 0.0025.
        assert!((v - density).abs() < 0.004, "t={t}: {v} vs {density}");
    }
    // What goes in comes out.
    assert!((last(&r, "total") - 1.0).abs() < 1e-3);
}

#[test]
fn a_first_order_smooth_answers_a_step_with_one_minus_exponential() {
    let mut m = model(
        20.0,
        0.01,
        Method::Euler,
        vec![
            Variable::aux("target", "STEP(1, 1)"),
            Variable::aux("smooth", "SMTH1(target, 2)"),
        ],
    );
    m.specs.save_step = Some(0.5);
    let r = once(&m);
    for (k, v) in mean(&r, "smooth").iter().enumerate() {
        let t = r.times.get(k).copied().unwrap();
        let exact = if t < 1.0 {
            0.0
        } else {
            1.0 - exp(-(t - 1.0) / 2.0)
        };
        // The script gave a largest difference of 0.0009.
        assert!((v - exact).abs() < 0.002, "t={t}: {v} vs {exact}");
    }
}

#[test]
fn delays_and_smooths_start_at_the_input_and_stay_there_when_it_does() {
    let r = once(&model(
        10.0,
        0.5,
        Method::Euler,
        vec![
            Variable::aux("input", "5"),
            Variable::aux("d1", "DELAY1(input, 4)"),
            Variable::aux("d3", "DELAY3(input, 4)"),
            Variable::aux("s1", "SMTH1(input, 4)"),
            Variable::aux("s3", "SMTH3(input, 4)"),
        ],
    ));
    for name in ["d1", "d3", "s1", "s3"] {
        for v in mean(&r, name) {
            assert!((v - 5.0).abs() < 1e-9, "{name}: {v}");
        }
    }
}

#[test]
fn a_non_negative_stock_gives_what_it_holds_and_material_is_conserved() {
    let r = once(&model(
        2.0,
        0.25,
        Method::Euler,
        vec![
            non_negative(Variable::stock("tank", "10", &[], &["drain"])),
            Variable::stock("sink", "0", &["drain"], &[]),
            Variable::flow("drain", "100"),
        ],
    ));
    let (tank, sink, drain) = (mean(&r, "tank"), mean(&r, "sink"), mean(&r, "drain"));
    assert_eq!(tank.first().copied(), Some(10.0));
    for k in 0..tank.len() {
        assert!(tank.get(k).copied().unwrap() >= 0.0);
        let total = tank.get(k).copied().unwrap() + sink.get(k).copied().unwrap();
        assert!((total - 10.0).abs() < 1e-12, "step {k}: {total}");
    }
    // The first step takes all of it (10 / 0.25 = 40 a time unit), then there is nothing.
    assert_eq!(drain.first().copied(), Some(40.0));
    assert_eq!(tank.get(1).copied(), Some(0.0));
    assert_eq!(drain.get(1).copied(), Some(0.0));
    assert_eq!(sink.last().copied(), Some(10.0));
}

#[test]
fn outflows_are_limited_in_the_order_the_stock_lists_them() {
    // The second one listed sorts first by name; it must still be worked out after the first.
    let r = once(&model(
        1.0,
        0.25,
        Method::Euler,
        vec![
            non_negative(Variable::stock("tank", "10", &[], &["z_first", "a_second"])),
            Variable::flow("z_first", "30"),
            Variable::flow("a_second", "30"),
        ],
    ));
    assert_eq!(mean(&r, "z_first").first().copied(), Some(30.0));
    assert_eq!(mean(&r, "a_second").first().copied(), Some(10.0));
    assert_eq!(mean(&r, "tank").get(1).copied(), Some(0.0));
}

#[test]
fn regrowth_is_worked_out_from_the_stock_left_after_the_catch() {
    // The order of updates within a step (ROADMAP M4.2, from the fisheries of Brailsford et al.
    // ch. 9): regrowth is a function of what is left after the catch, so it must be worked out
    // after the catch even though it sorts before it by name. Worked out from the stock before
    // the catch, the equilibrium would be 887.30; from what is left, 937.30. The script's
    // iteration of the same rule gave 937.2983346.
    let m = model(
        300.0,
        1.0,
        Method::Euler,
        vec![
            Variable::stock("fish", "500", &["a_regrowth"], &["c_catch"]),
            Variable::flow("c_catch", "50"),
            Variable::aux("b_left", "fish - c_catch * DT"),
            Variable::flow("a_regrowth", "0.5 * b_left * (1 - b_left / 1000)"),
        ],
    );
    let r = once(&m);
    assert!(
        (last(&r, "fish") - 937.298_334_620_741_7).abs() < 1e-6,
        "{}",
        last(&r, "fish")
    );
    let left_star = 500.0 * (1.0 + (1.0_f64 - 4.0 * 50.0 / (0.5 * 1000.0)).sqrt());
    assert!((last(&r, "b_left") - left_star).abs() < 1e-6);
}

fn walk(extra: bool) -> SdModel {
    let mut vars = vec![
        Variable::stock("w", "0", &["step_noise"], &[]),
        Variable::flow("step_noise", "NORMAL(0, 1)"),
    ];
    if extra {
        vars.push(Variable::aux("unrelated", "NORMAL(0, 1)"));
        vars.push(Variable::aux("another", "RANDOM(0, 10)"));
    }
    model(100.0, 1.0, Method::Euler, vars)
}

#[test]
fn a_random_model_replicates_reproducibly_by_seed() {
    let opts = RunOptions {
        replications: Some(200),
        seed: 7,
        ..RunOptions::default()
    };
    let a = go(&walk(false), &opts);
    let b = go(&walk(false), &opts);
    assert_eq!(a.series, b.series);
    assert!(a.random);
    assert_eq!(a.replications, 200);
    // Another seed, another run.
    let c = go(
        &walk(false),
        &RunOptions {
            seed: 8,
            ..opts.clone()
        },
    );
    assert_ne!(a.series, c.series);
    // Adding other random variables leaves this one's draws as they were.
    let d = go(&walk(true), &opts);
    assert_eq!(mean(&a, "w"), mean(&d, "w"));
    // The walk's spread is 10 at the end, so the mean of 200 of them is within five standard
    // errors (3.6) of 0.
    assert!(last(&a, "w").abs() < 3.6, "{}", last(&a, "w"));
    // The band has the median inside it.
    let s = a.series("w").unwrap();
    assert_eq!(s.median.len(), a.times.len());
    for (k, ((lo, mid), hi)) in s.low.iter().zip(&s.median).zip(&s.high).enumerate() {
        assert!(lo <= mid, "step {k}: {lo} {mid} {hi}");
        assert!(mid <= hi, "step {k}: {lo} {mid} {hi}");
    }
    let end_spread = s.high.last().unwrap() - s.low.last().unwrap();
    // The 5% to 95% band of a normal with standard deviation 10 is 33 wide.
    assert!((20.0..45.0).contains(&end_spread), "{end_spread}");
}

#[test]
fn a_model_with_no_random_function_runs_once() {
    let r = go(
        &growth(0.5, Method::Euler),
        &RunOptions {
            replications: Some(50),
            ..RunOptions::default()
        },
    );
    assert_eq!(r.replications, 1);
    assert!(!r.random);
    assert!(r.series("population").unwrap().median.is_empty());
    assert!(r.warnings.iter().any(|w| w.contains("no random function")));
}

#[test]
fn a_draw_holds_for_its_interval_and_does_not_depend_on_the_step() {
    let m = model(
        12.0,
        1.0,
        Method::Euler,
        vec![Variable::aux("x", "NORMAL(0, 1, 0, 4)")],
    );
    let r = go(
        &m,
        &RunOptions {
            replications: Some(1),
            seed: 3,
            ..RunOptions::default()
        },
    );
    let x = mean(&r, "x");
    assert_eq!(x.first(), x.get(3));
    assert_ne!(x.get(3), x.get(4));
    assert_eq!(x.get(4), x.get(7));
    assert_ne!(x.get(7), x.get(8));
    // Halving the step draws the same numbers at the same times.
    let half = go(
        &m,
        &RunOptions {
            replications: Some(1),
            seed: 3,
            dt: Some(0.5),
            ..RunOptions::default()
        },
    );
    assert_eq!(mean(&half, "x"), x);
    // By default a draw lasts a step of the file.
    let each = model(
        5.0,
        1.0,
        Method::Euler,
        vec![Variable::aux("y", "RANDOM(0, 1)")],
    );
    let y = mean(
        &go(
            &each,
            &RunOptions {
                replications: Some(1),
                ..RunOptions::default()
            },
        ),
        "y",
    );
    assert_ne!(y.first(), y.get(1));
}

#[test]
fn a_pulse_delivers_its_volume_whole_under_either_method() {
    for method in [Method::Euler, Method::Rk4] {
        let m = model(
            10.0,
            0.25,
            method,
            vec![
                Variable::stock("s", "0", &["f"], &[]),
                Variable::flow("f", "PULSE(5, 2)"),
                Variable::stock("t", "0", &["g"], &[]),
                Variable::flow("g", "PULSE(1, 2, 3)"),
            ],
        );
        let r = once(&m);
        assert!((last(&r, "s") - 5.0).abs() < 1e-9, "{method:?}");
        // At 2, 5 and 8.
        assert!((last(&r, "t") - 3.0).abs() < 1e-9, "{method:?}");
    }
}

#[test]
fn operators_and_functions_follow_the_specification() {
    let cases: [(&str, f64); 16] = [
        ("IF 3 > 2 AND NOT 0 THEN MIN(4, 2 + 1) ELSE 9", 3.0),
        ("2 ^ 3 ^ 2", 512.0),
        ("-2 ^ 2", -4.0),
        ("7 MOD 4", 3.0),
        ("-7 MOD 4", 1.0),
        ("ABS(-3) + SQRT(16)", 7.0),
        ("EXP(0) + LN(1)", 1.0),
        ("1 = 1", 1.0),
        ("1 <> 1", 0.0),
        ("MAX(1, 5, 3)", 5.0),
        ("TIME + DT", 5.0),
        ("STEP(3, 2)", 3.0),
        ("RAMP(2, 1)", 6.0),
        ("IF_THEN_ELSE(0, 1, 2)", 2.0),
        ("1 < 2 = 1", 1.0),
        ("2 * 3 + 4 / 2 - 1", 7.0),
    ];
    for (eqn, want) in cases {
        let r = once(&model(
            4.0,
            1.0,
            Method::Euler,
            vec![Variable::aux("v", eqn)],
        ));
        let got = last(&r, "v");
        assert!((got - want).abs() < 1e-12, "{eqn}: {got} vs {want}");
    }
}

#[test]
fn a_graphical_function_is_clamped_at_its_ends_with_a_warning() {
    let table = Gf {
        kind: GfKind::Continuous,
        x: GfX::Scale {
            min: 0.0,
            max: 10.0,
        },
        y: vec![0.0, 10.0, 5.0],
    };
    let mut effect = Variable::aux("effect", "TIME");
    effect.gf = Some(table.clone());
    let mut called = Variable::aux("called", "curve(TIME)");
    called.eqn = Some("curve(TIME)".to_owned());
    let mut curve = Variable::aux("curve", "0");
    curve.eqn = None;
    curve.gf = Some(table);
    let mut m = model(12.0, 0.5, Method::Euler, vec![effect, curve, called]);
    m.specs.save_step = Some(2.5);
    let r = once(&m);
    // Saved at 0, 2.5, 5, 7.5 and 10, and at 12, where the run ends.
    let e = mean(&r, "effect");
    assert_eq!(e.get(1).copied(), Some(5.0));
    assert_eq!(e.get(2).copied(), Some(10.0));
    assert_eq!(e.get(3).copied(), Some(7.5));
    assert_eq!(e.last().copied(), Some(5.0));
    assert_eq!(mean(&r, "called"), e);
    // A table with no equation is a function of time.
    assert_eq!(mean(&r, "curve"), e);
    let warnings: Vec<&String> = r
        .warnings
        .iter()
        .filter(|w| w.contains("outside its points"))
        .collect();
    assert!(!warnings.is_empty(), "{:?}", r.warnings);
    // Said once for the variable, not at every step.
    assert!(warnings.len() <= 3, "{warnings:?}");
}

#[test]
fn an_algebraic_loop_is_an_error_naming_the_cycle() {
    let m = model(
        1.0,
        1.0,
        Method::Euler,
        vec![Variable::aux("a", "b + 1"), Variable::aux("b", "a * 2")],
    );
    let e = compile(&m).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Loop);
    assert!(e.message.contains("a -> b -> a"), "{}", e.message);
    // A stock on the loop is no loop.
    let ok = model(
        1.0,
        1.0,
        Method::Euler,
        vec![
            Variable::stock("s", "1", &["f"], &[]),
            Variable::flow("f", "s * 0.5"),
        ],
    );
    assert!(compile(&ok).is_ok());
}

#[test]
fn a_loop_through_a_smooth_is_no_algebraic_loop_but_needs_an_initial_value() {
    let looped = |init: &str| {
        model(
            5.0,
            1.0,
            Method::Euler,
            vec![
                Variable::aux("x", &format!("SMTH1(y, 5{init})")),
                Variable::aux("y", "x + 1"),
            ],
        )
    };
    // Stepping needs no order between them.
    let program = compile(&looped(", 0")).unwrap();
    assert!(run(&program, &RunOptions::default()).is_ok());
    // But the smooth's own initial value is the input's, which needs the smooth: written out
    // or not at all.
    let program = compile(&looped("")).unwrap();
    let e = run(&program, &RunOptions::default()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Loop);
    assert!(e.message.contains("initial"), "{}", e.message);
}

#[test]
fn what_is_not_supported_is_refused_by_name() {
    let file = |body: &str| {
        format!(
            "<xmile version=\"1.0\"><header/><sim_specs><start>0</start><stop>1</stop></sim_specs>{body}</xmile>"
        )
    };
    let with_model = |vars: &str| file(&format!("<model><variables>{vars}</variables></model>"));
    let e = read_xmile(&file(
        "<dimensions><dim name=\"d\"/></dimensions><model><variables/></model>",
    ))
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Unsupported);
    assert!(e.message.contains("dimensions"), "{}", e.message);

    let e = read_xmile(&with_model(
        "<aux name=\"a\"><dimensions><dim name=\"d\"/></dimensions><eqn>1</eqn></aux>",
    ))
    .unwrap_err();
    assert!(e.message.contains("arrays"), "{}", e.message);

    let e = read_xmile(&with_model("<module name=\"m\"/>")).unwrap_err();
    assert!(e.message.contains("module"), "{}", e.message);

    let e = read_xmile(&with_model(
        "<stock name=\"s\"><eqn>0</eqn><conveyor><len>3</len></conveyor></stock>",
    ))
    .unwrap_err();
    assert!(e.message.contains("conveyors"), "{}", e.message);

    // Arrays and unsupported functions in an equation, at compile time.
    for (eqn, word) in [
        ("x[1]", "arrays"),
        ("SUM(x)", "SUM"),
        ("SIN(1)", "SIN"),
        ("nope", "nope"),
    ] {
        let m = model(
            1.0,
            1.0,
            Method::Euler,
            vec![Variable::aux("v", eqn), Variable::aux("x", "1")],
        );
        let e = compile(&m).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Equation, "{eqn}");
        assert!(e.message.contains(word), "{eqn}: {}", e.message);
        assert!(e.message.contains('v'), "names the variable: {}", e.message);
    }

    // Not well-formed.
    let e = read_xmile("<xmile><a></xmile>").unwrap_err();
    assert_eq!(e.kind, ErrorKind::Xml);
    // A method that is not a method.
    let e = read_xmile(
        "<xmile version=\"1.0\"><sim_specs method=\"magic\"><start>0</start><stop>1</stop></sim_specs><model><variables/></model></xmile>",
    )
    .unwrap_err();
    assert!(e.message.contains("magic"), "{}", e.message);
}

#[test]
fn units_that_do_not_balance_are_warnings() {
    let mut m = model(
        5.0,
        1.0,
        Method::Euler,
        vec![
            Variable::stock("people", "10", &["births"], &[]).with_units("people"),
            Variable::flow("births", "people * rate").with_units("people/year"),
            Variable::aux("rate", "0.1").with_units("1/year"),
        ],
    );
    m.specs.time_units = "Years".to_owned();
    assert!(
        compile(&m).unwrap().warnings.is_empty(),
        "{:?}",
        compile(&m).unwrap().warnings
    );

    // A flow in the wrong units for its stock and time unit.
    m.vars[1] = Variable::flow("births", "people * rate").with_units("dollars/year");
    let w = compile(&m).unwrap().warnings;
    assert!(w.iter().any(|x| x.contains("flow `births`")), "{w:?}");

    // A sum of unlike things.
    m.vars[1] = Variable::flow("births", "people * rate").with_units("people/year");
    m.vars
        .push(Variable::aux("money", "5").with_units("dollars"));
    m.vars.push(Variable::aux("mixed", "people + money"));
    let w = compile(&m).unwrap().warnings;
    assert!(w.iter().any(|x| x.contains("a sum")), "{w:?}");

    // A time unit that does not match the flows.
    m.vars.truncate(3);
    m.specs.time_units = "Months".to_owned();
    let w = compile(&m).unwrap().warnings;
    assert!(w.iter().any(|x| x.contains("time unit")), "{w:?}");
}

#[test]
fn times_are_reported_in_the_unit_asked_for() {
    let mut m = growth(1.0, Method::Euler);
    m.specs.time_units = "Years".to_owned();
    let r = go(
        &m,
        &RunOptions {
            time_unit: Some("months".to_owned()),
            ..RunOptions::default()
        },
    );
    assert_eq!(r.time_unit, "month");
    assert!((r.times.last().copied().unwrap() - 120.0).abs() < 1e-9);
    assert!((r.stop - 120.0).abs() < 1e-9);
    let w = go(
        &m,
        &RunOptions {
            time_unit: Some("parsecs".to_owned()),
            ..RunOptions::default()
        },
    );
    assert!(
        w.warnings.iter().any(|x| x.contains("parsecs")),
        "{:?}",
        w.warnings
    );
    assert_eq!(w.times.last().copied(), Some(10.0));
}

#[test]
fn xmile_written_and_read_again_is_the_same_model() {
    let mut m = model(
        50.0,
        0.0,
        Method::Rk4,
        vec![
            non_negative(Variable::stock(
                "Stock A",
                "10 + b",
                &["in 1", "in2"],
                &["out"],
            ))
            .with_units("things"),
            Variable::flow("in 1", "IF b < 2 THEN 1 ELSE 0").with_units("things/day"),
            Variable::flow("in2", "SMTH3(b, 4)"),
            Variable::flow("out", "0.1 * \"Stock A\""),
            Variable::aux("b", "NORMAL(3, 1, 7, 2)"),
        ],
    );
    m.name = "Round & trip".to_owned();
    m.vendor = "Someone".to_owned();
    m.product = "Tool".to_owned();
    m.product_version = "1.2".to_owned();
    m.specs.dt = Step::Reciprocal(8.0);
    m.specs.time_units = "Days".to_owned();
    m.specs.save_step = Some(0.5);
    if let Some(v) = m.vars.get_mut(1) {
        v.doc = Some("a <doc> & more".to_owned());
    }
    let mut table = Variable::aux("lookup 1", "b");
    table.gf = Some(Gf {
        kind: GfKind::Discrete,
        x: GfX::Points(vec![0.0, 1.0, 4.0]),
        y: vec![1.0, 2.0, 2.0],
    });
    m.vars.push(table);
    let mut only = Variable::aux("only", "1");
    only.eqn = None;
    only.gf = Some(Gf {
        kind: GfKind::Extrapolate,
        x: GfX::Scale { min: 0.0, max: 2.0 },
        y: vec![0.0, 1.0, 4.0],
    });
    m.vars.push(only);
    // A model built in code has no diagram; it is written with one, and read back with it.
    m.layout = Layout::auto(&m);

    let text = write_xmile(&m);
    let back = read_xmile(&text).unwrap();
    assert_eq!(back.model, m, "{text}");
    assert!(back.notes.is_empty(), "{:?}", back.notes);
    // And writing that again gives the same text.
    assert_eq!(write_xmile(&back.model), text);
}

fn logistic_file() -> &'static str {
    include_str!("../../../tests/models/logistic.xmile")
}

fn sir_file() -> &'static str {
    include_str!("../../../tests/models/sir.xmile")
}

#[test]
fn the_logistic_file_runs_to_its_closed_form() {
    let f = read_xmile(logistic_file()).unwrap();
    assert_eq!(f.model.name, "Logistic population");
    assert_eq!(f.model.specs.method, Method::Rk4);
    assert_eq!(f.model.specs.dt, Step::Reciprocal(8.0));
    assert_eq!(f.model.specs.save_step, Some(1.0));
    // The file's one (empty) view is not a loss; every variable still has a place.
    assert!(f.notes.is_empty(), "{:?}", f.notes);
    assert!(f.model.layout.is_complete(&f.model));
    assert_eq!(f.model.layout.first_overlap(&f.model), None);
    let p = compile(&f.model).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let r = run(&p, &RunOptions::default()).unwrap();
    for (k, v) in mean(&r, "Population").iter().enumerate() {
        let exact = 1000.0 / (1.0 + 99.0 * exp(-0.5 * k as f64));
        assert!((v - exact).abs() / exact < 1e-6, "t={k}: {v} vs {exact}");
    }
    assert_eq!(
        r.series("Population").unwrap().units.as_deref(),
        Some("fish")
    );
}

#[test]
fn the_sir_file_matches_an_rk4_reference_with_a_small_step() {
    let f = read_xmile(sir_file()).unwrap();
    let p = compile(&f.model).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let r = run(&p, &RunOptions::default()).unwrap();
    assert_eq!(r.times.len(), 11);

    // The reference: the same equations by RK4 at a step of 0.001, written here.
    let (beta, gamma, n) = (0.4, 0.1, 1000.0);
    let f = |x: [f64; 3]| {
        let inf = beta * x[0] * x[1] / n;
        [-inf, inf - gamma * x[1], gamma * x[1]]
    };
    let add =
        |x: [f64; 3], k: [f64; 3], h: f64| [x[0] + h * k[0], x[1] + h * k[1], x[2] + h * k[2]];
    let dt = 0.001;
    let mut x = [990.0, 10.0, 0.0];
    let (s, i, rc) = (
        mean(&r, "Susceptible"),
        mean(&r, "Infected"),
        mean(&r, "Recovered"),
    );
    let mut worst = 0.0_f64;
    for step in 0..=100_000_u32 {
        if step.is_multiple_of(10_000) {
            let j = (step / 10_000) as usize;
            for (got, want) in [(&s, x[0]), (&i, x[1]), (&rc, x[2])] {
                worst = worst.max((got.get(j).copied().unwrap() - want).abs());
            }
        }
        let k1 = f(x);
        let k2 = f(add(x, k1, dt / 2.0));
        let k3 = f(add(x, k2, dt / 2.0));
        let k4 = f(add(x, k3, dt));
        x = [
            x[0] + dt / 6.0 * (k1[0] + 2.0 * k2[0] + 2.0 * k3[0] + k4[0]),
            x[1] + dt / 6.0 * (k1[1] + 2.0 * k2[1] + 2.0 * k3[1] + k4[1]),
            x[2] + dt / 6.0 * (k1[2] + 2.0 * k2[2] + 2.0 * k3[2] + k4[2]),
        ];
    }
    // The script gave a largest difference of 7.4e-5 people.
    assert!(worst < 3e-4, "{worst}");
    // Nobody is made or lost.
    for (k, ((a, b), c)) in s.iter().zip(&i).zip(&rc).enumerate() {
        let total = a + b + c;
        assert!((total - 1000.0).abs() < 1e-6, "step {k}: {total}");
    }
    // The epidemic peaks at about 406 (the script's).
    let peak = i.iter().copied().fold(0.0_f64, f64::max);
    assert!(peak > 100.0);
}

#[test]
fn entities_character_references_and_cdata_are_read_and_bad_files_are_errors() {
    let file = "<xmile version=\"1.0\"><header><name>R&amp;D &#38; &#x41;</name></header>\
        <sim_specs><start>0</start><stop>1</stop></sim_specs><model><variables>\
        <aux name=\"R&amp;D\"><eqn>IF 1 &lt; 2 THEN &#52; ELSE 0</eqn><doc>a &lt;b&gt;</doc></aux>\
        <aux name=\"c\"><eqn><![CDATA[IF 1 < 2 THEN 5 ELSE 0]]></eqn></aux>\
        </variables></model></xmile>";
    let f = read_xmile(file).unwrap();
    assert_eq!(f.model.name, "R&D & A");
    let first = f.model.vars.first().unwrap();
    assert_eq!(first.name, "R&D");
    assert_eq!(first.eqn.as_deref(), Some("IF 1 < 2 THEN 4 ELSE 0"));
    assert_eq!(first.doc.as_deref(), Some("a <b>"));
    assert_eq!(
        f.model.vars.get(1).unwrap().eqn.as_deref(),
        Some("IF 1 < 2 THEN 5 ELSE 0")
    );
    let r = once(&f.model);
    assert_eq!(last(&r, "R&D"), 4.0);
    assert_eq!(last(&r, "c"), 5.0);
    // Written and read again, the same.
    assert_eq!(read_xmile(&write_xmile(&f.model)).unwrap().model, f.model);

    let head = "<xmile version=\"1.0\"><sim_specs><start>0</start><stop>1</stop></sim_specs>";
    for bad in [
        "<xmile version=\"1.0\"><sim_specs>",
        "<xmile><a></b></xmile>",
        "<xmile a=1/>",
        "<xmile>&bogus;</xmile>",
        "<!DOCTYPE xmile [<!ENTITY x \"y\">]><xmile/>",
        "<xmile/><xmile/>",
        "not xml at all",
        "",
    ] {
        let e = read_xmile(bad).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Xml, "{bad}: {e}");
        assert!(!e.message.is_empty());
    }
    let e = read_xmile(&format!(
        "{head}<model><variables><aux name=\"x\"><eqn>1 &lt;</eqn></aux></variables></model></xmile>"
    ));
    // Well-formed, but not an equation: caught when the model is made ready.
    assert!(compile(&e.unwrap().model).is_err());
}

const DRAWN: &str = "<xmile version=\"1.0\"><sim_specs><start>0</start><stop>10</stop><dt>1</dt></sim_specs>    <model><variables>    <stock name=\"Pop\"><eqn>100</eqn><inflow>births</inflow></stock>    <flow name=\"births\"><eqn>rate * Pop</eqn></flow>    <aux name=\"rate\"><eqn>0.1</eqn></aux>    </variables><views>    <view type=\"stock_flow\" page_width=\"800\">    <stock name=\"Pop\" x=\"300\" y=\"200\" width=\"60\" height=\"40\" color=\"red\"/>    <flow name=\"births\" x=\"200\" y=\"220\"><pts><pt x=\"100\" y=\"220\"/><pt x=\"300\" y=\"220\"/></pts></flow>    <cloud uid=\"9\" flow_uid=\"3\" x=\"100\" y=\"220\"/>    <aux name=\"rate\" x=\"200\" y=\"120\"/>    <connector angle=\"45\"><from>rate</from><to>births</to></connector>    <connector><from>Pop</from><to>births</to></connector>    <connector><from>ghost</from><to>births</to></connector>    <alias uid=\"5\" x=\"1\" y=\"1\"><of>rate</of></alias>    </view>    <view type=\"stock_flow\"/><view type=\"interface\"/>    </views></model></xmile>";

#[test]
fn a_diagram_is_kept_through_a_read_and_a_write() {
    let f = read_xmile(DRAWN).unwrap();
    let l = &f.model.layout;
    // With a size, x and y are the top left corner; the layout holds the centre.
    let pop = &l.shapes["pop"];
    assert_eq!((pop.x, pop.y), (330.0, 220.0));
    assert_eq!((pop.width, pop.height), (Some(60.0), Some(40.0)));
    let births = &l.shapes["births"];
    assert_eq!((births.x, births.y), (200.0, 220.0));
    assert_eq!(births.pts, vec![(100.0, 220.0), (300.0, 220.0)]);
    // A shape with no size has the default size, centred where it is put.
    let rate = &l.shapes["rate"];
    assert_eq!((rate.x, rate.y), (200.0, 120.0));
    assert_eq!(l.connectors.len(), 2);
    assert_eq!(l.connectors.first().and_then(|c| c.angle), Some(45.0));
    for want in [
        "display attribute `color`",
        "diagram setting `page_width`",
        "aliases",
        "only the first diagram",
        "type `interface`",
        "not a variable",
    ] {
        assert!(
            f.notes.iter().any(|n| n.contains(want)),
            "{want}: {:?}",
            f.notes
        );
    }
    // Written and read again: the same positions, pipes and links, and the same text.
    let text = write_xmile(&f.model);
    let back = read_xmile(&text).unwrap();
    assert_eq!(back.model.layout, f.model.layout, "{text}");
    assert_eq!(back.model, f.model);
    assert!(back.notes.is_empty(), "{:?}", back.notes);
    assert_eq!(write_xmile(&back.model), text);
    // The cloud at the open end of the inflow is written.
    assert!(text.contains("<cloud "), "{text}");
}

#[test]
fn a_file_with_no_diagram_gets_one_with_nothing_overlapping() {
    let f = read_xmile(sir_file()).unwrap();
    let l = &f.model.layout;
    assert!(l.is_complete(&f.model));
    assert_eq!(l.first_overlap(&f.model), None);
    // The three stocks are in a row, left to right along the flows.
    let x = |n: &str| l.shapes[n].x;
    assert!(x("susceptible") < x("infected") && x("infected") < x("recovered"));
    // Reading again gives the same, and it is what a write-read gives.
    assert_eq!(read_xmile(sir_file()).unwrap().model.layout, *l);
    let back = read_xmile(&write_xmile(&f.model)).unwrap();
    assert_eq!(back.model.layout, *l);
}
