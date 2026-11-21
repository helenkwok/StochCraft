//! The journal: its version and starting state, and replaying it safely.

use serde_json::{Value, json};
use stochcraft_engine::{
    COMMANDS, CommandKind, Engine, ErrorCode, JOURNAL_FORMAT, Journal, JournalEntry, ReplayMode,
    ReplayOptions, ReplayStep, StartKind, model_digest,
};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("journal-tests");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

fn mm1_bytes() -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/models/mm1.bpmn");
    std::fs::read(path).unwrap_or_default()
}

fn step(command: &str, params: Value) -> JournalEntry {
    JournalEntry {
        command: command.to_owned(),
        params,
    }
}

/// A journal that starts from a fresh engine's model, with the given steps.
fn journal_of(entries: Vec<JournalEntry>) -> Journal {
    let mut journal = Engine::new().journal_document();
    journal.entries = entries;
    journal
}

#[test]
fn the_journal_header_records_the_versions_and_the_starting_model() {
    let fresh = Engine::new();
    let mut e = Engine::new();
    ok(&mut e, "model.rename", json!({"name": "Shop"}));
    let journal = e.journal_document();
    assert_eq!(journal.format, JOURNAL_FORMAT);
    assert_eq!(journal.stochcraft, env!("CARGO_PKG_VERSION"));
    assert_eq!(journal.start.kind, StartKind::Engine);
    // The start is the model as it was before the edit, not as it is now.
    assert_eq!(journal.start.model_digest, model_digest(fresh.model()));
    assert_ne!(journal.start.model_digest, model_digest(e.model()));
    assert_eq!(journal.entries.len(), 1);
    // As written to a file.
    let text = serde_json::to_value(&journal).unwrap_or_default();
    assert_eq!(text["format"], 1);
    assert_eq!(text["start"]["kind"], "engine");
    assert_eq!(text["entries"][0]["command"], "model.rename");
}

#[test]
fn a_new_model_starts_a_new_journal_that_records_it() {
    let mut e = Engine::new();
    ok(&mut e, "model.rename", json!({"name": "Before"}));
    assert_eq!(e.journal().len(), 1);
    ok(&mut e, "model.new", json!({"name": "Shop"}));
    assert!(e.journal().is_empty(), "{:?}", e.journal());
    let start = e.journal_start();
    assert_eq!(start.kind, StartKind::New);
    assert_eq!(start.name.as_deref(), Some("Shop"));
    assert_eq!(start.model_digest, model_digest(e.model()));
    ok(&mut e, "block.add", json!({"kind": "sink", "id": "exit"}));
    assert_eq!(e.journal().len(), 1);
}

#[test]
fn opening_a_file_starts_a_new_journal_that_records_the_file() {
    let bytes = mm1_bytes();
    assert!(!bytes.is_empty());
    let mut e = Engine::new();
    ok(&mut e, "model.rename", json!({"name": "Before"}));
    assert!(e.open_bytes("mm1.bpmn", &bytes, None).is_ok());
    assert!(e.journal().is_empty());
    let start = e.journal_start().clone();
    assert_eq!(start.kind, StartKind::File);
    assert_eq!(start.name.as_deref(), Some("mm1.bpmn"));
    assert_eq!(start.model_digest, model_digest(e.model()));
    let digest = start.file_digest.clone().unwrap_or_default();
    assert!(
        digest.starts_with("sha256:") && digest.len() == 71,
        "{digest}"
    );
    // The same bytes give the same file digest, other bytes another.
    let mut again = Engine::new();
    assert!(again.open_bytes("elsewhere.bpmn", &bytes, None).is_ok());
    assert_eq!(again.journal_start().file_digest, start.file_digest);
    let mut other = bytes.clone();
    other.extend_from_slice(b"\n");
    let mut changed = Engine::new();
    assert!(changed.open_bytes("mm1.bpmn", &other, None).is_ok());
    assert_ne!(changed.journal_start().file_digest, start.file_digest);
}

#[test]
fn the_file_open_command_starts_a_journal_without_being_an_entry() {
    let path = scratch("started.bpmn");
    let opened_as = path.to_string_lossy().into_owned();
    let mut e = Engine::new();
    ok(&mut e, "model.rename", json!({"name": "Saved"}));
    ok(&mut e, "file.save", json!({"path": path.to_string_lossy()}));
    ok(&mut e, "file.open", json!({"path": path.to_string_lossy()}));
    assert!(e.journal().is_empty());
    assert_eq!(e.journal_start().kind, StartKind::File);
    assert_eq!(e.journal_start().name.as_deref(), Some(opened_as.as_str()));
}

