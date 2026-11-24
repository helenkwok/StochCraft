//! Drawing a conveyor's track (ROADMAP.md, M5.5, L4): `track.set` sets or removes it in one undo
//! step, the conveyor's length follows the drawn path unless a typed length overrides it (D23),
//! and a saved file reads back the same track.

#![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use stochcraft_engine::{Engine, ErrorCode};
use stochcraft_model::{ElementId, ElementKind, Outline};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

fn err(e: &mut Engine, command: &str, params: Value) -> ErrorCode {
    let r = e.execute(command, params.clone());
    assert!(r.is_err(), "{command} {params} should fail: {r:?}");
    r.err().map_or(ErrorCode::Internal, |e| e.code)
}

fn sorter() -> Engine {
    let mut e = Engine::new();
    e.open_bytes(
        "sorter.bpmn",
        include_bytes!("../../../tests/models/sorter.bpmn"),
        None,
    )
    .unwrap();
    e
}

fn length_of(e: &Engine, id: &str) -> f64 {
    match e
        .model()
        .element(&ElementId::unchecked(id))
        .map(|x| &x.kind)
    {
        Some(ElementKind::Conveyor { length, .. }) => *length,
        _ => f64::NAN,
    }
}

/// An L: 3 m along x from (1, 1), a left bend of 1 m, 2 m up: 3 + 2 − 2 + π/2 = 3 + π/2.
fn l_track() -> Value {
    json!({
        "start": { "x": 1.0, "y": 1.0 },
        "segments": [
            { "type": "line", "length": 2.0 },
            { "type": "arc", "radius": 1.0, "angle": 90.0 },
            { "type": "line", "length": 1.0 }
        ],
        "width": 0.5
    })
}

#[test]
fn a_drawn_path_gives_the_length_and_undo_takes_it_back_exactly() {
    let mut e = sorter();
    let before = e.model().clone();
    assert_eq!(length_of(&e, "lane_red"), 4.0);
    let r = ok(
        &mut e,
        "track.set",
        json!({ "id": "lane_red", "track": l_track() }),
    );
    let drawn = 3.0 + std::f64::consts::FRAC_PI_2;
    let step = 1e-4;
    assert!((r["length"].as_f64().unwrap() - drawn).abs() <= step, "{r}");
    assert_eq!(r["override"], false);
    assert!((length_of(&e, "lane_red") - drawn).abs() <= step);
    let after = e.model().clone();
    let track = &after.site.paths[&ElementId::unchecked("lane_red")];
    assert_eq!(track.width, 0.5);
    assert_eq!(track.path.segments.len(), 3);
    // One step back, exactly; and forward again.
    ok(&mut e, "undo", json!({}));
    assert_eq!(e.model(), &before);
    ok(&mut e, "redo", json!({}));
    assert_eq!(e.model(), &after);
    let h = ok(&mut e, "history", json!({}));
    assert_eq!(h["undo"].as_array().unwrap().last().unwrap(), "track.set");
}

#[test]
fn a_typed_length_that_overrides_the_path_stays() {
    let mut e = sorter();
    ok(
        &mut e,
        "block.set",
        json!({ "id": "lane_red", "length": 5.0 }),
    );
    let r = ok(
        &mut e,
        "track.set",
        json!({ "id": "lane_red", "track": l_track() }),
    );
    assert_eq!(r["length"], 5.0);
    assert_eq!(r["override"], true);
    assert_eq!(length_of(&e, "lane_red"), 5.0);
}

#[test]
fn a_conveyor_drawn_for_the_first_time_takes_the_drawn_length() {
    let mut e = Engine::new();
    ok(
        &mut e,
        "block.add",
        json!({ "kind": "conveyor", "id": "belt", "length": 10.0, "speed": 1.0, "accumulating": true, "item_length": 0.4, "gap": 0.2 }),
    );
    let r = ok(
        &mut e,
        "track.set",
        json!({ "id": "belt", "track": { "start": { "x": 0.0, "y": 0.0 }, "segments": [{ "type": "line", "length": 7.5 }] } }),
    );
    assert_eq!(r["length"], 7.5);
    assert_eq!(length_of(&e, "belt"), 7.5);
    // Left out, the width is 0.6 m.
    let t = &e.model().site.paths[&ElementId::unchecked("belt")];
    assert_eq!(t.width, 0.6);
    // Removing the track leaves the length and unplaces the conveyor.
    ok(&mut e, "track.set", json!({ "id": "belt", "track": null }));
    assert!(e.model().site.paths.is_empty());
    assert_eq!(length_of(&e, "belt"), 7.5);
}

