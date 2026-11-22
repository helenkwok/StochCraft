//! A file read in the background (the browser's picker and dropped files) arrives late, maybe
//! after another file was asked for or the model was edited. The app is driven without a
//! browser: the tests make the completions arrive in the order a slow read could give them.

#![allow(clippy::panic)] // A widget missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), {
            let mut app = App::new(Platform::BROWSER);
            app.set_lang(craft_i18n::Lang::EN);
            app
        });
    h.run();
    h
}

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The file's name and bytes: an example under another name.
fn file(i: usize) -> (&'static str, Vec<u8>) {
    (EXAMPLES[i].file, EXAMPLES[i].bytes.to_vec())
}

/// The file the model came from.
fn open_name(h: &Harness<'static, App>) -> String {
    h.state().engine().path().unwrap_or("").to_owned()
}

fn model_name(h: &Harness<'static, App>) -> String {
    h.state().engine().model().name.clone()
}

#[test]
fn completions_in_reverse_order_open_only_the_newest_request() {
    let mut h = harness();
    // Two picks; the first file is slow and arrives last.
    let first = h.state_mut().begin_open();
    let second = h.state_mut().begin_open();
    let (name2, bytes2) = file(1);
    h.state_mut().finish_open(second, name2, bytes2, false);
    let newest = open_name(&h);
    assert_eq!(newest, EXAMPLES[1].file);
    assert!(
        h.state().status().starts_with("Opened"),
        "{}",
        h.state().status()
    );

    let (name1, bytes1) = file(0);
    h.state_mut().finish_open(first, name1, bytes1, false);
    assert_eq!(open_name(&h), newest, "the late one did not replace it");
    assert!(
        h.state().status().starts_with("Did not open"),
        "{}",
        h.state().status()
    );
}

#[test]
fn an_older_request_that_arrives_first_is_dropped_too() {
    let mut h = harness();
    let first = h.state_mut().begin_open();
    let second = h.state_mut().begin_open();
    let before = open_name(&h);
    let (name1, bytes1) = file(0);
    h.state_mut().finish_open(first, name1, bytes1, true);
    assert_eq!(open_name(&h), before, "a newer request is on its way");
    let (name2, bytes2) = file(1);
    h.state_mut().finish_open(second, name2, bytes2, true);
    assert_ne!(open_name(&h), before);
}

#[test]
fn a_file_that_arrives_after_the_model_was_replaced_is_dropped() {
    let mut h = harness();
    let ticket = h.state_mut().begin_open();
    // The user opens an example meanwhile.
    h.state_mut().open_file(EXAMPLES[1].file, EXAMPLES[1].bytes);
    let shown = open_name(&h);
    let (name, bytes) = file(0);
    h.state_mut().finish_open(ticket, name, bytes, false);
    assert_eq!(open_name(&h), shown);
    assert!(h.state().status().starts_with("Did not open"));
}

#[test]
fn a_picked_file_is_asked_about_if_the_model_was_edited_meanwhile() {
    let mut h = harness();
    let ticket = h.state_mut().begin_open();
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    let (name, bytes) = file(0);
    h.state_mut().finish_open(ticket, name, bytes, false);
    settle(&mut h);
    // Not opened over the edits: the question comes first.
    assert_eq!(model_name(&h), "Draft");
    assert!(h.state().has_unsaved_changes());
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    settle(&mut h);
    assert_eq!(model_name(&h), "Draft", "cancelled: the edits are kept");

    // Don't Save opens it.
    let ticket = h.state_mut().begin_open();
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft 2" }));
    let (name, bytes) = file(0);
    h.state_mut().finish_open(ticket, name, bytes, false);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    settle(&mut h);
    assert_eq!(open_name(&h), EXAMPLES[0].file);
    assert!(!h.state().has_unsaved_changes());
}

#[test]
fn a_picked_file_opens_without_a_second_question_if_nothing_changed() {
    let mut h = harness();
    // Unsaved changes, asked about before the picker (the user chose Don't Save): the model is
    // still changed when the file arrives, but it is not asked about again.
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    let ticket = h.state_mut().begin_open();
    let (name, bytes) = file(0);
    h.state_mut().finish_open(ticket, name, bytes, false);
    settle(&mut h);
    assert!(
        h.query_by_role_and_label(Role::Button, "Don't Save")
            .is_none()
    );
    assert_eq!(open_name(&h), EXAMPLES[0].file);
}

#[test]
fn a_dropped_file_is_always_asked_about_unsaved_changes() {
    let mut h = harness();
    let ticket = h.state_mut().begin_open();
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    let (name, bytes) = file(0);
    h.state_mut().finish_open(ticket, name, bytes, true);
    settle(&mut h);
    assert_eq!(model_name(&h), "Draft");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    settle(&mut h);
    assert_eq!(model_name(&h), "Draft");
}
