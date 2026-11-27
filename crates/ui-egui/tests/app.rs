//! The app driven through its window, as a user would: menus, shortcuts, runs and the
//! unsaved-changes prompt (egui_kittest, through the accessibility tree; nothing is rendered).

#![allow(clippy::panic, clippy::expect_used)] // A request or result missing fails the test.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, Autosave, Platform, Request};

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
    // The start page's first example card.
    h.get_by_role_and_label(
        egui::accesskit::Role::Button,
        "How long do customers wait at one busy clerk?",
    )
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
        "results of another model are current"
    );
    assert!(h.state().results_out_of_date());
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
fn results_stay_marked_out_of_date_after_an_edit() {
    let tab = |h: &mut Harness<'static, App>, label: &str| {
        h.get_by_role_and_label(egui::accesskit::Role::Tab, label)
            .click();
        settle(h);
    };
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    assert!(h.state().results().is_some());

    // Moving a block changes no result.
    h.state_mut().execute(
        "layout.set",
        json!({ "shapes": [{ "id": "serve", "x": 400, "y": 300 }] }),
    );
    settle(&mut h);
    assert!(h.state().results().is_some(), "moved, not changed");
    assert!(h.query_by_label("Out of date:").is_none());

    // Any other change leaves the results on show, marked, with Run again.
    h.state_mut()
        .execute("block.set", json!({ "id": "serve", "capacity": 2 }));
    settle(&mut h);
    assert!(h.state().results().is_none());
    assert!(h.state().results_out_of_date());
    tab(&mut h, "Results");
    h.get_by_label("Out of date:");
    h.get_by_label("· out of date");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Run again");

    // Undone, they are current again.
    h.state_mut().undo();
    settle(&mut h);
    assert!(h.state().results().is_some());
    assert!(h.query_by_label("Out of date:").is_none());

    // Run again replaces them with results of the model as it is.
    h.state_mut().redo();
    settle(&mut h);
    click(&mut h, "Run again");
    finish_run(&mut h);
    assert!(h.state().results().is_some());
    assert!(h.query_by_label("Out of date:").is_none());

    // Results of a block that is gone still draw, and so does the Analysis view.
    h.state_mut()
        .execute("block.remove", json!({ "id": "serve" }));
    settle(&mut h);
    assert!(h.state().results_out_of_date());
    tab(&mut h, "Results");
    h.get_by_label("Out of date:");
    tab(&mut h, "Analysis");
    tab(&mut h, "Results");
    h.get_by_label("Out of date:");
}

#[test]
fn a_baseline_is_compared_with_the_next_run() {
    // Wide enough for the change marks.
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 900.0))
        .build_ui_state(
            |ui, app: &mut App| app.ui(ui),
            english(App::new(Platform::DESKTOP)),
        );
    h.run();
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    click(&mut h, "Keep as baseline");
    assert!(h.state().baseline().is_some());
    h.get_by_label("This run is the baseline.");
    assert!(h.query_by_label("What the change did").is_none());

    // A second server: the same seed and runs, so the runs are compared in pairs.
    h.state_mut()
        .execute("block.set", json!({ "id": "serve", "capacity": 2 }));
    settle(&mut h);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    h.get_by_label("Compared with the baseline:");
    // What changed, by block and field, both values.
    h.get_by_label("Changed since the baseline:");
    h.get_by_label("Serve customer \u{203a} Capacity: 1 \u{2192} 2");
    h.get_by_label_contains("Both used seed");
    h.get_by_label("What the change did");
    assert!(h.query_all_by_label_contains("fell from").count() >= 2);
    h.get_by_label("Difference (paired, 95%)");
    assert!(h.query_all_by_label("Real: lower").count() >= 2);
    // Each change is drawn against zero, and says what it is.
    assert!(h.query_all_by_label_contains(" ± ").count() >= 4);

    // Kept again, the new run is the baseline; cleared, there is none.
    click(&mut h, "Keep this as the baseline");
    h.get_by_label("This run is the baseline.");
    assert!(h.query_by_label("What the change did").is_none());
    click(&mut h, "Clear baseline");
    assert!(h.state().baseline().is_none());
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Keep as baseline");

    // Another model forgets it.
    click(&mut h, "Keep as baseline");
    assert!(h.state().baseline().is_some());
    h.state_mut()
        .control("model.new", json!({}))
        .expect("a new model");
    assert!(h.state().baseline().is_none());
}

#[test]
fn what_changed_since_the_baseline_follows_the_interface_language() {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1600.0, 900.0))
        .build_ui_state(
            |ui, app: &mut App| app.ui(ui),
            english(App::new(Platform::DESKTOP)),
        );
    h.run();
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    click(&mut h, "Keep as baseline");
    h.state_mut()
        .execute("block.set", json!({ "id": "serve", "capacity": 2 }));
    settle(&mut h);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    let english_line = "Serve customer \u{203a} Capacity: 1 \u{2192} 2";
    h.get_by_label(english_line);
    // In German the list of changes is written again, not kept from before.
    h.state_mut().set_language(Some("de"));
    settle(&mut h);
    assert!(h.query_by_label(english_line).is_none());
    assert!(
        h.query_all_by_label_contains("Serve customer \u{203a} ")
            .count()
            > 0
    );
}

