//! Editing system dynamics models through the engine's commands: building one, renaming and
//! removing, refused edits, undo and redo, saving, and the diagram.

use serde_json::{Value, json};
use stochcraft_engine::{Engine, ErrorCode, ReplayOptions};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

fn fail(e: &mut Engine, command: &str, params: Value) -> stochcraft_engine::CommandError {
    let r = e.execute(command, params.clone());
    assert!(r.is_err(), "{command} {params} should fail: {r:?}");
    r.err().unwrap_or_else(|| {
        stochcraft_engine::CommandError::new(ErrorCode::Internal, "no error".to_owned())
    })
}

fn model_path(name: &str) -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/models")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn scratch(name: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("sd-edit-tests");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name).to_string_lossy().into_owned()
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect()
}

fn series(results: &Value, name: &str) -> Vec<f64> {
    results["variables"]
        .as_array()
        .and_then(|vs| vs.iter().find(|v| v["name"] == name))
        .map(|v| floats(&v["mean"]))
        .unwrap_or_default()
}

fn dirty(e: &mut Engine) -> bool {
    ok(e, "history", json!({}))["dirty"] == json!(true)
}

/// A growth model built by commands: P(n + 1) = P(n) + 1 * (rate * P(n)).
fn growth(e: &mut Engine) {
    ok(
        e,
        "sd.new",
        json!({ "name": "Growth", "stop": 10.0, "dt": 1.0, "time_units": "Years" }),
    );
    ok(
        e,
        "sd.variable.add",
        json!({ "kind": "aux", "name": "growth rate", "equation": "0.1" }),
    );
    ok(
        e,
        "sd.variable.add",
        json!({ "kind": "stock", "name": "Population", "equation": "100" }),
    );
    ok(
        e,
        "sd.variable.add",
        json!({ "kind": "flow", "name": "births", "equation": "growth_rate * Population",
                "into_stock": "Population" }),
    );
}

#[test]
fn a_model_built_by_commands_grows_exponentially() {
    let mut e = Engine::new();
    growth(&mut e);
    let r = ok(&mut e, "sd.run", json!({}));
    let p = series(&r, "Population");
    assert_eq!(p.len(), 11);
    let mut exact = 100.0;
    for (n, v) in p.iter().enumerate() {
        assert!((v - exact).abs() < 1e-9, "n={n}: {v} vs {exact}");
        exact *= 1.1;
    }
    // The diagram has a shape for each variable, and the flow's pipe ends on the stock.
    let g = ok(&mut e, "sd.get", json!({}));
    let vars = g["variables"].as_array().cloned().unwrap_or_default();
    assert_eq!(vars.len(), 3);
    assert!(vars.iter().all(|v| v["x"].is_number()));
    let births = vars.iter().find(|v| v["name"] == "births");
    assert_eq!(births.map(|v| &v["sink"]), Some(&json!("Population")));
    assert_eq!(births.map(|v| &v["source"]), Some(&Value::Null));
    assert!(
        births
            .and_then(|v| v["pipe"].as_array())
            .is_some_and(|p| p.len() >= 2)
    );
    // The links follow the equation: the rate and the stock feed the flow.
    let links = g["connectors"].as_array().cloned().unwrap_or_default();
    assert_eq!(links.len(), 2, "{links:?}");
    assert!(links.iter().all(|c| c["to"] == "births"));
}

