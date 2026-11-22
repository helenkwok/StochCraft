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
    h.state_mut().offer_recovery("session-1", &copy);
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
    h.state_mut().offer_recovery(
        "session-1",
        r#"{"stochcraft_recovery": 1, "name": "x", "model": {"name": "x"}}"#,
    );
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Discard").click();
    settle(&mut h);
    assert_eq!(
        h.state_mut().autosave(),
        Autosave::Keep,
        "this session has no copy of its own"
    );
    assert_eq!(h.state_mut().take_dropped_copies(), vec!["session-1"]);
    assert!(h.state_mut().take_dropped_copies().is_empty(), "once");

    let mut h = harness();
    h.state_mut().offer_recovery("session-2", "not json");
    settle(&mut h);
    assert!(h.query_by_label("Recover unsaved changes?").is_none());
    assert_eq!(h.state_mut().take_dropped_copies(), vec!["session-2"]);
}

/// A copy of an edited example, as a session that ended with unsaved changes left it.
fn left_behind(name: &str) -> String {
    let mut h = harness();
    h.state_mut().open_file(EXAMPLES[0].file, EXAMPLES[0].bytes);
    h.state_mut()
        .execute("block.rename", json!({ "id": "serve", "name": name }));
    copy_of(h.state_mut().autosave())
}

#[test]
fn a_failed_write_is_tried_again_even_if_nothing_changed() {
    let mut h = harness();
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    let first = copy_of(h.state_mut().autosave());
    // The shell could not write it: the model is unchanged, but the copy is not kept.
    h.state_mut().autosave_write_failed();
    assert_eq!(copy_of(h.state_mut().autosave()), first, "handed out again");
    assert_eq!(h.state_mut().autosave(), Autosave::Keep, "kept this time");

    // An older copy may be there after a failed write: it goes once nothing is unsaved.
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft 2" }));
    let _ = copy_of(h.state_mut().autosave());
    h.state_mut().autosave_write_failed();
    h.state_mut().undo();
    h.state_mut().undo();
    assert!(!h.state().has_unsaved_changes());
    assert_eq!(h.state_mut().autosave(), Autosave::Clear);
}

#[test]
fn a_copy_that_could_not_be_removed_is_removed_again() {
    let mut h = harness();
    h.state_mut()
        .execute("model.rename", json!({ "name": "Draft" }));
    let _ = copy_of(h.state_mut().autosave());
    h.state_mut().undo();
    assert_eq!(h.state_mut().autosave(), Autosave::Clear);
    h.state_mut().autosave_clear_failed();
    assert_eq!(h.state_mut().autosave(), Autosave::Clear, "told again");
    assert_eq!(h.state_mut().autosave(), Autosave::Keep);
}

#[test]
fn two_sessions_keep_and_clear_only_their_own_copies() {
    // Session B starts while A has unsaved changes (its copy is in A's slot, which the shell
    // does not offer to B while A is running). B has nothing unsaved and never touches it.
    let mut a = harness();
    a.state_mut()
        .execute("model.rename", json!({ "name": "A's draft" }));
    assert!(matches!(a.state_mut().autosave(), Autosave::Write(_)));
    let mut b = harness();
    assert_eq!(
        b.state_mut().autosave(),
        Autosave::Keep,
        "B has no copy to clear"
    );
    assert!(b.state_mut().take_dropped_copies().is_empty());

    // B edits and finishes: it clears its own copy, which is not A's.
    b.state_mut()
        .execute("model.rename", json!({ "name": "B's draft" }));
    assert!(matches!(b.state_mut().autosave(), Autosave::Write(_)));
    b.state_mut().undo();
    assert_eq!(b.state_mut().autosave(), Autosave::Clear);
    assert!(
        b.state_mut().take_dropped_copies().is_empty(),
        "nothing of A's"
    );
    // A is as it was: still unsaved, and its copy is kept.
    assert!(a.state().has_unsaved_changes());
    assert_eq!(a.state_mut().autosave(), Autosave::Keep);
}

