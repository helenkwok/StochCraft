//! Runs in slices (`"mode": "slices"`, how the browser runs when it has no workers): each
//! `run.status` runs a bounded number of events, a replication that does not finish carries on at
//! the next call, and the results are exactly those of a run on threads, whatever the slice size.

#![allow(clippy::expect_used, clippy::panic)] // A failed step fails the test.

use std::path::Path;

use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, Engine, JournalEntry};
use stochcraft_model::{Dist, ElementId, Experiment, Need, Stop};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    e.execute(command, params.clone())
        .unwrap_or_else(|err| panic!("{command} {params}: {err}"))
}

fn root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// The M/M/1-like help desk (with a warm-up) of the command examples.
fn help_desk() -> Engine {
    let mut e = Engine::new();
    ok(&mut e, "model.set", COMMANDS[0].example());
    e
}

fn opened(name: &str) -> Engine {
    let path = root().join("tests/models").join(name);
    let mut e = Engine::new();
    ok(
        &mut e,
        "file.open",
        json!({ "path": path.to_string_lossy() }),
    );
    e
}

/// The acceptance model: shared pools with shifts on a calendar.
fn plant() -> Engine {
    let path = root().join("tests/models/plant.json");
    let text = std::fs::read_to_string(&path).expect("plant.json");
    let script: Vec<JournalEntry> = serde_json::from_str(&text).expect("a script");
    let mut e = Engine::new();
    e.replay(&script).expect("replays");
    e
}

/// A warm-up, a shared pool and a delay (the model of `blocks/tests/live.rs`).
fn pooled() -> Engine {
    let mut b = stochcraft_blocks::Builder::new();
    b.pool("staff", 2, &[]);
    let src = b.source("arrivals", Dist::exponential_rate(1.5), None);
    let desk = b.server("desk", 2, Dist::exponential_rate(1.0));
    let walk = b.delay("walk", Dist::Uniform { min: 0.5, max: 1.5 });
    let help = b.shared_task(
        "help",
        Dist::exponential_rate(1.2),
        vec![Need::Resource {
            id: ElementId::unchecked("staff"),
            quantity: 1,
        }],
        0.0,
    );
    let out = b.sink("exit");
    b.connect(src, desk);
    b.connect(desk, walk);
    b.connect(walk, help);
    b.connect(help, out);
    Engine::with_model(b.build(Experiment {
        warmup: 20.0,
        duration: 200.0,
        ..Experiment::default()
    }))
}

/// mm1.bpmn ending each replication when 50 entities have left (a stop condition, which the
/// run checks after each instant).
fn stopping() -> Engine {
    let mut e = opened("mm1.bpmn");
    let exit = e
        .model()
        .elements
        .iter()
        .find(|x| x.kind.name() == "sink")
        .map(|x| x.id.to_string())
        .expect("a sink");
    ok(
        &mut e,
        "experiment.set",
        json!({ "stop": { "when": "count", "element": exit, "count": 50 } }),
    );
    e
}

/// An activity network ending at its critical path: a stop met by events at one instant.
fn project() -> Engine {
    let mut b = stochcraft_blocks::Builder::new();
    let start = b.source("start", Dist::Constant(0.0), Some(1));
    let fork = b.parallel("fork");
    let a = b.delay("A", Dist::Constant(3.0));
    let c = b.delay("C", Dist::Exponential { mean: 3.0 });
    let join = b.parallel("join");
    let done = b.sink("done");
    b.connect(start, fork);
    b.connect(fork, a);
    b.connect(fork, c);
    b.connect(a, join);
    b.connect(c, join);
    b.connect(join, done);
    Engine::with_model(b.build(Experiment {
        duration: 1_000.0,
        stop: Some(Stop::Count {
            element: ElementId::unchecked("done"),
            count: 1,
        }),
        ..Experiment::default()
    }))
}

