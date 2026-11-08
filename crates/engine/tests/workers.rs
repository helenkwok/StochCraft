//! Runs handed to the caller's workers (`"mode": "workers"`, how the browser uses every core)
//! give exactly the results of a run on threads, whichever worker runs which replication and in
//! whatever order they answer.

#![allow(clippy::expect_used, clippy::panic)] // A failed step fails the test.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use stochcraft_engine::{Engine, ErrorCode, Worker, wire};
use stochcraft_model::Model;

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    e.execute(command, params)
        .unwrap_or_else(|err| panic!("{command}: {err}"))
}

/// The committed test models, and the BPSim reference models when `cargo xtask fetch-bpsim`
/// has downloaded them.
fn models() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut paths: Vec<PathBuf> = ["mm1.bpmn", "tandem.xpdl", "distributions.bpmn"]
        .iter()
        .map(|n| root.join("tests/models").join(n))
        .collect();
    let mut dirs = vec![root.join("target/bpsim/guide")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p
                .extension()
                .is_some_and(|x| x == "bpmn" || x == "xpdl" || x == "bpsim")
            {
                paths.push(p);
            }
        }
    }
    paths.sort();
    paths
}

/// Runs the engine's model on `workers` workers, handing replications out from the last to the
/// first and taking turns, so answers arrive out of order.
fn on_workers(e: &mut Engine, replications: u32, workers: usize) -> Value {
    let status = ok(
        e,
        "run.start",
        json!({"replications": replications, "mode": "workers"}),
    );
    assert_eq!(status["state"], "running", "{status}");
    let task = e.worker_task().expect("a task while running").to_owned();
    let pool: Vec<Worker> = (0..workers)
        .map(|_| Worker::new(&task).expect("the task makes a worker"))
        .collect();
    for (n, index) in (0..u64::from(replications)).rev().enumerate() {
        let answer = pool[n % workers].run(index);
        e.deliver(&answer).expect("the answer is taken");
        let s = ok(e, "run.status", json!({}));
        if index > 0 {
            assert_eq!(s["state"], "running");
            assert_eq!(s["done"], n as u64 + 1);
        }
    }
    assert!(e.worker_task().is_none(), "no task once finished");
    ok(e, "run.status", json!({}))
}

#[test]
fn workers_give_the_results_of_threads() {
    let mut ran = 0;
    for path in models() {
        let mut e = Engine::new();
        let path_text = path.to_string_lossy().into_owned();
        if e.execute("file.open", json!({ "path": path_text }))
            .is_err()
        {
            continue;
        }
        // The model goes to workers as JSON: it must come back as the same model.
        let doc = serde_json::to_string(&wire::ModelDoc::from(e.model())).expect("JSON");
        let back: wire::ModelDoc = serde_json::from_str(&doc).expect("reads back");
        assert_eq!(&Model::from(back), e.model(), "{path_text}");

        let Ok(threads) = e.execute("run", json!({"replications": 5, "threads": 2})) else {
            continue; // A reference model StochCraft cannot run yet.
        };
        let status = on_workers(&mut e, 5, 2);
        assert_eq!(status["state"], "finished", "{path_text}: {status}");
        assert_eq!(status["result"], threads, "{path_text}");
        ran += 1;
    }
    eprintln!("{ran} models ran on workers");
    assert!(ran >= 3, "only {ran} models ran");
}

#[test]
fn a_cancelled_or_failing_run_on_workers_says_so() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/mm1.bpmn");
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": root.to_string_lossy() }),
    );
    ok(
        &mut e,
        "run.start",
        json!({"replications": 4, "mode": "workers"}),
    );
    let worker = Worker::new(e.worker_task().expect("task")).expect("worker");
    e.deliver(&worker.run(0)).expect("taken");
    let cancelled = ok(&mut e, "run.cancel", json!({}));
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    // Answers after the end are ignored.
    e.deliver(&worker.run(1)).expect("ignored");
    assert_eq!(ok(&mut e, "run.status", json!({}))["state"], "cancelled");

    // Garbage, or an answer for a replication the run does not have, is refused and the run
    // goes on.
    ok(
        &mut e,
        "run.start",
        json!({"replications": 2, "mode": "workers"}),
    );
    for bad in ["not json", r#"{"index": 7, "report": null}"#] {
        let err = e.deliver(bad).expect_err("refused");
        assert_eq!(err.code, ErrorCode::InvalidParams, "{bad}");
    }
    assert_eq!(ok(&mut e, "run.status", json!({}))["state"], "running");
    // A worker that failed fails the run with its error.
    e.deliver(r#"{"index": 1, "error": {"code": "time_stopped", "message": "replication 1: time stopped"}}"#)
        .expect("taken");
    let failed = ok(&mut e, "run.status", json!({}));
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["error"]["code"], "time_stopped");

    // A task that is not a run, or a model with errors, makes no worker.
    assert!(Worker::new("{}").is_err());
    let broken = json!({
        "model": { "name": "broken", "elements": [],
                   "connections": [{ "id": "c", "from": "a", "to": "b" }] },
        "seed": 1,
        "replications": 1,
    });
    assert_eq!(
        Worker::new(&broken.to_string()).err().map(|e| e.code),
        Some(ErrorCode::InvalidModel)
    );
}

#[test]
fn every_error_code_is_found_by_its_name() {
    for name in [
        "unknown_command",
        "invalid_params",
        "not_found",
        "conflict",
        "nothing_to_undo",
        "nothing_to_redo",
        "invalid_model",
        "io",
        "bad_file",
        "unsupported",
        "cancelled",
        "time_stopped",
        "internal",
    ] {
        assert_eq!(ErrorCode::named(name).map(ErrorCode::name), Some(name));
    }
    assert_eq!(ErrorCode::named("nonsense"), None);
}
