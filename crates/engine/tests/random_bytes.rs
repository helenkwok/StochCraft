//! Runs on a file of random bytes (`random` with `"mode": "byte_file"`): the same results on
//! threads, in slices and on workers; a segment that runs out stops the run, and one that would
//! run past the end of the file is refused before it starts; the SHA-256 is checked and recorded;
//! runs are paired only when they read the same bytes; and the commands that cannot read a file
//! refuse one.

#![allow(clippy::expect_used, clippy::panic)] // A failed step fails the test.

use std::path::Path;

use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, Engine, ErrorCode, Worker};
use stochcraft_random::Rng;

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    e.execute(command, params)
        .unwrap_or_else(|err| panic!("{command}: {err}"))
}

fn refused(e: &mut Engine, command: &str, params: Value) -> (ErrorCode, String) {
    match e.execute(command, params) {
        Ok(v) => panic!("{command} should be refused: {v}"),
        Err(err) => (err.code, err.message),
    }
}

/// `n` bytes from xoshiro256** seeded with `seed`: a stand-in for a file of random bytes.
fn bytes(n: usize, seed: u64) -> Vec<u8> {
    let mut r = Rng::new(seed);
    let mut out = Vec::with_capacity(n + 8);
    while out.len() < n {
        out.extend(r.next_u64().to_le_bytes());
    }
    out.truncate(n);
    out
}

/// The help desk (the commands' example model), with a file of random bytes loaded as `data`.
fn help_desk(data: Vec<u8>) -> Engine {
    let mut e = Engine::new();
    ok(&mut e, "model.set", COMMANDS[0].example());
    e.load_random_data("data", data);
    e
}

/// The `random` parameter for the loaded file, with segments of `segment` bytes.
fn from_file(segment: u64) -> Value {
    json!({ "mode": "byte_file", "file": "data", "segment_bytes": segment })
}

fn element<'a>(results: &'a Value, id: &str) -> &'a Value {
    results["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|x| x["id"] == id))
        .unwrap_or_else(|| panic!("no element {id}: {results}"))
}

#[test]
fn a_run_on_a_file_gives_the_same_results_on_threads_in_slices_and_on_workers() {
    let mut e = help_desk(bytes(1 << 20, 7));
    let random = from_file(32_768);
    let one = ok(
        &mut e,
        "run",
        json!({ "replications": 6, "threads": 1, "random": random }),
    );
    let four = ok(
        &mut e,
        "run",
        json!({ "replications": 6, "threads": 4, "random": random }),
    );
    assert_eq!(one, four);
    assert_eq!(one["record"]["random"]["mode"], "byte_file", "{one}");
    assert!(
        one["record"]["random"]["bytes_used"]
            .as_u64()
            .is_some_and(|b| b > 0 && b % 8 == 0),
        "{one}"
    );

    // In slices of a few events.
    let started = ok(
        &mut e,
        "run.start",
        json!({ "replications": 6, "mode": "slices", "random": random }),
    );
    assert_eq!(started["state"], "running");
    let sliced = loop {
        let s = ok(&mut e, "run.status", json!({ "events": 37 }));
        if s["state"] != "running" {
            break s;
        }
    };
    assert_eq!(sliced["result"], one, "{sliced}");

    // On workers, answering last first.
    ok(
        &mut e,
        "run.start",
        json!({ "replications": 6, "mode": "workers", "random": random }),
    );
    let task = e.worker_task().expect("a task").to_owned();
    let workers: Vec<Worker> = (0..2)
        .map(|_| Worker::new(&task).expect("a worker"))
        .collect();
    for (n, index) in (0..6u64).rev().enumerate() {
        e.deliver(&workers[n % 2].run(index)).expect("taken");
    }
    let status = ok(&mut e, "run.status", json!({}));
    assert_eq!(status["result"], one, "{status}");

    // And a pseudo-random run is not the same run.
    let prng = ok(&mut e, "run", json!({ "replications": 6 }));
    assert_eq!(prng["record"]["random"]["mode"], "prng");
    assert_ne!(prng["elements"], one["elements"]);
}

