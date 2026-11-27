//! System dynamics models whose answers are known (ROADMAP M4.2's checks).
//!
//! The tolerances come from running the same stepping in a separate script (Euler or
//! fourth-order Runge-Kutta, the same pulse and the same order of updates) and leaving a margin
//! of about two over what it gave; each says which it is.

#![allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]

use stochcraft_random::math::{exp, powf};
use stochcraft_sd::{
    ErrorKind, Gf, GfKind, GfX, Layout, Method, RunOptions, SdJob, SdModel, SdResults, SimSpecs,
    Step, StopReason, StopWhen, VarKind, Variable, compile, read_xmile, run, write_xmile,
};

fn specs(stop: f64, dt: f64, method: Method) -> SimSpecs {
    SimSpecs {
        start: 0.0,
        stop,
        dt: Step::Value(dt),
        method,
        time_units: String::new(),
        save_step: None,
        stop_when: None,
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

fn stepped(m: &SdModel, opts: &RunOptions, at_a_time: usize) -> SdResults {
    let p = compile(m).unwrap();
    let mut job = SdJob::new(&p, opts).unwrap();
    let (done, total) = job.progress();
    assert_eq!(done, 0);
    let mut last_done = 0;
    while !job.is_done() {
        job.step(at_a_time).unwrap();
        let (done, total_now) = job.progress();
        assert_eq!(total_now, total);
        assert!(done > last_done && done <= total, "{done} of {total}");
        assert!(done - last_done <= at_a_time as u64);
        last_done = done;
    }
    assert_eq!(job.progress(), (total, total));
    job.finish().unwrap()
}

#[test]
fn a_job_stepped_a_few_replications_at_a_time_gives_what_run_gives() {
    let opts = RunOptions {
        replications: Some(23),
        seed: 5,
        ..RunOptions::default()
    };
    let whole = go(&walk(true), &opts);
    for at_a_time in [1, 7, 23, 1000] {
        assert_eq!(
            stepped(&walk(true), &opts, at_a_time),
            whole,
            "{at_a_time} at a time"
        );
    }
    // A model with no random function is one replication.
    let m = growth(0.25, Method::Rk4);
    let once = go(&m, &RunOptions::default());
    let job = stepped(&m, &RunOptions::default(), 1);
    assert_eq!(job, once);
    assert_eq!(job.replications, 1);
}

#[test]
fn the_halved_step_check_counts_in_the_jobs_progress_and_is_the_same() {
    let opts = RunOptions {
        replications: Some(6),
        seed: 3,
        dt_check: true,
        ..RunOptions::default()
    };
    let m = walk(false);
    let p = compile(&m).unwrap();
    let job = SdJob::new(&p, &opts).unwrap();
    assert_eq!(job.progress(), (0, 12));
    let whole = go(&m, &opts);
    assert!(whole.dt_check.is_some());
    assert_eq!(stepped(&m, &opts, 5), whole);
    assert_eq!(stepped(&m, &opts, 1), whole);
}

#[test]
fn a_job_refuses_what_run_refuses_and_finish_makes_what_is_left() {
    let p = compile(&walk(false)).unwrap();
    let bad = RunOptions {
        replications: Some(0),
        ..RunOptions::default()
    };
    assert!(SdJob::new(&p, &bad).is_err());
    assert!(run(&p, &bad).is_err());
    let opts = RunOptions {
        replications: Some(4),
        ..RunOptions::default()
    };
    let mut job = SdJob::new(&p, &opts).unwrap();
    job.step(1).unwrap();
    assert_eq!(job.progress(), (1, 4));
    assert_eq!(job.finish().unwrap(), run(&p, &opts).unwrap());
}

// Numerical contracts: what a flow that is not a number, or runs backwards, does to a stock that
// cannot go negative; the halved-step check compared by time; XMILE namespaces; time units;
// jumps under Runge-Kutta.

const XMILE_NS: &str = "http://docs.oasis-open.org/xmile/ns/XMILE/v1.0";

/// An XMILE file with the given method, stop, step and variables.
fn xmile_file(method: &str, stop: &str, dt: &str, variables: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><xmile version=\"1.0\" xmlns=\"{XMILE_NS}\"><header><name>t</name></header><sim_specs method=\"{method}\" time_units=\"Hours\"><start>0</start><stop>{stop}</stop><dt>{dt}</dt></sim_specs><model><variables>{variables}</variables></model></xmile>"
    )
}

fn from_file(text: &str) -> SdModel {
    read_xmile(text).unwrap().model
}

#[test]
fn a_rate_that_is_not_a_number_is_not_limited_into_a_transfer() {
    let text = xmile_file(
        "Euler",
        "1",
        "1",
        "<stock name=\"A\"><eqn>10</eqn><outflow>f</outflow><non_negative/></stock><stock name=\"B\"><eqn>0</eqn><inflow>f</inflow></stock><flow name=\"f\"><eqn>IF TIME >= 1 THEN SQRT(-1) ELSE 0</eqn></flow>",
    );
    let text = text.replace("<stop>1</stop>", "<stop>3</stop>");
    let r = once(&from_file(&text));
    // Not "A = 0, B = 10" from time 2: limited, the rate would be the 10 A holds, a real
    // transfer. It stays not a number, so the run ends at 0, the last step that is finite.
    assert_eq!(r.times, vec![0.0]);
    assert_eq!(last(&r, "A"), 10.0);
    assert_eq!(last(&r, "B"), 0.0);
    let s = r.stopped.unwrap();
    assert_eq!(s.time, 0.0);
    assert_eq!(
        s.reason,
        StopReason::NotFinite {
            variables: vec!["f".to_owned()],
            failed_at: 1.0
        }
    );
    // Not a number at the start: there is no step to show.
    let at_start = xmile_file(
        "Euler",
        "1",
        "1",
        "<stock name=\"A\"><eqn>10</eqn><outflow>f</outflow></stock><flow name=\"f\"><eqn>SQRT(-1)</eqn></flow>",
    );
    let e = run(
        &compile(&from_file(&at_start)).unwrap(),
        &RunOptions::default(),
    )
    .unwrap_err();
    assert_eq!(e.kind, ErrorKind::Run);
    assert!(
        e.message.contains("`f`") && e.message.contains("at the start"),
        "{e}"
    );
}

/// u' = u² from u0 is 1 / (1/u0 − t), which is infinite at 1/u0.
fn blow_up(dt: f64, stop_when: Option<StopWhen>) -> SdModel {
    let mut m = model(
        0.1,
        dt,
        Method::Rk4,
        vec![
            Variable::stock("u", "20", &["growth"], &[]),
            Variable::flow("growth", "u * u"),
        ],
    );
    m.specs.stop_when = stop_when;
    m
}

#[test]
fn a_run_stops_when_its_condition_holds_and_says_when_it_crossed() {
    let dt = 2.5e-5;
    for threshold in [1e3, 1e4] {
        let w = StopWhen {
            variable: "U".to_owned(),
            above: true,
            value: threshold,
        };
        let r = once(&blow_up(dt, Some(w)));
        let s = r.stopped.clone().unwrap();
        assert_eq!(s.reason, StopReason::Condition);
        assert_eq!(s.runs, 1);
        // The exact crossing of u = U is 1/u0 − 1/U. RK4 at this step is far closer than the
        // step itself (the script gives an error under 1e-8); the interpolation adds well under
        // one step.
        let exact = 1.0 / 20.0 - 1.0 / threshold;
        let crossing = s.crossing.unwrap();
        assert!(
            (crossing - exact).abs() < 1e-7,
            "{crossing} against {exact}"
        );
        // It stops at the first step past it, which is the last saved time, and the value there
        // is past the threshold while the step before is not.
        assert!(s.time >= crossing && s.time - crossing < dt * (1.0 + 1e-9));
        assert_eq!(r.times.last().copied(), Some(s.time));
        let u = mean(&r, "u");
        assert!(u.last().copied().unwrap() > threshold);
        // So crossing + 1/U is the blow-up time 1/u0, near 0.05.
        assert!((crossing + 1.0 / threshold - 0.05).abs() < 1e-7);
    }
    // The variable named by any case or spacing; below a value; held at the start.
    let w = StopWhen {
        variable: "u".to_owned(),
        above: false,
        value: 30.0,
    };
    let r = once(&blow_up(dt, Some(w)));
    let s = r.stopped.unwrap();
    assert_eq!((s.time, s.crossing), (0.0, Some(0.0)));
    assert_eq!(r.times, vec![0.0]);
}

#[test]
fn a_stop_condition_on_no_variable_is_refused() {
    let w = StopWhen {
        variable: "v".to_owned(),
        above: true,
        value: 1.0,
    };
    let e = compile(&blow_up(1e-3, Some(w))).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Model);
    assert!(e.message.contains("`v`"), "{e}");
}

