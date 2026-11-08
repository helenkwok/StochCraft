//! The app driven through its window, as a user would: menus, shortcuts, runs and the
//! unsaved-changes prompt (egui_kittest, through the accessibility tree; nothing is rendered).

#![allow(clippy::panic, clippy::expect_used)] // A request or result missing fails the test.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
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

/// Workers that run the replications they are given on this thread, the last first, two per
/// frame, as web workers would (in any order, a few at a time).
#[derive(Default)]
struct Pool {
    worker: Option<stochcraft_engine::Worker>,
    pending: Vec<u64>,
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
        p.started += 1;
        p.worker.is_some()
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
