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

/// The `random` parameter for the loaded file, from byte `offset` in segments of `segment` bytes.
fn file_at(offset: u64, segment: u64) -> Value {
    json!({ "mode": "byte_file", "file": "data", "offset": offset, "segment_bytes": segment })
}

fn compared(e: &mut Engine, a: &Value, b: &Value) -> Value {
    ok(e, "results.compare", json!({ "a": a, "b": b }))
}

/// Every comparison's (`interval`, `difference`, `half_width`).
fn intervals(r: &Value) -> Vec<(Value, Option<f64>, Option<f64>)> {
    r["comparisons"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| {
            (
                c["interval"].clone(),
                c["difference"].as_f64(),
                c["half_width"].as_f64(),
            )
        })
        .collect()
}

/// Two runs are paired when replication i of each read the same bytes, independent when they
/// share none, and dependent when they share some but not replication for replication.
#[test]
fn runs_are_compared_by_which_bytes_they_read() {
    let mut e = help_desk(bytes(1 << 18, 21));
    let run = |e: &mut Engine, n: u64, random: Value| {
        ok(e, "run", json!({ "replications": n, "random": random }))
    };
    // Bytes 0 to 65,535 in four segments.
    let a = run(&mut e, 4, file_at(0, 16_384));
    let b = run(&mut e, 4, file_at(0, 16_384));
    let same = compared(&mut e, &a, &b);
    assert_eq!(
        (&same["class"], &same["basis"], &same["paired"]),
        (&json!("paired"), &json!("same_file"), &json!(true)),
        "{same}"
    );
    assert_eq!(same["paired_replications"], 4, "{same}");
    assert!(same.get("reason").is_none(), "{same}");
    assert!(
        same["method"]
            .as_str()
            .is_some_and(|m| m.contains("same random-data file") && m.contains("shift")),
        "{same}"
    );
    // The same model on the same bytes: exactly no difference.
    for (interval, d, hw) in intervals(&same) {
        assert_eq!(interval, "paired_t", "{same}");
        assert!(
            d.is_none_or(|d| d == 0.0) && hw.is_none_or(|h| h == 0.0),
            "{same}"
        );
    }

    // More replications from the same place: paired over the first four, each the same.
    let eight = run(&mut e, 8, file_at(0, 16_384));
    let longer = compared(&mut e, &a, &eight);
    assert_eq!(
        (&longer["class"], &longer["paired_replications"]),
        (&json!("paired"), &json!(4)),
        "{longer}"
    );
    for c in longer["comparisons"].as_array().into_iter().flatten() {
        assert!(c["pairs"].as_u64().is_some_and(|p| p <= 4), "{c}");
        assert!(c["difference"].as_f64().is_none_or(|d| d == 0.0), "{c}");
    }

    // One segment further on: b's replication i reads a's replication i + 1's bytes.
    let shifted = run(&mut e, 4, file_at(16_384, 16_384));
    // Segments twice as long from the same place: b's first reads a's first two.
    let wide = run(&mut e, 4, file_at(0, 32_768));
    for other in [&shifted, &wide] {
        let r = compared(&mut e, &a, other);
        assert_eq!(
            (&r["class"], &r["basis"], &r["paired"]),
            (
                &json!("dependent"),
                &json!("overlapping_parts"),
                &json!(false)
            ),
            "{r}"
        );
        assert!(
            r["reason"]
                .as_str()
                .is_some_and(|m| m.contains("neither independent nor paired")),
            "{r}"
        );
        assert!(r.get("paired_replications").is_none(), "{r}");
        for c in r["comparisons"].as_array().into_iter().flatten() {
            assert_eq!(c["interval"], "none", "{c}");
            assert!(
                c["half_width"].is_null() && c["df"].is_null() && c["significant"].is_null(),
                "{c}"
            );
            assert!(c.get("pairs").is_none(), "{c}");
            // Only the means, and b's minus a's.
            if let (Some(ma), Some(mb), Some(d)) = (
                c["a"]["mean"].as_f64(),
                c["b"]["mean"].as_f64(),
                c["difference"].as_f64(),
            ) {
                assert!((d - (mb - ma)).abs() <= 1e-9 * (1.0 + mb.abs()), "{c}");
            }
        }
    }

    // The next four segments, which a never read: independent.
    let apart = run(&mut e, 4, file_at(65_536, 16_384));
    let r = compared(&mut e, &a, &apart);
    assert_eq!(
        (&r["class"], &r["basis"]),
        (&json!("independent"), &json!("disjoint_parts")),
        "{r}"
    );
    assert!(intervals(&r).iter().all(|(i, _, _)| i == "welch_t"), "{r}");
    // Touching but not overlapping is still apart: a's eight segments end where it starts.
    let next = run(&mut e, 4, file_at(131_072, 16_384));
    assert_eq!(compared(&mut e, &eight, &next)["class"], "independent");
    assert_eq!(compared(&mut e, &apart, &eight)["class"], "dependent");

    // Another file: taken as independent, and said to be assumed.
    e.load_random_data("data", bytes(1 << 18, 22));
    let c = run(&mut e, 4, file_at(0, 16_384));
    let r = compared(&mut e, &a, &c);
    assert_eq!(
        (&r["class"], &r["basis"], &r["paired"]),
        (
            &json!("independent"),
            &json!("different_files"),
            &json!(false)
        ),
        "{r}"
    );
    assert!(
        r["method"].as_str().is_some_and(|m| m.contains("assumed")),
        "{r}"
    );

    // The seed's pseudo-random numbers, with the same seed as the file runs recorded.
    let p = ok(&mut e, "run", json!({ "replications": 4 }));
    assert_eq!(p["seed"], a["seed"]);
    let mixed = compared(&mut e, &p, &a);
    assert_eq!(
        (&mixed["class"], &mixed["basis"]),
        (&json!("independent"), &json!("seed_and_file")),
        "{mixed}"
    );
    // Two pseudo-random runs with one seed are paired, as always.
    let q = ok(&mut e, "run", json!({ "replications": 4 }));
    let prng = compared(&mut e, &p, &q);
    assert_eq!(
        (&prng["class"], &prng["basis"], &prng["paired"]),
        (&json!("paired"), &json!("same_seed"), &json!(true)),
        "{prng}"
    );
}