#[test]
fn a_flow_that_overflows_stops_the_run_at_the_last_finite_step() {
    // No condition: u' = u² runs past 1/u0 = 0.05 at a step of 1e-3, where the values grow
    // without bound until they overflow. The results end at the last step where u and its
    // flow were both finite numbers, and none of them is infinite or not a number.
    let dt = 1e-3;
    let r = once(&blow_up(dt, None));
    let s = r.stopped.clone().unwrap();
    let StopReason::NotFinite {
        variables,
        failed_at,
    } = &s.reason
    else {
        panic!("{s:?}")
    };
    assert!(variables.contains(&"growth".to_owned()), "{variables:?}");
    assert!((failed_at - s.time - dt).abs() < 1e-12);
    assert!(s.time > 0.05 && s.time < 0.1, "{}", s.time);
    assert_eq!(r.times.last().copied(), Some(s.time));
    for v in ["u", "growth"] {
        assert!(mean(&r, v).iter().all(|x| x.is_finite()), "{v}");
    }
    assert!(
        !r.warnings.iter().any(|w| w.contains("not a finite number")),
        "{:?}",
        r.warnings
    );
    // In another time unit the times are converted too.
    let mut m = blow_up(dt, None);
    m.specs.time_units = "Hours".to_owned();
    let r = go(
        &m,
        &RunOptions {
            time_unit: Some("Minutes".to_owned()),
            ..RunOptions::default()
        },
    );
    let t = r.stopped.unwrap().time;
    assert!((t - 60.0 * s.time).abs() < 1e-9, "{t}");
}