#[test]
fn the_start_page_asks_what_to_find_out() {
    let mut h = harness(Platform::DESKTOP);
    h.get_by_label("What do you want to find out?");
    // The featured examples, each a card named by its question, with its proof.
    let mm1 = "How long do customers wait at one busy clerk?";
    let case_a = "What do failures and weekend breaks do to a line's output and energy per part?";
    h.get_by_role_and_label(egui::accesskit::Role::Button, mm1);
    h.get_by_role_and_label(egui::accesskit::Role::Button, case_a);
    assert!(
        h.query_all_by_label("Checked against queueing theory")
            .count()
            >= 2
    );
    assert!(
        h.query_by_label("How do the time distributions differ?")
            .is_none(),
        "reference models wait behind See all"
    );
    // A question narrows the cards; All brings them back.
    click(&mut h, "Why did it break down?");
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Button, mm1)
            .is_none()
    );
    h.get_by_role_and_label(egui::accesskit::Role::Button, case_a);
    click(&mut h, "All");
    // See all (below the fold, so as from the keyboard) shows the reference models too.
    h.get_by_label("See all 27 examples, including reference models")
        .click_accesskit();
    settle(&mut h);
    h.get_by_role_and_label(
        egui::accesskit::Role::Button,
        "How do the time distributions differ?",
    );
    // All of them are grouped under a heading per field, the reference models last.
    let heading = |h: &Harness<'_, App>, field: &str| {
        h.get_by_role_and_label(egui::accesskit::Role::Heading, field)
            .rect()
            .top()
    };
    let manufacturing = heading(&h, "Manufacturing");
    let logistics = heading(&h, "Logistics and warehousing");
    let reference = heading(&h, "Reference");
    assert!(manufacturing < logistics && logistics < reference);
    h.get_by_label("Show the featured examples only")
        .click_accesskit();
    settle(&mut h);
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Heading, "Reference")
            .is_none()
    );

    // The parcel sorter is featured.
    let sorter = "How many parcels go round the loop because their lane is full?";
    h.get_by_role_and_label(egui::accesskit::Role::Button, sorter);

    // A search looks through every example, featured or not, by the words of its title, what it
    // is about, its question and what it teaches; Esc in the box clears it.
    let search = |h: &mut Harness<'static, App>, text: &str| {
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Search examples")
            .click();
        settle(h);
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Search examples")
            .type_text(text);
        settle(h);
    };
    search(&mut h, "distribution");
    h.get_by_role_and_label(
        egui::accesskit::Role::Button,
        "How do the time distributions differ?",
    );
    h.get_by_role_and_label(egui::accesskit::Role::Heading, "Reference");
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::Button, mm1)
            .is_none()
    );
    assert!(
        h.query_by_label("See all 27 examples, including reference models")
            .is_none(),
        "a search already covers every example"
    );
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    h.get_by_role_and_label(egui::accesskit::Role::Button, mm1);
    // Nothing found says so; the clear button empties the box.
    search(&mut h, "zzz");
    h.get_by_label("No example matches these choices yet.");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Clear search")
        .click_accesskit();
    settle(&mut h);
    h.get_by_role_and_label(egui::accesskit::Role::Button, mm1);

    // A click anywhere on a card opens its example.
    h.get_by_role_and_label(egui::accesskit::Role::Button, mm1)
        .click();
    settle(&mut h);
    assert_eq!(h.state().engine().model().elements.len(), 3);

    // With every block removed, the start page is back; a blank model from it is a new document
    // (after the unsaved-changes question), not the old one's file and history.
    for id in ["arrive", "serve", "leave"] {
        h.state_mut().execute("block.remove", json!({ "id": id }));
    }
    settle(&mut h);
    h.get_by_label("What do you want to find out?");
    h.get_by_label("Start from a blank model").click_accesskit();
    settle(&mut h);
    click(&mut h, "Don't Save");
    assert_eq!(h.state().engine().path(), None);
    assert!(!h.state().has_unsaved_changes());
    assert!(h.query_by_label("What do you want to find out?").is_none());

    // Build a first model (below the cards in this narrow window, so from the keyboard): an
    // empty model on the canvas with Getting started beside it.
    let mut h = harness(Platform::DESKTOP);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Build a first model")
        .click_accesskit();
    settle(&mut h);
    assert!(h.state().help_open());
    h.get_by_label("0 of 5 done");
    assert!(h.query_by_label("What do you want to find out?").is_none());

    // The guide for people from other tools opens in the browser.
    let mut h = harness(Platform::DESKTOP);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Read the two-page guide")
        .click_accesskit();
    settle(&mut h);
    let requests = h.state_mut().take_requests();
    assert!(
        matches!(requests.as_slice(), [Request::OpenUrl { url }] if url.ends_with("docs/switching.md")),
        "{requests:?}"
    );
}

#[test]
fn help_opens_beside_the_work_and_follows_the_model() {
    let mut h = harness(Platform::DESKTOP);
    // F1 opens it; nothing is done yet.
    h.key_press(egui::Key::F1);
    settle(&mut h);
    assert!(h.state().help_open());
    h.get_by_label("Getting started");
    h.get_by_label("0 of 5 done");
    h.get_by_label("Glossary");
    h.get_by_label("Utilisation");

    // An example with arrivals, a connected task and departures: three steps done; a run, four;
    // its results read, five.
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    h.get_by_label("3 of 5 done");
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    h.get_by_label("5 of 5 done");

    // Search narrows the glossary to the matching terms.
    // By its name: the results below have number fields too.
    let field = "Search help and terms";
    let search = h.get_by_role_and_label(egui::accesskit::Role::TextInput, field);
    search.click();
    settle(&mut h);
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, field)
        .type_text("random");
    settle(&mut h);
    h.get_by_label_contains("The starting point of the random numbers");
    assert!(
        h.query_by_label_contains("a task's units are busy")
            .is_none()
    );
    assert!(h.query_by_label("Getting started").is_none());
    // A section chosen from the menu clears the search, so it shows.
    menu(&mut h, &["Help", "Getting Started"]);
    h.get_by_label("Getting started");

    // The guide opens in the browser.
    h.state_mut().take_requests();
    menu(&mut h, &["Help", "User Guide"]);
    let requests = h.state_mut().take_requests();
    assert!(
        matches!(requests.as_slice(), [Request::OpenUrl { url }] if url.ends_with("docs/user-guide.md")),
        "{requests:?}"
    );

    // F1 again, or the close button, closes it.
    h.key_press(egui::Key::F1);
    settle(&mut h);
    assert!(!h.state().help_open());
    menu(&mut h, &["Help", "Glossary"]);
    assert!(h.state().help_open());
    click(&mut h, "\u{00d7}");
    assert!(!h.state().help_open());
}

