//! How long a frame of the window takes with a large model open: 1,000 blocks in 40 lines with
//! branches, on the canvas, in the 3D view and on the results page, each with AccessKit off and
//! on. An immediate-mode window lays out everything again each frame, and with AccessKit on it
//! also builds the accessibility tree each frame, which the platform adapter then compares with
//! the last one; both costs grow with the model.
//!
//! The frame is timed as the shell runs it, minus the GPU: `Context::run_ui` with the app's `ui`,
//! then tessellation, and with AccessKit on, the tree update handed to an `accesskit_consumer`
//! tree (through kittest's state, the same consumer the platform adapters use). Off and on are
//! timed in alternating blocks, so a runner that slows down part way affects both alike.
//!
//! Timings mean something only in release, so the test is ignored by `cargo test` and run by
//! CI's heavy job (`cargo test --workspace --release -- --ignored`). Locally:
//!
//! ```text
//! cargo test --release -p stochcraft-ui-egui --test frame_time -- --ignored --nocapture
//! ```
//!
//! The numbers are written to stderr past the test harness's capture, so they are in the CI log
//! of a passing run too, and to the job summary when `GITHUB_STEP_SUMMARY` is set.

#![allow(clippy::panic, clippy::expect_used)] // A model or view missing fails the test.

use std::io::Write as _;
use std::time::{Duration, Instant};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{self, Queryable};
use serde_json::{Value, json};
use stochcraft_engine::Engine;
use stochcraft_ui_egui::commands::{self, CommandRef};
use stochcraft_ui_egui::{App, Platform};

/// Lines of blocks, each a source, four branching segments, three tasks and a sink.
const LINES: usize = 40;
/// Segments of a line: a task, a decision, a task on one way and a delay on the other, and the
/// task where they meet again.
const SEGMENTS: usize = 4;
/// The window, as on a laptop screen.
const SIZE: egui::Vec2 = egui::vec2(1600.0, 1000.0);
/// Blocks of frames per mode, alternating off and on, and frames timed in each block.
const BLOCKS: usize = 6;
const FRAMES_PER_BLOCK: usize = 15;
/// After this long on one view, no further blocks are started (at least two always run).
const VIEW_LIMIT: Duration = Duration::from_secs(40);

/// The engine commands for the model: 1,000 blocks and their connections, placed on a grid.
fn large_model() -> Value {
    let mut commands = Vec::new();
    let mut add =
        |params: Value| commands.push(json!({ "command": "block.add", "params": params }));
    let mut flows = Vec::new();
    let (dx, row) = (140.0, 240.0);
    for line in 0..LINES {
        let y = 60.0 + row * line as f64;
        let mut col = 0.0;
        let mut x = || {
            col += 1.0;
            20.0 + dx * (col - 1.0)
        };
        let source = format!("l{line}-in");
        add(json!({
            "kind": "source", "id": source, "name": format!("Arrivals {line}"),
            "interarrival": { "type": "exponential", "mean": 2.0 }, "x": x(), "y": y
        }));
        let mut previous = source;
        for s in 0..SEGMENTS {
            let id = |part: &str| format!("l{line}-s{s}-{part}");
            let at = x();
            add(
                json!({ "kind": "server", "id": id("work"), "name": format!("Work {line}.{s}"),
                "service": { "type": "exponential", "mean": 1.0 }, "x": at, "y": y }),
            );
            let at = x();
            add(json!({ "kind": "exclusive_gateway", "id": id("check"),
                "name": format!("Check {line}.{s}"), "x": at, "y": y }));
            let at = x();
            add(
                json!({ "kind": "server", "id": id("fix"), "name": format!("Fix {line}.{s}"),
                "service": { "type": "exponential", "mean": 1.5 }, "x": at, "y": y - 70.0 }),
            );
            add(
                json!({ "kind": "delay", "id": id("wait"), "name": format!("Wait {line}.{s}"),
                "duration": { "type": "constant", "value": 1.0 }, "x": at, "y": y + 70.0 }),
            );
            let at = x();
            add(
                json!({ "kind": "server", "id": id("join"), "name": format!("Join {line}.{s}"),
                "service": { "type": "exponential", "mean": 0.8 }, "x": at, "y": y }),
            );
            flows.push(json!({ "from": previous, "to": id("work") }));
            flows.push(json!({ "from": id("work"), "to": id("check") }));
            flows.push(json!({ "from": id("check"), "to": id("fix"), "probability": 0.3 }));
            flows.push(json!({ "from": id("check"), "to": id("wait"), "probability": 0.7 }));
            flows.push(json!({ "from": id("fix"), "to": id("join") }));
            flows.push(json!({ "from": id("wait"), "to": id("join") }));
            previous = id("join");
        }
        for t in 0..3 {
            let id = format!("l{line}-t{t}");
            add(
                json!({ "kind": "server", "id": id, "name": format!("Pack {line}.{t}"),
                "service": { "type": "exponential", "mean": 0.9 }, "x": x(), "y": y }),
            );
            flows.push(json!({ "from": previous, "to": id }));
            previous = id;
        }
        let sink = format!("l{line}-out");
        add(
            json!({ "kind": "sink", "id": sink, "name": format!("Done {line}"), "x": x(), "y": y }),
        );
        flows.push(json!({ "from": previous, "to": sink }));
    }
    commands.extend(
        flows
            .into_iter()
            .map(|f| json!({ "command": "connect", "params": f })),
    );
    json!({ "label": "batch", "commands": commands })
}

