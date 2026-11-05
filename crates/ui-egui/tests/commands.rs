//! Menus and shortcuts on craft-menu and craft-keymap: the menu bar's commands, the keymap's
//! default shortcuts, recent files, the appearance choice and the preferences a shell keeps.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, EXAMPLES, Platform, Prefs, Request, ThemeChoice};

/// The app with English numbers (`1,234.5`), so the tests read the same in every region.
fn english(mut app: App) -> App {
    app.set_number_region("en");
    app
}

fn harness(platform: Platform) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), english(App::new(platform)));
    h.run();
    h
}

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

fn menu_item(h: &mut Harness<'static, App>, label: &str) {
    h.get_all_by_role_and_label(Role::MenuItem, label)
        .last()
        .unwrap_or_else(|| panic!("no menu item {label:?}"))
        .click();
    settle(h);
}

#[test]
fn the_menu_bar_runs_commands() {
    let mut h = harness(Platform::DESKTOP);
    menu_item(&mut h, "File");
    menu_item(&mut h, "Open Example");
    menu_item(&mut h, "M/M/1 queue");
    assert!(
        h.state().status().starts_with("Opened mm1.bpmn"),
        "{}",
        h.state().status()
    );
    menu_item(&mut h, "Help");
    menu_item(&mut h, "About StochCraft");
    h.get_by_label_contains("MIT License or the Apache License 2.0");
}

#[test]
fn shortcuts_come_from_the_keymap() {
    let mut h = harness(Platform::DESKTOP);
    let ctx = h.ctx.clone();
    assert_eq!(
        h.state().shortcut(commands::RUN, &ctx).as_deref(),
        Some("F5")
    );
    assert_eq!(
        h.state().shortcut(commands::OPEN, &ctx).as_deref(),
        Some("Ctrl+O")
    );
    // Ctrl+O asks the shell for a file.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
    settle(&mut h);
    assert_eq!(h.state_mut().take_requests(), vec![Request::PickOpen]);
}

#[test]
fn opened_and_saved_files_are_recent_files() {
    let mut h = harness(Platform::DESKTOP);
    let e = &EXAMPLES[0];
    h.state_mut().open_path("/models/a.bpmn", e.bytes);
    h.state_mut().open_path("/models/b.bpmn", e.bytes);
    h.state_mut().open_path("/models/a.bpmn", e.bytes);
    settle(&mut h);
    assert_eq!(
        h.state().prefs().files.recent,
        ["/models/a.bpmn", "/models/b.bpmn"],
        "newest first, once each"
    );
    // File ▸ Open Recent ▸ b.bpmn asks the shell to read it.
    h.state_mut().run_command(&CommandRef::with_params(
        commands::OPEN_RECENT,
        serde_json::json!({ "index": 1 }),
    ));
    assert_eq!(
        h.state_mut().take_requests(),
        vec![Request::OpenPath {
            path: "/models/b.bpmn".into()
        }]
    );
    h.state_mut()
        .run_command(&CommandRef::new(commands::CLEAR_RECENT));
    assert!(h.state().prefs().files.recent.is_empty());
}

#[test]
fn the_browser_keeps_no_recent_files_and_no_quit() {
    let mut h = harness(Platform::BROWSER);
    h.state_mut().open_path("model.bpmn", EXAMPLES[0].bytes);
    assert!(h.state().prefs().files.recent.is_empty());
    menu_item(&mut h, "File");
    assert!(h.query_by_label("Open Recent").is_none());
    assert!(h.query_by_label("Quit").is_none());
}

#[test]
fn the_appearance_is_chosen_from_the_menu_and_remembered() {
    let mut h = harness(Platform::DESKTOP);
    menu_item(&mut h, "View");
    menu_item(&mut h, "Appearance");
    // A radio item: one choice of the appearances.
    h.get_by_role_and_label(Role::MenuItemRadio, "Dark").click();
    settle(&mut h);
    assert_eq!(h.state().prefs().appearance.theme, ThemeChoice::Dark);
    assert_eq!(h.ctx.theme(), egui::Theme::Dark);

    let mut prefs = Prefs::default();
    prefs.appearance.theme = ThemeChoice::Light;
    prefs.files.recent = vec!["/x.bpmn".into()];
    h.state_mut().set_prefs(&prefs);
    settle(&mut h);
    assert_eq!(h.state().prefs(), prefs);
    assert_eq!(h.ctx.theme(), egui::Theme::Light);
}
