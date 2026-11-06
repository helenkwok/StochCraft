//! The app driven through its window, as a user would: menus, shortcuts, runs and the
//! unsaved-changes prompt (egui_kittest, through the accessibility tree; nothing is rendered).

#![allow(clippy::panic, clippy::expect_used)] // A request or result missing fails the test.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use stochcraft_ui_egui::{App, Platform, Request};

/// The app with English numbers (`1,234.5`), so the tests read the same in every region.
fn english(mut app: App) -> App {
    app.set_number_region("en");
    // The interface in English too, whatever this machine's language.
    app.set_lang(craft_i18n::Lang::EN);
    app
}

fn harness(platform: Platform) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), english(App::new(platform)));
    h.run();
    h
}

/// Clicks the widget labelled `label` (the last one drawn, so a menu over the window wins), or a
/// menu item that shows a shortcut after its label.
fn click(h: &mut Harness<'static, App>, label: &str) {
    let menu_item = egui::accesskit::Role::MenuItem;
    let button = egui::accesskit::Role::Button;
    if let Some(node) = h.query_all_by_role_and_label(menu_item, label).last() {
        node.click();
    } else if let Some(node) = h.query_all_by_role_and_label(button, label).last() {
        node.click();
    } else if let Some(node) = h.query_all_by_label(label).last() {
        node.click();
    } else {
        h.get_by_label_contains(&format!("{label} ")).click();
    }
    settle(h);
}

/// Runs frames until the window settles, or a few if a run keeps it repainting.
fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// Clicks through a menu path, such as File ▸ Open Example ▸ M/M/1 queue.
fn menu(h: &mut Harness<'static, App>, path: &[&str]) {
    for (i, label) in path.iter().enumerate() {
        if i == 0 {
            // The menu bar's title (craft-menu), not the work area's tab of the same name.
            h.get_by_role_and_label(egui::accesskit::Role::MenuItem, label)
                .click();
            settle(h);
        } else {
            click(h, label);
        }
    }
}

/// Steps until the background run finishes.
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
fn an_example_opens_runs_and_shows_results() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    assert!(h.state().status().starts_with("Opened mm1.bpmn"));
    assert!(!h.state().engine().model().elements.is_empty());

    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    let status = h.state().status().to_owned();
    let results = h.state().results().expect(&status);
    assert!(
        results["elements"]
            .as_array()
            .is_some_and(|e| !e.is_empty())
    );
    assert!(
        h.state().status().starts_with("Finished"),
        "{}",
        h.state().status()
    );
    // The table is on screen.
    assert!(h.query_all_by_label("± 95% CI").count() >= 1);
    assert!(h.query_all_by_label("Mean time in system").count() >= 1);
}

#[test]
fn the_browser_runs_in_slices_and_downloads_instead_of_saving() {
    let mut h = harness(Platform::BROWSER);
    // The welcome screen's first example.
    h.get_all_by_label("Open")
        .next()
        .expect("an example's Open button")
        .click();
    settle(&mut h);
    h.key_press(egui::Key::F5);
    finish_run(&mut h);
    assert!(h.state().results().is_some(), "{}", h.state().status());

    menu(&mut h, &["File", "Download"]);
    let requests = h.state_mut().take_requests();
    let [Request::Download { name, bytes }] = requests.as_slice() else {
        panic!("expected one download, got {requests:?}");
    };
    assert_eq!(name, "mm1.bpmn");
    // Unchanged, the file downloads byte for byte as it was opened.
    assert_eq!(bytes.as_slice(), stochcraft_ui_egui::EXAMPLES[0].bytes);
}

#[test]
fn undo_and_redo_name_the_change_and_results_go_stale() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    assert!(h.state().results().is_some());

    let app = h.state_mut();
    assert!(
        app.execute("model.rename", json!({ "name": "Renamed" }))
            .is_some()
    );
    settle(&mut h);
    assert!(
        h.state().results().is_none(),
        "results of another model stay"
    );
    assert!(h.state().title().contains('•'), "{}", h.state().title());

    click(&mut h, "Edit");
    h.get_by_label_contains("Undo Rename Model").click();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Rename Model");
    assert!(!h.state().has_unsaved_changes());

    h.key_press_modifiers(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::Z,
    );
    settle(&mut h);
    assert_eq!(h.state().engine().model().name, "Renamed");
}

