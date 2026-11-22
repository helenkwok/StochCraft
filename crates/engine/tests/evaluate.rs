//! `experiment.evaluate`: given values of the decisions, for a range of replications, as an
//! optimiser of the user's own drives the runs (ROADMAP M9, stage P1).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // A failed step fails the test.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, CommandKind, Engine, ErrorCode, ReplayMode};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

/// The M/M/1-like help desk of the catalogue's first example.
fn help_desk() -> Engine {
    let mut e = Engine::new();
    ok(&mut e, "model.set", COMMANDS[0].example());
    e
}

fn evaluation(value: f64, first: u64, count: u64) -> Value {
    json!({
        "decisions": [{ "id": "desk", "field": "mean", "value": value }],
        "results": [
            { "element": "desk", "metric": "mean_wait" },
            { "element": "exit", "metric": "left" },
        ],
        "first": first,
        "count": count,
        "seed": 7,
    })
}

fn values(r: &Value, i: usize) -> Vec<Value> {
    r["results"][i]["values"].as_array().cloned().unwrap()
}

#[test]
fn the_same_decisions_and_seed_give_identical_results() {
    let mut e = help_desk();
    let a = ok(&mut e, "experiment.evaluate", evaluation(1.1, 0, 6));
    let b = ok(&mut e, "experiment.evaluate", evaluation(1.1, 0, 6));
    assert_eq!(a.to_string(), b.to_string());
    assert_eq!(values(&a, 0).len(), 6);
    assert_eq!(a["first"], 0);
    assert_eq!(a["count"], 6);
    assert_eq!(a["seed"], 7);
    assert_eq!(a["decisions"][0]["value"], 1.1);
    // Another seed gives other numbers, and so does another value.
    let mut other = evaluation(1.1, 0, 6);
    other["seed"] = json!(8);
    let c = ok(&mut e, "experiment.evaluate", other);
    assert_ne!(values(&a, 0), values(&c, 0));
    let d = ok(&mut e, "experiment.evaluate", evaluation(1.3, 0, 6));
    assert_ne!(values(&a, 0), values(&d, 0));
    // The mean and interval are those of the values.
    let v: Vec<f64> = values(&a, 0).iter().map(|v| v.as_f64().unwrap()).collect();
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    let m = a["results"][0]["mean"].as_f64().unwrap();
    assert!((m - mean).abs() < 1e-12, "{m} vs {mean}");
    assert!(a["results"][0]["half_width"].as_f64().unwrap() > 0.0);
    assert_eq!(a["results"][0]["n"], 6);
}

/// Replications 0–9 and then 10–19 give exactly what 0–19 give, value for value.
#[test]
fn staged_replications_equal_the_whole() {
    let mut e = help_desk();
    let whole = ok(&mut e, "experiment.evaluate", evaluation(1.2, 0, 20));
    let early = ok(&mut e, "experiment.evaluate", evaluation(1.2, 0, 10));
    let late = ok(&mut e, "experiment.evaluate", evaluation(1.2, 10, 10));
    for i in 0..2 {
        let mut staged = values(&early, i);
        staged.extend(values(&late, i));
        assert_eq!(staged, values(&whole, i), "result {i}");
    }
    // A single replication is the same value as in the whole, with no interval.
    let one = ok(&mut e, "experiment.evaluate", evaluation(1.2, 13, 1));
    assert_eq!(values(&one, 0), vec![values(&whole, 0)[13].clone()]);
    assert_eq!(one["results"][0]["half_width"], Value::Null);
    // `run` with the same seed and replications gives the same mean as the evaluation at the
    // model's own value: the same streams.
    let own = e.model().experiment.clone();
    let desk_mean = 1.5;
    let r = ok(
        &mut e,
        "experiment.evaluate",
        json!({
            "decisions": [{ "id": "desk", "field": "mean", "value": desk_mean }],
            "results": [{ "element": "desk", "metric": "mean_wait" }],
            "count": own.replications,
        }),
    );
    let run = ok(&mut e, "run", json!({}));
    let ran = run["elements"]
        .as_array()
        .and_then(|els| els.iter().find(|x| x["id"] == "desk"))
        .and_then(|x| x["metrics"]["mean_wait"]["mean"].as_f64())
        .expect("the run reports the desk's mean wait");
    let evaluated = r["results"][0]["mean"].as_f64().unwrap();
    assert!(
        (evaluated - ran).abs() <= 1e-12 * ran.abs(),
        "{evaluated} vs {ran}"
    );
}

fn refused(e: &mut Engine, params: Value) -> (ErrorCode, String) {
    let r = e.execute("experiment.evaluate", params.clone());
    match r {
        Err(err) => (err.code, err.message),
        Ok(v) => panic!("{params} should be refused: {v}"),
    }
}

fn with(decision: Value) -> Value {
    json!({
        "decisions": [decision],
        "results": [{ "element": "desk", "metric": "mean_wait" }],
        "count": 2,
    })
}

