//! Warnings that judge a run's results, before it (from the model) and after it (from what the
//! replications measured): a stop condition that can end a replication inside its warm-up
//! (#67), and a system with more work than capacity, which has no steady state (#68).

use serde_json::{Value, json};
use stochcraft_engine::Engine;

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

/// The codes of the warnings that go with `results`: those found before the run (in the run
/// record) and those found from its results.
fn codes(results: &Value) -> Vec<String> {
    results["record"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(results["warnings"].as_array().into_iter().flatten())
        .filter_map(|w| w["code"].as_str().map(str::to_owned))
        .collect()
}

/// A single server fed straight from a source: exponential times between arrivals of mean
/// `interarrival`, exponential service of mean `service`.
fn queue(interarrival: f64, service: f64, warmup: f64, duration: f64) -> Engine {
    let mut e = Engine::new();
    ok(
        &mut e,
        "model.set",
        json!({ "model": {
            "name": "Queue",
            "elements": [
                {"id": "arrivals", "kind": "source", "interarrival": {"type": "exponential", "mean": interarrival}},
                {"id": "desk", "kind": "server", "capacity": 1, "service": {"type": "exponential", "mean": service}},
                {"id": "exit", "kind": "sink"}
            ],
            "connections": [
                {"id": "flow-1", "from": "arrivals", "to": "desk"},
                {"id": "flow-2", "from": "desk", "to": "exit"}
            ],
            "experiment": {"time_unit": "min", "warmup": warmup, "duration": duration, "replications": 10, "seed": 7}
        } }),
    );
    e
}

// #67: a stop before the warm-up ends.

#[test]
fn a_stop_with_a_warm_up_is_warned_about_before_the_run() {
    let mut e = queue(1.0, 0.8, 100.0, 3000.0);
    ok(
        &mut e,
        "experiment.set",
        json!({ "stop": { "when": "count", "element": "exit", "count": 5 } }),
    );
    let v = ok(&mut e, "model.validate", json!({}));
    let found: Vec<&str> = v["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["code"].as_str())
        .collect();
    assert_eq!(found, vec!["stop_with_warmup"], "{v}");
    // No warm-up, nothing to warn about.
    ok(&mut e, "experiment.set", json!({ "warmup": 0.0 }));
    let v = ok(&mut e, "model.validate", json!({}));
    assert_eq!(v["diagnostics"], json!([]), "{v}");
}

#[test]
fn a_replication_that_stops_inside_its_warm_up_does_not_claim_the_window() {
    // Warm-up 100, stop once 5 entities have left: every replication stops near t = 6.
    let mut e = queue(1.0, 0.8, 100.0, 3000.0);
    ok(
        &mut e,
        "experiment.set",
        json!({ "stop": { "when": "count", "element": "exit", "count": 5 } }),
    );
    let r = ok(&mut e, "run", json!({}));
    let found = codes(&r);
    assert!(found.contains(&"stop_with_warmup".to_owned()), "{found:?}");
    assert!(found.contains(&"ended_in_warmup".to_owned()), "{found:?}");
    // Every replication met the stop, inside the warm-up.
    assert_eq!(r["stop"]["met"], 10, "{}", r["stop"]);
    let times: Vec<f64> = r["stop"]["times"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    assert_eq!(times.len(), 10);
    assert!(times.iter().all(|t| *t < 100.0), "{times:?}");
    // What each replication measured: from 0 (no reset happened) to its stop, and none of it
    // after a warm-up, so none of it is used.
    let measured = &r["measured"];
    let spans = measured["spans"].as_array().cloned().unwrap_or_default();
    assert_eq!(spans.len(), 10, "{measured}");
    for (span, t) in spans.iter().zip(&times) {
        assert_eq!(span["from"], 0.0, "{span}");
        assert_eq!(span["to"].as_f64(), Some(*t), "{span}");
        assert_eq!(span["used"], false, "{span}");
    }
    assert!(measured["mean_duration"].is_null(), "{measured}");
    // No statistics are reported as if they came from after the warm-up.
    for el in r["elements"].as_array().into_iter().flatten() {
        assert_eq!(el["metrics"], json!({}), "{el}");
    }
}

#[test]
fn a_replication_that_stops_after_its_warm_up_measures_from_the_warm_up_to_the_stop() {
    let mut e = queue(1.0, 0.8, 100.0, 3000.0);
    ok(
        &mut e,
        "experiment.set",
        json!({ "stop": { "when": "count", "element": "exit", "count": 300 } }),
    );
    let r = ok(&mut e, "run", json!({}));
    let found = codes(&r);
    assert_eq!(found, vec!["stop_with_warmup".to_owned()], "{found:?}");
    let spans = r["measured"]["spans"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(spans.len(), 10);
    for (span, t) in spans
        .iter()
        .zip(r["stop"]["times"].as_array().into_iter().flatten())
    {
        assert_eq!(span["from"], 100.0, "{span}");
        assert_eq!(&span["to"], t, "{span}");
        assert_eq!(span["used"], true, "{span}");
    }
    let mean = r["measured"]["mean_duration"].as_f64().unwrap_or(0.0);
    assert!(mean > 100.0 && mean < 300.0, "{mean}");
    assert_eq!(r["elements"][1]["metrics"]["entered"]["n"], 10);
}

#[test]
fn a_run_without_a_stop_measures_its_whole_duration() {
    let mut e = queue(1.0, 0.8, 100.0, 300.0);
    let r = ok(&mut e, "run", json!({}));
    for span in r["measured"]["spans"].as_array().into_iter().flatten() {
        assert_eq!(span, &json!({ "from": 100.0, "to": 400.0, "used": true }));
    }
    assert_eq!(r["measured"]["mean_duration"], 300.0);
}
