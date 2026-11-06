//! Every word the interface shows goes through translation. In craft-i18n's pseudo locale each
//! looked-up string comes back marked `⟦…⟧`, so any label without the marks was drawn without a
//! lookup, unless it is data: the model's own names and ids, numbers, file names, shortcuts.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use std::collections::BTreeSet;

use craft_i18n::Lang;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

fn pseudo() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(Lang::PSEUDO);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    h
}

/// Messages the engine writes (file notes, validation), which are English until the engine's own
/// strings have a catalog.
const ENGINE: &[&str] = &["SetUpTime is not simulated yet and is ignored"];

/// Text that is the model's or the user's, not the interface's: names and ids from the model,
/// numbers and units, file names, shortcuts; and egui's own ("Resize panel").
fn is_data(label: &str, model_words: &BTreeSet<String>) -> bool {
    let words: String = label
        .chars()
        .filter(|c| c.is_alphabetic())
        .collect::<String>();
    words.is_empty()
        || model_words.contains(label)
        || label.ends_with(".bpmn")
        || label.ends_with(".xpdl")
        || label.starts_with("https://")
        // A shortcut ("Ctrl+O", "Cmd+Shift+S").
        || (label.contains('+') && !label.contains(' '))
        // The brand, key names and time units ("min" after a number).
        || ["StochCraft", "Esc", "F5", "min"].contains(&label)
        // egui's own labels, and the version line of About.
        || ["Resize panel", "Close window"].contains(&label)
        || label.strip_prefix("StochCraft ").is_some_and(|v| !v.contains(' '))
        || ENGINE.contains(&label)
}

/// The labels on screen that skipped translation.
fn untranslated(h: &Harness<'static, App>) -> BTreeSet<String> {
    let model = stochcraft_engine::wire::model(h.state().engine().model());
    let mut model_words: BTreeSet<String> = BTreeSet::new();
    for key in ["elements", "connections"] {
        for e in model[key].as_array().into_iter().flatten() {
            for f in ["id", "name"] {
                if let Some(s) = e[f].as_str() {
                    model_words.insert(s.to_owned());
                }
            }
        }
    }
    if let Some(n) = model["name"].as_str() {
        model_words.insert(n.to_owned());
    }
    h.query_all(By::new())
        .filter_map(|n| {
            let a = n.accesskit_node();
            let text = a.label().or_else(|| a.value())?;
            let text = text.trim().to_owned();
            // A wrapped label's later lines carry only the closing mark.
            let marked = text.contains('⟦') || text.contains('⟧');
            (!text.is_empty() && !marked && !is_data(&text, &model_words)).then_some(text)
        })
        .collect()
}

#[test]
fn every_screen_is_translated() {
    let mut h = pseudo();
    let mut missed = BTreeSet::new();
    // The welcome screen.
    missed.extend(untranslated(&h));

    // A model: the overview, then a block's inspector.
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    missed.extend(untranslated(&h));
    h.get_all_by_role(Role::ListBoxOption)
        .nth(1)
        .expect("a second block")
        .click();
    settle(&mut h);
    missed.extend(untranslated(&h));

    // Results.
    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    missed.extend(untranslated(&h));

    // About, and the question before unsaved changes are replaced.
    h.state_mut().run_command(&CommandRef::new(commands::ABOUT));
    settle(&mut h);
    missed.extend(untranslated(&h));
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    h.state_mut().execute(
        "block.set",
        serde_json::json!({"id": "serve", "capacity": 2}),
    );
    h.state_mut().run_command(&CommandRef::new(commands::NEW));
    settle(&mut h);
    h.get_by_label_contains("Don't Save");
    missed.extend(untranslated(&h));

    assert!(missed.is_empty(), "drawn without translation: {missed:#?}");
}

/// The menus (File, Edit, View, Model, Help) and what the layout adds.
#[test]
fn the_menus_are_translated() {
    let mut app = App::new(Platform::DESKTOP);
    app.set_lang(Lang::PSEUDO);
    let layout = app.menu_layout(craft_menu::Target::Linux);
    let mut missed = Vec::new();
    for menu in &layout.bar.menus {
        if !menu.title.contains('⟦') {
            missed.push(menu.title.clone());
        }
        visit(&menu.children, &mut missed);
    }
    // View ▸ Language names each language in its own language, on purpose.
    let names: Vec<String> = stochcraft_ui_egui::i18n::languages()
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    missed.retain(|l| !names.contains(l));
    assert!(
        missed.is_empty(),
        "menu labels without translation: {missed:?}"
    );
}

fn visit(nodes: &[craft_menu::Node], missed: &mut Vec<String>) {
    for n in nodes {
        match n {
            craft_menu::Node::Item(i) if !i.label.contains('⟦') => missed.push(i.label.clone()),
            craft_menu::Node::Submenu {
                label, children, ..
            } => {
                if !label.contains('⟦') {
                    missed.push(label.clone());
                }
                visit(children, missed);
            }
            _ => {}
        }
    }
}

/// A language this build doesn't embed shows English (or the system's) but stays the user's
/// choice, so a build that has it shows it again.
#[test]
fn a_chosen_language_is_remembered_even_when_not_built_in() {
    let mut app = App::new(Platform::DESKTOP);
    app.run_command(&CommandRef::with_params(
        commands::LANGUAGE,
        serde_json::json!("xx"),
    ));
    assert_eq!(app.prefs().appearance.language.as_deref(), Some("xx"));
    let mut again = App::new(Platform::DESKTOP);
    again.set_prefs(&app.prefs());
    assert_eq!(again.prefs().appearance.language.as_deref(), Some("xx"));
    again.run_command(&CommandRef::with_params(
        commands::LANGUAGE,
        serde_json::json!("auto"),
    ));
    assert_eq!(again.prefs().appearance.language, None);
}

/// StochCraft's own catalogs are built in: every left-to-right language craft-i18n has, each
/// complete, and choosing one changes the menus and the window at once.
#[test]
fn the_built_in_languages_translate_the_window() {
    let names: Vec<String> = stochcraft_ui_egui::i18n::languages()
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    assert_eq!(names.len(), 45, "English and 44 languages: {names:?}");
    assert!(names.iter().any(|n| n == "Deutsch") && names.iter().any(|n| n == "日本語"));

    let mut app = App::new(Platform::DESKTOP);
    app.set_language(Some("de"));
    let layout = app.menu_layout(craft_menu::Target::Linux);
    let titles: Vec<&str> = layout.bar.menus.iter().map(|m| m.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Datei", "Bearbeiten", "Ansicht", "Modell", "Hilfe"]
    );
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    h.get_by_label("Öffnen Sie ein Modell, um es zu simulieren");
}
