//! Replications that would never end: entities going round a loop of steps that take no time
//! stop with an error naming an element of the loop, and a replication that would take very long
//! can be cancelled while it runs (#49).
//!
//! Each case runs on a thread of its own and fails after a bound, so that if the guard ever
//! breaks the test fails instead of hanging the suite.

#![allow(clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use stochcraft_blocks::{Builder, RunError, compile};
use stochcraft_model::Experiment;
use stochcraft_random::{Dist, Rng};

/// Runs `f` on a thread of its own and returns what it gives, or fails after `secs` seconds.
fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(v) => v,
        Err(_) => panic!("still running after {secs} s"),
    }
}

/// One entity, arriving at 1, is worked and inspected in no time, and the inspection always
/// sends it back: it goes round for ever while simulated time stays at 1.
fn zero_time_loop() -> Builder {
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::Constant(1.0), Some(1));
    let work = m.server("work", 1, Dist::Constant(0.0));
    let inspect = m.server("inspect", 1, Dist::Constant(0.0));
    let check = m.exclusive("check");
    let done = m.sink("done");
    m.connect(arrivals, work);
    m.connect(work, inspect);
    m.connect(inspect, check);
    m.route(check, work, Some(1.0), None);
    m.route(check, done, Some(0.0), None);
    m
}

/// The guard acts at the limit set (the default, `MAX_EVENTS_AT_ONE_TIME`, is checked through
/// the engine, in its tests).
#[test]
fn a_loop_of_steps_that_take_no_time_stops_with_an_error_naming_it() {
    let model = zero_time_loop().build(Experiment::default());
    let Ok(compiled) = compile(&model) else {
        panic!("{model:?}")
    };
    let r = within(60, move || {
        compiled.max_events_at_one_time(100_000).run(Rng::new(1))
    });
    let Err(
        e @ RunError::TimeStopped {
            time,
            events,
            element,
        },
    ) = &r
    else {
        panic!("{r:?}")
    };
    assert_eq!((*time, *events), (1.0, 100_000));
    let at = element.as_ref().map(|(id, _)| id.as_str());
    assert!(
        matches!(at, Some("work" | "inspect" | "check")),
        "the element named is in the loop: {at:?}"
    );
    let message = e.to_string();
    assert!(
        message.contains("stopped moving at 1") && message.contains(at.unwrap_or("?")),
        "{message}"
    );
}

/// A loop that is left with some probability ends almost surely, and runs as before.
#[test]
fn a_loop_with_a_way_out_still_runs() {
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::Constant(1.0), None);
    let work = m.server("work", 1, Dist::Constant(0.0));
    let check = m.exclusive("check");
    let done = m.sink("done");
    m.connect(arrivals, work);
    m.connect(work, check);
    m.route(check, work, Some(0.9), None);
    m.route(check, done, Some(0.1), None);
    let r = within(60, move || m.run(Rng::new(1), 0.0, 1_000.0));
    let Ok(r) = r else { panic!("{r:?}") };
    assert_eq!(r.element("done").map(|e| e.entered), Some(1_000));
}

/// A replication that would take far too long (a hundred billion arrivals) stops soon after it
/// is cancelled.
#[test]
fn cancelling_stops_a_long_replication_while_it_runs() {
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::Constant(1.0), None);
    let desk = m.server("desk", 1, Dist::Constant(0.5));
    let done = m.sink("done");
    m.connect(arrivals, desk);
    m.connect(desk, done);
    let model = m.build(Experiment {
        duration: 1e11,
        ..Experiment::default()
    });
    let Ok(compiled) = compile(&model) else {
        panic!("{model:?}")
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let c = Arc::clone(&cancel);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = compiled.run_watched(Rng::new(1), &|| c.load(Ordering::Relaxed));
        let _ = tx.send(r);
    });
    std::thread::sleep(Duration::from_millis(200));
    cancel.store(true, Ordering::Relaxed);
    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(r) => assert_eq!(r, Err(RunError::Cancelled)),
        Err(_) => panic!("the replication went on after it was cancelled"),
    }
}