#[test]
fn a_stop_condition_is_kept_through_a_write_and_a_read() {
    let w = StopWhen {
        variable: "u".to_owned(),
        above: true,
        value: 1000.0,
    };
    let m = blow_up(2.5e-5, Some(w.clone()));
    let text = write_xmile(&m);
    assert!(
        text.contains("<stochcraft:stop_when variable=\"u\" above=\"1000\"/>"),
        "{text}"
    );
    let back = read_xmile(&text).unwrap();
    assert_eq!(back.model.specs.stop_when, Some(w));
    assert!(back.notes.is_empty(), "{:?}", back.notes);
    let below = text.replace("above=", "below=");
    let b = read_xmile(&below).unwrap().model.specs.stop_when.unwrap();
    assert!(!b.above);
    // Both, neither, or a value that is not a number: the file is refused.
    for bad in [
        text.replace("above=\"1000\"", "above=\"1\" below=\"2\""),
        text.replace(" above=\"1000\"", ""),
        text.replace("above=\"1000\"", "above=\"many\""),
    ] {
        assert_eq!(read_xmile(&bad).unwrap_err().kind, ErrorKind::Xml, "{bad}");
    }
}

#[test]
fn a_stop_condition_follows_a_rename_and_holds_back_a_removal() {
    let w = StopWhen {
        variable: "growth".to_owned(),
        above: true,
        value: 1000.0,
    };
    let mut m = blow_up(2.5e-5, Some(w));
    stochcraft_sd::rename_variable(&mut m, "growth", "Heat flow").unwrap();
    assert_eq!(m.specs.stop_when.as_ref().unwrap().variable, "Heat flow");
    // No equation names the flow, and `force` would take it out of the stock's inflows: only
    // the stop condition holds the removal back.
    let e = stochcraft_sd::remove_variable(&mut m, "heat_flow", true).unwrap_err();
    assert!(e.message.contains("stop condition"), "{e}");
}

