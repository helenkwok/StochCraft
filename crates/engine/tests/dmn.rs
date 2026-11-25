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

/// The model's example with a desk, for tables made with commands.
fn example() -> Engine {
    let mut e = Engine::new();
    let example = stochcraft_engine::COMMANDS
        .iter()
        .find(|c| c.name == "model.set")
        .map(|c| c.example())
        .unwrap_or_default();
    ok(&mut e, "model.set", example);
    e
}

/// A table of one rule giving `value`, with the rule id `rule` if given.
fn one_rule(value: &str, rule: Option<&str>) -> Value {
    let mut r = json!({ "inputs": [], "outputs": [value] });
    if let Some(id) = rule {
        r["id"] = json!(id);
    }
    json!({ "hit_policy": "FIRST", "outputs": [{ "name": "priority" }], "rules": [r] })
}

/// DMN's ids are XML ids, one per element of a file: a new file's own id is one its decision does
/// not use, and an id given twice in a file is refused, naming it.
#[test]
fn every_id_in_a_dmn_file_is_used_once() {
    let dir = folder("ids");
    let mut e = example();
    // A decision named as a new file's id would be: the file takes another.
    ok(
        &mut e,
        "dmn.set",
        json!({ "file": "desk.dmn", "decision": "definitions", "table": one_rule("1", None) }),
    );
    let root = e.model().dmn["desk.dmn"].id.clone();
    assert_ne!(root, "definitions");
    // Another decision with the file's id, or a rule with its decision's, is refused.
    for (decision, rule) in [(root.as_str(), None), ("second", Some("second"))] {
        let refused = e.execute(
            "dmn.set",
            json!({ "file": "desk.dmn", "decision": decision, "table": one_rule("2", rule) }),
        );
        assert!(
            refused.is_err_and(|err| err.message.contains("used twice")),
            "{decision}"
        );
    }
    assert_eq!(e.model().dmn["desk.dmn"].decisions.len(), 1);
    let path = dir.join("desk.bpmn");
    ok(
        &mut e,
        "file.save",
        json!({ "path": path.to_string_lossy() }),
    );
    let defs = stochcraft_io::dmn::read_dmn(&read(&dir.join("desk.dmn")))
        .unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(defs.id, root);
    assert_eq!(defs.decisions[0].id, "definitions");

    // In a file read from disk, an id of what the model does not hold counts too.
    let (bpmn, dmn) = write_sorter(&dir);
    let text = String::from_utf8_lossy(&read(&dmn)).replacen(
        "<decision ",
        "<inputData id=\"size\" name=\"size\"/>\n  <decision ",
        1,
    );
    std::fs::write(&dmn, text).unwrap_or_else(|err| panic!("{err}"));
    let mut opened = Engine::new();
    ok(
        &mut opened,
        "file.open",
        json!({ "path": bpmn.to_string_lossy() }),
    );
    let got = ok(&mut opened, "dmn.get", json!({ "file": "sorting.dmn" }));
    let mut t = got["files"][0]["decisions"][0]["table"].clone();
    t["rules"][0]["id"] = json!("size");
    let refused = opened.execute(
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "chute", "table": t }),
    );
    assert!(refused.is_err_and(|err| err.message.contains("`size`")));
    t["rules"][0]["id"] = json!("small");
    ok(
        &mut opened,
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "chute", "table": t }),
    );
}

/// A decision cannot be added to a `.dmn` file read from disk yet (saving writes edited tables
/// into it, not new decisions), so `dmn.set` refuses it before changing the model, pointing to a
/// new file; a new file, and an edit of a decision the file has, still go ahead and save.
#[test]
fn a_decision_is_added_to_a_new_file_not_to_one_read() {
    let dir = folder("add-decision");
    let (bpmn, _) = write_sorter(&dir);
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": bpmn.to_string_lossy() }),
    );
    let before = e.model().clone();
    let refused = e.execute(
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "backup", "table": one_rule("1", None) }),
    );
    assert!(
        refused.is_err_and(|err| err.message.contains("new .dmn file")),
        "refused"
    );
    assert!(e.model() == &before, "the model is unchanged");
    ok(
        &mut e,
        "dmn.set",
        json!({ "file": "backup.dmn", "decision": "backup", "table": one_rule("1", None) }),
    );
    let got = ok(&mut e, "dmn.get", json!({ "file": "sorting.dmn" }));
    let mut t = got["files"][0]["decisions"][0]["table"].clone();
    t["rules"][0]["outputs"][0] = json!("\"chute_b\"");
    ok(
        &mut e,
        "dmn.set",
        json!({ "file": "sorting.dmn", "decision": "chute", "table": t }),
    );
    ok(&mut e, "file.save", json!({}));
    assert!(!e.is_dirty());
    let backup = stochcraft_io::dmn::read_dmn(&read(&dir.join("backup.dmn")))
        .unwrap_or_else(|err| panic!("{err}"));
    assert_eq!(backup.decisions[0].id, "backup");
}

