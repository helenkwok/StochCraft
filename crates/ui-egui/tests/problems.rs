//! Problems where they are: Run says how many errors the model has and shows them instead of
//! running, a chip above the canvas and the status bar's error lead to their blocks, and the
//! inspector keeps a fixed place for its block's problems.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use serde_json::json;
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

/// The app with English numbers (`1,234.5`), so the tests read the same in every region.
fn english(mut app: App) -> App {
    app.set_number_region("en");
    // The interface in English too, whatever this machine's language.
    app.set_lang(craft_i18n::Lang::EN);
    app
}

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The M/M/1 example (arrive → serve → leave) in a 1200 × 900 window.
fn mm1() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(
            |ui, app: &mut App| app.ui(ui),
            english(App::new(Platform::DESKTOP)),
        );
    h.run();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h
}

fn set(h: &mut Harness<'static, App>, params: serde_json::Value) {
    h.state_mut()
        .execute("block.set", params)
        .expect("the block is set");
    settle(h);
}

/// The block the inspector shows, by its id line ("id serve"); `None` on the model page.
fn shown(h: &Harness<'static, App>) -> Option<String> {
    h.query_all_by_role(Role::Label)
        .filter_map(|n| n.accesskit_node().value())
        .find_map(|l| l.strip_prefix("id ").map(str::to_owned))
}

/// The label of the focused widget, if any.
fn focused(h: &Harness<'static, App>) -> Option<String> {
    h.query_all(By::new())
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
}

#[test]
fn run_says_how_many_problems_and_shows_the_first_instead_of_running() {
    let mut h = mm1();
    h.get_by_role_and_label(Role::Button, "Run");
    set(&mut h, json!({"id": "serve", "capacity": 0}));
    h.get_by_role_and_label(Role::Button, "Run (1 problem)")
        .click();
    settle(&mut h);
    assert!(!h.state().is_running());
    assert_eq!(shown(&h).as_deref(), Some("serve"));
    // Its field, focused, and what is wrong in the status bar.
    assert_eq!(focused(&h).as_deref(), Some("Capacity"));
    assert_eq!(
        h.state().status(),
        "Serve customer would never have a unit to work with. Give it at least 1 unit."
    );

    // A second error: Run counts both, and shows them one after the other.
    set(
        &mut h,
        json!({"id": "arrive", "interarrival": {"type": "exponential", "mean": -1.0}}),
    );
    h.get_by_role_and_label(Role::Button, "Run (2 problems)")
        .click();
    settle(&mut h);
    let first = shown(&h);
    h.get_by_role_and_label(Role::Button, "Run (2 problems)")
        .click();
    settle(&mut h);
    let second = shown(&h);
    assert_ne!(first, second);
    let mut both = [first, second];
    both.sort();
    assert_eq!(both, [Some("arrive".to_owned()), Some("serve".to_owned())]);
    assert!(!h.state().is_running());
}

#[test]
fn f5_and_model_run_show_the_problem_too_and_warnings_alone_run() {
    let mut h = mm1();
    set(&mut h, json!({"id": "serve", "capacity": 0}));
    h.state_mut().run_command(&CommandRef::new(commands::RUN));
    settle(&mut h);
    assert!(!h.state().is_running());
    assert_eq!(shown(&h).as_deref(), Some("serve"));

    // A warning (a normal time often below 0) does not stop a run.
    set(
        &mut h,
        json!({"id": "serve", "capacity": 1, "service": {"type": "normal", "mean": 0.1, "std_dev": 1.0}}),
    );
    assert!(h.query_by_label_contains("problem").is_none());
    h.get_by_role_and_label(Role::Button, "Run").click();
    settle(&mut h);
    assert!(h.state().is_running());
}

#[test]
fn the_problems_chip_names_the_blocks_and_shows_them() {
    let mut h = mm1();
    assert!(h.query_by_label_contains("1 problem:").is_none());
    set(&mut h, json!({"id": "serve", "capacity": 0}));
    assert_eq!(shown(&h), None);
    h.get_by_role_and_label(Role::Button, "1 problem: Serve customer")
        .click();
    settle(&mut h);
    assert_eq!(shown(&h).as_deref(), Some("serve"));
    assert_eq!(focused(&h).as_deref(), Some("Capacity"));
}

#[test]
fn a_status_bar_error_shows_its_block_when_clicked() {
    let mut h = mm1();
    set(&mut h, json!({"id": "serve", "capacity": 0}));
    h.state_mut().run();
    settle(&mut h);
    // Back to the model page, then the status bar's error leads to the block again.
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    assert_eq!(shown(&h), None);
    h.get_by_role_and_label(
        Role::Button,
        "Serve customer would never have a unit to work with. Give it at least 1 unit.",
    )
    .click();
    settle(&mut h);
    assert_eq!(shown(&h).as_deref(), Some("serve"));
}

