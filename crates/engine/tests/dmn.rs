//! Decision tables through the engine (ROADMAP M5.5, D30): a model opened with its `.dmn` file
//! beside it, run, edited with `dmn.set`, undone, and saved, the file written back only where
//! its table changed.
#![allow(clippy::panic)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use stochcraft_blocks::{Builder, compile};
use stochcraft_engine::Engine;
use stochcraft_model::dmn::{Decision, DecisionUse, Definitions, Input, Output, Rule, Table};
use stochcraft_model::{Dist, ElementId, Experiment, Model, PropertyValue};
use stochcraft_random::Rng;

fn folder(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("engine-dmn")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    dir
}

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    e.execute(command, params)
        .unwrap_or_else(|err| panic!("{command}: {err}"))
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The sorter: 30 parcels of size 5 and 70 of size 20 into a divert that a table sorts to
/// chute_a or chute_b, with a loop as its default way.
fn sorter() -> Model {
    let mut m = Builder::new();
    let sort = m.divert("sort");
    for (name, size, count) in [("small", 5.0, 30), ("large", 20.0, 70)] {
        let s = m.source(name, Dist::Constant(1.0), Some(count));
        m.property(s, "size", None, PropertyValue::Number(size));
        m.connect(s, sort);
    }
    let (a, b, round) = (m.sink("chute_a"), m.sink("chute_b"), m.sink("loop"));
    m.connect(sort, a);
    m.connect(sort, b);
    m.connect(sort, round);
    m.default_route(sort, round);
    let mut model = m.build(Experiment {
        warmup: 0.0,
        duration: 1_000.0,
        replications: 1,
        ..Experiment::default()
    });
    model.decisions.insert(
        ElementId::unchecked("sort"),
        DecisionUse {
            file: "sorting.dmn".into(),
            decision: "chute".into(),
            route: None,
        },
    );
    model
}

fn table() -> Definitions {
    Definitions {
        id: "sorting".into(),
        name: "Sorting".into(),
        namespace: "https://example.org/sorting".into(),
        decisions: vec![Decision {
            id: "chute".into(),
            name: "Chute".into(),
            variable: None,
            table: Table {
                inputs: vec![Input {
                    expression: "size".into(),
                    type_ref: Some("number".into()),
                    ..Input::default()
                }],
                outputs: vec![Output {
                    name: Some("chute".into()),
                    ..Output::default()
                }],
                rules: vec![
                    Rule {
                        inputs: vec!["< 10".into()],
                        outputs: vec!["\"chute_a\"".into()],
                        ..Rule::default()
                    },
                    Rule {
                        inputs: vec![">= 10".into()],
                        outputs: vec!["\"chute_b\"".into()],
                        ..Rule::default()
                    },
                ],
                ..Table::default()
            },
        }],
    }
}

/// How many parcels each chute takes in one run of the engine's model.
fn chutes(e: &Engine) -> (u64, u64, u64) {
    let compiled = compile(e.model()).unwrap_or_else(|ds| panic!("{ds:?}"));
    let r = compiled
        .run(Rng::new(1))
        .unwrap_or_else(|err| panic!("{err}"));
    let n = |id: &str| r.element(id).map_or(0, |x| x.entered);
    (n("chute_a"), n("chute_b"), n("loop"))
}

fn write_sorter(dir: &Path) -> (PathBuf, PathBuf) {
    let (bpmn, dmn) = (dir.join("sorter.bpmn"), dir.join("sorting.dmn"));
    std::fs::write(&bpmn, stochcraft_io::write_new(&sorter())).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(&dmn, stochcraft_io::dmn::write_dmn(&table())).unwrap_or_else(|e| panic!("{e}"));
    (bpmn, dmn)
}