/// Two references that may name one file, here new ones differing only in case (one file where
/// the file system ignores case), with different tables: the save is refused before anything is
/// written, rather than one table writing over the other.
#[test]
fn two_references_to_one_file_with_different_tables_are_not_saved() {
    let dir = folder("one-file");
    let mut e = example();
    for (file, value) in [("sorting.dmn", "1"), ("Sorting.dmn", "2")] {
        ok(
            &mut e,
            "dmn.set",
            json!({ "file": file, "decision": "priority", "table": one_rule(value, None) }),
        );
    }
    let path = dir.join("desk.bpmn");
    let refused = e.execute("file.save", json!({ "path": path.to_string_lossy() }));
    assert!(
        refused.is_err_and(|err| err.message.contains("name one file")),
        "refused"
    );
    assert!(!dir.join("sorting.dmn").exists() && !dir.join("Sorting.dmn").exists());
    assert!(!path.exists());
}

/// A link in the model's folder to a table file the model also reads by its own name: an edit
/// through one reference is not saved over the other's table.
#[cfg(unix)]
#[test]
fn a_link_to_a_table_file_is_the_same_file() {
    let dir = folder("link");
    let mut model = sorter();
    model.decisions.insert(
        ElementId::unchecked("chute_a"),
        DecisionUse {
            file: "link.dmn".into(),
            decision: "chute".into(),
            route: None,
        },
    );
    let (bpmn, dmn) = (dir.join("sorter.bpmn"), dir.join("sorting.dmn"));
    std::fs::write(&bpmn, stochcraft_io::write_new(&model)).unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(&dmn, stochcraft_io::dmn::write_dmn(&table())).unwrap_or_else(|e| panic!("{e}"));
    std::os::unix::fs::symlink("sorting.dmn", dir.join("link.dmn"))
        .unwrap_or_else(|e| panic!("{e}"));
    let dmn_bytes = read(&dmn);
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": bpmn.to_string_lossy() }),
    );
    assert_eq!(e.model().dmn.len(), 2);
    // Unchanged, both read as the one file: saved as it is.
    ok(&mut e, "file.save", json!({}));
    assert_eq!(read(&dmn), dmn_bytes);
    // Edited through the link: refused, and the file is as it was.
    let got = ok(&mut e, "dmn.get", json!({ "file": "link.dmn" }));
    let mut t = got["files"][0]["decisions"][0]["table"].clone();
    t["rules"][0]["outputs"][0] = json!("\"chute_b\"");
    ok(
        &mut e,
        "dmn.set",
        json!({ "file": "link.dmn", "decision": "chute", "table": t }),
    );
    let refused = e.execute("file.save", json!({}));
    assert!(
        refused.is_err_and(|err| err.message.contains("name one file")),
        "refused"
    );
    assert_eq!(read(&dmn), dmn_bytes);
    assert!(
        std::fs::symlink_metadata(dir.join("link.dmn")).is_ok_and(|m| m.file_type().is_symlink())
    );
}

/// The app's Save As of the model's file alone into another folder: the tables are not beside it
/// there, so the model is unsaved, though no table was edited; saved back into its own folder,
/// or with `file.save` (which writes the tables there too), it is saved.
#[test]
fn a_save_of_the_model_s_file_alone_into_another_folder_leaves_it_unsaved() {
    let dir = folder("model-only-elsewhere");
    let (bpmn, _) = write_sorter(&dir);
    let path = bpmn.to_string_lossy().into_owned();
    let other = dir.join("other");
    std::fs::create_dir_all(&other).unwrap_or_else(|err| panic!("{err}"));
    let elsewhere = other.join("sorter.bpmn").to_string_lossy().into_owned();
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({ "path": path }));
    let bytes = e.save_bytes(false).unwrap_or_else(|err| panic!("{err}"));
    // Another name in the same folder: the tables are beside it.
    let renamed = dir.join("renamed.bpmn").to_string_lossy().into_owned();
    e.saved_to(&renamed, &bytes)
        .unwrap_or_else(|err| panic!("{err}"));
    assert!(!e.is_dirty());
    // Another folder: they are not.
    e.saved_to(&elsewhere, &bytes)
        .unwrap_or_else(|err| panic!("{err}"));
    assert!(e.is_dirty(), "the tables are not in the other folder");
    // Back into the first folder: they are again.
    e.saved_to(&path, &bytes)
        .unwrap_or_else(|err| panic!("{err}"));
    assert!(!e.is_dirty());
    // Into the other folder again, then saved there with its tables: saved.
    e.saved_to(&elsewhere, &bytes)
        .unwrap_or_else(|err| panic!("{err}"));
    assert!(e.is_dirty());
    ok(&mut e, "file.save", json!({}));
    assert!(other.join("sorting.dmn").exists());
    assert!(!e.is_dirty());
}
