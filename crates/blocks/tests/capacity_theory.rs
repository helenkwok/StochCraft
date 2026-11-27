//! The capacity table's formulas against the simulator: for each family the table calls exact
//! (`Model::capacity`), the simulated 95% confidence interval must cover the formula's mean
//! wait, mean queue, utilisation and shares, each as the results define them.

use stochcraft_blocks::{ExperimentSummary, Metric, compile, summarise};
use stochcraft_model::capacity::{DEFAULT_TARGET, Formula};
use stochcraft_model::{
    BalkJoin, Balking, Connection, Dist, Element, ElementId, ElementKind, Experiment, Model,
    OnShiftEnd, Patience, PatienceUntil, QueueLimit, Setup,
};
use stochcraft_random::Rng;

const REPS: u32 = 16;

fn id(s: &str) -> ElementId {
    ElementId::new(s).unwrap_or_else(|| ElementId::unchecked(s))
}

fn el(i: &str, kind: ElementKind) -> Element {
    Element::new(id(i), i, kind)
}

fn link(from: &str, to: &str) -> Connection {
    Connection::new(id(&format!("{from}-{to}")), id(from), id(to))
}

fn source(rate: f64) -> Element {
    el(
        "in",
        ElementKind::Source {
            interarrival: Dist::exponential_rate(rate),
            limit: None,
            timed: Vec::new(),
            idle_outside: false,
        },
    )
}

fn server(i: &str, capacity: u32, service: Dist) -> Element {
    el(
        i,
        ElementKind::Server {
            capacity,
            service,
            processing_by: None,
            queue_capacity: None,
            queue_limit: QueueLimit::Fixed,
            breakdown: None,
            transfer: None,
            available: true,
            availability: Vec::new(),
            shifts: Vec::new(),
            on_shift_end: OnShiftEnd::Finish,
            power: None,
            setup: None,
            planned_run_time: None,
            patience: None,
            balking: None,
            health_check: None,
        },
    )
}

fn sink(i: &str) -> Element {
    el(
        i,
        ElementKind::Sink {
            scrap: None,
            demand: None,
            time_in_system: None,
        },
    )
}

fn model(elements: Vec<Element>, connections: Vec<Connection>) -> Model {
    Model {
        name: "capacity theory".into(),
        elements,
        connections,
        experiment: Experiment {
            warmup: 1_000.0,
            duration: 20_000.0,
            replications: REPS,
            ..Experiment::default()
        },
        ..Model::default()
    }
}

/// Source → server `s` → sink, with `edit` applied to the server.
fn line(rate: f64, units: u32, service: Dist, edit: impl FnOnce(&mut ElementKind)) -> Model {
    let mut s = server("s", units, service);
    edit(&mut s.kind);
    model(
        vec![source(rate), s, sink("out")],
        vec![link("in", "s"), link("s", "out")],
    )
}

fn run(model: &Model) -> ExperimentSummary {
    let compiled = compile(model);
    assert!(compiled.is_ok(), "{compiled:?}");
    let Ok(compiled) = compiled else {
        unreachable!()
    };
    let reports: Vec<_> = (0..u64::from(REPS))
        .map(|r| {
            compiled
                .run(Rng::new(20_000 + r))
                .unwrap_or_else(|e| unreachable!("{e}"))
        })
        .collect();
    summarise(&reports)
}

/// The table's formula and utilisation at `station`.
fn table(m: &Model, station: &str) -> (Formula, f64) {
    let c = m.capacity(DEFAULT_TARGET);
    let row = c.stations.iter().find(|s| s.id.as_str() == station);
    let formula = row.and_then(|s| s.formula.as_ref().ok()).cloned();
    let utilisation = row.and_then(|s| s.utilisation.exact_value());
    match (formula, utilisation) {
        (Some(f), Some(u)) => (f, u),
        _ => unreachable!("no formula or exact utilisation at {station}: {c:#?}"),
    }
}

fn covers(s: &ExperimentSummary, element: &str, metric: Metric, theory: f64) {
    let got = s.get(element, metric);
    eprintln!("{element} {}: {got:?} (table {theory})", metric.name());
    assert!(
        got.is_some_and(|g| g.n == u64::from(REPS) && g.contains(theory)),
        "{element} {}: {got:?} vs the table's {theory}",
        metric.name()
    );
}