#[test]
fn show_me_guides_a_first_model_on_the_canvas() {
    let button = egui::accesskit::Role::Button;
    let id_of = |h: &Harness<'static, App>, kind: &str| {
        h.state()
            .engine()
            .model()
            .elements
            .iter()
            .find(|e| e.kind.name() == kind)
            .map(|e| e.id.as_str().to_owned())
            .unwrap_or_default()
    };
    // Build a first model starts the guide at its first step: Arrivals, from the palette.
    let mut h = harness(Platform::DESKTOP);
    h.get_by_role_and_label(button, "Build a first model")
        .click_accesskit();
    settle(&mut h);
    h.get_by_label("Step 1 of 5");
    h.get_by_label("Drag Arrivals onto the canvas, or click it.");
    h.get_by_role_and_label(button, "arrivals").click();
    settle(&mut h);
    // Then a task with a resource; once added, Arrivals is selected so its connect dot shows.
    h.get_by_label("Step 2 of 5");
    h.get_by_label_contains("drag \u{201c}Task with a resource\u{201d}");
    h.get_by_role_and_label(button, "task with a resource")
        .click();
    settle(&mut h);
    h.get_by_label_contains("Drag from the dot on Arrivals to the task");
    let (arrivals, task) = (id_of(&h, "source"), id_of(&h, "server"));
    assert_eq!(
        h.state().selection(),
        std::collections::BTreeSet::from([arrivals.clone()])
    );
    h.get_by_label(&format!("Connect from {arrivals}"));
    // Esc stops the guide on the first press, though it selected Arrivals; Show me resumes it.
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    assert!(h.query_by_label("Step 2 of 5").is_none());
    h.get_by_label("Show me").click_accesskit();
    settle(&mut h);
    h.get_by_label("Step 2 of 5");
    h.state_mut()
        .execute("connect", json!({ "from": arrivals, "to": task }));
    settle(&mut h);
    // Departures, connected from the task.
    h.get_by_label("Step 3 of 5");
    h.get_by_label("Add Departures, where entities leave: drag it onto the canvas.");
    h.get_by_role_and_label(button, "departures").click();
    settle(&mut h);
    h.get_by_label_contains("Drag from the dot on the task to Departures");
    let departures = id_of(&h, "sink");
    h.state_mut()
        .execute("connect", json!({ "from": task, "to": departures }));
    settle(&mut h);
    // Run, then the last callout by the results, which the × closes.
    h.get_by_label("Step 4 of 5");
    h.get_by_label("Press Run to run the model.");
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    h.get_by_label("Done");
    h.get_by_label_contains("That is your first model.");
    h.get_by_label("5 of 5 done");
    click(&mut h, "\u{00d7}");
    assert!(
        h.query_by_label_contains("That is your first model.")
            .is_none()
    );
    assert!(h.state().help_open(), "the × closed the guide, not Help");

    // Show me in Getting started starts it too, and Esc stops it.
    let mut h = harness(Platform::DESKTOP);
    h.key_press(egui::Key::F1);
    settle(&mut h);
    h.get_by_label("Show me").click_accesskit();
    settle(&mut h);
    h.get_by_label("Step 1 of 5");
    // Over the start page, Show me opens the canvas, so the chip it points at is there.
    assert!(h.query_by_label("What do you want to find out?").is_none());
    h.get_by_role_and_label(button, "arrivals");
    h.key_press(egui::Key::Escape);
    settle(&mut h);
    assert!(h.query_by_label("Step 1 of 5").is_none());
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
            name: "Draft.bpmn".into(),
            xpdl: false
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

/// Takes the one download the app asked for.
fn take_download(h: &mut Harness<'static, App>) -> String {
    let requests = h.state_mut().take_requests();
    let [Request::Download { name, .. }] = requests.as_slice() else {
        panic!("expected one download, got {requests:?}");
    };
    name.clone()
}

#[test]
fn a_download_does_not_count_as_saved() {
    let mut h = harness(Platform::BROWSER);
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    settle(&mut h);
    assert!(matches!(h.state_mut().autosave(), Autosave::Write(_)));

    menu(&mut h, &["File", "Download"]);
    let name = take_download(&mut h);
    h.state_mut().downloaded(&name);
    settle(&mut h);
    // The browser can't say whether the file was kept, so nothing is marked saved: the title
    // keeps its mark, the tab still asks before closing and the recovery copy stays.
    assert!(h.state().has_unsaved_changes());
    assert!(h.state().title().contains('•'), "{}", h.state().title());
    assert_eq!(h.state_mut().autosave(), Autosave::Keep, "the copy stays");
    assert_eq!(h.state_mut().take_requests(), vec![]);
    assert_eq!(h.state().status(), "Downloaded Draft.bpmn");
}

#[test]
fn downloading_in_the_unsaved_changes_question_carries_on() {
    let mut h = harness(Platform::BROWSER);
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    settle(&mut h);
    assert!(matches!(h.state_mut().autosave(), Autosave::Write(_)));

    // New ▸ Download: the user chose to download, so New follows once the download starts,
    // and the recovery copy goes with the model it was for (as with Don't Save).
    menu(&mut h, &["File", "New"]);
    click(&mut h, "Download");
    let name = take_download(&mut h);
    h.state_mut().downloaded(&name);
    settle(&mut h);
    assert_eq!(h.state().engine().model().name, "", "New followed");
    assert_eq!(h.state().status(), "New model");
    assert_eq!(h.state_mut().autosave(), Autosave::Clear);
}

const XPDL_NS: &str = "http://www.wfmc.org/2009/XPDL2.2";
const BPMN_NS: &str = "http://www.omg.org/spec/BPMN/20100524/MODEL";

/// The one file the app asked to write (desktop) or download (browser): its name and text.
fn take_file(h: &mut Harness<'static, App>) -> (String, String) {
    let requests = h.state_mut().take_requests();
    match requests.as_slice() {
        [Request::Write { path: name, bytes }] | [Request::Download { name, bytes }] => {
            (name.clone(), String::from_utf8_lossy(bytes).into_owned())
        }
        _ => panic!("expected one file, got {requests:?}"),
    }
}

/// The model in `text`, read as a file named `name` would be.
fn read_back(name: &str, text: &str) -> stochcraft_model::Model {
    let mut e = stochcraft_engine::Engine::new();
    e.open_bytes(name, text.as_bytes(), None)
        .expect("a file that reads back");
    e.model().clone()
}

