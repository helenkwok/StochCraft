//! The properties of the selection (M5.5, U2): a value changed from its default, or from the
//! scenario the opened one inherits from, is in bold with a reset beside it; a reset is one undo
//! step; several selected blocks show the properties they share and are edited together in one
//! step; a selected flow shows its own properties; the keyboard reaches a field's reset right
//! after the field.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use std::collections::BTreeSet;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use serde_json::{Value, json};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

/// The M/M/1 model with its two scenarios: "base", and "two-clerks", which inherits from it and
/// gives the server a second clerk.
const MM1: &[u8] = include_bytes!("../../../tests/models/mm1.bpmn");

fn harness() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    // English words and numbers (`1,234.5`), whatever this machine's region and language.
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h
}

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The M/M/1 example (arrive → serve → leave), its server's page shown.
fn mm1_server() -> Harness<'static, App> {
    let mut h = harness();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h.get_by_role_and_label(Role::ListBoxOption, "Serve customer")
        .click();
    settle(&mut h);
    h
}

fn element(h: &Harness<'static, App>, id: &str) -> Value {
    stochcraft_engine::wire::model(h.state().engine().model())["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|e| e["id"] == id).cloned())
        .unwrap_or_else(|| panic!("no element {id}"))
}

fn connection(h: &Harness<'static, App>, id: &str) -> Value {
    stochcraft_engine::wire::model(h.state().engine().model())["connections"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["id"] == id).cloned())
        .unwrap_or_else(|| panic!("no connection {id}"))
}

/// What a field's label says to a screen reader besides its text: why it is in bold, or
/// nothing while its value is the reference one.
fn changed_note(h: &Harness<'static, App>, label: &str) -> Option<String> {
    h.query_all(By::new().role(Role::Label).label(label))
        .find_map(|n| n.accesskit_node().description().map(str::to_owned))
}

fn has_reset(h: &Harness<'static, App>, label: &str) -> bool {
    h.query_by_role_and_label(Role::Button, &format!("Reset {label}"))
        .is_some()
}

