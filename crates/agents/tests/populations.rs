//! Populations, messages, re-tested conditions, removal, arrivals, the stepped mode and
//! random streams (ROADMAP M4.3, stage A2), each worked out by hand.
#![allow(clippy::panic)] // A model that fails to build or run fails the test.

use std::sync::Arc;

use stochcraft_agents::{
    Action, Chart, ChartSpec, Model, ModelError, Population, Run, SendSpec, Simulation, StateSpec,
    Statistic, Stepping, TransitionSpec,
};

fn build(spec: &ChartSpec) -> Arc<Chart> {
    match spec.build() {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("{e}"),
    }
}

fn simulate(model: &Model, seed: u64, end: f64) -> Simulation {
    let mut sim = match Simulation::new(model, seed) {
        Ok(s) => s,
        Err(e) => panic!("{e}"),
    };
    if let Err(e) = sim.run_until(end) {
        panic!("{e}");
    }
    sim
}

fn run(model: &Model, seed: u64, end: f64) -> Run {
    match model.run(seed, end) {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    }
}

/// Agent `serial`'s variable `var` as a number (NaN if there is no such agent).
fn value(sim: &Simulation, population: &str, serial: u64, var: &str) -> f64 {
    sim.agent(population, serial)
        .and_then(|a| a.value(var))
        .map_or(f64::NAN, |v| v.number())
}

/// Agent 0 waits until time 1, then broadcasts `hi` with data 7, sends `nobody` (which no
/// transition takes) to agent 3 and `hi` to agent 9 (which does not exist). Each of the others
/// numbers itself by how many had the message before it.
fn messenger() -> Arc<Chart> {
    build(
        &ChartSpec::new("messenger")
            .data("rank", "-1")
            .data("x", "0")
            .state(
                StateSpec::state("start")
                    .transition(
                        TransitionSpec::eventless()
                            .when("serial() = 0")
                            .to("sender"),
                    )
                    .transition(TransitionSpec::eventless().to("listen")),
            )
            .state(
                StateSpec::state("sender")
                    .on_entry(SendSpec::new("go").after(1.0))
                    .transition(
                        TransitionSpec::on("go")
                            .then(SendSpec::new("hi").to_all("p").with_data("3 + 4"))
                            .then(SendSpec::new("nobody").to_agent("p", "1 + 2"))
                            .then(SendSpec::new("hi").to_agent("p", "9")),
                    ),
            )
            .state(
                StateSpec::state("listen").transition(
                    TransitionSpec::on("hi")
                        .to("got")
                        .then(Action::assign("rank", "population-count('p', 'got')"))
                        .then(Action::assign("x", "getProperty('_event.data')")),
                ),
            )
            .state(StateSpec::state("got")),
    )
}

#[test]
fn broadcasts_arrive_in_serial_order_with_their_data() {
    let model = Model::new().population(Population::new("p", messenger()).count(4));
    let sim = simulate(&model, 1, 5.0);
    assert!(sim.agent("p", 0).is_some_and(|a| a.is_in("sender")));
    assert_eq!(
        value(&sim, "p", 0, "rank"),
        -1.0,
        "the sender is not sent to"
    );
    for s in 1..4 {
        assert_eq!(value(&sim, "p", s, "rank"), (s - 1) as f64, "agent {s}");
        assert_eq!(value(&sim, "p", s, "x"), 7.0, "agent {s}");
    }
    let c = &sim.counts()[0];
    assert_eq!((c.created, c.alive, c.removed), (4, 4, 0));
    assert_eq!(c.dropped, 1, "`nobody` was taken by no transition");
    assert_eq!(c.undelivered, 1, "there is no agent 9");
}

