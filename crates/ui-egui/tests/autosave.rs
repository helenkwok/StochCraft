//! Autosave: when a recovery copy is kept, when it goes, and restoring it after a crash.

#![allow(clippy::panic)] // A copy missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use stochcraft_ui_egui::{App, Autosave, EXAMPLES, Platform, Request};

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 760.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), {
            // In English, whatever this machine's language.
            let mut app = App::new(Platform::DESKTOP);
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

fn copy_of(a: Autosave) -> String {
    match a {
        Autosave::Write(c) => c,
        other => panic!("expected a copy, got {other:?}"),
    }
}

#[test]
fn a_copy_is_kept_while_changes_are_unsaved_and_goes_with_them() {
    let mut h = harness();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    assert_eq!(h.state_mut().autosave(), Autosave::Keep, "nothing unsaved");

    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    let copy = copy_of(h.state_mut().autosave());
    assert!(copy.contains("\"Draft\""), "{copy}");
    assert_eq!(h.state_mut().autosave(), Autosave::Keep, "once per change");

    h.state_mut().undo();
    assert_eq!(
        h.state_mut().autosave(),
        Autosave::Clear,
        "back to the saved model"
    );
    assert_eq!(h.state_mut().autosave(), Autosave::Keep);
}

#[test]
fn a_copy_found_at_start_is_recovered_as_an_untitled_unsaved_model() {
    // A session that ended with unsaved changes.
    let copy = {
        let mut h = harness();
        h.state_mut().open_file(EXAMPLES[0].file, EXAMPLES[0].bytes);
        h.state_mut()
            .execute("block.rename", json!({ "id": "serve", "name": "Clerk" }));
        copy_of(h.state_mut().autosave())
    };

    let mut h = harness();
    h.state_mut().offer_recovery(&copy);
    settle(&mut h);
    h.get_by_label("Recover unsaved changes?");
    assert_eq!(
        h.state_mut().autosave(),
        Autosave::Keep,
        "the copy stays until answered"
    );
    h.get_by_role_and_label(Role::Button, "Recover").click();
    settle(&mut h);

    let model = h.state().engine().model();
    assert!(model.elements.iter().any(|e| e.name == "Clerk"));
    assert!(h.state().has_unsaved_changes());
    assert!(
        h.state().engine().path().is_none(),
        "Save asks where it goes"
    );
    assert!(
        h.state().status().starts_with("Recovered mm1.bpmn"),
        "{}",
        h.state().status()
    );
    // Save asks where: it is untitled.
    h.state_mut().save();
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::PickSave { .. }]
    ));
}

#[test]
fn a_discarded_or_damaged_copy_goes() {
    let mut h = harness();
    h.state_mut()
        .offer_recovery(r#"{"stochcraft_recovery": 1, "name": "x", "model": {"name": "x"}}"#);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Discard").click();
    settle(&mut h);
    assert_eq!(h.state_mut().autosave(), Autosave::Clear);

    let mut h = harness();
    h.state_mut().offer_recovery("not json");
    settle(&mut h);
    assert!(h.query_by_label("Recover unsaved changes?").is_none());
    assert_eq!(h.state_mut().autosave(), Autosave::Clear);
}

#[test]
fn quitting_without_saving_keeps_no_copy() {
    let mut h = harness();
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    let _ = h.state_mut().autosave();
    assert!(!h.state_mut().close_requested());
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Don't Save").click();
    settle(&mut h);
    assert_eq!(h.state_mut().take_requests(), vec![Request::Quit]);
    assert_eq!(h.state_mut().autosave(), Autosave::Clear);
}
