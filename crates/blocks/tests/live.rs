//! A live run (one replication stepped for watching) is the same replication as a plain run:
//! advanced in uneven slices, or one event at a time, it ends with exactly the report
//! `Compiled::run` gives with the same random stream, and its state can be read on the way.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, Compiled, compile};
use stochcraft_model::{Dist, ElementId, Experiment, Model, Need};
use stochcraft_random::Rng;

fn model() -> Model {
    let mut b = Builder::new();
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
    b.build(Experiment {
        warmup: 20.0,
        duration: 200.0,
        replications: 1,
        ..Experiment::default()
    })
}

fn compiled() -> Compiled {
    let c = compile(&model());
    let Ok(c) = c else { panic!("{c:?}") };
    c
}

#[test]
fn a_live_run_in_slices_matches_a_plain_run() {
    let c = compiled();
    let plain = c.run(Rng::new(7));
    let mut live = c.live(Rng::new(7));
    assert_eq!(live.now(), 0.0);
    assert_eq!(live.end(), 220.0);
    let mut t = 0.0;
    let mut most_waiting = 0.0f64;
    while !live.is_done() {
        t += 3.7;
        let r = live.advance_to(t);
        assert!(r.is_ok(), "{r:?}");
        assert!(live.now() <= live.end());
        let states = live.states();
        // Four elements and one pool, each balanced: what entered left, was lost, or is inside.
        assert_eq!(states.len(), 6);
        for s in &states[..5] {
            assert!(s.busy >= 0.0 && s.waiting >= 0.0, "{s:?}");
        }
        most_waiting = most_waiting.max(states[1].waiting);
        assert!(states[5].busy <= 2.0, "{:?}", states[5]);
    }
    assert!(most_waiting > 0.0, "someone waited at the desk");
    assert_eq!(Ok(live.report()), plain);
}

#[test]
fn stepping_one_event_at_a_time_matches_too() {
    let c = compiled();
    let plain = c.run(Rng::new(11));
    let mut live = c.live(Rng::new(11));
    let mut steps = 0u64;
    let mut last = 0.0;
    loop {
        match live.step() {
            Ok(Some(t)) => {
                assert!(t >= last, "time never goes back");
                last = t;
                steps += 1;
            }
            Ok(None) => break,
            Err(e) => panic!("{e:?}"),
        }
    }
    assert!(steps > 100, "{steps}");
    assert!(live.is_done());
    assert_eq!(Ok(live.report()), plain);
    // Past the end, nothing more happens.
    assert_eq!(live.step(), Ok(None));
    assert_eq!(live.now(), live.end());
}

#[test]
fn the_report_so_far_covers_the_time_run() {
    let c = compiled();
    let mut live = c.live(Rng::new(3));
    assert!(live.advance_to(120.0).is_ok());
    let r = live.report();
    assert_eq!(r.warmup, 20.0);
    assert_eq!(r.duration, 100.0);
    let desk = r.element("desk").and_then(|e| e.utilisation);
    assert!(desk.is_some_and(|u| (0.0..=1.0).contains(&u)), "{desk:?}");
}

/// A replication run a bounded number of events at a time (how the browser runs without
/// workers, #43) ends with exactly the plain run's report, whatever the slice size, with or
/// without a stop condition. Slices of one event put the warm-up's end on a slice boundary.
#[test]
fn advancing_by_events_matches_a_plain_run() {
    let mut stopping = model();
    stopping.experiment.stop = Some(stochcraft_model::Stop::Count {
        element: ElementId::unchecked("exit"),
        count: 40,
    });
    let stopping = compile(&stopping).unwrap_or_else(|e| panic!("{e:?}"));
    for (c, stops) in [(compiled(), false), (stopping, true)] {
        let plain = c.run(Rng::new(5));
        assert_eq!(
            plain.as_ref().map(|r| r.stopped_at.is_some()),
            Ok(stops),
            "the stop is met"
        );
        for budget in [1, 2, 7, 500, u64::MAX] {
            let mut live = c.live(Rng::new(5));
            let mut slices = 0u64;
            loop {
                let before = live.events();
                let done = live.advance_by(budget).unwrap_or_else(|e| panic!("{e:?}"));
                assert!(live.events() - before <= budget, "at most {budget} events");
                slices += 1;
                if done {
                    break;
                }
            }
            assert!(live.is_done());
            assert_eq!(Ok(live.report()), plain, "slices of {budget}");
            if budget == 1 {
                assert!(slices > 100, "{slices}");
            }
        }
    }
}