#[test]
fn a_track_a_file_could_not_hold_is_refused() {
    let mut e = sorter();
    let before = e.model().clone();
    let set = |track: Value| json!({ "id": "lane_red", "track": track });
    for bad in [
        json!({ "start": { "x": 0.0, "y": 0.0 }, "segments": [] }),
        json!({ "start": { "x": 0.0, "y": 0.0 }, "segments": [{ "type": "line", "length": -1.0 }] }),
        json!({ "start": { "x": 0.0, "y": 0.0 }, "segments": [{ "type": "arc", "radius": -1.0, "angle": 90.0 }] }),
        json!({ "start": { "x": 0.0, "y": 0.0 }, "segments": [{ "type": "line", "length": 0.0 }] }),
        json!({ "start": { "x": 0.0, "y": 0.0 }, "segments": [{ "type": "line", "length": 1.0 }], "width": 0.0 }),
    ] {
        assert_eq!(err(&mut e, "track.set", set(bad)), ErrorCode::InvalidParams);
    }
    // Only a conveyor has a track.
    assert_eq!(
        err(
            &mut e,
            "track.set",
            json!({ "id": "parcels", "track": l_track() })
        ),
        ErrorCode::InvalidParams
    );
    assert_eq!(
        err(
            &mut e,
            "track.set",
            json!({ "id": "nowhere", "track": l_track() })
        ),
        ErrorCode::NotFound
    );
    assert_eq!(e.model(), &before);
}

#[test]
fn two_track_edits_in_a_batch_are_one_undo_step() {
    let mut e = sorter();
    let before = e.model().clone();
    ok(
        &mut e,
        "batch",
        json!({ "label": "move", "commands": [
            { "command": "track.set", "params": { "id": "lane_red", "track": l_track() } },
            { "command": "track.set", "params": { "id": "lane_blue", "track": null } }
        ] }),
    );
    assert!(
        !e.model()
            .site
            .paths
            .contains_key(&ElementId::unchecked("lane_blue"))
    );
    ok(&mut e, "undo", json!({}));
    assert_eq!(e.model(), &before);
}

#[test]
fn a_path_edited_by_its_points_is_saved_and_read_back_the_same() {
    let mut e = sorter();
    // The loop's outline, one corner moved out by 2 m: the racetrack becomes lopsided.
    let id = ElementId::unchecked("loop");
    let outline: Outline = e.model().site.paths[&id].path.outline().unwrap();
    let moved = outline.moved(1, [outline.points[1][0] + 2.0, outline.points[1][1]]);
    let path = moved.to_path();
    let segments: Vec<Value> = path
        .segments
        .iter()
        .map(|s| match *s {
            stochcraft_model::Segment::Line { length, .. } => {
                json!({ "type": "line", "length": length })
            }
            stochcraft_model::Segment::Arc {
                radius, angle_deg, ..
            } => json!({ "type": "arc", "radius": radius, "angle": angle_deg }),
            stochcraft_model::Segment::Helix { .. } => json!(null),
        })
        .collect();
    ok(
        &mut e,
        "track.set",
        json!({ "id": "loop", "track": {
            "start": { "x": path.start.x, "y": path.start.y, "heading": path.start.heading_deg },
            "segments": segments
        } }),
    );
    let longer = length_of(&e, "loop");
    assert!(longer > 30.0, "{longer}");
    let bytes = e.save_bytes(false).unwrap();
    let mut back = Engine::new();
    back.open_bytes("sorter.bpmn", &bytes, None).unwrap();
    assert_eq!(
        back.model().site.paths[&id],
        e.model().site.paths[&id],
        "the track"
    );
    assert_eq!(length_of(&back, "loop"), longer);
}