/// Every model the slices are checked on, by name.
fn models() -> Vec<(&'static str, Engine)> {
    vec![
        ("mm1.bpmn", opened("mm1.bpmn")),
        ("tandem.xpdl", opened("tandem.xpdl")),
        ("distributions.bpmn", opened("distributions.bpmn")),
        ("plant.json", plant()),
        ("help desk", help_desk()),
        ("pooled", pooled()),
        ("stopping", stopping()),
        ("project", project()),
    ]
}

/// Runs `replications` in slices of at most `events` events each, until the run ends. Returns
/// the last status and the number of polls.
fn in_slices(e: &mut Engine, replications: u32, events: u64) -> (Value, u64) {
    let started = ok(
        e,
        "run.start",
        json!({"replications": replications, "mode": "slices"}),
    );
    assert_eq!(started["state"], "running", "{started}");
    let mut polls = 0u64;
    loop {
        let status = ok(e, "run.status", json!({ "step": 1, "events": events }));
        polls += 1;
        if status["state"] != "running" {
            return (status, polls);
        }
    }
}

/// Slices of one event, a few events and more than a replication has: every model, every size,
/// gives exactly the results of a run on threads. One-event slices put every warm-up end, stop
/// and run end on a slice boundary.
#[test]
fn sliced_runs_give_the_results_of_threads_whatever_the_slice() {
    for (name, mut e) in models() {
        let threads = ok(&mut e, "run", json!({"replications": 3, "threads": 2}));
        for events in [1, 7, 1_000, u64::MAX] {
            let (status, polls) = in_slices(&mut e, 3, events);
            assert_eq!(status["state"], "finished", "{name}, {events}: {status}");
            assert_eq!(status["result"], threads, "{name}, slices of {events}");
            if events == 1 {
                assert!(polls > 30, "{name}: {polls} polls of one event");
            }
        }
    }
}

/// #43: a long replication no longer runs in one poll, which froze the page until it ended.
#[test]
fn a_long_replication_runs_over_several_polls() {
    let mut e = help_desk();
    ok(&mut e, "experiment.set", json!({"duration": 200_000.0}));
    let threads = ok(&mut e, "run", json!({"replications": 2, "threads": 1}));
    ok(
        &mut e,
        "run.start",
        json!({"replications": 2, "mode": "slices"}),
    );
    // The default slice.
    let first = ok(&mut e, "run.status", json!({}));
    assert_eq!(first["state"], "running", "{first}");
    assert_eq!(first["done"], 0, "{first}");
    // Partial progress: how far the replication under way has got, in simulated time.
    let partial = first["partial"].as_f64().expect("partial progress");
    assert!(partial > 0.0 && partial < 1.0, "{first}");
    let second = ok(&mut e, "run.status", json!({}));
    assert!(second["partial"].as_f64() > Some(partial), "{second}");
    let mut status = second;
    let mut polls = 2;
    while status["state"] == "running" {
        status = ok(&mut e, "run.status", json!({}));
        polls += 1;
        assert!(polls < 100_000, "{status}");
    }
    assert!(polls > 4, "{polls} polls");
    assert_eq!(status["result"], threads);
}

/// A poll stops at whichever comes first: `step` replications finished, or its events spent.
#[test]
fn a_poll_finishes_at_most_step_replications() {
    let mut e = help_desk();
    let threads = ok(&mut e, "run", json!({"replications": 5, "threads": 1}));
    ok(
        &mut e,
        "run.start",
        json!({"replications": 5, "mode": "slices"}),
    );
    let s = ok(&mut e, "run.status", json!({"step": 2, "events": u64::MAX}));
    assert_eq!(
        (s["done"].as_u64(), s["partial"].as_f64()),
        (Some(2), Some(0.0))
    );
    let s = ok(&mut e, "run.status", json!({"step": 5, "events": u64::MAX}));
    assert_eq!(s["state"], "finished", "{s}");
    assert_eq!(s["result"], threads);
}

