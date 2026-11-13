//! Colour themes on the Appearance menu and in the Themes window, and the preferences that keep
//! them (egui_kittest, through the accessibility tree; nothing is rendered).

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use stochcraft_ui_egui::theme::{self, Theme};
use stochcraft_ui_egui::{App, Platform, Prefs, ThemeChoice};

/// The app with English numbers and text, so the tests read the same in every region.
fn english(mut app: App) -> App {
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    app
}

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .build_ui_state(
            |ui, app: &mut App| app.ui(ui),
            english(App::new(Platform::DESKTOP)),
        );
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

fn open_appearance(h: &mut Harness<'static, App>) {
    menu_item(h, "View");
    menu_item(h, "Appearance");
}

fn open_themes(h: &mut Harness<'static, App>) {
    open_appearance(h);
    menu_item(h, "Edit themes…");
}

fn built_in(name: &str) -> Theme {
    theme::presets()
        .into_iter()
        .find(|t| t.name == name)
        .expect("a built-in theme")
}

#[test]
fn a_built_in_colour_theme_is_chosen_from_the_appearance_menu() {
    let mut h = harness();
    open_appearance(&mut h);
    h.get_by_role_and_label(Role::MenuItemRadio, "Warm").click();
    settle(&mut h);
    let prefs = h.state().prefs();
    assert_eq!(prefs.appearance.colors, "Warm");
    // The light or dark mode is a separate choice, and it stays as it was.
    assert_eq!(prefs.appearance.theme, ThemeChoice::System);
    let warm = built_in("Warm");
    assert_eq!(
        h.ctx.style_of(egui::Theme::Light).visuals.panel_fill,
        warm.light.canvas
    );
    assert_eq!(
        h.ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
        warm.dark.canvas
    );
    // The palettes the widgets draw with are the theme's, in each mode.
    h.ctx.set_theme(egui::ThemePreference::Light);
    assert_eq!(theme::palette(&h.ctx), warm.light);
    h.ctx.set_theme(egui::ThemePreference::Dark);
    assert_eq!(theme::palette(&h.ctx), warm.dark);
}

#[test]
fn a_built_in_theme_duplicates_into_an_editable_copy() {
    let mut h = harness();
    open_themes(&mut h);
    // StochCraft is read-only: no Delete for it, and Duplicate is there.
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Delete")
            .count(),
        0
    );
    h.get_by_role_and_label(Role::Button, "Duplicate").click();
    settle(&mut h);
    let custom = h.state().prefs().appearance.custom_themes;
    assert_eq!(custom.len(), 1);
    assert_eq!(custom[0].name, "StochCraft copy");
    assert_eq!(custom[0].light, theme::default_theme().light);
    assert_eq!(custom[0].dark, theme::default_theme().dark);
    // Nothing was changed, so the theme in use is still StochCraft.
    assert_eq!(h.state().prefs().appearance.colors, "StochCraft");
}

#[test]
fn a_custom_theme_in_use_goes_back_to_stochcraft_when_deleted() {
    let mut h = harness();
    let mut prefs = Prefs::default();
    prefs.appearance.custom_themes = vec![Theme {
        name: "Mine".to_owned(),
        ..theme::default_theme()
    }];
    prefs.appearance.colors = "Mine".to_owned();
    h.state_mut().set_prefs(&prefs);
    settle(&mut h);
    // The window opens on the theme in use.
    open_themes(&mut h);
    h.get_by_role_and_label(Role::Button, "Delete").click();
    settle(&mut h);
    let after = h.state().prefs().appearance;
    assert!(after.custom_themes.is_empty());
    assert_eq!(after.colors, "StochCraft");
}

#[test]
fn colour_themes_are_remembered_as_hex_text_and_old_preferences_still_load() {
    let mut h = harness();
    let mut prefs = Prefs::default();
    prefs.appearance.colors = "Warm".to_owned();
    prefs.appearance.custom_themes = vec![Theme {
        name: "Mine".to_owned(),
        ..theme::default_theme()
    }];
    h.state_mut().set_prefs(&prefs);
    settle(&mut h);
    assert_eq!(h.state().prefs(), prefs);

    let text = serde_json::to_string(&prefs).expect("prefs as JSON");
    assert!(text.contains("\"#17222e\""), "{text}");
    let back: Prefs = serde_json::from_str(&text).expect("prefs from JSON");
    assert_eq!(back, prefs);

    // Preferences written before colour themes existed.
    let old: Prefs = serde_json::from_str(r#"{"appearance":{"theme":"dark"}}"#).expect("old");
    assert_eq!(old.appearance.theme, ThemeChoice::Dark);
    assert_eq!(old.appearance.colors, "StochCraft");
    assert!(old.appearance.custom_themes.is_empty());
}

#[test]
fn an_unknown_colour_theme_name_shows_stochcraft() {
    let mut h = harness();
    let mut prefs = Prefs::default();
    prefs.appearance.colors = "Gone".to_owned();
    h.state_mut().set_prefs(&prefs);
    settle(&mut h);
    assert_eq!(
        h.ctx.style_of(egui::Theme::Light).visuals.panel_fill,
        theme::default_theme().light.canvas
    );
    // The name is kept as it was saved, as an unknown language code is.
    assert_eq!(h.state().prefs().appearance.colors, "Gone");
}
