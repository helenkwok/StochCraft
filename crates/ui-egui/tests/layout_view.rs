//! The Layout view: its tab shows the sorter example's plan to scale, with a scale bar, the
//! blocks' names and the note that positions come from the layout. What is drawn is checked by the
//! unit tests in `layout_view.rs`.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use stochcraft_ui_egui::{App, Platform};

const VIEW: &str = "Layout of the model, to scale";

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

fn sorter() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1500.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h.state_mut().open_file(
        "sorter.bpmn",
        include_bytes!("../../../tests/models/sorter.bpmn"),
    );
    settle(&mut h);
    h
}

fn has(h: &Harness<'static, App>, label: &str) -> bool {
    !h.query_all_by_label(label).collect::<Vec<_>>().is_empty()
}

#[test]
fn the_layout_tab_shows_the_scale_bar_and_the_blocks() {
    let mut h = sorter();
    assert!(!has(&h, VIEW), "the view is on its own tab");
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    settle(&mut h);
    assert!(has(&h, VIEW));
    assert!(has(&h, "Positions from the layout"));
    // Every block of the sorter has a place in its layout, so none is listed as unplaced.
    assert!(has(&h, "Every block is placed."));
    assert!(has(&h, "Unplaced"));
    assert!(
        !h.query_all_by_label_contains("Scale bar: ")
            .collect::<Vec<_>>()
            .is_empty()
    );
    // A block's name, written over its footprint.
    assert!(
        !h.query_all_by_label_contains("Infeed")
            .collect::<Vec<_>>()
            .is_empty()
    );
    h.get_by_role_and_label(Role::Button, "Fit").click();
    settle(&mut h);
    assert!(has(&h, VIEW));
    // Back to the model, and across to the 3D view, which keeps its own tab.
    h.get_by_role_and_label(Role::Tab, "3D").click();
    settle(&mut h);
    assert!(!has(&h, VIEW));
    assert!(has(&h, "3D view of the model"));
}

/// The track of conveyor `id`, as its outline's points (`None` without a track).
fn points(h: &Harness<'static, App>, id: &str) -> Option<Vec<[f64; 2]>> {
    let model = h.state().engine().model();
    let track = model
        .site
        .paths
        .get(&stochcraft_model::ElementId::unchecked(id))?;
    Some(track.path.outline().expect("a flat path").points)
}

fn click_at(h: &mut Harness<'static, App>, pos: egui::Pos2) {
    h.hover_at(pos);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    settle(h);
}

fn on_layout(h: &mut Harness<'static, App>, select: &str) {
    h.get_by_role_and_label(Role::Tab, "Layout").click();
    settle(h);
    h.state_mut()
        .select(std::iter::once(select.to_owned()).collect());
    settle(h);
}

#[test]
fn a_path_is_drawn_by_clicks_and_enter_and_undone_in_one_step() {
    let mut h = sorter();
    on_layout(&mut h, "lane_blue");
    let before = points(&h, "lane_blue");
    h.get_by_role_and_label(Role::Button, "Draw path").click();
    settle(&mut h);
    assert!(
        !h.query_all_by_label_contains("Drawing the path of Lane blue")
            .collect::<Vec<_>>()
            .is_empty()
    );
    let canvas = h.get_by_role_and_label(Role::Canvas, VIEW).rect();
    let at = |fx: f32, fy: f32| canvas.min + egui::vec2(canvas.width() * fx, canvas.height() * fy);
    for spot in [at(0.2, 0.8), at(0.5, 0.8), at(0.5, 0.6)] {
        click_at(&mut h, spot);
    }
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    let drawn = points(&h, "lane_blue").expect("a track");
    assert_eq!(drawn.len(), 3, "{drawn:?}");
    assert_ne!(Some(drawn), before);
    // The conveyor's length is the drawn path's.
    let model = h.state().engine().model();
    let id = stochcraft_model::ElementId::unchecked("lane_blue");
    let path_length = model.site.paths[&id].length(model.experiment.length_unit);
    match &model.element(&id).expect("the lane").kind {
        stochcraft_model::ElementKind::Conveyor { length, .. } => {
            assert_eq!(*length, path_length);
        }
        other => panic!("{other:?}"),
    }
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(points(&h, "lane_blue"), before);
}

