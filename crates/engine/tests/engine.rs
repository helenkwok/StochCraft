//! The engine through its commands, as the command line, the app and agents use it.

use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, CommandKind, Engine, ErrorCode, JournalEntry};
use stochcraft_random::{Dist, Rng};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

fn err(e: &mut Engine, command: &str, params: Value) -> ErrorCode {
    let r = e.execute(command, params.clone());
    assert!(r.is_err(), "{command} {params} should fail: {r:?}");
    r.err().map_or(ErrorCode::Internal, |e| e.code)
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("engine-tests");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

/// The M/M/1-like help desk most tests start from.
fn help_desk() -> Engine {
    let mut e = Engine::new();
    ok(&mut e, "model.set", COMMANDS[0].example());
    e
}

#[test]
fn catalogue_is_complete_and_its_examples_run_in_order() {
    let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
    for wanted in [
        "model.new",
        "model.rename",
        "block.add",
        "block.remove",
        "block.set",
        "block.rename",
        "connect",
        "disconnect",
        "experiment.set",
        "run",
        "run.cancel",
        "file.open",
        "file.save",
        "undo",
        "redo",
    ] {
        assert!(names.contains(&wanted), "{wanted} missing");
    }
    let mut unique = names.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), names.len(), "duplicate command names");

    let mut e = Engine::new();
    for c in COMMANDS {
        let schema = c.params_schema();
        assert_eq!(schema["type"], "object", "{}: {schema}", c.name);
        assert!(!c.summary.is_empty());
        assert_eq!(c.to_json()["name"], c.name);
        let mut params = c.example();
        if let Some(p) = params.get("path").and_then(Value::as_str) {
            params["path"] = json!(scratch(p).to_string_lossy());
        }
        ok(&mut e, c.name, params);
    }
    // Queries are not journalled; everything else is.
    let journalled = COMMANDS
        .iter()
        .filter(|c| c.kind != CommandKind::Query)
        .count();
    assert_eq!(e.journal().len(), journalled);
}

#[test]
fn schemas_describe_the_parameters() {
    let schema = |name: &str| {
        COMMANDS
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.params_schema())
            .unwrap_or_default()
    };
    let set = schema("block.set");
    assert_eq!(set["required"], json!(["id"]), "{set}");
    assert_eq!(set["additionalProperties"], false);
    assert!(set["properties"]["service"].is_object());
    // The kinds a block can be appear in block.add's schema.
    let add = schema("block.add").to_string();
    for kind in ["source", "server", "delay", "sink"] {
        assert!(add.contains(&format!("\"{kind}\"")), "{kind}: {add}");
    }
}

#[test]
fn every_edit_undoes_exactly_and_redoes_exactly() {
    // Random edits: after each, undo must restore the model before it, and redo the model after.
    let mut rng = Rng::new(2026);
    let kinds = ["source", "server", "delay", "sink"];
    for _ in 0..40 {
        let mut e = help_desk();
        for _ in 0..30 {
            let ids: Vec<String> = e
                .model()
                .elements
                .iter()
                .map(|x| x.id.to_string())
                .collect();
            let flows: Vec<String> = e
                .model()
                .connections
                .iter()
                .map(|x| x.id.to_string())
                .collect();
            let pick = |rng: &mut Rng, v: &[String]| {
                v.get((rng.next_u64() % v.len().max(1) as u64) as usize)
                    .cloned()
                    .unwrap_or_default()
            };
            let names = ["n", "tier", "rush"];
            let (command, params) = match rng.next_u64() % 11 {
                9 => (
                    "property.set",
                    json!({
                        "element": pick(&mut rng, &ids),
                        "name": names[(rng.next_u64() % 3) as usize],
                        "value": {"expression": format!("getProperty('n') + {}", rng.next_u64() % 5)},
                    }),
                ),
                10 => (
                    "property.remove",
                    json!({"element": pick(&mut rng, &ids), "name": names[(rng.next_u64() % 3) as usize]}),
                ),
                0 => (
                    "block.add",
                    json!({"kind": kinds[(rng.next_u64() % 4) as usize], "index": rng.next_u64() % 5}),
                ),
                1 => ("block.remove", json!({"id": pick(&mut rng, &ids)})),
                2 => (
                    "block.rename",
                    json!({"id": pick(&mut rng, &ids), "name": format!("n{}", rng.next_u64() % 100)}),
                ),
                3 => (
                    "connect",
                    json!({"from": pick(&mut rng, &ids), "to": pick(&mut rng, &ids)}),
                ),
                4 => ("disconnect", json!({"id": pick(&mut rng, &flows)})),
                5 => (
                    "experiment.set",
                    json!({"replications": rng.next_u64() % 20, "warmup": (rng.next_u64() % 50) as f64}),
                ),
                6 => (
                    "model.rename",
                    json!({"name": format!("m{}", rng.next_u64() % 9)}),
                ),
                7 => (
                    "block.set",
                    json!({"id": pick(&mut rng, &ids), "capacity": rng.next_u64() % 4}),
                ),
                _ => (
                    "block.set",
                    json!({"id": pick(&mut rng, &ids), "duration": {"type": "constant", "value": 2.5}}),
                ),
            };
            let before = e.model().clone();
            if e.execute(command, params.clone()).is_err() {
                // A refused edit changes nothing.
                assert_eq!(e.model(), &before, "{command} {params}");
                continue;
            }
            let after = e.model().clone();
            ok(&mut e, "undo", json!({}));
            assert_eq!(e.model(), &before, "undo of {command} {params}");
            ok(&mut e, "redo", json!({}));
            assert_eq!(e.model(), &after, "redo of {command} {params}");
        }
        // And all the way back, past the model.set that loaded the help desk.
        while e.execute("undo", json!({})).is_ok() {}
        assert_eq!(e.model(), Engine::new().model());
    }
}

#[test]
fn removing_a_block_and_undoing_restores_its_connections_in_place() {
    let mut e = help_desk();
    let before = e.model().clone();
    ok(&mut e, "block.remove", json!({"id": "desk"}));
    assert!(e.model().connections.is_empty());
    ok(&mut e, "undo", json!({}));
    assert_eq!(e.model(), &before);
}

