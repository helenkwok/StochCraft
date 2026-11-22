//! The diagram of a system dynamics model, driven with the pointer and the keyboard as a user would:
//! the example's five variables drawn with their labels, a stock dragged to a new place (one
//! command, one undo step), a stock and a flow added, a variable that is in use refused when it is
//! deleted, Tab moving the selection, and an equation edited in the side panel (egui_kittest,
//! through the accessibility tree).

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui::{Modifiers, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The adoption example open on its Diagram tab.
fn diagram() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    let e = EXAMPLES
        .iter()
        .find(|e| e.file == "adoption.xmile")
        .expect("the adoption example");
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h.get_by_role_and_label(Role::Tab, "Diagram").click();
    settle(&mut h);
    h
}

/// The screen rectangle of the shape of the variable `label` ("Stock Customers").
fn shape(h: &Harness<'static, App>, label: &str) -> egui::Rect {
    h.get_by_role_and_label(Role::Button, label).rect()
}

/// Presses at `from`, moves to `to` in steps and lets go.
fn drag(h: &mut Harness<'static, App>, from: Pos2, to: Pos2) {
    h.hover_at(from);
    settle(h);
    h.drag_at(from);
    h.run_steps(1);
    for f in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(from + (to - from) * f);
        h.run_steps(1);
    }
    h.drop_at(to);
    settle(h);
}

fn click_at(h: &mut Harness<'static, App>, pos: Pos2) {
    h.hover_at(pos);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    settle(h);
}

/// The model as `sd.get` describes it.
fn info(h: &mut Harness<'static, App>) -> Value {
    h.state_mut().control("sd.get", json!({})).expect("sd.get")
}

fn variable(h: &mut Harness<'static, App>, name: &str) -> Value {
    info(h)["variables"]
        .as_array()
        .and_then(|vs| vs.iter().find(|v| v["name"] == name))
        .cloned()
        .unwrap_or(Value::Null)
}

/// The labels of the changes Undo would take back, oldest first.
fn undo_labels(h: &mut Harness<'static, App>) -> Vec<String> {
    let history = h
        .state_mut()
        .control("history", json!({}))
        .expect("history");
    history["undo"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str().map(str::to_owned))
        .collect()
}

/// Replaces the text of the field labelled `label` and presses Enter.
fn type_into(h: &mut Harness<'static, App>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label).click();
    settle(h);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    settle(h);
    h.key_press(egui::Key::Enter);
    settle(h);
}

#[test]
fn the_diagram_shows_the_five_variables_with_their_labels() {
    let h = diagram();
    for label in [
        "Stock Customers",
        "Stock Potential customers",
        "Flow adoption",
        "Auxiliary innovation",
        "Auxiliary imitation",
    ] {
        assert!(
            h.query_by_role_and_label(Role::Button, label).is_some(),
            "no shape labelled {label:?}"
        );
    }
    assert!(h.query_by_role_and_label(Role::Canvas, "Diagram").is_some());
    // The tools, and nothing selected yet.
    for tool in ["Select", "Stock", "Flow", "Auxiliary", "Link"] {
        assert!(
            h.query_by_role_and_label(Role::Button, tool).is_some(),
            "no tool {tool:?}"
        );
    }
    assert_eq!(h.state().sd_selected(), None);
    h.get_by_label_contains("Click a variable to select it");
}

#[test]
fn a_dragged_stock_moves_with_one_command_and_undo_puts_it_back() {
    let mut h = diagram();
    let before = variable(&mut h, "Customers");
    let steps = undo_labels(&mut h).len();
    let r = shape(&h, "Stock Customers");
    drag(&mut h, r.center(), r.center() + Vec2::new(0.0, 90.0));

    let after = variable(&mut h, "Customers");
    assert!(
        after["y"].as_f64() > before["y"].as_f64(),
        "{before} → {after}"
    );
    assert_eq!(after["x"], before["x"]);
    // One command, one undo step, with a name Undo shows.
    let labels = undo_labels(&mut h);
    assert_eq!(labels.len(), steps + 1, "{labels:?}");
    assert_eq!(labels.last().map(String::as_str), Some("sd.layout.set"));
    assert_eq!(h.state().status(), "Moved Customers");
    // The pipe of the flow that enters the stock follows it.
    let pipe = variable(&mut h, "adoption")["pipe"].clone();
    let last = pipe
        .as_array()
        .and_then(|p| p.last())
        .expect("the pipe's last point");
    let (ex, ey) = (
        last[0].as_f64().expect("pipe x"),
        last[1].as_f64().expect("pipe y"),
    );
    let (cx, cy) = (
        after["x"].as_f64().expect("x"),
        after["y"].as_f64().expect("y"),
    );
    let (hw, hh) = (
        after["width"].as_f64().expect("width") / 2.0,
        after["height"].as_f64().expect("height") / 2.0,
    );
    // The pipe ends on the stock's outline: on a side or on the top or bottom edge.
    let tol = 1.0;
    let on_side = ((ex - cx).abs() - hw).abs() <= tol && (ey - cy).abs() <= hh + tol;
    let on_edge = ((ey - cy).abs() - hh).abs() <= tol && (ex - cx).abs() <= hw + tol;
    assert!(
        on_side || on_edge,
        "the pipe ends at ({ex}, {ey}), off the stock at ({cx}, {cy}) of {hw}x{hh} (half sizes)"
    );

    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Change Diagram");
    assert_eq!(variable(&mut h, "Customers")["y"], before["y"]);
    assert_eq!(undo_labels(&mut h).len(), steps);
}