#[test]
fn a_model_opens_with_its_table_runs_and_saves_it_back_only_when_edited() {
    let dir = folder("open-edit-save");
    let (bpmn, dmn) = write_sorter(&dir);
    let (bpmn_bytes, dmn_bytes) = (read(&bpmn), read(&dmn));
    let mut e = Engine::new();
    let opened = ok(
        &mut e,
        "file.open",
        json!({ "path": bpmn.to_string_lossy() }),
    );
    assert_eq!(opened["diagnostics"], json!([]), "{opened}");
    assert_eq!(e.model().dmn.get("sorting.dmn"), Some(&table()));
    assert_eq!(chutes(&e), (30, 70, 0));

    // Saved unchanged: both files byte for byte.
    ok(&mut e, "file.save", json!({}));
    assert_eq!(read(&bpmn), bpmn_bytes);
    assert_eq!(read(&dmn), dmn_bytes);

    // Swap the chutes: the large parcels now go to chute_a.
    let got = ok(&mut e, "dmn.get", json!({ "file": "sorting.dmn" }));
    let mut t = got["files"][0]["decisions"][0]["table"].clone();
    t["rules"][0]["outputs"][0] = json!("\"chute_b\"");
    t["rules"][1]["outputs"][0] = json!("\"chute_a\"");
    let set = ok(
        &mut e,
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "chute", "table": t }),
    );
    assert_eq!(set["findings"], json!([]));
    assert_eq!(chutes(&e), (70, 30, 0));
    ok(&mut e, "undo", json!({}));
    assert_eq!(chutes(&e), (30, 70, 0));
    ok(&mut e, "redo", json!({}));
    assert_eq!(chutes(&e), (70, 30, 0));

    // Saved: the table file is written back with the edit; the model's file is unchanged.
    ok(&mut e, "file.save", json!({}));
    assert_eq!(read(&bpmn), bpmn_bytes);
    let written = read(&dmn);
    assert_ne!(written, dmn_bytes);
    let back = stochcraft_io::dmn::read_dmn(&written).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(back.decisions[0].table.rules[0].outputs[0], "\"chute_b\"");
    let mut again = Engine::new();
    ok(
        &mut again,
        "file.open",
        json!({ "path": bpmn.to_string_lossy() }),
    );
    assert_eq!(chutes(&again), (70, 30, 0));

    // A table file changed on disk since it was read is never written over.
    std::fs::write(&dmn, b"<changed/>").unwrap_or_else(|err| panic!("{err}"));
    t["rules"][0]["outputs"][0] = json!("\"chute_a\"");
    t["rules"][1]["outputs"][0] = json!("\"chute_b\"");
    ok(
        &mut again,
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "chute", "table": t }),
    );
    assert!(again.execute("file.save", json!({})).is_err());
    assert_eq!(read(&dmn), b"<changed/>");
}

