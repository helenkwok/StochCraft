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
        "sd.run",
        "run.start",
        "run.cancel",
        "experiment.sweep",
        "experiment.sensitivity",
        "experiment.design",
        "experiment.size",
        "experiment.evaluate",
        "experiment.optimise",
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

// ---------------------------------------------------------------------------------------------
// Identity: which document a journal belongs to, and what a replay says it reproduced.

fn xmile(rate: &str) -> String {
    format!(
        "<xmile version=\"1.0\" xmlns=\"http://docs.oasis-open.org/xmile/ns/XMILE/v1.0\"><header><name>Growth</name></header><sim_specs method=\"Euler\"><start>0</start><stop>10</stop><dt>0.25</dt><run/></sim_specs><model><variables><stock name=\"Population\"><eqn>100</eqn><inflow>growth</inflow></stock><flow name=\"growth\"><eqn>{rate} * Population</eqn></flow></variables></model></xmile>"
    )
}

fn open_xmile(text: &str, name: &str) -> Engine {
    let mut e = Engine::new();
    let r = e.open_bytes(name, text.as_bytes(), None);
    assert!(r.is_ok(), "{r:?}");
    e
}

#[test]
fn an_xmile_journal_records_the_system_dynamics_model_not_an_empty_process_model() {
    let mut e = open_xmile(&xmile("0.1"), "a.xmile");
    ok(
        &mut e,
        "sd.variable.set",
        json!({"name": "Population", "doc": "people"}),
    );
    let journal = e.journal_document();
    assert_eq!(journal.start.document.as_deref(), Some("system_dynamics"));
    assert_ne!(
        journal.start.model_digest,
        model_digest(&stochcraft_model::Model::default())
    );
    // The same file replays; a different model of the same kind is refused, as is a fresh engine
    // with no model open, which used to match the placeholder and skip the run silently.
    let mut same = open_xmile(&xmile("0.1"), "a.xmile");
    assert!(
        same.replay_journal(&journal, ReplayOptions::default())
            .is_ok()
    );
    let all = ReplayOptions {
        mode: ReplayMode::All,
        force: false,
    };
    let mut other = open_xmile(&xmile("0.2"), "a.xmile");
    let refused = other.replay_journal(&journal, all);
    assert!(
        refused
            .as_ref()
            .is_err_and(|r| r.error.code == ErrorCode::Conflict),
        "{refused:?}"
    );
    let mut empty = Engine::new();
    assert!(
        empty
            .replay_journal(&journal, ReplayOptions::default())
            .is_err()
    );
    // Forced, it replays onto the other model.
    let forced = other.replay_journal(
        &journal,
        ReplayOptions {
            mode: ReplayMode::EditsOnly,
            force: true,
        },
    );
    assert!(forced.is_ok(), "{forced:?}");
}

#[test]
fn a_journal_of_one_kind_of_document_is_not_replayed_onto_the_other() {
    let mut sd = open_xmile(&xmile("0.1"), "a.xmile");
    ok(
        &mut sd,
        "sd.variable.set",
        json!({"name": "Population", "doc": "x"}),
    );
    let sd_journal = sd.journal_document();
    let mut process = Engine::new();
    let r = process.replay_journal(&sd_journal, ReplayOptions::default());
    assert!(
        r.as_ref()
            .is_err_and(|e| e.error.code == ErrorCode::Conflict
                && e.error.message.contains("system dynamics")),
        "{r:?}"
    );
    let process_journal = Engine::new().journal_document();
    let mut sd2 = open_xmile(&xmile("0.1"), "a.xmile");
    let r = sd2.replay_journal(&process_journal, ReplayOptions::default());
    assert!(
        r.as_ref()
            .is_err_and(|e| e.error.code == ErrorCode::Conflict),
        "{r:?}"
    );
}

