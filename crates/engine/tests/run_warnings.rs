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
// #68: an overloaded system, and work left inside.

#[test]
fn work_in_progress_at_the_end_is_reported() {
    let mut e = queue(1.0, 0.8, 100.0, 1000.0);
    let r = ok(&mut e, "run", json!({}));
    let desk = &r["elements"][1];
    let wip = &desk["metrics"]["work_in_progress"];
    assert_eq!(wip["n"], 10, "{desk}");
    assert!(wip["mean"].as_f64().is_some_and(|m| m >= 0.0), "{wip}");
    // One value per replication, for its distribution.
    assert_eq!(
        r["values"]["desk"]["work_in_progress"]
            .as_array()
            .map(Vec::len),
        Some(10)
    );
    // Sources and sinks hold nothing.
    assert!(r["elements"][0]["metrics"]["work_in_progress"].is_null());
    assert!(r["elements"][2]["metrics"]["work_in_progress"].is_null());
}

#[test]
fn an_overloaded_queue_is_warned_about_before_and_after_the_run() {
    // M/M/1 with mean service 1.5 and mean time between arrivals 1: 50% more work than capacity.
    let mut e = queue(1.0, 1.5, 100.0, 3000.0);
    let v = ok(&mut e, "model.validate", json!({}));
    let pre: Vec<&Value> = v["diagnostics"].as_array().into_iter().flatten().collect();
    assert_eq!(pre.len(), 1, "{v}");
    assert_eq!(pre[0]["code"], "overloaded");
    assert_eq!(pre[0]["at"], "desk");
    assert_eq!(pre[0]["severity"], "warning");
    let r = ok(&mut e, "run", json!({}));
    let found = codes(&r);
    assert!(found.contains(&"overloaded".to_owned()), "{found:?}");
    assert!(found.contains(&"no_steady_state".to_owned()), "{found:?}");
    let after = r["warnings"]
        .as_array()
        .and_then(|w| w.iter().find(|w| w["code"] == "no_steady_state"))
        .cloned()
        .unwrap_or_default();
    assert_eq!(after["at"], "desk", "{after}");
    // Work piles up: far more still inside at the end than a stable queue would hold.
    let wip = r["elements"][1]["metrics"]["work_in_progress"]["mean"]
        .as_f64()
        .unwrap_or(0.0);
    assert!(wip > 500.0, "{wip}");
}

#[test]
fn an_overload_the_model_cannot_show_is_found_from_the_results() {
    // The issue's case: the help desk mm1.bpmn with service mean 1.5 (12.5% more work than
    // capacity), reached through a delay, so the offered load is not worked out before the run.
    let mut e = Engine::new();
    ok(
        &mut e,
        "model.set",
        json!({ "model": {
            "name": "Hidden overload",
            "elements": [
                {"id": "arrivals", "kind": "source", "interarrival": {"type": "exponential", "mean": 1.3333333}},
                {"id": "walk", "kind": "delay", "duration": {"type": "constant", "value": 1.0}},
                {"id": "desk", "kind": "server", "capacity": 1, "service": {"type": "exponential", "mean": 1.5}},
                {"id": "exit", "kind": "sink"}
            ],
            "connections": [
                {"id": "flow-1", "from": "arrivals", "to": "walk"},
                {"id": "flow-2", "from": "walk", "to": "desk"},
                {"id": "flow-3", "from": "desk", "to": "exit"}
            ],
            "experiment": {"time_unit": "min", "warmup": 1000.0, "duration": 3000.0, "replications": 10, "seed": 3}
        } }),
    );
    let v = ok(&mut e, "model.validate", json!({}));
    assert_eq!(v["diagnostics"], json!([]), "{v}");
    let r = ok(&mut e, "run", json!({}));
    assert_eq!(codes(&r), vec!["no_steady_state".to_owned()]);
}

#[test]
fn a_busy_but_stable_queue_is_not_warned_about() {
    for (interarrival, service) in [(1.0, 0.75), (1.0, 0.9)] {
        let mut e = queue(interarrival, service, 1000.0, 20000.0);
        let r = ok(&mut e, "run", json!({}));
        assert_eq!(codes(&r), Vec::<String>::new(), "service {service}");
    }
}

#[test]
fn the_help_desk_example_is_not_warned_about() {
    let mut e = Engine::new();
    ok(
        &mut e,
        "model.set",
        stochcraft_engine::COMMANDS[0].example(),
    );
    let r = ok(&mut e, "run", json!({}));
    assert_eq!(codes(&r), Vec::<String>::new());
}

/// Every model committed under tests/models, and every BPSim reference model when they have been
/// downloaded (`cargo xtask fetch-bpsim`), runs without a warning about its results.
#[test]
fn no_committed_or_reference_model_is_warned_about() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("tests/models"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|d| d.path())
        .collect();
    let guide = root.join("target/bpsim/guide");
    if guide.is_dir() {
        let mut stack = vec![guide];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| {
                    x.eq_ignore_ascii_case("bpmn") || x.eq_ignore_ascii_case("xpdl")
                }) {
                    files.push(p);
                }
            }
        }
    } else {
        eprintln!("BPSim reference models skipped: run `cargo xtask fetch-bpsim` to include them");
    }
    files.sort();
    let mut checked = 0;
    for path in &files {
        let name = path.display().to_string();
        let mut e = Engine::new();
        if name.ends_with(".json") {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            let script: Vec<stochcraft_engine::JournalEntry> =
                serde_json::from_str(&text).unwrap_or_default();
            if e.replay(&script).is_err() {
                continue;
            }
        } else {
            let bytes = std::fs::read(path).unwrap_or_default();
            if e.open_bytes(&name, &bytes, None).is_err() {
                // Not every reference file is a runnable model (some are fragments).
                continue;
            }
        }
        let Ok(r) = e.execute("run", json!({ "replications": 4 })) else {
            continue;
        };
        let ours: Vec<String> = codes(&r)
            .into_iter()
            .filter(|c| {
                [
                    "stop_with_warmup",
                    "ended_in_warmup",
                    "overloaded",
                    "no_steady_state",
                ]
                .contains(&c.as_str())
            })
            .collect();
        assert_eq!(ours, Vec::<String>::new(), "{name}");
        checked += 1;
    }
    assert!(checked >= 4, "only {checked} models ran");
}
