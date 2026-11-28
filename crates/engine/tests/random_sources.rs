//! Random number sources through the engine, with a scripted fetch that never calls out: a seed
//! drawn from a recorded beacon pulse and its provenance in the run record; a live run that
//! equals a run on the file it recorded; the quota estimate; the refusals; and a key that never
//! appears in any command's output, the run record, the journal or an error.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)] // A failed step fails the test.

use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, Engine, SLICE_EVENTS};
use stochcraft_qrng::testing::{ScriptedFetch, anu_stream};
use stochcraft_qrng::{FetchError, MemoryKeys};

const PULSE: &[u8] = include_bytes!("../../qrng/tests/fixtures/nist-beacon-pulse.json");
const KEY: &str = "SENTINEL-KEY-7f3a91";

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    e.execute(command, params)
        .unwrap_or_else(|err| panic!("{command}: {err}"))
}

fn scratch(name: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("random-sources");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name).to_string_lossy().into_owned()
}

fn help_desk(fetch: ScriptedFetch) -> Engine {
    let mut e = Engine::new();
    ok(&mut e, "model.set", COMMANDS[0].example());
    e.set_random_net(Box::new(fetch), Box::new(MemoryKeys::default()));
    e
}

fn without_record(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.remove("record");
    }
    v
}

#[test]
fn a_seed_from_the_beacon_is_recorded_and_the_run_repeats_without_calling_out() {
    let mut e = help_desk(ScriptedFetch::always(PULSE.to_vec()));
    let draw = ok(
        &mut e,
        "random.seed.draw",
        json!({ "source": "nist-beacon" }),
    );
    assert_eq!(draw["state"], "finished", "{draw}");
    let r = &draw["result"];
    let seed = r["seed"].as_u64().unwrap();
    assert!(seed < 1 << 53);
    assert_eq!(
        r["random"]["pulse"],
        "https://beacon.nist.gov/beacon/2.0/chain/2/pulse/2044989"
    );
    assert!(
        r["random"]["source"]
            .as_str()
            .unwrap()
            .contains("beacon.nist.gov")
    );
    let run = ok(
        &mut e,
        "run",
        json!({ "replications": 2, "seed": seed, "random": r["random"] }),
    );
    let rec = &run["record"];
    assert_eq!(rec["seed"], seed);
    assert_eq!(rec["random"]["mode"], "seed_source");
    assert_eq!(rec["random"]["pulse"], r["random"]["pulse"]);
    // The draw is not journalled: replaying the journal runs from the seed alone.
    let journal = serde_json::to_string(&e.journal_document()).unwrap();
    assert!(!journal.contains("random.seed.draw"));
    let mut again = Engine::new();
    ok(&mut again, "model.set", COMMANDS[0].example());
    let replay = ok(
        &mut again,
        "run",
        json!({ "replications": 2, "seed": seed, "random": rec["random"] }),
    );
    assert_eq!(without_record(replay), without_record(run));
}

/// Runs `params` with run.start and polls it to its end.
fn run_started(e: &mut Engine, params: Value) -> Value {
    ok(e, "run.start", params);
    for _ in 0..200_000 {
        let s = e.poll_run(1, SLICE_EVENTS);
        if s["state"] != "running" {
            return s;
        }
    }
    panic!("the run did not finish");
}

#[test]
fn a_live_run_equals_a_run_on_the_file_it_recorded() {
    let mut e = help_desk(ScriptedFetch::anu(11));
    e.set_random_key("anu", KEY).unwrap();
    let file = scratch("live.bin");
    let status = run_started(
        &mut e,
        json!({ "replications": 6, "random": {
            "mode": "live", "source": "anu", "file": file } }),
    );
    assert_eq!(status["state"], "finished", "{status}");
    let live = status["result"].clone();
    let rec = &live["record"]["random"];
    assert_eq!(rec["mode"], "byte_file");
    assert_eq!(rec["live"]["source"], "anu");
    // Every segment, from a pilot, is recorded, in the order the source sent them.
    let segment = rec["segment_bytes"].as_u64().unwrap() as usize;
    let bytes = std::fs::read(&file).unwrap();
    assert!(bytes.len() >= 6 * segment, "{}", bytes.len());
    assert_eq!(bytes, anu_stream(11, bytes.len()));
    assert_eq!(rec["sha256"], stochcraft_qrng::sha256_hex(&bytes));
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{file}.json")).unwrap()).unwrap();
    assert_eq!(manifest["format"], "stochcraft-random-bytes/1");
    assert_eq!(manifest["sha256"], rec["sha256"]);
    // The same run from the file: identical results, without the source.
    let mut replay = Engine::new();
    ok(&mut replay, "model.set", COMMANDS[0].example());
    let from_file = ok(
        &mut replay,
        "run",
        json!({ "replications": 6, "random": rec }),
    );
    assert_eq!(without_record(from_file), without_record(live));
    // Its journal replays from the file.
    let journal = serde_json::to_string(&e.journal_document()).unwrap();
    assert!(
        journal.contains("byte_file") && !journal.contains("\"mode\":\"live\""),
        "{journal}"
    );
}