#[test]
fn a_segment_that_runs_out_stops_the_run_and_its_bytes_are_never_read_twice() {
    let mut e = help_desk(bytes(4096, 3));
    // 64 bytes is eight draws: the first replication runs out long before its end.
    let err = e
        .execute(
            "run",
            json!({ "replications": 2, "threads": 1, "random": from_file(64) }),
        )
        .expect_err("the segment runs out");
    assert_eq!(err.code, ErrorCode::RandomDataExhausted, "{err:?}");
    let details = err.details.expect("details");
    assert_eq!(details["replication"], 0, "{details}");
    assert_eq!(details["bytes_used"], 64, "{details}");
    assert!(
        details["bytes_needed_estimate"]
            .as_u64()
            .is_some_and(|n| n > 64),
        "{details}"
    );
    assert!(err.message.contains("never read twice"), "{}", err.message);
    // In slices too.
    ok(
        &mut e,
        "run.start",
        json!({ "replications": 2, "mode": "slices", "random": from_file(64) }),
    );
    let status = loop {
        let s = ok(&mut e, "run.status", json!({}));
        if s["state"] != "running" {
            break s;
        }
    };
    assert_eq!(status["state"], "failed", "{status}");
    assert_eq!(status["error"]["code"], "random_data_exhausted", "{status}");
}

#[test]
fn a_file_too_short_for_every_replication_is_refused_before_the_run() {
    let mut e = help_desk(bytes(100_000, 3));
    // Ten replications of 16 KiB need 163,840 bytes.
    let (code, message) = refused(
        &mut e,
        "run",
        json!({ "replications": 10, "random": from_file(16_384) }),
    );
    assert_eq!(code, ErrorCode::InvalidParams);
    assert!(message.contains("163840"), "{message}");
    // So is one where an offset pushes the last segment past the end.
    let (code, _) = refused(
        &mut e,
        "run",
        json!({ "replications": 6, "random": { "mode": "byte_file", "file": "data", "offset": 10_000, "segment_bytes": 16_384 } }),
    );
    assert_eq!(code, ErrorCode::InvalidParams);
    // A sequential run is checked against its maximum, not its first batch.
    let (code, _) = refused(
        &mut e,
        "run",
        json!({ "replications": 2, "random": from_file(16_384),
                "sequential": { "relative": 0.05, "max": 50 } }),
    );
    assert_eq!(code, ErrorCode::InvalidParams);
}

#[test]
fn the_sha_256_is_recorded_and_a_replay_on_another_file_is_refused() {
    let data = bytes(1 << 18, 11);
    let mut e = help_desk(data.clone());
    let first = ok(
        &mut e,
        "run",
        json!({ "replications": 3, "random": from_file(16_384) }),
    );
    let record = first["record"]["random"].clone();
    let sha = record["sha256"].as_str().expect("a SHA-256").to_owned();
    assert_eq!(sha.len(), 64, "{record}");
    // The record's `random` repeats the run exactly.
    let again = ok(
        &mut e,
        "run",
        json!({ "replications": 3, "random": record }),
    );
    assert_eq!(again["elements"], first["elements"]);
    // The same name with other bytes is not the file the record names.
    let mut other = data;
    other[0] ^= 1;
    e.load_random_data("data", other);
    let (code, message) = refused(
        &mut e,
        "run",
        json!({ "replications": 3, "random": record }),
    );
    assert_eq!(code, ErrorCode::InvalidParams);
    assert!(message.contains(&sha), "{message}");
}

