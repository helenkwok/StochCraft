//! Experiments for decisions (M2.8) in the window, as a user would use them: a stop condition in
//! the experiment, a block's time from a three-point estimate, a result's spread with a target,
//! and the Analysis view's sweep, sensitivity and sizing.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The M/M/1 example (arrive → serve → leave), made short so the analyses run quickly.
fn mm1() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1300.0, 1000.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    h.state_mut().execute(
        "experiment.set",
        json!({ "replications": 6, "duration": 600.0, "warmup": 0.0 }),
    );
    settle(&mut h);
    h
}

fn model(h: &Harness<'static, App>) -> Value {
    stochcraft_engine::wire::model(h.state().engine().model())
}

fn off_screen(r: egui::Rect) -> bool {
    r.top() < 110.0 || r.bottom() > 920.0
}

/// Clicks the last widget with `role` and `label`, scrolled into view first.
fn click(h: &mut Harness<'static, App>, role: Role, label: &str) {
    fn last<'a>(
        h: &'a Harness<'static, App>,
        role: Role,
        label: &'a str,
    ) -> egui_kittest::Node<'a> {
        h.get_all_by_role_and_label(role, label)
            .last()
            .unwrap_or_else(|| panic!("no {role:?} labelled {label:?}"))
    }
    if off_screen(last(h, role, label).rect()) {
        last(h, role, label).scroll_to_me();
        settle(h);
    }
    last(h, role, label).click();
    settle(h);
}

/// Picks `option` from the drop-down labelled `label`.
fn choose(h: &mut Harness<'static, App>, label: &str, option: &str) {
    click(h, Role::ComboBox, label);
    click(h, Role::Button, option);
}