#[test]
fn a_table_that_cannot_be_used_is_a_note_and_a_diagnostic() {
    let dir = folder("unsupported");
    let (bpmn, dmn) = write_sorter(&dir);
    let text = String::from_utf8_lossy(&read(&dmn))
        .replace("hitPolicy=\"UNIQUE\"", "hitPolicy=\"RULE ORDER\"");
    let text = if text.contains("RULE ORDER") {
        text
    } else {
        text.replace("<decisionTable", "<decisionTable hitPolicy=\"RULE ORDER\"")
    };
    std::fs::write(&dmn, text).unwrap_or_else(|err| panic!("{err}"));
    let mut e = Engine::new();
    let opened = ok(
        &mut e,
        "file.open",
        json!({ "path": bpmn.to_string_lossy() }),
    );
    let notes = opened["notes"].to_string();
    assert!(notes.contains("RULE ORDER"), "{notes}");
    let codes: Vec<&str> = opened["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["code"].as_str())
        .collect();
    assert_eq!(codes, vec!["decision_unusable"]);
    // Opened without its folder, the file cannot be read at all.
    let mut bare = Engine::new();
    let opened = bare
        .open_bytes("sorter.bpmn", &read(&bpmn), None)
        .unwrap_or_else(|err| panic!("{err}"));
    assert!(opened["notes"].to_string().contains("sorting.dmn"));
}

#[test]
fn a_table_made_with_commands_is_saved_beside_the_model() {
    let dir = folder("made");
    let mut e = Engine::new();
    let example = stochcraft_engine::COMMANDS
        .iter()
        .find(|c| c.name == "model.set")
        .map(|c| c.example())
        .unwrap_or_default();
    ok(&mut e, "model.set", example);
    ok(
        &mut e,
        "dmn.set",
        json!({
            "file": "rules/desk.dmn",
            "decision": "priority",
            "table": {
                "hit_policy": "FIRST",
                "outputs": [{ "name": "priority" }],
                "rules": [{ "inputs": [], "outputs": ["2"] }]
            }
        }),
    );
    ok(
        &mut e,
        "dmn.use",
        json!({ "element": "desk", "file": "rules/desk.dmn", "decision": "priority" }),
    );
    assert_eq!(
        ok(&mut e, "model.validate", json!({}))["diagnostics"],
        json!([])
    );
    // A cell outside the subset is refused, naming it.
    let bad = e.execute(
        "dmn.set",
        json!({
            "file": "rules/desk.dmn",
            "decision": "priority",
            "table": { "outputs": [{ "name": "priority" }], "rules": [{ "inputs": [], "outputs": ["x + 1"] }] }
        }),
    );
    assert!(
        bad.is_err_and(|err| err.message.contains("rule 1, output 1")),
        "refused"
    );
    let path = dir.join("desk.bpmn");
    ok(
        &mut e,
        "file.save",
        json!({ "path": path.to_string_lossy() }),
    );
    let written = read(&dir.join("rules").join("desk.dmn"));
    let defs = stochcraft_io::dmn::read_dmn(&written).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(defs.decisions[0].id, "priority");
    let mut again = Engine::new();
    ok(
        &mut again,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    assert_eq!(again.model().decisions, e.model().decisions);
    assert_eq!(again.model().dmn, e.model().dmn);
}

/// A save that writes the table file and then fails (here the model's own file cannot be
/// written) leaves the table file recorded as written: saving again, unchanged or after undoing
/// the edit, finds it as this engine left it and goes ahead, rather than taking it for a change
/// made by another program.
#[test]
fn a_table_file_written_by_a_failed_save_is_not_taken_for_a_change() {
    let dir = folder("failed-save");
    let (bpmn, dmn) = write_sorter(&dir);
    let (bpmn_bytes, dmn_bytes) = (read(&bpmn), read(&dmn));
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": bpmn.to_string_lossy() }),
    );
    let got = ok(&mut e, "dmn.get", json!({ "file": "sorting.dmn" }));
    let mut t = got["files"][0]["decisions"][0]["table"].clone();
    t["rules"][0]["outputs"][0] = json!("\"chute_b\"");
    t["rules"][1]["outputs"][0] = json!("\"chute_a\"");
    ok(
        &mut e,
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "chute", "table": t }),
    );
    // The model's file cannot be written: a folder stands in its place.
    let block = |there: bool| {
        if there {
            std::fs::remove_file(&bpmn).unwrap_or_else(|err| panic!("{err}"));
            std::fs::create_dir(&bpmn).unwrap_or_else(|err| panic!("{err}"));
        } else {
            std::fs::remove_dir(&bpmn).unwrap_or_else(|err| panic!("{err}"));
            std::fs::write(&bpmn, &bpmn_bytes).unwrap_or_else(|err| panic!("{err}"));
        }
    };
    block(true);
    assert!(e.execute("file.save", json!({})).is_err());
    let edited = read(&dmn);
    assert_ne!(edited, dmn_bytes, "the table file was written first");
    // Saved again as it is: it goes ahead.
    block(false);
    ok(&mut e, "file.save", json!({}));
    assert_eq!(read(&dmn), edited);
    // Failed again, then saved after undoing the edit: the table goes back.
    t["rules"][0]["outputs"][0] = json!("\"chute_a\"");
    ok(
        &mut e,
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "chute", "table": t }),
    );
    block(true);
    assert!(e.execute("file.save", json!({})).is_err());
    block(false);
    ok(&mut e, "undo", json!({}));
    // The model is as last saved, but its table file is not: still unsaved.
    assert!(e.is_dirty(), "the table file holds the undone edit");
    ok(&mut e, "file.save", json!({}));
    assert_eq!(read(&dmn), edited);
    assert!(!e.is_dirty());
}