#[test]
fn a_recovered_copy_goes_only_once_this_session_has_its_own() {
    let copy = left_behind("Clerk");
    let mut h = harness();
    h.state_mut().offer_recovery("session-1", &copy);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Recover").click();
    settle(&mut h);
    assert!(h.state_mut().take_dropped_copies().is_empty(), "not yet");

    // The next tick writes this session's copy of the recovered model, and then the old one goes.
    assert!(matches!(h.state_mut().autosave(), Autosave::Write(_)));
    assert_eq!(h.state_mut().take_dropped_copies(), vec!["session-1"]);

    // If that write fails, the old copy stays until one is written.
    let mut h = harness();
    h.state_mut().offer_recovery("session-1", &copy);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Recover").click();
    settle(&mut h);
    assert!(matches!(h.state_mut().autosave(), Autosave::Write(_)));
    h.state_mut().autosave_write_failed();
    assert!(
        h.state_mut().take_dropped_copies().is_empty(),
        "it is the only copy"
    );
    assert!(matches!(h.state_mut().autosave(), Autosave::Write(_)));
    assert_eq!(h.state_mut().take_dropped_copies(), vec!["session-1"]);
}

#[test]
fn saving_right_after_recovering_does_not_leave_the_old_copy_to_come_back() {
    let copy = left_behind("Clerk");
    let mut h = harness();
    h.state_mut().offer_recovery("session-1", &copy);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Recover").click();
    settle(&mut h);

    // Save before the next autosave tick.
    h.state_mut().save();
    assert!(matches!(
        h.state_mut().take_requests().as_slice(),
        [Request::PickSave { .. }]
    ));
    h.state_mut().save_to("/tmp/Recovered.bpmn");
    let Some(Request::Write { path, bytes }) = h.state_mut().take_requests().pop() else {
        panic!("expected a write");
    };
    h.state_mut().written(&path, &bytes);
    assert!(!h.state().has_unsaved_changes());

    // Nothing is unsaved, so this session keeps no copy, and the one it recovered is removed.
    assert!(!matches!(h.state_mut().autosave(), Autosave::Write(_)));
    assert_eq!(h.state_mut().take_dropped_copies(), vec!["session-1"]);
}

#[test]
fn several_copies_are_asked_about_one_at_a_time() {
    let (first, second) = (left_behind("First"), left_behind("Second"));
    let mut h = harness();
    h.state_mut().offer_recovery("newer", &first);
    h.state_mut().offer_recovery("older", &second);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Recover").click();
    settle(&mut h);
    // The model just recovered has unsaved changes, so the next question waits: recovering
    // it would replace them.
    assert!(
        h.state()
            .engine()
            .model()
            .elements
            .iter()
            .any(|e| e.name == "First")
    );
    assert!(h.query_by_label("Recover unsaved changes?").is_none());

    // Once it is saved, the other copy is asked about.
    h.state_mut().save();
    let _ = h.state_mut().take_requests();
    h.state_mut().save_to("/tmp/First.bpmn");
    let Some(Request::Write { path, bytes }) = h.state_mut().take_requests().pop() else {
        panic!("expected a write");
    };
    h.state_mut().written(&path, &bytes);
    settle(&mut h);
    h.get_by_label("Recover unsaved changes?");
    h.get_by_role_and_label(Role::Button, "Recover").click();
    settle(&mut h);
    assert!(
        h.state()
            .engine()
            .model()
            .elements
            .iter()
            .any(|e| e.name == "Second")
    );
    // Both old copies go once this session has a copy of its own (or nothing is unsaved).
    let _ = h.state_mut().autosave();
    let mut dropped = h.state_mut().take_dropped_copies();
    dropped.sort();
    assert_eq!(dropped, vec!["newer", "older"]);
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