#[test]
fn a_random_pick_is_never_the_sender() {
    let chart = build(
        &ChartSpec::new("picker")
            .data("n", "0")
            .data("got", "0")
            .state(
                StateSpec::state("s")
                    .on_entry(SendSpec::new("go").after(1.0))
                    .transition(
                        TransitionSpec::on("go")
                            .when("serial() = 0 and $n < 400")
                            .then(Action::assign("n", "$n + 1"))
                            .then(SendSpec::new("pick").to_random("p"))
                            .then(SendSpec::new("go")),
                    )
                    .transition(TransitionSpec::on("pick").then(Action::assign("got", "$got + 1"))),
            ),
    );
    let model = Model::new().population(Population::new("p", Arc::clone(&chart)).count(3));
    let sim = simulate(&model, 3, 2.0);
    let got: Vec<f64> = (0..3).map(|s| value(&sim, "p", s, "got")).collect();
    assert_eq!(got[0], 0.0);
    assert_eq!(got[1] + got[2], 400.0);
    // Binomial(400, 1/2): standard deviation 10.
    assert!((got[1] - 200.0).abs() < 40.0, "{got:?}");
    // Alone, an agent has no one to pick.
    let alone = Model::new().population(Population::new("p", chart).count(1));
    let sim = simulate(&alone, 3, 2.0);
    assert_eq!(sim.counts()[0].undelivered, 400);
}

/// Three waiters go on once two of three finishers are done; the finishers finish at times 1,
/// 2 and 3. The waiters are tested again when the finishers change, so they go at exactly 2,
/// in serial order.
#[test]
fn conditions_on_populations_are_tested_again_when_they_change() {
    let finisher = build(
        &ChartSpec::new("finisher")
            .state(
                StateSpec::state("work")
                    .on_entry(SendSpec::new("fin").after_expr("serial() + 1"))
                    .transition(TransitionSpec::on("fin").to("done")),
            )
            .state(StateSpec::state("done")),
    );
    let waiter = build(
        &ChartSpec::new("waiter")
            .data("at", "-1")
            .data("rank", "-1")
            .state(
                StateSpec::state("wait").transition(
                    TransitionSpec::eventless()
                        .when("population-count('f', 'done') >= 2")
                        .to("go")
                        .then(Action::assign("rank", "population-count('w', 'go')")),
                ),
            )
            .state(StateSpec::state("go").on_entry(Action::assign("at", "time()"))),
    );
    assert_eq!(waiter.waits_on(), ["f"]);
    let model = Model::new()
        .population(Population::new("w", waiter).count(3))
        .population(Population::new("f", finisher).count(3));
    let sim = simulate(&model, 1, 10.0);
    for s in 0..3 {
        assert_eq!(value(&sim, "w", s, "at"), 2.0, "waiter {s}");
        assert_eq!(value(&sim, "w", s, "rank"), s as f64, "waiter {s}");
    }
}

#[test]
fn agents_leave_when_their_chart_ends_or_another_removes_them() {
    // Agent s ends at time s + 1; agent 0, at time 0.5, removes agent 3 (and agent 7, which
    // does not exist).
    let chart = build(
        &ChartSpec::new("mortal")
            .data("bye", "0")
            .state(
                StateSpec::state("alive")
                    .on_entry(SendSpec::new("end").after_expr("serial() + 1"))
                    .on_entry(SendSpec::new("cull").after(0.5))
                    .on_exit(Action::assign("bye", "1"))
                    .transition(TransitionSpec::on("end").to("gone"))
                    .transition(
                        TransitionSpec::on("cull")
                            .when("serial() = 0")
                            .then(Action::remove("p", "3"))
                            .then(Action::remove("p", "7")),
                    ),
            )
            .state(StateSpec::final_state("gone")),
    );
    let model = Model::new()
        .population(Population::new("p", chart).count(5))
        .statistic(Statistic::count("alive", "p"))
        .save_every(1.0);
    let sim = simulate(&model, 1, 2.5);
    assert_eq!(sim.alive("p"), [2, 4]);
    let r = run(&model, 1, 10.0);
    // At 0: 5; at 0.5 agent 3 goes; at 1, 2, 3 (agents 0, 1, 2) and 5 (agent 4).
    assert_eq!(r.series[0].values[..7], [5.0, 3.0, 2.0, 1.0, 1.0, 0.0, 0.0]);
    let c = &r.counts[0];
    assert_eq!((c.created, c.removed, c.alive), (5, 5, 0));
}

