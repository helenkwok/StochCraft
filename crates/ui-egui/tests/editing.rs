//! Editing a model in the window, as a user would: selecting blocks in the process list, the
//! inspector's forms (generated from the engine's command schemas), connections, the experiment,
//! the keyboard, and that each edit is one undo step.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), App::new(Platform::DESKTOP));
    h.run();
    h
}

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// Opens the M/M/1 example (arrive → serve → leave).
fn mm1() -> Harness<'static, App> {
    let mut h = harness();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h
}

fn model(h: &Harness<'static, App>) -> Value {
    stochcraft_engine::wire::model(h.state().engine().model())
}

fn element(h: &Harness<'static, App>, id: &str) -> Value {
    model(h)["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|e| e["id"] == id).cloned())
        .unwrap_or_else(|| panic!("no element {id}"))
}

fn click(h: &mut Harness<'static, App>, role: Role, label: &str) {
    h.get_all_by_role_and_label(role, label)
        .last()
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
        .click();
    settle(h);
}

/// Replaces the text of the field labelled `label` and presses Enter.
fn type_into(h: &mut Harness<'static, App>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label).click();
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

#[test]
fn a_parameter_typed_in_the_inspector_is_one_undo_step() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    h.get_by_label("Parameters");
    type_into(&mut h, "Mean", "1.25");
    assert_eq!(
        element(&h, "serve")["service"],
        json!({"type": "exponential", "mean": 1.25})
    );
    assert!(h.state().has_unsaved_changes());

    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Change Block");
    assert_eq!(element(&h, "serve")["service"]["mean"], json!(1.0));
}

#[test]
fn text_that_is_not_a_number_stays_in_its_field_until_fixed() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    type_into(&mut h, "Capacity", "two");
    // Not committed, and marked.
    assert_eq!(element(&h, "serve")["capacity"], json!(1));
    h.get_by_label("a whole number");
    assert!(!h.state().has_unsaved_changes());
    type_into(&mut h, "Capacity", "2");
    assert_eq!(element(&h, "serve")["capacity"], json!(2));
    assert!(h.query_by_label("a whole number").is_none());
}

#[test]
fn choosing_another_distribution_keeps_its_scale() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    click(&mut h, Role::ComboBox, "Service time");
    click(&mut h, Role::Button, "Triangular");
    let service = element(&h, "serve")["service"].clone();
    assert_eq!(service["type"], "triangular");
    assert_eq!(service["mode"], json!(1.0), "{service}");
    assert!(service["min"].as_f64() < service["mode"].as_f64());
    // The new kind's fields are on screen with the time unit.
    h.get_by_role_and_label(Role::TextInput, "Most likely");
}

#[test]
fn a_model_is_built_from_blocks_connected_and_run() {
    let mut h = harness();
    h.state_mut()
        .execute("model.new", json!({"name": "Clinic"}));
    settle(&mut h);
    for kind in ["Arrivals", "Task with a resource", "Departures"] {
        click(&mut h, Role::Button, "+ Add block");
        click(&mut h, Role::Button, kind);
    }
    let ids: Vec<String> = model(&h)["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["id"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(ids, ["source-1", "server-1", "sink-1"]);

    // Join them from the model overview's connections table.
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    for (from, to) in [("source-1", "server-1"), ("server-1", "sink-1")] {
        click(&mut h, Role::ComboBox, "From");
        click(&mut h, Role::Button, from);
        click(&mut h, Role::ComboBox, "To");
        click(&mut h, Role::Button, to);
        click(&mut h, Role::Button, "Connect");
    }
    assert_eq!(model(&h)["connections"].as_array().map(Vec::len), Some(2));
    h.get_by_label("None found. The model is ready to run.");

    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    assert!(h.state().results().is_some(), "{}", h.state().status());
}

#[test]
fn the_keyboard_moves_through_blocks_and_deletes_one() {
    let mut h = mm1();
    h.key_press(egui::Key::ArrowDown);
    settle(&mut h);
    h.key_press(egui::Key::ArrowDown);
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "Capacity"); // the server's inspector
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    assert!(
        model(&h)["elements"]
            .as_array()
            .is_some_and(|e| e.iter().all(|e| e["id"] != "serve"))
    );
    // Removing it left the other two unconnected: the overview says so, and the block is back
    // with one undo.
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    assert!(
        h.query_by_label("None found. The model is ready to run.")
            .is_none()
    );
    h.state_mut().undo();
    settle(&mut h);
    element(&h, "serve");
}

