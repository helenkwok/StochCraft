//! The Capacity view: the table before any run, its target, and the simulated values beside the
//! formulas once there are results of this model.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use stochcraft_ui_egui::{App, Platform};

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The M/M/1 model (arrivals 0.75, service 1), short so a run is quick, on the Capacity view.
fn capacity() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.state_mut()
        .open_file("mm1.bpmn", include_bytes!("../../../tests/models/mm1.bpmn"));
    h.state_mut().execute(
        "experiment.set",
        json!({ "replications": 6, "duration": 600.0, "warmup": 0.0 }),
    );
    settle(&mut h);
    click(&mut h, Role::Tab, "Capacity");
    h
}

fn click(h: &mut Harness<'static, App>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    settle(h);
}

fn has(h: &Harness<'static, App>, label: &str) -> bool {
    !h.query_all_by_label(label).collect::<Vec<_>>().is_empty()
}

fn type_target(h: &mut Harness<'static, App>, text: &str) {
    h.get_by_role_and_label(Role::TextInput, "Target utilisation")
        .click();
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, "Target utilisation")
        .type_text(text);
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

fn run(h: &mut Harness<'static, App>) {
    h.state_mut().run();
    for _ in 0..2000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(h);
    assert!(
        h.state().status().starts_with("Finished"),
        "{}",
        h.state().status()
    );
}

#[test]
fn the_capacity_table_shows_the_formula_before_any_run() {
    let h = capacity();
    h.get_by_label("Station");
    assert!(has(&h, "Serve customer"));
    // Utilisation 75%, fine, M/M/1 and a mean wait of 3 (0.75 ÷ (1 − 0.75) × 1).
    for label in ["75%", "Fine", "M/M/1", "3"] {
        assert!(has(&h, label), "no {label:?} in the table");
    }
    // No run yet, so nothing simulated.
    assert!(has(&h, "–"));
    assert_eq!(h.query_all_by_label_contains("±").count(), 0);
    h.get_by_label_contains("Bottleneck: Serve customer (load 75%)");
}

#[test]
fn the_simulated_mean_wait_sits_under_its_header() {
    let mut h = capacity();
    // A frame more, so the grid's columns have their measured widths.
    h.run_steps(2);
    let header = h.get_by_label("Simulated mean wait").rect();
    // The last "–" in the row is the simulated mean wait's (no run yet).
    let dash = h
        .query_all_by_label("–")
        .map(|n| n.rect())
        .max_by(|a, b| a.right().total_cmp(&b.right()))
        .expect("a dash in the simulated columns");
    assert!(
        dash.right() <= header.right() + 1.0 && dash.left() >= header.left() - 1.0,
        "the dash at {dash:?} is not under its header at {header:?}"
    );
}

#[test]
fn a_target_outside_zero_to_a_hundred_shows_its_error() {
    let mut h = capacity();
    h.get_by_label("Units for 85%");
    type_target(&mut h, "150");
    h.get_by_label_contains("Must be above 0% and below 100%");
    // The table stays as it was for the last good target.
    h.get_by_label("Units for 85%");
    type_target(&mut h, "0");
    h.get_by_label_contains("Must be above 0% and below 100%");
    type_target(&mut h, "60");
    assert!(
        h.query_by_label_contains("Must be above 0% and below 100%")
            .is_none()
    );
    h.get_by_label("Units for 60%");
}

#[test]
fn the_simulated_values_fill_after_a_run_and_clear_after_an_edit() {
    let mut h = capacity();
    run(&mut h);
    click(&mut h, Role::Tab, "Capacity");
    assert_eq!(
        h.query_all_by_label_contains("±").count(),
        2,
        "the simulated utilisation and mean wait"
    );
    // An edit makes the results out of date: the simulated cells show "–" again.
    h.state_mut()
        .execute("experiment.set", json!({ "replications": 7 }));
    settle(&mut h);
    assert!(h.state().results_out_of_date());
    assert_eq!(h.query_all_by_label_contains("±").count(), 0);
}
