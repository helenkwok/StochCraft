//! Validation against observed data (#226): a run's results checked against real measurements,
//! an observed value (agrees or differs by the interval of the difference) or a bound (passes,
//! fails, or cannot yet be told, by the whole simulated interval). The rules themselves are
//! tested in `stochcraft-stats`; here are the commands, the results and the model problem.

#![allow(clippy::expect_used, clippy::panic)] // A failed step fails the test.

use std::path::Path;

use serde_json::{Value, json};
use stochcraft_engine::{Engine, ErrorCode};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    e.execute(command, params.clone())
        .unwrap_or_else(|err| panic!("{command} {params}: {err}"))
}

fn fails(e: &mut Engine, command: &str, params: Value) -> ErrorCode {
    match e.execute(command, params.clone()) {
        Ok(v) => panic!("{command} {params} should fail, gave {v}"),
        Err(err) => err.code,
    }
}

/// The M/M/1 example: a clerk (a shared pool) serving customers who arrive every 80 seconds on
/// average, 16 replications.
fn mm1() -> Engine {
    let path = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/models/mm1.bpmn"
    ));
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    e
}

/// A result of a run: `{mean, half_width, n}` of `metric` of element `id`.
fn simulated(results: &Value, id: &str, metric: &str) -> Value {
    results["elements"]
        .as_array()
        .and_then(|a| a.iter().find(|e| e["id"] == id))
        .map(|e| e["metrics"][metric].clone())
        .unwrap_or_else(|| panic!("no result {id}.{metric}"))
}

/// The check of `id`'s `metric` in a run's `observed`.
fn verdict(results: &Value, id: &str, metric: &str) -> String {
    results["observed"]
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|c| c["element"] == id && c["metric"] == metric)
        })
        .and_then(|c| c["verdict"].as_str())
        .unwrap_or_else(|| panic!("no check of {id}.{metric}: {}", results["observed"]))
        .to_owned()
}

fn num(v: &Value) -> f64 {
    v.as_f64().expect("a number")
}

/// Results without observed data have no `observed`, so every other result is as before.
#[test]
fn a_run_without_observed_data_says_nothing_of_it() {
    let mut e = mm1();
    let r = ok(&mut e, "run", json!({}));
    assert!(r.get("observed").is_none(), "{}", r["observed"]);
    assert!(
        ok(&mut e, "model.get", json!({}))["experiment"]
            .get("observed")
            .is_none()
    );
}