#[test]
fn agents_arrive_at_their_rate_and_by_other_agents() {
    let brief = build(&ChartSpec::new("brief").state(StateSpec::final_state("gone")));
    let parent = build(
        &ChartSpec::new("parent").state(
            StateSpec::state("s")
                .on_entry(SendSpec::new("go").after(1.0))
                .transition(
                    TransitionSpec::on("go")
                        .then(Action::create("kids"))
                        .then(Action::create("kids")),
                ),
        ),
    );
    let kid = build(&ChartSpec::new("kid").state(StateSpec::state("here")));
    let model = Model::new()
        .population(Population::new("visitors", brief).arrivals(2.0))
        .population(Population::new("parents", parent).count(3))
        .population(Population::new("kids", kid));
    let r = run(&model, 11, 5000.0);
    let v = &r.counts[0];
    // Poisson with mean 10,000: standard deviation 100.
    assert!((v.created as f64 - 10_000.0).abs() < 400.0, "{v:?}");
    assert_eq!((v.removed, v.alive), (v.created, 0));
    assert_eq!(r.counts[2].created, 6);
    assert_eq!(r.counts[2].alive, 6);
}

/// Five agents in a ring each take the value of the agent before them. Synchronously (the
/// before-step stores it, the step copies it), the values turn by one whatever the order;
/// asynchronously in serial order, agent 0 takes 4's value and every later one the value just
/// copied, so all become 4.
#[test]
fn the_stepped_mode_runs_its_phases_in_order() {
    let synchronous = build(
        &ChartSpec::new("sync")
            .data("v", "serial()")
            .data("next", "-1")
            .state(
                StateSpec::state("s")
                    .transition(TransitionSpec::on("before-step").then(Action::assign(
                        "next",
                        "agent-value('ring', (serial() + 4) mod 5, 'v')",
                    )))
                    .transition(TransitionSpec::on("step").then(Action::assign("v", "$next"))),
            ),
    );
    let asynchronous = build(
        &ChartSpec::new("async")
            .data("v", "serial()")
            .data("next", "-1")
            .state(
                StateSpec::state("s").transition(TransitionSpec::on("step").then(Action::assign(
                    "v",
                    "agent-value('ring', (serial() + 4) mod 5, 'v')",
                ))),
            ),
    );
    let env = build(
        &ChartSpec::new("env")
            .data("before", "0")
            .data("after", "0")
            .state(
                StateSpec::state("on")
                    .transition(
                        TransitionSpec::on("before-step")
                            .then(Action::assign("before", "population-sum('ring', 'next')")),
                    )
                    .transition(
                        TransitionSpec::on("after-step")
                            .then(Action::assign("after", "agent-value('ring', 0, 'v')")),
                    ),
            ),
    );
    let ring = |chart: &Arc<Chart>, stepping: Stepping| {
        Model::new()
            .population(Population::new("ring", Arc::clone(chart)).count(5))
            .environment(Arc::clone(&env))
            .stepped(stepping)
    };
    let values =
        |sim: &Simulation| -> Vec<f64> { (0..5).map(|s| value(sim, "ring", s, "v")).collect() };
    for stepping in [Stepping::every(1.0), Stepping::every(1.0).random_order()] {
        let sim = simulate(&ring(&synchronous, stepping), 5, 1.0);
        assert_eq!(values(&sim), [4.0, 0.0, 1.0, 2.0, 3.0], "{stepping:?}");
        let Some(e) = sim.environment() else {
            panic!("no environment");
        };
        // Before the first step, no agent has stored a next value yet; after it, agent 0 has 4.
        assert_eq!(e.value("before").map(|v| v.number()), Some(-5.0));
        assert_eq!(e.value("after").map(|v| v.number()), Some(4.0));
        let sim = simulate(&ring(&synchronous, stepping), 5, 3.0);
        assert_eq!(values(&sim), [2.0, 3.0, 4.0, 0.0, 1.0], "{stepping:?}");
    }
    let sim = simulate(&ring(&asynchronous, Stepping::every(1.0)), 5, 1.0);
    assert_eq!(values(&sim), [4.0; 5]);
}