/// `text`, the file saved as `name`, holds the app's model (shapes aside: a new file gives every
/// element one).
fn holds_the_model(h: &Harness<'static, App>, name: &str, text: &str) {
    let mut saved = read_back(name, text);
    let mut model = h.state().engine().model().clone();
    saved.layout.clear();
    model.layout.clear();
    assert_eq!(saved, model);
}

/// Adds a pool and two tasks that share it: a change an opened file can't take.
fn add_a_pool(h: &mut Harness<'static, App>) {
    let app = h.state_mut();
    app.execute("resource.add", json!({ "id": "agents", "quantity": 2 }));
    for id in ["triage", "callback"] {
        app.execute(
            "block.add",
            json!({ "kind": "shared_task", "id": id, "needs": [{ "resource": "agents" }] }),
        );
    }
    assert_eq!(app.engine().model().resources.len(), 1, "{}", app.status());
    settle(h);
}

/// File ▸ Open Example ▸ M/M/1 queue, then a pool added.
fn mm1_with_a_new_pool(platform: Platform) -> Harness<'static, App> {
    let mut h = harness(platform);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    add_a_pool(&mut h);
    h
}

#[test]
fn save_as_bpmn_writes_bpmn_even_from_an_xpdl_file() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "Tandem line"]);
    menu(&mut h, &["File", "Save As…"]);
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::PickSave { name, xpdl: true }] if name == "tandem.xpdl"
    ));
    // Kept as XPDL, the file is written into, byte for byte while unchanged.
    h.state_mut().save_to("/tmp/copy.xpdl");
    let (_, text) = take_file(&mut h);
    assert_eq!(text.as_bytes(), stochcraft_ui_egui::EXAMPLES[1].bytes);

    // As .bpmn: a new BPMN file, once the user has agreed to leave out what it can't hold.
    h.state_mut().save_to("/tmp/tandem.bpmn");
    assert_eq!(h.state_mut().take_requests(), vec![], "nothing written yet");
    settle(&mut h);
    h.get_by_label("Save as a new BPMN file?");
    click(&mut h, "Save");
    let (path, text) = take_file(&mut h);
    assert_eq!(path, "/tmp/tandem.bpmn");
    assert!(text.contains(BPMN_NS) && !text.contains(XPDL_NS), "{text}");
    holds_the_model(&h, &path, &text);
    h.state_mut().written(&path, text.as_bytes());
    assert!(!h.state().has_unsaved_changes());
    assert_eq!(h.state().status(), "Saved tandem.bpmn");
}

#[test]
fn a_change_the_file_cannot_take_offers_save_as_in_plain_words() {
    let mut h = mm1_with_a_new_pool(Platform::DESKTOP);
    menu(&mut h, &["File", "Save"]);
    assert_eq!(h.state_mut().take_requests(), vec![], "nothing written");
    h.get_by_label("Save as a new BPMN file?");
    // The reason, without the command line's `as_new` hint.
    assert!(
        h.query_by_label_contains("resource pools were added")
            .is_some()
    );
    assert!(h.query_by_label_contains("as_new").is_none());
    assert!(
        !h.state().status().contains("as_new"),
        "{}",
        h.state().status()
    );
    click(&mut h, "Save As…");
    assert_eq!(
        h.state_mut().take_requests(),
        vec![Request::PickSave {
            name: "mm1.bpmn".into(),
            xpdl: false
        }]
    );
    // The user agreed already: the chosen file gets the new BPMN file at once.
    h.state_mut().save_to("/tmp/mm1-agents.bpmn");
    let (path, text) = take_file(&mut h);
    holds_the_model(&h, &path, &text);
    h.state_mut().written(&path, text.as_bytes());
    assert!(!h.state().has_unsaved_changes());
}

#[test]
fn save_as_after_a_change_the_file_cannot_take_writes_a_new_file() {
    let mut h = mm1_with_a_new_pool(Platform::DESKTOP);
    menu(&mut h, &["File", "Save As…"]);
    h.state_mut().take_requests();
    h.state_mut().save_to("/tmp/mm1-agents.bpmn");
    assert_eq!(h.state_mut().take_requests(), vec![], "asked first");
    settle(&mut h);
    assert!(
        h.query_by_label_contains("resource pools were added")
            .is_some()
    );
    click(&mut h, "Save");
    let (path, text) = take_file(&mut h);
    assert_eq!(path, "/tmp/mm1-agents.bpmn");
    holds_the_model(&h, &path, &text);

    // Cancel saves nothing and drops what was to follow the save (here: New).
    let mut h = mm1_with_a_new_pool(Platform::DESKTOP);
    menu(&mut h, &["File", "New"]);
    click(&mut h, "Save");
    h.get_by_label("Save as a new BPMN file?");
    click(&mut h, "Cancel");
    assert_eq!(h.state_mut().take_requests(), vec![]);
    assert_eq!(h.state().engine().model().resources.len(), 1, "no New");
    assert!(h.query_by_label("Save as a new BPMN file?").is_none());
}

#[test]
fn a_new_model_is_not_written_as_bpmn_under_an_xpdl_name() {
    let mut h = harness(Platform::DESKTOP);
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    h.state_mut().save_to("/tmp/Draft.xpdl");
    assert_eq!(h.state_mut().take_requests(), vec![]);
    assert!(
        h.state().status().contains("only as BPMN"),
        "{}",
        h.state().status()
    );
}