#[test]
fn renaming_rewrites_every_equation_and_flow_list() {
    let mut e = Engine::new();
    ok(&mut e, "sd.new", json!({ "stop": 5.0 }));
    for (kind, name, eqn) in [
        ("aux", "rate", "0.1"),
        ("stock", "S", "100"),
        ("flow", "gain", "rate * S"),
        ("flow", "loss", "rate * S / 2"),
    ] {
        ok(
            &mut e,
            "sd.variable.add",
            json!({ "kind": kind, "name": name, "equation": eqn }),
        );
    }
    ok(
        &mut e,
        "sd.variable.set",
        json!({ "name": "S", "inflows": ["gain"], "outflows": ["loss"] }),
    );
    let before = series(&ok(&mut e, "sd.run", json!({})), "S");

    ok(
        &mut e,
        "sd.variable.set",
        json!({ "name": "rate", "rename": "Growth Rate" }),
    );
    ok(
        &mut e,
        "sd.variable.set",
        json!({ "name": "gain", "rename": "net gain" }),
    );
    let g = ok(&mut e, "sd.get", json!({}));
    let vars = g["variables"].as_array().cloned().unwrap_or_default();
    let by = |n: &str| vars.iter().find(|v| v["name"] == n).cloned();
    assert_eq!(
        by("loss").map(|v| v["equation"].clone()),
        Some(json!("Growth_Rate * S / 2"))
    );
    assert_eq!(
        by("net gain").map(|v| v["equation"].clone()),
        Some(json!("Growth_Rate * S"))
    );
    assert_eq!(
        by("S").map(|v| v["inflows"].clone()),
        Some(json!(["net gain"]))
    );
    // The diagram and its links went with the names.
    assert!(
        g["connectors"]
            .as_array()
            .is_some_and(|c| c.iter().any(|k| k["from"] == "Growth Rate"))
    );
    let after = series(&ok(&mut e, "sd.run", json!({})), "S");
    assert_eq!(before, after);
    // A name taken by another variable is refused.
    let c = fail(
        &mut e,
        "sd.variable.set",
        json!({ "name": "loss", "rename": "NET GAIN" }),
    );
    assert_eq!(c.code, ErrorCode::InvalidModel);
}

