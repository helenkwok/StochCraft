//! Sequential replications (#224): a run that adds replications until the results it watches
//! are precise enough, or it reaches its maximum. Replication i is the same as in a fixed run, so
//! a sequential run that stops at n gives exactly the results of a fixed run of n, on threads, in
//! slices or on the caller's workers alike.

#![allow(clippy::expect_used, clippy::panic)] // A failed step fails the test.

use std::path::Path;

use serde_json::{Value, json};
use stochcraft_engine::{Engine, ErrorCode, Worker};
use stochcraft_model::{Dist, Experiment, Sequential};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    e.execute(command, params.clone())
        .unwrap_or_else(|err| panic!("{command} {params}: {err}"))
}

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

/// What two runs must agree on to be the same results: everything but the run record (which
/// says whether the run was sequential) and the sequential summary.
fn numbers(results: &Value) -> Value {
    let mut v = results.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("record");
        o.remove("sequential");
    }
    v
}

/// The default: every key measure within ± 5% of its mean (± 1 point for a share), at most 200
/// replications, after the scenario's 16. M/M/1 at a load of 0.75 over 20,000 minutes settles
/// well before the maximum.
#[test]
fn a_sequential_run_stops_with_every_key_measure_settled_and_equals_a_fixed_run() {
    let mut e = mm1();
    ok(&mut e, "experiment.set", json!({ "sequential": {} }));
    let r = ok(&mut e, "run", json!({}));
    let q = &r["sequential"];
    assert_eq!(q["reason"], "target_met", "{q}");
    assert_eq!(q["met"], true);
    let n = q["replications"].as_u64().expect("a count");
    assert!((16..200).contains(&n), "{q}");
    assert_eq!(r["replications"], n);
    assert_eq!(r["record"]["replications"], n);
    assert_eq!(q["stages"][0], 16);
    assert_eq!(
        q["stages"].as_array().and_then(|s| s.last()),
        Some(&json!(n))
    );
    let measures = q["measures"].as_array().expect("the measures watched");
    assert!(measures.len() >= 4, "{q}");
    for m in measures {
        assert_eq!(m["status"], "settled", "{m}");
        assert!(m["score"].as_f64().is_some_and(|s| s <= 1.0), "{m}");
    }
    assert_eq!(q["no_spread"], json!([]));
    assert_eq!(q["settings"]["max"], 200);
    assert_eq!(q["settings"]["relative"], 0.05);
    // The run record says the interval is an estimate of precision after adaptive stopping.
    let uncertainty = r["record"]["uncertainty"].as_str().unwrap_or("");
    assert!(
        uncertainty.starts_with("estimated precision after adaptive stopping"),
        "{uncertainty}"
    );
    assert_eq!(r["record"]["sequential"]["first"], 16);

    // A fixed run of the same count: the same numbers, bit for bit.
    let fixed = ok(
        &mut e,
        "run",
        json!({ "replications": n, "sequential": null }),
    );
    assert!(fixed["sequential"].is_null());
    assert_eq!(numbers(&fixed), numbers(&r));
    assert!(
        fixed["record"]["uncertainty"]
            .as_str()
            .is_some_and(|u| u.starts_with("95% Student-t")),
        "{}",
        fixed["record"]["uncertainty"]
    );
    assert!(fixed["record"]["sequential"].is_null());
}

/// A run with no sequential setting is as before: no `sequential` in its results or record.
#[test]
fn a_fixed_run_is_unchanged() {
    let mut e = mm1();
    let r = ok(&mut e, "run", json!({ "replications": 3 }));
    assert!(r.get("sequential").is_none());
    assert!(r["record"].get("sequential").is_none());
    assert_eq!(r["replications"], 3);
    assert_eq!(r["record"]["replications"], 3);
}

/// One result against an absolute half-width, as `run` overrides it for one run, and a target
/// too tight to meet: the run goes on to its maximum and says so.
#[test]
fn one_named_result_and_a_target_out_of_reach() {
    let mut e = mm1();
    ok(&mut e, "experiment.set", json!({ "duration": 2000 }));
    let r = ok(
        &mut e,
        "run",
        json!({ "replications": 4, "sequential": {
            "measure": { "element": "serve", "metric": "utilisation", "half_width": 0.5 },
            "max": 30,
        }}),
    );
    // ± 50 points on a utilisation is met at once.
    assert_eq!(
        r["sequential"]["reason"], "target_met",
        "{}",
        r["sequential"]
    );
    assert_eq!(r["sequential"]["replications"], 4);
    assert_eq!(r["sequential"]["measures"][0]["metric"], "utilisation");
    // ± 0.0000001% of the mean wait: out of reach of 12 replications.
    let r = ok(
        &mut e,
        "run",
        json!({ "replications": 4, "sequential": {
            "measure": { "element": "serve", "metric": "mean_wait" },
            "relative": 1e-9,
            "max": 12,
        }}),
    );
    assert_eq!(
        r["sequential"]["reason"], "max_reached",
        "{}",
        r["sequential"]
    );
    assert_eq!(r["sequential"]["replications"], 12);
    assert_eq!(r["sequential"]["met"], false);
    // The experiment's own setting is untouched by an override.
    assert!(e.model().experiment.sequential.is_none());
}