#[test]
fn the_browser_downloads_a_new_bpmn_file_when_the_opened_one_cannot_take_a_change() {
    let mut h = mm1_with_a_new_pool(Platform::BROWSER);
    menu(&mut h, &["File", "Download"]);
    assert_eq!(h.state_mut().take_requests(), vec![], "asked first");
    h.get_by_label("Save as a new BPMN file?");
    click(&mut h, "Download");
    let (name, text) = take_file(&mut h);
    assert_eq!(name, "mm1.bpmn");
    holds_the_model(&h, &name, &text);

    // From an XPDL file, the download is named for what it holds.
    let mut h = harness(Platform::BROWSER);
    menu(&mut h, &["File", "Open Example", "Tandem line"]);
    add_a_pool(&mut h);
    menu(&mut h, &["File", "Download"]);
    click(&mut h, "Download");
    let (name, text) = take_file(&mut h);
    assert_eq!(name, "tandem.bpmn");
    assert!(text.contains(BPMN_NS) && !text.contains(XPDL_NS));
    holds_the_model(&h, &name, &text);
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
    // Two charts for the question: where an entity's time goes (a straight line of one task),
    // and the waiting before the busiest task over the run; the rest folded.
    h.get_by_label_contains("in the system goes");
    h.get_by_label("Waiting before Serve customer over the run");
    h.get_by_label("Worked on");
    h.get_by_label("More charts");
    assert!(h.query_by_label("Utilisation (%)").is_none());
    // And the findings above them.
    h.get_by_label("What the run found");
    h.get_by_label_contains("Serve customer is the busiest block: busy");
    click(&mut h, "Export table (CSV)");
    let requests = h.state_mut().take_requests();
    let [Request::Export { name, bytes }] = requests.as_slice() else {
        panic!("expected one export, got {requests:?}");
    };
    assert_eq!(name, "mm1-results.csv");
    let text = String::from_utf8_lossy(bytes);
    assert!(text.starts_with(
        "element,name,kind,metric,mean,half_width,std_dev,n,\
         split_cut,split_low_n,split_low_mean,split_high_n,split_high_mean,split_group\n"
    ));
    assert!(text.contains("serve,Serve customer,server,utilisation,"));
    h.state_mut().exported("/tmp/mm1-results.csv");
    assert_eq!(h.state().status(), "Exported mm1-results.csv");
}

#[test]
fn exporting_the_charts_brings_them_into_view_first() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    // In a 700-point window the charts are below the key measures, out of view, where a
    // screenshot cannot see them.
    let title = "Waiting before Serve customer over the run";
    let bottom = |h: &Harness<'static, App>| h.get_by_label(title).rect().bottom();
    assert!(bottom(&h) > 700.0, "{}", bottom(&h));
    click(&mut h, "Export charts (PNG)");
    for _ in 0..6 {
        h.step();
    }
    // Scrolled into view before the screenshot is asked for.
    let top = h.get_by_label(title).rect().top();
    assert!(top > 0.0 && bottom(&h) < 700.0, "{top} {}", bottom(&h));
    // They fit, so the capture went ahead.
    assert!(
        !h.state().status().starts_with("The charts are taller"),
        "{}",
        h.state().status()
    );
}

#[test]
fn charts_taller_than_the_window_are_not_exported_in_part() {
    // Too short a window for the charts to be on screen whole, however far it scrolls.
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 300.0))
        .build_ui_state(
            |ui, app: &mut App| app.ui(ui),
            english(App::new(Platform::DESKTOP)),
        );
    h.run();
    let e = &stochcraft_ui_egui::EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    h.state_mut().run();
    finish_run(&mut h);
    assert!(h.state().results().is_some(), "{}", h.state().status());
    click(&mut h, "Export charts (PNG)");
    for _ in 0..6 {
        h.step();
    }
    assert_eq!(
        h.state().status(),
        "The charts are taller than the window: make the window taller, then export them again."
    );
    assert_eq!(h.state_mut().take_requests(), vec![]);
}

#[test]
fn the_results_tab_before_the_first_run_offers_to_run() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Results")
        .click();
    settle(&mut h);
    h.get_by_label("No results yet");
    click(&mut h, "Run experiment");
    finish_run(&mut h);
    assert!(h.state().results().is_some());
    assert!(h.query_by_label("No results yet").is_none());
}

#[test]
fn results_are_marked_out_of_date_in_the_frame_an_undo_from_the_menu_makes() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    h.state_mut()
        .execute("block.set", json!({ "id": "serve", "capacity": 2 }));
    settle(&mut h);
    h.get_by_label("Out of date:");
    // Edit ▸ Undo in the window's menu bar, drawn before the Results view: every frame, the
    // view says what the model is now.
    h.get_by_role_and_label(egui::accesskit::Role::MenuItem, "Edit")
        .click();
    settle(&mut h);
    // The item's label carries what it undoes ("Undo …").
    h.get_by_label_contains("Undo ").click();
    for _ in 0..4 {
        h.step();
        assert_eq!(
            h.query_by_label("Out of date:").is_some(),
            h.state().results_out_of_date()
        );
    }
    assert!(!h.state().results_out_of_date(), "undone");
}

#[test]
fn the_new_file_question_holds_back_shortcuts() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "Tandem line"]);
    h.state_mut().save_to("/tmp/tandem.bpmn");
    settle(&mut h);
    h.get_by_label("Save as a new BPMN file?");
    assert!(h.state().has_open_dialog());
    // File ▸ New's shortcut waits: the model the question is about stays.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    settle(&mut h);
    assert!(!h.state().engine().model().elements.is_empty());
    h.get_by_label("Save as a new BPMN file?");
    click(&mut h, "Cancel");
    assert!(!h.state().has_open_dialog());
}

#[test]
fn an_error_elsewhere_during_a_save_keeps_what_follows_it() {
    let mut h = harness(Platform::DESKTOP);
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    settle(&mut h);
    menu(&mut h, &["File", "New"]);
    click(&mut h, "Save");
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::PickSave { .. }]
    ));
    // While the Save dialog is open, something unrelated fails (the settings, say).
    h.state_mut()
        .report_error("the settings could not be saved");
    h.state_mut().save_to("/tmp/Draft.bpmn");
    let Some(Request::Write { path, bytes }) = h.state_mut().take_requests().pop() else {
        panic!("expected a write");
    };
    h.state_mut().written(&path, &bytes);
    settle(&mut h);
    assert_eq!(h.state().engine().model().name, "", "New followed the save");
}

#[test]
fn a_save_dialog_that_fails_drops_the_agreement_to_a_new_file() {
    let mut h = mm1_with_a_new_pool(Platform::DESKTOP);
    menu(&mut h, &["File", "New"]);
    click(&mut h, "Save");
    h.get_by_label("Save as a new BPMN file?");
    click(&mut h, "Save As…");
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::PickSave { .. }]
    ));
    // The Save dialog can't be shown: nothing is saved and New does not follow.
    h.state_mut().save_failed("no file dialog");
    settle(&mut h);
    assert_eq!(h.state().status(), "no file dialog");
    assert_eq!(h.state().engine().model().resources.len(), 1, "no New");
    // A later Save As asks again before it writes a new file.
    h.state_mut().save_to("/tmp/mm1-copy.bpmn");
    assert_eq!(h.state_mut().take_requests(), vec![], "asked first");
    settle(&mut h);
    h.get_by_label("Save as a new BPMN file?");
}

