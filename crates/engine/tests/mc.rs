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
