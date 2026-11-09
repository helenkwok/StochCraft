//! Compiled models against exact queueing results: independent replications, summarised with a
//! 95% confidence interval that must cover the theoretical value.

use stochcraft_blocks::{ExperimentSummary, Metric, compile, summarise};
use stochcraft_model::{Connection, Dist, Element, ElementId, ElementKind, Experiment, Model};
use stochcraft_random::Rng;

fn id(s: &str) -> ElementId {
    ElementId::new(s).unwrap_or_else(|| ElementId::unchecked(s))
}

/// An M/M/c model written as data, the way a model file or the engine will produce it.
fn mmc_model(lambda: f64, mu: f64, c: u32) -> Model {
    let element = |i: &str, kind| Element::new(id(i), i, kind);
    let flow = |i: &str, from: &str, to: &str| Connection::new(id(i), id(from), id(to));
    Model {
        name: format!("M/M/{c}"),
        elements: vec![
            element(
                "arrivals",
                ElementKind::Source {
                    interarrival: Dist::exponential_rate(lambda),
                    limit: None,
                    idle_outside: false,
                    timed: Vec::new(),
                },
            ),
            element(
                "servers",
                ElementKind::Server {
                    capacity: c,
                    service: Dist::exponential_rate(mu),
                    queue_capacity: None,
                    breakdown: None,
                    transfer: None,
                    available: true,
                    availability: Vec::new(),
                    power: None,
                },
            ),
            element("exit", ElementKind::Sink),
        ],
        connections: vec![
            flow("f1", "arrivals", "servers"),
            flow("f2", "servers", "exit"),
        ],
        resources: Vec::new(),
        experiment: Experiment {
            warmup: 1_000.0,
            duration: 20_000.0,
            replications: 16,
            ..Experiment::default()
        },
        ..Model::default()
    }
}

fn run(model: &Model) -> ExperimentSummary {
    let compiled = compile(model);
    assert!(compiled.is_ok(), "{compiled:?}");
    let Ok(compiled) = compiled else {
        unreachable!()
    };
    let reps = u64::from(model.experiment.replications);
    let reports: Vec<_> = (0..reps)
        .map(|r| {
            compiled
                .run(Rng::new(10_000 + r))
                .unwrap_or_else(|e| unreachable!("{e}"))
        })
        .collect();
    summarise(&reports)
}

fn covers(s: &ExperimentSummary, element: &str, metric: Metric, theory: f64) {
    let got = s.get(element, metric);
    eprintln!("{element} {}: {got:?} (theory {theory})", metric.name());
    assert!(
        got.is_some_and(|g| g.n == 16 && g.contains(theory)),
        "{element} {}: {got:?} vs {theory}",
        metric.name()
    );
}

#[test]
fn mm1_time_in_system_queue_and_utilisation() {
    // λ = 0.75, μ = 1: W = 1/(μ−λ) = 4, Lq = ρ²/(1−ρ) = 2.25, Wq = 3, utilisation ρ = 0.75.
    let s = run(&mmc_model(0.75, 1.0, 1));
    covers(&s, "exit", Metric::MeanTimeInSystem, 4.0);
    covers(&s, "servers", Metric::MeanQueueLength, 2.25);
    covers(&s, "servers", Metric::MeanWait, 3.0);
    covers(&s, "servers", Metric::Utilisation, 0.75);
}

#[test]
fn mm2_matches_erlang_c() {
    // λ = 1.5, μ = 1, c = 2: offered load a = 1.5, ρ = 0.75.
    // P0 = 1 / (1 + a + a²/(2!(1−ρ))) = 1/7; Erlang C = (a²/(2!(1−ρ))) P0 = 4.5/7;
    // Wq = C / (cμ − λ) = 9/7 ≈ 1.2857; W = Wq + 1/μ = 16/7 ≈ 2.2857.
    let s = run(&mmc_model(1.5, 1.0, 2));
    covers(&s, "exit", Metric::MeanTimeInSystem, 16.0 / 7.0);
    covers(&s, "servers", Metric::MeanWait, 9.0 / 7.0);
    covers(&s, "servers", Metric::Utilisation, 0.75);
}