/// Workers that run the replications they are given on this thread, the last first, two per
/// frame, as web workers would (in any order, a few at a time).
#[derive(Default)]
struct Pool {
    worker: Option<stochcraft_engine::Worker>,
    pending: Vec<u64>,
    /// The replications of the run so far.
    total: u64,
    started: usize,
    stopped: usize,
    /// Refuses to start, as a browser without web workers would.
    unavailable: bool,
}

struct Shared(std::rc::Rc<std::cell::RefCell<Pool>>);

impl stochcraft_ui_egui::Workers for Shared {
    fn start(&mut self, task: &str, replications: u64) -> bool {
        let mut p = self.0.borrow_mut();
        if p.unavailable {
            return false;
        }
        p.worker = stochcraft_engine::Worker::new(task).ok();
        p.pending = (0..replications).collect();
        p.total = replications;
        p.started += 1;
        p.worker.is_some()
    }

    fn extend(&mut self, replications: u64) {
        let mut p = self.0.borrow_mut();
        let from = p.total.min(replications);
        p.pending.extend(from..replications);
        p.total = replications;
    }

    fn answers(&mut self) -> Vec<String> {
        let mut p = self.0.borrow_mut();
        let mut out = Vec::new();
        for _ in 0..2 {
            if let (Some(i), Some(w)) = (p.pending.pop(), p.worker.as_ref()) {
                out.push(w.run(i));
            }
        }
        out
    }

    fn stop(&mut self) {
        let mut p = self.0.borrow_mut();
        p.pending.clear();
        p.stopped += 1;
    }
}

fn run_example(h: &mut Harness<'static, App>) -> serde_json::Value {
    h.state_mut()
        .open_file("mm1.bpmn", stochcraft_ui_egui::EXAMPLES[0].bytes);
    settle(h);
    h.key_press(egui::Key::F5);
    finish_run(h);
    let status = h.state().status().to_owned();
    let mut results = h.state().results().expect(&status).clone();
    // Where the model came from is the only difference allowed.
    results["record"]["file"] = serde_json::Value::Null;
    results
}

#[test]
fn runs_go_to_the_shells_workers_with_the_same_results() {
    let pool = std::rc::Rc::new(std::cell::RefCell::new(Pool::default()));
    let mut h = harness(Platform::BROWSER);
    h.state_mut()
        .set_workers(Box::new(Shared(std::rc::Rc::clone(&pool))));
    let on_workers = run_example(&mut h);
    assert_eq!(pool.borrow().started, 1);
    assert_eq!(
        pool.borrow().stopped,
        1,
        "the workers stop once it has finished"
    );
    assert!(
        h.state().status().starts_with("Finished"),
        "{}",
        h.state().status()
    );

    let mut alone = harness(Platform::BROWSER);
    assert_eq!(run_example(&mut alone), on_workers);

    // Without workers to be had, the app runs the replications itself.
    pool.borrow_mut().unavailable = true;
    h.state_mut().run();
    assert!(h.state().is_running());
    finish_run(&mut h);
    assert!(
        h.state().status().starts_with("Finished"),
        "{}",
        h.state().status()
    );
    assert_eq!(pool.borrow().started, 1);
    assert_eq!(pool.borrow().stopped, 1);

    // Cancelling stops the workers.
    pool.borrow_mut().unavailable = false;
    h.state_mut()
        .execute("experiment.set", json!({ "seed": 7 }));
    h.state_mut().run();
    assert!(h.state().is_running());
    h.state_mut().cancel_run();
    assert_eq!(pool.borrow().stopped, 2);
    assert_eq!(h.state().status(), "Run cancelled");
}

#[test]
fn a_palette_chip_adds_its_block_by_click_keyboard_or_drag() {
    let button = egui::accesskit::Role::Button;
    let count = |h: &Harness<'static, App>| h.state().engine().model().elements.len();
    let mut h = harness(Platform::DESKTOP);
    h.get_by_label("Start from a blank model").click_accesskit();
    settle(&mut h);
    // A click: the chip, not a drag area over it, takes it.
    h.get_by_role_and_label(button, "arrivals").click();
    settle(&mut h);
    assert_eq!(count(&h), 1);
    // From the keyboard.
    h.get_by_role_and_label(button, "departures").focus();
    settle(&mut h);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert_eq!(count(&h), 2);
    // Dragged onto the canvas: a press on the chip, a move well away, a release there.
    let from = h.get_by_role_and_label(button, "task").rect().center();
    // Halfway down to the zoom buttons in the canvas's corner: on the canvas.
    let zoom = h.get_by_role_and_label(button, "+").rect().center();
    let to = egui::pos2(from.x + 40.0, (from.y + zoom.y) / 2.0);
    let press = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(egui::Event::PointerMoved(from));
    h.event(press(from, true));
    h.step();
    for i in 1..=10 {
        h.event(egui::Event::PointerMoved(
            from + (to - from) * (i as f32 / 10.0),
        ));
        h.step();
    }
    h.event(press(to, false));
    settle(&mut h);
    assert_eq!(count(&h), 3, "the drop adds a task where it lands");
}