#[test]
fn a_model_names_only_what_exists() {
    let error = |model: &Model| match Simulation::new(model, 1) {
        Err(ModelError::Invalid(e)) => e.to_string(),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("the model was accepted"),
    };
    let sender = build(
        &ChartSpec::new("sender")
            .state(StateSpec::state("s").on_entry(SendSpec::new("x").to_random("nowhere"))),
    );
    let m = Model::new().population(Population::new("p", sender));
    assert!(
        error(&m).contains("no population `nowhere`"),
        "{}",
        error(&m)
    );
    let counter = build(
        &ChartSpec::new("counter").state(
            StateSpec::state("s").transition(
                TransitionSpec::eventless()
                    .when("population-count('p', 'nope') > 0")
                    .to("s"),
            ),
        ),
    );
    let m = Model::new().population(Population::new("p", counter));
    assert!(error(&m).contains("no state `nope`"), "{}", error(&m));
    let plain = build(&ChartSpec::new("plain").state(StateSpec::state("s")));
    let m = Model::new()
        .population(Population::new("p", Arc::clone(&plain)))
        .population(Population::new("p", plain));
    assert!(error(&m).contains("another population"), "{}", error(&m));
    // Names are literals, and a rate reads only the agent's own variables.
    let computed = ChartSpec::new("c").state(
        StateSpec::state("s").transition(
            TransitionSpec::eventless()
                .when("population-size(concat('p', '')) > 0")
                .to("s"),
        ),
    );
    assert!(computed.build().is_err());
    let rate = ChartSpec::new("r")
        .state(StateSpec::state("s").on_entry(SendSpec::new("x").at_rate("population-size('p')")));
    assert!(rate.build().is_err());
    let delayed = ChartSpec::new("d")
        .state(StateSpec::state("s").on_entry(SendSpec::new("x").to_all("p").after(1.0)));
    assert!(delayed.build().is_err());
}

fn failures(n: u64) -> Model {
    let chart = build(
        &ChartSpec::new("machine")
            .data("failed_at", "-1")
            .state(
                StateSpec::state("working")
                    .on_entry(SendSpec::new("fail").at_rate("0.1"))
                    .transition(
                        TransitionSpec::on("fail")
                            .to("failed")
                            .then(Action::assign("failed_at", "time()")),
                    ),
            )
            .state(StateSpec::state("failed")),
    );
    Model::new()
        .population(Population::new("machines", chart).count(n))
        .statistic(Statistic::count("working", "machines").when("In('working')"))
        .statistic(Statistic::mean("mean failure", "machines", "$failed_at").when("In('failed')"))
        .save_every(5.0)
}

/// Each agent's streams are keyed by the seed, its population, its serial and their purpose
/// (D18): one more agent leaves every other agent's draws as they were.
#[test]
fn adding_an_agent_leaves_the_others_draws_unchanged() {
    let a = simulate(&failures(200), 9, 100.0);
    let b = simulate(&failures(201), 9, 100.0);
    let mut failed = 0;
    for s in 0..200 {
        let x = value(&a, "machines", s, "failed_at");
        assert_eq!(x, value(&b, "machines", s, "failed_at"), "agent {s}");
        failed += u32::from(x >= 0.0);
    }
    assert!(failed > 190, "{failed} failed by 100");
    assert!(value(&b, "machines", 200, "failed_at").is_finite());
}

#[test]
fn the_same_seed_gives_the_same_series() {
    let model = failures(50);
    // NaN is not equal to itself, so the series are compared bit for bit.
    let bits = |r: &Run| -> Vec<Vec<u64>> {
        r.series
            .iter()
            .map(|s| s.values.iter().map(|v| v.to_bits()).collect())
            .collect()
    };
    let one = run(&model, 4, 30.0);
    let again = run(&model, 4, 30.0);
    assert_eq!(bits(&one), bits(&again));
    assert_eq!((&one.times, &one.counts), (&again.times, &again.counts));
    assert_ne!(bits(&one), bits(&run(&model, 5, 30.0)));
    assert_eq!(one.times, [0.0, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0]);
    assert_eq!(one.series[0].values[0], 50.0);
    assert!(one.series[1].values[0].is_nan(), "the mean over no agents");
    assert!(one.series[0].values.windows(2).all(|w| w[1] <= w[0]));
}