#[test]
fn adding_a_stock_and_a_flow_between_two_stocks_updates_their_flow_lists() {
    let mut h = diagram();
    let steps = undo_labels(&mut h).len();
    // The Stock tool, then a click on empty ground.
    h.get_by_role_and_label(Role::Button, "Stock").click();
    settle(&mut h);
    let canvas = h.get_by_role_and_label(Role::Canvas, "Diagram").rect();
    click_at(
        &mut h,
        Pos2::new(canvas.left() + 100.0, canvas.bottom() - 120.0),
    );
    let added = variable(&mut h, "Stock 1");
    assert_eq!(added["kind"], "stock", "{added}");
    assert_eq!(undo_labels(&mut h).len(), steps + 1);
    assert_eq!(h.state().sd_selected(), Some("Stock 1"));
    // Placed where it was clicked, and its side panel is open.
    assert!(added["x"].as_f64().is_some_and(|x| x < 300.0));
    h.get_by_role_and_label(Role::TextInput, "Initial value");

    // The Flow tool, dragged from the new stock to Customers.
    h.get_by_role_and_label(Role::Button, "Flow").click();
    settle(&mut h);
    let (from, to) = (shape(&h, "Stock Stock 1"), shape(&h, "Stock Customers"));
    drag(&mut h, from.center(), to.center());
    let flow = variable(&mut h, "Flow 1");
    assert_eq!(flow["kind"], "flow", "{flow}");
    assert_eq!(flow["source"], "Stock 1");
    assert_eq!(flow["sink"], "Customers");
    assert_eq!(undo_labels(&mut h).len(), steps + 2);
    let list = |v: &Value, key: &str| -> Vec<String> {
        v[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n.as_str().map(str::to_owned))
            .collect()
    };
    assert!(list(&variable(&mut h, "Customers"), "inflows").contains(&"Flow 1".to_owned()));
    assert!(list(&variable(&mut h, "Stock 1"), "outflows").contains(&"Flow 1".to_owned()));
    assert!(
        h.query_by_role_and_label(Role::Button, "Flow Flow 1")
            .is_some()
    );

    // One undo takes the flow back, another the stock.
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(variable(&mut h, "Flow 1"), Value::Null);
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(variable(&mut h, "Stock 1"), Value::Null);
}

#[test]
fn deleting_a_variable_that_is_used_is_refused_with_the_message() {
    let mut h = diagram();
    let steps = undo_labels(&mut h).len();
    // What the engine says when asked to remove it.
    let refusal = h
        .state_mut()
        .control("sd.variable.remove", json!({ "name": "Customers" }))
        .expect_err("Customers is used by the flow's equation")
        .message;
    assert_eq!(undo_labels(&mut h).len(), steps);

    h.get_by_role_and_label(Role::Button, "Stock Customers")
        .focus();
    settle(&mut h);
    assert_eq!(h.state().sd_selected(), Some("Customers"));
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    assert!(
        h.state().status().contains(&refusal),
        "{:?} does not say {refusal:?}",
        h.state().status()
    );
    // The panel says it too, and nothing changed.
    assert!(
        h.query_all_by_label_contains(&refusal).count() >= 1,
        "the panel does not say {refusal:?}"
    );
    assert_ne!(variable(&mut h, "Customers"), Value::Null);
    assert_eq!(undo_labels(&mut h).len(), steps);

    // A variable nothing uses goes, in one undo step.
    let added = h
        .state_mut()
        .control(
            "sd.variable.add",
            json!({ "kind": "aux", "name": "spare", "equation": "1", "position": { "x": 300.0, "y": 400.0 } }),
        )
        .is_ok();
    assert!(added);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Auxiliary spare")
        .focus();
    settle(&mut h);
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    assert_eq!(variable(&mut h, "spare"), Value::Null);
    assert_eq!(h.state().status(), "Removed spare");
    assert_eq!(undo_labels(&mut h).len(), steps + 2);
}