/// A run going when the model is replaced (a new model, a file opened, or an agent's model) is
/// cancelled, so its results cannot turn up later over the new model.
#[test]
fn replacing_the_model_cancels_the_run_in_progress() {
    let mut h = harness(Platform::BROWSER);
    for replace in ["open", "control model.new", "control model.set"] {
        h.state_mut()
            .open_file("mm1.bpmn", stochcraft_ui_egui::EXAMPLES[0].bytes);
        settle(&mut h);
        h.state_mut().run();
        assert!(h.state().is_running(), "{replace}");
        match replace {
            "open" => h
                .state_mut()
                .open_file("mm1.bpmn", stochcraft_ui_egui::EXAMPLES[0].bytes),
            "control model.new" => {
                h.state_mut()
                    .control("model.new", json!({}))
                    .expect("model.new");
            }
            _ => {
                let model = stochcraft_engine::COMMANDS[0].example();
                h.state_mut()
                    .control("model.set", model)
                    .expect("model.set");
            }
        }
        assert!(!h.state().is_running(), "{replace}");
        for _ in 0..20 {
            h.step();
        }
        assert!(h.state().results().is_none(), "{replace}");
        // A new run can start: nothing is left running in the engine.
        if replace == "control model.new" {
            h.state_mut()
                .control("model.set", stochcraft_engine::COMMANDS[0].example())
                .expect("model.set");
        }
        h.state_mut().run();
        assert!(h.state().is_running(), "{replace}: {}", h.state().status());
        h.state_mut().cancel_run();
    }
}

/// What a closed Open dialog must leave as it was: the model, the file name in the title and
/// the unsaved mark.
fn the_document(h: &Harness<'static, App>) -> (serde_json::Value, String, bool) {
    (
        stochcraft_engine::wire::model(h.state().engine().model()),
        h.state().title(),
        h.state().has_unsaved_changes(),
    )
}

#[test]
fn a_cancelled_open_dialog_changes_nothing() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    settle(&mut h);
    let before = the_document(&h);
    assert!(before.2, "the rename is an unsaved change");
    let status = h.state().status().to_owned();

    // Open ▸ Don't Save asks the shell for a file; the user closes the dialog without one, so the
    // shell answers nothing.
    h.state_mut().run_command(&CommandRef::new(commands::OPEN));
    settle(&mut h);
    assert!(
        h.query_by_label_contains("Save changes to").is_some(),
        "the unsaved changes are asked about"
    );
    click(&mut h, "Don't Save");
    assert_eq!(h.state_mut().take_requests(), vec![Request::PickOpen]);
    settle(&mut h);
    assert_eq!(the_document(&h), before, "model, title and unsaved mark");
    assert_eq!(h.state().status(), status);
    assert_eq!(h.state_mut().take_requests(), vec![]);

    // Nothing is left half done: Open asks about the unsaved changes again, and Cancel keeps them.
    h.state_mut().run_command(&CommandRef::new(commands::OPEN));
    settle(&mut h);
    assert!(
        h.query_by_label_contains("Save changes to").is_some(),
        "the unsaved changes are asked about"
    );
    click(&mut h, "Cancel");
    assert_eq!(the_document(&h), before);
    assert_eq!(h.state_mut().take_requests(), vec![]);
}

#[test]
fn a_cancelled_open_dialog_leaves_a_saved_model_saved() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    let before = the_document(&h);
    assert!(!before.2, "an example opens without unsaved changes");
    // Nothing unsaved, so Open goes straight to the picker; closing it changes nothing.
    h.state_mut().run_command(&CommandRef::new(commands::OPEN));
    settle(&mut h);
    assert_eq!(h.state_mut().take_requests(), vec![Request::PickOpen]);
    assert_eq!(the_document(&h), before);
    assert!(h.query_by_label_contains("Save changes to").is_none());
}

#[test]
fn exporting_the_charts_sends_a_png_to_the_shell() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Export charts (PNG)")
        .click();
    // The shell's part: egui_kittest has no renderer here, so the first screenshot the app asks
    // for (a frame or more after the click, once the charts are in view) is answered with a
    // blank window-sized image, as the web or desktop renderer would answer with the pixels.
    let mut asked = false;
    for _ in 0..60 {
        h.step();
        let callbacks: Vec<egui::ScreenshotCallback> = h
            .output()
            .viewport_output
            .values()
            .flat_map(|v| v.commands.iter())
            .filter_map(|c| match c {
                egui::ViewportCommand::Screenshot(callback) => Some(callback.clone()),
                _ => None,
            })
            .collect();
        for callback in callbacks {
            callback.complete(std::sync::Arc::new(egui::ColorImage::filled(
                [1000, 700],
                egui::Color32::WHITE,
            )));
            asked = true;
        }
        if asked {
            break;
        }
    }
    assert!(asked, "the app never asked for a screenshot");
    h.run_steps(3);
    let requests = h.state_mut().take_requests();
    let [Request::Export { name, bytes }] = requests.as_slice() else {
        panic!("expected one export, got {requests:?}");
    };
    assert_eq!(name, "mm1-charts.png");
    assert_eq!(bytes[..8], *b"\x89PNG\r\n\x1a\n", "the PNG signature");
    assert_eq!(bytes[12..16], *b"IHDR", "the header chunk comes first");
    let dimension =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    assert!(dimension(16) > 0 && dimension(20) > 0, "an empty image");
    h.state_mut().exported("/tmp/mm1-charts.png");
    assert_eq!(h.state().status(), "Exported mm1-charts.png");
}

/// Clicks Export charts (PNG) on the results of M/M/1 and steps until the app asks for a
/// screenshot, whose callback it returns unanswered.
fn export_the_charts_of_mm1() -> (Harness<'static, App>, egui::ScreenshotCallback) {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Export charts (PNG)")
        .click();
    for _ in 0..60 {
        h.step();
        let asked = h
            .output()
            .viewport_output
            .values()
            .flat_map(|v| v.commands.iter())
            .find_map(|c| match c {
                egui::ViewportCommand::Screenshot(callback) => Some(callback.clone()),
                _ => None,
            });
        if let Some(callback) = asked {
            return (h, callback);
        }
    }
    panic!("the app never asked for a screenshot");
}

/// A window-sized screenshot, as a renderer would answer with the pixels.
fn blank_window() -> std::sync::Arc<egui::ColorImage> {
    std::sync::Arc::new(egui::ColorImage::filled([1000, 700], egui::Color32::WHITE))
}