#[test]
fn replay_by_default_applies_edits_and_skips_saves_runs_and_experiments() {
    let bad_path = scratch("no-such-folder").join("x.bpmn");
    let journal = journal_of(vec![
        step("model.rename", json!({"name": "Replayed"})),
        // The model is empty, so these would fail if they were carried out.
        step("run", json!({})),
        step("experiment.sweep", json!({})),
        step("run.start", json!({})),
        step("file.save", json!({"path": bad_path.to_string_lossy()})),
        step("experiment.set", json!({"seed": 7})),
    ]);
    let mut e = Engine::new();
    let steps = e.replay_journal(&journal, ReplayOptions::default());
    let steps = steps.unwrap_or_default();
    assert_eq!(steps.len(), 6);
    assert!(matches!(steps[0], ReplayStep::Applied(_)));
    assert!(steps[1..5].iter().all(|s| *s == ReplayStep::Skipped));
    assert!(matches!(steps[5], ReplayStep::Applied(_)));
    assert_eq!(e.model().name, "Replayed");
    assert_eq!(e.model().experiment.seed, 7);
    // Only what happened is in the replaying engine's journal.
    assert_eq!(e.journal().len(), 2);
    assert!(!bad_path.exists());
}

#[test]
fn replay_in_all_mode_carries_out_every_step() {
    let journal = journal_of(vec![
        step("model.rename", json!({"name": "Replayed"})),
        step("run", json!({})),
        step("model.rename", json!({"name": "Unreached"})),
    ]);
    let mut e = Engine::new();
    let options = ReplayOptions {
        mode: ReplayMode::All,
        force: false,
    };
    let r = e.replay_journal(&journal, options);
    // Every step ran: the run gives results, and the edit after it is applied.
    assert!(
        r.as_ref().is_ok_and(|steps| steps.len() == 3
            && steps.iter().all(|s| matches!(s, ReplayStep::Applied(_)))
            && matches!(&steps[1], ReplayStep::Applied(v) if v.get("replications").is_some())),
        "{r:?}"
    );
    assert_eq!(e.model().name, "Unreached");
}

#[test]
fn the_old_replay_of_a_plain_script_still_carries_out_every_step() {
    let script = vec![
        step("model.rename", json!({"name": "Script"})),
        step("run", json!({})),
    ];
    let r = Engine::new().replay(&script);
    // The run is carried out: its result holds the replications.
    assert!(
        r.as_ref()
            .is_ok_and(|out| out.len() == 2 && out[1].get("replications").is_some()),
        "run is not skipped: {r:?}"
    );
}

#[test]
fn skipped_commands_are_catalogue_commands_that_are_not_queries() {
    let skipped = [
        "file.save",
        "run",
        "mc.run",
        "mc.sobol",
        "run.start",
        "run.cancel",
        "experiment.sweep",
        "experiment.sensitivity",
        "experiment.design",
        "experiment.size",
    ];
    for name in skipped {
        let c = COMMANDS.iter().find(|c| c.name == name);
        assert!(
            c.is_some_and(|c| c.kind != CommandKind::Query),
            "{name} is not a journalled command"
        );
        assert!(!ReplayMode::EditsOnly.applies(name));
        assert!(ReplayMode::All.applies(name));
    }
    for name in [
        "model.new",
        "file.open",
        "undo",
        "redo",
        "block.add",
        "batch",
    ] {
        assert!(ReplayMode::EditsOnly.applies(name), "{name}");
    }
}