fn type_into(h: &mut Harness<'static, App>, label: &str, text: &str) {
    if off_screen(h.get_by_role_and_label(Role::TextInput, label).rect()) {
        h.get_by_role_and_label(Role::TextInput, label)
            .scroll_to_me();
        settle(h);
    }
    h.get_by_role_and_label(Role::TextInput, label).click();
    settle(h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

fn run(h: &mut Harness<'static, App>) {
    h.state_mut().run();
    for _ in 0..2000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
    }
    settle(h);
    assert!(
        h.state().status().starts_with("Finished"),
        "{}",
        h.state().status()
    );
}

#[test]
fn a_replication_ends_when_enough_have_left_and_the_results_say_when() {
    let mut h = mm1();
    choose(&mut h, "Ends early", "When enough have left a block");
    let stop = model(&h)["experiment"]["stop"].clone();
    assert_eq!(stop["when"], "count");
    assert_eq!(stop["element"], "leave", "the departures, by default");
    type_into(&mut h, "How many", "50");
    assert_eq!(model(&h)["experiment"]["stop"]["count"], 50);
    // One change, one undo.
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(model(&h)["experiment"]["stop"]["count"], 100);

    run(&mut h);
    assert!(
        h.query_by_label_contains("The stop condition was met in 6 of 6 replications")
            .is_some()
    );
    // The time to the stop is one of the results whose spread can be read.
    choose(&mut h, "Result", "Time to the stop condition");
    h.get_by_label_contains("6 replications · mean");

    click(&mut h, Role::Tab, "Model");
    choose(&mut h, "Ends early", "Never: it runs for its duration");
    assert!(model(&h)["experiment"]["stop"].is_null());
}

#[test]
fn a_tasks_time_is_fitted_to_a_three_point_estimate() {
    let mut h = mm1();
    click(&mut h, Role::ListBoxOption, "Serve customer");
    h.get_by_label("Three-point estimate");
    click(&mut h, Role::Button, "Estimate from three values…");
    choose(&mut h, "Family", "Triangular");
    type_into(&mut h, "Minimum", "1");
    type_into(&mut h, "Most likely", "2");
    type_into(&mut h, "Maximum", "4");
    type_into(&mut h, "Source", "the clerks");
    click(&mut h, Role::Button, "Fit the time");
    let serve = model(&h)["elements"][1].clone();
    assert_eq!(serve["service"]["type"], "triangular", "{serve}");
    assert_eq!(serve["service"]["max"], json!(4.0));
    assert_eq!(serve["estimate"]["source"], "the clerks");
    h.get_by_label_contains("Triangular: minimum 1, most likely 2, maximum 4");
    // The form closes once fitted, and opens again with the estimate in it.
    assert!(
        h.query_by_role_and_label(Role::Button, "Fit the time")
            .is_none()
    );
    click(&mut h, Role::Button, "Change the estimate");
    click(&mut h, Role::Button, "Cancel");
    assert!(
        h.query_by_role_and_label(Role::Button, "Fit the time")
            .is_none()
    );

    click(&mut h, Role::Button, "Forget the estimate");
    assert!(model(&h)["elements"][1]["estimate"].is_null());
    assert_eq!(model(&h)["elements"][1]["service"]["type"], "triangular");
}

#[test]
fn a_results_spread_and_the_chance_of_meeting_a_target() {
    let mut h = mm1();
    run(&mut h);
    h.get_by_label("Spread across replications");
    choose(&mut h, "Result", "Serve customer · Mean wait");
    h.get_by_label("Percentile");
    choose(&mut h, "Target", "At most");
    type_into(&mut h, "Target value", "1000");
    h.get_by_label_contains("6 of 6 replications met the target: 100%");
    type_into(&mut h, "Target value", "0");
    h.get_by_label_contains("0 of 6 replications met the target: 0%");
}

#[test]
fn the_analysis_view_sweeps_ranks_and_sizes() {
    let mut h = mm1();
    click(&mut h, Role::Tab, "Analysis");
    h.get_by_label("Sweep a parameter");

    // A sweep of the clerks.
    choose(&mut h, "Parameter", "Serve customer: units");
    assert_eq!(
        h.get_by_role_and_label(Role::TextInput, "Values")
            .accesskit_node()
            .value()
            .as_deref(),
        Some("1; 2; 3")
    );
    choose(&mut h, "Result to compare", "Serve customer · Mean wait");
    click(&mut h, Role::Button, "Run the sweep");
    h.get_by_label_contains("Across the range it changed by");
    h.get_by_label_contains("6 replications at each step · seed");

    // Which parameters matter.
    choose(
        &mut h,
        "Result to explain",
        "Customer leaves · Mean time in system",
    );
    click(&mut h, Role::Button, "Rank the parameters");
    assert!(
        h.query_by_label("Elasticity").is_some(),
        "{}",
        h.state().status()
    );
    h.get_by_label("Serve customer: mean service time");

    // Sizing the clerks: a wait of at most a minute in 70% of replications.
    choose(&mut h, "Result with a target", "Serve customer · Mean wait");
    h.get_by_role_and_label(Role::TextInput, "Target share");
    click(&mut h, Role::Button, "Size");
    h.get_by_label_contains("With every time at its mean:");
    // How sure the share is: with 6 replications, Wilson's interval reaches well below 70%
    // whatever share met it, so the note says the target share is not shown to be met.
    h.get_by_label_contains("95% interval of the share meeting the target:");
    h.get_by_label_contains("so the share is not shown to be met with 95% confidence");
    let before = model(&h);
    click(&mut h, Role::Button, "Use these units in the model");
    let capacity = |m: &Value| m["elements"][1]["capacity"].as_u64().unwrap_or(1);
    assert!(
        capacity(&model(&h)) > capacity(&before),
        "{}",
        model(&h)["elements"][1]
    );
    // The answers belong to the model they were found for: hidden once it has changed, and back
    // when Undo returns to it.
    assert!(
        h.query_by_label_contains("With every time at its mean:")
            .is_none()
    );
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(capacity(&model(&h)), capacity(&before));
    h.get_by_label_contains("With every time at its mean:");
}