/// A table file that could not be read stops a run only while a block uses it.
#[test]
fn a_missing_table_stops_a_run_only_while_a_block_uses_it() {
    let dir = folder("missing");
    let (bpmn, dmn) = write_sorter(&dir);
    std::fs::remove_file(&dmn).unwrap_or_else(|err| panic!("{err}"));
    let mut e = Engine::new();
    let opened = ok(
        &mut e,
        "file.open",
        json!({ "path": bpmn.to_string_lossy() }),
    );
    assert!(opened["notes"].to_string().contains("sorting.dmn"));
    let run = json!({ "replications": 1 });
    assert!(e.execute("run", run.clone()).is_err());
    ok(&mut e, "dmn.use", json!({ "element": "sort" }));
    ok(&mut e, "run", run);
}

/// What `dmn.set` and `dmn.use` take is kept as the files read it back, so the model saves:
/// spaces around a cell, an empty id, a blank route.
#[test]
fn edits_are_kept_as_the_files_read_them_back() {
    let dir = folder("kept");
    let (bpmn, dmn) = write_sorter(&dir);
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": bpmn.to_string_lossy() }),
    );
    let got = ok(&mut e, "dmn.get", json!({ "file": "sorting.dmn" }));
    let mut t = got["files"][0]["decisions"][0]["table"].clone();
    t["rules"][0]["outputs"][0] = json!("  \"chute_a\"  ");
    t["rules"][0]["id"] = json!("");
    t["rules"][1]["inputs"][0] = json!(" >= 10 ");
    ok(
        &mut e,
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "chute", "table": t }),
    );
    let rules = &e.model().dmn["sorting.dmn"].decisions[0].table.rules;
    assert_eq!(rules[0].outputs[0], "\"chute_a\"");
    assert_eq!(rules[0].id, None);
    assert_eq!(rules[1].inputs[0], ">= 10");
    ok(
        &mut e,
        "dmn.use",
        json!({ "element": "sort", "file": "sorting.dmn", "decision": "chute", "route": " " }),
    );
    assert_eq!(
        e.model().decisions[&ElementId::unchecked("sort")].route,
        None
    );
    ok(&mut e, "file.save", json!({}));
    let back = stochcraft_io::dmn::read_dmn(&read(&dmn)).unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(back, e.model().dmn["sorting.dmn"]);
}

/// The app's save writes the model's file only ([`Engine::saved_to`]): a table edited since the
/// `.dmn` file was written keeps the model unsaved, until a save that writes the file.
#[test]
fn a_save_of_the_model_s_file_alone_leaves_an_edited_table_unsaved() {
    let dir = folder("model-only-save");
    let (bpmn, _) = write_sorter(&dir);
    let path = bpmn.to_string_lossy().into_owned();
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({ "path": path }));
    // Nothing edited: the model's file alone is the whole save.
    let bytes = e.save_bytes(false).unwrap_or_else(|err| panic!("{err}"));
    e.saved_to(&path, &bytes)
        .unwrap_or_else(|err| panic!("{err}"));
    assert!(!e.is_dirty());
    // A table edited: the model's file written, the table not.
    let got = ok(&mut e, "dmn.get", json!({ "file": "sorting.dmn" }));
    let mut t = got["files"][0]["decisions"][0]["table"].clone();
    t["rules"][0]["outputs"][0] = json!("\"chute_b\"");
    ok(
        &mut e,
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "chute", "table": t }),
    );
    let bytes = e.save_bytes(false).unwrap_or_else(|err| panic!("{err}"));
    e.saved_to(&path, &bytes)
        .unwrap_or_else(|err| panic!("{err}"));
    assert!(e.is_dirty(), "the edited table is not on disk");
    ok(&mut e, "file.save", json!({}));
    assert!(!e.is_dirty());
}
