//! Gateways against exact results: a Jackson network and a superposition of Poisson streams
//! against queueing theory (independent replications, a 95% confidence interval that must cover
//! the theoretical value), and deterministic cases that must come out exactly.

use stochcraft_blocks::{Builder, ExperimentSummary, Metric, Report, compile, summarise};
use stochcraft_model::{Dist, Experiment, PropertyType, PropertyValue};
use stochcraft_random::Rng;

const REPS: u32 = 16;

fn experiment(warmup: f64, duration: f64) -> Experiment {
    Experiment {
        warmup,
        duration,
        replications: REPS,
        ..Experiment::default()
    }
}

fn run(b: &Builder, x: Experiment) -> ExperimentSummary {
    let compiled = compile(&b.build(x.clone()));
    assert!(compiled.is_ok(), "{compiled:?}");
    let Ok(compiled) = compiled else {
        unreachable!()
    };
    let reports: Vec<_> = (0..u64::from(x.replications))
        .map(|r| compiled.run(Rng::new(20_000 + r)))
        .collect();
    summarise(&reports)
}

fn once(b: &Builder, duration: f64) -> Report {
    let r = b.run(Rng::new(7), 0.0, duration);
    assert!(r.is_ok(), "{r:?}");
    let Ok(r) = r else { unreachable!() };
    r
}

/// The theoretical value lies within two half-widths of the mean: a check that fails by chance
/// about once in a thousand (Student t, 15 degrees of freedom). A network's metrics share the
/// same arrivals, so a plain 95% interval on each of eight would fail together too often.
fn covers(s: &ExperimentSummary, element: &str, metric: Metric, theory: f64) {
    let got = s.get(element, metric);
    eprintln!("{element} {}: {got:?} (theory {theory})", metric.name());
    assert!(
        got.is_some_and(|g| g.n == u64::from(REPS)
            && (g.mean - theory).abs() <= 2.0 * g.half_width),
        "{element} {}: {got:?} vs {theory}",
        metric.name()
    );
}

fn entered(r: &Report, id: &str) -> u64 {
    r.element(id).map_or(0, |e| e.entered)
}

#[test]
fn jackson_network_matches_product_form() {
    // Arrivals at rate 1 to station A (μ = 4); after A, half go to B (μ = 2) and back to A, half
    // leave. Traffic equations: λA = 1 + λB, λB = λA / 2, so λA = 2 and λB = 1, ρA = ρB = 0.5.
    // Product form: each station is M/M/1, Wq = ρ / (μ − λ): 0.25 at A, 0.5 at B; the mean
    // time in the network is (LA + LB) / 1 = (1 + 1) / 1 = 2.
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::exponential_rate(1.0), None);
    let a = m.server("a", 1, Dist::exponential_rate(4.0));
    let after_a = m.exclusive("after_a");
    let b = m.server("b", 1, Dist::exponential_rate(2.0));
    let exit = m.sink("exit");
    m.connect(arrivals, a);
    m.connect(a, after_a);
    m.route(after_a, b, Some(0.5), None);
    m.route(after_a, exit, Some(0.5), None);
    m.connect(b, a);
    let duration = 20_000.0;
    let s = run(&m, experiment(1_000.0, duration));
    // Throughput: entries per reporting period.
    covers(&s, "a", Metric::Entered, 2.0 * duration);
    covers(&s, "b", Metric::Entered, duration);
    covers(&s, "exit", Metric::Entered, duration);
    covers(&s, "a", Metric::Utilisation, 0.5);
    covers(&s, "b", Metric::Utilisation, 0.5);
    covers(&s, "a", Metric::MeanWait, 0.25);
    covers(&s, "b", Metric::MeanWait, 0.5);
    covers(&s, "exit", Metric::MeanTimeInSystem, 2.0);
}

#[test]
fn merged_poisson_streams_are_one_poisson_stream() {
    // Two Poisson streams (0.3 and 0.45 a minute) merge into one of rate 0.75 at an M/M/1
    // server with μ = 1: W = 1 / (μ − λ) = 4, utilisation 0.75.
    let mut m = Builder::new();
    let s1 = m.source("walk_ins", Dist::exponential_rate(0.3), None);
    let s2 = m.source("calls", Dist::exponential_rate(0.45), None);
    let desk = m.server("desk", 1, Dist::exponential_rate(1.0));
    let exit = m.sink("exit");
    m.connect(s1, desk);
    m.connect(s2, desk);
    m.connect(desk, exit);
    let s = run(&m, experiment(1_000.0, 20_000.0));
    covers(&s, "exit", Metric::MeanTimeInSystem, 4.0);
    covers(&s, "desk", Metric::Utilisation, 0.75);
}