#[test]
fn tab_moves_the_selection_through_the_variables_in_name_order() {
    let mut h = diagram();
    h.get_by_role_and_label(Role::Button, "Auxiliary imitation")
        .focus();
    settle(&mut h);
    assert_eq!(h.state().sd_selected(), Some("imitation"));
    // adoption, Customers, imitation, innovation, Potential customers.
    h.key_press(egui::Key::Tab);
    settle(&mut h);
    assert_eq!(h.state().sd_selected(), Some("innovation"));
    h.key_press(egui::Key::Tab);
    settle(&mut h);
    assert_eq!(h.state().sd_selected(), Some("Potential customers"));
    h.key_press_modifiers(Modifiers::SHIFT, egui::Key::Tab);
    settle(&mut h);
    assert_eq!(h.state().sd_selected(), Some("innovation"));
}

#[test]
fn an_equation_typed_without_enter_is_kept_when_the_tab_changes() {
    let mut h = diagram();
    h.get_by_role_and_label(Role::Button, "Auxiliary innovation")
        .focus();
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "Equation").click();
    settle(&mut h);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, "Equation")
        .type_text("0.04");
    settle(&mut h);
    // No Enter: the Results tab is chosen while the field still has the keyboard.
    h.get_by_role_and_label(Role::Tab, "Results").click();
    settle(&mut h);
    assert_eq!(variable(&mut h, "innovation")["equation"], "0.04");
}

#[test]
fn an_equation_is_edited_in_the_side_panel_and_a_bad_one_is_refused_in_the_field() {
    let mut h = diagram();
    h.get_by_role_and_label(Role::Button, "Auxiliary innovation")
        .focus();
    settle(&mut h);
    assert_eq!(variable(&mut h, "innovation")["equation"], "0.03");
    // The panel names the variable (the shape is labelled the same, so two).
    assert!(h.query_all_by_label("Auxiliary innovation").count() >= 2);
    let steps = undo_labels(&mut h).len();

    type_into(&mut h, "Equation", "0.05");
    assert_eq!(variable(&mut h, "innovation")["equation"], "0.05");
    let labels = undo_labels(&mut h);
    assert_eq!(labels.len(), steps + 1);
    assert_eq!(labels.last().map(String::as_str), Some("sd.variable.set"));
    assert_eq!(h.state().status(), "Edited innovation");
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Edit Variable");
    assert_eq!(variable(&mut h, "innovation")["equation"], "0.03");

    // A name the model does not have: refused, with the engine's message under the field, the
    // text left in it, and the model as it was.
    let refusal = h
        .state_mut()
        .control(
            "sd.variable.set",
            json!({ "name": "innovation", "equation": "no_such_name + 1" }),
        )
        .expect_err("an unknown name is refused")
        .message;
    let steps = undo_labels(&mut h).len();
    type_into(&mut h, "Equation", "no_such_name + 1");
    assert!(
        h.query_all_by_label_contains(&refusal).count() >= 1,
        "the field does not say {refusal:?}"
    );
    assert_eq!(variable(&mut h, "innovation")["equation"], "0.03");
    assert_eq!(undo_labels(&mut h).len(), steps);

    // A unit and a new name go through the same panel.
    type_into(&mut h, "Unit", "1/month");
    assert_eq!(
        variable(&mut h, "innovation")["units"],
        "1/month",
        "status {:?}; the panel says {:?}",
        h.state().status(),
        h.get_all_by_role(Role::TextInput)
            .map(|n| {
                let n = n.accesskit_node();
                (n.label(), n.value())
            })
            .collect::<Vec<_>>()
    );
}

/// Types `text` into the field labelled `label` without pressing Enter.
fn type_without_enter(h: &mut Harness<'static, App>, label: &str, text: &str) {
    h.get_by_role_and_label(Role::TextInput, label).click();
    settle(h);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, label)
        .type_text(text);
    settle(h);
}