#[test]
fn a_point_is_dragged_and_moved_by_the_keys() {
    let mut h = sorter();
    on_layout(&mut h, "lane_red");
    let before = points(&h, "lane_red").expect("a track");
    assert_eq!(before.len(), 2);
    // The lane's end, a control a screen reader can find.
    let end = h
        .get_by_label_contains("Point 2 of Lane red")
        .rect()
        .center();
    h.hover_at(end);
    settle(&mut h);
    h.drag_at(end);
    h.run_steps(1);
    for f in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(end + egui::vec2(60.0 * f, 0.0));
        h.run_steps(1);
    }
    h.drop_at(end + egui::vec2(60.0, 0.0));
    settle(&mut h);
    let moved = points(&h, "lane_red").expect("a track");
    assert!(moved[1][0] > before[1][0] + 0.1, "{moved:?}");
    assert!((moved[0][0] - before[0][0]).abs() < 1e-9, "the start stays");
    // One undo step puts it back.
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(points(&h, "lane_red").expect("a track"), before);
    // Picked and given the keyboard, the end moves 0.1 m right with the arrow key.
    h.get_by_label_contains("Point 2 of Lane red").click();
    settle(&mut h);
    h.key_press(egui::Key::ArrowRight);
    settle(&mut h);
    let nudged = points(&h, "lane_red").expect("a track");
    assert!(
        (nudged[1][0] - before[1][0] - 0.1).abs() < 1e-6,
        "{nudged:?}"
    );
}

/// Drags from `from` by `by` in four held steps, the last `held` of the way, then lets go at
/// `from + by` (in the same frame as any movement left).
fn drag_by(h: &mut Harness<'static, App>, from: egui::Pos2, by: egui::Vec2, held: f32) {
    h.hover_at(from);
    settle(h);
    h.drag_at(from);
    h.run_steps(1);
    for f in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(from + by * (held * f));
        h.run_steps(1);
    }
    h.drop_at(from + by);
    settle(h);
}

fn point_of(h: &Harness<'static, App>, label: &str) -> egui::Pos2 {
    h.get_by_label_contains(label).rect().center()
}

#[test]
fn a_point_and_the_belt_go_where_they_are_let_go() {
    let mut h = sorter();
    on_layout(&mut h, "lane_red");
    let before = points(&h, "lane_red").expect("a track");
    let end = point_of(&h, "Point 2 of Lane red");
    let by = egui::vec2(60.0, 0.0);
    // Held all the way, then let go where the pointer already is.
    drag_by(&mut h, end, by, 1.0);
    let all_the_way = points(&h, "lane_red").expect("a track");
    h.state_mut().undo();
    settle(&mut h);
    // Held half way, then let go 30 px further on in the same frame: the same place.
    drag_by(&mut h, end, by, 0.5);
    let let_go = points(&h, "lane_red").expect("a track");
    assert!(all_the_way[1][0] > before[1][0] + 0.1, "{all_the_way:?}");
    assert_eq!(let_go, all_the_way);
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(points(&h, "lane_red").expect("a track"), before);

    // The whole belt, grabbed a quarter of the way along it, likewise.
    let start = point_of(&h, "Point 1 of Lane red");
    let grab = start + (end - start) * 0.25;
    drag_by(&mut h, grab, by, 1.0);
    let moved = points(&h, "lane_red").expect("a track");
    let shift = moved[0][0] - before[0][0];
    assert!(shift > 0.1, "{moved:?}");
    assert!(
        ((moved[1][0] - before[1][0]) - shift).abs() < 1e-6,
        "{moved:?}"
    );
    h.state_mut().undo();
    settle(&mut h);
    drag_by(&mut h, grab, by, 0.5);
    assert_eq!(points(&h, "lane_red").expect("a track"), moved);
}

#[test]
fn shift_and_an_arrow_move_a_point_a_metre() {
    let mut h = sorter();
    on_layout(&mut h, "lane_red");
    let before = points(&h, "lane_red").expect("a track");
    h.get_by_label_contains("Point 2 of Lane red").click();
    settle(&mut h);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
    settle(&mut h);
    let nudged = points(&h, "lane_red").expect("a track");
    assert!(
        (nudged[1][0] - before[1][0] - 1.0).abs() < 1e-6,
        "{nudged:?}"
    );
}

