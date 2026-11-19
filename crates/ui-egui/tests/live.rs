//! Watching a run (M3.5) in the window, from the run bar (M5.5 U1): replication 1 played, paused,
//! stepped, reset, sped up and slowed down, and paused at a time; the selected block's queue over
//! time, the end, and the model changing underneath.

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

/// Runs frames, one at a time, until `done` holds, for at most `frames` of them. A jump far
/// ahead runs only a frame's budget of events per frame (#80), so how many frames it needs
/// depends on the machine; a slow runner needs more than [`settle`] gives.
fn until(h: &mut Harness<'static, App>, frames: usize, done: impl Fn(&App) -> bool) {
    for _ in 0..frames {
        if done(h.state()) {
            return;
        }
        h.run_steps(1);
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
    assert_eq!(h.state().status(), "Watching run 1");
    h
}

/// The clock's text: "Run 1 · 12.3 of 540 min".
fn clock(h: &Harness<'static, App>) -> String {
    h.query_all(By::new())
        .filter_map(|n| n.accesskit_node().value().or(n.accesskit_node().label()))
        .find(|l| l.starts_with("Run 1 · "))
        .unwrap_or_default()
}

/// The time on the clock.
fn time(h: &Harness<'static, App>) -> f64 {
    let c = clock(h);
    c.trim_start_matches("Run 1 · ")
        .split(' ')
        .next()
        .and_then(|t| t.replace(',', "").parse().ok())
        .unwrap_or_else(|| panic!("no time in {c:?}"))
}

#[test]
fn a_run_plays_pauses_steps_resets_and_pauses_at_a_time() {
    let mut h = watching();
    // The bar says this is one replication of several, whose results differ.
    let reps = h.state().engine().model().experiment.replications;
    assert!(reps > 1, "{reps}");
    h.get_by_label(&format!(
        "One run of {reps}: the results of the others differ."
    ));
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
    // Reset: back to the start, paused.
    h.get_by_role_and_label(Role::Button, "Reset").click();
    settle(&mut h);
    assert_eq!(time(&h), 0.0, "{}", clock(&h));
    h.run_steps(10);
    assert_eq!(time(&h), 0.0, "reset leaves the run paused");
    h.get_by_role_and_label(Role::Button, "Play");
    // Pause at a time: the run plays on to it, and pauses there.
    h.get_by_label("As fast as possible").click();
    settle(&mut h);
    let target = 100.0;
    pause_at(&mut h, "100");
    settle(&mut h);
    assert_eq!(time(&h), target, "{}", clock(&h));
    h.run_steps(10);
    assert_eq!(time(&h), target, "paused there");
    h.get_by_role_and_label(Role::Button, "Play");
    // Text that is not a time is said so.
    pause_at(&mut h, "soon");
    settle(&mut h);
    h.get_by_label_contains("Type a time from 0 to");
    // Stop watching: the bar offers to watch again.
    h.state_mut()
        .run_command(&CommandRef::new(commands::STOP_WATCHING));
    settle(&mut h);
    assert!(h.query_by_role_and_label(Role::Button, "Step").is_none());
    h.get_by_role_and_label(Role::Button, "Watch a run");
}

/// Types `text` in the bar's "Pause at" field and presses its button.
fn pause_at(h: &mut Harness<'static, App>, text: &str) {
    h.get_by_role_and_label(Role::TextInput, "Pause at").click();
    settle(h);
    h.get_by_role_and_label(Role::TextInput, "Pause at")
        .type_text(text);
    settle(h);
    h.get_by_role_and_label(Role::Button, "Pause at").click();
    settle(h);
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
    // Flat out, so a time past the end is got to in a few frames.
    h.get_by_label("As fast as possible").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "Pause at").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "Pause at")
        .type_text("1000000");
    settle(&mut h);
    h.key_press(egui::Key::Enter);
    settle(&mut h);
    until(&mut h, 100_000, |app| {
        app.status().starts_with("Finished at ")
    });
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
/// it: it goes round for ever while simulated time stays at 1 (#49). Watching it, pausing at a time past 1
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
    h.get_by_role_and_label(Role::TextInput, "Pause at").click();
    frames(&mut h, 1);
    h.get_by_role_and_label(Role::TextInput, "Pause at")
        .type_text("50");
    frames(&mut h, 1);
    h.key_press(egui::Key::Enter);
    frames(&mut h, 30);
    // A frame that freezes on the loop takes seconds (7.9 s before the per-frame event budget);
    // shared CI runners alone have taken over 600 ms for an ordinary frame (#196).
    assert!(slowest.as_millis() < 2_000, "a frame took {slowest:?}");
    assert_eq!(time(&h), 1.0, "{}", clock(&h));
    assert_eq!(h.state().status(), "Watching run 1");
}

/// The M/M/1 example, open on the Model tab, nothing watched yet.
fn opened() -> Harness<'static, App> {
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
    h
}