#[test]
fn a_new_stock_takes_a_name_after_a_pending_rename_is_committed() {
    let mut h = diagram();
    h.get_by_role_and_label(Role::Button, "Auxiliary innovation")
        .focus();
    settle(&mut h);
    type_without_enter(&mut h, "Name", "Stock 1");
    h.get_by_role_and_label(Role::Button, "Stock").click();
    settle(&mut h);
    let canvas = h.get_by_role_and_label(Role::Canvas, "Diagram").rect();
    click_at(
        &mut h,
        Pos2::new(canvas.left() + 100.0, canvas.bottom() - 120.0),
    );
    assert_ne!(variable(&mut h, "Stock 1"), Value::Null);
    assert_eq!(
        variable(&mut h, "Stock 1")["kind"],
        "aux",
        "the renamed one"
    );
    assert_eq!(variable(&mut h, "Stock 2")["kind"], "stock");
    assert_eq!(h.state().sd_selected(), Some("Stock 2"));
}

#[test]
fn a_refused_pending_edit_keeps_the_selection_and_adds_nothing() {
    let mut h = diagram();
    h.get_by_role_and_label(Role::Button, "Auxiliary innovation")
        .focus();
    settle(&mut h);
    let steps = undo_labels(&mut h).len();
    type_without_enter(&mut h, "Equation", "no_such_name + 1");
    h.get_by_role_and_label(Role::Button, "Stock").click();
    settle(&mut h);
    let canvas = h.get_by_role_and_label(Role::Canvas, "Diagram").rect();
    click_at(
        &mut h,
        Pos2::new(canvas.left() + 100.0, canvas.bottom() - 120.0),
    );
    assert_eq!(variable(&mut h, "Stock 1"), Value::Null);
    assert_eq!(undo_labels(&mut h).len(), steps);
    assert_eq!(h.state().sd_selected(), Some("innovation"));
    // The text and the refusal are still in the panel.
    assert_eq!(variable(&mut h, "innovation")["equation"], "0.03");
    h.get_by_role_and_label(Role::TextInput, "Equation");
}

#[test]
fn a_draft_is_not_committed_over_an_external_edit() {
    let mut h = diagram();
    h.get_by_role_and_label(Role::Button, "Auxiliary innovation")
        .focus();
    settle(&mut h);
    type_without_enter(&mut h, "Equation", "0.04");
    h.state_mut()
        .control(
            "sd.variable.set",
            json!({ "name": "innovation", "equation": "0.09" }),
        )
        .expect("an external edit");
    h.get_by_role_and_label(Role::Tab, "Results").click();
    settle(&mut h);
    assert_eq!(variable(&mut h, "innovation")["equation"], "0.09");
}

#[test]
fn a_flow_drag_from_a_stock_renamed_meanwhile_adds_nothing() {
    let mut h = diagram();
    h.get_by_role_and_label(Role::Button, "Flow").click();
    settle(&mut h);
    let r = shape(&h, "Stock Customers");
    let canvas = h.get_by_role_and_label(Role::Canvas, "Diagram").rect();
    let to = Pos2::new(canvas.left() + 80.0, canvas.bottom() - 60.0);
    h.hover_at(r.center());
    settle(&mut h);
    h.drag_at(r.center());
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(1);
    h.state_mut()
        .control(
            "sd.variable.set",
            json!({ "name": "Customers", "rename": "Clients" }),
        )
        .expect("a rename");
    h.run_steps(2);
    h.drop_at(to);
    settle(&mut h);
    assert_eq!(variable(&mut h, "Flow 1"), Value::Null);
    assert_ne!(variable(&mut h, "Clients"), Value::Null);
}

/// A flow drawn from empty space to empty space, with a zoom about the pointer's end in between.
fn cloud_flow(zoom: bool) -> Value {
    let mut h = diagram();
    h.get_by_role_and_label(Role::Button, "Flow").click();
    settle(&mut h);
    let canvas = h.get_by_role_and_label(Role::Canvas, "Diagram").rect();
    let from = Pos2::new(canvas.left() + 60.0, canvas.bottom() - 60.0);
    let to = Pos2::new(canvas.left() + 200.0, canvas.bottom() - 60.0);
    h.hover_at(from);
    settle(&mut h);
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(1);
    if zoom {
        h.event(egui::Event::Zoom(1.5));
        h.run_steps(2);
    }
    h.drop_at(to);
    settle(&mut h);
    variable(&mut h, "Flow 1")
}

#[test]
fn a_zoom_during_a_flow_drag_leaves_the_cloud_where_it_was_pressed() {
    let plain = cloud_flow(false);
    assert_eq!(plain["kind"], "flow", "{plain}");
    assert_eq!(cloud_flow(true), plain);
}