#[test]
fn unknown_decisions_and_values_out_of_range_are_refused_with_the_decision_named() {
    let mut e = help_desk();
    let before = e.model().clone();
    let cases = [
        // No such element.
        (
            json!({ "id": "ghost", "field": "mean", "value": 1.0 }),
            "ghost.mean",
            ErrorCode::NotFound,
        ),
        // No such pool.
        (
            json!({ "id": "desk", "field": "quantity", "value": 2 }),
            "desk.quantity",
            ErrorCode::NotFound,
        ),
        // The element has no such field.
        (
            json!({ "id": "exit", "field": "mean", "value": 1.0 }),
            "exit.mean",
            ErrorCode::InvalidParams,
        ),
        // Out of the range given.
        (
            json!({ "id": "desk", "field": "mean", "value": 0.9, "min": 1.0, "max": 2.0 }),
            "desk.mean",
            ErrorCode::InvalidParams,
        ),
        (
            json!({ "id": "desk", "field": "mean", "value": 2.5, "max": 2.0 }),
            "desk.mean",
            ErrorCode::InvalidParams,
        ),
        // Out of the field's own range.
        (
            json!({ "id": "desk", "field": "mean", "value": 0.0 }),
            "desk.mean",
            ErrorCode::InvalidParams,
        ),
        (
            json!({ "id": "desk", "field": "capacity", "value": 0 }),
            "desk.capacity",
            ErrorCode::InvalidParams,
        ),
        // The wrong kind of value.
        (
            json!({ "id": "desk", "field": "mean", "value": "fast" }),
            "desk.mean",
            ErrorCode::InvalidParams,
        ),
        (
            json!({ "id": "desk", "field": "capacity", "value": 1.5 }),
            "desk.capacity",
            ErrorCode::InvalidParams,
        ),
        // An empty range.
        (
            json!({ "id": "desk", "field": "mean", "value": 1.0, "min": 2.0, "max": 1.0 }),
            "desk.mean",
            ErrorCode::InvalidParams,
        ),
    ];
    for (decision, name, code) in cases {
        let (c, message) = refused(&mut e, with(decision.clone()));
        assert_eq!(c, code, "{decision}: {message}");
        assert!(message.contains(name), "{decision}: {message}");
    }
    // A decision given twice.
    let d = json!({ "id": "desk", "field": "mean", "value": 1.0 });
    let mut twice = with(d.clone());
    twice["decisions"] = json!([d, d]);
    let (c, message) = refused(&mut e, twice);
    assert_eq!(c, ErrorCode::InvalidParams);
    assert!(message.contains("desk.mean"), "{message}");
    // A field that is not one: refused by the schema.
    let (c, _) = refused(
        &mut e,
        with(json!({ "id": "desk", "field": "speed", "value": 1.0 })),
    );
    assert_eq!(c, ErrorCode::InvalidParams);
    // A result that names nothing, and no replications.
    let mut bad = with(json!({ "id": "desk", "field": "mean", "value": 1.0 }));
    bad["results"] = json!([{ "element": "nowhere", "metric": "mean_wait" }]);
    assert_eq!(refused(&mut e, bad).0, ErrorCode::NotFound);
    let mut none = with(json!({ "id": "desk", "field": "mean", "value": 1.0 }));
    none["count"] = json!(0);
    assert_eq!(refused(&mut e, none).0, ErrorCode::InvalidParams);
    // Nothing was changed by any of it.
    assert_eq!(e.model(), &before);
}

/// An evaluation changes nothing in the model and is no edit to undo; it is journalled as the
/// other experiments are, and a replay of edits only skips it.
#[test]
fn an_evaluation_leaves_the_model_and_its_history_alone() {
    let mut e = help_desk();
    let before = e.model().clone();
    let undo_before = ok(&mut e, "history", json!({}));
    ok(&mut e, "experiment.evaluate", evaluation(0.8, 0, 3));
    assert_eq!(e.model(), &before);
    assert_eq!(ok(&mut e, "history", json!({})), undo_before);
    assert_eq!(
        e.journal().last().map(|j| j.command.as_str()),
        Some("experiment.evaluate")
    );
    let c = COMMANDS
        .iter()
        .find(|c| c.name == "experiment.evaluate")
        .unwrap();
    assert_eq!(c.kind, CommandKind::Action);
    assert!(!ReplayMode::EditsOnly.applies("experiment.evaluate"));
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/optimisers")
}

/// The user guide's M/M/1 (arrivals at rate 1): at mean service s, the server's mean number
/// inside is s / (1 − s), so 1 at s = 0.5 and 0.25 at s = 0.2; the objective s / (1 − s) − 4s is
/// least, −1, at s = 0.5.
#[test]
fn the_guide_s_mm1_gives_its_mean_number_inside() {
    let mut e = Engine::new();
    let path = examples().join("mm1-cost.bpmn");
    ok(
        &mut e,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    for (s, theory) in [(0.5, 1.0), (0.2, 0.25)] {
        let r = ok(
            &mut e,
            "experiment.evaluate",
            json!({
                "decisions": [{ "id": "server", "field": "mean", "value": s, "min": 0.2, "max": 0.9 }],
                "results": [{ "element": "server", "metric": "mean_work_in_progress" }],
            }),
        );
        let m = r["results"][0]["mean"].as_f64().unwrap();
        let hw = r["results"][0]["half_width"].as_f64().unwrap();
        assert_eq!(r["count"], 10);
        assert!(
            (m - theory).abs() <= 2.0 * hw.max(0.01),
            "s = {s}: {m} ± {hw} vs {theory}"
        );
    }
}