#[test]
fn a_new_edit_clears_redo_and_history_reports_it() {
    let mut e = help_desk();
    ok(&mut e, "model.rename", json!({"name": "a"}));
    ok(&mut e, "model.rename", json!({"name": "b"}));
    ok(&mut e, "undo", json!({}));
    let h = ok(&mut e, "history", json!({}));
    assert_eq!(h["undo"], json!(["model.set", "model.rename"]));
    assert_eq!(h["redo"], json!(["model.rename"]));
    ok(
        &mut e,
        "block.rename",
        json!({"id": "desk", "name": "Desk"}),
    );
    assert_eq!(err(&mut e, "redo", json!({})), ErrorCode::NothingToRedo);
    let mut fresh = Engine::new();
    assert_eq!(err(&mut fresh, "undo", json!({})), ErrorCode::NothingToUndo);
}

#[test]
fn the_journal_replays_to_the_same_model() {
    let mut e = help_desk();
    ok(&mut e, "block.add", json!({"kind": "delay", "id": "walk"}));
    ok(&mut e, "disconnect", json!({"from": "desk", "to": "exit"}));
    ok(&mut e, "connect", json!({"from": "desk", "to": "walk"}));
    ok(&mut e, "connect", json!({"from": "walk", "to": "exit"}));
    ok(&mut e, "experiment.set", json!({"seed": 7}));
    ok(&mut e, "undo", json!({}));
    ok(
        &mut e,
        "block.set",
        json!({"id": "walk", "duration": {"type": "uniform", "min": 1.0, "max": 3.0}}),
    );
    ok(&mut e, "model.get", json!({})); // a query: not journalled
    // The journal survives a trip through JSON, as a script file would.
    let script: Vec<JournalEntry> =
        serde_json::from_str(&serde_json::to_string(e.journal()).unwrap_or_default())
            .unwrap_or_default();
    assert_eq!(script.len(), 8);
    let mut again = Engine::new();
    let replayed = again.replay(&script);
    assert!(replayed.is_ok(), "{replayed:?}");
    assert_eq!(again.model(), e.model());
    assert_eq!(again.journal(), e.journal());
    // A failing step stops the replay and says which.
    let bad = vec![
        JournalEntry {
            command: "model.rename".into(),
            params: json!({"name": "x"}),
        },
        JournalEntry {
            command: "block.remove".into(),
            params: json!({"id": "nope"}),
        },
    ];
    let r = Engine::new().replay(&bad);
    assert!(
        r.as_ref()
            .is_err_and(|e| e.index == 1 && e.error.code == ErrorCode::NotFound),
        "{r:?}"
    );
}

#[test]
fn bad_commands_are_refused_without_changing_anything() {
    let mut e = help_desk();
    let before = e.model().clone();
    assert_eq!(
        err(&mut e, "block.explode", json!({})),
        ErrorCode::UnknownCommand
    );
    // A misspelt parameter is an error, not silently ignored.
    for params in [
        json!({"kind": "server", "capcity": 2}),
        json!({"kind": "server", "duration": {"type": "constant", "value": 1.0}}),
        json!({"kind": "teleporter"}),
        json!({"kind": "sink", "id": "desk"}),
        json!({"kind": "sink", "id": "9lives"}),
    ] {
        let code = err(&mut e, "block.add", params.clone());
        assert!(
            matches!(code, ErrorCode::InvalidParams | ErrorCode::Conflict),
            "{params}: {code:?}"
        );
    }
    assert_eq!(
        err(&mut e, "block.set", json!({"id": "desk", "srevice": 1})),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(
            &mut e,
            "block.set",
            json!({"id": "arrivals", "capacity": 2})
        ),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(&mut e, "block.set", json!({"id": "desk"})),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(&mut e, "block.set", json!({"id": "ghost", "capacity": 2})),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(&mut e, "connect", json!({"from": "desk", "to": "ghost"})),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(
            &mut e,
            "connect",
            json!({"from": "desk", "to": "exit", "id": "desk"})
        ),
        ErrorCode::Conflict
    );
    assert_eq!(
        err(&mut e, "disconnect", json!({"from": "desk"})),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(&mut e, "experiment.set", json!({})),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(
            &mut e,
            "model.set",
            json!({"model": {"elements": [{"id": "a", "kind": "sink", "colour": 1}]}})
        ),
        ErrorCode::InvalidParams
    );
    assert_eq!(e.model(), &before);
    // Each error carries a stable code and a message.
    let r = e.execute("block.remove", json!({"id": "ghost"}));
    assert_eq!(
        r.err().map(|e| e.to_json()),
        Some(json!({"code": "not_found", "message": "no element with id `ghost`"}))
    );
}

#[test]
fn block_set_changes_only_what_is_given() {
    let mut e = help_desk();
    ok(&mut e, "block.set", json!({"id": "arrivals", "limit": 5}));
    ok(
        &mut e,
        "block.set",
        json!({"id": "desk", "service": {"type": "erlang", "mean": 1.5, "k": 3}}),
    );
    let m = ok(&mut e, "model.get", json!({}));
    assert_eq!(m["elements"][0]["limit"], 5);
    assert_eq!(
        m["elements"][0]["interarrival"],
        json!({"type": "exponential", "mean": 2.0})
    );
    assert_eq!(m["elements"][1]["capacity"], 1);
    assert_eq!(
        m["elements"][1]["service"],
        json!({"type": "erlang", "mean": 1.5, "k": 3})
    );
    // `null` removes the limit.
    ok(
        &mut e,
        "block.set",
        json!({"id": "arrivals", "limit": null}),
    );
    assert_eq!(
        ok(&mut e, "model.get", json!({}))["elements"][0]["limit"],
        Value::Null
    );
}

#[test]
fn ids_are_generated_when_not_given() {
    let mut e = Engine::new();
    assert_eq!(
        ok(&mut e, "block.add", json!({"kind": "server"}))["id"],
        "server-1"
    );
    assert_eq!(
        ok(&mut e, "block.add", json!({"kind": "server"}))["id"],
        "server-2"
    );
    ok(&mut e, "block.add", json!({"kind": "sink", "id": "out"}));
    assert_eq!(
        ok(&mut e, "connect", json!({"from": "server-1", "to": "out"}))["id"],
        "flow-1"
    );
    // The name defaults to the id.
    assert_eq!(e.model().elements[0].name, "server-1");
}