#[test]
fn runs_in_replications_that_end_early_are_shown_to_the_last_time_all_reached() {
    // A random growth rate: each run crosses 100 at its own time. The results end at the last
    // saved time every run reached, and the earliest stop is reported, with how many ended.
    let mut m = model(
        10.0,
        0.01,
        Method::Euler,
        vec![
            Variable::stock("x", "1", &["g"], &[]),
            Variable::flow("g", "r * x"),
            Variable::aux("r", "UNIFORM(0.5, 1.5, 0, 100)"),
        ],
    );
    m.specs.save_step = Some(0.1);
    m.specs.stop_when = Some(StopWhen {
        variable: "x".to_owned(),
        above: true,
        value: 100.0,
    });
    let r = go(
        &m,
        &RunOptions {
            replications: Some(20),
            seed: 3,
            ..RunOptions::default()
        },
    );
    let s = r.stopped.clone().unwrap();
    assert_eq!(s.runs, 20);
    let end = r.times.last().copied().unwrap();
    assert!(end <= s.time + 1e-12, "{end} after {}", s.time);
    // Every row is a time on the saving interval that each run had.
    for t in &r.times {
        assert!(((t / 0.1).round() * 0.1 - t).abs() < 1e-9, "{t}");
    }
    assert_eq!(mean(&r, "x").len(), r.times.len());
    assert!(
        r.warnings.iter().any(|w| w.contains("20 of 20 runs ended")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn a_flow_that_runs_backwards_is_limited_by_the_stock_it_takes_from() {
    // Declared from B to A, but it comes out as -2: A, which holds 1 and cannot go negative,
    // is the one that gives, so only 1 moves: (A, B) = (0, 1), not (-1, 2).
    let text = xmile_file(
        "Euler",
        "1",
        "1",
        "<stock name=\"A\"><eqn>1</eqn><inflow>f</inflow><non_negative/></stock><stock name=\"B\"><eqn>0</eqn><outflow>f</outflow></stock><flow name=\"f\"><eqn>-2</eqn></flow>",
    );
    let r = once(&from_file(&text));
    assert_eq!(last(&r, "A"), 0.0);
    assert_eq!(last(&r, "B"), 1.0);
    // At the end A is empty, so the flow it would give is limited to nothing.
    assert_eq!(last(&r, "f"), 0.0);
    // Two flows between the same stocks, both negative: A gives to B through `g` (3 asked,
    // only the 1 it holds at the start of the step), and takes 2 from B through `f`.
    let text = xmile_file(
        "Euler",
        "1",
        "1",
        "<stock name=\"A\"><eqn>1</eqn><outflow>f</outflow><inflow>g</inflow><non_negative/></stock><stock name=\"B\"><eqn>0</eqn><outflow>g</outflow><inflow>f</inflow></stock><flow name=\"f\"><eqn>-2</eqn></flow><flow name=\"g\"><eqn>-3</eqn></flow>",
    );
    let r = once(&from_file(&text));
    assert!(last(&r, "A") >= 0.0, "{}", last(&r, "A"));
    assert!((last(&r, "A") + last(&r, "B") - 1.0).abs() < 1e-12);
}

#[test]
fn a_flow_listed_twice_at_a_stock_is_refused() {
    let text = xmile_file(
        "Euler",
        "1",
        "1",
        "<stock name=\"A\"><eqn>10</eqn><outflow>f</outflow></stock><stock name=\"B\"><eqn>0</eqn><inflow>f</inflow><inflow>f</inflow></stock><flow name=\"f\"><eqn>1</eqn></flow>",
    );
    let e = compile(&from_file(&text)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Model);
    assert!(e.message.contains("more than once"), "{}", e.message);
}

fn check_of(text: &str) -> stochcraft_sd::DtCheck {
    let r = go(
        &from_file(text),
        &RunOptions {
            dt_check: true,
            ..RunOptions::default()
        },
    );
    r.dt_check.unwrap()
}

#[test]
fn the_halved_step_check_compares_by_time_and_an_exact_case_passes() {
    // X' = 1 is integrated exactly at any step; the run stops at 2 (1.6 rounded to whole steps),
    // and the halved run goes to the same time.
    let text = xmile_file(
        "Euler",
        "1.6",
        "1",
        "<stock name=\"s\"><eqn>0</eqn><inflow>f</inflow></stock><flow name=\"f\"><eqn>1</eqn></flow>",
    );
    let c = check_of(&text);
    assert!(!c.inconclusive);
    assert!(c.within_threshold);
    assert!(c.max_relative_difference.abs() < 1e-12, "{c:?}");
    let s = c.stocks.first().unwrap();
    assert_eq!(s.compared, 2);
    assert!(c.draws_matched);
}

#[test]
fn the_halved_step_check_is_inconclusive_when_a_value_is_not_a_number() {
    // The rate is not a number from time 1, so the run at the step ends at 0 and the one at half
    // of it at 0.5: they share no time after the start to compare.
    let text = xmile_file(
        "Euler",
        "2",
        "1",
        "<stock name=\"s\"><eqn>0</eqn><inflow>f</inflow></stock><flow name=\"f\"><eqn>IF TIME >= 1 THEN SQRT(-1) ELSE 1</eqn></flow>",
    );
    let c = check_of(&text);
    assert!(c.inconclusive);
    assert!(!c.within_threshold);
    assert!(c.stocks.first().unwrap().inconclusive);
    assert!(!c.notes.is_empty());
}

#[test]
fn the_halved_step_check_claims_the_same_draws_only_when_the_clocks_agree() {
    // A draw every 1.4 hours is every whole step: 1 step at 1 hour, 3 at half an hour (1.5
    // hours), so the draws are at different times.
    let draws = |every: &str| {
        xmile_file(
            "Euler",
            "10",
            "1",
            &format!(
                "<stock name=\"s\"><eqn>0</eqn><inflow>r</inflow></stock><flow name=\"r\"><eqn>RANDOM(0,1,0,{every})</eqn></flow>"
            ),
        )
    };
    let c = check_of(&draws("1.4"));
    assert!(!c.draws_matched);
    assert!(
        c.notes.iter().any(|n| n.contains("random")),
        "{:?}",
        c.notes
    );
    // An interval of a whole number of both steps agrees, at either method.
    assert!(check_of(&draws("2")).draws_matched);
    assert!(check_of(&draws("2").replace("\"Euler\"", "\"RK4\"")).draws_matched);
    // Equal intervals are not enough when a condition decides when the first draw is made: at a
    // step of 1 it is at 1, at half of it at 0.5, and with parameters taken then.
    let conditional = xmile_file(
        "Euler",
        "10",
        "1",
        "<stock name=\"s\"><eqn>0</eqn><inflow>r</inflow></stock><flow name=\"r\"><eqn>IF TIME >= 0.5 THEN RANDOM(TIME,TIME+1,0,2) ELSE 0</eqn></flow>",
    );
    let c = check_of(&conditional);
    assert!(!c.draws_matched, "{c:?}");
    assert!(
        c.notes.iter().any(|n| n.contains("random")),
        "{:?}",
        c.notes
    );
}

#[test]
fn xmile_namespaces_are_resolved_by_uri_not_by_prefix() {
    let prefixed = "<?xml version=\"1.0\"?><xm:xmile version=\"1.0\" xmlns:xm=\"http://docs.oasis-open.org/xmile/ns/XMILE/v1.0\"><xm:header><xm:name>p</xm:name></xm:header><xm:sim_specs method=\"Euler\" time_units=\"Hours\"><xm:start>0</xm:start><xm:stop>2</xm:stop><xm:dt>1</xm:dt></xm:sim_specs><xm:model><xm:variables><xm:stock name=\"s\"><xm:eqn>0</xm:eqn><xm:inflow>f</xm:inflow></xm:stock><xm:flow name=\"f\"><xm:eqn>1</xm:eqn></xm:flow></xm:variables></xm:model></xm:xmile>";
    let f = read_xmile(prefixed).unwrap();
    assert_eq!(f.model.name, "p");
    assert_eq!(last(&once(&f.model), "s"), 2.0);
    // A prefix is not enough: it has to be bound to the XMILE namespace.
    let elsewhere = prefixed.replace(XMILE_NS, "urn:not-xmile");
    assert_eq!(read_xmile(&elsewhere).unwrap_err().kind, ErrorKind::Xml);
    // Nor is a default namespace that is another one.
    let wrong = xmile_file("Euler", "2", "1", "").replace(XMILE_NS, "urn:not-xmile");
    let e = read_xmile(&wrong).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Xml);
    assert!(e.message.contains("namespace"), "{}", e.message);
    // An empty default namespace is no namespace, which a file may leave out.
    let none = xmile_file(
        "Euler",
        "2",
        "1",
        "<stock name=\"s\"><eqn>0</eqn><inflow>f</inflow></stock><flow name=\"f\"><eqn>1</eqn></flow>",
    )
    .replace(XMILE_NS, "");
    assert_eq!(last(&once(&read_xmile(&none).unwrap().model), "s"), 2.0);
}

#[test]
fn a_change_of_time_unit_converts_the_steps_too() {
    let text = xmile_file(
        "Euler",
        "2",
        "1",
        "<stock name=\"s\"><eqn>0</eqn><inflow>f</inflow></stock><flow name=\"f\"><eqn>1</eqn></flow>",
    );
    let r = go(
        &from_file(&text),
        &RunOptions {
            dt_check: true,
            time_unit: Some("Minutes".to_owned()),
            ..RunOptions::default()
        },
    );
    assert_eq!(r.times, vec![0.0, 60.0, 120.0]);
    assert_eq!(r.save_step, 60.0);
    assert_eq!(r.dt, 60.0);
    let c = r.dt_check.unwrap();
    assert_eq!((c.dt, c.half_dt), (60.0, 30.0));
}

#[test]
fn a_step_on_a_step_boundary_is_integrated_the_same_by_both_methods() {
    // s' = STEP(6, 1): s is 0 up to 1 and 6 (t - 1) after, so 6 at t = 2. A fourth-order step
    // takes the value from the left at its end, so the jump at 1 is not counted in [0, 1].
    for method in ["Euler", "RK4"] {
        let text = xmile_file(
            method,
            "2",
            "1",
            "<stock name=\"s\"><eqn>0</eqn><inflow>f</inflow></stock><flow name=\"f\"><eqn>STEP(6,1)</eqn></flow>",
        );
        let r = once(&from_file(&text));
        assert_eq!(mean(&r, "s"), vec![0.0, 0.0, 6.0], "{method}");
    }
}

#[test]
fn a_step_whose_time_moves_with_the_clock_stays_on_at_every_stage() {
    // s' = STEP(6, TIME) is 6 at every time, so s = 6 t. Taking the call from the left at the end
    // of a fourth-order step must not drop that stage to 0.
    for method in ["Euler", "RK4"] {
        let text = xmile_file(
            method,
            "2",
            "1",
            "<stock name=\"s\"><eqn>0</eqn><inflow>f</inflow></stock><flow name=\"f\"><eqn>STEP(6,TIME)</eqn></flow>",
        );
        let r = once(&from_file(&text));
        assert_eq!(mean(&r, "s"), vec![0.0, 6.0, 12.0], "{method}");
    }
}

fn heat_rod() -> SdModel {
    let f = read_xmile(include_str!("../../../tests/models/heat-rod.xmile")).unwrap();
    assert!(f.notes.is_empty(), "{:?}", f.notes);
    f.model
}

/// The heat rod with its source set to `source` (1, as the file has it, or 0).
fn heat_rod_with(source: &str) -> SdModel {
    let mut m = heat_rod();
    let v = m.vars.iter_mut().find(|v| v.name == "source").unwrap();
    v.eqn = Some(source.to_owned());
    m
}

#[test]
fn the_heat_rod_without_its_source_cools_as_its_sine_mode() {
    // u_t = u_xx on (0, 1), held at 0 at the ends, by the method of lines: 19 stocks at
    // x_i = i h, h = 0.05. The sine 20 sin(pi x_i) is an eigenvector of the second difference,
    // with eigenvalue -lambda1, lambda1 = (4 / h^2) sin^2(pi h / 2), so each stock is
    // 20 sin(pi x_i) e^(-lambda1 t) exactly; RK4 at a step of 2.5e-5 is within about 1e-14 of it.
    let p = compile(&heat_rod_with("0")).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let r = run(&p, &RunOptions::default()).unwrap();
    assert_eq!(r.stopped, None);
    let h = 0.05;
    let pi = std::f64::consts::PI;
    let s = stochcraft_random::math::sin(pi * h / 2.0);
    let lambda1 = 4.0 / (h * h) * s * s;
    for at in [0.02, 0.1] {
        let k = r
            .times
            .iter()
            .position(|t| (t - at).abs() < 1e-9)
            .unwrap_or_else(|| panic!("no saved time {at}"));
        for i in 1..=19 {
            let x = i as f64 * h;
            let exact = 20.0 * stochcraft_random::math::sin(pi * x) * exp(-lambda1 * at);
            let got = mean(&r, &format!("u{i}"))[k];
            assert!(
                (got - exact).abs() < 1e-9,
                "u{i} at {at}: {got} against {exact}"
            );
        }
        // The rod itself (the PDE) decays at pi^2, a little faster than the 19 points do: the
        // midpoint is 20 e^(-pi^2 t), and the difference is the grid's O(h^2) error, a relative
        // e^((pi^2 - lambda1) t) - 1, 0.04% at 0.02 and 0.2% at 0.1.
        let pde = 20.0 * exp(-pi * pi * at);
        let u10 = mean(&r, "u10")[k];
        let grid = exp((pi * pi - lambda1) * at) - 1.0;
        assert!(
            ((u10 - pde) / pde - grid).abs() < 1e-9,
            "{u10} against {pde}"
        );
        assert!(grid > 0.0 && grid < 2.1e-3, "{grid}");
    }
}

#[test]
fn the_heated_rod_runs_away_near_the_published_blow_up_time() {
    // With the source, u_t = u_xx + u^2 from 20 sin(pi x) blows up in finite time; Soheili and
    // Stockie (Applied Numerical Mathematics 58(3), 2008, p. 12) give t* = 0.08243786. The run
    // stops when the midpoint u10 passes 10,000; near blow-up u' is close to u^2, whose solution
    // reaches infinity 1/U after it passes U, so the crossing time plus 1/U estimates t*. On 19
    // points the grid's error is about 0.25% (early): within 0.3%.
    let p = compile(&heat_rod()).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let r = run(&p, &RunOptions::default()).unwrap();
    let s = r.stopped.clone().unwrap();
    assert_eq!(s.reason, StopReason::Condition);
    let threshold = p.stop_when().unwrap().value;
    assert_eq!(threshold, 1e4);
    let crossing = s.crossing.unwrap();
    let estimate = crossing + 1.0 / threshold;
    let published = 0.082_437_86;
    let off = (estimate - published) / published;
    assert!(off.abs() < 3e-3, "{estimate}: {off}");
    // The run ends at the step past the crossing, with every value finite.
    assert!(s.time >= crossing && s.time - crossing < 2.5e-5 * 1.001);
    assert_eq!(r.times.last().copied(), Some(s.time));
    assert!(mean(&r, "u10").last().copied().unwrap() > threshold);
    for v in &r.series {
        assert!(v.mean.iter().all(|x| x.is_finite()), "{}", v.name);
    }
    // The midpoint is the hottest point throughout (the rod is symmetric about it).
    let u10 = mean(&r, "u10");
    for i in [1, 5, 9, 11, 19] {
        let u = mean(&r, &format!("u{i}"));
        assert!(u.iter().zip(&u10).all(|(a, b)| a <= b), "u{i}");
    }
}

#[test]
fn one_stock_squared_crosses_where_its_exact_solution_does() {
    // The anchor of the rod's estimate: u' = u^2 from 20 is 1 / (0.05 - t), which passes U at
    // 0.05 - 1/U, so the crossing plus 1/U is the blow-up time 0.05.
    let w = StopWhen {
        variable: "u".to_owned(),
        above: true,
        value: 1e4,
    };
    let r = once(&blow_up(2.5e-5, Some(w)));
    let crossing = r.stopped.unwrap().crossing.unwrap();
    assert!((crossing - (0.05 - 1e-4)).abs() < 1e-7, "{crossing}");
}
