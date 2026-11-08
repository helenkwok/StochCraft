//! Watching a run (M3.5) in the window: replication 1 played, paused, stepped and jumped to a
//! time, the selected block's queue over time, the end, and the model changing underneath.

#![allow(clippy::panic, clippy::expect_used)] // A widget or value missing fails the test.

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{By, NodeT, Queryable};
use serde_json::json;
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, EXAMPLES, Platform};

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// The M/M/1 example, open, with the run being watched (and paused, so frames don't move it).
fn watching() -> Harness<'static, App> {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Watch a run").click();
    h.run_steps(3);
    assert_eq!(h.state().status(), "Watching replication 1");
    h
}

/// The clock's text: "Replication 1 · 12.3 of 540 min".
fn clock(h: &Harness<'static, App>) -> String {
    h.query_all(By::new())
        .filter_map(|n| n.accesskit_node().value().or(n.accesskit_node().label()))
        .find(|l| l.starts_with("Replication 1 · "))
        .unwrap_or_default()
}

/// The time on the clock.
fn time(h: &Harness<'static, App>) -> f64 {
    let c = clock(h);
    c.trim_start_matches("Replication 1 · ")
        .split(' ')
        .next()
        .and_then(|t| t.replace(',', "").parse().ok())
        .unwrap_or_else(|| panic!("no time in {c:?}"))
}

#[test]
fn a_run_plays_pauses_steps_and_jumps() {
    let mut h = watching();
    // Playing: frames move simulated time on.
    let t0 = time(&h);
    h.run_steps(20);
    let t1 = time(&h);
    assert!(t1 > t0, "{t0} → {t1}");
    h.get_by_role_and_label(Role::Button, "Pause").click();
    settle(&mut h);
    let paused = time(&h);
    h.run_steps(10);
    assert_eq!(time(&h), paused);
    // Step: the next event, and no further.
    h.get_by_role_and_label(Role::Button, "Step").click();
    settle(&mut h);
    assert!(time(&h) > paused);
    // Jump to a time.
    h.get_by_role_and_label(Role::TextInput, "Jump to time")
        .click();
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "Jump to time")
        .type_text(&format!("{}", (time(&h) + 100.0).ceil()));
    let target = (time(&h) + 100.0).ceil();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Jump").click();
    settle(&mut h);
    assert_eq!(time(&h), target, "{}", clock(&h));
    // Past the end: the run finishes there.
    h.state_mut()
        .run_command(&CommandRef::new(commands::STOP_WATCHING));
    settle(&mut h);
    assert!(h.query_by_role_and_label(Role::Button, "Step").is_none());
    h.get_by_role_and_label(Role::Button, "Watch a run");
}

#[test]
fn the_end_is_reported_and_the_selected_block_charted() {
    let mut h = watching();
    h.get_by_role_and_label(Role::Button, "Pause").click();
    settle(&mut h);
    h.get_by_label("Select a block to see its queue over time.");
    h.get_all_by_role(Role::ListBoxOption)
        .nth(1)
        .expect("the server in the block list")
        .click();
    settle(&mut h);
    h.get_by_label_contains("Waiting over time: now");
    h.get_by_role_and_label(Role::TextInput, "Jump to time")
        .click();
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "Jump to time")
        .type_text("1000000");
    settle(&mut h);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    assert!(
        h.state().status().starts_with("Finished at "),
        "{}",
        h.state().status()
    );
    assert!(
        h.query_by_role_and_label(Role::Button, "Play")
            .is_some_and(|n| n.accesskit_node().is_disabled())
    );
}

#[test]
fn changing_the_model_stops_watching_but_moving_shapes_does_not() {
    let mut h = watching();
    h.state_mut().execute(
        "layout.set",
        json!({ "shapes": [{ "id": "serve", "x": 400, "y": 300 }] }),
    );
    settle(&mut h);
    assert!(
        h.query_by_role_and_label(Role::Button, "Stop watching")
            .is_some()
    );
    h.state_mut()
        .execute("block.set", json!({ "id": "serve", "capacity": 2 }));
    settle(&mut h);
    assert_eq!(h.state().status(), "The model changed, so watching stopped");
    assert!(
        h.query_by_role_and_label(Role::Button, "Stop watching")
            .is_none()
    );
}

/// One entity, arriving at 1, is worked in no time and always sent back by the gateway after
/// it: it goes round for ever while simulated time stays at 1 (#49). Watching it, a jump past 1
/// runs those events a frame's budget at a time, so no frame freezes the window (#76): before,
/// the first frame ran all ten million events up to the stuck-time limit at once.
#[test]
fn a_loop_at_one_time_never_freezes_a_frame() {
    let mut h = watching();
    h.state_mut().execute(
        "model.set",
        json!({"model": {
            "name": "Round and round",
            "elements": [
                {"id": "arrivals", "kind": "source", "interarrival": {"type": "constant", "value": 1.0}, "limit": 1},
                {"id": "work", "kind": "server", "capacity": 1, "service": {"type": "constant", "value": 0.0}},
                {"id": "check", "kind": "exclusive_gateway"},
                {"id": "done", "kind": "sink"}
            ],
            "connections": [
                {"id": "f1", "from": "arrivals", "to": "work"},
                {"id": "f2", "from": "work", "to": "check"},
                {"id": "f3", "from": "check", "to": "work", "probability": 1.0},
                {"id": "f4", "from": "check", "to": "done", "probability": 0.0}
            ],
            "experiment": {"time_unit": "min", "warmup": 0.0, "duration": 100.0, "replications": 1, "seed": 1}
        }}),
    );
    settle(&mut h);
    // Every frame from here on is timed.
    let mut slowest = std::time::Duration::ZERO;
    let mut frames = |h: &mut Harness<'static, App>, n: usize| {
        for _ in 0..n {
            let start = std::time::Instant::now();
            h.step();
            slowest = slowest.max(start.elapsed());
        }
    };
    h.get_by_role_and_label(Role::Button, "Watch a run").click();
    frames(&mut h, 2);
    h.get_by_role_and_label(Role::Button, "Pause").click();
    frames(&mut h, 2);
    h.get_by_role_and_label(Role::TextInput, "Jump to time")
        .click();
    frames(&mut h, 1);
    h.get_by_role_and_label(Role::TextInput, "Jump to time")
        .type_text("50");
    frames(&mut h, 1);
    h.key_press(egui::Key::Enter);
    frames(&mut h, 30);
    assert!(slowest.as_millis() < 500, "a frame took {slowest:?}");
    assert_eq!(time(&h), 1.0, "{}", clock(&h));
    assert_eq!(h.state().status(), "Watching replication 1");
}
