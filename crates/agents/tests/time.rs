//! Sends on the discrete-event clock: fixed timeouts, cancels, rates and rates redrawn.
//!
//! The rate checks compare the waiting times with the exponential distribution they should
//! follow: the mean within four standard errors of 1 / rate (a band a correct sampler leaves
//! about once in 16,000 seeds), and the Kolmogorov–Smirnov distance D below the asymptotic 1%
//! critical value, sqrt(−ln(0.005) / 2) / √n = 1.6276 / √n (from Kolmogorov's limiting
//! distribution, whose upper tail is 2 e^(−2x²) to within 2 × 10⁻⁹ here). The seeds are fixed, so
//! the checks are deterministic.
#![allow(clippy::panic)] // A chart that fails to build or run fails the test.

use std::sync::Arc;

use stochcraft_agents::{
    Action, Agent, Chart, ChartSpec, Driver, RunError, SendSpec, StateSpec, Step, TransitionSpec,
};
use stochcraft_random::math::exp;

fn build(spec: &ChartSpec) -> Arc<Chart> {
    match spec.build() {
        Ok(c) => Arc::new(c),
        Err(e) => panic!("{e}"),
    }
}

fn add(driver: &mut Driver, agent: Agent) -> u32 {
    match driver.add(agent) {
        Ok(i) => i,
        Err(e) => panic!("{e}"),
    }
}

fn run_until(driver: &mut Driver, end: f64) -> u64 {
    match driver.run_until(end) {
        Ok(n) => n,
        Err(e) => panic!("{e}"),
    }
}

fn step(driver: &mut Driver) -> Option<f64> {
    match driver.step() {
        Ok(t) => t,
        Err(e) => panic!("{e}"),
    }
}

fn agent(driver: &Driver, i: u32) -> &Agent {
    match driver.agent(i) {
        Some(a) => a,
        None => panic!("no agent {i}"),
    }
}

/// The times at which `event` was handled.
fn times_of(a: &Agent, event: &str) -> Vec<f64> {
    a.trace()
        .iter()
        .filter(|r| matches!(&r.step, Step::Event { name, .. } if name == event))
        .map(|r| r.time)
        .collect()
}

#[test]
fn a_fixed_timeout_of_2_fires_at_exactly_2() {
    let chart = build(
        &ChartSpec::new("timeout")
            .state(
                StateSpec::state("wait")
                    .on_entry(SendSpec::new("timeout").after(2.0))
                    .transition(TransitionSpec::on("timeout").to("done")),
            )
            .state(StateSpec::final_state("done")),
    );
    let mut d = Driver::new(0.0);
    let i = add(&mut d, Agent::new(chart, 1, "a").with_trace());
    assert_eq!(run_until(&mut d, 1.999), 0);
    assert!(agent(&d, i).is_in("wait"));
    assert_eq!(run_until(&mut d, 10.0), 1);
    let a = agent(&d, i);
    assert_eq!(times_of(a, "timeout"), [2.0]);
    assert!(!a.is_running());
    assert_eq!(
        a.trace().last().map(|r| (r.time, &r.step)),
        Some((2.0, &Step::Done))
    );
}

#[test]
fn a_cancelled_send_never_fires() {
    let chart = build(
        &ChartSpec::new("cancel")
            .state(
                StateSpec::state("wait")
                    .on_entry(SendSpec::new("timeout").id("t").after(2.0))
                    .on_entry(SendSpec::new("stop").after(1.0))
                    .transition(
                        TransitionSpec::on("stop")
                            .then(Action::cancel("t"))
                            .then(Action::log("stopped")),
                    )
                    .transition(TransitionSpec::on("timeout").to("late")),
            )
            .state(StateSpec::state("late")),
    );
    let mut d = Driver::new(0.0);
    let i = add(&mut d, Agent::new(chart, 1, "a").with_trace());
    // Only `stop` is handled: the cancelled timeout is skipped when it is popped at 2.
    assert_eq!(run_until(&mut d, 10.0), 1);
    let a = agent(&d, i);
    assert_eq!(times_of(a, "stop"), [1.0]);
    assert!(times_of(a, "timeout").is_empty());
    assert!(a.is_in("wait"));
    assert_eq!(a.pending_sends(), 0);
}

#[test]
fn a_send_without_delay_comes_after_the_events_already_due() {
    let chart = build(
        &ChartSpec::new("now").state(
            StateSpec::state("s")
                .on_entry(SendSpec::new("first"))
                .on_entry(SendSpec::new("second"))
                .transition(TransitionSpec::on("first").then(SendSpec::new("third"))),
        ),
    );
    let mut d = Driver::new(5.0);
    let i = add(&mut d, Agent::new(chart, 1, "a").with_trace());
    assert_eq!(run_until(&mut d, 5.0), 3);
    let order: Vec<String> = agent(&d, i)
        .trace()
        .iter()
        .filter_map(|r| match &r.step {
            Step::Event { name, .. } => Some(format!("{name}@{}", r.time)),
            _ => None,
        })
        .collect();
    assert_eq!(order, ["first@5", "second@5", "third@5"]);
}

