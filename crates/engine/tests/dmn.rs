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