/// Results whose replications all gave the same value are never settled by that alone: a line
/// with fixed times gives the same results in every run, so the run goes on to its maximum, and
/// the results say there was no spread in the runs made (Dot's rare disruption, at model level:
/// the first runs agreeing says nothing of a rare event they have not met).
#[test]
fn zero_spread_runs_to_the_maximum_and_says_so() {
    let mut b = stochcraft_blocks::Builder::new();
    let src = b.source("arrivals", Dist::Constant(1.0), None);
    let desk = b.server("desk", 1, Dist::Constant(0.5));
    let out = b.sink("exit");
    b.connect(src, desk);
    b.connect(desk, out);
    let mut e = Engine::with_model(b.build(Experiment {
        duration: 100.0,
        replications: 3,
        sequential: Some(Sequential {
            max: 12,
            ..Sequential::default()
        }),
        ..Experiment::default()
    }));
    let r = ok(&mut e, "run", json!({}));
    let q = &r["sequential"];
    assert_eq!(q["reason"], "no_spread", "{q}");
    assert_eq!(q["met"], false);
    assert_eq!(q["replications"], 12);
    assert_eq!(q["stages"], json!([3, 6, 12]));
    let no_spread = q["no_spread"].as_array().expect("a list");
    assert!(
        no_spread.contains(&json!({ "element": "desk", "metric": "utilisation" })),
        "{q}"
    );
    assert!(
        no_spread.contains(&json!({ "element": "exit", "metric": "entered" })),
        "{q}"
    );
    // Every result has no spread: the same mean, a half-width of 0.
    let entered = &r["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|e| e["id"] == "exit"))
        .expect("the exit")["metrics"]["entered"];
    assert_eq!(entered["half_width"], 0.0);
    assert_eq!(entered["std_dev"], 0.0);
}

/// A count fixed by the model (arrivals on a timetable) has no spread, but does not hold the run
/// back: once the results that vary are settled, the run stops, names the fixed count, and says
/// how often per run an event none of its replications met may still happen.
#[test]
fn a_fixed_count_does_not_hold_back_results_that_vary() {
    let mut b = stochcraft_blocks::Builder::new();
    let src = b.source("arrivals", Dist::Constant(1.0), None);
    let desk = b.server("desk", 1, Dist::Exponential { mean: 0.5 });
    let out = b.sink("exit");
    b.connect(src, desk);
    b.connect(desk, out);
    let mut e = Engine::with_model(b.build(Experiment {
        duration: 1000.0,
        replications: 10,
        sequential: Some(Sequential {
            relative: 0.5,
            points: 5.0,
            ..Sequential::default()
        }),
        ..Experiment::default()
    }));
    let r = ok(&mut e, "run", json!({}));
    let q = &r["sequential"];
    assert_eq!(q["reason"], "target_met", "{q}");
    let n = q["replications"].as_u64().expect("a count");
    assert!(n < 200, "{q}");
    let no_spread = q["no_spread"].as_array().expect("a list");
    assert!(
        no_spread.contains(&json!({ "element": "arrivals", "metric": "left" })),
        "{q}"
    );
    let p = q["unseen_rate"].as_f64().expect("a rate");
    assert_eq!(Some(p), stochcraft_engine::precision::unseen_rate(n), "{q}");
}

/// The setting is part of the experiment: set, shown in the model, checked, undone and removed.
#[test]
fn the_setting_is_set_shown_checked_and_removed() {
    let mut e = mm1();
    ok(
        &mut e,
        "experiment.set",
        json!({ "sequential": { "relative": 0.02, "max": 64 } }),
    );
    let x = &e.model().experiment;
    let q = x.sequential.as_ref().expect("sequential");
    assert_eq!((q.relative, q.points, q.max), (0.02, 1.0, 64));
    assert!(q.measure.is_none());
    let model = ok(&mut e, "model.get", json!({}));
    assert_eq!(
        model["experiment"]["sequential"],
        json!({ "relative": 0.02, "points": 1.0, "max": 64 })
    );
    // Bad settings are refused, and the model keeps its own.
    for (bad, code) in [
        (json!({ "relative": 0.0 }), ErrorCode::InvalidParams),
        (json!({ "points": -1.0 }), ErrorCode::InvalidParams),
        (json!({ "max": 0 }), ErrorCode::InvalidParams),
        (
            json!({ "measure": { "element": "nowhere", "metric": "mean_wait" } }),
            ErrorCode::NotFound,
        ),
        (
            json!({ "measure": { "element": "serve", "metric": "speed" } }),
            ErrorCode::InvalidParams,
        ),
        (
            json!({ "measure": { "element": "serve", "metric": "mean_wait", "half_width": 0.0 } }),
            ErrorCode::InvalidParams,
        ),
    ] {
        let err = e
            .execute("experiment.set", json!({ "sequential": bad }))
            .expect_err("refused");
        assert_eq!(err.code, code, "{bad}: {err}");
    }
    assert_eq!(
        e.model().experiment.sequential.as_ref().map(|q| q.max),
        Some(64)
    );
    // `null` turns it off; undo turns it back on.
    ok(&mut e, "experiment.set", json!({ "sequential": null }));
    assert!(e.model().experiment.sequential.is_none());
    let model = ok(&mut e, "model.get", json!({}));
    assert!(model["experiment"].get("sequential").is_none());
    ok(&mut e, "undo", json!({}));
    assert!(e.model().experiment.sequential.is_some());
}

/// Runs `e`'s experiment on the caller's workers, handing out the replications the run has set
/// out to have, last first, and more whenever a look makes it go on.
fn on_workers(e: &mut Engine) -> Value {
    let started = ok(e, "run.start", json!({ "mode": "workers" }));
    assert_eq!(started["state"], "running", "{started}");
    let task = e.worker_task().expect("a task while running").to_owned();
    let worker = Worker::new(&task).expect("the task makes a worker");
    let mut handed = 0;
    loop {
        let status = ok(e, "run.status", json!({}));
        if status["state"] != "running" {
            return status;
        }
        let total = status["total"].as_u64().expect("a total");
        assert!(
            total > handed,
            "running with every replication answered: {status}"
        );
        for index in (handed..total).rev() {
            e.deliver(&worker.run(index)).expect("the answer is taken");
        }
        handed = total;
    }
}

/// Runs `e`'s experiment in slices of `events` events.
fn in_slices(e: &mut Engine, events: u64) -> Value {
    let started = ok(e, "run.start", json!({ "mode": "slices" }));
    assert_eq!(started["state"], "running", "{started}");
    loop {
        let status = ok(e, "run.status", json!({ "step": 1, "events": events }));
        if status["state"] != "running" {
            return status;
        }
    }
}

/// Runs `e`'s experiment on a background thread, waiting for it.
fn on_a_thread(e: &mut Engine) -> Value {
    let started = ok(e, "run.start", json!({ "mode": "threads", "threads": 2 }));
    assert_eq!(started["state"], "running", "{started}");
    loop {
        let status = ok(e, "run.status", json!({}));
        if status["state"] != "running" {
            return status;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// The same sequential run on threads, in slices and on workers: the same looks, the same count
/// and exactly the same results.
#[test]
fn every_way_of_running_gives_the_same_sequential_run() {
    let mut e = mm1();
    ok(
        &mut e,
        "experiment.set",
        json!({ "duration": 2000, "replications": 4,
                "sequential": { "relative": 0.1, "points": 0.5, "max": 40 } }),
    );
    let now = ok(&mut e, "run", json!({}));
    let n = now["sequential"]["replications"].as_u64().expect("a count");
    assert!(
        now["sequential"]["stages"]
            .as_array()
            .is_some_and(|s| s.len() > 1),
        "more than one look, so batches are added: {}",
        now["sequential"]
    );
    let threads = on_a_thread(&mut e);
    assert_eq!(threads["state"], "finished", "{threads}");
    assert_eq!(threads["total"], n);
    assert_eq!(threads["result"], now);
    for events in [1_000, u64::MAX] {
        let sliced = in_slices(&mut e, events);
        assert_eq!(sliced["state"], "finished", "{sliced}");
        assert_eq!(sliced["total"], n);
        assert_eq!(sliced["result"], now, "slices of {events}");
    }
    let workers = on_workers(&mut e);
    assert_eq!(workers["state"], "finished", "{workers}");
    assert_eq!(workers["total"], n);
    assert_eq!(workers["done"], n);
    assert_eq!(workers["result"], now);
}

/// A sequential run is cancelled like any other, between or within its batches.
#[test]
fn a_sequential_run_can_be_cancelled() {
    let mut e = mm1();
    ok(
        &mut e,
        "experiment.set",
        json!({ "replications": 2, "sequential": { "relative": 1e-6, "max": 100_000 } }),
    );
    let started = ok(&mut e, "run.start", json!({ "mode": "slices" }));
    assert_eq!(started["state"], "running");
    for _ in 0..50 {
        ok(&mut e, "run.status", json!({ "events": 50_000 }));
    }
    let cancelled = ok(&mut e, "run.cancel", json!({}));
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
}
