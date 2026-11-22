//! What frames the window asks for and whether its widgets keep who they are between frames.
//!
//! - Idle: once a view has settled (nothing running or moving), drawing it again must not ask
//!   for another frame, or the window keeps the processor busy while nothing changes. Checked on
//!   the start page, the canvas, the 3D view and the results of a finished run.
//! - Identity: an error appearing under a field, and going again, must leave the fields' ids
//!   (and so their AccessKit nodes) as they were, and the field being typed in keeps the
//!   keyboard. An automatic id counts the widgets before it, so a line inserted above a field
//!   gives it a new id, and a screen reader and egui's focus both lose it. Checked on a
//!   decision's ways out, a task's parameters and a system dynamics variable's panel.
//!
//! Each test gathers every problem it finds before failing, so one run reports them all.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use std::time::Duration;

use egui::accesskit::{NodeId, Role};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn harness() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    h
}

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The M/M/1 example (arrive → serve → leave).
fn mm1() -> Harness<'static, App> {
    let mut h = harness();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h
}

// --- Idle ---

/// Whether the view the harness shows settles, and then stays quiet for a few more frames:
/// what it got wrong, with egui's record of where the repaints were asked for.
fn idle_problems(h: &mut Harness<'static, App>, view: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if let Err(e) = h.try_run() {
        problems.push(format!("{view}: does not settle: {e}"));
    }
    for frame in 1..=3 {
        h.step();
        let delay = h
            .output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay);
        if delay != Some(Duration::MAX) {
            problems.push(format!(
                "{view}: frame {frame} after settling asks for another in {delay:?}; asked at {:?}",
                h.ctx
                    .repaint_causes()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            ));
            break;
        }
    }
    problems
}

