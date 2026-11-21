//! Monte Carlo models through the engine's commands.

use serde_json::{Value, json};
use stochcraft_engine::{Engine, ErrorCode};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

fn err(e: &mut Engine, command: &str, params: Value) -> ErrorCode {
    let r = e.execute(command, params.clone());
    assert!(r.is_err(), "{command} {params} should fail: {r:?}");
    r.err().map_or(ErrorCode::Internal, |e| e.code)
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("engine-mc-tests");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

/// A project's cost: labour hours at a rate, plus materials, with labour and materials
/// correlated.
fn cost_model() -> Engine {
    let mut e = Engine::new();
    ok(&mut e, "model.new", json!({ "name": "Project cost" }));
    ok(
        &mut e,
        "mc.input.set",
        json!({ "name": "labour", "distribution": { "type": "triangular", "min": 80.0, "mode": 100.0, "max": 150.0 }, "unit": "h" }),
    );
    ok(
        &mut e,
        "mc.input.set",
        json!({ "name": "rate", "distribution": { "type": "uniform", "min": 40.0, "max": 50.0 }, "unit": "USD/h" }),
    );
    ok(
        &mut e,
        "mc.input.set",
        json!({ "name": "materials", "distribution": { "type": "normal", "mean": 500.0, "std_dev": 50.0 }, "unit": "USD" }),
    );
    ok(
        &mut e,
        "mc.output.set",
        json!({ "name": "cost", "expression": "$labour * $rate + $materials", "unit": "USD", "at_most": 5500.0 }),
    );
    ok(
        &mut e,
        "mc.output.set",
        json!({ "name": "over", "expression": "$cost > 6000" }),
    );
    ok(
        &mut e,
        "mc.correlation.set",
        json!({ "a": "labour", "b": "materials", "rank": 0.6 }),
    );
    e
}

#[test]
fn a_monte_carlo_model_is_built_run_and_undone_by_commands() {
    let mut e = cost_model();
    let got = ok(&mut e, "mc.get", json!({}));
    assert_eq!(got["is_monte_carlo"], true);
    assert_eq!(
        got["trials"], 10_000,
        "a new Monte Carlo model runs 10,000 trials"
    );
    assert_eq!(got["problems"], json!([]));
    assert_eq!(got["monte_carlo"]["inputs"][0]["unit"], "h");

    let r = ok(&mut e, "mc.run", json!({ "trials": 20_000, "seed": 3 }));
    assert_eq!(r["trials"], 20_000);
    let cost = &r["outputs"][0];
    // Labour and rate are independent, so the mean is exactly E[labour] E[rate] + E[materials]
    // = 110 × 45 + 500 = 5450.
    let mean = cost["mean"].as_f64().unwrap_or(f64::NAN);
    let hw = cost["half_width"].as_f64().unwrap_or(f64::NAN);
    assert!((mean - 5_450.0).abs() < 2.6 * hw, "{mean} ± {hw}");
    assert!(cost["percentiles"]["95"].as_f64() > cost["percentiles"]["5"].as_f64());
    let t = &cost["target"];
    assert_eq!(t["at_most"], 5_500.0);
    assert_eq!(t["of"], 20_000);
    assert!(t["interval"][0].as_f64() < t["probability"].as_f64());
    // A condition is 1 or 0 in each trial, so its mean is a probability.
    let over = r["outputs"][1]["mean"].as_f64().unwrap_or(f64::NAN);
    assert!((0.05..0.5).contains(&over), "{over}");
    let achieved = r["correlations"][0]["achieved"]
        .as_f64()
        .unwrap_or(f64::NAN);
    assert!((achieved - 0.6).abs() < 0.03, "{achieved}");
    let counts: u64 = r["outputs"][0]["histogram"]["counts"]
        .as_array()
        .map_or(0, |c| c.iter().filter_map(Value::as_u64).sum());
    assert_eq!(counts, 20_000);
    assert_eq!(r["outputs"][0]["cdf"].as_array().map(Vec::len), Some(101));
    let record = &r["record"];
    assert_eq!(record["seed"], 3);
    assert_eq!(record["sampling"], "random");
    // A correlation makes the trials in a batch dependent: the intervals are across batches.
    assert_eq!(
        record["interval"],
        "Student-t across 20 independent batches of 1000 trials"
    );
    assert!(
        record["model_digest"]
            .as_str()
            .is_some_and(|d| d.starts_with("sha256:"))
    );

    // A process run is refused, saying what to use.
    assert_eq!(err(&mut e, "run", json!({})), ErrorCode::Unsupported);

    // Every edit undoes exactly, back to the empty model with its 10 replications.
    while e.execute("undo", json!({})).is_ok() {}
    assert_eq!(e.model().monte_carlo, None);
    assert_eq!(e.model().experiment.replications, 10);
}

#[test]
fn results_do_not_depend_on_the_number_of_threads() {
    let mut e = cost_model();
    ok(&mut e, "mc.set", json!({ "sampling": "latin_hypercube" }));
    let one = ok(&mut e, "mc.run", json!({ "trials": 5_500, "threads": 1 }));
    let four = ok(&mut e, "mc.run", json!({ "trials": 5_500, "threads": 4 }));
    assert_eq!(one, four);
    assert_eq!(one["record"]["sampling"], "latin_hypercube");
}

#[test]
fn a_sequential_run_says_why_it_stopped() {
    let mut e = cost_model();
    let r = ok(
        &mut e,
        "mc.run",
        json!({ "trials": 1_000, "stop": { "relative": 0.002, "max": 50_000 } }),
    );
    let stop = &r["stop"];
    assert!(
        stop["reason"] == "target_met" || stop["reason"] == "max_reached",
        "{stop}"
    );
    assert_eq!(
        stop["looks"].as_array().and_then(|l| l.last()),
        Some(&r["trials"])
    );
}

#[test]
fn bad_edits_are_refused() {
    let mut e = cost_model();
    assert_eq!(
        err(&mut e, "mc.input.set", json!({ "name": "new" })),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(
            &mut e,
            "mc.input.set",
            json!({ "name": "a-b", "distribution": { "type": "constant", "value": 1.0 } })
        ),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(
            &mut e,
            "mc.input.set",
            json!({ "name": "cost", "distribution": { "type": "constant", "value": 1.0 } })
        ),
        ErrorCode::Conflict
    );
    assert_eq!(
        err(
            &mut e,
            "mc.output.set",
            json!({ "name": "x", "expression": "1 +" })
        ),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(
            &mut e,
            "mc.correlation.set",
            json!({ "a": "labour", "b": "nobody", "rank": 0.5 })
        ),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(
            &mut e,
            "mc.correlation.set",
            json!({ "a": "labour", "b": "rate", "rank": 1.0 })
        ),
        ErrorCode::InvalidParams
    );
    // labour goes with materials (0.6) and with rate (0.5); rate strongly against materials
    // cannot also hold.
    ok(
        &mut e,
        "mc.correlation.set",
        json!({ "a": "labour", "b": "rate", "rank": 0.5 }),
    );
    let r = e.execute(
        "mc.correlation.set",
        json!({ "a": "rate", "b": "materials", "rank": -0.9 }),
    );
    let refused = r.err();
    assert!(
        refused.as_ref().is_some_and(
            |x| x.code == ErrorCode::InvalidParams && x.message.contains("positive definite")
        ),
        "{refused:?}"
    );
    // An output that uses one computed after it cannot run, and says why.
    ok(
        &mut e,
        "mc.output.set",
        json!({ "name": "early", "expression": "$cost * 2", "position": 0 }),
    );
    let r = e.execute("mc.run", json!({ "trials": 100 }));
    assert!(
        r.as_ref().is_err_and(|x| x.code == ErrorCode::InvalidModel
            && x.details
                .as_ref()
                .is_some_and(|d| d.to_string().contains("not_yet_computed"))),
        "{r:?}"
    );
}

#[test]
fn the_input_table_goes_out_and_back_as_csv() {
    let mut e = cost_model();
    let csv = ok(&mut e, "mc.inputs.export", json!({}))["csv"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(
        csv.starts_with("name,distribution,parameters,unit\n"),
        "{csv}"
    );
    assert!(
        csv.contains("labour,triangular,min=80;mode=100;max=150,h"),
        "{csv}"
    );
    let before = e.model().monte_carlo.clone();
    ok(
        &mut e,
        "mc.inputs.import",
        json!({ "csv": csv, "replace": true }),
    );
    assert_eq!(e.model().monte_carlo, before);
    assert_eq!(
        err(
            &mut e,
            "mc.inputs.import",
            json!({ "csv": "name,distribution\nx,normal\n" })
        ),
        ErrorCode::InvalidParams
    );
}

#[test]
fn a_monte_carlo_file_saves_opens_and_runs_the_same() {
    let mut e = cost_model();
    ok(
        &mut e,
        "mc.set",
        json!({ "sampling": "latin_hypercube", "trials": 3_000, "seed": 9 }),
    );
    let before = ok(&mut e, "mc.run", json!({}));
    let path = scratch("cost.bpmn");
    ok(
        &mut e,
        "file.save",
        json!({ "path": path.to_string_lossy() }),
    );
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(!text.contains("bpmn:process"), "{text}");
    assert!(text.contains("stochcraft:monteCarlo"), "{text}");

    let mut other = Engine::new();
    let opened = ok(
        &mut other,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    assert_eq!(opened["kind"], "monte_carlo");
    assert_eq!(opened["diagnostics"], json!([]));
    assert_eq!(other.model(), e.model());
    let after = ok(&mut other, "mc.run", json!({}));
    assert_eq!(after["outputs"], before["outputs"]);
    assert_eq!(
        after["record"]["model_digest"],
        before["record"]["model_digest"]
    );

    // Saved unchanged, the file is the same bytes; an edit is written into it.
    ok(&mut other, "file.save", json!({}));
    assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), text);
    ok(
        &mut other,
        "mc.output.set",
        json!({ "name": "cost", "at_most": null }),
    );
    ok(&mut other, "file.save", json!({}));
    let mut third = Engine::new();
    ok(
        &mut third,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    assert_eq!(third.model(), other.model());
}

/// The window's run, made a batch at a time between frames, gives `mc.run`'s results: with Latin
/// hypercube sampling and a correlation (whole batches), and with a sequential stop.
#[test]
fn a_run_made_a_batch_at_a_time_gives_mc_runs_results() {
    let mut e = cost_model();
    ok(&mut e, "mc.set", json!({ "sampling": "latin_hypercube" }));
    for params in [
        json!({ "trials": 4_500, "seed": 5 }),
        json!({ "trials": 1_000, "stop": { "relative": 0.002, "max": 30_000 } }),
    ] {
        let at_once = ok(&mut e, "mc.run", params.clone());
        let job = e.mc_start(params.clone());
        assert!(job.is_ok(), "{params}");
        let Ok(mut job) = job else { return };
        let mut steps = 0;
        while job.step(1).ok() == Some(false) && steps < 1_000 {
            let (made, asked) = job.progress();
            assert!(made <= asked && made > 0, "{made} of {asked}");
            steps += 1;
        }
        assert!(steps > 1, "{params}: in more than one step");
        let stepped = e.mc_results(&mut job).unwrap_or_default();
        assert_eq!(stepped["outputs"], at_once["outputs"], "{params}");
        assert_eq!(stepped["trials"], at_once["trials"]);
        assert_eq!(stepped["stop"], at_once["stop"]);
        assert_eq!(stepped["seed"], at_once["record"]["seed"]);
        assert_eq!(stepped["interval_basis"]["across"], "batches");
    }
    // A model with errors does not start.
    ok(
        &mut e,
        "mc.output.set",
        json!({ "name": "broken", "expression": "$nobody" }),
    );
    assert!(e.mc_start(json!({})).is_err());
}

/// The record says what the run was made from, which is the model as the run started, not as it
/// is when the results are collected.
#[test]
fn the_run_record_is_of_the_model_the_run_started_on() {
    let mut e = cost_model();
    let at_start = ok(&mut e, "mc.run", json!({ "trials": 100 }))["record"]["model_digest"].clone();
    let job = e.mc_start(json!({ "trials": 100 }));
    assert!(job.is_ok());
    let Ok(mut job) = job else { return };
    ok(
        &mut e,
        "mc.output.set",
        json!({ "name": "extra", "expression": "$cost + 1" }),
    );
    let mut steps = 0;
    while job.step(usize::MAX).ok() == Some(false) && steps < 1_000 {
        steps += 1;
    }
    let results = e.mc_results(&mut job).unwrap_or_default();
    assert_eq!(results["record"]["model_digest"], at_start);
    let now = ok(&mut e, "mc.run", json!({ "trials": 100 }))["record"]["model_digest"].clone();
    assert_ne!(now, at_start, "the edit changed the model");
}

/// Runs of Latin hypercube sampling and of correlated inputs are whole batches, so a most below
/// one batch is refused (not raised to 1,000); with plain sampling it stands.
#[test]
fn a_most_below_one_batch_is_refused_where_runs_are_whole_batches() {
    let mut e = cost_model();
    assert_eq!(
        err(&mut e, "mc.set", json!({ "stop": { "max": 500 } })),
        ErrorCode::InvalidParams
    );
    let started = e.mc_start(json!({ "stop": { "max": 500 } }));
    assert!(started.is_err(), "refused before it starts");
    ok(&mut e, "mc.set", json!({ "stop": { "max": 1_000 } }));
    // Without the correlation the sampling is plain, and 500 is fine until it is changed.
    ok(
        &mut e,
        "mc.correlation.set",
        json!({ "a": "labour", "b": "materials", "rank": 0.0 }),
    );
    ok(&mut e, "mc.set", json!({ "stop": { "max": 500 } }));
    assert_eq!(
        err(&mut e, "mc.set", json!({ "sampling": "latin_hypercube" })),
        ErrorCode::InvalidParams
    );
}

/// A model whose shares are known exactly: y = 2a + b with a and b independent uniforms on (0, 1)
/// has variance 4/12 + 1/12, so a accounts for 4/5 of it and b for 1/5, with no interactions.
fn linear_model() -> Engine {
    let mut e = Engine::new();
    ok(&mut e, "model.new", json!({ "name": "Linear" }));
    for name in ["a", "b"] {
        ok(
            &mut e,
            "mc.input.set",
            json!({ "name": name, "distribution": { "type": "uniform", "min": 0.0, "max": 1.0 } }),
        );
    }
    ok(
        &mut e,
        "mc.output.set",
        json!({ "name": "y", "expression": "2 * $a + $b" }),
    );
    e
}

/// The first and total index of `input` for output `output` in `mc.sobol`'s results.
fn shares(r: &Value, output: &str, input: &str) -> (f64, f64) {
    let o = r["outputs"]
        .as_array()
        .and_then(|os| os.iter().find(|o| o["name"] == output))
        .cloned()
        .unwrap_or_default();
    let i = o["inputs"]
        .as_array()
        .and_then(|is| is.iter().find(|i| i["input"] == input))
        .cloned()
        .unwrap_or_default();
    (
        i["first"]["value"].as_f64().unwrap_or(f64::NAN),
        i["total"]["value"].as_f64().unwrap_or(f64::NAN),
    )
}

/// `mc.sobol` on a linear model gives the exact shares, within several standard errors of the
/// estimate at 5,000 rows (about 0.01 for each), with intervals around them.
#[test]
fn sobol_shares_of_a_linear_model_are_exact() {
    let mut e = linear_model();
    let r = ok(
        &mut e,
        "mc.sobol",
        json!({ "rows": 5_000, "seed": 7, "bootstrap": 100 }),
    );
    assert_eq!(r["rows"], 5_000);
    assert_eq!(r["evaluations"], 20_000);
    assert_eq!(r["seed"], 7);
    assert_eq!(r["inputs"], json!(["a", "b"]));
    let near = |what: &str, got: f64, want: f64| {
        assert!(
            (got - want).abs() < 0.06,
            "{what}: got {got}, want {want}: {r}"
        );
    };
    let (a1, at) = shares(&r, "y", "a");
    let (b1, bt) = shares(&r, "y", "b");
    near("a alone", a1, 0.8);
    near("a total", at, 0.8);
    near("b alone", b1, 0.2);
    near("b total", bt, 0.2);
    let low = r["outputs"][0]["inputs"][0]["first"]["interval"][0].as_f64();
    let high = r["outputs"][0]["inputs"][0]["first"]["interval"][1].as_f64();
    assert!(
        matches!((low, high), (Some(l), Some(h)) if l < a1 + 0.02 && a1 - 0.02 < h && l < h),
        "{r}"
    );
    assert_eq!(r["outputs"][0]["rows_dropped"], 0);
    assert!(r["outputs"][0]["unavailable"].is_null());
}

/// Correlated inputs have no Sobol indices: the command says why. Numbers that cannot be used
/// are refused, and so is a model that is not a Monte Carlo one.
#[test]
fn sobol_refuses_correlated_inputs_and_bad_numbers() {
    let mut e = cost_model();
    let r = e.execute("mc.sobol", json!({}));
    let refused = r.err();
    assert!(
        refused
            .as_ref()
            .is_some_and(|x| x.code == ErrorCode::InvalidModel
                && x.message.contains("independent inputs")
                && x.message.contains("labour")),
        "{refused:?}"
    );
    assert!(e.sobol_correlated());
    ok(
        &mut e,
        "mc.correlation.set",
        json!({ "a": "labour", "b": "materials", "rank": 0.0 }),
    );
    assert!(!e.sobol_correlated());
    assert_eq!(
        err(&mut e, "mc.sobol", json!({ "rows": 5 })),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(&mut e, "mc.sobol", json!({ "rows": 10_000_000_000u64 })),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(&mut e, "mc.sobol", json!({ "bootstrap": 1_000_000 })),
        ErrorCode::InvalidParams
    );
    // Not a Monte Carlo model at all.
    assert_eq!(
        err(&mut Engine::new(), "mc.sobol", json!({})),
        ErrorCode::NotFound
    );
}

/// The run the window makes a piece at a time is `mc.sobol`'s: the same results however the
/// pieces are spread, and it is done when `step` says so.
#[test]
fn a_sobol_run_made_a_piece_at_a_time_gives_mc_sobols_results() {
    let mut e = linear_model();
    let params = json!({ "rows": 2_500, "seed": 3, "bootstrap": 20 });
    let blocking = ok(&mut e, "mc.sobol", params.clone());
    let mut job = e
        .sobol_start(params)
        .unwrap_or_else(|x| unreachable!("{x:?}"));
    assert_eq!(job.evaluations(), 10_000);
    assert!(job.progress() < 0.01);
    let mut steps = 0;
    while !job.step(1) {
        steps += 1;
        assert!(steps < 100, "never done");
    }
    // 3 batches of rows, the output (a step to start, then its bootstrap) and the end.
    assert!(steps + 1 >= 5, "{steps}");
    assert!((job.progress() - 1.0).abs() < 1e-12);
    assert_eq!(job.results(), Some(blocking));
    assert_eq!(job.results(), None);
    // Stepping a finished job changes nothing.
    assert!(job.step(5));
    assert_eq!(e.sobol_evaluations(), Some(4_000));
}

/// Antithetic pairs and Sobol points: `mc.set` takes them, the run says what its intervals are
/// across (pairs, or batches of randomised Sobol points), a run made a batch at a time gives
/// `mc.run`'s results, and antithetic pairs are refused with correlated inputs, which reorder
/// the trials.
#[test]
fn antithetic_and_sobol_sampling_run_and_say_what_their_intervals_are_across() {
    let mut e = cost_model();
    ok(&mut e, "mc.set", json!({ "sampling": "antithetic" }));
    assert_eq!(
        ok(&mut e, "mc.get", json!({}))["monte_carlo"]["sampling"],
        "antithetic"
    );
    // A correlation reorders each batch, which breaks the pairs: the run does not start.
    assert!(e.mc_start(json!({ "trials": 2_000 })).is_err());
    ok(
        &mut e,
        "mc.correlation.set",
        json!({ "a": "labour", "b": "materials", "rank": 0.0 }),
    );
    for (sampling, across, trials) in [("antithetic", "pairs", 4_501), ("sobol", "batches", 4_500)]
    {
        ok(&mut e, "mc.set", json!({ "sampling": sampling }));
        let params = json!({ "trials": trials, "seed": 5 });
        let at_once = ok(&mut e, "mc.run", params.clone());
        assert_eq!(at_once["sampling"], sampling);
        assert_eq!(at_once["interval_basis"]["across"], across, "{sampling}");
        assert_eq!(at_once["record"]["sampling"], sampling);
        let text = at_once["interval"].as_str().unwrap_or_default();
        assert!(
            text.contains(if sampling == "sobol" {
                "Sobol"
            } else {
                "antithetic"
            }),
            "{text}"
        );
        // Antithetic runs are whole pairs, Sobol runs whole batches.
        let made = at_once["trials"].as_u64().unwrap_or(0);
        assert_eq!(made, if sampling == "sobol" { 5_000 } else { 4_502 });
        let job = e.mc_start(params.clone());
        assert!(job.is_ok(), "{params}");
        let Ok(mut job) = job else { return };
        let mut steps = 0;
        while job.step(1).ok() == Some(false) && steps < 1_000 {
            steps += 1;
        }
        assert!(steps > 1, "{sampling}: in more than one step");
        let stepped = e.mc_results(&mut job).unwrap_or_default();
        assert_eq!(stepped["outputs"], at_once["outputs"], "{sampling}");
        assert_eq!(stepped["trials"], at_once["trials"]);
    }
}

/// y = 2a + b with a (uniform) as its control: the coefficient is 2, the controlled interval is
/// narrower (a accounts for 80% of y's variance), the plain results are as without the control,
/// a run made a batch at a time gives the same results, and bad controls are refused.
#[test]
fn controls_narrow_the_interval_and_bad_ones_are_refused() {
    let mut e = linear_model();
    let plain = ok(&mut e, "mc.run", json!({ "trials": 20_000, "seed": 4 }));
    assert!(plain["outputs"][0].get("controlled").is_none());

    ok(
        &mut e,
        "mc.output.set",
        json!({ "name": "y", "controls": [{ "input": "a" }] }),
    );
    let got = ok(&mut e, "mc.get", json!({}));
    assert_eq!(
        got["monte_carlo"]["outputs"][0]["controls"],
        json!([{ "input": "a" }])
    );
    let r = ok(&mut e, "mc.run", json!({ "trials": 20_000, "seed": 4 }));
    // Everything but the controlled estimate is what it was.
    let mut without = r["outputs"][0].clone();
    if let Some(o) = without.as_object_mut() {
        o.remove("controlled");
    }
    assert_eq!(without, plain["outputs"][0]);
    let c = &r["outputs"][0]["controlled"];
    assert_eq!(c["controls"][0]["name"], "a");
    assert_eq!(c["controls"][0]["kind"], "input");
    assert_eq!(c["controls"][0]["mean"], 0.5);
    let b = c["controls"][0]["coefficient"].as_f64().unwrap_or(f64::NAN);
    assert!((b - 2.0).abs() < 0.2, "coefficient {b}");
    assert_eq!(c["pilot_trials"], 1_000);
    let est = &c["estimate"];
    assert_eq!(est["trials"], 19_000);
    let mean = est["mean"].as_f64().unwrap_or(f64::NAN);
    assert!((mean - 1.5).abs() < 0.02, "mean {mean}");
    let narrowing = est["narrowing"].as_f64().unwrap_or(f64::NAN);
    assert!(
        (0.4..0.7).contains(&narrowing),
        "narrowing {narrowing} (about 0.55 expected)"
    );
    assert_eq!(est["helped"], true);

    // A run made a batch at a time is the same run.
    let job = e.mc_start(json!({ "trials": 20_000, "seed": 4 }));
    assert!(job.is_ok());
    let Ok(mut job) = job else { return };
    while job.step(3).ok() == Some(false) {}
    let stepped = e.mc_results(&mut job).unwrap_or_default();
    assert_eq!(stepped["outputs"], r["outputs"]);

    // The CSV has the controlled mean.
    let csv = stochcraft_engine::mc::results_csv(&r);
    assert!(
        csv.lines()
            .next()
            .is_some_and(|h| h.contains("controlled_mean"))
    );

    // A control that is not usable is refused, saying why.
    assert_eq!(
        err(
            &mut e,
            "mc.output.set",
            json!({ "name": "y", "controls": [{ "input": "nobody" }] })
        ),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(
            &mut e,
            "mc.output.set",
            json!({ "name": "y", "controls": [{ "output": "y", "expected": 1.5 }] })
        ),
        ErrorCode::InvalidParams
    );
    ok(
        &mut e,
        "mc.input.set",
        json!({ "name": "k", "distribution": { "type": "constant", "value": 3.0 } }),
    );
    assert_eq!(
        err(
            &mut e,
            "mc.output.set",
            json!({ "name": "y", "controls": [{ "input": "k" }] })
        ),
        ErrorCode::InvalidParams
    );
    // An output with a known mean is a control of a later output.
    ok(
        &mut e,
        "mc.output.set",
        json!({ "name": "z", "expression": "$y + $b", "controls": [{ "output": "y", "expected": 1.5 }] }),
    );
    // Removing what a control names removes the control.
    ok(&mut e, "mc.input.remove", json!({ "name": "a" }));
    let got = ok(&mut e, "mc.get", json!({}));
    assert!(
        got["monte_carlo"]["outputs"][0].get("controls").is_none(),
        "{got}"
    );
}