#[test]
fn a_track_set_during_a_drag_ends_it_and_is_kept() {
    let mut h = sorter();
    on_layout(&mut h, "lane_red");
    let end = point_of(&h, "Point 2 of Lane red");
    h.hover_at(end);
    settle(&mut h);
    h.drag_at(end);
    h.run_steps(1);
    for f in [0.25, 0.5] {
        h.hover_at(end + egui::vec2(60.0 * f, 0.0));
        h.run_steps(1);
    }
    // An agent draws the lane anew while the point is held.
    h.state_mut()
        .control(
            "track.set",
            serde_json::json!({ "id": "lane_red", "track": {
                "start": { "x": 0.0, "y": 0.0 },
                "segments": [{ "type": "line", "length": 3.0 }]
            } }),
        )
        .expect("track.set");
    h.run_steps(1);
    h.hover_at(end + egui::vec2(60.0, 0.0));
    h.run_steps(1);
    h.drop_at(end + egui::vec2(60.0, 0.0));
    settle(&mut h);
    assert_eq!(
        points(&h, "lane_red").expect("a track"),
        vec![[0.0, 0.0], [3.0, 0.0]]
    );
}

/// Scrolls the widget with `role` and `label` into view, if the inspector has it off screen.
fn into_view(h: &mut Harness<'static, App>, role: Role, label: &str) {
    let r = h.get_by_role_and_label(role, label).rect();
    if r.top() < 110.0 || r.bottom() > 820.0 {
        h.get_by_role_and_label(role, label).scroll_to_me();
        settle(h);
    }
}

#[test]
fn removing_a_path_drops_a_coordinate_typed_and_not_entered() {
    let mut h = sorter();
    h.state_mut()
        .select(std::iter::once("lane_red".to_owned()).collect());
    settle(&mut h);
    assert!(points(&h, "lane_red").is_some());
    into_view(&mut h, Role::TextInput, "x of point 2");
    h.get_by_role_and_label(Role::TextInput, "x of point 2")
        .click();
    settle(&mut h);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, "x of point 2")
        .type_text("12");
    settle(&mut h);
    // The click on Remove path takes the keyboard from the field, which commits its text in the
    // frame the button is clicked: the path stays removed all the same.
    into_view(&mut h, Role::Button, "Remove path");
    h.get_by_role_and_label(Role::Button, "Remove path").click();
    settle(&mut h);
    assert_eq!(points(&h, "lane_red"), None);
}

/// Sets conveyor `id`'s track from `start` (x, y, heading) through `segments`, as an agent would.
fn set_track(
    h: &mut Harness<'static, App>,
    id: &str,
    start: [f64; 3],
    segments: serde_json::Value,
) {
    h.state_mut()
        .control(
            "track.set",
            serde_json::json!({ "id": id, "track": {
                "start": { "x": start[0], "y": start[1], "heading": start[2] },
                "segments": segments
            } }),
        )
        .expect("track.set");
    settle(h);
}

fn near_points(got: &[[f64; 2]], want: &[[f64; 2]]) {
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(
            (g[0] - w[0]).abs() < 1e-6 && (g[1] - w[1]).abs() < 1e-6,
            "{got:?}"
        );
    }
}

