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
    pseudo_sized(egui::vec2(1200.0, 900.0))
}

fn pseudo_sized(size: egui::Vec2) -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(Lang::PSEUDO);
    let mut h = Harness::builder()
        .with_size(size)
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
        // The brand and key names.
        || ["StochCraft", "Esc", "F5"].contains(&label)
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

/// Every string the interface looks up has a row in every one of StochCraft's catalogs, so no
/// language shows English beside its own words. The strings come from two places: the source
/// text passed straight to `t`, `tf`, `tc`, `tn` and `tnf` (read from the code of the interface
/// and of the shells), and every string looked up in the pseudo locale while the screens below
/// are drawn (which catches labels kept in tables: menu items, field names, block kinds). A table
/// entry that none of these screens draws (an undo label other than the one made here, a rare
/// distribution's field) is not checked.
///
/// Only the catalogs in `i18n/` count, not craft-i18n's shared `common.tsv`, which is embedded
/// only when `CRAFT_I18N_DIR` is set at build time (never in CI or the published web app).
#[test]
fn every_string_has_a_row_in_every_catalog() {
    let mut wanted = in_the_code();
    for s in looked_up_on_screen() {
        if !is_shared(&s) && !wanted.contains(&Key::Plain(s.clone())) {
            wanted.insert(Key::Seen(s));
        }
    }
    assert!(
        wanted.len() > 200,
        "too few strings found: {}",
        wanted.len()
    );

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../i18n");
    let mut catalogs: Vec<_> = std::fs::read_dir(&dir)
        .expect("i18n/")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "tsv"))
        .collect();
    catalogs.sort();
    assert_eq!(catalogs.len(), 44, "{catalogs:?}");
    let mut report = String::new();
    for path in &catalogs {
        let text = std::fs::read_to_string(path).expect("a catalog");
        let rows = rows(&text);
        let missing: Vec<String> = wanted
            .iter()
            .filter(|k| !k.found_in(&rows))
            .map(Key::to_string)
            .collect();
        if !missing.is_empty() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            report += &format!(
                "\n{} lacks {}:\n  {}",
                name.unwrap_or_default(),
                missing.len(),
                missing.join("\n  ")
            );
        }
    }
    assert!(report.is_empty(), "strings without a translation:{report}");
}

/// Looked-up text that is data, not words to translate: library names and licences in About.
const NOT_TEXT: &[&str] = &[
    "craft-libs",
    "serde, serde_json",
    "libm",
    "sha2",
    "MIT OR Apache-2.0",
];

/// Whether a string looked up on screen is left to craft-i18n's shared catalog: craft-keymap's
/// shortcuts dialog draws its own words (`craft_keymap::ui::STRINGS`, for the shared catalog,
/// not the app's) and asks for key caps by name ("N", "F5", under context `key`). StochCraft's
/// own uses of the same words are still checked, as they come from its code.
fn is_shared(s: &str) -> bool {
    let key_cap = s.len() == 1
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        || s.strip_prefix('F')
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    NOT_TEXT.contains(&s) || key_cap || craft_keymap::ui::STRINGS.contains(&s)
}

/// A string the interface looks up, as a catalog keys it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    /// `t`, `tf`: a row without context.
    Plain(String),
    /// `tc`: a row with this context, or (as `tc` falls back to it) one without.
    Context(String, String),
    /// `tn`, `tnf`: an `@plural` row of the English `one` and `other`.
    Plural(String, String),
    /// Looked up on screen, with or without a context (the pseudo locale doesn't say which).
    Seen(String),
}

impl Key {
    fn found_in(&self, rows: &BTreeSet<(String, String)>) -> bool {
        let has = |c: &str, s: &str| rows.contains(&(c.to_owned(), s.to_owned()));
        match self {
            Key::Plain(s) => has("", s),
            Key::Context(c, s) => has(c, s) || has("", s),
            Key::Plural(one, other) => has("@plural", &format!("{one}|{other}")),
            Key::Seen(s) => rows.iter().any(|(c, r)| r == s && !c.starts_with('@')),
        }
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Key::Plain(s) | Key::Seen(s) => write!(f, "{s:?}"),
            Key::Context(c, s) => write!(f, "{s:?} (context {c})"),
            Key::Plural(one, other) => write!(f, "{one:?} | {other:?} (plural)"),
        }
    }
}