/// Replication i's pseudo-random numbers depend only on the seed and i: runs of 10 and of 20
/// replications with one seed are paired over the first 10, which are the same runs.
#[test]
fn the_same_seed_pairs_the_first_replications_of_runs_of_different_lengths() {
    let mut e = help_desk(Vec::new());
    let ten = ok(&mut e, "run", json!({ "replications": 10 }));
    let twenty = ok(&mut e, "run", json!({ "replications": 20 }));
    let r = compared(&mut e, &ten, &twenty);
    assert_eq!(
        (
            &r["class"],
            &r["basis"],
            &r["paired"],
            &r["paired_replications"]
        ),
        (
            &json!("paired"),
            &json!("same_seed"),
            &json!(true),
            &json!(10)
        ),
        "{r}"
    );
    let mut checked = 0;
    for c in r["comparisons"].as_array().into_iter().flatten() {
        assert_eq!(c["interval"], "paired_t", "{c}");
        assert!(c["pairs"].as_u64().is_some_and(|p| p <= 10), "{c}");
        // The same first ten replications: no difference at all.
        assert!(c["difference"].as_f64().is_none_or(|d| d == 0.0), "{c}");
        assert!(c["half_width"].as_f64().is_none_or(|h| h == 0.0), "{c}");
        // Each run's mean is over all its replications.
        assert!(c["b"]["n"].as_u64() >= c["a"]["n"].as_u64(), "{c}");
        if c["pairs"] == 10 {
            checked += 1;
        }
    }
    assert!(checked > 0, "{r}");
    // The same the other way round.
    let back = compared(&mut e, &twenty, &ten);
    assert_eq!(back["paired_replications"], 10, "{back}");
}

/// A record that cannot say which bytes a run read, or values that are not one per
/// replication, are refused rather than compared.
#[test]
fn malformed_results_are_refused_by_the_comparison() {
    let mut e = help_desk(bytes(1 << 18, 5));
    let a = ok(
        &mut e,
        "run",
        json!({ "replications": 2, "random": from_file(16_384) }),
    );
    let b = a.clone();
    for (path, value) in [
        ("sha256", Value::Null),
        ("sha256", json!("not a hash")),
        ("offset", json!("zero")),
        ("segment_bytes", json!(0)),
        ("mode", json!("dice")),
    ] {
        let mut bad = a.clone();
        bad["record"]["random"][path] = value.clone();
        let (code, message) = refused(&mut e, "results.compare", json!({ "a": bad, "b": b }));
        assert_eq!(
            code,
            ErrorCode::InvalidParams,
            "{path} = {value}: {message}"
        );
    }
    // Two records each missing their SHA-256 are not the same file.
    let mut x = a.clone();
    if let Some(o) = x["record"]["random"].as_object_mut() {
        o.remove("sha256");
    }
    let (code, _) = refused(&mut e, "results.compare", json!({ "a": x, "b": x }));
    assert_eq!(code, ErrorCode::InvalidParams);
    // A list of values that is not one per replication.
    let mut short = a.clone();
    short["values"]["desk"]["mean_wait"] = json!([1.0]);
    let (code, message) = refused(&mut e, "results.compare", json!({ "a": short, "b": b }));
    assert_eq!(code, ErrorCode::InvalidParams, "{message}");
    assert!(message.contains("one per replication"), "{message}");
    // A value that is neither a number nor null.
    let mut text = a.clone();
    text["values"]["desk"]["mean_wait"] = json!(["1", 2.0]);
    let (code, _) = refused(&mut e, "results.compare", json!({ "a": text, "b": b }));
    assert_eq!(code, ErrorCode::InvalidParams);
}

