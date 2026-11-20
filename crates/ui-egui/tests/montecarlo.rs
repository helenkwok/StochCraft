//! Monte Carlo models in the window, as a user would use them: the example opened from the start
//! page, an input edited in its distribution editor, a run made a few batches a frame, and the
//! findings in words; File ▸ New Monte Carlo Model, an input and an output added, an expression
//! refused in place, and a table of inputs pasted in (egui_kittest, through the accessibility
//! tree; nothing is rendered).

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use stochcraft_ui_egui::{App, Platform, Request};

/// The app with English numbers and words, so the tests read the same in every region.
fn english(mut app: App) -> App {
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    app
}

/// Tall enough that a Monte Carlo model's whole page is on screen.
fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 3000.0))
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

/// Clicks the last widget labelled `label` with `role`.
fn click(h: &mut Harness<'static, App>, role: Role, label: &str) {
    h.get_all_by_role_and_label(role, label)
        .last()
        .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
        .click();
    settle(h);
}

/// Replaces the text of the first field labelled `label` and presses Enter.
fn type_into(h: &mut Harness<'static, App>, role: Role, label: &str, text: &str) {
    let first =
        |h: &Harness<'static, App>| h.get_all_by_role_and_label(role, label).next().is_some();
    assert!(first(h), "no {role:?} labelled {label:?}");
    if let Some(n) = h.get_all_by_role_and_label(role, label).next() {
        n.click();
    }
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    if let Some(n) = h.get_all_by_role_and_label(role, label).next() {
        n.type_text(text);
    }
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

/// Steps until the run finishes.
fn finish_run(h: &mut Harness<'static, App>) {
    h.step();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(h);
}

fn inputs(h: &Harness<'static, App>) -> Vec<String> {
    h.state()
        .engine()
        .model()
        .monte_carlo
        .as_ref()
        .map(|m| m.inputs.iter().map(|i| i.name.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn the_budget_example_runs_and_says_the_chance_of_an_overrun() {
    let mut h = harness();
    click(
        &mut h,
        Role::Button,
        "Will the project come in under budget?",
    );
    assert!(h.state().engine().model().is_monte_carlo());
    // Its own tabs: the model and its results, and none of a process model's views.
    assert!(h.query_by_role_and_label(Role::Tab, "Model").is_some());
    assert!(h.query_by_role_and_label(Role::Tab, "Results").is_some());
    assert!(h.query_by_role_and_label(Role::Tab, "3D").is_none());
    assert!(h.query_all_by_label("Monte Carlo model").count() >= 1);

    // An input's distribution, edited as a task's time is: the design's minimum.
    type_into(&mut h, Role::TextInput, "Minimum", "9");
    let design = h
        .state()
        .engine()
        .model()
        .monte_carlo
        .as_ref()
        .and_then(|m| m.inputs.first())
        .map(|i| i.dist.clone());
    assert_eq!(
        design,
        Some(stochcraft_model::Dist::Triangular {
            min: 9.0,
            mode: 10.0,
            max: 16.0
        })
    );

    h.state_mut().run();
    assert!(h.state().is_running(), "{}", h.state().status());
    finish_run(&mut h);
    let status = h.state().status().to_owned();
    let results = h.state().results().cloned().expect(&status);
    assert_eq!(results["kind"], "monte_carlo");
    assert_eq!(results["trials"], 10_000);
    assert!(status.starts_with("Finished"), "{status}");
    // The findings first, in words: the chance each target is missed.
    h.get_by_label_contains("chance that cost is more than");
    h.get_by_label_contains("chance that duration is more than 38 days");
    assert_eq!(
        h.query_all_by_label_contains("95% interval of the chance")
            .count(),
        2
    );
    assert!(
        h.query_all_by_label("How many trials gave each value")
            .count()
            >= 2
    );

    // The table, as CSV.
    click(&mut h, Role::Button, "Export table (CSV)");
    let requests = h.state_mut().take_requests();
    let [Request::Export { name, bytes }] = requests.as_slice() else {
        panic!("expected one export, got {requests:?}");
    };
    assert_eq!(name, "budget-results.csv");
    assert!(String::from_utf8_lossy(bytes).starts_with("output,unit,trials"));

    // Edited after the run, the results stay on show, marked out of date.
    h.state_mut().execute(
        "mc.input.set",
        serde_json::json!({ "name": "materials", "unit": "EUR" }),
    );
    settle(&mut h);
    assert!(h.state().results_out_of_date());
    assert!(h.query_all_by_label_contains("Out of date").count() >= 1);
}

#[test]
fn a_new_monte_carlo_model_is_built_in_the_window() {
    let mut h = harness();
    h.state_mut()
        .run_command(&stochcraft_ui_egui::commands::CommandRef::new(
            stochcraft_ui_egui::commands::NEW_MONTE_CARLO,
        ));
    settle(&mut h);
    assert!(h.state().engine().model().is_monte_carlo());
    assert_eq!(h.state().status(), "New Monte Carlo model");

    type_into(&mut h, Role::TextInput, "New input", "labour");
    assert_eq!(inputs(&h), ["labour"]);
    type_into(&mut h, Role::TextInput, "New output", "cost");
    let output = |h: &Harness<'static, App>| {
        h.state()
            .engine()
            .model()
            .monte_carlo
            .as_ref()
            .and_then(|m| m.outputs.first())
            .map(|o| o.expression.clone())
    };
    assert_eq!(output(&h).as_deref(), Some("$labour"));

    // An expression that cannot be read is refused, and says why under its field.
    type_into(&mut h, Role::TextInput, "Expression", "$labour *");
    assert_eq!(output(&h).as_deref(), Some("$labour"));
    h.get_by_label_contains("the expression cannot be read");
    type_into(&mut h, Role::TextInput, "Expression", "$labour * 45");
    assert_eq!(output(&h).as_deref(), Some("$labour * 45"));
    assert!(
        h.query_by_label_contains("the expression cannot be read")
            .is_none()
    );

    // Inputs pasted as a table, without its header.
    if let Some(n) = h.query_all_by_label("Paste inputs from a table").next() {
        n.click();
    }
    settle(&mut h);
    if let Some(n) = h
        .get_all_by_role_and_label(Role::MultilineTextInput, "Inputs as a table")
        .next()
    {
        n.click();
    }
    settle(&mut h);
    if let Some(n) = h
        .get_all_by_role_and_label(Role::MultilineTextInput, "Inputs as a table")
        .next()
    {
        n.type_text("rate,uniform,min=40;max=50,USD/h");
    }
    settle(&mut h);
    click(&mut h, Role::Button, "Import inputs");
    assert_eq!(inputs(&h), ["labour", "rate"]);
    assert_eq!(h.state().status(), "Imported 1 input");
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(inputs(&h), ["labour"]);
    assert_eq!(h.state().status(), "Undid Import Inputs");
}
