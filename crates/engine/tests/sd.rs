//! System dynamics models through the engine's commands.

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

fn model_path(name: &str) -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/models")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect()
}

static NULL: Value = Value::Null;

fn variable<'a>(results: &'a Value, name: &str) -> &'a Value {
    results["variables"]
        .as_array()
        .and_then(|vs| vs.iter().find(|v| v["name"] == name))
        .unwrap_or(&NULL)
}

#[test]
fn file_open_reads_an_xmile_file_and_sd_run_runs_it() {
    let mut e = Engine::new();
    let opened = ok(
        &mut e,
        "file.open",
        json!({ "path": model_path("logistic.xmile") }),
    );
    assert_eq!(opened["kind"], "system_dynamics");
    assert_eq!(opened["format"], "xmile");
    assert_eq!(opened["model"]["name"], "Logistic population");
    assert_eq!(
        opened["model"]["variables"].as_array().map(Vec::len),
        Some(4)
    );
    // The file's empty diagram is no loss: every variable has a place.
    assert_eq!(opened["notes"].as_array().map(Vec::len), Some(0));
    assert!(
        opened["model"]["variables"]
            .as_array()
            .is_some_and(|vs| vs.iter().all(|v| v["x"].is_number() && v["y"].is_number()))
    );

    let r = ok(&mut e, "sd.run", json!({}));
    assert_eq!(r["replications"], 1);
    assert_eq!(r["random"], false);
    assert_eq!(r["method"], "rk4");
    assert_eq!(r["time_unit"], "Years");
    let pop = floats(&variable(&r, "Population")["mean"]);
    assert_eq!(pop.len(), 21);
    let exact = 1000.0 / (1.0 + 99.0 * stochcraft_random::math::exp(-0.5 * 20.0));
    assert!((pop.last().copied().unwrap_or_default() - exact).abs() / exact < 1e-6);
    assert_eq!(variable(&r, "Population")["final"], json!(pop.last()));
    assert!(
        r["record"]["model_digest"]
            .as_str()
            .is_some_and(|d| d.starts_with("sha256:"))
    );
    assert!(
        r["record"]["file"]
            .as_str()
            .is_some_and(|f| f.ends_with("logistic.xmile"))
    );

    // sd.get describes the model, and with a variable gives its values from the last run.
    let g = ok(&mut e, "sd.get", json!({ "variable": "population" }));
    assert_eq!(g["values"]["mean"], variable(&r, "Population")["mean"]);
    assert_eq!(g["variables"][0]["kind"], "stock");
    assert_eq!(g["variables"][0]["inflows"], json!(["net growth"]));
    assert_eq!(
        err(&mut e, "sd.get", json!({ "variable": "nope" })),
        ErrorCode::NotFound
    );
}

#[test]
fn a_system_dynamics_document_is_refused_by_the_process_commands() {
    let mut e = Engine::new();
    assert_eq!(err(&mut e, "sd.run", json!({})), ErrorCode::NotFound);
    ok(
        &mut e,
        "file.open",
        json!({ "path": model_path("sir.xmile") }),
    );
    assert_eq!(err(&mut e, "run", json!({})), ErrorCode::Unsupported);
    assert_eq!(err(&mut e, "run.start", json!({})), ErrorCode::Unsupported);
    // Opening a process model replaces it, and then sd.run has nothing to run.
    ok(&mut e, "model.new", json!({ "name": "x" }));
    assert_eq!(err(&mut e, "sd.run", json!({})), ErrorCode::NotFound);
}

#[test]
fn sd_open_takes_the_text_and_sd_run_checks_the_step_and_replicates_noise() {
    let mut e = Engine::new();
    let text = std::fs::read_to_string(model_path("sir.xmile")).unwrap_or_default();
    let opened = ok(
        &mut e,
        "sd.open",
        json!({ "text": text, "name": "sir.xmile" }),
    );
    assert_eq!(opened["path"], "sir.xmile");
    let r = ok(&mut e, "sd.run", json!({ "dt_check": true, "dt": 1.0 }));
    assert_eq!(r["dt"], 1.0);
    let check = &r["dt_check"];
    assert_eq!(check["half_dt"], 0.5);
    assert_eq!(check["stocks"].as_array().map(Vec::len), Some(3));
    assert!(
        check["max_relative_difference"]
            .as_f64()
            .is_some_and(|x| x >= 0.0)
    );
    // The journal starts at the opened file and holds the run, which replay skips.
    assert_eq!(e.journal().len(), 1);

    let noise = "<xmile version=\"1.0\"><sim_specs><start>0</start><stop>50</stop><dt>1</dt></sim_specs><model><variables><stock name=\"w\"><eqn>0</eqn><inflow>n</inflow></stock><flow name=\"n\"><eqn>NORMAL(0, 1)</eqn></flow></variables></model></xmile>";
    ok(&mut e, "sd.open", json!({ "text": noise }));
    let a = ok(&mut e, "sd.run", json!({ "replications": 30, "seed": 5 }));
    let b = ok(&mut e, "sd.run", json!({ "replications": 30, "seed": 5 }));
    assert_eq!(a["variables"], b["variables"]);
    assert_eq!(a["replications"], 30);
    let w = variable(&a, "w");
    for key in ["mean", "median", "p5", "p95"] {
        assert_eq!(floats(&w[key]).len(), 51, "{key}");
    }
}

#[test]
fn what_cannot_be_read_is_refused_with_a_code_and_the_name() {
    let mut e = Engine::new();
    let arrays = "<xmile version=\"1.0\"><header><options><uses_arrays/></options></header><sim_specs><start>0</start><stop>1</stop></sim_specs><model><variables/></model></xmile>";
    let r = e.execute("sd.open", json!({ "text": arrays }));
    let c = r.err();
    assert_eq!(c.as_ref().map(|c| c.code), Some(ErrorCode::Unsupported));
    assert!(c.is_some_and(|c| c.message.contains("arrays")));

    let looped = "<xmile version=\"1.0\"><sim_specs><start>0</start><stop>1</stop></sim_specs><model><variables><aux name=\"a\"><eqn>b</eqn></aux><aux name=\"b\"><eqn>a</eqn></aux></variables></model></xmile>";
    let r = e.execute("sd.open", json!({ "text": looped }));
    let c = r.err();
    assert_eq!(c.as_ref().map(|c| c.code), Some(ErrorCode::InvalidModel));
    assert!(c.is_some_and(|c| c.message.contains("a -> b -> a")));

    assert_eq!(
        err(&mut e, "sd.open", json!({ "text": "<xmile><a></xmile>" })),
        ErrorCode::BadFile
    );
    // A refused file leaves what was open as it was.
    ok(
        &mut e,
        "file.open",
        json!({ "path": model_path("sir.xmile") }),
    );
    assert_eq!(
        err(&mut e, "sd.open", json!({ "text": "<xmile><a></xmile>" })),
        ErrorCode::BadFile
    );
    ok(&mut e, "sd.run", json!({}));
}

#[test]
fn results_come_out_as_csv() {
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": model_path("sir.xmile") }),
    );
    let r = ok(&mut e, "sd.run", json!({}));
    let csv = stochcraft_engine::sd::results_csv(&r);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 12);
    assert_eq!(
        lines.first().copied(),
        Some(
            "time (Days),Susceptible,Infected,Recovered,infection,recovery,contact rate,recovery rate,population"
        )
    );
    assert!(lines.get(1).is_some_and(|l| l.starts_with("0,990,10,0,")));
}