#[test]
fn a_condition_loop_repeats_exactly_as_often_as_it_says() {
    // Each of 10 entities arrives with 3 issues and loops through repair (1 minute, one issue
    // fewer) until none are left: 30 repairs, every entity 3 minutes in the system.
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::Constant(1.0), Some(10));
    m.property(
        arrivals,
        "issues",
        Some(PropertyType::Long),
        PropertyValue::Number(3.0),
    );
    let more = m.exclusive("more");
    let repair = m.delay("repair", Dist::Constant(1.0));
    m.property(
        repair,
        "issues",
        None,
        PropertyValue::Expression("getProperty('issues') - 1".into()),
    );
    let exit = m.sink("exit");
    m.connect(arrivals, more);
    m.route(more, repair, None, Some("getProperty('issues') > 0"));
    m.route(more, exit, None, None);
    m.default_route(more, exit);
    m.connect(repair, more);
    let r = once(&m, 100.0);
    assert_eq!(entered(&r, "repair"), 30);
    assert_eq!(entered(&r, "exit"), 10);
    assert_eq!(entered(&r, "more"), 40);
    assert_eq!(
        r.element("exit").and_then(|e| e.mean_time_in_system),
        Some(3.0)
    );
}

#[test]
fn with_no_condition_holding_and_no_default_an_entity_leaves_at_the_gateway() {
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::Constant(1.0), Some(10));
    let check = m.exclusive("check");
    let exit = m.sink("exit");
    m.connect(arrivals, check);
    m.route(check, exit, None, Some("false()"));
    let model = m.build(experiment(0.0, 100.0));
    assert!(
        model
            .validate()
            .iter()
            .any(|d| d.problem.code() == "no_default"),
        "a warning says so"
    );
    let r = once(&m, 100.0);
    assert_eq!(entered(&r, "check"), 10);
    assert_eq!(r.element("check").map(|e| e.left), Some(10));
    assert_eq!(entered(&r, "exit"), 0);
}

#[test]
fn a_fork_and_join_waits_for_the_slower_branch() {
    // Five entities, 10 minutes apart: each is copied down a 2-minute and a 5-minute branch,
    // and the join sends one on when both copies are in, so each spends exactly 5 minutes.
    // The branches set a property each; the joined entity holds both.
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::Constant(10.0), Some(5));
    let split = m.parallel("split");
    let short = m.delay("short", Dist::Constant(2.0));
    m.property(short, "a", None, PropertyValue::Number(1.0));
    let long = m.delay("long", Dist::Constant(5.0));
    m.property(long, "b", None, PropertyValue::Number(2.0));
    let join = m.parallel("join");
    let done = m.delay("done", Dist::Constant(0.0));
    m.property(
        done,
        "sum",
        None,
        PropertyValue::Expression("getProperty('a') + getProperty('b')".into()),
    );
    let exit = m.sink("exit");
    m.connect(arrivals, split);
    m.connect(split, short);
    m.connect(split, long);
    m.connect(short, join);
    m.connect(long, join);
    m.connect(join, done);
    m.connect(done, exit);
    let r = once(&m, 100.0);
    assert_eq!(entered(&r, "short"), 5);
    assert_eq!(entered(&r, "long"), 5);
    assert_eq!(entered(&r, "join"), 10);
    assert_eq!(entered(&r, "exit"), 5);
    assert_eq!(
        r.element("exit").and_then(|e| e.mean_time_in_system),
        Some(5.0)
    );
    let sum = r
        .element("done")
        .and_then(|e| e.properties.first())
        .and_then(|(_, v)| *v);
    assert_eq!(sum, Some(3.0));
}

#[test]
fn connections_without_a_probability_share_what_is_left() {
    // One connection takes 0.5; the other two share the rest: 0.25 each.
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::exponential_rate(1.0), None);
    let pick = m.exclusive("pick");
    let (x, y, z) = (m.sink("x"), m.sink("y"), m.sink("z"));
    m.connect(arrivals, pick);
    m.route(pick, x, Some(0.5), None);
    m.route(pick, y, None, None);
    m.route(pick, z, None, None);
    let s = run(&m, experiment(0.0, 10_000.0));
    covers(&s, "pick", Metric::Entered, 10_000.0);
    // Each sink's count is binomial given the total; the means cover the shares.
    for (id, share) in [("x", 0.5), ("y", 0.25), ("z", 0.25)] {
        let got = s.get(id, Metric::Entered).map(|g| g.mean / 10_000.0);
        assert!(
            got.is_some_and(|g| (g - share).abs() < 0.01),
            "{id}: {got:?} vs {share}"
        );
    }
}

#[test]
fn the_first_arrival_can_be_at_time_zero() {
    // Decision D8: by default the first arrival comes after the first interval.
    let mut m = Builder::new();
    let arrivals = m.source("arrivals", Dist::Constant(24.0), None);
    let exit = m.sink("exit");
    m.connect(arrivals, exit);
    let model = m.build(experiment(0.0, 3_600.0));
    let Ok(c) = compile(&model) else {
        unreachable!()
    };
    let after = c.clone().run(Rng::new(1));
    let at_zero = c.first_arrival_at_zero(true).run(Rng::new(1));
    assert_eq!(entered(&after, "exit"), 150);
    assert_eq!(entered(&at_zero, "exit"), 151);
}