/// A worker's answer is matched to its replication first: one for a replication the run does
/// not have is refused, one for a replication that has answered is ignored (even an error), and
/// a worker that stopped fails the run whatever has answered.
#[test]
fn a_workers_answer_is_checked_by_its_index_first_and_counted_once() {
    let mut e = help_desk(bytes(1 << 20, 7));
    ok(
        &mut e,
        "run.start",
        json!({ "replications": 3, "mode": "workers", "random": from_file(32_768) }),
    );
    let task = e.worker_task().expect("a task").to_owned();
    let worker = Worker::new(&task).expect("a worker");
    let first = worker.run(0);
    e.deliver(&first).expect("taken");
    e.deliver(&first).expect("a second copy is ignored");
    let status = ok(&mut e, "run.status", json!({}));
    assert_eq!(
        (&status["state"], &status["done"]),
        (&json!("running"), &json!(1)),
        "{status}"
    );
    let error = r#""error": {"code": "time_stopped", "message": "replication 0: time stopped"}"#;
    e.deliver(&format!(r#"{{"index": 0, {error}}}"#))
        .expect("an error for a replication that answered is ignored");
    assert_eq!(ok(&mut e, "run.status", json!({}))["state"], "running");
    let err = e
        .deliver(&format!(r#"{{"index": 9, {error}}}"#))
        .expect_err("no replication 9");
    assert_eq!(err.code, ErrorCode::InvalidParams);
    assert_eq!(ok(&mut e, "run.status", json!({}))["state"], "running");
    let err = e
        .deliver(r#"{"report": null}"#)
        .expect_err("no index, no error");
    assert_eq!(err.code, ErrorCode::InvalidParams);
    // A worker that stopped, after replication 0 answered.
    e.deliver(r#"{"error": {"code": "internal", "message": "a web worker stopped: unreachable"}}"#)
        .expect("taken");
    let failed = ok(&mut e, "run.status", json!({}));
    assert_eq!(failed["state"], "failed", "{failed}");
    assert_eq!(failed["error"]["code"], "internal", "{failed}");

    // A worker asked for a replication past the bytes its task carries answers with an error.
    let past: Value = serde_json::from_str(&worker.run(3)).expect("an answer");
    assert_eq!(past["index"], 3, "{past}");
    assert_eq!(past["error"]["code"], "invalid_params", "{past}");
}

/// Every worker gets the bytes of every replication the run may have, so a run on workers whose
/// replications need more than the cap is refused before it starts; in slices it runs.
#[test]
fn a_run_on_workers_with_too_many_bytes_is_refused_before_it_starts() {
    let segment: u64 = 3 << 20;
    let mut e = help_desk(vec![0x5a; 9 << 20]);
    let random = from_file(segment);
    let (code, message) = refused(
        &mut e,
        "run.start",
        json!({ "replications": 3, "mode": "workers", "random": random }),
    );
    assert_eq!(code, ErrorCode::InvalidParams, "{message}");
    assert!(message.contains("slices"), "{message}");
    let err = e
        .execute(
            "run.start",
            json!({ "replications": 3, "mode": "workers", "random": random }),
        )
        .expect_err("refused");
    assert_eq!(
        err.details,
        Some(json!({ "bytes": 3 * segment, "limit_bytes": 8 << 20 }))
    );
    assert!(e.worker_task().is_none());
    // Two replications fit.
    ok(
        &mut e,
        "run.start",
        json!({ "replications": 2, "mode": "workers", "random": random }),
    );
    assert!(e.worker_task().is_some());
    ok(&mut e, "run.cancel", json!({}));
}

/// A worker checks its task's bytes before it decodes them.
#[test]
fn a_worker_refuses_bytes_that_do_not_fit_its_task() {
    let mut e = help_desk(bytes(1 << 16, 3));
    ok(
        &mut e,
        "run.start",
        json!({ "replications": 2, "mode": "workers", "random": from_file(4096) }),
    );
    let task: Value = serde_json::from_str(e.worker_task().expect("a task")).expect("json");
    assert!(Worker::new(&task.to_string()).is_ok());
    let with = |segment: Value, hex: Value| {
        let mut t = task.clone();
        t["random"] = json!({ "segment_bytes": segment, "hex": hex });
        Worker::new(&t.to_string()).err().map(|e| e.code)
    };
    let hex = task["random"]["hex"].clone();
    let text = hex.as_str().expect("hex").to_owned();
    for (segment, hex) in [
        // Segments of fewer than eight bytes.
        (json!(4), hex.clone()),
        // Not a whole number of segments.
        (json!(4096), json!(format!("{text}00"))),
        // Fewer segments than replications.
        (json!(4096), json!(text[..8192].to_owned())),
        // An odd number of digits, and not hexadecimal.
        (json!(4096), json!(format!("{text}0"))),
        (json!(4096), json!("zz".repeat(8192))),
        // More than the cap.
        (json!(4096), json!("00".repeat((8 << 20) + 4096))),
    ] {
        assert_eq!(with(segment, hex), Some(ErrorCode::InvalidParams));
    }
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