#[test]
fn validate_reports_problems_with_codes() {
    let mut e = Engine::new();
    ok(
        &mut e,
        "block.add",
        json!({"kind": "server", "id": "m", "capacity": 0}),
    );
    let v = ok(&mut e, "model.validate", json!({}));
    let codes: Vec<&str> = v["diagnostics"]
        .as_array()
        .map(|a| a.iter().filter_map(|d| d["code"].as_str()).collect())
        .unwrap_or_default();
    for c in ["zero_capacity", "no_source", "no_output"] {
        assert!(codes.contains(&c), "{c}: {v}");
    }
    // A model with errors cannot run, and says why.
    let r = e.execute("run", json!({}));
    let r = r.err();
    assert_eq!(r.as_ref().map(|e| e.code), Some(ErrorCode::InvalidModel));
    assert!(
        r.and_then(|e| e.details)
            .is_some_and(|d| d.as_array().is_some_and(|a| !a.is_empty()))
    );
}

fn utilisation(results: &Value) -> Value {
    results["elements"][1]["metrics"]["utilisation"].clone()
}

#[test]
fn runs_give_the_same_results_at_once_on_threads_and_in_slices() {
    let mut e = help_desk();
    let now = ok(&mut e, "run", json!({"replications": 8, "threads": 1}));
    assert_eq!(now["replications"], 8);
    assert_eq!(now["elements"][1]["id"], "desk");
    assert_eq!(utilisation(&now)["n"], 8);
    let threaded = ok(&mut e, "run", json!({"replications": 8, "threads": 3}));
    assert_eq!(now, threaded);

    ok(
        &mut e,
        "run.start",
        json!({"replications": 8, "mode": "slices"}),
    );
    let mut status = json!({});
    let mut polls = 0;
    while status["state"] != "finished" {
        status = ok(&mut e, "run.status", json!({"step": 3}));
        polls += 1;
        assert!(polls < 10, "{status}");
    }
    assert_eq!(polls, 3, "8 replications, 3 at a time");
    assert_eq!(status["result"], now);

    ok(
        &mut e,
        "run.start",
        json!({"replications": 8, "mode": "threads", "threads": 2}),
    );
    let mut status = json!({});
    for _ in 0..10_000 {
        status = ok(&mut e, "run.status", json!({}));
        if status["state"] != "running" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(status["state"], "finished", "{status}");
    assert_eq!(status["result"], now);
}

#[test]
fn background_runs_report_progress_and_can_be_cancelled() {
    let mut e = help_desk();
    assert_eq!(ok(&mut e, "run.status", json!({}))["state"], "idle");
    let started = ok(
        &mut e,
        "run.start",
        json!({"replications": 5, "mode": "slices"}),
    );
    assert_eq!(
        (
            started["state"].clone(),
            started["done"].clone(),
            started["total"].clone()
        ),
        (json!("running"), json!(0), json!(5))
    );
    assert_eq!(ok(&mut e, "run.status", json!({"step": 2}))["done"], 2);
    // One run at a time.
    assert_eq!(err(&mut e, "run.start", json!({})), ErrorCode::Conflict);
    let cancelled = ok(&mut e, "run.cancel", json!({}));
    assert_eq!(cancelled["state"], "cancelled");
    assert_eq!(cancelled["error"]["code"], "cancelled");

    // Threads: a long run, cancelled almost at once, stops well short of the end.
    ok(&mut e, "experiment.set", json!({"duration": 20_000.0}));
    ok(
        &mut e,
        "run.start",
        json!({"replications": 100_000, "mode": "threads", "threads": 2}),
    );
    let cancelled = ok(&mut e, "run.cancel", json!({}));
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    // A new run can start after a cancelled one, and edits made meanwhile do not affect it.
    ok(
        &mut e,
        "run.start",
        json!({"replications": 2, "mode": "slices"}),
    );
    ok(&mut e, "experiment.set", json!({"duration": 1.0}));
    assert_eq!(
        ok(&mut e, "run.status", json!({"step": 2}))["result"]["duration"],
        20_000.0
    );
}

/// Runs `f` on a thread of its own and returns what it gives, or `None` after `secs` seconds, so
/// that a run that never ends fails its test instead of hanging the suite.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(std::time::Duration::from_secs(secs)).ok()
}

/// One entity, arriving at 1, is worked in no time and always sent back by the gateway after
/// it: it goes round for ever while simulated time stays at 1 (#49).
fn zero_time_loop() -> Engine {
    let mut m = stochcraft_blocks::Builder::new();
    let arrivals = m.source("arrivals", Dist::Constant(1.0), Some(1));
    let work = m.server("work", 1, Dist::Constant(0.0));
    let check = m.exclusive("check");
    let done = m.sink("done");
    m.connect(arrivals, work);
    m.connect(work, check);
    m.route(check, work, Some(1.0), None);
    m.route(check, done, Some(0.0), None);
    Engine::with_model(m.build(stochcraft_model::Experiment::default()))
}

/// The error a run of the zero-time loop must stop with: `time_stopped`, at time 1, naming an
/// element of the loop.
fn assert_time_stopped(error: &Value) {
    assert_eq!(error["code"], "time_stopped", "{error}");
    assert_eq!(error["details"]["time"], 1.0, "{error}");
    let at = error["details"]["at"].as_str();
    assert!(matches!(at, Some("work" | "check")), "{error}");
    let message = error["message"].as_str().unwrap_or_default();
    assert!(message.contains("stopped moving at 1"), "{error}");
}

#[test]
fn a_run_where_time_stops_moving_ends_with_an_error_at_once() {
    let r = within(300, || {
        let mut e = zero_time_loop();
        e.execute("run", json!({"replications": 4, "threads": 2}))
    });
    assert!(r.is_some(), "the run did not end");
    let error = r.and_then(Result::err).map(|e| e.to_json());
    assert_time_stopped(&error.unwrap_or_default());
}

