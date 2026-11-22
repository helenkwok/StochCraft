//! Random streams (ROADMAP D18): the same seed gives the same trace, and an agent's numbers do
//! not depend on which other agents exist or the order they were made in.
#![allow(clippy::panic)] // A chart that fails to build or run fails the test.

use std::sync::Arc;

use stochcraft_agents::{
    Action, Agent, Chart, ChartSpec, Driver, Record, SendSpec, StateSpec, TransitionSpec,
};

fn chart() -> Arc<Chart> {
    let spec = ChartSpec::new("random")
        .data("x", "0")
        .data("tries", "0")
        .state(
            StateSpec::state("s")
                .on_entry(SendSpec::new("go").after_expr("exponential(1)"))
                .on_entry(Action::assign("x", "uniform(0, 10)"))
                .on_entry(Action::log_value("x", "$x"))
                .transition(
                    TransitionSpec::on("go")
                        .when("random() < 0.5")
                        .to("s")
                        .then(Action::assign("tries", "$tries + 1")),
                )
                .transition(TransitionSpec::on("go").to("s")),
        );
    match spec.build() {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("{e}"),
    }
}

/// Runs agents with these (seed, key)s to time 20 and returns each one's trace.
fn run(agents: &[(u64, &str)]) -> Vec<Vec<Record>> {
    let chart = chart();
    let mut d = Driver::new(0.0);
    for &(seed, key) in agents {
        if let Err(e) = d.add(Agent::new(Arc::clone(&chart), seed, key).with_trace()) {
            panic!("{e}");
        }
    }
    if let Err(e) = d.run_until(20.0) {
        panic!("{e}");
    }
    d.agents().iter().map(|a| a.trace().to_vec()).collect()
}

#[test]
fn the_same_seed_gives_the_same_trace() {
    let a = run(&[(7, "pop/0/agent/1")]);
    let b = run(&[(7, "pop/0/agent/1")]);
    assert_eq!(a, b);
    assert!(
        a[0].len() > 40,
        "the run does something: {} steps",
        a[0].len()
    );
    assert_ne!(
        a,
        run(&[(8, "pop/0/agent/1")]),
        "another seed, other numbers"
    );
    assert_ne!(
        a,
        run(&[(7, "pop/0/agent/2")]),
        "another agent, other numbers"
    );
}

#[test]
fn agents_streams_do_not_depend_on_each_other() {
    let together = run(&[(7, "pop/0/agent/1"), (7, "pop/0/agent/2")]);
    let reversed = run(&[(7, "pop/0/agent/2"), (7, "pop/0/agent/1")]);
    let crowded = run(&[
        (7, "pop/0/agent/3"),
        (7, "pop/0/agent/2"),
        (7, "pop/1/agent/1"),
        (7, "pop/0/agent/1"),
    ]);
    let alone = run(&[(7, "pop/0/agent/1")]);
    assert_eq!(together[0], alone[0]);
    assert_eq!(reversed[1], alone[0]);
    assert_eq!(crowded[3], alone[0]);
    assert_eq!(together[1], reversed[0]);
    assert_eq!(together[1], crowded[1]);
    assert_ne!(together[0], together[1]);
}