#[test]
fn a_sequential_live_run_that_stops_early_replays_from_what_it_recorded() {
    let mut e = help_desk(ScriptedFetch::anu(13));
    e.set_random_key("anu", KEY).unwrap();
    let file = scratch("sequential-live.bin");
    // ± 50 points on the desk's utilisation is met by the first batch of 4, far short of 200.
    let sequential = json!({
        "measure": { "element": "desk", "metric": "utilisation", "half_width": 0.5 },
        "max": 200,
    });
    let status = run_started(
        &mut e,
        json!({ "replications": 4, "sequential": sequential, "random": {
            "mode": "live", "source": "anu", "file": file } }),
    );
    assert_eq!(status["state"], "finished", "{status}");
    let live = status["result"].clone();
    assert_eq!(live["sequential"]["reason"], "target_met", "{live}");
    let rec = live["record"]["random"].clone();
    assert_eq!(rec["replications"], 4, "{rec}");
    let segment = rec["segment_bytes"].as_u64().unwrap();
    let bytes = std::fs::read(&file).unwrap();
    let held = bytes.len() as u64;
    // The file holds the segments fetched, not the 200 the run might have made.
    assert!(held >= 4 * segment && held < 200 * segment, "{held}");
    assert_eq!(rec["sha256"], stochcraft_qrng::sha256_hex(&bytes));
    // The record repeats the run exactly: the same bytes, so the same stop.
    let mut replay = Engine::new();
    ok(&mut replay, "model.set", COMMANDS[0].example());
    let from_file = ok(
        &mut replay,
        "run",
        json!({ "replications": 4, "sequential": sequential, "random": rec }),
    );
    assert_eq!(from_file["record"]["random"]["replications"], 4);
    assert_eq!(without_record(from_file), without_record(live.clone()));
    // So does the journal, which knows no count: its file is checked for the first batch.
    let journal = e.journal_document();
    let mut again = Engine::new();
    again.replay(&journal.entries).unwrap();
    let replayed = ok(&mut again, "run.status", json!({}));
    assert_eq!(replayed["state"], "finished", "{replayed}");
    assert_eq!(
        without_record(replayed["result"].clone()),
        without_record(live)
    );
    // A replay that would go further than the run did stops where the bytes run out.
    let tight = json!({
        "measure": { "element": "desk", "metric": "mean_wait" },
        "relative": 1e-9,
        "max": 200,
    });
    let err = replay
        .execute(
            "run",
            json!({ "replications": 4, "sequential": tight, "random": rec }),
        )
        .unwrap_err();
    assert_eq!(err.code.name(), "random_data_exhausted", "{err}");
}

#[test]
fn a_live_run_whose_source_fails_stops_with_its_reason() {
    let mut e = help_desk(ScriptedFetch::failing(FetchError::Status(403)));
    e.set_random_key("anu", KEY).unwrap();
    let status = run_started(
        &mut e,
        json!({ "replications": 2, "random": {
            "mode": "live", "source": "anu", "file": scratch("failed.bin"), "segment_bytes": 4096 } }),
    );
    assert_eq!(status["state"], "failed");
    assert_eq!(status["error"]["code"], "random_source_failed", "{status}");
    assert!(status["error"]["message"].as_str().unwrap().contains("403"));
}

#[test]
fn the_estimate_counts_requests_against_the_free_tier() {
    let mut e = help_desk(ScriptedFetch::anu(1));
    let seed = ok(
        &mut e,
        "random.estimate",
        json!({ "source": "anu", "mode": "seed" }),
    );
    assert_eq!(
        (
            seed["requests"].as_u64(),
            seed["within_free_tier"].as_bool()
        ),
        (Some(1), Some(true))
    );
    let live = ok(
        &mut e,
        "random.estimate",
        json!({ "source": "anu", "replications": 1000, "segment_bytes": 65_536 }),
    );
    assert_eq!(live["requests"], 3200);
    assert_eq!(live["within_free_tier"], false);
    assert!(live["note"].is_string());
    // From a pilot when no segment is given.
    let piloted = ok(
        &mut e,
        "random.estimate",
        json!({ "source": "anu", "replications": 10 }),
    );
    assert_eq!(piloted["segment_bytes"].as_u64().unwrap() % 4096, 0);
}