#[test]
fn a_file_is_read_from_its_path() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join("random-bytes-test.bin");
    std::fs::write(&path, bytes(1 << 17, 5)).expect("written");
    let mut e = help_desk(Vec::new());
    let path_text = path.to_string_lossy().into_owned();
    let r = ok(
        &mut e,
        "run",
        json!({ "replications": 2, "random": { "mode": "byte_file", "file": path_text, "segment_bytes": 32_768 } }),
    );
    assert_eq!(r["record"]["random"]["file"], path_text.as_str());
    assert_eq!(r["record"]["random"]["file_bytes"], 1 << 17);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn runs_are_paired_only_when_they_read_the_same_bytes() {
    let mut e = help_desk(bytes(1 << 18, 21));
    let a = ok(
        &mut e,
        "run",
        json!({ "replications": 4, "random": from_file(16_384) }),
    );
    let b = ok(
        &mut e,
        "run",
        json!({ "replications": 4, "random": from_file(16_384) }),
    );
    let same = ok(&mut e, "results.compare", json!({ "a": a, "b": b }));
    assert_eq!(same["paired"], true, "{same}");
    assert!(
        same["method"]
            .as_str()
            .is_some_and(|m| m.contains("same random-data file")),
        "{same}"
    );
    // Another file.
    e.load_random_data("data", bytes(1 << 18, 22));
    let c = ok(
        &mut e,
        "run",
        json!({ "replications": 4, "random": from_file(16_384) }),
    );
    let other = ok(&mut e, "results.compare", json!({ "a": a, "b": c }));
    assert_eq!(other["paired"], false, "{other}");
    // Another segment length of the same file.
    let d = ok(
        &mut e,
        "run",
        json!({ "replications": 4, "random": from_file(32_768) }),
    );
    let longer = ok(&mut e, "results.compare", json!({ "a": c, "b": d }));
    assert_eq!(longer["paired"], false, "{longer}");
    // The seed's pseudo-random numbers, with the same seed as the file runs recorded.
    let p = ok(&mut e, "run", json!({ "replications": 4 }));
    assert_eq!(p["seed"], a["seed"]);
    let mixed = ok(&mut e, "results.compare", json!({ "a": p, "b": a }));
    assert_eq!(mixed["paired"], false, "{mixed}");
    // Two pseudo-random runs with one seed are paired, as always.
    let q = ok(&mut e, "run", json!({ "replications": 4 }));
    let prng = ok(&mut e, "results.compare", json!({ "a": p, "b": q }));
    assert_eq!(prng["paired"], true, "{prng}");
}

/// Every draw reads eight bytes of the file, and nothing else does: a model whose only draws are
/// one time per entity at each step (a constant time between arrivals, a server with a setup and
/// a service time before every job, both exponential, a delay with uniform times) reads exactly
/// 8 × 3 bytes per entity. A stream left on the
/// generator would read fewer.
#[test]
fn the_bytes_read_are_eight_per_draw() {
    let mut e = Engine::new();
    ok(
        &mut e,
        "model.set",
        json!({ "model": {
            "name": "Counted draws",
            "elements": [
                { "id": "arrivals", "kind": "source", "interarrival": { "type": "constant", "value": 1.0 }, "limit": 50 },
                { "id": "desk", "kind": "server", "capacity": 1, "service": { "type": "exponential", "mean": 0.5 },
                  "setup": { "time": { "type": "exponential", "mean": 0.1 } } },
                { "id": "walk", "kind": "delay", "duration": { "type": "uniform", "min": 0.1, "max": 0.3 } },
                { "id": "exit", "kind": "sink" }
            ],
            "connections": [
                { "id": "f1", "from": "arrivals", "to": "desk" },
                { "id": "f2", "from": "desk", "to": "walk" },
                { "id": "f3", "from": "walk", "to": "exit" }
            ],
            "experiment": { "time_unit": "min", "warmup": 0.0, "duration": 10_000.0, "replications": 1, "seed": 1 }
        }}),
    );
    e.load_random_data("data", bytes(1 << 16, 9));
    let r = ok(
        &mut e,
        "run",
        json!({ "replications": 1, "random": from_file(1 << 16) }),
    );
    let desk = element(&r, "desk")["metrics"]["left"]["mean"].as_f64();
    let walk = element(&r, "walk")["metrics"]["left"]["mean"].as_f64();
    assert_eq!(desk, Some(50.0), "{r}");
    assert_eq!(walk, Some(50.0), "{r}");
    assert_eq!(
        r["record"]["random"]["bytes_used"],
        8 * (50 + 50 + 50),
        "{r}"
    );
    // A pilot counts the same.
    let pilot = ok(&mut e, "random.pilot", json!({ "replications": 1 }));
    assert_eq!(pilot["bytes"], json!([1200]), "{pilot}");
    assert_eq!(pilot["segment_bytes"], 4096, "{pilot}");
}