/// Cancel stops a replication between slices: at once, on the one thread there is.
#[test]
fn cancel_stops_a_sliced_replication_between_slices() {
    let mut e = help_desk();
    ok(&mut e, "experiment.set", json!({"duration": 1e11}));
    ok(
        &mut e,
        "run.start",
        json!({"replications": 2, "mode": "slices"}),
    );
    let s = ok(&mut e, "run.status", json!({}));
    assert_eq!(
        (s["state"].as_str(), s["done"].as_u64()),
        (Some("running"), Some(0))
    );
    let cancelled = ok(&mut e, "run.cancel", json!({}));
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    assert_eq!(cancelled["error"]["code"], "cancelled");
    // A new run starts cleanly after it.
    ok(&mut e, "experiment.set", json!({"duration": 100.0}));
    let threads = ok(&mut e, "run", json!({"replications": 2, "threads": 1}));
    let (status, _) = in_slices(&mut e, 2, 50);
    assert_eq!(status["result"], threads);
}

/// One entity, arriving at 1, is worked in no time and always sent back by the gateway after
/// it: it goes round for ever while simulated time stays at 1 (#49).
fn zero_time_loop() -> Engine {
    let mut m = stochcraft_blocks::Builder::new();
    let arrivals = m.source("arrivals", Dist::Constant(1.0), Some(1));
    let work = m.server("work", 1, Dist::Constant(0.0));
    let check = m.exclusive("check");
    let done = m.sink("done");
    m.connect(arrivals, work);
    m.connect(work, check);
    m.route(check, work, Some(1.0), None);
    m.route(check, done, Some(0.0), None);
    Engine::with_model(m.build(Experiment::default()))
}

/// The stuck-time limit counts events at one instant across slices, so it still fires, with
/// the error a run on one thread gives, word for word.
#[test]
fn the_stuck_time_limit_fires_across_slices() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut e = zero_time_loop();
        let threads = e
            .execute("run", json!({"replications": 3, "threads": 1}))
            .err()
            .map(|e| e.to_json());
        let (status, polls) = in_slices(&mut e, 3, 1_000_003);
        let _ = tx.send((threads, status, polls));
    });
    let (threads, status, polls) = rx
        .recv_timeout(std::time::Duration::from_secs(300))
        .expect("the run did not end");
    assert_eq!(status["state"], "failed", "{status}");
    assert_eq!(status["error"]["code"], "time_stopped", "{status}");
    assert_eq!(Some(&status["error"]), threads.as_ref());
    // Ten million events at time 1, a million at a time.
    assert!(polls >= 10, "{polls} polls");
}

/// Not a check: measures how many events a millisecond each model runs, for choosing
/// [`stochcraft_engine::SLICE_EVENTS`]. Run it on a release build:
/// `cargo test --release -p stochcraft-engine --test slices -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, for release builds"]
fn measure_events_per_millisecond() {
    let streams = stochcraft_experiment::Experiment::new(u64::MAX, 1);
    for (name, e) in models() {
        let compiled = stochcraft_blocks::compile(e.model()).expect("compiles");
        let (mut events, mut seconds, mut index) = (0u64, 0.0f64, 0u64);
        // Whole replications until at least a second of running and a million events.
        while seconds < 1.0 || events < 1_000_000 {
            let mut live = compiled.live(streams.stream(index));
            let start = std::time::Instant::now();
            while !live
                .advance_by(stochcraft_engine::SLICE_EVENTS)
                .expect("runs")
            {}
            seconds += start.elapsed().as_secs_f64();
            events += live.events();
            index += 1;
        }
        let per_ms = events as f64 / seconds / 1000.0;
        let slice_ms = stochcraft_engine::SLICE_EVENTS as f64 / per_ms;
        eprintln!(
            "{name:>20}: {per_ms:>6.0} events/ms over {index} replications; a slice takes \
             {slice_ms:.2} ms"
        );
    }
}