#[test]
fn the_experiment_and_flows_are_edited_from_their_forms() {
    let mut h = mm1();
    type_into(&mut h, "Replications", "32");
    assert_eq!(model(&h)["experiment"]["replications"], json!(32));
    click(&mut h, Role::ComboBox, "Time unit");
    click(&mut h, Role::Button, "hours");
    assert_eq!(model(&h)["experiment"]["time_unit"], json!("hour"));

    click(&mut h, Role::ListBoxOption, "Serve customer");
    // The inspector lists the flows; removing one is an edit like any other.
    h.get_all_by_role_and_label(Role::Button, "Remove")
        .last()
        .expect("a flow's Remove")
        .click();
    settle(&mut h);
    assert_eq!(model(&h)["connections"].as_array().map(Vec::len), Some(1));
    click(&mut h, Role::ComboBox, "Add a flow to…");
    click(&mut h, Role::Button, "Customer leaves");
    assert_eq!(model(&h)["connections"].as_array().map(Vec::len), Some(2));
}

#[test]
fn a_property_is_added_typed_given_an_expression_and_removed() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    h.get_by_role_and_label(Role::TextInput, "New property name")
        .click();
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "New property name")
        .type_text("visits");
    settle(&mut h);
    click(&mut h, Role::Button, "Add property");
    assert_eq!(
        element(&h, "serve")["properties"],
        json!([{"name": "visits", "value": {"number": 0.0}}])
    );

    click(&mut h, Role::ComboBox, "Type");
    click(&mut h, Role::Button, "Whole number");
    assert_eq!(element(&h, "serve")["properties"][0]["type"], "long");

    click(&mut h, Role::ComboBox, "Value");
    click(&mut h, Role::Button, "Expression");
    type_into(&mut h, "Expression", "getProperty('visits') + 1");
    assert_eq!(
        element(&h, "serve")["properties"][0]["value"],
        json!({"expression": "getProperty('visits') + 1"})
    );
    assert!(
        h.query_by_label("Error").is_none(),
        "no error: {}",
        h.state().status()
    );

    // A malformed expression is the model's problem, shown on the block, not a failure.
    type_into(&mut h, "Expression", "getProperty('visits' +");
    h.get_by_label_contains("the expression ends too early (at character 23)");
    assert!(h.query_by_label("Error").is_some());

    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Set Property");
    let remove = h
        .get_all_by_role_and_label(Role::Button, "Remove")
        .next()
        .expect("the property's Remove");
    remove.click();
    settle(&mut h);
    assert!(element(&h, "serve")["properties"].is_null());
}

#[test]
fn a_decision_routes_by_probability_and_condition() {
    let mut h = harness();
    h.state_mut()
        .execute("model.new", json!({"name": "Triage"}));
    settle(&mut h);
    for kind in [
        "Arrivals",
        "Decision (one way)",
        "Task",
        "Task",
        "Departures",
    ] {
        click(&mut h, Role::Button, "+ Add block");
        click(&mut h, Role::Button, kind);
    }
    let gw = "exclusive_gateway-1";
    assert_eq!(element(&h, gw)["kind"], "exclusive_gateway");
    for (from, to) in [
        ("source-1", gw),
        (gw, "delay-1"),
        (gw, "delay-2"),
        ("delay-1", "sink-1"),
        ("delay-2", "sink-1"),
    ] {
        h.state_mut()
            .execute("connect", json!({ "from": from, "to": to }));
    }
    settle(&mut h);
    let flow_to = |h: &Harness<'static, App>, to: &str| {
        model(h)["connections"]
            .as_array()
            .and_then(|cs| {
                cs.iter()
                    .find(|c| c["from"] == gw && c["to"] == to)
                    .and_then(|c| c["id"].as_str().map(str::to_owned))
            })
            .expect("a flow out of the decision")
    };
    let first = flow_to(&h, "delay-1");

    // The decision's inspector: a probability on one way out, blank on the other.
    click(&mut h, Role::ListBoxOption, gw);
    type_into(&mut h, &format!("Probability of {first}"), "0.3");
    let conn = |h: &Harness<'static, App>, id: &str| {
        model(h)["connections"]
            .as_array()
            .and_then(|cs| cs.iter().find(|c| c["id"] == id).cloned())
            .unwrap_or_default()
    };
    assert_eq!(conn(&h, &first)["probability"], json!(0.3));
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Change Flow");
    h.state_mut().redo();
    settle(&mut h);

    // A default way out.
    click(&mut h, Role::ComboBox, "Default flow");
    click(&mut h, Role::Button, "to delay-2");
    assert_eq!(element(&h, gw)["default"], json!(flow_to(&h, "delay-2")));

    // A condition that can't be read is the decision's problem, shown on it.
    type_into(
        &mut h,
        &format!("Condition of {first}"),
        "getProperty('x' >",
    );
    assert_eq!(conn(&h, &first)["condition"], json!("getProperty('x' >"));
    assert!(h.query_by_label("Error").is_some());
    // Cleared, the condition goes.
    let field = format!("Condition of {first}");
    h.get_by_role_and_label(Role::TextInput, &field).click();
    settle(&mut h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert!(conn(&h, &first)["condition"].is_null());

    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    assert!(h.state().results().is_some(), "{}", h.state().status());
}
