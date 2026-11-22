//! The engine's boundary: a result is not produced from a file that was read with errors unless
//! that is asked for, and sizing does not apply an answer that is not one.

use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, CommandError, Engine, ErrorCode};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

fn err(e: &mut Engine, command: &str, params: Value) -> CommandError {
    match e.execute(command, params.clone()) {
        Ok(v) => unreachable!("{command} {params} should fail, gave {v}"),
        Err(f) => f,
    }
}

fn help_desk() -> Engine {
    let mut e = Engine::new();
    ok(&mut e, "model.set", COMMANDS[0].example());
    e
}

const MM1: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/models/mm1.bpmn");

/// The checked-in M/M/1 file with the service time replaced by a processing-time expression,
/// which the reader cannot use: it reads as a time of zero and notes an error.
fn expression_file() -> Vec<u8> {
    let text = std::fs::read_to_string(MM1).unwrap_or_default();
    let altered = text.replacen(
        "<bpsim:NegativeExponentialDistribution mean=\"1\"/>",
        "<bpsim:ExpressionParameter value=\"1 + 9\"/>",
        1,
    );
    assert_ne!(text, altered, "the fixture changed: {MM1}");
    altered.into_bytes()
}

fn opened_with_error() -> Engine {
    let mut e = Engine::new();
    let opened = e.open_bytes("expression.bpmn", &expression_file(), None);
    let notes = opened.map(|o| o["notes"].clone()).unwrap_or_default();
    assert!(
        notes
            .as_array()
            .is_some_and(|n| n.iter().any(|n| n["severity"] == "error")),
        "the file should be read with an error: {notes}"
    );
    e
}

#[test]
fn a_file_read_with_errors_does_not_run() {
    let mut e = opened_with_error();
    let reps = json!({ "replications": 2 });
    for command in ["run", "run.start"] {
        let f = err(&mut e, command, reps.clone());
        assert_eq!(f.code, ErrorCode::InvalidModel, "{command}: {f:?}");
        assert!(
            f.message.contains("allow_approximations"),
            "{command}: {}",
            f.message
        );
        let details = f.details.unwrap_or_default();
        assert!(
            details.as_array().is_some_and(|d| d
                .iter()
                .any(|n| n["at"] == "serve" && n["severity"] == "error")),
            "{command}: {details}"
        );
    }
    let target = json!({ "element": "leave", "metric": "entered", "at_least": 1 });
    let experiments = [
        (
            "experiment.sweep",
            json!({
                "parameter": { "id": "serve", "field": "mean" },
                "values": [1.0, 2.0],
                "results": [{ "element": "leave", "metric": "entered" }],
                "replications": 2,
            }),
        ),
        (
            "experiment.sensitivity",
            json!({
                "parameters": [{ "id": "arrive", "field": "mean" }],
                "results": [{ "element": "leave", "metric": "entered" }],
                "replications": 2,
            }),
        ),
        (
            "experiment.design",
            json!({
                "factors": [{ "id": "arrive", "field": "mean", "levels": [1.0, 2.0] }],
                "results": [{ "element": "leave", "metric": "entered" }],
                "replications": 2,
            }),
        ),
        (
            "experiment.size",
            json!({ "target": target, "target_share": 0.5, "replications": 2 }),
        ),
    ];
    for (command, params) in experiments {
        let f = err(&mut e, command, params);
        assert_eq!(f.code, ErrorCode::InvalidModel, "{command}: {f:?}");
    }
    assert!(e.live_run(0).is_err());
}

