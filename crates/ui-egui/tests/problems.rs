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
    assert_eq!(h.state().status(), "a server needs at least 1 unit");

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
    h.get_by_role_and_label(Role::Button, "a server needs at least 1 unit")
        .click();
    settle(&mut h);
    assert_eq!(shown(&h).as_deref(), Some("serve"));
}