#[test]
fn without_a_segment_length_a_pilot_chooses_one_and_the_record_says_so() {
    let mut e = help_desk(bytes(1 << 20, 4));
    let pilot = ok(&mut e, "random.pilot", json!({}));
    let segment = pilot["segment_bytes"].as_u64().expect("a segment");
    assert_eq!(segment % 4096, 0, "{pilot}");
    let most = pilot["most_bytes"].as_u64().expect("most");
    assert!(segment >= most + most / 2, "{pilot}");
    let r = ok(
        &mut e,
        "run",
        json!({ "replications": 4, "random": { "mode": "byte_file", "file": "data" } }),
    );
    assert_eq!(r["record"]["random"]["segment_bytes"], segment, "{r}");
    assert_eq!(
        r["record"]["random"]["pilot"]["segment_bytes"], segment,
        "{r}"
    );
}

#[test]
fn a_seed_from_a_source_is_the_seed_with_its_source_recorded() {
    let mut e = help_desk(Vec::new());
    let plain = ok(&mut e, "run", json!({ "replications": 3, "seed": 424_242 }));
    let sourced = ok(
        &mut e,
        "run",
        json!({ "replications": 3, "seed": 424_242, "random": {
            "mode": "seed_source", "source": "a randomness beacon",
            "time": "2026-11-27T00:00:00Z", "pulse": "https://example.org/pulse/1"
        } }),
    );
    assert_eq!(plain["elements"], sourced["elements"]);
    let r = &sourced["record"]["random"];
    assert_eq!(r["mode"], "seed_source");
    assert_eq!(r["source"], "a randomness beacon");
    assert_eq!(r["pulse"], "https://example.org/pulse/1");
    // Its seed must be given.
    let (code, _) = refused(
        &mut e,
        "run",
        json!({ "random": { "mode": "seed_source", "source": "a beacon" } }),
    );
    assert_eq!(code, ErrorCode::InvalidParams);
}

fn opened(name: &str) -> Engine {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/models")
        .join(name);
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    e.load_random_data("data", bytes(1 << 16, 1));
    e
}

#[test]
fn system_dynamics_agents_and_monte_carlo_refuse_a_file() {
    let file = from_file(4096);
    for (model, command) in [
        ("logistic.xmile", "sd.run"),
        ("sir-agents.bpmn", "agents.run"),
        ("budget.bpmn", "mc.run"),
    ] {
        let mut e = opened(model);
        let (code, message) = refused(&mut e, command, json!({ "random": file }));
        assert_eq!(code, ErrorCode::Unsupported, "{model}: {message}");
        assert!(
            message.contains("cannot read random numbers from a file"),
            "{model}: {message}"
        );
        // `run` refuses these models whatever the random numbers.
        let (code, _) = refused(&mut e, "run", json!({ "random": file }));
        assert_eq!(code, ErrorCode::Unsupported, "{model}");
        // The seed's numbers are still taken.
        ok(&mut e, command, json!({ "random": { "mode": "prng" } }));
    }
}

#[test]
fn sweeps_designs_and_optimisation_refuse_a_file() {
    let mut e = help_desk(bytes(1 << 16, 2));
    let file = from_file(4096);
    for (command, params) in [
        (
            "experiment.sweep",
            json!({ "parameter": { "id": "desk", "field": "capacity" }, "values": [1, 2],
                    "results": [{ "element": "desk", "metric": "mean_wait" }], "random": file }),
        ),
        (
            "experiment.design",
            json!({ "factors": [{ "id": "desk", "field": "mean", "levels": [1.0, 1.2] }],
                    "results": [{ "element": "desk", "metric": "mean_wait" }], "random": file }),
        ),
        (
            "experiment.optimise",
            json!({ "decisions": [{ "id": "desk", "field": "capacity", "min": 1, "max": 3 }],
                    "objective": { "results": [{ "element": "desk", "metric": "mean_wait", "weight": 1.0 }] },
                    "indifference": 1.0, "random": file }),
        ),
    ] {
        let (code, message) = refused(&mut e, command, params);
        assert_eq!(code, ErrorCode::Unsupported, "{command}: {message}");
    }
}