#[test]
fn a_run_where_time_stops_moving_ends_with_an_error_in_slices() {
    let status = within(300, || {
        let mut e = zero_time_loop();
        ok(
            &mut e,
            "run.start",
            json!({"replications": 3, "mode": "slices"}),
        );
        ok(&mut e, "run.status", json!({"step": 1}))
    });
    assert!(status.is_some(), "the run did not end");
    let status = status.unwrap_or_default();
    assert_eq!(status["state"], "failed", "{status}");
    assert_time_stopped(&status["error"]);
}

/// Cancelling reaches into a replication that is running: one that would take far too long
/// stops soon after.
#[test]
fn cancel_stops_a_replication_while_it_runs() {
    let status = within(60, || {
        let mut e = help_desk();
        ok(&mut e, "experiment.set", json!({"duration": 1e11}));
        ok(
            &mut e,
            "run.start",
            json!({"replications": 2, "mode": "threads", "threads": 2}),
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
        ok(&mut e, "run.cancel", json!({}))
    });
    assert!(status.is_some(), "cancel did not return");
    let status = status.unwrap_or_default();
    assert_eq!(status["state"], "cancelled", "{status}");
}

#[test]
fn saving_replaces_the_file_instead_of_overwriting_it() {
    // A second name (hard link) for the same file shows whether saving wrote into the file in
    // place (it would see the new content) or wrote a new file and renamed it over the old name,
    // so a crash or a full disk can never leave the user's file half written.
    let dir = scratch("atomic");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("model.bpmn");
    let other_name = dir.join("other-name.bpmn");
    let mut e = help_desk();
    ok(&mut e, "file.save", json!({"path": path.to_string_lossy()}));
    let first = std::fs::read(&path).unwrap_or_default();
    if std::fs::hard_link(&path, &other_name).is_err() {
        // A file system without hard links cannot tell; nothing to check here.
        return;
    }
    ok(&mut e, "model.rename", json!({"name": "Renamed"}));
    ok(&mut e, "file.save", json!({}));
    let second = std::fs::read(&path).unwrap_or_default();
    assert!(second != first, "the save changed nothing");
    assert!(
        std::fs::read(&other_name).unwrap_or_default() == first,
        "the file was written in place"
    );
    // Nothing is left beside the file.
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|d| d.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["model.bpmn", "other-name.bpmn"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_open_save_and_track_changes() {
    let path = scratch("files.bpmn");
    let path_s = path.to_string_lossy().to_string();
    let mut e = help_desk();
    assert!(e.is_dirty());
    assert_eq!(
        err(&mut e, "file.save", json!({})),
        ErrorCode::InvalidParams
    );
    let saved = ok(&mut e, "file.save", json!({"path": path_s}));
    assert_eq!(saved["format"], "bpmn");
    assert!(!e.is_dirty());
    let written = std::fs::read(&path).unwrap_or_default();

    let mut f = Engine::new();
    ok(&mut f, "model.rename", json!({"name": "to be replaced"}));
    let opened = ok(&mut f, "file.open", json!({"path": path_s}));
    assert_eq!(opened["format"], "bpmn");
    assert_eq!(opened["diagnostics"], json!([]));
    assert_eq!(f.model(), e.model(), "what was saved is what opens");
    // Opening a file starts a new history.
    assert_eq!(err(&mut f, "undo", json!({})), ErrorCode::NothingToUndo);
    assert_eq!(f.path(), Some(path_s.as_str()));
    // Saved unchanged: the same bytes.
    ok(&mut f, "file.save", json!({}));
    assert_eq!(std::fs::read(&path).unwrap_or_default(), written);
    // Edited: the changes are written into the opened file.
    ok(&mut f, "block.set", json!({"id": "desk", "capacity": 3}));
    ok(&mut f, "block.add", json!({"kind": "delay", "id": "walk"}));
    ok(&mut f, "disconnect", json!({"from": "desk", "to": "exit"}));
    ok(&mut f, "connect", json!({"from": "desk", "to": "walk"}));
    ok(&mut f, "connect", json!({"from": "walk", "to": "exit"}));
    assert!(f.is_dirty());
    ok(&mut f, "file.save", json!({}));
    assert!(!f.is_dirty());
    let mut g = Engine::new();
    ok(&mut g, "file.open", json!({"path": path_s}));
    assert_eq!(g.model(), f.model());
    // An edit that cannot go into the file is refused with the reason; `as_new` still saves.
    let mm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/mm1.bpmn");
    let copy = scratch("mm1-copy.bpmn");
    let _ = std::fs::copy(&mm1, &copy);
    // A new time unit saves into the file: values with their own unit are rescaled.
    let mut h = Engine::new();
    ok(&mut h, "file.open", json!({"path": copy.to_string_lossy()}));
    ok(&mut h, "experiment.set", json!({"time_unit": "hour"}));
    ok(&mut h, "file.save", json!({}));
    let mut g2 = Engine::new();
    ok(
        &mut g2,
        "file.open",
        json!({"path": copy.to_string_lossy()}),
    );
    assert_eq!(g2.model(), h.model());
    // But not in a scenario that takes those values from another one.
    let _ = std::fs::copy(&mm1, &copy);
    let mut h = Engine::new();
    ok(
        &mut h,
        "file.open",
        json!({"path": copy.to_string_lossy(), "scenario": "two-clerks"}),
    );
    ok(&mut h, "experiment.set", json!({"time_unit": "hour"}));
    let refused = h.execute("file.save", json!({}));
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.code == ErrorCode::Unsupported && e.message.contains("as_new")),
        "{refused:?}"
    );
    ok(&mut h, "file.save", json!({"as_new": true}));
    // Undoing back to the saved model is clean again.
    ok(&mut f, "undo", json!({}));
    assert!(f.is_dirty());
    ok(&mut f, "redo", json!({}));
    assert!(!f.is_dirty());

    assert_eq!(
        err(
            &mut g,
            "file.open",
            json!({"path": scratch("missing.bpmn").to_string_lossy()})
        ),
        ErrorCode::Io
    );
    let junk = scratch("junk.bpmn");
    let _ = std::fs::write(&junk, b"not xml");
    assert_eq!(
        err(&mut g, "file.open", json!({"path": junk.to_string_lossy()})),
        ErrorCode::BadFile
    );
}

#[test]
fn saving_again_writes_into_the_opened_scenario() {
    // mm1.bpmn has "base" and "two-clerks", which inherits from it.
    let mm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/mm1.bpmn");
    let copy = scratch("mm1-two-saves.bpmn");
    let _ = std::fs::copy(&mm1, &copy);
    let path = json!(copy.to_string_lossy());
    let reps = |scenario: &str| {
        let mut r = Engine::new();
        ok(
            &mut r,
            "file.open",
            json!({"path": path, "scenario": scenario}),
        );
        r.model().experiment.replications
    };
    let base = reps("base");

    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({"path": path, "scenario": "two-clerks"}),
    );
    ok(&mut e, "experiment.set", json!({"replications": base + 5}));
    ok(&mut e, "file.save", json!({}));
    ok(&mut e, "experiment.set", json!({"replications": base + 7}));
    ok(&mut e, "file.save", json!({}));
    assert_eq!(reps("two-clerks"), base + 7);
    assert_eq!(reps("base"), base, "the other scenario is untouched");
    // Saved again unchanged: the same bytes.
    let written = std::fs::read(&copy).unwrap_or_default();
    ok(&mut e, "file.save", json!({}));
    assert_eq!(std::fs::read(&copy).unwrap_or_default(), written);
}

