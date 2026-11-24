//! The 3D view: its tab shows the model, the Fit button and the keyboard work, and a watched run
//! draws without trouble. What is drawn is checked by the unit tests in `view3d.rs`.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, Platform};

const VIEW: &str = "3D view of the model";

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The M/M/1 model, open on the Model tab.
fn mm1() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.state_mut()
        .open_file("mm1.bpmn", include_bytes!("../../../tests/models/mm1.bpmn"));
    settle(&mut h);
    h
}

fn click(h: &mut Harness<'static, App>, role: Role, label: &str) {
    h.get_by_role_and_label(role, label).click();
    settle(h);
}

fn has(h: &Harness<'static, App>, label: &str) -> bool {
    !h.query_all_by_label(label).collect::<Vec<_>>().is_empty()
}

#[test]
fn the_3d_tab_shows_the_model_and_fit_works() {
    let mut h = mm1();
    assert!(!has(&h, VIEW), "the view is on its own tab");
    click(&mut h, Role::Tab, "3D");
    assert!(has(&h, VIEW));
    click(&mut h, Role::Button, "Fit");
    assert!(has(&h, VIEW));
    // Back to the model: the flowchart is there again and the 3D view is not.
    click(&mut h, Role::Tab, "Model");
    assert!(!has(&h, VIEW));
}

#[test]
fn the_keyboard_orbits_and_zooms_the_focused_view() {
    let mut h = mm1();
    click(&mut h, Role::Tab, "3D");
    h.get_by_label(VIEW).click();
    settle(&mut h);
    for key in [
        egui::Key::ArrowLeft,
        egui::Key::ArrowRight,
        egui::Key::ArrowUp,
        egui::Key::ArrowDown,
        egui::Key::Plus,
        egui::Key::Minus,
    ] {
        h.key_press(key);
        settle(&mut h);
    }
    assert!(has(&h, VIEW));
}

#[test]
fn a_watched_run_draws_in_the_3d_view() {
    let mut h = mm1();
    h.state_mut().run_command(&CommandRef::new(commands::WATCH));
    h.run_steps(3);
    click(&mut h, Role::Tab, "3D");
    for _ in 0..30 {
        h.step();
    }
    assert!(has(&h, VIEW));
    h.state_mut()
        .run_command(&CommandRef::new(commands::STOP_WATCHING));
    settle(&mut h);
    assert!(has(&h, VIEW));
}

/// The parcel sorter, drawn from its layout: its 3D view shows, and a watched run draws the
/// parcels on its belts without trouble.
#[test]
fn the_sorter_draws_from_its_layout_while_a_run_is_watched() {
    let mut h = mm1();
    h.state_mut().open_file(
        "sorter.bpmn",
        include_bytes!("../../../tests/models/sorter.bpmn"),
    );
    settle(&mut h);
    click(&mut h, Role::Tab, "3D");
    assert!(has(&h, VIEW));
    h.state_mut().run_command(&CommandRef::new(commands::WATCH));
    for _ in 0..60 {
        h.step();
    }
    assert!(has(&h, VIEW));
    click(&mut h, Role::Button, "Fit");
    assert!(has(&h, VIEW));
    h.state_mut()
        .run_command(&CommandRef::new(commands::STOP_WATCHING));
    settle(&mut h);
    assert!(has(&h, VIEW));
}