/// The speed of the watched run, as the bar's ×N says it.
fn speed(h: &Harness<'static, App>) -> f64 {
    h.state().watch_speed().expect("a run is watched")
}

#[test]
fn the_run_bar_is_on_the_model_and_3d_views_and_not_on_the_start_page_or_other_views() {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    // The start page has no model, so no bar.
    assert!(
        h.query_by_role_and_label(Role::Button, "Watch a run")
            .is_none()
    );
    let e = &EXAMPLES[0];
    h.state_mut().open_file(e.file, e.bytes);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Watch a run");
    h.get_by_role_and_label(Role::Tab, "3D").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Watch a run").click();
    settle(&mut h);
    // On 3D too, the bar has all its controls, each with a name for a screen reader.
    for control in ["Reset", "Step", "Halve Speed", "Double Speed", "Pause at"] {
        h.get_by_role_and_label(Role::Button, control);
    }
    h.get_by_role_and_label(Role::TextInput, "Pause at");
    h.get_by_label("As fast as possible");
    // The same bar on the Model tab; not on a view that is not of the model.
    h.get_by_role_and_label(Role::Tab, "Model").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Reset");
    h.get_by_role_and_label(Role::Tab, "Analysis").click();
    settle(&mut h);
    assert!(h.query_by_role_and_label(Role::Button, "Reset").is_none());
}

#[test]
fn space_plays_and_pauses_and_brackets_halve_and_double_the_speed() {
    let mut h = opened();
    // Space starts watching a run, with nothing typed in and no dialog open.
    h.key_press(egui::Key::Space);
    settle(&mut h);
    assert_eq!(h.state().status(), "Watching run 1");
    h.get_by_role_and_label(Role::Button, "Pause");
    // Space again pauses: the clock stands still. And again plays.
    h.key_press(egui::Key::Space);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Play");
    let paused = time(&h);
    h.run_steps(10);
    assert_eq!(time(&h), paused);
    h.key_press(egui::Key::Space);
    h.run_steps(20);
    assert!(time(&h) > paused);
    // [ halves the speed and ] doubles it.
    let s = speed(&h);
    h.key_press(egui::Key::OpenBracket);
    settle(&mut h);
    assert_eq!(speed(&h), s / 2.0);
    h.key_press(egui::Key::CloseBracket);
    settle(&mut h);
    h.key_press(egui::Key::CloseBracket);
    settle(&mut h);
    assert_eq!(speed(&h), s * 2.0);
    // The buttons do the same, and the bar shows the speed as ×N.
    h.get_by_role_and_label(Role::Button, "Halve Speed").click();
    settle(&mut h);
    assert_eq!(speed(&h), s);
    h.get_by_label(&format!("×{s}"));
    h.get_by_role_and_label(Role::Button, "Double Speed")
        .click();
    settle(&mut h);
    assert_eq!(speed(&h), s * 2.0);
}

#[test]
fn space_does_not_play_while_a_field_is_being_typed_in() {
    let mut h = watching();
    h.get_by_role_and_label(Role::Button, "Pause").click();
    settle(&mut h);
    h.get_by_role_and_label(Role::TextInput, "Pause at").click();
    settle(&mut h);
    h.key_press(egui::Key::Space);
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Play");
}

#[test]
fn the_clock_says_warm_up_while_it_lasts() {
    let mut h = opened();
    h.state_mut().execute(
        "model.set",
        json!({"model": {
            "name": "Warm",
            "elements": [
                {"id": "arrivals", "kind": "source", "interarrival": {"type": "constant", "value": 1.0}},
                {"id": "work", "kind": "server", "capacity": 1, "service": {"type": "constant", "value": 0.5}},
                {"id": "done", "kind": "sink"}
            ],
            "connections": [
                {"id": "f1", "from": "arrivals", "to": "work"},
                {"id": "f2", "from": "work", "to": "done"}
            ],
            "experiment": {"time_unit": "min", "warmup": 40.0, "duration": 100.0, "replications": 1, "seed": 1}
        }}),
    );
    settle(&mut h);
    h.get_by_role_and_label(Role::Button, "Watch a run").click();
    settle(&mut h);
    // Pause at 20 first: as fast as possible would otherwise run on to the end.
    pause_at(&mut h, "20");
    h.get_by_label("As fast as possible").click();
    for _ in 0..10_000 {
        if time(&h) >= 20.0 {
            break;
        }
        h.run_steps(1);
    }
    settle(&mut h);
    assert_eq!(time(&h), 20.0, "{}", clock(&h));
    assert!(clock(&h).contains("warm-up"), "{}", clock(&h));
    // Played on past the warm-up, the clock no longer says so.
    h.get_by_role_and_label(Role::Button, "Play").click();
    until(&mut h, 10_000, |app| {
        app.status().starts_with("Finished at ")
    });
    settle(&mut h);
    assert!(time(&h) > 40.0, "{}", clock(&h));
    assert!(!clock(&h).contains("warm-up"), "{}", clock(&h));
}