#[test]
fn new_ids_avoid_the_ids_an_opened_file_uses() {
    // mm1.bpmn with an element StochCraft keeps but does not model, under the id the engine
    // would give the next delay.
    let mm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/mm1.bpmn");
    let text = std::fs::read_to_string(&mm1).unwrap_or_default().replace(
        r#"<semantic:resource id="clerk" name="Clerk"/>"#,
        r#"<semantic:resource id="clerk" name="Clerk"/>
  <semantic:dataStore id="delay-1" name="Records"/>"#,
    );
    let copy = scratch("mm1-ids.bpmn");
    let _ = std::fs::write(&copy, text);
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({"path": copy.to_string_lossy()}));
    let delay = json!({"kind": "delay", "duration": {"type": "constant", "value": 1.0}});
    // Ids given that the file already uses (a resource of one task, a scenario, the process)
    // are refused.
    for taken in ["clerk", "two-clerks", "p1", "delay-1"] {
        let mut p = delay.clone();
        p["id"] = json!(taken);
        assert_eq!(err(&mut e, "block.add", p), ErrorCode::Conflict, "{taken}");
    }
    assert_eq!(
        err(
            &mut e,
            "resource.add",
            json!({"id": "clerk", "quantity": 1})
        ),
        ErrorCode::Conflict
    );
    // A generated id skips them, and the model saves.
    let added = ok(&mut e, "block.add", delay);
    assert_eq!(added["id"], json!("delay-2"), "{added}");
    ok(&mut e, "file.save", json!({}));
}

#[test]
fn the_committed_test_models_open_and_run() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models");
    for name in ["mm1.bpmn", "tandem.xpdl"] {
        let mut e = Engine::new();
        let opened = ok(
            &mut e,
            "file.open",
            json!({"path": root.join(name).to_string_lossy()}),
        );
        assert!(
            opened["scenarios"]
                .as_array()
                .is_some_and(|s| !s.is_empty()),
            "{opened}"
        );
        let r = ok(&mut e, "run", json!({"replications": 3}));
        assert_eq!(r["replications"], 3, "{name}");
    }
}

#[test]
fn results_carry_a_run_record() {
    let mut e = help_desk();
    let r = ok(&mut e, "run", json!({"replications": 3, "seed": 9}));
    let rec = &r["record"];
    assert_eq!(rec["seed"], 9);
    assert_eq!(rec["replications"], 3);
    assert_eq!(rec["warmup"], 60.0);
    assert_eq!(rec["file"], Value::Null);
    assert!(
        rec["uncertainty"]
            .as_str()
            .is_some_and(|u| u.contains("95%"))
    );
    let digest = rec["model_digest"].as_str().unwrap_or_default().to_owned();
    assert!(
        digest.starts_with("sha256:") && digest.len() == 7 + 64,
        "{digest}"
    );
    // The digest follows the model, not the run.
    let again = ok(&mut e, "run", json!({"replications": 2}));
    assert_eq!(again["record"]["model_digest"], json!(digest));
    ok(&mut e, "block.set", json!({"id": "desk", "capacity": 2}));
    let changed = ok(&mut e, "run", json!({"replications": 2}));
    assert_ne!(changed["record"]["model_digest"], json!(digest));
    // From a file: its name, scenario and what was noted when reading it.
    let mm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/mm1.bpmn");
    let mut f = Engine::new();
    ok(&mut f, "file.open", json!({"path": mm1.to_string_lossy()}));
    let r = ok(&mut f, "run", json!({"replications": 2}));
    assert_eq!(r["record"]["scenario"], "base");
    assert_eq!(r["record"]["format"], "bpmn");
    assert_eq!(r["record"]["unsaved_changes"], false);
    assert!(
        r["record"]["file_notes"]
            .as_array()
            .is_some_and(|n| !n.is_empty())
    );
}