/// Replaces the text of the field labelled `label` and presses Enter.
fn type_into(h: &mut Harness<'static, App>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label)
        .scroll_to_me();
    settle(h);
    h.get_by_role_and_label(Role::TextInput, label).click();
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

/// The label of the focused widget, if any.
fn focused(h: &Harness<'static, App>) -> Option<String> {
    h.query_all(By::new())
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
}

#[test]
fn a_changed_value_is_bold_and_the_default_is_not() {
    let mut h = mm1_server();
    // A new task with a resource serves in a mean of 0.8; this one in 1, so its service time is
    // changed. It has one unit, as a new one does.
    assert_eq!(
        changed_note(&h, "Service time").as_deref(),
        Some("Changed from the default")
    );
    assert!(has_reset(&h, "Service time"));
    assert_eq!(changed_note(&h, "Capacity"), None);
    assert!(!has_reset(&h, "Capacity"));

    type_into(&mut h, "Capacity", "2");
    assert_eq!(element(&h, "serve")["capacity"], json!(2));
    assert_eq!(
        changed_note(&h, "Capacity").as_deref(),
        Some("Changed from the default")
    );
    assert!(has_reset(&h, "Capacity"));
}

#[test]
fn a_reset_restores_the_default_in_one_undo_step() {
    let mut h = mm1_server();
    h.get_by_role_and_label(Role::Button, "Reset Service time")
        .click();
    settle(&mut h);
    assert_eq!(
        element(&h, "serve")["service"],
        json!({"type": "exponential", "mean": 0.8})
    );
    assert_eq!(changed_note(&h, "Service time"), None);
    assert!(!has_reset(&h, "Service time"));

    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Change Block");
    assert_eq!(
        element(&h, "serve")["service"],
        json!({"type": "exponential", "mean": 1.0})
    );
    // That one undo took the model back to the file as it was opened.
    assert!(!h.state().has_unsaved_changes());
    assert!(has_reset(&h, "Service time"));
}

#[test]
fn a_scenario_override_is_bold_against_its_base_and_resets_to_it() {
    let mut h = harness();
    h.state_mut()
        .open_scenario("mm1.bpmn", MM1, Some("two-clerks"));
    settle(&mut h);
    h.get_by_role_and_label(Role::ListBoxOption, "Serve customer")
        .click();
    settle(&mut h);
    // Two clerks where the base scenario has one.
    assert_eq!(element(&h, "serve")["capacity"], json!(2));
    assert_eq!(
        changed_note(&h, "Capacity").as_deref(),
        Some("Changed from the inherited scenario")
    );
    // A service time of 1, as in the base scenario: not a change of this one, though a new
    // block's would be 0.8.
    assert_eq!(changed_note(&h, "Service time"), None);

    h.get_by_role_and_label(Role::Button, "Reset Capacity")
        .click();
    settle(&mut h);
    assert_eq!(element(&h, "serve")["capacity"], json!(1));
    assert_eq!(changed_note(&h, "Capacity"), None);
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(element(&h, "serve")["capacity"], json!(2));

    // The base scenario inherits from none: its values are measured against a new block's.
    h.state_mut().open_scenario("mm1.bpmn", MM1, Some("base"));
    settle(&mut h);
    h.get_by_role_and_label(Role::ListBoxOption, "Serve customer")
        .click();
    settle(&mut h);
    assert_eq!(changed_note(&h, "Capacity"), None);
    assert_eq!(
        changed_note(&h, "Service time").as_deref(),
        Some("Changed from the default")
    );
}

#[test]
fn several_blocks_show_what_they_share_and_change_together_in_one_step() {
    let mut h = harness();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    let added = h
        .state_mut()
        .execute(
            "block.add",
            json!({
                "kind": "server",
                "name": "Second desk",
                "capacity": 1,
                "service": {"type": "exponential", "mean": 2.0},
            }),
        )
        .expect("a second task with a resource");
    let second = added["id"].as_str().expect("its id").to_owned();
    h.state_mut()
        .select(BTreeSet::from(["serve".to_owned(), second.clone()]));
    settle(&mut h);
    h.get_by_label("2 blocks selected");
    h.get_by_label("Shared properties");
    // Means of 1 and 2: the field says they differ, rather than showing either.
    let mean = h.get_by_role_and_label(Role::TextInput, "Mean");
    assert_eq!(mean.accesskit_node().description(), Some("Mixed values"));

    // One capacity for both, each keeping its own mean, in one undo step.
    type_into(&mut h, "Capacity", "3");
    assert_eq!(element(&h, "serve")["capacity"], json!(3));
    assert_eq!(element(&h, &second)["capacity"], json!(3));
    assert_eq!(element(&h, "serve")["service"]["mean"], json!(1.0));
    assert_eq!(element(&h, &second)["service"]["mean"], json!(2.0));
    assert!(has_reset(&h, "Capacity"));
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Changes");
    assert_eq!(element(&h, "serve")["capacity"], json!(1));
    assert_eq!(element(&h, &second)["capacity"], json!(1));

    // A mean typed into the mixed field sets both.
    type_into(&mut h, "Mean", "1.5");
    assert_eq!(element(&h, "serve")["service"]["mean"], json!(1.5));
    assert_eq!(element(&h, &second)["service"]["mean"], json!(1.5));

    // A reset puts each back to its own default, in one step.
    h.get_by_role_and_label(Role::Button, "Reset Service time")
        .click();
    settle(&mut h);
    for id in ["serve", second.as_str()] {
        assert_eq!(
            element(&h, id)["service"],
            json!({"type": "exponential", "mean": 0.8})
        );
    }
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(element(&h, "serve")["service"]["mean"], json!(1.5));
}

#[test]
fn a_selected_flow_shows_its_own_properties() {
    let mut h = harness();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    // The flow from the server to the departures.
    let flow = stochcraft_engine::wire::model(h.state().engine().model())["connections"]
        .as_array()
        .and_then(|cs| cs.iter().find(|c| c["from"] == "serve"))
        .and_then(|c| c["id"].as_str().map(str::to_owned))
        .expect("a flow out of the server");
    h.state_mut().select_flow(&flow);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Remove flow");
    let rework_note = |h: &Harness<'static, App>| {
        h.get_by_role_and_label(Role::CheckBox, "Rework")
            .accesskit_node()
            .description()
            .map(str::to_owned)
    };
    assert_eq!(rework_note(&h), None);

    h.get_by_role_and_label(Role::CheckBox, "Rework").click();
    settle(&mut h);
    assert_eq!(connection(&h, &flow)["rework"], json!(true));
    assert_eq!(rework_note(&h).as_deref(), Some("Changed from the default"));
    h.get_by_role_and_label(Role::Button, "Reset Rework")
        .click();
    settle(&mut h);
    assert!(connection(&h, &flow)["rework"].as_bool() != Some(true));
    assert!(!has_reset(&h, "Rework"));
    assert_eq!(h.state().selected_flow(), Some(flow.as_str()));
}

/// Tab goes from a field to its reset, then on to the next field; Shift+Tab comes back.
#[test]
fn the_keyboard_reaches_a_fields_reset_right_after_the_field() {
    let mut h = mm1_server();
    h.get_by_role_and_label(Role::TextInput, "Capacity").focus();
    settle(&mut h);
    // On to the service time's choice of distribution (past its "?").
    for _ in 0..4 {
        if focused(&h).as_deref() == Some("Service time") {
            break;
        }
        h.key_press(egui::Key::Tab);
        settle(&mut h);
    }
    assert_eq!(focused(&h).as_deref(), Some("Service time"));
    h.key_press(egui::Key::Tab);
    settle(&mut h);
    assert_eq!(focused(&h).as_deref(), Some("Reset Service time"));
    h.key_press(egui::Key::Tab);
    settle(&mut h);
    assert_eq!(focused(&h).as_deref(), Some("Mean"));
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Tab);
    settle(&mut h);
    assert_eq!(focused(&h).as_deref(), Some("Reset Service time"));
    // Enter on the focused reset puts the default back.
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert_eq!(
        element(&h, "serve")["service"],
        json!({"type": "exponential", "mean": 0.8})
    );
}