#[test]
fn a_variable_in_use_is_not_removed_and_the_refusal_names_the_user() {
    let mut e = Engine::new();
    growth(&mut e);
    let c = fail(
        &mut e,
        "sd.variable.remove",
        json!({ "name": "growth rate" }),
    );
    assert_eq!(c.code, ErrorCode::Conflict);
    assert!(c.message.contains("`births`"), "{}", c.message);
    // A flow a stock lists: refused without force, and with it only the list is cleared; the
    // stock's equation names nothing, so it goes.
    let c = fail(&mut e, "sd.variable.remove", json!({ "name": "births" }));
    assert!(c.message.contains("`Population`"), "{}", c.message);
    ok(
        &mut e,
        "sd.variable.remove",
        json!({ "name": "births", "force": true }),
    );
    let g = ok(&mut e, "sd.get", json!({}));
    assert_eq!(g["variables"][1]["inflows"], json!([]));
    // Nothing names the rate now.
    ok(
        &mut e,
        "sd.variable.remove",
        json!({ "name": "growth rate" }),
    );
    assert_eq!(
        ok(&mut e, "sd.get", json!({}))["variables"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let c = fail(&mut e, "sd.variable.remove", json!({ "name": "nothing" }));
    assert_eq!(c.code, ErrorCode::NotFound);
}

#[test]
fn an_edit_that_does_not_compile_is_refused_and_changes_nothing() {
    let mut e = Engine::new();
    growth(&mut e);
    ok(
        &mut e,
        "sd.variable.add",
        json!({ "kind": "aux", "name": "a", "equation": "1" }),
    );
    ok(
        &mut e,
        "sd.variable.add",
        json!({ "kind": "aux", "name": "b", "equation": "a + 1" }),
    );
    let before = ok(&mut e, "sd.get", json!({}));
    let history = ok(&mut e, "history", json!({}));
    // An algebraic loop.
    let c = fail(
        &mut e,
        "sd.variable.set",
        json!({ "name": "a", "equation": "b" }),
    );
    assert_eq!(c.code, ErrorCode::InvalidModel);
    assert!(c.message.contains("a -> b -> a"), "{}", c.message);
    // A bad equation, a name that is not there, a duplicate and a bad number.
    for (command, params) in [
        ("sd.variable.set", json!({ "name": "a", "equation": "1 +" })),
        (
            "sd.variable.add",
            json!({ "kind": "aux", "name": "c", "equation": "nope * 2" }),
        ),
        (
            "sd.variable.add",
            json!({ "kind": "aux", "name": "B", "equation": "1" }),
        ),
        ("sd.specs.set", json!({ "stop": -1.0 })),
        ("sd.specs.set", json!({ "dt": 0.0 })),
    ] {
        let c = fail(&mut e, command, params.clone());
        assert_eq!(c.code, ErrorCode::InvalidModel, "{command} {params}");
    }
    assert_eq!(ok(&mut e, "sd.get", json!({})), before);
    assert_eq!(ok(&mut e, "history", json!({})), history);
    assert_eq!(
        fail(&mut e, "sd.variable.set", json!({ "name": "a" })).code,
        ErrorCode::InvalidParams
    );
}

#[test]
fn a_stop_condition_is_set_run_reported_and_taken_away() {
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": model_path("logistic.xmile") }),
    );
    ok(
        &mut e,
        "sd.specs.set",
        json!({ "stop_when": { "variable": "population", "above": 500.0 } }),
    );
    let got = ok(&mut e, "sd.get", json!({}));
    assert_eq!(
        got["stop_when"],
        json!({ "variable": "Population", "above": 500.0 })
    );
    let r = ok(&mut e, "sd.run", json!({}));
    let s = &r["stopped"];
    assert_eq!(s["reason"], "condition");
    // The closed form 1000 / (1 + 99 e^(-t/2)) passes 500 at 2 ln 99 = 9.19 years.
    let crossing = s["crossing"].as_f64().unwrap_or(f64::NAN);
    assert!((crossing - 9.190_24).abs() < 0.01, "{crossing}");
    assert!(
        s["message"]
            .as_str()
            .is_some_and(|m| m.contains("Population rose above 500")),
        "{s}"
    );
    let times = floats(&r["times"]);
    assert_eq!(times.last().copied(), s["time"].as_f64());
    // Refusals leave the model as it was.
    for (params, code) in [
        (
            json!({ "stop_when": { "variable": "nobody", "above": 1.0 } }),
            ErrorCode::InvalidModel,
        ),
        (
            json!({ "stop_when": { "variable": "Population", "above": 1.0, "below": 2.0 } }),
            ErrorCode::InvalidParams,
        ),
        (
            json!({ "stop_when": { "variable": "Population" } }),
            ErrorCode::InvalidParams,
        ),
    ] {
        assert_eq!(
            fail(&mut e, "sd.specs.set", params.clone()).code,
            code,
            "{params}"
        );
    }
    ok(&mut e, "sd.specs.set", json!({ "stop_when": null }));
    assert_eq!(ok(&mut e, "sd.get", json!({}))["stop_when"], Value::Null);
    let r = ok(&mut e, "sd.run", json!({}));
    assert_eq!(r["stopped"], Value::Null);
    assert_eq!(floats(&r["times"]).last().copied(), Some(20.0));
}

#[test]
fn three_edits_are_undone_exactly_redone_and_make_the_model_dirty() {
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": model_path("sir.xmile") }),
    );
    let opened = ok(&mut e, "sd.get", json!({}));
    assert!(!dirty(&mut e));
    ok(&mut e, "sd.run", json!({}));
    assert_eq!(ok(&mut e, "sd.get", json!({}))["results_stale"], false);

    ok(
        &mut e,
        "sd.variable.set",
        json!({ "name": "recovery rate", "equation": "0.2" }),
    );
    ok(&mut e, "sd.specs.set", json!({ "stop": 50.0, "dt": 0.5 }));
    ok(
        &mut e,
        "sd.layout.set",
        json!({ "shapes": [{ "name": "Infected", "x": 600.0, "y": 500.0 }] }),
    );
    assert!(dirty(&mut e));
    let edited = ok(&mut e, "sd.get", json!({}));
    assert_ne!(edited, opened);
    // The results of the run before the edits are marked out of date.
    assert_eq!(edited["results_stale"], true);
    let values = ok(&mut e, "sd.get", json!({ "variable": "Infected" }));
    assert_eq!(values["values"]["stale"], true);
    let h = ok(&mut e, "history", json!({}));
    assert_eq!(
        h["undo"],
        json!(["sd.variable.set", "sd.specs.set", "sd.layout.set"])
    );

    for _ in 0..3 {
        ok(&mut e, "undo", json!({}));
    }
    let mut back = ok(&mut e, "sd.get", json!({}));
    assert!(!dirty(&mut e));
    // Everything but the staleness of the run is as it was opened.
    assert_eq!(back["results_stale"], true);
    back["results_stale"] = json!(false);
    assert_eq!(back, opened);
    let c = fail(&mut e, "undo", json!({}));
    assert_eq!(c.code, ErrorCode::NothingToUndo);

    for _ in 0..3 {
        ok(&mut e, "redo", json!({}));
    }
    assert!(dirty(&mut e));
    assert_eq!(ok(&mut e, "sd.get", json!({})), edited);
    // A new edit after an undo drops what could be redone.
    ok(&mut e, "undo", json!({}));
    ok(&mut e, "sd.specs.set", json!({ "stop": 40.0 }));
    assert_eq!(
        fail(&mut e, "redo", json!({})).code,
        ErrorCode::NothingToRedo
    );
    // Running again makes the results current.
    ok(&mut e, "sd.run", json!({}));
    assert_eq!(ok(&mut e, "sd.get", json!({}))["results_stale"], false);
}

