//! A file's scenarios: the Scenario choice on the model page and in the sidebar reopens the
//! file in another of them, the values it changes show in bold, the status bar names it, and
//! unsaved changes are asked about beside the choice first.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use serde_json::{Value, json};
use stochcraft_ui_egui::{App, EXAMPLES, Platform, Request};

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

/// The bank example, whose file holds two scenarios: "Five cashiers" (its first) and "Six
/// cashiers", which inherits the rest from it.
fn bank() -> Harness<'static, App> {
    let mut h = harness();
    let e = EXAMPLES
        .iter()
        .find(|e| e.file == "bank.bpmn")
        .expect("the bank example");
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h
}

/// The id of the open scenario.
fn open(h: &Harness<'static, App>) -> Value {
    h.state().engine().scenarios()["open"].clone()
}

fn cashiers(h: &Harness<'static, App>) -> Value {
    stochcraft_engine::wire::model(h.state().engine().model())["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|e| e["id"] == "cashiers"))
        .map(|e| e["capacity"].clone())
        .unwrap_or_default()
}

/// Opens the Scenario choice (the model page's or the sidebar's: they are the same) and picks
/// `name`.
fn choose(h: &mut Harness<'static, App>, name: &str) {
    h.get_all_by_role_and_label(Role::ComboBox, "Scenario")
        .next()
        .expect("a Scenario choice")
        .click();
    settle(h);
    h.get_by_role_and_label(Role::Button, name).click();
    settle(h);
}

/// What a field's label says to a screen reader about why it is in bold, if it is.
fn changed_note(h: &Harness<'static, App>, label: &str) -> Option<String> {
    h.query_all(By::new().role(Role::Label).label(label))
        .find_map(|n| n.accesskit_node().description())
        .and_then(|d| {
            let why = d.split_once(". ").map_or(d.as_str(), |(why, _)| why);
            why.starts_with("Changed from").then(|| why.to_owned())
        })
}

#[test]
fn the_bank_switches_to_six_cashiers_and_shows_what_changed() {
    let mut h = bank();
    // A choice on the model page and one in the sidebar's experiment summary, and the status
    // bar names the scenario.
    assert_eq!(
        h.get_all_by_role_and_label(Role::ComboBox, "Scenario")
            .count(),
        2
    );
    h.get_by_label("Scenario: Five cashiers");
    assert_eq!(open(&h), "scenario");
    assert_eq!(cashiers(&h), json!(5));

    choose(&mut h, "Six cashiers");
    assert_eq!(open(&h), "six-cashiers");
    h.get_by_label("Scenario: Six cashiers");
    assert!(h.query_by_label("Scenario: Five cashiers").is_none());
    assert!(!h.state().has_unsaved_changes());
    assert!(h.state().status().starts_with("Opened bank.bpmn"));

    // The sixth cashier is the scenario's own value, in bold against the one it inherits.
    h.get_by_role_and_label(Role::ListBoxOption, "Cashiers")
        .click();
    settle(&mut h);
    assert_eq!(cashiers(&h), json!(6));
    assert_eq!(
        changed_note(&h, "Capacity").as_deref(),
        Some("Changed from the inherited scenario")
    );
}

#[test]
fn results_of_one_scenario_are_not_shown_for_another() {
    let mut h = bank();
    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    assert!(h.state().results().is_some());
    choose(&mut h, "Six cashiers");
    assert!(
        h.state().results().is_none(),
        "the five cashiers' results went"
    );
}

#[test]
fn unsaved_changes_are_asked_about_beside_the_choice() {
    let mut h = bank();
    h.state_mut()
        .execute("experiment.set", json!({ "replications": 3 }));
    settle(&mut h);
    let question = "Opening Six cashiers drops the unsaved changes to bank.bpmn. Save them first?";

    // Cancel keeps the scenario and the changes.
    choose(&mut h, "Six cashiers");
    h.get_by_label(question);
    assert_eq!(open(&h), "scenario");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    settle(&mut h);
    assert!(h.query_by_label(question).is_none());
    assert_eq!(h.state().engine().model().experiment.replications, 3);

    // Save writes the changes into the file, then opens the other scenario of it, which
    // inherits them.
    choose(&mut h, "Six cashiers");
    h.get_by_role_and_label(Role::Button, "Save").click();
    settle(&mut h);
    let requests = h.state_mut().take_requests();
    let [Request::Write { path, bytes }] = requests.as_slice() else {
        panic!("expected one write, got {requests:?}");
    };
    h.state_mut().written(path, bytes);
    settle(&mut h);
    assert_eq!(open(&h), "six-cashiers");
    assert_eq!(cashiers(&h), json!(6));
    assert_eq!(h.state().engine().model().experiment.replications, 3);

    // Don't Save drops the changes and opens it.
    h.state_mut()
        .execute("experiment.set", json!({ "replications": 7 }));
    settle(&mut h);
    choose(&mut h, "Five cashiers");
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    settle(&mut h);
    assert_eq!(open(&h), "scenario");
    assert_eq!(h.state().engine().model().experiment.replications, 3);
    assert!(!h.state().has_unsaved_changes());
}

#[test]
fn a_file_with_one_scenario_has_no_choice() {
    let mut h = harness();
    let e = EXAMPLES
        .iter()
        .find(|e| e.file == "call-centre.bpmn")
        .expect("the call centre example");
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    assert!(
        h.query_by_role_and_label(Role::ComboBox, "Scenario")
            .is_none()
    );
    assert!(h.query_by_label_contains("Scenario:").is_none());
}

#[test]
fn the_start_page_says_which_examples_have_scenarios() {
    let h = harness();
    // The M/M/1 queue's card, which is featured, counts its file's scenarios.
    let n = stochcraft_engine::file_scenarios(EXAMPLES[0].bytes).len();
    assert!(n > 1);
    h.get_by_label(&format!("{n} scenarios to compare"));
    assert!(h.query_by_label("1 scenario to compare").is_none());
}