/// The Kolmogorov–Smirnov distance between `xs` and the exponential with this rate.
fn ks_exponential(xs: &mut [f64], rate: f64) -> f64 {
    xs.sort_by(f64::total_cmp);
    let n = xs.len() as f64;
    xs.iter()
        .enumerate()
        .map(|(i, &x)| {
            let f = 1.0 - exp(-rate * x);
            let i = i as f64;
            ((i + 1.0) / n - f).max(f - i / n)
        })
        .fold(0.0, f64::max)
}

/// Checks that `xs` look exponential with this rate.
fn check_exponential(xs: &mut [f64], rate: f64, what: &str) {
    let n = xs.len() as f64;
    assert!(n >= 5000.0, "{what}: only {n} draws");
    let mean = xs.iter().sum::<f64>() / n;
    let se = (1.0 / rate) / n.sqrt();
    assert!(
        (mean - 1.0 / rate).abs() < 4.0 * se,
        "{what}: mean {mean}, expected {} ± {}",
        1.0 / rate,
        4.0 * se
    );
    let d = ks_exponential(xs, rate);
    let critical = 1.6276 / n.sqrt();
    assert!(
        d < critical,
        "{what}: KS D = {d}, 1% critical value {critical}"
    );
}

#[test]
fn a_rate_of_3_gives_exponential_times_with_mean_one_third() {
    let chart = build(
        &ChartSpec::new("rate").state(
            StateSpec::state("wait")
                .on_entry(SendSpec::new("tick").at_rate("3"))
                .transition(TransitionSpec::on("tick").to("wait")),
        ),
    );
    let mut d = Driver::new(0.0);
    add(&mut d, Agent::new(chart, 2026, "a"));
    let mut last = 0.0;
    let mut gaps = Vec::new();
    for _ in 0..20_000 {
        let Some(t) = step(&mut d) else {
            panic!("the ticks stopped");
        };
        gaps.push(t - last);
        last = t;
    }
    check_exponential(&mut gaps, 3.0, "rate 3");
}

/// A rate of 1 that becomes 6 at time 0.5: the time left after 0.5 is exponential with mean
/// 1/6. (Were the time not redrawn, it would be exponential with mean 1.)
#[test]
fn a_rate_is_redrawn_when_its_variable_changes() {
    let chart = build(
        &ChartSpec::new("redraw")
            .data("r", "1")
            .state(
                StateSpec::state("wait")
                    .on_entry(SendSpec::new("fire").at_rate("$r").id("f"))
                    .on_entry(SendSpec::new("change").after(0.5))
                    .transition(TransitionSpec::on("change").then(Action::assign("r", "6")))
                    .transition(TransitionSpec::on("fire").to("done")),
            )
            .state(StateSpec::final_state("done")),
    );
    let mut left = Vec::new();
    for seed in 0..20_000u64 {
        let mut d = Driver::new(0.0);
        let i = add(&mut d, Agent::new(Arc::clone(&chart), seed, "a"));
        let mut fired = None;
        while let Some(t) = step(&mut d) {
            fired = Some(t);
        }
        assert!(!agent(&d, i).is_running());
        if let Some(t) = fired
            && t > 0.5
        {
            left.push(t - 0.5);
        }
    }
    check_exponential(&mut left, 6.0, "rate 6 after the change");
}

#[test]
fn a_rate_of_0_waits_until_it_becomes_positive() {
    let chart = build(
        &ChartSpec::new("dormant")
            .data("r", "0")
            .state(
                StateSpec::state("wait")
                    .on_entry(SendSpec::new("fire").at_rate("$r"))
                    .on_entry(SendSpec::new("go").after(1.0))
                    .transition(TransitionSpec::on("go").then(Action::assign("r", "1000000000")))
                    .transition(TransitionSpec::on("fire").to("done")),
            )
            .state(StateSpec::state("done")),
    );
    let mut d = Driver::new(0.0);
    let i = add(&mut d, Agent::new(chart, 3, "a").with_trace());
    assert_eq!(run_until(&mut d, 0.999), 0);
    assert_eq!(agent(&d, i).pending_sends(), 2);
    run_until(&mut d, 2.0);
    let fire = times_of(agent(&d, i), "fire");
    assert_eq!(fire.len(), 1);
    assert!(fire[0] >= 1.0 && fire[0] < 1.0 + 1e-6, "{fire:?}");
}

/// A chart with a rate send on entry to `wait`, beside a state `busy` that `In()` can name.
fn rate_chart(rate: &str) -> ChartSpec {
    ChartSpec::new("rated")
        .data("k", "2")
        .state(
            StateSpec::state("wait")
                .on_entry(SendSpec::new("fire").at_rate(rate))
                .transition(TransitionSpec::on("fire").to("busy")),
        )
        .state(StateSpec::state("busy"))
}