#[test]
fn unsaved_changes_are_asked_about_before_they_are_lost() {
    let mut h = harness(Platform::DESKTOP);
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    settle(&mut h);

    // Cancel keeps the model.
    menu(&mut h, &["File", "New"]);
    h.get_by_label("Save changes to Draft?");
    click(&mut h, "Cancel");
    assert_eq!(h.state().engine().model().name, "Draft");

    // Save asks where, writes, then carries on with New.
    menu(&mut h, &["File", "New"]);
    click(&mut h, "Save");
    assert_eq!(
        h.state_mut().take_requests(),
        vec![Request::PickSave {
            name: "Draft.bpmn".into()
        }]
    );
    h.state_mut().save_to("/tmp/Draft.bpmn");
    let Some(Request::Write { path, bytes }) = h.state_mut().take_requests().pop() else {
        panic!("expected a write");
    };
    h.state_mut().written(&path, &bytes);
    settle(&mut h);
    assert_eq!(h.state().engine().model().name, "", "New followed the save");
    assert_eq!(h.state().status(), "New model");

    // Quitting with nothing unsaved asks nothing.
    assert!(h.state_mut().close_requested());
    h.state_mut()
        .execute("model.rename", json!({ "name": "Again" }));
    assert!(!h.state_mut().close_requested());
    settle(&mut h);
    click(&mut h, "Don't Save");
    assert_eq!(h.state_mut().take_requests(), vec![Request::Quit]);
}

#[test]
fn a_cancelled_save_drops_what_was_to_follow_it() {
    let mut h = harness(Platform::DESKTOP);
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    settle(&mut h);
    // New ▸ Save, then the user closes the Save dialog without choosing a file.
    menu(&mut h, &["File", "New"]);
    click(&mut h, "Save");
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::PickSave { .. }]
    ));
    h.state_mut().save_cancelled();
    settle(&mut h);
    assert_eq!(h.state().engine().model().name, "Draft");
    // A later, ordinary save does only that: no New in its wake.
    h.state_mut().save_to("/tmp/Draft.bpmn");
    let Some(Request::Write { path, bytes }) = h.state_mut().take_requests().pop() else {
        panic!("expected a write");
    };
    h.state_mut().written(&path, &bytes);
    settle(&mut h);
    assert_eq!(h.state().engine().model().name, "Draft", "nothing followed");
    assert_eq!(h.state_mut().take_requests(), vec![]);
}

#[test]
fn a_dropped_file_waits_for_the_unsaved_changes_question() {
    let mut h = harness(Platform::BROWSER);
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    let example = &stochcraft_ui_egui::EXAMPLES[0];
    h.state_mut()
        .drop_file(example.file, example.bytes.to_vec());
    settle(&mut h);
    assert_eq!(h.state().engine().model().name, "Draft");
    click(&mut h, "Don't Save");
    assert!(h.state().status().starts_with("Opened mm1.bpmn"));
}

#[test]
fn a_file_that_cannot_be_read_is_reported_and_the_model_kept() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    h.state_mut().open_file("broken.bpmn", b"<not a model");
    settle(&mut h);
    assert!(
        h.state().status().starts_with("broken.bpmn"),
        "{}",
        h.state().status()
    );
    assert!(!h.state().engine().model().elements.is_empty());
}

#[test]
fn about_names_the_licence() {
    let mut h = harness(Platform::BROWSER);
    menu(&mut h, &["Help", "About StochCraft"]);
    h.get_by_label_contains("MIT License or the Apache License 2.0");
    // The browser has no Quit.
    click(&mut h, "File");
    assert!(h.query_by_label("Quit").is_none());
}

#[test]
fn results_export_as_csv_and_charts_are_drawn() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    // One chart per measure that applies, titled with its unit.
    h.get_by_label("Utilisation (%)");
    h.get_by_label("Mean wait (min)");
    h.get_by_label("Mean time in system (min)");
    click(&mut h, "Export table (CSV)");
    let requests = h.state_mut().take_requests();
    let [Request::Export { name, bytes }] = requests.as_slice() else {
        panic!("expected one export, got {requests:?}");
    };
    assert_eq!(name, "mm1-results.csv");
    let text = String::from_utf8_lossy(bytes);
    assert!(text.starts_with("element,name,kind,metric,mean,half_width,std_dev,n\n"));
    assert!(text.contains("serve,Serve customer,server,utilisation,"));
    h.state_mut().exported("/tmp/mm1-results.csv");
    assert_eq!(h.state().status(), "Exported mm1-results.csv");
}