#[test]
fn allowing_approximations_runs_and_records_each_substitution() {
    let mut e = opened_with_error();
    let r = ok(
        &mut e,
        "run",
        json!({ "replications": 2, "allow_approximations": true }),
    );
    let a = r["record"]["approximations"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // Every substitution is an error note, and the service time at `serve` is one.
    assert!(
        a.iter().all(|n| n["severity"] == "error"),
        "{}",
        r["record"]
    );
    assert!(a.iter().any(|n| n["at"] == "serve"), "{}", r["record"]);
    // The notes the record always carries are unchanged.
    assert!(r["record"]["file_notes"].is_array());
}

#[test]
fn a_file_read_cleanly_runs_with_no_approximations() {
    let bytes = std::fs::read(MM1).unwrap_or_default();
    let mut e = Engine::new();
    assert!(e.open_bytes("mm1.bpmn", &bytes, None).is_ok());
    let r = ok(&mut e, "run", json!({ "replications": 2 }));
    assert!(
        r["record"].get("approximations").is_none(),
        "{}",
        r["record"]
    );
}

#[test]
fn only_setting_the_substituted_value_clears_the_error_and_undo_brings_it_back() {
    let mut e = opened_with_error();
    let refused = |e: &mut Engine| {
        let f = err(e, "run", json!({ "replications": 2 }));
        assert_eq!(f.code, ErrorCode::InvalidModel, "{f:?}");
    };
    // Another element's edit is no answer to it.
    ok(
        &mut e,
        "block.rename",
        json!({ "id": "leave", "name": "Exit" }),
    );
    refused(&mut e);
    // Nor is renaming the element, or editing another of its properties: the stand-in is still
    // in the model.
    ok(
        &mut e,
        "block.rename",
        json!({ "id": "serve", "name": "Counter" }),
    );
    refused(&mut e);

    // Setting the processing time replaces the stand-in: nothing of it is recorded.
    ok(
        &mut e,
        "block.set",
        json!({ "id": "serve", "service": { "type": "exponential", "mean": 1.0 } }),
    );
    let r = ok(&mut e, "run", json!({ "replications": 2 }));
    assert!(
        r["record"].get("approximations").is_none(),
        "{}",
        r["record"]
    );

    // Undoing it puts the stand-in back, and the error with it.
    ok(&mut e, "undo", json!({}));
    refused(&mut e);
    ok(&mut e, "redo", json!({}));
    ok(&mut e, "run", json!({ "replications": 2 }));
}

#[test]
fn sizing_refuses_a_target_that_is_not_in_the_model_and_changes_nothing() {
    let mut e = help_desk();
    let before = e.model().clone();
    let f = err(
        &mut e,
        "experiment.size",
        json!({
            "target": { "element": "typo", "metric": "left", "at_least": 1 },
            "target_share": 0.7, "replications": 2, "max_added": 2, "apply": true,
        }),
    );
    assert_eq!(f.code, ErrorCode::NotFound, "{f:?}");
    assert_eq!(e.model(), &before);
}

#[test]
fn sizing_refuses_a_metric_the_element_does_not_report() {
    let mut e = help_desk();
    let before = e.model().clone();
    // A source has no waiting time.
    let f = err(
        &mut e,
        "experiment.size",
        json!({
            "target": { "element": "arrivals", "metric": "mean_wait", "at_most": 1.0 },
            "target_share": 0.7, "replications": 2, "max_added": 2, "apply": true,
        }),
    );
    assert_eq!(f.code, ErrorCode::InvalidParams, "{f:?}");
    assert!(f.message.contains("arrivals"), "{}", f.message);
    assert_eq!(e.model(), &before);
}

#[test]
fn sizing_does_not_apply_an_unreached_answer_unless_asked() {
    let mut e = help_desk();
    let before = e.model().clone();
    // No waiting at all is not what two more units reach: some arrivals still find the desk busy.
    let params = json!({
        "target": { "element": "desk", "metric": "mean_wait", "at_most": 0.0 },
        "target_share": 0.9, "replications": 4, "max_added": 2, "apply": true,
    });
    let r = ok(&mut e, "experiment.size", params.clone());
    assert_eq!(r["stochastic"]["reached"], false, "{r}");
    assert_eq!(r["applied"], false, "{r}");
    assert!(r["not_applied"].is_string(), "{r}");
    assert_eq!(e.model(), &before, "an unreached answer changed the model");

    let mut forced = params;
    forced["apply_unreached"] = json!(true);
    let r = ok(&mut e, "experiment.size", forced);
    assert_eq!(r["applied"], true, "{r}");
    assert_ne!(e.model(), &before);
}

#[test]
fn sizing_says_it_is_a_heuristic() {
    let mut e = help_desk();
    let r = ok(
        &mut e,
        "experiment.size",
        json!({
            "target": { "element": "desk", "metric": "mean_wait", "at_most": 1.0 },
            "target_share": 0.7, "replications": 4,
        }),
    );
    assert!(
        r["method"]
            .as_str()
            .is_some_and(|m| m.contains("heuristic")),
        "{r}"
    );
}

#[test]
fn a_prior_run_csv_says_so_on_every_row() {
    let mut e = help_desk();
    let r = ok(&mut e, "run", json!({ "replications": 3, "seed": 7 }));
    let csv = stochcraft_engine::wire::results_csv(&r);
    let labelled = stochcraft_engine::wire::label_prior_run(&csv, &r);
    let (plain, with): (Vec<&str>, Vec<&str>) = (csv.lines().collect(), labelled.lines().collect());
    assert_eq!(plain.len(), with.len());
    assert!(
        with[0].ends_with(",run_status,run_model_digest,run_seed,run_runs,run_settings"),
        "{}",
        with[0]
    );
    let digest = r["record"]["model_digest"].as_str().unwrap_or("none");
    for (row, was) in with.iter().zip(&plain).skip(1) {
        // The numbers are the ones of the run, unchanged; the label follows them.
        assert!(
            row.starts_with(&format!("{was},prior run: the model has changed since,")),
            "{row}"
        );
        assert!(row.contains(&format!(",{digest},7,3,")), "{row}");
        // A discrete-event run has no integration settings to report.
        assert!(!row.contains("method="), "{row}");
    }
}