/// The wait, queue and utilisation every formula gives.
fn covers_basics(s: &ExperimentSummary, station: &str, f: &Formula, utilisation: f64) {
    covers(s, station, Metric::MeanWait, f.mean_wait);
    covers(s, station, Metric::MeanQueueLength, f.mean_queue);
    covers(s, station, Metric::Utilisation, utilisation);
}

#[test]
fn a_finite_room_fed_by_a_source() {
    let m = line(0.9, 1, Dist::exponential_rate(1.0), |k| {
        if let ElementKind::Server { queue_capacity, .. } = k {
            *queue_capacity = Some(3);
        }
    });
    let (f, u) = table(&m, "s");
    assert_eq!(f.name, "M/M/1/4");
    let s = run(&m);
    covers_basics(&s, "s", &f, u);
    covers(
        &s,
        "s",
        Metric::LossProbability,
        f.lost_share.unwrap_or(f64::NAN),
    );
}

#[test]
fn patience_until_service_starts() {
    // As the call centre: 1 a minute, three agents of 2.5 minutes, patience 2 minutes.
    let m = line(1.0, 3, Dist::exponential_rate(0.4), |k| {
        if let ElementKind::Server { patience, .. } = k {
            *patience = Some(Patience {
                time: Dist::exponential_rate(0.5),
                until: PatienceUntil::Start,
            });
        }
    });
    let (f, u) = table(&m, "s");
    assert_eq!(f.name, "M/M/3+M");
    let s = run(&m);
    covers_basics(&s, "s", &f, u);
    covers(
        &s,
        "s",
        Metric::AbandonedShare,
        f.abandoned_share.unwrap_or(f64::NAN),
    );
}

#[test]
fn balking_by_those_waiting_and_by_those_inside() {
    for counts_in_service in [false, true] {
        let m = line(1.2, 1, Dist::exponential_rate(1.0), |k| {
            if let ElementKind::Server { balking, .. } = k {
                *balking = Some(Balking {
                    join: BalkJoin::Reciprocal,
                    counts_in_service,
                });
            }
        });
        let (f, u) = table(&m, "s");
        let s = run(&m);
        covers_basics(&s, "s", &f, u);
        covers(
            &s,
            "s",
            Metric::BalkedShare,
            f.balked_share.unwrap_or(f64::NAN),
        );
    }
}

#[test]
fn one_unit_with_a_setup_before_every_job() {
    // Service uniform 0.2–0.8 and a setup of 0.2: S = 0.7 with variance 0.03, λ = 0.8.
    let m = line(0.8, 1, Dist::Uniform { min: 0.2, max: 0.8 }, |k| {
        if let ElementKind::Server { setup, .. } = k {
            *setup = Some(Setup::every_job(Dist::Constant(0.2)));
        }
    });
    let (f, u) = table(&m, "s");
    assert_eq!(f.name, "M/G/1");
    covers_basics(&run(&m), "s", &f, u);
}

#[test]
fn two_units_on_a_rework_loop() {
    let mut back = link("check", "s");
    back.probability = Some(0.2);
    let mut on = link("check", "out");
    on.probability = Some(0.8);
    let m = model(
        vec![
            source(1.0),
            server("s", 2, Dist::exponential_rate(1.0)),
            el(
                "check",
                ElementKind::ExclusiveGateway {
                    default: None,
                    routing: stochcraft_model::Routing::Standard,
                },
            ),
            sink("out"),
        ],
        vec![link("in", "s"), link("s", "check"), back, on],
    );
    let (f, u) = table(&m, "s");
    assert_eq!(f.name, "M/M/2");
    covers_basics(&run(&m), "s", &f, u);
}

#[test]
fn a_general_server_after_an_exponential_one() {
    // Burke: the first's departures are Poisson, so the second is M/G/1.
    let m = model(
        vec![
            source(0.6),
            server("first", 1, Dist::exponential_rate(1.0)),
            server("second", 1, Dist::Constant(1.2)),
            sink("out"),
        ],
        vec![
            link("in", "first"),
            link("first", "second"),
            link("second", "out"),
        ],
    );
    let (f1, u1) = table(&m, "first");
    let (f2, u2) = table(&m, "second");
    assert_eq!(f2.name, "M/G/1");
    let s = run(&m);
    covers_basics(&s, "first", &f1, u1);
    covers_basics(&s, "second", &f2, u2);
}