/// The Name field's place in the inspector of the selected block.
fn name_field(h: &Harness<'static, App>) -> egui::Rect {
    h.get_by_role_and_label(Role::TextInput, "Name").rect()
}

#[test]
fn the_inspector_keeps_its_place_when_a_problem_appears() {
    let mut h = mm1();
    h.get_by_role_and_label(Role::ListBoxOption, "Serve customer")
        .click();
    settle(&mut h);
    let before = name_field(&h);
    set(&mut h, json!({"id": "serve", "capacity": 0}));
    h.get_by_label("Serve customer would never have a unit to work with. Give it at least 1 unit.");
    assert_eq!(name_field(&h), before);
    set(&mut h, json!({"id": "serve", "capacity": 1}));
    assert!(
        h.query_by_label(
            "Serve customer would never have a unit to work with. Give it at least 1 unit."
        )
        .is_none()
    );
    assert_eq!(name_field(&h), before);
}

/// The description of the folded section `title`: what is set in it, in a line.
fn summary(h: &Harness<'static, App>, title: &str) -> String {
    h.get_by_role_and_label(Role::DisclosureTriangle, title)
        .accesskit_node()
        .description()
        .unwrap_or_default()
}

#[test]
fn the_basics_come_first_and_a_folded_section_says_what_is_set() {
    let mut h = mm1();
    h.get_by_role_and_label(Role::ListBoxOption, "Serve customer")
        .click();
    settle(&mut h);
    // The basics are on show; the rest folded, closed with nothing set in it.
    h.get_by_role_and_label(Role::TextInput, "Capacity");
    h.get_by_role_and_label(Role::ComboBox, "Service time");
    assert!(h.query_by_label("Transfer time").is_none());
    assert_eq!(summary(&h, "More settings"), "None");
    assert_eq!(summary(&h, "Patience"), "None");

    // Set, it starts open; closed, it says what is set.
    set(
        &mut h,
        json!({"id": "serve", "transfer": {"type": "constant", "value": 2.0}}),
    );
    let unit =
        stochcraft_engine::wire::model(h.state().engine().model())["experiment"]["time_unit"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_default();
    let said = summary(&h, "More settings");
    assert!(
        said.starts_with("Transfer time Constant 2"),
        "{said} ({unit})"
    );
    set(
        &mut h,
        json!({"id": "serve", "breakdown": {
            "time_to_failure": {"type": "exponential", "mean": 120.0},
            "repair": {"type": "constant", "value": 6.0},
        }}),
    );
    let said = summary(&h, "Breakdowns and availability");
    assert!(
        said.starts_with("Breakdowns: Exponential 120") && said.contains(", Constant 6"),
        "{said}"
    );
}

#[test]
fn the_question_mark_opens_its_explanation_from_the_keyboard() {
    let mut h = mm1();
    h.get_by_role_and_label(Role::ListBoxOption, "Serve customer")
        .click();
    settle(&mut h);
    let says = "How many units serve here at the same time, such as machines or staff, each one \
                entity at a time.";
    assert!(h.query_by_label(says).is_none());
    h.get_by_role_and_label(Role::Button, "Capacity ?").focus();
    settle(&mut h);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    h.get_by_label(says);
    assert_eq!(
        h.get_by_role_and_label(Role::Button, "Capacity ?")
            .accesskit_node()
            .data()
            .is_expanded(),
        Some(true)
    );
    // Again, and it closes.
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert!(h.query_by_label(says).is_none());
    // Service time and the waiting room have one too.
    h.get_by_role_and_label(Role::Button, "Service time ?");
    h.get_by_role_and_label(Role::Button, "Waiting room ?");
}

#[test]
fn run_reaches_a_blocks_error_after_the_models_own() {
    let mut h = mm1();
    h.state_mut()
        .execute("experiment.set", json!({"replications": 0}))
        .expect("no runs");
    set(&mut h, json!({"id": "serve", "capacity": 0}));
    let mut seen = Vec::new();
    for _ in 0..3 {
        h.get_by_role_and_label(Role::Button, "Run (2 problems)")
            .click();
        settle(&mut h);
        seen.push(shown(&h));
    }
    assert!(seen.contains(&None), "{seen:?}");
    assert!(seen.contains(&Some("serve".to_owned())), "{seen:?}");
    // The model page's field, when that error is shown.
    if seen[0].is_none() {
        assert_eq!(seen[1].as_deref(), Some("serve"));
    }
}
