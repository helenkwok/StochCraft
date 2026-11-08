//! Commands from an agent (the MCP server through the desktop app's control channel) change the
//! window as the person's own would: a model opened replaces the one shown, and a run's results
//! are shown.

#![allow(clippy::panic, clippy::expect_used)] // A missing value fails the test.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use stochcraft_engine::ErrorCode;
use stochcraft_ui_egui::{App, Platform};

fn harness() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run_steps(2);
    h
}

#[test]
fn an_agents_commands_show_in_the_window() {
    let mut h = harness();
    let mm1 = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models/mm1.bpmn");
    let opened = h
        .state_mut()
        .control("file.open", json!({ "path": mm1.to_string_lossy() }))
        .expect("opens");
    assert_eq!(opened["format"], "bpmn");
    assert_eq!(h.state().status(), "Opened mm1.bpmn");
    h.run_steps(2);
    assert!(!h.state().engine().model().elements.is_empty());

    let results = h
        .state_mut()
        .control("run", json!({ "replications": 2 }))
        .expect("runs");
    assert_eq!(h.state().results(), Some(&results));
    assert_eq!(h.state().status(), "Finished 2 replications");
    h.run_steps(2);
    // The results are on screen.
    assert!(h.query_all_by_label("± 95% CI").count() >= 1);

    // An edit goes through the engine, so the person can undo it; the results go stale.
    h.state_mut()
        .control("block.rename", json!({ "id": "serve", "name": "Clerk" }))
        .expect("renames");
    h.run_steps(2);
    assert!(h.state().results().is_none());

    // The window's background runs are its own.
    for command in ["run.start", "run.status", "run.cancel"] {
        let e = h
            .state_mut()
            .control(command, json!({}))
            .expect_err(command);
        assert_eq!(e.code, ErrorCode::Unsupported);
    }
    // A failure is the engine's, and changes nothing.
    let before = h.state().engine().model().clone();
    let e = h
        .state_mut()
        .control("block.set", json!({ "id": "ghost" }))
        .expect_err("no such block");
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(h.state().engine().model(), &before);
}