#[test]
fn a_saved_model_opens_again_with_the_same_model_and_diagram() {
    let mut e = Engine::new();
    growth(&mut e);
    ok(
        &mut e,
        "sd.layout.set",
        json!({ "shapes": [{ "name": "Population", "x": 400.0, "y": 300.0, "width": 80.0, "height": 50.0 }] }),
    );
    ok(
        &mut e,
        "sd.variable.set",
        json!({ "name": "births", "units": "people/year", "doc": "births <per> year" }),
    );
    assert!(dirty(&mut e));
    // A model that was never saved has no file, so a save needs a path.
    assert_eq!(
        fail(&mut e, "file.save", json!({})).code,
        ErrorCode::InvalidParams
    );
    let path = scratch("growth.xmile");
    let saved = ok(&mut e, "file.save", json!({ "path": path }));
    assert_eq!(saved["format"], "xmile");
    assert!(!dirty(&mut e));
    assert_eq!(e.format(), Some("xmile"));
    assert_eq!(e.path(), Some(path.as_str()));
    let mine = ok(&mut e, "sd.get", json!({}));

    let mut other = Engine::new();
    let opened = ok(&mut other, "file.open", json!({ "path": path }));
    assert_eq!(opened["format"], "xmile");
    assert_eq!(opened["notes"], json!([]));
    let theirs = ok(&mut other, "sd.get", json!({}));
    assert_eq!(theirs["variables"], mine["variables"]);
    assert_eq!(theirs["connectors"], mine["connectors"]);
    assert_eq!(theirs["start"], mine["start"]);
    assert_eq!(theirs["dt"], mine["dt"]);
    assert_eq!(theirs["time_unit"], "Years");
    assert!(!dirty(&mut other));
    assert_eq!(
        series(&ok(&mut other, "sd.run", json!({})), "Population"),
        series(&ok(&mut e, "sd.run", json!({})), "Population")
    );
    // Saving the same model again, with no path, writes the same file.
    let before = std::fs::read(&path).unwrap_or_default();
    ok(&mut other, "file.save", json!({}));
    assert_eq!(std::fs::read(&path).unwrap_or_default(), before);
}

/// The width and height of a shape in a described variable (a valve is 20 by 20).
fn extent(v: &Value) -> (f64, f64) {
    (
        v["width"].as_f64().unwrap_or(20.0),
        v["height"].as_f64().unwrap_or(20.0),
    )
}

fn no_overlaps(vars: &[Value]) {
    for (i, a) in vars.iter().enumerate() {
        for b in vars.iter().skip(i + 1) {
            let ((aw, ah), (bw, bh)) = (extent(a), extent(b));
            let dx = (a["x"].as_f64().unwrap_or(0.0) - b["x"].as_f64().unwrap_or(0.0)).abs();
            let dy = (a["y"].as_f64().unwrap_or(0.0) - b["y"].as_f64().unwrap_or(0.0)).abs();
            assert!(
                !(dx * 2.0 < aw + bw && dy * 2.0 < ah + bh),
                "{} and {} overlap",
                a["name"],
                b["name"]
            );
        }
    }
}