#[test]
fn a_chart_export_is_dropped_if_the_view_changes_before_the_screenshot() {
    // The web's screenshot is of a frame after the one that asked. Frames that show the same
    // charts in the same place change nothing: the export goes ahead.
    let (mut h, screenshot) = export_the_charts_of_mm1();
    h.run_steps(3);
    screenshot.complete(blank_window());
    h.run_steps(3);
    let requests = h.state_mut().take_requests();
    assert!(
        matches!(requests.as_slice(), [Request::Export { name, .. }] if name == "mm1-charts.png"),
        "{requests:?}"
    );

    // Another tab before the screenshot arrives: it is not of the charts, so nothing is exported.
    let (mut h, screenshot) = export_the_charts_of_mm1();
    h.get_by_role_and_label(egui::accesskit::Role::Tab, "Model")
        .click();
    h.run_steps(2);
    screenshot.complete(blank_window());
    h.run_steps(3);
    assert_eq!(h.state_mut().take_requests(), vec![]);
    assert_eq!(h.state().status(), "the charts could not be captured");
    assert!(h.state().status_announcement().starts_with("Error"));
}

#[test]
fn a_save_dialog_answered_after_the_model_was_replaced_saves_nothing() {
    let mut h = harness(Platform::DESKTOP);
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    settle(&mut h);
    // New ▸ Save: the Save dialog opens for Draft, and New is to follow the save.
    menu(&mut h, &["File", "New"]);
    click(&mut h, "Save");
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::PickSave { .. }]
    ));
    // While it is open, another model replaces Draft (a file the shell opens or one dropped).
    let e = &stochcraft_ui_egui::EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    // The file was chosen for Draft: nothing is written there, and New does not follow.
    h.state_mut().save_to("/tmp/Draft.bpmn");
    settle(&mut h);
    assert_eq!(h.state_mut().take_requests(), vec![]);
    assert_eq!(
        h.state().status(),
        "The model changed while choosing where to save it, so it was not saved. Save again."
    );
    assert!(!h.state().engine().model().elements.is_empty(), "no New");
    assert!(!h.state().has_open_dialog(), "the menus work again");

    // Quit ▸ Save, and an agent starts a new model while the Save dialog is open: no write and
    // no Quit.
    h.state_mut()
        .control("model.new", json!({}))
        .expect("a new model");
    h.state_mut()
        .execute("model.rename", json!({ "name": "Again" }));
    assert!(!h.state_mut().close_requested());
    settle(&mut h);
    click(&mut h, "Save");
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::PickSave { .. }]
    ));
    h.state_mut()
        .control("model.new", json!({}))
        .expect("a new model");
    h.state_mut().save_to("/tmp/Again.bpmn");
    settle(&mut h);
    assert_eq!(h.state_mut().take_requests(), vec![], "no write, no Quit");
    assert!(h.state().status().starts_with("The model changed"));
}

#[test]
fn the_menus_wait_while_the_save_dialog_is_open() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["File", "Save As…"]);
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::PickSave { .. }]
    ));
    assert!(h.state().has_open_dialog());
    // File ▸ Open's shortcut waits: the model the dialog is for stays.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
    settle(&mut h);
    assert_eq!(h.state_mut().take_requests(), vec![]);
    // Closed without a file: the menus work again.
    h.state_mut().save_cancelled();
    assert!(!h.state().has_open_dialog());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::O);
    settle(&mut h);
    assert_eq!(h.state_mut().take_requests(), vec![Request::PickOpen]);
}

#[test]
fn a_csv_exported_after_an_edit_says_it_is_of_a_prior_run() {
    let mut h = harness(Platform::DESKTOP);
    menu(&mut h, &["File", "Open Example", "M/M/1 queue"]);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    let digest = h
        .state()
        .results()
        .and_then(|r| r["record"]["model_digest"].as_str().map(str::to_owned))
        .expect("the run has a record");
    // The model changes after the run: the table on show is out of date.
    let changed = h
        .state_mut()
        .execute("block.set", json!({ "id": "serve", "capacity": 2 }));
    assert!(changed.is_some());
    settle(&mut h);
    assert!(h.state().results_out_of_date());
    h.state_mut().export_csv();
    let requests = h.state_mut().take_requests();
    let [Request::Export { name, bytes }] = requests.as_slice() else {
        panic!("expected one export, got {requests:?}");
    };
    assert_eq!(name, "mm1-results-prior-run.csv");
    let text = String::from_utf8_lossy(bytes);
    let mut rows = text.lines();
    let header = rows.next().expect("a header");
    assert!(
        header.ends_with(",run_status,run_model_digest,run_seed,run_runs,run_settings"),
        "{header}"
    );
    let rows: Vec<&str> = rows.collect();
    assert!(!rows.is_empty());
    for row in rows {
        assert!(
            row.contains(",prior run: the model has changed since,"),
            "{row}"
        );
        assert!(row.contains(&digest), "{row}");
    }
}

/// Where nothing waits but the machine goes down, the results lead with its availability, and
/// the spread across runs starts on it, not on a mean wait of 0.
#[test]
fn a_machine_that_never_makes_anything_wait_leads_with_its_availability() {
    // A tall window, so the charts below the findings are all on screen.
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 3000.0))
        .build_ui_state(
            |ui, app: &mut App| app.ui(ui),
            english(App::new(Platform::DESKTOP)),
        );
    h.run();
    menu(
        &mut h,
        &["File", "Open Example", "One machine's availability"],
    );
    // Short, so the run finishes within the test's patience.
    h.state_mut().execute(
        "experiment.set",
        json!({ "replications": 4, "duration": 3000.0, "warmup": 0.0 }),
    );
    settle(&mut h);
    menu(&mut h, &["Model", "Run"]);
    finish_run(&mut h);
    eprintln!("status: {}", h.state().status());
    h.get_by_label("Availability (%)");
    assert!(h.query_by_label("Mean wait (h)").is_none());
    // The finding answers it too, not "limits the flow" with nothing waiting.
    h.get_by_label_contains("Machine is down ");
    h.get_by_label_contains("nothing waits for it");
    assert!(h.query_by_label_contains("limits the flow").is_none());
    // A queue that never forms is no chart: the second one is each block's utilisation.
    assert!(
        h.query_by_label("Waiting before Machine over the run")
            .is_none()
    );
    h.get_by_label("Utilisation (%)");
    // The spread across runs starts on the same result.
    let result = h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Result");
    let shown = result.accesskit_node().value();
    assert_eq!(
        shown.as_deref(),
        Some("Machine · Availability"),
        "{shown:?}"
    );
}
