//! Chart palettes on the Appearance menu and in the Chart colours window, and the preferences that
//! keep them (egui_kittest, through the accessibility tree; nothing is rendered).

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::Color32;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use stochcraft_ui_egui::chart_palette::{self, ChartPalette, Level, PaletteKind};
use stochcraft_ui_egui::{App, Platform, Prefs};

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

fn open_chart_colours(h: &mut Harness<'static, App>) {
    menu_item(h, "View");
    menu_item(h, "Appearance");
    menu_item(h, "Chart colours…");
}

fn button(h: &mut Harness<'static, App>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    settle(h);
}

fn preset(name: &str) -> ChartPalette {
    chart_palette::presets()
        .into_iter()
        .find(|p| p.name == name)
        .expect("a preset")
}

#[test]
fn a_categorical_palette_is_chosen_for_chart_series() {
    let mut h = harness();
    open_chart_colours(&mut h);
    // The window opens on the palette in use: Okabe–Ito, whose first colour is orange.
    assert_eq!(
        chart_palette::series_color(&h.ctx, 0),
        Color32::from_rgb(0xe6, 0x9f, 0x00)
    );
    h.get_by_label("Tol bright").click();
    settle(&mut h);
    button(&mut h, "Use for charts");
    assert_eq!(h.state().prefs().appearance.chart_palette, "Tol bright");
    assert_eq!(
        chart_palette::series_color(&h.ctx, 0),
        Color32::from_rgb(0x44, 0x77, 0xaa)
    );
    // Beyond the last colour the last one is returned: more series should fold into "Other".
    assert_eq!(
        chart_palette::series_color(&h.ctx, 99),
        Color32::from_rgb(0xbb, 0xbb, 0xbb)
    );
}

#[test]
fn a_sequential_palette_cannot_colour_series() {
    let mut h = harness();
    let mut prefs = Prefs::default();
    prefs.appearance.chart_palette = "Viridis".to_owned();
    h.state_mut().set_prefs(&prefs);
    settle(&mut h);
    // The name is kept as saved; the colours fall back to the default categorical palette.
    assert_eq!(h.state().prefs().appearance.chart_palette, "Viridis");
    assert_eq!(
        chart_palette::series_color(&h.ctx, 0),
        Color32::from_rgb(0xe6, 0x9f, 0x00)
    );
}

#[test]
fn a_built_in_palette_duplicates_and_its_copy_is_edited_and_deleted() {
    let mut h = harness();
    open_chart_colours(&mut h);
    h.get_by_label("Okabe–Ito").click();
    settle(&mut h);
    // Built-in palettes have no Delete, Rename or colour editing: Duplicate is there.
    assert_eq!(
        h.query_all_by_role_and_label(Role::Button, "Delete")
            .count(),
        0
    );
    button(&mut h, "Duplicate");
    let custom = h.state().prefs().appearance.custom_palettes;
    assert_eq!(custom.len(), 1);
    assert_eq!(custom[0].name, "Okabe–Ito copy");
    assert_eq!(custom[0].kind, PaletteKind::Categorical);
    assert_eq!(custom[0].colors, preset("Okabe–Ito").colors);

    // The copy is selected: add a colour, then take it back off.
    button(&mut h, "Add colour");
    assert_eq!(
        h.state().prefs().appearance.custom_palettes[0].colors.len(),
        9
    );
    button(&mut h, "Remove colour");
    assert_eq!(
        h.state().prefs().appearance.custom_palettes[0].colors.len(),
        8
    );

    // Still in use is the built-in one, so deleting the copy leaves the choice alone.
    button(&mut h, "Delete");
    let after = h.state().prefs().appearance;
    assert!(after.custom_palettes.is_empty());
    assert_eq!(after.chart_palette, "Okabe–Ito");
}

#[test]
fn deleting_the_palette_in_use_goes_back_to_the_default() {
    let mut h = harness();
    open_chart_colours(&mut h);
    button(&mut h, "Duplicate");
    button(&mut h, "Use for charts");
    assert_eq!(h.state().prefs().appearance.chart_palette, "Okabe–Ito copy");
    button(&mut h, "Delete");
    let after = h.state().prefs().appearance;
    assert!(after.custom_palettes.is_empty());
    assert_eq!(after.chart_palette, chart_palette::DEFAULT);
}

#[test]
fn a_palette_saved_as_hex_text_comes_back_unchanged() {
    let mut h = harness();
    let mut prefs = Prefs::default();
    prefs.appearance.chart_palette = "Mine".to_owned();
    prefs.appearance.custom_palettes = vec![ChartPalette {
        name: "Mine".to_owned(),
        kind: PaletteKind::Categorical,
        colors: vec![
            Color32::from_rgb(0x12, 0x34, 0x56),
            Color32::from_rgb(0xab, 0xcd, 0xef),
        ],
    }];
    h.state_mut().set_prefs(&prefs);
    settle(&mut h);
    assert_eq!(h.state().prefs(), prefs);

    let text = serde_json::to_string(&prefs).expect("prefs as JSON");
    assert!(text.contains("\"#123456\""), "{text}");
    assert!(text.contains("\"categorical\""), "{text}");
    let back: Prefs = serde_json::from_str(&text).expect("prefs from JSON");
    assert_eq!(back, prefs);
}

#[test]
fn preferences_from_before_chart_palettes_still_load() {
    let old: Prefs = serde_json::from_str(r#"{"appearance":{"theme":"dark"}}"#).expect("old");
    assert_eq!(old.appearance.chart_palette, chart_palette::DEFAULT);
    assert!(old.appearance.custom_palettes.is_empty());
}

#[test]
fn an_unknown_chart_palette_name_shows_the_default() {
    let mut h = harness();
    let mut prefs = Prefs::default();
    prefs.appearance.chart_palette = "Gone".to_owned();
    h.state_mut().set_prefs(&prefs);
    settle(&mut h);
    assert_eq!(
        chart_palette::series_color(&h.ctx, 0),
        Color32::from_rgb(0xe6, 0x9f, 0x00)
    );
    // The name is kept as it was saved, as an unknown colour theme's is.
    assert_eq!(h.state().prefs().appearance.chart_palette, "Gone");
}

#[test]
fn the_black_slot_of_okabe_ito_fails_on_the_dark_canvas_only() {
    // On white the black slot (8) is fine; on a dark canvas it is invisible.
    let okabe = preset(chart_palette::DEFAULT);
    let light = chart_palette::check(&okabe, Color32::WHITE);
    assert!(light.iter().all(|f| !f.slots.contains(&7)), "{light:?}");
    let dark = chart_palette::check(&okabe, Color32::from_rgb(0x14, 0x17, 0x1b));
    assert!(
        dark.iter()
            .any(|f| f.level == Level::Fail && f.slots == vec![7]),
        "{dark:?}"
    );
}