#[test]
fn a_legacy_xmile_journal_cannot_vouch_for_its_model_and_needs_force() {
    // As written before the document kind was recorded: the placeholder digest of an empty
    // process model, and no kind.
    let mut e = open_xmile(&xmile("0.1"), "a.xmile");
    let mut journal = e.journal_document();
    journal.start.document = None;
    journal.start.model_digest = model_digest(&stochcraft_model::Model::default());
    journal.engine = None;
    let r = e.replay_journal(&journal, ReplayOptions::default());
    assert!(
        r.as_ref().is_err_and(
            |e| e.error.code == ErrorCode::Conflict && e.error.message.contains("before")
        ),
        "{r:?}"
    );
    let forced = e.replay_journal(
        &journal,
        ReplayOptions {
            mode: ReplayMode::EditsOnly,
            force: true,
        },
    );
    assert!(forced.is_ok(), "{forced:?}");
    // A legacy journal of a new system dynamics model rebuilds it by its own step.
    let mut made = Engine::new();
    ok(&mut made, "sd.new", json!({"stop": 5.0}));
    let mut legacy = made.journal_document();
    legacy.start.document = None;
    legacy.start.model_digest = model_digest(&stochcraft_model::Model::default());
    let mut again = Engine::new();
    let r = again.replay_journal(&legacy, ReplayOptions::default());
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn a_journal_started_from_other_bytes_of_the_same_model_is_refused_without_force() {
    let bytes = mm1_bytes();
    let mut e = Engine::new();
    assert!(e.open_bytes("mm1.bpmn", &bytes, None).is_ok());
    ok(&mut e, "model.rename", json!({"name": "Edited"}));
    let journal = e.journal_document();
    // Trailing text after the root changes the bytes and not the model.
    let mut edited = bytes.clone();
    edited.extend_from_slice(b"\n<!-- edited -->\n");
    let mut other = Engine::new();
    assert!(other.open_bytes("mm1.bpmn", &edited, None).is_ok());
    assert_eq!(model_digest(other.model()), journal.start.model_digest);
    let r = other.replay_journal(&journal, ReplayOptions::default());
    assert!(
        r.as_ref().is_err_and(
            |e| e.error.code == ErrorCode::Conflict && e.error.message.contains("bytes")
        ),
        "{r:?}"
    );
    let forced = other.replay_journal(
        &journal,
        ReplayOptions {
            mode: ReplayMode::EditsOnly,
            force: true,
        },
    );
    assert!(forced.is_ok(), "{forced:?}");
    let mut same = Engine::new();
    assert!(same.open_bytes("mm1.bpmn", &bytes, None).is_ok());
    assert!(
        same.replay_journal(&journal, ReplayOptions::default())
            .is_ok()
    );
}

#[test]
fn a_completed_sliced_run_replayed_in_full_reaches_completion() {
    let mut e = Engine::new();
    assert!(e.open_bytes("mm1.bpmn", &mm1_bytes(), None).is_ok());
    ok(
        &mut e,
        "run.start",
        json!({"mode": "slices", "replications": 2}),
    );
    // The window or an agent polls it to the end; the journal holds only the start.
    for _ in 0..100_000 {
        if e.poll_run(1, 5_000)["state"] != "running" {
            break;
        }
    }
    let journal = e.journal_document();
    assert_eq!(journal.entries[0].command, "run.start");
    let mut again = Engine::new();
    assert!(again.open_bytes("mm1.bpmn", &mm1_bytes(), None).is_ok());
    let all = ReplayOptions {
        mode: ReplayMode::All,
        force: false,
    };
    let steps = again.replay_journal(&journal, all);
    assert!(steps.is_ok(), "{steps:?}");
    let status = ok(&mut again, "run.status", json!({}));
    assert_eq!(status["state"], "finished", "{status}");
}

#[test]
fn the_journal_names_its_build_and_a_replay_says_what_it_reproduced() {
    let mut e = Engine::new();
    assert!(e.open_bytes("mm1.bpmn", &mm1_bytes(), None).is_ok());
    ok(&mut e, "model.rename", json!({"name": "Edited"}));
    ok(&mut e, "run", json!({"replications": 2}));
    let mut journal = e.journal_document();
    assert_eq!(
        journal.engine.as_deref(),
        Some(stochcraft_engine::build_id().as_str())
    );
    let edits = stochcraft_engine::replay_summary(&journal, ReplayMode::EditsOnly);
    assert_eq!(edits["replay"]["edits_replayed"], 1);
    assert_eq!(edits["replay"]["runs_made_again"], 0);
    assert_eq!(edits["replay"]["steps_skipped"], 1);
    assert!(
        edits["replay"]["numerical_results"]
            .as_str()
            .is_some_and(|s| s.starts_with("none"))
    );
    // This build's own journal: the same build only when the build was stamped with its source
    // revision, else unknown.
    let same = stochcraft_engine::replay_summary(&journal, ReplayMode::All);
    assert_eq!(same["replay"]["runs_made_again"], 1);
    let stamped = option_env!("STOCHCRAFT_REVISION").is_some();
    assert_eq!(
        same["replay"]["build_match"],
        if stamped { "same" } else { "unknown" },
        "{same}"
    );
    let text = same["replay"]["numerical_results"].as_str().unwrap_or("");
    assert!(
        text.contains(if stamped {
            "same build"
        } else {
            "build match unknown"
        }),
        "{text}"
    );
    journal.engine = Some("0.0.0 other-build".to_owned());
    let other = stochcraft_engine::replay_summary(&journal, ReplayMode::All);
    assert!(
        other["replay"]["numerical_results"]
            .as_str()
            .is_some_and(|s| s.contains("may differ"))
    );
    assert_eq!(other["replay"]["build_match"], "different");
}

#[test]
fn only_two_builds_with_the_same_source_revision_are_the_same_build() {
    use stochcraft_engine::{BuildMatch, build_match};
    let here = "0.3.0 x86_64-linux 1111111";
    assert_eq!(build_match(Some(here), here), BuildMatch::Same);
    // One version on one platform from two commits: a sampler can differ between them.
    assert_eq!(
        build_match(Some("0.3.0 x86_64-linux 2222222"), here),
        BuildMatch::Different
    );
    assert_eq!(
        build_match(Some("0.2.0 x86_64-linux 1111111"), here),
        BuildMatch::Different
    );
    assert_eq!(
        build_match(Some("0.3.0 aarch64-macos"), here),
        BuildMatch::Different
    );
    // Without a revision on either side, a matching version and platform say nothing more.
    assert_eq!(
        build_match(Some("0.3.0 x86_64-linux"), "0.3.0 x86_64-linux"),
        BuildMatch::Unknown
    );
    assert_eq!(
        build_match(Some("0.3.0 x86_64-linux"), here),
        BuildMatch::Unknown
    );
    assert_eq!(
        build_match(Some(here), "0.3.0 x86_64-linux"),
        BuildMatch::Unknown
    );
    assert_eq!(build_match(None, here), BuildMatch::Unknown);
}

#[test]
fn saving_an_unchanged_xmile_file_keeps_its_bytes_and_a_lossy_save_needs_allow_loss() {
    let path = scratch("kept.xmile");
    let text = xmile("0.1");
    assert!(std::fs::write(&path, &text).is_ok());
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({"path": path.to_string_lossy()}));
    // Unchanged: saved as read, into the file and as a copy.
    ok(&mut e, "file.save", json!({}));
    assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), text);
    let copy = scratch("kept-copy.xmile");
    ok(&mut e, "file.save", json!({"path": copy.to_string_lossy()}));
    assert_eq!(std::fs::read_to_string(&copy).unwrap_or_default(), text);
    // Changed, into the file the reader noted content of that it does not keep: refused.
    let mut f = Engine::new();
    ok(&mut f, "file.open", json!({"path": path.to_string_lossy()}));
    ok(
        &mut f,
        "sd.variable.set",
        json!({"name": "Population", "doc": "people"}),
    );
    let refused = f.execute("file.save", json!({}));
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.code == ErrorCode::Conflict
                && e.message.contains("allow_loss")
                && e.details.is_some()),
        "{refused:?}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap_or_default(), text);
    // To another file it loses nothing of that one.
    let other = scratch("kept-other.xmile");
    ok(
        &mut f,
        "file.save",
        json!({"path": other.to_string_lossy()}),
    );
    ok(
        &mut f,
        "sd.variable.set",
        json!({"name": "Population", "doc": "more"}),
    );
    ok(
        &mut f,
        "file.save",
        json!({"path": path.to_string_lossy(), "allow_loss": true}),
    );
    assert_ne!(std::fs::read_to_string(&path).unwrap_or_default(), text);
}

#[test]
fn a_new_file_over_an_opened_process_file_with_notes_needs_allow_loss() {
    let src = scratch("notes-src.bpmn");
    assert!(std::fs::write(&src, mm1_bytes()).is_ok());
    let mut e = Engine::new();
    let opened = ok(&mut e, "file.open", json!({"path": src.to_string_lossy()}));
    let has_notes = opened["notes"].as_array().is_some_and(|n| !n.is_empty());
    let r = e.execute("file.save", json!({"as_new": true}));
    if has_notes {
        assert!(
            r.as_ref().is_err_and(|e| e.code == ErrorCode::Conflict),
            "{r:?}"
        );
        ok(
            &mut e,
            "file.save",
            json!({"as_new": true, "allow_loss": true}),
        );
    } else {
        assert!(r.is_ok(), "{r:?}");
    }
}
