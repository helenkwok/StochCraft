//! Validation against observed data (M2.9, #226) in the window, as a user would use it: observed
//! values and bounds added, changed and removed in the model page, and the results saying whether
//! the run agrees with them.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The M/M/1 example (arrive → serve → leave), made short so a run is quick.
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

/// The id of the block called `name`.
fn id_of(h: &Harness<'static, App>, name: &str) -> String {
    model(h)["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|e| e["name"] == name)
        .and_then(|e| e["id"].as_str())
        .unwrap_or_else(|| panic!("no block called {name}"))
        .to_owned()
}

/// The first button labelled `label` on the page, from the top.
fn click_first(h: &mut Harness<'static, App>, label: &str) {
    fn first<'a>(h: &'a Harness<'static, App>, label: &'a str) -> egui_kittest::Node<'a> {
        h.get_all_by_role_and_label(Role::Button, label)
            .next()
            .unwrap_or_else(|| panic!("no button {label:?}"))
    }
    if off_screen(first(h, label).rect()) {
        first(h, label).scroll_to_me();
        settle(h);
    }
    first(h, label).click();
    settle(h);
}

fn observed(h: &Harness<'static, App>) -> Vec<Value> {
    model(h)["experiment"]["observed"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn the_model_page_adds_changes_and_removes_observed_data() {
    let mut h = mm1();
    let serve = id_of(&h, "Serve customer");
    h.get_by_label("Observed data");
    assert!(observed(&h).is_empty());

    // A result to add observed data for, by block and result.
    choose(
        &mut h,
        "Add an observed value",
        "Serve customer · Mean wait",
    );
    assert_eq!(
        observed(&h),
        vec![json!({ "check": "value", "element": serve, "metric": "mean_wait", "value": 0.0 })]
    );

    // The value, the half-width of its own interval, and the tolerance (none until set).
    type_into(&mut h, "Observed value", "3");
    type_into(&mut h, "Its 95% half-width", "0.5");
    assert!(observed(&h)[0].get("tolerance").is_none());
    type_into(&mut h, "Tolerance (0 for none)", "0.25");
    let o = observed(&h)[0].clone();
    assert_eq!(
        (
            o["value"].as_f64(),
            o["half_width"].as_f64(),
            o["tolerance"].as_f64()
        ),
        (Some(3.0), Some(0.5), Some(0.25)),
        "{o}"
    );
    type_into(&mut h, "Tolerance (0 for none)", "0");
    assert!(observed(&h)[0].get("tolerance").is_none());
    type_into(&mut h, "Tolerance (0 for none)", "0.25");

    // As a bound: the number stays, the half-width and tolerance go.
    choose(&mut h, "Checked as", "At most");
    assert_eq!(
        observed(&h),
        vec![json!({ "check": "at_most", "element": serve, "metric": "mean_wait", "bound": 3.0 })]
    );
    for gone in ["Its 95% half-width", "Tolerance (0 for none)"] {
        assert!(h.query_by_role_and_label(Role::TextInput, gone).is_none());
    }

    // One undo for each change.
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(observed(&h)[0]["check"], "value");

    // Removed (undone, too).
    click_first(&mut h, "Remove");
    assert!(observed(&h).is_empty());
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(observed(&h).len(), 1);
    click_first(&mut h, "Remove");
    assert!(observed(&h).is_empty());

    // A share is a percentage in the window and a fraction in the model.
    choose(
        &mut h,
        "Add an observed value",
        "Serve customer · Utilisation",
    );
    type_into(&mut h, "Observed value", "75");
    let share = observed(&h)[0].clone();
    assert_eq!(share["metric"], "utilisation");
    assert_eq!(share["value"].as_f64(), Some(0.75), "{share}");
}

#[test]
fn the_results_say_whether_the_run_agrees_with_the_observed_data() {
    let mut h = mm1();
    let serve = id_of(&h, "Serve customer");
    let source = id_of(&h, "Customer arrives");
    run(&mut h);
    // Without observed data the results say nothing of it.
    assert!(h.query_by_label("Checked against observed data").is_none());
    let mean = h
        .state()
        .results()
        .and_then(|r| {
            r["elements"]
                .as_array()?
                .iter()
                .find(|e| e["id"] == serve.as_str())?["metrics"]["mean_wait"]["mean"]
                .as_f64()
        })
        .expect("the simulated mean wait");

    // One that agrees (it is the simulated mean), one far off, and one for a result a source does
    // not have; the far one is given last, and still comes first.
    let state = h.state_mut();
    state.execute(
        "observation.add",
        json!({ "check": "value", "element": serve, "metric": "mean_wait", "value": mean,
                "tolerance": 100.0 }),
    );
    state.execute(
        "observation.add",
        json!({ "check": "at_least", "element": serve, "metric": "mean_queue_length", "bound": 250.0 }),
    );
    state.execute(
        "observation.add",
        json!({ "check": "at_most", "element": source, "metric": "utilisation", "bound": 0.9 }),
    );
    settle(&mut h);
    run(&mut h);

    let results = h.state().results().expect("current results").clone();
    let verdicts: Vec<&str> = results["observed"]
        .as_array()
        .expect("the checks")
        .iter()
        .filter_map(|c| c["verdict"].as_str())
        .collect();
    assert_eq!(verdicts, ["fails", "no_result", "agrees"]);

    // In words, the section and a finding for the one that fails.
    h.get_by_label("Checked against observed data");
    // (Said twice: as the finding, and in the section.)
    assert_eq!(
        h.query_all_by_label_contains(
            "Mean queue length (Serve customer): must be at least 250, simulated"
        )
        .count(),
        2
    );
    h.get_by_label_contains("Mean wait (Serve customer): observed");
    h.get_by_label_contains("Simulated minus observed: 0");
    assert_eq!(h.query_all_by_label("Finding").count(), 1);
    // The finding and the row both say Fails; the others say what they found.
    assert_eq!(h.query_all_by_label("Fails").count(), 2);
    h.get_by_label("Agrees within ±100 min");
    h.get_by_label("No result");
    // The one that fails comes first, then the one with no result, then the one that agrees.
    let top = |label: &str| h.get_all_by_label(label).next().map(|n| n.rect().top());
    let (fails, none, agrees) = (
        top("Fails"),
        top("No result"),
        top("Agrees within ±100 min"),
    );
    assert!(
        fails < none && none < agrees && fails.is_some(),
        "{fails:?} {none:?} {agrees:?}"
    );

    // A bound the interval straddles is inconclusive, and an observed value with no tolerance
    // that the interval holds is no difference detected, which says it is not agreement.
    let mut h = mm1();
    let serve = id_of(&h, "Serve customer");
    run(&mut h);
    let mean = h
        .state()
        .results()
        .and_then(|r| {
            r["elements"]
                .as_array()?
                .iter()
                .find(|e| e["id"] == serve.as_str())?["metrics"]["mean_wait"]["mean"]
                .as_f64()
        })
        .expect("the simulated mean wait");
    let queue = h
        .state()
        .results()
        .and_then(|r| {
            r["elements"]
                .as_array()?
                .iter()
                .find(|e| e["id"] == serve.as_str())?["metrics"]["mean_queue_length"]["mean"]
                .as_f64()
        })
        .expect("the simulated mean queue length");
    h.state_mut().execute(
        "observation.add",
        json!({ "check": "at_most", "element": serve, "metric": "mean_wait", "bound": mean }),
    );
    h.state_mut().execute(
        "observation.add",
        json!({ "check": "value", "element": serve, "metric": "mean_queue_length",
                "value": queue }),
    );
    settle(&mut h);
    run(&mut h);
    h.get_by_label("Inconclusive at current precision");
    h.get_by_label("No difference detected");
    h.get_by_label_contains("not evidence of agreement");
    assert!(h.query_by_label_contains("Agrees").is_none());
    assert!(h.query_by_label("Finding").is_none());
}