#[test]
fn a_picked_point_is_let_go_when_the_track_changes_under_it() {
    let mut h = sorter();
    on_layout(&mut h, "lane_red");
    // North 2 m, a sharp right turn, east 3 m: (0, 0), (0, 2), (3, 2).
    set_track(
        &mut h,
        "lane_red",
        [0.0, 0.0, 90.0],
        serde_json::json!([
            { "type": "line", "length": 2.0 },
            { "type": "arc", "radius": 0.0, "angle": -90.0 },
            { "type": "line", "length": 3.0 }
        ]),
    );
    h.get_by_role_and_label(Role::Button, "Fit").click();
    settle(&mut h);
    // Picked, moved by the keys, and taken away: the view's own edit keeps the point picked.
    h.get_by_label_contains("Point 2 of Lane red").click();
    settle(&mut h);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowRight);
    settle(&mut h);
    near_points(
        &points(&h, "lane_red").expect("a track"),
        &[[0.0, 0.0], [1.0, 2.0], [3.0, 2.0]],
    );
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    near_points(
        &points(&h, "lane_red").expect("a track"),
        &[[0.0, 0.0], [3.0, 2.0]],
    );
    h.state_mut().undo();
    settle(&mut h);
    // Picked again, then an agent draws the lane anew: the second point is another one now, so
    // Delete takes nothing away.
    h.get_by_label_contains("Point 2 of Lane red").click();
    settle(&mut h);
    let anew = [[0.0, 0.0], [3.0, 0.0], [3.0, 2.0]];
    set_track(
        &mut h,
        "lane_red",
        [0.0, 0.0, 0.0],
        serde_json::json!([
            { "type": "line", "length": 3.0 },
            { "type": "arc", "radius": 0.0, "angle": 90.0 },
            { "type": "line", "length": 2.0 }
        ]),
    );
    near_points(&points(&h, "lane_red").expect("a track"), &anew);
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    near_points(&points(&h, "lane_red").expect("a track"), &anew);
}

#[test]
fn an_arrow_key_in_the_frame_a_point_is_let_go_moves_it_on_from_there() {
    let mut h = sorter();
    on_layout(&mut h, "lane_red");
    let before = points(&h, "lane_red").expect("a track");
    let end = point_of(&h, "Point 2 of Lane red");
    let by = egui::vec2(60.0, 0.0);
    // Where a drag alone leaves the end.
    drag_by(&mut h, end, by, 1.0);
    let let_go = points(&h, "lane_red").expect("a track");
    assert!(let_go[1][0] > before[1][0] + 0.1, "{let_go:?}");
    h.state_mut().undo();
    settle(&mut h);
    // The end picked, so it has the keyboard, then dragged the same way and let go in the frame
    // that Shift and the right arrow are pressed.
    h.get_by_label_contains("Point 2 of Lane red").click();
    settle(&mut h);
    h.hover_at(end);
    settle(&mut h);
    h.drag_at(end);
    h.run_steps(1);
    for f in [0.25, 0.5, 0.75, 1.0] {
        h.hover_at(end + by * f);
        h.run_steps(1);
    }
    let key = |pressed: bool| egui::Event::Key {
        key: egui::Key::ArrowRight,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::SHIFT,
    };
    h.input_mut().events.extend([
        egui::Event::PointerButton {
            pos: end + by,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
        key(true),
        key(false),
    ]);
    h.run_steps(1);
    settle(&mut h);
    let moved = points(&h, "lane_red").expect("a track");
    assert!((moved[1][0] - let_go[1][0] - 1.0).abs() < 1e-6, "{moved:?}");
    assert!((moved[1][1] - let_go[1][1]).abs() < 1e-6, "{moved:?}");
    // One edit: one undo puts the end back.
    h.state_mut().undo();
    settle(&mut h);
    assert_eq!(points(&h, "lane_red").expect("a track"), before);
}

#[test]
fn a_picked_point_moved_onto_its_neighbour_is_let_go() {
    let mut h = sorter();
    on_layout(&mut h, "lane_red");
    // A straight 3 m with points put on it at 1 m and 2 m.
    set_track(
        &mut h,
        "lane_red",
        [0.0, 0.0, 0.0],
        serde_json::json!([
            { "type": "line", "length": 1.0 },
            { "type": "arc", "radius": 1.0, "angle": 0.0 },
            { "type": "line", "length": 1.0 },
            { "type": "arc", "radius": 1.0, "angle": 0.0 },
            { "type": "line", "length": 1.0 }
        ]),
    );
    h.get_by_role_and_label(Role::Button, "Fit").click();
    settle(&mut h);
    near_points(
        &points(&h, "lane_red").expect("a track"),
        &[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]],
    );
    // The point at 1 m, moved 1 m back onto the start, is one with it: the saved path has the
    // point at 2 m second now, and Delete does not take that away.
    h.get_by_label_contains("Point 2 of Lane red").click();
    settle(&mut h);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::ArrowLeft);
    settle(&mut h);
    let merged = [[0.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
    near_points(&points(&h, "lane_red").expect("a track"), &merged);
    h.key_press(egui::Key::Delete);
    settle(&mut h);
    near_points(&points(&h, "lane_red").expect("a track"), &merged);
}