#[test]
fn replay_refuses_a_journal_that_started_from_another_model() {
    let bytes = mm1_bytes();
    let mut source = Engine::new();
    assert!(source.open_bytes("mm1.bpmn", &bytes, None).is_ok());
    ok(
        &mut source,
        "block.set",
        json!({"id": "serve", "capacity": 2}),
    );
    let journal = source.journal_document();
    assert_eq!(journal.start.kind, StartKind::File);

    // An engine with another model: refused, with a clear reason, and nothing is changed.
    let mut other = Engine::new();
    let r = other.replay_journal(&journal, ReplayOptions::default());
    let refused = r.err();
    assert!(
        refused
            .as_ref()
            .is_some_and(|err| err.error.code == ErrorCode::Conflict
                && err.error.message.contains("mm1.bpmn")
                && err.error.message.contains("starts from")),
        "{refused:?}"
    );
    assert!(other.journal().is_empty());
    assert_eq!(other.model().name, "");

    // The same file opened first: accepted, and the edit is re-applied.
    let mut same = Engine::new();
    assert!(same.open_bytes("copy.bpmn", &bytes, None).is_ok());
    assert!(
        same.replay_journal(&journal, ReplayOptions::default())
            .is_ok()
    );
    assert_eq!(same.model(), source.model());

    // Forced onto another model, the steps are tried and the first that cannot apply fails.
    let mut forced = Engine::new();
    let options = ReplayOptions {
        mode: ReplayMode::EditsOnly,
        force: true,
    };
    let r = forced.replay_journal(&journal, options);
    assert!(
        r.is_err_and(|err| err.index == 0 && err.error.code == ErrorCode::NotFound),
        "forced replay reaches the steps"
    );
}

#[test]
fn replay_refuses_a_journal_in_a_newer_format() {
    let mut journal = journal_of(vec![step("model.rename", json!({"name": "x"}))]);
    journal.format = JOURNAL_FORMAT + 1;
    let mut e = Engine::new();
    let r = e.replay_journal(&journal, ReplayOptions::default());
    assert!(
        r.is_err_and(|err| err.error.code == ErrorCode::Unsupported),
        "a newer format is refused"
    );
    assert_eq!(e.model().name, "");
}

#[test]
fn a_journal_that_began_with_a_new_model_replays_from_nothing() {
    let mut e = Engine::new();
    ok(&mut e, "model.new", json!({"name": "Shop"}));
    ok(
        &mut e,
        "block.add",
        json!({"kind": "source", "id": "in", "interarrival": {"type": "exponential", "mean": 2.0}}),
    );
    ok(&mut e, "block.add", json!({"kind": "sink", "id": "out"}));
    ok(&mut e, "connect", json!({"from": "in", "to": "out"}));
    // Through JSON, as a file.
    let text = serde_json::to_string(&e.journal_document()).unwrap_or_default();
    let parsed: Result<Journal, _> = serde_json::from_str(&text);
    assert!(parsed.is_ok(), "{parsed:?}");
    let Ok(journal) = parsed else { return };

    let mut again = Engine::new();
    ok(
        &mut again,
        "model.rename",
        json!({"name": "Something else"}),
    );
    let r = again.replay_journal(&journal, ReplayOptions::default());
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(again.model(), e.model());
    assert_eq!(again.journal_document(), e.journal_document());
}

#[test]
fn saving_with_the_command_is_journalled_once() {
    let path = scratch("once.bpmn");
    let mut e = Engine::new();
    ok(&mut e, "model.rename", json!({"name": "Once"}));
    ok(&mut e, "file.save", json!({"path": path.to_string_lossy()}));
    let saves = e.journal().iter().filter(|s| s.command == "file.save");
    assert_eq!(saves.count(), 1);
}

#[test]
fn the_apps_open_and_save_are_journalled_as_events() {
    // The app calls `open_bytes` and `saved_to` with what its shell read and wrote.
    let bytes = mm1_bytes();
    let mut e = Engine::new();
    assert!(e.open_bytes("mm1.bpmn", &bytes, None).is_ok());
    ok(&mut e, "block.set", json!({"id": "serve", "capacity": 3}));
    let saved = e.save_bytes(false).unwrap_or_default();
    assert!(e.saved_to("here/mm1.bpmn", &saved).is_ok());
    let journal = e.journal_document();
    assert_eq!(journal.start.kind, StartKind::File);
    let commands: Vec<&str> = journal.entries.iter().map(|s| s.command.as_str()).collect();
    assert_eq!(commands, ["block.set", "file.save"]);
    assert_eq!(journal.entries[1].params, json!({"path": "here/mm1.bpmn"}));
    // Replaying it re-applies the edit and leaves the save out.
    let mut again = Engine::new();
    assert!(again.open_bytes("mm1.bpmn", &bytes, None).is_ok());
    let steps = again.replay_journal(&journal, ReplayOptions::default());
    assert_eq!(steps.map(|s| s.len()).unwrap_or(0), 2);
    assert_eq!(again.journal().len(), 1);
    let elements = |e: &mut Engine| ok(e, "model.get", json!({}))["elements"].clone();
    assert_eq!(elements(&mut again), elements(&mut e));
}