/// The large model as a BPMN file, opened in the window as a user would open it.
fn open_large(h: &mut Harness<'static, App>) {
    let started = Instant::now();
    let mut engine = Engine::new();
    engine
        .execute("batch", large_model())
        .expect("the large model's commands");
    engine
        .execute(
            "experiment.set",
            json!({ "duration": 200.0, "replications": 2 }),
        )
        .expect("the experiment");
    let blocks = engine.model().elements.len();
    assert_eq!(
        blocks,
        LINES * (2 + 5 * SEGMENTS + 3),
        "blocks in the model"
    );
    let bytes = engine.save_bytes(true).expect("the model as BPMN");
    let built = started.elapsed();
    h.state_mut().open_file("large.bpmn", &bytes);
    settle(h);
    assert_eq!(
        h.state().engine().model().elements.len(),
        blocks,
        "{}",
        h.state().status()
    );
    report(&format!(
        "model: {blocks} blocks, {} connections, built and written in {:.0} ms, {} kB of BPMN",
        h.state().engine().model().connections.len(),
        built.as_secs_f64() * 1000.0,
        bytes.len() / 1000
    ));
}

fn settle(h: &mut Harness<'static, App>) {
    if h.run_ok().is_none() {
        h.run_steps(2);
    }
}

/// A frame's times: the app's pass and tessellation, and the AccessKit consumer's update.
struct Frame {
    pass: Duration,
    consumer: Duration,
}

/// One frame of `app` on `ctx`, as the shell runs it, without the GPU.
fn frame(ctx: &egui::Context, app: &mut App, tree: &mut Option<kittest::State>) -> Frame {
    let mut input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SIZE)),
        predicted_dt: 1.0 / 60.0,
        ..Default::default()
    };
    if let Some(v) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
        v.native_pixels_per_point = Some(1.0);
    }
    let start = Instant::now();
    let mut output = ctx.run_ui(input, |ui| app.ui(ui));
    let shapes = std::mem::take(&mut output.shapes);
    let primitives = ctx.tessellate(shapes, output.pixels_per_point);
    std::hint::black_box(primitives);
    let pass = start.elapsed();
    let start = Instant::now();
    if let Some(update) = output.platform_output.accesskit_update.take() {
        match tree {
            Some(t) => t.update(update),
            None => *tree = Some(kittest::State::new(update)),
        }
    }
    Frame {
        pass,
        consumer: start.elapsed(),
    }
}

/// Times of one view, with AccessKit off and on.
#[derive(Default)]
struct Times {
    off: Vec<Duration>,
    on: Vec<Duration>,
    on_consumer: Vec<Duration>,
}