#[test]
fn properties_are_set_listed_run_saved_and_undone() {
    let mut e = help_desk();
    let plain = e.model().clone();
    ok(
        &mut e,
        "property.set",
        json!({"element": "arrivals", "name": "issues", "type": "long", "value": {"number": 3}}),
    );
    ok(
        &mut e,
        "property.set",
        json!({"element": "desk", "name": "issues", "value": {"expression": "getProperty('issues') - 1"}}),
    );
    ok(
        &mut e,
        "property.set",
        json!({"element": "exit", "name": "done", "value": {"expression": "getProperty('issues') = 2"}}),
    );
    // Replacing a value keeps the declared type unless a new one (or null) is given.
    ok(
        &mut e,
        "property.set",
        json!({"element": "arrivals", "name": "issues", "value": {"number": 3}}),
    );
    let m = ok(&mut e, "model.get", json!({}));
    assert_eq!(
        m["elements"][0]["properties"],
        json!([{"name": "issues", "type": "long", "value": {"number": 3.0}}])
    );
    assert_eq!(
        ok(&mut e, "property.list", json!({})),
        json!({"properties": [
            {"name": "done", "type": "boolean", "set_by": ["exit"]},
            {"name": "issues", "type": "long", "set_by": ["arrivals", "desk"]},
        ]})
    );

    // Results carry the mean value each element assigned.
    let r = ok(&mut e, "run", json!({"replications": 2}));
    let element = |id: &str| {
        r["elements"]
            .as_array()
            .and_then(|v| v.iter().find(|x| x["id"] == id))
            .cloned()
            .unwrap_or_default()
    };
    assert_eq!(element("arrivals")["properties"]["issues"]["mean"], 3.0);
    assert_eq!(element("desk")["properties"]["issues"]["mean"], 2.0);
    assert_eq!(element("exit")["properties"]["done"]["mean"], 1.0);

    // Saved and opened again, the model is the same.
    let path = scratch("properties.bpmn");
    let path = path.to_string_lossy();
    ok(&mut e, "file.save", json!({"path": path}));
    let saved = e.model().clone();
    let mut again = Engine::new();
    ok(&mut again, "file.open", json!({"path": path}));
    assert_eq!(again.model(), &saved);

    // A bad expression stops the run with its position.
    ok(
        &mut e,
        "property.set",
        json!({"element": "desk", "name": "issues", "value": {"expression": "getProperty('issues') -"}}),
    );
    let failed = e.execute("run", json!({}));
    let details = failed.err().and_then(|f| f.details).unwrap_or_default();
    assert_eq!(details[0]["code"], "bad_expression", "{details}");
    assert_eq!(
        err(
            &mut e,
            "property.remove",
            json!({"element": "desk", "name": "nothing"})
        ),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(
            &mut e,
            "property.set",
            json!({"element": "desk", "name": "x", "value": {"number": 1}, "colour": "red"})
        ),
        ErrorCode::InvalidParams
    );

    // Every change undoes exactly.
    ok(
        &mut e,
        "property.remove",
        json!({"element": "exit", "name": "done"}),
    );
    for _ in 0..6 {
        ok(&mut e, "undo", json!({}));
    }
    // Saving gave the shapes their places in the file; that is not an edit to undo.
    let mut plain = plain.clone();
    plain.layout = e.model().layout.clone();
    assert_eq!(e.model(), &plain);
    assert!(
        ok(&mut e, "model.get", json!({}))["elements"][0]
            .get("properties")
            .is_none(),
        "no properties, no key: models without properties read as before"
    );
}

#[test]
fn gateways_route_by_probability_and_condition_and_undo() {
    let mut e = help_desk();
    let before = ok(&mut e, "model.get", json!({}));
    // A gateway after the desk: most leave, some go back for a second look.
    ok(
        &mut e,
        "disconnect",
        json!({ "from": "desk", "to": "exit" }),
    );
    let g = ok(
        &mut e,
        "block.add",
        json!({ "kind": "exclusive_gateway", "id": "check" }),
    );
    assert_eq!(g["id"], "check");
    ok(&mut e, "connect", json!({ "from": "desk", "to": "check" }));
    let back = ok(
        &mut e,
        "connect",
        json!({ "from": "check", "to": "desk", "id": "again", "probability": 0.2 }),
    );
    assert_eq!(back["id"], "again");
    ok(
        &mut e,
        "connect",
        json!({ "from": "check", "to": "exit", "id": "done" }),
    );
    let diagnostics = ok(&mut e, "model.validate", json!({}));
    assert_eq!(diagnostics["diagnostics"], json!([]), "{diagnostics}");

    // flow.set: one parameter at a time, `null` clears, and each is one undo step.
    ok(
        &mut e,
        "flow.set",
        json!({ "id": "done", "probability": 0.8 }),
    );
    ok(
        &mut e,
        "flow.set",
        json!({ "id": "again", "condition": "getProperty('x') > 1", "probability": null }),
    );
    let m = ok(&mut e, "model.get", json!({}));
    let flow = |id: &str| {
        m["connections"]
            .as_array()
            .and_then(|cs| cs.iter().find(|c| c["id"] == id).cloned())
            .unwrap_or_default()
    };
    assert_eq!(
        flow("again"),
        json!({ "id": "again", "from": "check", "to": "desk", "condition": "getProperty('x') > 1" })
    );
    assert_eq!(flow("done")["probability"], 0.8);
    let codes: Vec<Value> = ok(&mut e, "model.validate", json!({}))["diagnostics"]
        .as_array()
        .map(|d| d.iter().map(|x| x["code"].clone()).collect())
        .unwrap_or_default();
    // Routing by condition now: the probability is ignored, the property is unknown, and there
    // is no default.
    for want in ["unknown_property", "routing_ignored", "no_default"] {
        assert!(codes.contains(&json!(want)), "{want} in {codes:?}");
    }
    ok(
        &mut e,
        "block.set",
        json!({ "id": "check", "default": "done" }),
    );
    assert_eq!(
        ok(&mut e, "model.get", json!({}))["elements"][3],
        json!({ "id": "check", "name": "check", "kind": "exclusive_gateway", "default": "done" })
    );
    assert_eq!(
        err(
            &mut e,
            "block.set",
            json!({ "id": "desk", "default": "done" })
        ),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(
            &mut e,
            "flow.set",
            json!({ "id": "nope", "probability": 0.5 })
        ),
        ErrorCode::NotFound
    );
    assert_eq!(
        err(&mut e, "flow.set", json!({ "id": "done" })),
        ErrorCode::InvalidParams
    );

    let label = |e: &mut Engine| ok(e, "undo", json!({}))["undone"].clone();
    assert_eq!(label(&mut e), "block.set");
    assert_eq!(label(&mut e), "flow.set");
    // Undoing restored the probability and removed the condition.
    let again = ok(&mut e, "model.get", json!({}))["connections"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["id"] == "again").cloned())
        .unwrap_or_default();
    assert_eq!(
        again,
        json!({ "id": "again", "from": "check", "to": "desk", "probability": 0.2 })
    );
    for _ in 0..6 {
        ok(&mut e, "undo", json!({}));
    }
    assert_eq!(ok(&mut e, "model.get", json!({})), before);

    // A parallel gateway runs: every entity is copied to both sinks.
    let mut e = help_desk();
    ok(
        &mut e,
        "disconnect",
        json!({ "from": "desk", "to": "exit" }),
    );
    ok(
        &mut e,
        "block.add",
        json!({ "kind": "parallel_gateway", "id": "copy" }),
    );
    ok(
        &mut e,
        "block.add",
        json!({ "kind": "sink", "id": "archive" }),
    );
    ok(&mut e, "connect", json!({ "from": "desk", "to": "copy" }));
    ok(&mut e, "connect", json!({ "from": "copy", "to": "exit" }));
    ok(
        &mut e,
        "connect",
        json!({ "from": "copy", "to": "archive" }),
    );
    let r = ok(&mut e, "run", json!({ "replications": 2 }));
    let count = |id: &str| {
        r["elements"]
            .as_array()
            .and_then(|es| es.iter().find(|x| x["id"] == id))
            .map(|x| x["metrics"]["entered"]["mean"].clone())
            .unwrap_or_default()
    };
    assert_eq!(count("exit"), count("archive"));
    assert_eq!(
        r["elements"]
            .as_array()
            .and_then(|es| es.iter().find(|x| x["id"] == "copy"))
            .map(|x| x["kind"].clone()),
        Some(json!("parallel_gateway"))
    );
}