/// An observed value equal to the simulated mean agrees; one 20 points of utilisation away
/// differs, and is listed first; bounds are decided by the whole interval.
#[test]
fn a_run_is_checked_against_observed_values_and_bounds() {
    let mut e = mm1();
    let first = ok(&mut e, "run", json!({}));
    let wait = simulated(&first, "serve", "mean_wait");
    let util = simulated(&first, "clerk", "utilisation");
    let (wait_mean, wait_hw) = (num(&wait["mean"]), num(&wait["half_width"]));
    let util_mean = num(&util["mean"]);
    let time = simulated(&first, "leave", "mean_time_in_system");
    let time_mean = num(&time["mean"]);
    assert!(wait_hw > 0.0 && wait["n"] == 16, "{wait}");
    assert!(num(&time["half_width"]) > 0.0, "{time}");

    // The mean wait agrees with its own mean, given with a half-width of its own.
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "value", "element": "serve", "metric": "mean_wait",
                "value": wait_mean, "half_width": 0.1 }),
    );
    // A utilisation 20 points off differs (the interval of 16 runs is a fraction of a point).
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "value", "element": "clerk", "metric": "utilisation",
                "value": util_mean + 0.2 }),
    );
    // Bounds: well above the interval passes, well below fails, on the mean straddles.
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "at_most", "element": "serve", "metric": "mean_queue_length",
                "bound": 1_000.0 }),
    );
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "at_least", "element": "serve", "metric": "left", "bound": 1.0e9 }),
    );
    // One for each result: the mean time in the system at the exit, on its mean.
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "at_most", "element": "leave", "metric": "mean_time_in_system",
                "bound": time_mean }),
    );

    let r = ok(&mut e, "run", json!({}));
    assert_eq!(verdict(&r, "serve", "mean_wait"), "agrees");
    assert_eq!(verdict(&r, "clerk", "utilisation"), "differs");
    assert_eq!(verdict(&r, "serve", "mean_queue_length"), "passes");
    assert_eq!(verdict(&r, "serve", "left"), "fails");
    // The mean on the bound: the interval straddles it, so it is not decided by the mean.
    assert_eq!(verdict(&r, "leave", "mean_time_in_system"), "cant_tell");
    let checks = r["observed"].as_array().expect("the checks");
    assert_eq!(checks.len(), 5);
    let straddling = checks
        .iter()
        .find(|c| c["check"] == "at_most" && c["metric"] == "mean_time_in_system")
        .expect("the bound on the mean time in the system");
    assert_eq!(straddling["verdict"], "cant_tell");

    // Those that differ or fail come first, the rest in the order kept.
    let order: Vec<(&str, &str)> = checks
        .iter()
        .map(|c| {
            (
                c["metric"].as_str().unwrap_or(""),
                c["verdict"].as_str().unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        order,
        [
            ("utilisation", "differs"),
            ("left", "fails"),
            ("mean_wait", "agrees"),
            ("mean_queue_length", "passes"),
            ("mean_time_in_system", "cant_tell"),
        ]
    );

    // The numbers: the observed value beside the simulated mean and the interval of the
    // difference, which combines both half-widths as the root of the sum of squares.
    let agrees = checks
        .iter()
        .find(|c| c["metric"] == "mean_wait" && c["check"] == "value")
        .expect("the observed mean wait");
    assert_eq!(agrees["name"], "Serve customer");
    assert_eq!(
        agrees["observed"],
        json!({ "value": wait_mean, "half_width": 0.1 })
    );
    assert_eq!(agrees["simulated"]["n"], 16);
    let (hw, d) = (
        num(&agrees["difference_half_width"]),
        num(&agrees["difference"]),
    );
    let sim_hw = num(&agrees["simulated"]["half_width"]);
    assert!(
        (hw - (sim_hw * sim_hw + 0.01).sqrt()).abs() < 1e-9,
        "{agrees}"
    );
    assert!(d.abs() < 1e-9, "{agrees}");
    // A bound has no difference.
    assert!(straddling["difference"].is_null());
    assert_eq!(straddling["bound"], json!(time_mean));

    // The run is the same run: the observed data change none of its numbers.
    let mut plain = r.clone();
    let mut before = first.clone();
    for v in [&mut plain, &mut before] {
        if let Some(o) = v.as_object_mut() {
            o.remove("observed");
            o.remove("record");
        }
    }
    assert_eq!(plain, before);
}

/// With fewer than 2 replications there is no interval: nothing is decided, whatever the mean.
#[test]
fn one_replication_decides_nothing() {
    let mut e = mm1();
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "at_most", "element": "serve", "metric": "mean_wait", "bound": 1_000.0 }),
    );
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "value", "element": "serve", "metric": "left", "value": 5.0 }),
    );
    let r = ok(&mut e, "run", json!({ "replications": 1 }));
    assert_eq!(verdict(&r, "serve", "mean_wait"), "cant_tell");
    assert_eq!(verdict(&r, "serve", "left"), "cant_tell");
    // Said without an interval, and the difference still given.
    let value = r["observed"]
        .as_array()
        .and_then(|a| a.iter().find(|c| c["check"] == "value"))
        .expect("the observed value");
    assert!(value["simulated"]["half_width"].is_null(), "{value}");
    assert!(value["difference"].is_number());
    assert!(value["difference_half_width"].is_null());
}

/// A result the run does not report (a metric the block never has, a block that is gone) is
/// `no_result`, and a block that is gone is a model problem, not a crash.
#[test]
fn a_result_that_does_not_exist_is_reported_not_a_crash() {
    let mut e = mm1();
    // A source has no utilisation.
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "at_most", "element": "arrive", "metric": "utilisation", "bound": 0.9 }),
    );
    let r = ok(&mut e, "run", json!({}));
    assert_eq!(verdict(&r, "arrive", "utilisation"), "no_result");
    assert!(r["observed"][0]["simulated"].is_null());

    // Remove the block the observed data is for, with its flows (undone below): the data stay,
    // and the model says there is nothing to check them against.
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "value", "element": "serve", "metric": "mean_wait", "value": 3.0 }),
    );
    ok(&mut e, "block.remove", json!({ "id": "serve" }));
    assert_eq!(e.model().experiment.observed.len(), 2);
    let problem = e
        .model()
        .validate()
        .into_iter()
        .find(|d| d.problem.code() == "bad_observation")
        .expect("a problem");
    assert_eq!(
        problem.at.as_ref().map(ToString::to_string).as_deref(),
        Some("serve")
    );
    assert_eq!(problem.severity, stochcraft_model::Severity::Warning);
    // It is among the diagnostics a client sees, in words, and does not itself stop a run.
    let found = ok(&mut e, "model.validate", json!({}));
    let text = found.to_string();
    assert!(text.contains("bad_observation"), "{text}");
    assert!(text.contains("Observed data is given for a block that does not exist"));
    // The observed data can be removed though the block is gone.
    ok(
        &mut e,
        "observation.remove",
        json!({ "element": "serve", "metric": "mean_wait" }),
    );
    assert_eq!(e.model().experiment.observed.len(), 1);
}