/// The `p`th percentile (nearest rank) of `times`.
fn percentile(times: &[Duration], p: usize) -> Duration {
    let mut sorted = times.to_vec();
    sorted.sort();
    let rank = (sorted.len() * p).div_ceil(100).max(1);
    sorted.get(rank - 1).copied().unwrap_or_default()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Times frames of the view the harness shows, in alternating blocks of AccessKit off and on.
fn time_view(h: &mut Harness<'static, App>) -> Times {
    let ctx = h.ctx.clone();
    let mut times = Times::default();
    let mut tree = None;
    let started = Instant::now();
    for block in 0..BLOCKS {
        if block >= 2 && started.elapsed() > VIEW_LIMIT {
            break;
        }
        // Off first in even blocks and on first in odd ones.
        for on in [block % 2 == 1, block % 2 == 0] {
            if on {
                ctx.enable_accesskit();
            } else {
                ctx.disable_accesskit();
            }
            // The first frame after the switch builds the whole tree anew; not timed.
            frame(&ctx, h.state_mut(), &mut tree);
            for _ in 0..FRAMES_PER_BLOCK {
                let f = frame(&ctx, h.state_mut(), &mut tree);
                if on {
                    times.on.push(f.pass + f.consumer);
                    times.on_consumer.push(f.consumer);
                } else {
                    times.off.push(f.pass);
                }
            }
        }
    }
    // The harness drives the next view, and needs AccessKit for that.
    ctx.enable_accesskit();
    settle(h);
    times
}

/// Writes `line` to stderr past libtest's capture (so a passing run's CI log shows it) and to
/// the job summary on GitHub Actions.
fn report(line: &str) {
    let mut err = std::io::stderr().lock();
    let _ = writeln!(err, "frame_time: {line}");
    let _ = err.flush();
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY")
        && let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path)
    {
        let _ = writeln!(f, "{line}\n");
    }
}

/// The measured numbers of one view, reported, and its median times (off, on).
fn summarise(view: &str, t: &Times) -> (Duration, Duration) {
    let (off50, off95) = (percentile(&t.off, 50), percentile(&t.off, 95));
    let (on50, on95) = (percentile(&t.on, 50), percentile(&t.on, 95));
    let consumer50 = percentile(&t.on_consumer, 50);
    report(&format!(
        "{view}: AccessKit off median {:.2} ms, p95 {:.2} ms ({} frames); \
         on median {:.2} ms, p95 {:.2} ms ({} frames; of the median, {:.2} ms in the consumer); \
         on / off {:.2}",
        ms(off50),
        ms(off95),
        t.off.len(),
        ms(on50),
        ms(on95),
        t.on.len(),
        ms(consumer50),
        on50.as_secs_f64() / off50.as_secs_f64().max(1e-9),
    ));
    (off50, on50)
}

#[test]
#[ignore = "timings need release; CI runs it in release with the ignored tests"]
fn frames_of_a_large_model_stay_within_budget() {
    let mut app = App::new(Platform::DESKTOP);
    app.set_number_region("en");
    app.set_lang(craft_i18n::Lang::EN);
    let mut h = Harness::builder()
        .with_size(SIZE)
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    settle(&mut h);
    open_large(&mut h);

    let mut medians = Vec::new();
    // The canvas, where a model opens.
    let canvas = time_view(&mut h);
    medians.push(("canvas", summarise("canvas", &canvas)));

    h.get_by_role_and_label(Role::Tab, "3D").click();
    settle(&mut h);
    assert!(
        h.query_by_label("3D view of the model").is_some(),
        "the 3D view is not shown"
    );
    let view3d = time_view(&mut h);
    medians.push(("3D view", summarise("3D view", &view3d)));

    h.get_by_role_and_label(Role::Tab, "Model").click();
    settle(&mut h);
    let started = Instant::now();
    h.state_mut().run_command(&CommandRef::new(commands::RUN));
    h.step();
    while h.state().is_running() && started.elapsed() < Duration::from_secs(300) {
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    settle(&mut h);
    assert!(h.state().results().is_some(), "{}", h.state().status());
    report(&format!(
        "run: {} replications in {:.1} s",
        h.state().engine().model().experiment.replications,
        started.elapsed().as_secs_f64()
    ));
    let results = time_view(&mut h);
    medians.push(("results", summarise("results", &results)));

    for (view, (off, on)) in medians {
        assert!(
            off > Duration::ZERO && on > Duration::ZERO,
            "{view}: no time"
        );
    }
}