#[test]
fn finite_buffers_are_set_undone_and_reported() {
    let mut e = help_desk();
    let before = ok(&mut e, "model.get", json!({}));
    // A model without buffers has no `queue_capacity` key: model JSON is as before.
    assert!(
        before["elements"][1].get("queue_capacity").is_none(),
        "{before}"
    );
    let plain = ok(&mut e, "run", json!({"replications": 2, "threads": 1}));
    assert!(
        plain["elements"][1]["metrics"]
            .get("loss_probability")
            .is_none()
    );

    ok(
        &mut e,
        "block.set",
        json!({"id": "desk", "queue_capacity": 0}),
    );
    let m = ok(&mut e, "model.get", json!({}));
    assert_eq!(m["elements"][1]["queue_capacity"], 0);
    assert_eq!(m["elements"][1]["capacity"], 1);
    let r = ok(&mut e, "run", json!({"replications": 2, "threads": 1}));
    let metrics = &r["elements"][1]["metrics"];
    // With no waiting room the desk is an Erlang loss system: B(1, 0.75) = 0.75 / 1.75.
    let loss = metrics["loss_probability"]["mean"].as_f64().unwrap_or(-1.0);
    assert!((loss - 0.75 / 1.75).abs() < 0.03, "{metrics}");
    assert!(
        metrics["lost"]["mean"].as_f64().unwrap_or(0.0) > 0.0,
        "{metrics}"
    );
    assert!(
        r["metrics"]
            .as_array()
            .is_some_and(|m| m.contains(&json!("blocking_probability")))
    );

    // Undo restores the model exactly; redo brings the buffer back; null removes it.
    ok(&mut e, "undo", json!({}));
    assert_eq!(ok(&mut e, "model.get", json!({})), before);
    ok(&mut e, "redo", json!({}));
    assert_eq!(ok(&mut e, "model.get", json!({})), m);
    ok(
        &mut e,
        "block.set",
        json!({"id": "desk", "queue_capacity": null}),
    );
    assert_eq!(ok(&mut e, "model.get", json!({})), before);

    // Only servers have a buffer; a negative capacity is refused.
    assert_eq!(
        err(
            &mut e,
            "block.set",
            json!({"id": "exit", "queue_capacity": 2})
        ),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(
            &mut e,
            "block.set",
            json!({"id": "desk", "queue_capacity": -1})
        ),
        ErrorCode::InvalidParams
    );
    // block.add and model.set take it with the other server parameters.
    ok(
        &mut e,
        "block.add",
        json!({"kind": "server", "id": "second", "queue_capacity": 3}),
    );
    let m = ok(&mut e, "model.get", json!({}));
    let second = m["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|el| el["id"] == "second"))
        .cloned()
        .unwrap_or_default();
    assert_eq!(second["queue_capacity"], 3, "{m}");
    assert_eq!(
        err(
            &mut e,
            "block.add",
            json!({"kind": "delay", "id": "walk", "queue_capacity": 3}),
        ),
        ErrorCode::InvalidParams
    );
    ok(&mut e, "model.set", json!({ "model": m }));
    assert_eq!(ok(&mut e, "model.get", json!({})), m);
}

/// Two desks share a pool of agents: added through commands, run, reported as a pool, and
/// guarded against losing a pool still in use; each edit undoes exactly.
#[test]
fn shared_pools_are_added_used_reported_and_guarded() {
    let mut e = help_desk();
    let before = ok(&mut e, "model.get", json!({}));
    let added = ok(
        &mut e,
        "resource.add",
        json!({"id": "agents", "quantity": 2, "roles": ["agent"]}),
    );
    assert_eq!(added["id"], "agents");
    ok(&mut e, "block.remove", json!({"id": "desk"}));
    ok(
        &mut e,
        "block.add",
        json!({"kind": "shared_task", "id": "phone", "service": {"type": "exponential", "mean": 1.5},
               "needs": [{"resource": "agents"}], "priority": 1.0}),
    );
    ok(
        &mut e,
        "block.add",
        json!({"kind": "shared_task", "id": "chat",
               "needs": [{"roles": ["agent"], "quantity": 1}]}),
    );
    ok(
        &mut e,
        "connect",
        json!({"from": "arrivals", "to": "phone"}),
    );
    ok(&mut e, "connect", json!({"from": "phone", "to": "exit"}));
    ok(&mut e, "block.set", json!({"id": "chat", "priority": 2.5}));
    assert_eq!(
        err(&mut e, "block.set", json!({"id": "chat", "capacity": 2})),
        ErrorCode::InvalidParams
    );
    // `chat` has no input or output yet: an error, so connect it to run.
    let src = ok(
        &mut e,
        "block.add",
        json!({"kind": "source", "id": "chats", "interarrival": {"type": "exponential", "mean": 4.0}}),
    );
    ok(&mut e, "connect", json!({"from": src["id"], "to": "chat"}));
    ok(&mut e, "connect", json!({"from": "chat", "to": "exit"}));

    let m = ok(&mut e, "model.get", json!({}));
    assert_eq!(
        m["resources"],
        json!([{"id": "agents", "name": "agents", "quantity": 2, "roles": ["agent"]}])
    );
    let chat = m["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|x| x["id"] == "chat"))
        .cloned();
    assert_eq!(chat.map(|c| c["priority"].clone()), Some(json!(2.5)));

    let r = ok(&mut e, "run", json!({"replications": 3}));
    let agents = r["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|x| x["id"] == "agents"))
        .cloned()
        .unwrap_or_default();
    assert_eq!(agents["kind"], "resource", "{r}");
    let u = agents["metrics"]["utilisation"]["mean"]
        .as_f64()
        .unwrap_or(-1.0);
    assert!(u > 0.0 && u < 1.0, "{agents}");

    // A pool a task names cannot be removed; once nothing names it, it can.
    assert_eq!(
        err(&mut e, "resource.remove", json!({"id": "agents"})),
        ErrorCode::Conflict
    );
    ok(
        &mut e,
        "resource.set",
        json!({"id": "agents", "quantity": 5}),
    );
    let m = ok(&mut e, "model.get", json!({}));
    assert_eq!(m["resources"][0]["quantity"], 5);
    ok(&mut e, "undo", json!({}));
    let m = ok(&mut e, "model.get", json!({}));
    assert_eq!(m["resources"][0]["quantity"], 2);
    // Undoing the other ten edits of this test gives back the model it started from.
    for _ in 0..10 {
        ok(&mut e, "undo", json!({}));
    }
    assert_eq!(ok(&mut e, "model.get", json!({})), before);
}

/// Shapes move in one undoable step; a block added to a laid-out model goes to the right of the
/// one before it, or where it is asked to go.
#[test]
fn shapes_move_and_new_blocks_are_placed() {
    let mut e = help_desk();
    assert!(e.model().layout.is_empty());
    // Placing every shape where it is drawn now, then moving one.
    let placed = e.model().placed();
    let shapes: Vec<Value> = placed
        .keys()
        .map(|id| json!({ "id": id.as_str() }))
        .collect();
    ok(&mut e, "layout.set", json!({ "shapes": shapes }));
    assert_eq!(e.model().layout, placed);
    ok(
        &mut e,
        "layout.set",
        json!({ "shapes": [{ "id": "desk", "x": 300, "y": 20 }] }),
    );
    let desk = ok(&mut e, "model.get", json!({}))["elements"][1]["bounds"].clone();
    assert_eq!(
        desk,
        json!({ "x": 300.0, "y": 20.0, "width": 100.0, "height": 80.0 })
    );
    ok(&mut e, "undo", json!({}));
    assert_eq!(e.model().layout, placed);

    // After the last element (the exit, 36 across), centred on it.
    let added = ok(
        &mut e,
        "block.add",
        json!({ "kind": "delay", "duration": { "type": "constant", "value": 1.0 } }),
    );
    let id = stochcraft_model::ElementId::unchecked(added["id"].as_str().unwrap_or(""));
    let exit = placed
        .get(&stochcraft_model::ElementId::unchecked("exit"))
        .copied();
    let new = e.model().layout.get(&id).copied();
    assert!(
        matches!((exit, new), (Some(x), Some(n)) if n.x == x.x + x.width + 50.0
            && n.y + n.height / 2.0 == x.y + x.height / 2.0),
        "{exit:?} {new:?}"
    );
    ok(&mut e, "undo", json!({}));
    assert!(!e.model().layout.contains_key(&id));
    ok(
        &mut e,
        "block.add",
        json!({ "kind": "sink", "id": "out2", "x": 10, "y": 400 }),
    );
    let out2 = ok(&mut e, "model.get", json!({}))["elements"][3]["bounds"].clone();
    assert_eq!(out2["x"], 10.0);
    assert_eq!(out2["y"], 400.0);

    for bad in [
        json!({ "shapes": [{ "id": "nowhere", "x": 1 }] }),
        json!({ "shapes": [{ "id": "desk", "width": 0 }] }),
    ] {
        assert_ne!(err(&mut e, "layout.set", bad), ErrorCode::Internal);
    }
}

/// A batch is one step: undone and redone whole, and if a step fails nothing changes.
#[test]
fn a_batch_is_one_step_and_all_or_nothing() {
    let mut e = help_desk();
    let before = e.model().clone();
    let r = ok(
        &mut e,
        "batch",
        json!({
            "label": "paste",
            "commands": [
                { "command": "block.add", "params": { "id": "desk2", "kind": "server", "capacity": 2,
                    "service": { "type": "exponential", "mean": 1.0 } } },
                { "command": "connect", "params": { "from": "desk2", "to": "exit" } },
                { "command": "block.rename", "params": { "id": "desk2", "name": "Second desk" } },
            ]
        }),
    );
    assert_eq!(r["results"][0]["id"], "desk2");
    let after = e.model().clone();
    assert_eq!(after.elements.len(), 4);
    let history = ok(&mut e, "history", json!({}));
    assert_eq!(
        history["undo"].as_array().map(Vec::len),
        Some(2),
        "{history}"
    );
    assert_eq!(history["undo"][1], "paste");
    ok(&mut e, "undo", json!({}));
    assert_eq!(e.model(), &before);
    ok(&mut e, "redo", json!({}));
    assert_eq!(e.model(), &after);

    // The third step fails: the first two are taken back and the redo history is kept.
    ok(&mut e, "undo", json!({}));
    let failed = e.execute(
        "batch",
        json!({ "commands": [
            { "command": "block.remove", "params": { "id": "desk" } },
            { "command": "block.rename", "params": { "id": "exit", "name": "Out" } },
            { "command": "block.remove", "params": { "id": "desk" } },
        ]}),
    );
    assert!(
        failed
            .as_ref()
            .is_err_and(|f| f.code == ErrorCode::NotFound && f.message.starts_with("step 3")),
        "{failed:?}"
    );
    assert_eq!(e.model(), &before);
    ok(&mut e, "redo", json!({}));
    assert_eq!(e.model(), &after);

    for bad in [
        json!({ "commands": [{ "command": "run.start", "params": {} }] }),
        json!({ "commands": [{ "command": "no.such" }] }),
        json!({ "commands": [], "label": "anything" }),
    ] {
        err(&mut e, "batch", bad);
    }
    assert_eq!(e.model(), &after);
}