#[test]
fn the_commands_add_change_and_remove_and_undo() {
    let mut e = mm1();
    let add = json!({ "check": "value", "element": "serve", "metric": "mean_wait",
                      "value": 3.2, "half_width": 0.4 });
    ok(&mut e, "observation.add", add.clone());
    assert_eq!(
        ok(&mut e, "model.get", json!({}))["experiment"]["observed"],
        json!([add])
    );
    // One for each result.
    assert_eq!(
        fails(&mut e, "observation.add", add.clone()),
        ErrorCode::Conflict
    );
    let bound = json!({ "check": "at_least", "element": "serve", "metric": "mean_wait",
                        "bound": 2.0 });
    assert_eq!(
        fails(
            &mut e,
            "observation.set",
            json!({ "check": "at_most", "element": "serve", "metric": "utilisation", "bound": 1.0 })
        ),
        ErrorCode::NotFound
    );
    ok(&mut e, "observation.set", bound.clone());
    assert_eq!(
        ok(&mut e, "model.get", json!({}))["experiment"]["observed"],
        json!([bound])
    );
    // A pool can be observed too.
    ok(
        &mut e,
        "observation.add",
        json!({ "check": "value", "element": "clerk", "metric": "utilisation", "value": 0.75 }),
    );
    assert_eq!(e.model().experiment.observed.len(), 2);

    // What is refused.
    for (params, code) in [
        (
            json!({ "check": "value", "element": "ghost", "metric": "mean_wait", "value": 1.0 }),
            ErrorCode::NotFound,
        ),
        (
            json!({ "check": "value", "element": "serve", "metric": "wait", "value": 1.0 }),
            ErrorCode::InvalidParams,
        ),
        (
            json!({ "check": "value", "element": "serve", "metric": "left", "value": 1.0,
                    "half_width": -1.0 }),
            ErrorCode::InvalidParams,
        ),
        (
            // A bound has no half-width, and a value has no bound.
            json!({ "check": "at_most", "element": "serve", "metric": "left", "bound": 1.0,
                    "half_width": 1.0 }),
            ErrorCode::InvalidParams,
        ),
        (
            json!({ "check": "value", "element": "serve", "metric": "left", "bound": 1.0 }),
            ErrorCode::InvalidParams,
        ),
        (
            json!({ "element": "serve", "metric": "left", "value": 1.0 }),
            ErrorCode::InvalidParams,
        ),
    ] {
        assert_eq!(
            fails(&mut e, "observation.add", params.clone()),
            code,
            "{params}"
        );
    }
    assert_eq!(
        fails(
            &mut e,
            "observation.remove",
            json!({ "element": "serve", "metric": "left" })
        ),
        ErrorCode::NotFound
    );
    assert_eq!(
        e.model().experiment.observed.len(),
        2,
        "nothing refused changed it"
    );

    // Each is one undoable, journalled edit.
    ok(
        &mut e,
        "observation.remove",
        json!({ "element": "clerk", "metric": "utilisation" }),
    );
    assert_eq!(e.model().experiment.observed.len(), 1);
    let before = e.journal().len();
    for expected in [2, 1, 1] {
        ok(&mut e, "undo", json!({}));
        assert_eq!(e.model().experiment.observed.len(), expected);
    }
    // After undoing the set, the first value is back.
    assert_eq!(
        ok(&mut e, "model.get", json!({}))["experiment"]["observed"],
        json!([add])
    );
    ok(&mut e, "redo", json!({}));
    assert_eq!(
        ok(&mut e, "model.get", json!({}))["experiment"]["observed"],
        json!([bound])
    );
    assert!(e.journal().len() > before);
    let names: Vec<&str> = e.journal().iter().map(|j| j.command.as_str()).collect();
    assert!(
        names.contains(&"observation.add")
            && names.contains(&"observation.set")
            && names.contains(&"observation.remove"),
        "{names:?}"
    );
}

/// The model's document carries the observed data, and `model.set` reads them back.
#[test]
fn observed_data_round_trip_through_the_model_document() {
    let mut e = mm1();
    for p in [
        json!({ "check": "value", "element": "serve", "metric": "mean_wait", "value": 3.2,
                "half_width": 0.4 }),
        json!({ "check": "at_most", "element": "clerk", "metric": "utilisation", "bound": 0.85 }),
        json!({ "check": "at_least", "element": "serve", "metric": "left", "bound": 400.0 }),
    ] {
        ok(&mut e, "observation.add", p);
    }
    let doc = ok(&mut e, "model.get", json!({}));
    assert_eq!(
        doc["experiment"]["observed"].as_array().map(Vec::len),
        Some(3)
    );
    let mut other = Engine::new();
    ok(&mut other, "model.set", json!({ "model": doc.clone() }));
    assert_eq!(
        other.model().experiment.observed,
        e.model().experiment.observed
    );
    assert_eq!(
        ok(&mut other, "model.get", json!({}))["experiment"]["observed"],
        doc["experiment"]["observed"]
    );
}
