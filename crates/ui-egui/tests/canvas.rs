//! The flowchart canvas, driven with the pointer and the keyboard as a user would: moving
//! shapes, connecting them from a shape's handle, selecting several and aligning them, copy and
//! paste, delete, and zoom. Each change is one undo step.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui::{Modifiers, Pos2, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use serde_json::json;
use stochcraft_model::{Bounds, ElementId};
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

fn harness() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    h
}

/// The M/M/1 example (arrive → serve → leave), which has no diagram of its own.
fn mm1() -> Harness<'static, App> {
    let mut h = harness();
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h
}

/// The screen rectangle of the shape drawn for the element named `name`.
fn shape(h: &Harness<'static, App>, name: &str) -> egui::Rect {
    h.query_all(By::new().role(Role::Button))
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l != name && l.ends_with(&format!(" {name}")))
        })
        .unwrap_or_else(|| panic!("no shape for {name}"))
        .rect()
}

fn placed(h: &Harness<'static, App>, id: &str) -> Bounds {
    let model = h.state().engine().model();
    model
        .placed()
        .get(&ElementId::unchecked(id))
        .copied()
        .unwrap_or_else(|| panic!("{id} is not drawn"))
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

fn click_at(h: &mut Harness<'static, App>, pos: Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.hover_at(pos);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    settle(h);
}

fn command(h: &mut Harness<'static, App>, id: &str) {
    h.state_mut().run_command(&CommandRef::new(id));
    settle(h);
}

fn ids(h: &Harness<'static, App>) -> Vec<String> {
    h.state()
        .engine()
        .model()
        .elements
        .iter()
        .map(|e| e.id.as_str().to_owned())
        .collect()
}

#[test]
fn a_dragged_shape_moves_in_one_step_and_the_others_stay() {
    let mut h = mm1();
    let before = h.state().engine().model().placed();
    assert!(h.state().engine().model().layout.is_empty());
    let r = shape(&h, "Serve customer");
    drag(&mut h, r.center(), r.center() + Vec2::new(80.0, 60.0));
    let serve = placed(&h, "serve");
    let was = before[&ElementId::unchecked("serve")];
    assert!(serve.x > was.x && serve.y > was.y, "{was:?} → {serve:?}");
    // On the grid.
    assert_eq!((serve.x % 10.0, serve.y % 10.0), (0.0, 0.0));
    // The other shapes are fixed where they were drawn, not moved.
    for id in ["arrive", "leave"] {
        assert_eq!(placed(&h, id), before[&ElementId::unchecked(id)], "{id}");
    }
    assert!(h.state().has_unsaved_changes());
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Move");
    assert!(h.state().engine().model().layout.is_empty());
}

#[test]
fn dragging_from_a_handle_connects_two_blocks() {
    let mut h = harness();
    h.state_mut().execute("model.new", json!({ "name": "Two" }));
    h.state_mut().execute(
        "block.add",
        json!({ "kind": "source", "interarrival": { "type": "exponential", "mean": 1.0 } }),
    );
    h.state_mut()
        .execute("block.add", json!({ "kind": "sink" }));
    settle(&mut h);
    let from = shape(&h, "source-1");
    let to = shape(&h, "sink-1");
    h.hover_at(from.center());
    settle(&mut h);
    let handle = h
        .get_by_role_and_label(Role::Button, "Connect from source-1")
        .rect();
    drag(&mut h, handle.center(), to.center());
    let c = &h.state().engine().model().connections;
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!(
        (c[0].from.as_str(), c[0].to.as_str()),
        ("source-1", "sink-1")
    );
}

#[test]
fn several_blocks_are_selected_aligned_and_deleted_together() {
    let mut h = mm1();
    // Move the exit down first, so aligning has something to do.
    let r = shape(&h, "Customer leaves");
    drag(&mut h, r.center(), r.center() + Vec2::new(0.0, 120.0));
    let arrive = shape(&h, "Customer arrives");
    let leave = shape(&h, "Customer leaves");
    click_at(&mut h, arrive.center(), Modifiers::NONE);
    click_at(&mut h, leave.center(), Modifiers::SHIFT);
    h.get_by_label("2 blocks selected");
    h.get_by_role_and_label(Role::Button, "Tops").click();
    settle(&mut h);
    assert_eq!(placed(&h, "arrive").y, placed(&h, "leave").y);
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Align");
    assert_ne!(placed(&h, "arrive").y, placed(&h, "leave").y);

    // Delete takes both (and their connections) in one step.
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    assert_eq!(ids(&h), ["serve"]);
    assert!(h.state().engine().model().connections.is_empty());
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(ids(&h), ["arrive", "serve", "leave"]);
    assert_eq!(h.state().engine().model().connections.len(), 2);
}

#[test]
fn copies_paste_with_new_ids_and_their_connections() {
    let mut h = mm1();
    command(&mut h, commands::SELECT_ALL);
    h.get_by_label("3 blocks selected");
    command(&mut h, commands::COPY);
    command(&mut h, commands::PASTE);
    assert_eq!(
        ids(&h),
        [
            "arrive",
            "serve",
            "leave",
            "arrive-copy",
            "serve-copy",
            "leave-copy"
        ]
    );
    let model = h.state().engine().model();
    assert_eq!(model.connections.len(), 4);
    assert!(
        model
            .connections
            .iter()
            .any(|c| c.from.as_str() == "serve-copy" && c.to.as_str() == "leave-copy")
    );
    // Down and to the right of the originals; the pasted blocks are selected.
    let (a, b) = (placed(&h, "serve"), placed(&h, "serve-copy"));
    assert_eq!((b.x - a.x, b.y - a.y), (30.0, 30.0));
    h.get_by_label("3 blocks selected");
    // One step to undo, and a duplicate is another.
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(h.state().status(), "Undid Paste");
    assert_eq!(ids(&h).len(), 3);
    let serve = shape(&h, "Serve customer").center();
    click_at(&mut h, serve, Modifiers::NONE);
    command(&mut h, commands::DUPLICATE);
    assert_eq!(ids(&h), ["arrive", "serve", "leave", "serve-copy"]);
}

#[test]
fn the_view_zooms_and_a_click_on_the_background_selects_nothing() {
    let mut h = mm1();
    let serve = shape(&h, "Serve customer").center();
    click_at(&mut h, serve, Modifiers::NONE);
    h.get_by_role_and_label(Role::TextInput, "Capacity");
    let canvas = h.get_by_role_and_label(Role::Canvas, "Diagram").rect();
    click_at(
        &mut h,
        canvas.left_bottom() + Vec2::new(8.0, -8.0),
        Modifiers::NONE,
    );
    assert!(
        h.query_by_role_and_label(Role::TextInput, "Capacity")
            .is_none()
    );

    let zoom = |h: &Harness<'static, App>| {
        h.query_all(By::new().role(Role::Button))
            .filter_map(|n| n.accesskit_node().label())
            .find(|l| l.ends_with('%'))
            .unwrap_or_default()
    };
    let at = zoom(&h);
    h.get_by_role_and_label(Role::Button, "+").click();
    settle(&mut h);
    assert_ne!(zoom(&h), at);
    h.get_by_role_and_label(Role::Button, &zoom(&h)).click();
    settle(&mut h);
    assert_eq!(zoom(&h), "100%");
}