#[test]
fn a_file_without_a_diagram_opens_with_nothing_overlapping() {
    let mut e = Engine::new();
    for file in ["sir.xmile", "logistic.xmile"] {
        let opened = ok(&mut e, "file.open", json!({ "path": model_path(file) }));
        let vars = opened["model"]["variables"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(vars.iter().all(|v| v["x"].is_number()), "{file}");
        no_overlaps(&vars);
    }
}

#[test]
fn shapes_move_and_the_pipes_follow_the_stocks() {
    let mut e = Engine::new();
    growth(&mut e);
    let before = ok(&mut e, "sd.get", json!({}));
    ok(
        &mut e,
        "sd.layout.set",
        json!({ "shapes": [{ "name": "Population", "x": 700.0, "y": 100.0 }] }),
    );
    let after = ok(&mut e, "sd.get", json!({}));
    let pipe = |g: &Value| {
        g["variables"]
            .as_array()
            .and_then(|vs| vs.iter().find(|v| v["name"] == "births"))
            .map(|v| v["pipe"].clone())
            .unwrap_or_default()
    };
    assert_ne!(pipe(&before), pipe(&after));
    // The last point of the pipe is on the edge of the stock: 700 +- 30, 100 +- 20.
    let end = pipe(&after)
        .as_array()
        .and_then(|p| p.last().cloned())
        .unwrap_or_default();
    let (x, y) = (
        end[0].as_f64().unwrap_or(0.0),
        end[1].as_f64().unwrap_or(0.0),
    );
    assert!(
        (x - 700.0).abs() <= 30.0 + 1e-9 && (y - 100.0).abs() <= 20.0 + 1e-9,
        "{end}"
    );
    // A pipe with a slanted segment, an unknown shape, and a link nobody asked for are refused.
    let c = fail(
        &mut e,
        "sd.layout.set",
        json!({ "shapes": [{ "name": "births", "pts": [{ "x": 0.0, "y": 0.0 }, { "x": 10.0, "y": 10.0 }] }] }),
    );
    assert_eq!(c.code, ErrorCode::InvalidParams);
    assert_eq!(
        fail(
            &mut e,
            "sd.layout.set",
            json!({ "shapes": [{ "name": "ghost", "x": 1.0 }] })
        )
        .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        fail(
            &mut e,
            "sd.layout.set",
            json!({ "remove_connectors": [{ "from": "growth rate", "to": "Population" }] })
        )
        .code,
        ErrorCode::NotFound
    );
    // A link can be added and removed.
    ok(
        &mut e,
        "sd.layout.set",
        json!({ "add_connectors": [{ "from": "growth rate", "to": "Population", "angle": 30.0 }] }),
    );
    let links = ok(&mut e, "sd.get", json!({}))["connectors"].clone();
    assert!(links.as_array().is_some_and(|l| {
        l.iter()
            .any(|c| c["to"] == "Population" && c["angle"] == 30.0)
    }));
    ok(
        &mut e,
        "sd.layout.set",
        json!({ "remove_connectors": [{ "from": "growth rate", "to": "Population" }] }),
    );
}

#[test]
fn edits_are_journalled_and_a_journal_from_a_new_model_replays() {
    let mut e = Engine::new();
    growth(&mut e);
    ok(
        &mut e,
        "sd.specs.set",
        json!({ "method": "rk4", "stop": 5.0 }),
    );
    ok(&mut e, "sd.run", json!({}));
    let journal = e.journal_document();
    // The new model is the journal's start; the three adds, the settings and the run follow.
    assert_eq!(journal.start.kind, stochcraft_engine::StartKind::NewSd);
    assert_eq!(journal.entries.len(), 5);

    let mut other = Engine::new();
    let r = other.replay_journal(&journal, ReplayOptions::default());
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(
        ok(&mut other, "sd.get", json!({}))["variables"],
        ok(&mut e, "sd.get", json!({}))["variables"]
    );
    assert_eq!(ok(&mut other, "sd.get", json!({}))["method"], "rk4");
}

#[test]
fn stocks_with_graphical_functions_and_other_settings_can_be_added() {
    let mut e = Engine::new();
    ok(
        &mut e,
        "sd.new",
        json!({ "stop": 4.0, "dt": 4.0, "dt_reciprocal": true }),
    );
    ok(
        &mut e,
        "sd.variable.add",
        json!({ "kind": "aux", "name": "demand curve", "graphical_function":
                { "x": [0.0, 2.0, 4.0], "y": [1.0, 3.0, 3.0] } }),
    );
    ok(
        &mut e,
        "sd.variable.add",
        json!({ "kind": "stock", "name": "level", "equation": "0", "non_negative": true,
                "units": "things" }),
    );
    ok(
        &mut e,
        "sd.variable.add",
        json!({ "kind": "flow", "name": "fill", "equation": "demand_curve(TIME)", "into_stock": "level" }),
    );
    let g = ok(&mut e, "sd.get", json!({}));
    assert_eq!(g["dt"], 0.25);
    let r = ok(&mut e, "sd.run", json!({}));
    let level = series(&r, "level");
    // 4 time units at a flow of about 1 to 3.
    assert!(
        level.last().is_some_and(|v| *v > 5.0 && *v < 12.0),
        "{level:?}"
    );
    // A kind's own settings are checked.
    assert_eq!(
        fail(
            &mut e,
            "sd.variable.add",
            json!({ "kind": "aux", "name": "z", "equation": "1", "inflows": ["fill"] })
        )
        .code,
        ErrorCode::InvalidParams
    );
    assert_eq!(
        fail(
            &mut e,
            "sd.variable.add",
            json!({ "kind": "aux", "name": "z" })
        )
        .code,
        ErrorCode::InvalidParams
    );
}
