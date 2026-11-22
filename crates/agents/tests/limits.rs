//! A chart that loops in zero time is stopped at the microstep limit, with its path.
#![allow(clippy::panic)] // A chart that fails to build fails the test.

use std::sync::Arc;

use stochcraft_agents::{
    Action, Agent, Chart, ChartSpec, DEFAULT_MICROSTEP_LIMIT, RunError, SendSpec, StateSpec, Step,
    TransitionSpec,
};

fn build(spec: &ChartSpec) -> Arc<Chart> {
    match spec.build() {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn an_eventless_loop_stops_at_the_limit_with_its_path() {
    let chart = build(
        &ChartSpec::new("loop")
            .state(StateSpec::state("a").transition(TransitionSpec::eventless().to("b")))
            .state(StateSpec::state("b").transition(TransitionSpec::eventless().to("a"))),
    );
    let mut agent = Agent::new(chart, 1, "looper").with_microstep_limit(100);
    match agent.start(0.0) {
        Err(RunError::MicrostepLimit {
            chart,
            agent: key,
            limit,
            path,
            ..
        }) => {
            assert_eq!(
                (chart.as_str(), key.as_str(), limit),
                ("loop", "looper", 100)
            );
            assert!(path.contains(&"a -> b".to_string()), "{path:?}");
            assert!(path.contains(&"b -> a".to_string()), "{path:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(!agent.is_running());
    let message = match Agent::new(agent.chart().clone().into(), 1, "x").start(0.0) {
        Err(e) => e.to_string(),
        Ok(()) => String::new(),
    };
    assert!(
        message.contains("loops") && message.contains("a -> b"),
        "{message}"
    );
    assert!(
        message.contains(&DEFAULT_MICROSTEP_LIMIT.to_string()),
        "{message}"
    );
}

#[test]
fn a_loop_through_raised_events_stops_too() {
    let chart = build(
        &ChartSpec::new("echo").state(
            StateSpec::state("a")
                .on_entry(Action::raise("again"))
                .transition(TransitionSpec::on("again").to("a")),
        ),
    );
    let mut agent = Agent::new(chart, 1, "echo").with_microstep_limit(50);
    match agent.start(0.0) {
        Err(RunError::MicrostepLimit { path, .. }) => {
            assert!(path.iter().all(|p| p == "a -> a on again"), "{path:?}");
            assert_eq!(path.len(), 12, "the latest twelve");
        }
        other => panic!("{other:?}"),
    }
}

/// A stopped agent keeps no pending sends or internal events, and has not ended normally.
#[test]
fn a_loop_stopped_at_the_limit_leaves_nothing_pending() {
    let chart = build(
        &ChartSpec::new("loop")
            .state(
                StateSpec::state("a")
                    .on_entry(SendSpec::new("later").after(5.0))
                    .on_entry(Action::raise("soon"))
                    .transition(TransitionSpec::eventless().to("b")),
            )
            .state(StateSpec::state("b").transition(TransitionSpec::eventless().to("a"))),
    );
    let mut agent = Agent::new(chart, 1, "looper")
        .with_microstep_limit(10)
        .with_trace();
    assert!(agent.start(0.0).is_err());
    assert_eq!(agent.pending_sends(), 0);
    assert!(agent.take_sends().is_empty());
    assert!(matches!(
        agent.trace().last().map(|r| &r.step),
        Some(Step::Error(_))
    ));
    assert!(!agent.trace().iter().any(|r| r.step == Step::Done));
    assert_eq!(
        agent.fire(0, 1.0),
        Ok(false),
        "a send already on a clock is skipped"
    );
}

/// A chain of exactly `limit` eventless transitions is not a loop; one more is refused.
#[test]
fn the_limit_counts_microsteps_exactly() {
    let mut spec = ChartSpec::new("chain");
    for k in 0..5 {
        spec = spec.state(
            StateSpec::state(&format!("s{k}"))
                .transition(TransitionSpec::eventless().to(&format!("s{}", k + 1))),
        );
    }
    spec = spec.state(StateSpec::state("s5"));
    let chart = build(&spec);
    let mut enough = Agent::new(Arc::clone(&chart), 1, "a").with_microstep_limit(5);
    assert_eq!(enough.start(0.0), Ok(()));
    assert!(enough.is_in("s5"));
    let mut short = Agent::new(chart, 1, "a").with_microstep_limit(4);
    assert!(matches!(
        short.start(0.0),
        Err(RunError::MicrostepLimit { limit: 4, .. })
    ));
}