/// A catalog's (context, English) pairs, unescaped.
fn rows(text: &str) -> BTreeSet<(String, String)> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut cols = l.split('\t');
            let context = cols.next()?;
            let source = cols.next()?;
            Some((context.to_owned(), unescape_tsv(source)))
        })
        .collect()
}

fn unescape_tsv(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The string literals passed straight to the lookup functions in the interface's and the
/// shells' code. A call whose text isn't a literal (a table's entry) is left to
/// [`looked_up_on_screen`].
fn in_the_code() -> BTreeSet<Key> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dirs = [
        root.join("src"),
        root.join("../../apps/stochcraft/src"),
        root.join("../../apps/stochcraft-web/src"),
    ];
    let mut keys = BTreeSet::new();
    for dir in &dirs {
        for file in std::fs::read_dir(dir).expect("a source folder").flatten() {
            let path = file.path();
            if path.extension().is_some_and(|e| e == "rs") {
                let code = std::fs::read_to_string(&path).expect("a source file");
                scan(&code, &mut keys);
            }
        }
    }
    keys
}

/// Finds `t("…")`, `tf("…", …)`, `tc("…", "…")`, `tn(n, "…", "…")` and `tnf(n, "…", "…", …)`
/// (also as `i18n::t(…)`), but not methods such as `I18N.t(…)`.
fn scan(code: &str, keys: &mut BTreeSet<Key>) {
    let b = code.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < b.len() {
        if !ident(b[i]) || (i > 0 && (ident(b[i - 1]) || b[i - 1] == b'.')) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && ident(b[i]) {
            i += 1;
        }
        let name = &code[start..i];
        if b.get(i) != Some(&b'(') || !["t", "tf", "tc", "tn", "tnf"].contains(&name) {
            continue;
        }
        let mut at = i + 1;
        if matches!(name, "tn" | "tnf") {
            // Skip the count.
            let Some(next) = skip_argument(code, at) else {
                continue;
            };
            at = next;
        }
        let Some((first, after)) = literal(code, at) else {
            continue;
        };
        match name {
            "t" | "tf" => {
                keys.insert(Key::Plain(first));
            }
            _ => {
                let Some(next) = after_comma(code, after) else {
                    continue;
                };
                let Some((second, _)) = literal(code, next) else {
                    continue;
                };
                keys.insert(if name == "tc" {
                    Key::Context(first, second)
                } else {
                    Key::Plural(first, second)
                });
            }
        }
    }
}

/// The position after the next top-level comma from `at`.
fn skip_argument(code: &str, at: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (k, c) in code.get(at..)?.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return None,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => return Some(at + k + 1),
            _ => {}
        }
    }
    None
}

/// The position after a comma, skipping white space before it.
fn after_comma(code: &str, at: usize) -> Option<usize> {
    let rest = code.get(at..)?;
    let trimmed = rest.trim_start();
    trimmed
        .strip_prefix(',')
        .map(|_| at + (rest.len() - trimmed.len()) + 1)
}

/// A Rust string literal at `at` (after white space): its value and the position after it.
fn literal(code: &str, at: usize) -> Option<(String, usize)> {
    let rest = code.get(at..)?;
    let lead = rest.len() - rest.trim_start().len();
    let start = at + lead;
    let s = code.get(start..)?;
    if let Some(raw) = s.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let body = raw.get(hashes..)?.strip_prefix('"')?;
        let close = format!("\"{}", "#".repeat(hashes));
        let end = body.find(&close)?;
        return Some((
            body[..end].to_owned(),
            start + 1 + hashes + 1 + end + close.len(),
        ));
    }
    let body = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.char_indices();
    while let Some((k, c)) = chars.next() {
        match c {
            '"' => return Some((out, start + 1 + k + 1)),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '0' => out.push('\0'),
                'u' => {
                    let mut hex = String::new();
                    for (_, h) in chars.by_ref() {
                        match h {
                            '{' => {}
                            '}' => break,
                            _ => hex.push(h),
                        }
                    }
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                // A line continuation: the newline and the next line's indent are dropped.
                '\n' => {
                    while chars.clone().next().is_some_and(|(_, w)| w.is_whitespace()) {
                        chars.next();
                    }
                }
                other => out.push(other),
            },
            _ => out.push(c),
        }
    }
    None
}