fn refusal(rate: &str) -> String {
    match rate_chart(rate).build() {
        Ok(_) => panic!("the rate `{rate}` was accepted"),
        Err(e) => e.to_string(),
    }
}

/// A rate is drawn again only when a variable it reads changes, so a rate that read the
/// active states or the current event would keep a stale delay: a rate meant to start when
/// the agent enters a state would never fire, one meant to stop would still fire after the
/// agent has left, and a new event would not start the clock. Such a chart is refused.
#[test]
fn a_rate_that_reads_in_or_the_event_is_refused() {
    for rate in ["In('busy')", "In('busy') or $k > 3"] {
        let e = refusal(rate);
        assert!(e.contains("reads In()"), "{e}");
        assert!(e.contains("own variables change"), "{e}");
    }
    for rate in [
        "getProperty('_event.name') = 'go'",
        "getProperty('_event.data')",
        "$k * getProperty('_event.data')",
    ] {
        let e = refusal(rate);
        assert!(e.contains("reads the event"), "{e}");
        assert!(e.contains("own variables change"), "{e}");
    }
    let e = refusal("time()");
    assert!(e.contains("reads time()"), "{e}");
    let e = refusal("population-size('p')");
    assert!(e.contains("reads the population `p`"), "{e}");
}

#[test]
fn a_rate_that_reads_only_the_agents_variables_is_accepted() {
    for rate in ["3", "$k", "0.5 * $k + 0.01"] {
        if let Err(e) = rate_chart(rate).build() {
            panic!("the rate `{rate}` was refused: {e}");
        }
    }
}

/// A rate is turned into a delay as −ln(U) / rate: a rate of 1e-300 waits about 1e300, and a
/// rate of 2^-1060, whose reciprocal is not a finite number, gives a delay too long to be a
/// time, which is an error, not an event at once.
#[test]
fn a_tiny_rate_waits_long_and_one_too_tiny_is_an_error() {
    let chart = |rate: &str| {
        build(
            &ChartSpec::new("tiny")
                .state(
                    StateSpec::state("wait")
                        .on_entry(SendSpec::new("fire").at_rate(rate))
                        .transition(TransitionSpec::on("fire").to("fired"))
                        .transition(TransitionSpec::on("error.execution").to("failed")),
                )
                .state(StateSpec::state("fired"))
                .state(StateSpec::state("failed")),
        )
    };
    let mut d = Driver::new(0.0);
    let i = add(&mut d, Agent::new(chart("1 div pow(10, 300)"), 1, "a"));
    assert_eq!(run_until(&mut d, 1e6), 0);
    assert!(agent(&d, i).is_in("wait"));
    assert_eq!(agent(&d, i).pending_sends(), 1);
    let mut d = Driver::new(0.0);
    let i = add(
        &mut d,
        Agent::new(chart("1 div pow(2, 1000) div pow(2, 60)"), 1, "a"),
    );
    assert!(agent(&d, i).is_in("failed"));
    assert_eq!(step(&mut d), None);
}

#[test]
fn a_send_cancelled_before_it_reaches_the_clock_is_not_scheduled() {
    let chart = build(
        &ChartSpec::new("cancelled").state(
            StateSpec::state("s")
                .on_entry(SendSpec::new("never").id("t").after(f64::MAX))
                .on_entry(Action::cancel("t")),
        ),
    );
    let mut a = Agent::new(Arc::clone(&chart), 1, "a");
    if let Err(e) = a.start(0.0) {
        panic!("{e}");
    }
    assert!(a.take_sends().is_empty());
    // At 1e308 the cancelled send's time would not be finite; it is never put on the clock.
    let mut d = Driver::new(1e308);
    let i = add(&mut d, Agent::new(chart, 1, "a"));
    assert!(agent(&d, i).is_running());
    assert_eq!(step(&mut d), None);
}

#[test]
fn a_send_the_clock_cannot_hold_stops_its_agent() {
    let chart = build(
        &ChartSpec::new("overflow")
            .state(
                StateSpec::state("s")
                    .on_entry(SendSpec::new("first"))
                    .on_entry(SendSpec::new("far").after(1e308))
                    .on_entry(SendSpec::new("third").after(1.0))
                    .transition(TransitionSpec::on("first").to("t")),
            )
            .state(StateSpec::state("t")),
    );
    let mut d = Driver::new(1e308);
    match d.add(Agent::new(chart, 1, "a").with_trace()) {
        Err(RunError::Schedule { delay, .. }) => assert_eq!(delay, 1e308),
        other => panic!("{other:?}"),
    }
    let a = &d.agents()[0];
    assert!(!a.is_running());
    assert_eq!(a.pending_sends(), 0);
    assert!(matches!(
        a.trace().last().map(|r| &r.step),
        Some(Step::Error(_))
    ));
    // `first` was on the clock already: it is skipped.
    assert_eq!(step(&mut d), None);
    assert!(d.agents()[0].is_in("s"));
}
