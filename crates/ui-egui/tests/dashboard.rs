//! The Dashboard panel (M5.4): four tiles beside the model, filled by a run.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The M/M/1 example, open, with the interface in English.
fn mm1() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 1000.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h
}

fn menu(h: &mut Harness<'static, App>, path: &[&str]) {
    let item = egui::accesskit::Role::MenuItem;
    h.get_by_role_and_label(item, path[0]).click();
    settle(h);
    for label in &path[1..] {
        h.get_by_role_and_label(item, label).click();
        settle(h);
    }
}

fn finish_run(h: &mut Harness<'static, App>) {
    h.step();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    settle(h);
}

#[test]
fn before_a_run_the_tiles_say_what_they_will_show() {
    let mut h = mm1();
    menu(&mut h, &["View", "Dashboard"]);
    assert!(
        h.query_all_by_label_contains("Run the model to fill the dashboard")
            .count()
            >= 1
    );
}

#[test]
fn a_run_fills_the_throughput_and_utilisation_tiles_and_a_model_without_conveyors_says_so() {
    let mut h = mm1();
    menu(&mut h, &["View", "Dashboard"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    assert!(h.state().results().is_some());
    // Throughput: a number per hour.
    let per_hour: Vec<String> = h
        .query_all_by_label_contains("per hour")
        .filter_map(|n| n.accesskit_node().label())
        .collect();
    assert!(
        per_hour
            .iter()
            .any(|l| l.chars().next().is_some_and(|c| c.is_ascii_digit())),
        "{per_hour:?}"
    );
    // Utilisation: the server's bar, with its busy share.
    assert!(
        h.query_all_by_label_contains("Serve customer: ").count() >= 1,
        "no bar for the server"
    );
    // Conveyors: the model has none.
    assert!(
        h.query_all_by_label_contains("No conveyors in this model")
            .count()
            >= 1
    );
}