/// Everything looked up in the pseudo locale while drawing the welcome screen, every example's
/// overview and each of its blocks, results, About, the keyboard shortcuts and the menus (with an
/// undo step, so Edit names it).
fn looked_up_on_screen() -> BTreeSet<String> {
    // Tall enough that every block in the longest example can be clicked.
    let mut h = pseudo_sized(egui::vec2(1200.0, 2400.0));
    let every_block = |h: &mut Harness<'static, App>| {
        let blocks = h.get_all_by_role(Role::ListBoxOption).count();
        for i in 0..blocks {
            if let Some(b) = h.get_all_by_role(Role::ListBoxOption).nth(i) {
                b.click();
            }
            settle(h);
        }
    };
    for e in EXAMPLES {
        h.state_mut().open_file(e.file, e.bytes);
        settle(&mut h);
        every_block(&mut h);
    }
    // No example has a shared pool yet: add one and a task that needs it, then look at the
    // overview (where pools are edited), the add-block menu and every block again.
    let pool = h.state_mut().execute("resource.add", serde_json::json!({}));
    let pool = pool
        .as_ref()
        .and_then(|r| r["id"].as_str())
        .expect("a pool");
    let pool = pool.to_owned();
    h.state_mut()
        .execute(
            "resource.set",
            serde_json::json!({"id": pool, "roles": ["clerk"]}),
        )
        .expect("roles set");
    h.state_mut()
        .execute(
            "block.add",
            serde_json::json!({
                "kind": "shared_task",
                "service": {"type": "exponential", "mean": 1.0},
                "needs": [{"resource": pool, "quantity": 1}, {"roles": ["clerk"], "quantity": 2}],
                "priority": 1,
            }),
        )
        .expect("a shared task");
    // No example has a calendar, breakdowns or a transfer either: a calendar, a server and a
    // source that use it, and the pool's shifts, so their fields are drawn too.
    h.state_mut()
        .execute(
            "calendar.add",
            serde_json::json!({
                "id": "lunch",
                "events": [{"start": "2026-01-01T12:00:00", "end": "2026-01-01T13:00:00", "repeat": "FREQ=DAILY"}],
            }),
        )
        .expect("a calendar");
    h.state_mut()
        .execute(
            "block.add",
            serde_json::json!({
                "kind": "server",
                "transfer": {"type": "constant", "value": 0.5},
                "breakdown": {
                    "time_to_failure": {"type": "exponential", "mean": 100.0},
                    "repair": {"type": "exponential", "mean": 5.0},
                },
                "availability": [{"calendar": "lunch", "available": false}],
            }),
        )
        .expect("a server with breakdowns");
    h.state_mut()
        .execute(
            "block.add",
            serde_json::json!({
                "kind": "source",
                "timed": [{"calendar": "lunch", "value": {"type": "exponential", "mean": 2.0}}],
            }),
        )
        .expect("a source with arrivals by calendar");
    h.state_mut()
        .execute(
            "resource.set",
            serde_json::json!({
                "id": pool,
                "shifts": [{"calendar": "lunch", "quantity": 0}],
                "availability": [{"calendar": "lunch", "available": false}],
            }),
        )
        .expect("shifts set");
    settle(&mut h);
    let name = h.state().engine().model().name.clone();
    h.get_by_label(&name).click();
    settle(&mut h);
    h.get_by_label_contains("+ Add block").click();
    settle(&mut h);
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    every_block(&mut h);
    h.state_mut().run();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(&mut h);
    for command in [commands::ABOUT, commands::SHORTCUTS] {
        h.state_mut().run_command(&CommandRef::new(command));
        settle(&mut h);
        h.key_press(egui::Key::Escape);
        settle(&mut h);
    }
    h.state_mut()
        .execute("model.rename", serde_json::json!({"name": "Renamed"}));
    for target in [
        craft_menu::Target::Linux,
        craft_menu::Target::Windows,
        craft_menu::Target::Mac,
        craft_menu::Target::Web,
    ] {
        let _ = h.state_mut().menu_layout(target);
    }
    stochcraft_ui_egui::i18n::I18N.looked_up()
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