#[test]
fn refusals() {
    let mut e = help_desk(ScriptedFetch::anu(1));
    // No key kept for ANU.
    let err = e
        .execute("random.seed.draw", json!({ "source": "anu" }))
        .unwrap_err();
    assert_eq!(err.code.name(), "random_source_failed");
    // A key in the address.
    let err = e
        .execute(
            "random.source.set",
            json!({ "id": "mine", "url": "https://x.example/r?n={n}&apikey=abc", "max_units": 8 }),
        )
        .unwrap_err();
    assert!(err.message.contains("key"), "{}", err.message);
    // The beacon serves seeds only; random.org is off until enabled; LfD waits.
    assert!(
        e.execute(
            "random.fetch",
            json!({ "source": "nist-beacon", "bytes": 64, "file": scratch("b.bin") })
        )
        .is_err()
    );
    assert!(
        e.execute(
            "random.fetch",
            json!({ "source": "random-org", "bytes": 64, "file": scratch("o.bin") })
        )
        .is_err()
    );
    assert!(
        e.execute("random.source.set", json!({ "id": "lfd", "enabled": true }))
            .is_err()
    );
    // A live run is for run.start, and the commands that cannot read a file refuse one.
    let live = json!({ "mode": "live", "source": "anu", "file": scratch("x.bin") });
    assert!(e.execute("run", json!({ "random": live })).is_err());
    assert!(
        e.execute("experiment.sweep", json!({ "random": live }))
            .is_err()
    );
}

#[test]
fn a_new_origin_unbinds_the_key() {
    let mut e = help_desk(ScriptedFetch::anu(1));
    ok(
        &mut e,
        "random.source.set",
        json!({ "id": "mine", "url": "https://a.example/r?n={n}", "key_header": "x-key",
                "format": { "type": "hex" }, "max_units": 64 }),
    );
    e.set_random_key("mine", KEY).unwrap();
    let key_of = |e: &mut Engine| {
        let s = ok(e, "random.sources", json!({}));
        s["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == "mine")
            .unwrap()["key"]
            .clone()
    };
    assert_eq!(key_of(&mut e), "present");
    ok(
        &mut e,
        "random.source.set",
        json!({ "id": "mine", "url": "https://b.example/r?n={n}" }),
    );
    assert_eq!(key_of(&mut e), "absent");
}

#[test]
fn the_key_never_appears_in_outputs_records_journals_or_errors() {
    let mut e = help_desk(ScriptedFetch::anu(3));
    e.set_random_key("anu", KEY).unwrap();
    let mut seen = Vec::new();
    for (c, p) in [
        ("random.sources", json!({})),
        (
            "random.estimate",
            json!({ "source": "anu", "replications": 2, "segment_bytes": 4096 }),
        ),
        ("random.seed.draw", json!({ "source": "anu" })),
        (
            "random.fetch",
            json!({ "source": "anu", "bytes": 5000, "file": scratch("k.bin") }),
        ),
        ("random.status", json!({})),
    ] {
        seen.push(ok(&mut e, c, p).to_string());
    }
    let draw: Value = serde_json::from_str(&seen[2]).unwrap();
    let r = &draw["result"];
    seen.push(
        ok(
            &mut e,
            "run",
            json!({ "replications": 2, "seed": r["seed"], "random": r["random"] }),
        )
        .to_string(),
    );
    let status = run_started(
        &mut e,
        json!({ "replications": 2, "random": { "mode": "live", "source": "anu", "file": scratch("k-live.bin"), "segment_bytes": 4096 } }),
    );
    seen.push(status.to_string());
    seen.push(serde_json::to_string(&e.journal_document()).unwrap());
    seen.push(serde_json::to_string(&e.random_sources_settings()).unwrap());
    seen.push(std::fs::read_to_string(scratch("k.bin.json")).unwrap());
    seen.push(std::fs::read_to_string(scratch("k-live.bin.json")).unwrap());
    // A failing fetch's error.
    let mut failing = help_desk(ScriptedFetch::failing(FetchError::Status(401)));
    failing.set_random_key("anu", KEY).unwrap();
    let err = failing
        .execute("random.seed.draw", json!({ "source": "anu" }))
        .unwrap_err();
    seen.push(format!("{err} {:?}", err.to_json()));
    for text in &seen {
        assert!(!text.contains(KEY), "the key leaked: {text}");
        assert!(!text.contains("SENTINEL"), "the key leaked: {text}");
    }
}