fn assert_none(problems: &[String]) {
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_start_page_is_idle_once_drawn() {
    let mut h = harness();
    assert_none(&idle_problems(&mut h, "start page"));
}

#[test]
fn the_canvas_is_idle_once_drawn() {
    let mut h = mm1();
    assert_none(&idle_problems(&mut h, "canvas"));
}

#[test]
fn the_3d_view_is_idle_once_drawn() {
    let mut h = mm1();
    h.get_by_role_and_label(Role::Tab, "3D").click();
    settle(&mut h);
    assert!(h.query_by_label("3D view of the model").is_some());
    assert_none(&idle_problems(&mut h, "3D view"));
}

#[test]
fn the_results_of_a_finished_run_are_idle_once_drawn() {
    let mut h = mm1();
    h.state_mut().run_command(&CommandRef::new(commands::RUN));
    h.step();
    for _ in 0..5000 {
        if !h.state().is_running() {
            break;
        }
        h.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    settle(&mut h);
    assert!(h.state().results().is_some(), "{}", h.state().status());
    assert!(h.query_all_by_label("± 95% CI").count() >= 1);
    assert_none(&idle_problems(&mut h, "results"));
}

// --- Identity ---

/// The fields on screen, in order: label and AccessKit node id.
fn fields(h: &Harness<'static, App>) -> Vec<(String, NodeId)> {
    [Role::TextInput, Role::ComboBox, Role::CheckBox]
        .into_iter()
        .flat_map(|role| h.get_all_by_role(role).collect::<Vec<_>>())
        .map(|n| {
            let n = n.accesskit_node();
            (n.label().unwrap_or_default(), n.locate().0)
        })
        .collect()
}

/// Compares the fields now with `before`, adding what moved to `problems`.
fn same_fields(
    h: &Harness<'static, App>,
    before: &[(String, NodeId)],
    when: &str,
    problems: &mut Vec<String>,
) {
    let now = fields(h);
    if now != before {
        let moved: Vec<String> = before
            .iter()
            .filter(|f| !now.contains(f))
            .map(|(label, _)| format!("{label:?}"))
            .collect();
        problems.push(format!(
            "{when}: fields changed id: {} (before {} fields, now {})",
            moved.join(", "),
            before.len(),
            now.len()
        ));
    }
}

fn focused(h: &Harness<'static, App>, label: &str, when: &str, problems: &mut Vec<String>) {
    if !h.get_by_role_and_label(Role::TextInput, label).is_focused() {
        problems.push(format!("{when}: {label:?} lost the keyboard"));
    }
}

/// Whether a widget is outside the inspector's visible part of the 1200 × 900 window.
fn off_screen(r: egui::Rect) -> bool {
    r.top() < 110.0 || r.bottom() > 820.0
}

/// Clicks the field labelled `label`, scrolled into view first.
fn click_field(h: &mut Harness<'static, App>, label: &str) {
    if off_screen(h.get_by_role_and_label(Role::TextInput, label).rect()) {
        h.get_by_role_and_label(Role::TextInput, label)
            .scroll_to_me();
        settle(h);
    }
    h.get_by_role_and_label(Role::TextInput, label).click();
    settle(h);
}

/// Moves the keyboard to the field labelled `label`, as a screen reader or Tab would: by its
/// node, not by a click where it was drawn (a line appearing above it moves it down).
fn focus_field(h: &mut Harness<'static, App>, label: &str) {
    h.get_by_role_and_label(Role::TextInput, label).focus();
    settle(h);
}

/// Replaces the text of the field labelled `label`, without Enter.
fn type_without_enter(h: &mut Harness<'static, App>, label: &str, text: &str) {
    click_field(h, label);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    settle(h);
}

fn type_into(h: &mut Harness<'static, App>, label: &str, text: &str) {
    type_without_enter(h, label, text);
    h.key_press(egui::Key::Enter);
    settle(h);
}

fn model(h: &Harness<'static, App>) -> Value {
    stochcraft_engine::wire::model(h.state().engine().model())
}

/// A refused probability on a decision's first way out puts a line under it; the second way's
/// fields, below that line, keep their ids, and the one clicked keeps the keyboard.
#[test]
fn a_refused_probability_leaves_the_other_ways_fields_as_they_were() {
    let mut h = harness();
    let app = h.state_mut();
    app.execute("model.new", json!({ "name": "Triage" }));
    for (kind, id, name) in [
        ("source", "in", "Arrivals"),
        ("exclusive_gateway", "check", "Decision 1"),
        ("delay", "a", "Task A"),
        ("delay", "b", "Task B"),
        ("sink", "out", "Departures"),
    ] {
        app.execute("block.add", json!({ "kind": kind, "id": id, "name": name }))
            .expect("a block");
    }
    for (from, to) in [
        ("in", "check"),
        ("check", "a"),
        ("check", "b"),
        ("a", "out"),
        ("b", "out"),
    ] {
        app.execute("connect", json!({ "from": from, "to": to }))
            .expect("a connection");
    }
    settle(&mut h);
    let flow_to = |h: &Harness<'static, App>, to: &str| {
        model(h)["connections"]
            .as_array()
            .and_then(|cs| {
                cs.iter()
                    .find(|c| c["from"] == "check" && c["to"] == to)
                    .and_then(|c| c["id"].as_str().map(str::to_owned))
            })
            .expect("a way out of the decision")
    };
    let (first, second) = (flow_to(&h, "a"), flow_to(&h, "b"));
    h.get_by_role_and_label(Role::ListBoxOption, "Decision 1")
        .click();
    settle(&mut h);
    let p1 = format!("Probability of {first}");
    let p2 = format!("Probability of {second}");
    let before = fields(&h);
    assert!(
        before.iter().any(|(l, _)| *l == p2),
        "no field {p2:?} in {before:?}"
    );

    let mut problems = Vec::new();
    // 1.5 typed into the first way, then moving to the second commits it: refused.
    type_without_enter(&mut h, &p1, "1.5");
    focus_field(&mut h, &p2);
    h.get_by_label_contains("Must be from 0 to 1");
    same_fields(&h, &before, "with the refusal shown", &mut problems);
    focused(&h, &p2, "with the refusal shown", &mut problems);
    // Fixed, the line goes.
    type_into(&mut h, &p1, "0.4");
    assert!(h.query_by_label_contains("Must be from 0 to 1").is_none());
    same_fields(&h, &before, "after the refusal went", &mut problems);
    assert_none(&problems);
}

/// A refused number under a task's capacity: the fields below keep their ids, and the one
/// clicked keeps the keyboard.
#[test]
fn a_refused_capacity_leaves_the_tasks_other_fields_as_they_were() {
    let mut h = mm1();
    h.get_by_role_and_label(Role::ListBoxOption, "Serve customer")
        .click();
    settle(&mut h);
    let before = fields(&h);
    let mut problems = Vec::new();
    type_without_enter(&mut h, "Capacity", "0");
    focus_field(&mut h, "Mean");
    h.get_by_label_contains("Must be 1 or more");
    same_fields(&h, &before, "with the refusal shown", &mut problems);
    focused(&h, "Mean", "with the refusal shown", &mut problems);
    type_into(&mut h, "Capacity", "2");
    assert!(h.query_by_label_contains("Must be 1 or more").is_none());
    same_fields(&h, &before, "after the refusal went", &mut problems);
    assert_none(&problems);
}

/// A refused equation in a system dynamics variable's panel: the unit field below keeps its id,
/// and the keyboard when it is clicked.
#[test]
fn a_refused_equation_leaves_the_panels_other_fields_as_they_were() {
    let mut h = harness();
    let e = EXAMPLES
        .iter()
        .find(|e| e.file == "adoption.xmile")
        .expect("the adoption example");
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h.get_by_role_and_label(Role::Tab, "Diagram").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Auxiliary innovation")
        .focus();
    settle(&mut h);
    // What the engine says to the equation, shown under the field when it is refused.
    let refusal = h
        .state_mut()
        .control(
            "sd.variable.set",
            json!({ "name": "innovation", "equation": "no_such_name + 1" }),
        )
        .expect_err("an unknown name is refused")
        .message;
    let before = fields(&h);
    let mut problems = Vec::new();
    type_without_enter(&mut h, "Equation", "no_such_name + 1");
    focus_field(&mut h, "Unit");
    if h.query_all_by_label_contains(&refusal).count() == 0 {
        problems.push(format!("the refusal {refusal:?} is not shown"));
    }
    same_fields(&h, &before, "with the refusal shown", &mut problems);
    focused(&h, "Unit", "with the refusal shown", &mut problems);
    type_into(&mut h, "Equation", "0.03");
    same_fields(&h, &before, "after the refusal went", &mut problems);
    assert_none(&problems);
}
