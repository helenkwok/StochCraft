//! Results over time in the engine's `run` results (`series`), the warm-up MSER-5 suggests
//! (`warmup_suggestion`), and the warnings that come with them (`one_replication`,
//! `no_warmup`), the same however the run is carried out.

use serde_json::{Value, json};
use stochcraft_engine::Engine;

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

fn codes(results: &Value) -> Vec<String> {
    results["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| w["code"].as_str().map(str::to_owned))
        .collect()
}

/// M/M/1 with service rate 1 and arrival rate `rho`, in minutes; `frequency`, the scenario's
/// result frequency in seconds, if any.
fn mm1(rho: f64, warmup: f64, duration: f64, frequency: Option<f64>) -> Engine {
    let mut e = Engine::new();
    let mut experiment = json!({
        "time_unit": "min", "warmup": warmup, "duration": duration, "replications": 30, "seed": 1
    });
    if let Some(f) = frequency {
        experiment["result_frequency"] = json!(f);
    }
    ok(
        &mut e,
        "model.set",
        json!({ "model": {
            "name": "M/M/1",
            "elements": [
                {"id": "arrivals", "kind": "source", "interarrival": {"type": "exponential", "mean": 1.0 / rho}},
                {"id": "desk", "kind": "server", "capacity": 1, "service": {"type": "exponential", "mean": 1.0}},
                {"id": "exit", "kind": "sink"}
            ],
            "connections": [
                {"id": "flow-1", "from": "arrivals", "to": "desk"},
                {"id": "flow-2", "from": "desk", "to": "exit"}
            ],
            "experiment": experiment
        } }),
    );
    e
}

#[test]
fn a_run_carries_its_results_over_time() {
    let mut e = mm1(0.9, 0.0, 2_000.0, None);
    let r = ok(&mut e, "run", json!({}));
    let s = &r["series"];
    // A hundredth of the run, from time 0 to its end.
    assert_eq!(s["interval"], 20.0);
    let times = s["times"].as_array().map_or(0, Vec::len);
    assert_eq!(times, 101);
    assert_eq!(s["times"][100], 2_000.0);
    assert!(s["n"].as_array().into_iter().flatten().all(|n| n == 30));
    let desk = &s["elements"]["desk"];
    for q in [
        "queue_length",
        "work_in_progress",
        "busy",
        "entered",
        "left",
    ] {
        for k in ["mean", "half_width", "low", "high"] {
            assert_eq!(desk[q][k].as_array().map_or(0, Vec::len), 101, "{q} {k}");
        }
    }
    assert!(s["elements"]["exit"]["entered"].is_object());
    assert!(s["elements"]["exit"].get("busy").is_none());
    // Started empty: nothing inside at time 0, and the band is a band.
    assert_eq!(desk["work_in_progress"]["mean"][0], 0.0);
    let m = desk["work_in_progress"]["mean"][100]
        .as_f64()
        .unwrap_or(0.0);
    let lo = desk["work_in_progress"]["low"][100].as_f64().unwrap_or(1e9);
    let hi = desk["work_in_progress"]["high"][100]
        .as_f64()
        .unwrap_or(-1.0);
    assert!(lo <= m && m <= hi, "{lo} {m} {hi}");

    // The busy queue started empty: a warm-up is suggested and warned about.
    let w = &r["warmup_suggestion"];
    assert_eq!(w["method"], "mser-5");
    assert_eq!(w["quantity"], "work_in_progress");
    assert_eq!(w["at"], "desk");
    let t = w["warmup"].as_f64().unwrap_or(0.0);
    assert!(t > 0.0 && t <= 1_000.0, "{w}");
    assert_eq!(w["elements"]["desk"]["matters"], true, "{w}");
    assert_eq!(w["matters"], true, "{w}");
    assert_eq!(codes(&r), vec!["no_warmup"]);
    let message = r["warnings"][0]["message"].as_str().unwrap_or("");
    assert!(
        message.contains(&format!("Set a warm-up of about {t} min")),
        "{message}"
    );
    assert_eq!(r["warnings"][0]["at"], "desk");

    // With the warm-up it suggests, the warning goes; the suggestion stays.
    ok(&mut e, "experiment.set", json!({ "warmup": t }));
    let r = ok(&mut e, "run", json!({}));
    assert!(codes(&r).is_empty(), "{}", r["warnings"]);
    assert!(r["warmup_suggestion"].is_object());
}

#[test]
fn the_scenarios_result_frequency_sets_the_interval() {
    // Every 5 minutes, given in seconds.
    let mut e = mm1(0.5, 100.0, 900.0, Some(300.0));
    let r = ok(&mut e, "run", json!({ "replications": 4 }));
    assert_eq!(r["series"]["interval"], 5.0);
    assert_eq!(r["series"]["times"].as_array().map_or(0, Vec::len), 201);
    // A file read back gives it again.
    assert_eq!(e.model().experiment.result_frequency, Some(300.0));
}

#[test]
fn one_replication_is_warned_about() {
    let mut e = mm1(0.5, 100.0, 1_000.0, None);
    let r = ok(&mut e, "run", json!({ "replications": 1 }));
    assert_eq!(codes(&r), vec!["one_replication"]);
    // One replication: no interval, so the half-widths are null.
    assert!(r["series"]["elements"]["desk"]["busy"]["half_width"][50].is_null());
    let r = ok(&mut e, "run", json!({ "replications": 2 }));
    assert!(codes(&r).is_empty(), "{}", r["warnings"]);
}

#[test]
fn a_light_queue_is_not_warned_about() {
    let mut e = mm1(0.3, 0.0, 5_000.0, None);
    let r = ok(&mut e, "run", json!({ "replications": 10 }));
    assert!(codes(&r).is_empty(), "{}", r["warnings"]);
    assert!(r["warmup_suggestion"]["elements"]["desk"]["matters"] == false);
    assert!(r["warmup_suggestion"]["matters"] == false);
}

#[test]
fn a_stop_condition_gives_no_suggestion() {
    let mut e = mm1(0.5, 0.0, 1_000.0, None);
    ok(
        &mut e,
        "experiment.set",
        json!({ "stop": { "when": "count", "element": "exit", "count": 100 } }),
    );
    let r = ok(&mut e, "run", json!({ "replications": 4 }));
    assert!(r["warmup_suggestion"].is_null());
    assert!(r["series"]["times"].as_array().map_or(0, Vec::len) < 101);
    assert!(codes(&r).is_empty(), "{}", r["warnings"]);
}

#[test]
fn the_series_is_the_same_on_threads_in_slices_and_on_workers() {
    let mut e = mm1(0.8, 50.0, 500.0, None);
    let threads = ok(&mut e, "run", json!({ "replications": 6 }));
    assert!(threads["series"].is_object());
    // In slices of 777 events.
    ok(
        &mut e,
        "run.start",
        json!({ "replications": 6, "mode": "slices" }),
    );
    let sliced = loop {
        let s = ok(&mut e, "run.status", json!({ "step": 1, "events": 777 }));
        if s["state"] != "running" {
            break s;
        }
    };
    assert_eq!(sliced["result"]["series"], threads["series"]);
    assert_eq!(
        sliced["result"]["warmup_suggestion"],
        threads["warmup_suggestion"]
    );
    // On workers, which get the run as text and answer in text.
    ok(
        &mut e,
        "run.start",
        json!({ "replications": 6, "mode": "workers" }),
    );
    let task = e.worker_task().unwrap_or_default().to_owned();
    let worker = stochcraft_engine::Worker::new(&task);
    assert!(worker.is_ok(), "{:?}", worker.err());
    let Ok(worker) = worker else { return };
    for i in (0..6).rev() {
        let answer = worker.run(i);
        let d = e.deliver(&answer);
        assert!(d.is_ok(), "{d:?}");
    }
    let done = ok(&mut e, "run.status", json!({}));
    assert_eq!(done["result"]["series"], threads["series"]);
    assert_eq!(done["result"]["warnings"], threads["warnings"]);
}

#[test]
fn series_false_records_nothing_and_changes_no_result() {
    let mut e = mm1(0.9, 0.0, 2_000.0, None);
    let with = ok(&mut e, "run", json!({}));
    let without = ok(&mut e, "run", json!({ "series": false }));
    assert!(without["series"].is_null());
    assert!(without["warmup_suggestion"].is_null());
    // Without the series nothing can suggest a warm-up, so nothing warns of its absence.
    assert!(codes(&without).is_empty(), "{}", without["warnings"]);
    for key in ["elements", "values", "measured"] {
        assert_eq!(with[key], without[key], "{key}");
    }
    // The size the series adds: three elements, 101 times.
    let size = |v: &Value| serde_json::to_string(v).map_or(0, |s| s.len());
    eprintln!(
        "results: {} bytes without the series, {} with",
        size(&without),
        size(&with)
    );
}

/// mm1.bpmn with a result frequency of 30 minutes in its scenario.
fn with_frequency() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/mm1.bpmn");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let old = r#"baseTimeUnit="min">"#;
    assert!(text.contains(old));
    text.replacen(old, r#"baseTimeUnit="min" baseResultFrequency="PT30M">"#, 1)
        .into_bytes()
}

fn opened(bytes: &[u8]) -> Engine {
    let mut e = Engine::new();
    let r = e.open_bytes("mm1.bpmn", bytes, None);
    assert!(r.is_ok(), "{r:?}");
    e
}

fn saved(e: &Engine) -> Vec<u8> {
    let r = e.save_bytes(false);
    assert!(r.is_ok(), "{r:?}");
    r.unwrap_or_default()
}

#[test]
fn saves_in_place_keep_the_result_frequency() {
    let bytes = with_frequency();
    let mut e = opened(&bytes);
    assert_eq!(e.model().experiment.result_frequency, Some(1_800.0));
    // Saved unchanged: byte for byte.
    assert_eq!(saved(&e), bytes);
    // An unrelated edit, then a change of time unit: still there, still 30 minutes.
    ok(&mut e, "experiment.set", json!({ "warmup": 500.0 }));
    let again = opened(&saved(&e));
    assert_eq!(again.model().experiment.result_frequency, Some(1_800.0));
    assert_eq!(again.model().experiment.warmup, 500.0);
    ok(&mut e, "experiment.set", json!({ "time_unit": "hour" }));
    let again = opened(&saved(&e));
    assert_eq!(again.model().experiment.result_frequency, Some(1_800.0));
    // A new frequency is not silently dropped: a save in place, which does not write it yet,
    // refuses with the reason.
    let mut doc = ok(&mut e, "model.get", json!({}));
    doc["experiment"]["result_frequency"] = json!(600.0);
    ok(&mut e, "model.set", json!({ "model": doc }));
    let refused = e.save_bytes(false);
    assert!(
        refused
            .as_ref()
            .is_err_and(|err| err.message.contains("would not read back")),
        "{refused:?}"
    );
    // As a new file it is written.
    let new = e.save_bytes(true).unwrap_or_default();
    assert_eq!(
        opened(&new).model().experiment.result_frequency,
        Some(600.0)
    );
}
