//! A task's processing time by an entity property (a time by product type): an entity whose
//! value has an entry takes that entry's time, any other the task's own.
//!
//! The exact cases are worked by hand. The statistical ones compare one server, Poisson
//! arrivals at rate λ = 0.5, each of kind A (probability 0.7) or B (0.3), exponential service
//! of mean 0.6 for A and 2.0 for B from the table (the task's own time, of mean 100, is never
//! used) with M/G/1 with hyperexponential service, by Pollaczek-Khinchine:
//!
//! - E[S] = 0.7 × 0.6 + 0.3 × 2.0 = 1.02, so ρ = λE[S] = 0.51;
//! - E[S²] = 0.7 × 2 × 0.6² + 0.3 × 2 × 2.0² = 2.904;
//! - Wq = λE[S²] / (2(1 − ρ)) = 1.4816326531, the same for both kinds (first come first
//!   served); Lq = λWq = 0.7408163265; L = Lq + ρ = 1.2508163265; W = Wq + E[S] = 2.5016326531;
//! - each kind's time in system: Wq + 0.6 = 2.0816326531 for A, Wq + 2.0 = 3.4816326531 for B.
#![allow(clippy::panic)]

use stochcraft_blocks::{
    Breakdown, Builder, ExperimentSummary, HealthCheck, Metric, OnShiftEnd, OnTakeout, Patience,
    PatienceUntil, PlaceWhileOut, Report, Setup, compile, summarise,
};
use stochcraft_model::{Dist, ElementId, Experiment, Need, Problem, PropertyValue, Severity};
use stochcraft_random::Rng;

/// One unit of the pool `crew`.
fn crew() -> Need {
    Need::Resource {
        id: ElementId::unchecked("crew"),
        quantity: 1,
    }
}

fn element<'a>(r: &'a Report, id: &str) -> &'a stochcraft_blocks::ElementReport {
    r.element(id).unwrap_or_else(|| panic!("no {id} in {r:?}"))
}

fn exactly(what: &str, got: Option<f64>, want: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() < 1e-9),
        "{what}: {got:?} vs {want}"
    );
}

/// Kinds A and B, 0.7 and 0.3, drawn for each entity at `block`.
fn kinds(b: &mut Builder, block: stochcraft_blocks::BlockId) {
    b.property(
        block,
        "kind",
        None,
        PropertyValue::Choice(vec![(0.7, "A".into()), (0.3, "B".into())]),
    );
}

/// After `task`, a gateway sends kind A to `out_a`, the others to `out_b`.
fn split(b: &mut Builder, task: stochcraft_blocks::BlockId) {
    let by_kind = b.exclusive("by_kind");
    let out_a = b.sink("out_a");
    let out_b = b.sink("out_b");
    b.connect(task, by_kind);
    b.route(by_kind, out_a, None, Some("getProperty('kind') = 'A'"));
    b.route(by_kind, out_b, None, None);
    b.default_route(by_kind, out_b);
}

fn run_once(b: &Builder, duration: f64) -> Report {
    let model = b.build(Experiment {
        duration,
        replications: 1,
        ..Experiment::default()
    });
    let errors: Vec<_> = model
        .validate()
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert_eq!(errors, vec![]);
    let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
    compiled.run(Rng::new(7)).unwrap_or_else(|e| panic!("{e}"))
}

/// A part every 10 minutes, kind A or B; A takes 2, B 3, anything else 50. No part ever waits,
/// so each kind's time in system is its time exactly, whatever kind of task does the work.
#[test]
fn each_kind_takes_its_own_time_on_every_kind_of_task() {
    // How the work is done: 0 a server, 1 a server whose units set up (no time) and so run as
    // units of their own, 2 a task with a shared pool, 3 a delay, 4 a server with a health
    // check that never fails and a patience until done that never runs out (units of their
    // own), 5 a server with that patience only.
    for how in 0..6 {
        let mut b = Builder::new();
        let src = b.source("parts", Dist::Constant(10.0), None);
        kinds(&mut b, src);
        let task = match how {
            2 => {
                b.pool("crew", 1, &[]);
                b.shared_task("task", Dist::Constant(50.0), vec![crew()], 0.0)
            }
            3 => b.delay("task", Dist::Constant(50.0)),
            _ => b.server("task", 1, Dist::Constant(50.0)),
        };
        if how == 1 {
            b.setup(task, Setup::every_job(Dist::Constant(0.0)));
        }
        if how == 4 {
            b.health_check(
                task,
                HealthCheck {
                    interval: Dist::Constant(1.0),
                    limit: f64::INFINITY,
                    restart: Dist::Constant(1.0),
                    keep: None,
                    on_takeout: OnTakeout::Resume,
                    place_while_out: PlaceWhileOut::Reserved,
                },
            );
        }
        if how >= 4 {
            b.patience(
                task,
                Patience {
                    time: Dist::Constant(100.0),
                    until: PatienceUntil::Done,
                },
            );
        }
        b.processing_by(
            task,
            "kind",
            vec![("A", Dist::Constant(2.0)), ("B", Dist::Constant(3.0))],
        );
        b.connect(src, task);
        split(&mut b, task);
        let r = run_once(&b, 10_000.0);
        exactly("A", element(&r, "out_a").mean_time_in_system, 2.0);
        exactly("B", element(&r, "out_b").mean_time_in_system, 3.0);
        let (a, bs) = (element(&r, "out_a").left, element(&r, "out_b").left);
        assert!(a > 500 && bs > 200, "both kinds came: {a} and {bs}");
    }
}

/// An entity with a value the table does not list, or with no value at all, takes the task's
/// own time.
#[test]
fn others_take_the_task_s_own_time() {
    let mut b = Builder::new();
    let listed = b.source("listed", Dist::Constant(10.0), None);
    b.property(listed, "kind", None, PropertyValue::Text("A".into()));
    let unlisted = b.source("unlisted", Dist::Constant(10.0), None);
    b.property(unlisted, "kind", None, PropertyValue::Text("C".into()));
    let none = b.source("none", Dist::Constant(10.0), None);
    for (src, name) in [(listed, "listed"), (unlisted, "unlisted"), (none, "none")] {
        let m = b.server(&format!("m_{name}"), 1, Dist::Constant(5.0));
        b.processing_by(m, "kind", vec![("A", Dist::Constant(2.0))]);
        let out = b.sink(&format!("out_{name}"));
        b.connect(src, m);
        b.connect(m, out);
    }
    let r = run_once(&b, 1_000.0);
    exactly("listed", element(&r, "out_listed").mean_time_in_system, 2.0);
    exactly(
        "unlisted",
        element(&r, "out_unlisted").mean_time_in_system,
        5.0,
    );
    exactly("none", element(&r, "out_none").mean_time_in_system, 5.0);
}

/// Values are compared as text: a number 2 matches the entry `2`.
#[test]
fn a_number_matches_its_text() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    b.property(src, "size", None, PropertyValue::Number(2.0));
    let m = b.server("m", 1, Dist::Constant(5.0));
    b.processing_by(m, "size", vec![("2", Dist::Constant(1.5))]);
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let r = run_once(&b, 1_000.0);
    exactly("2", element(&r, "out").mean_time_in_system, 1.5);
}

/// A failure interrupts a job and it resumes after the repair with what was left of the
/// entity's own time: a part every 10, A taking 2 and B 3, and a failure every 25 minutes of
/// elapsed time with a repair of 1. A part's time is its work plus any repair it waits
/// through, never the task's own 50.
#[test]
fn an_interrupted_job_resumes_with_its_own_time() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    kinds(&mut b, src);
    let m = b.server("task", 1, Dist::Constant(50.0));
    b.processing_by(
        m,
        "kind",
        vec![("A", Dist::Constant(2.0)), ("B", Dist::Constant(3.0))],
    );
    b.breakdown(
        m,
        Breakdown {
            time_to_failure: Dist::Constant(25.0),
            repair: Dist::Constant(1.0),
            interruptible: true,
            hands_on_while_down: true,
        },
    );
    b.connect(src, m);
    split(&mut b, m);
    let r = run_once(&b, 10_000.0);
    let a = element(&r, "out_a").mean_time_in_system.unwrap_or(0.0);
    let bs = element(&r, "out_b").mean_time_in_system.unwrap_or(0.0);
    assert!(a > 2.0 && a < 3.0, "A: {a}");
    assert!(bs > 3.0 && bs < 4.0, "B: {bs}");
}

/// A job paused by its unit's shift ending (resume) carries on with what was left of the
/// entity's own time: the unit is off from 10.5 to 20.5, and a part of kind A (3 minutes)
/// arriving at 9 works 1.5, waits 10 and finishes at 22, 13 after it came.
#[test]
fn a_job_paused_off_shift_resumes_with_its_own_time() {
    use stochcraft_model::DateTime;
    let at = |m: u32, s: u32| DateTime::new(2016, 1, 1, 0, m, s).unwrap_or(DateTime(0));
    let mut b = Builder::new();
    b.calendar(
        "break",
        vec![stochcraft_blocks::Event {
            start: at(10, 30),
            end: at(20, 30),
            rule: None,
        }],
    );
    let src = b.source("parts", Dist::Constant(9.0), Some(1));
    b.property(src, "kind", None, PropertyValue::Text("A".into()));
    let m = b.server("task", 1, Dist::Constant(50.0));
    b.server_shifts(m, 1, &[("break", 0)], OnShiftEnd::Resume);
    b.processing_by(m, "kind", vec![("A", Dist::Constant(3.0))]);
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let model = b.build(Experiment {
        duration: 60.0,
        replications: 1,
        start: Some(at(0, 0)),
        ..Experiment::default()
    });
    let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
    let r = compiled.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
    exactly(
        "time in system",
        element(&r, "out").mean_time_in_system,
        13.0,
    );
}

/// Entries that no entity matches change nothing: every draw is that of the model without
/// them, for a plain server, one whose units fail, a task with a shared pool and a delay.
#[test]
fn entries_no_entity_matches_change_nothing() {
    for how in 0..4 {
        let run = |table: bool| {
            let mut b = Builder::new();
            let src = b.source("parts", Dist::Exponential { mean: 1.0 }, None);
            kinds(&mut b, src);
            let task = match how {
                0 | 1 => b.server("task", 1, Dist::Exponential { mean: 0.8 }),
                2 => {
                    b.pool("crew", 1, &[]);
                    b.shared_task("task", Dist::Exponential { mean: 0.8 }, vec![crew()], 0.0)
                }
                _ => b.delay("task", Dist::Exponential { mean: 0.8 }),
            };
            if how == 1 {
                b.breakdown(
                    task,
                    Breakdown {
                        time_to_failure: Dist::Exponential { mean: 50.0 },
                        repair: Dist::Exponential { mean: 2.0 },
                        interruptible: true,
                        hands_on_while_down: true,
                    },
                );
            }
            if table {
                b.processing_by(task, "kind", vec![("Z", Dist::Exponential { mean: 9.0 })]);
            }
            let out = b.sink("out");
            b.connect(src, task);
            b.connect(task, out);
            let Ok(r) = b.run(Rng::new(11), 0.0, 5_000.0) else {
                panic!("no report")
            };
            let (t, e) = (element(&r, "task").clone(), element(&r, "out").clone());
            (e.left, e.mean_time_in_system, t.utilisation, t.mean_wait)
        };
        assert_eq!(run(true), run(false), "case {how}");
    }
}

/// Entries draw from a stream of their own, and the task's own time from its service stream
/// as before: entities of kind B, which have no entry, take the service stream's draws in
/// order. With no waiting (a part every 10, times under 10), B's times are then the first draws
/// of the service stream: the same as those of the first parts of a model of B alone, which
/// has the same arrivals and kinds but serves only B by the task's own time.
#[test]
fn the_others_keep_the_service_stream() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    kinds(&mut b, src);
    let m = b.server("task", 1, Dist::Uniform { min: 1.0, max: 2.0 });
    b.processing_by(m, "kind", vec![("A", Dist::Constant(1.0))]);
    b.connect(src, m);
    split(&mut b, m);
    let with = run_once(&b, 10_000.0);
    exactly("A", element(&with, "out_a").mean_time_in_system, 1.0);
    // The same B parts, each served by the stream's next draw: a model where A's entry is the
    // stream's own distribution would differ, so compare with all parts B instead.
    let n = element(&with, "out_b").left;
    let mut only_b = Builder::new();
    let src = only_b.source("parts", Dist::Constant(10.0), Some(n));
    only_b.property(src, "kind", None, PropertyValue::Text("B".into()));
    let m = only_b.server("task", 1, Dist::Uniform { min: 1.0, max: 2.0 });
    only_b.processing_by(m, "kind", vec![("A", Dist::Constant(1.0))]);
    let out = only_b.sink("out");
    only_b.connect(src, m);
    only_b.connect(m, out);
    let limited = run_once(&only_b, 10_000.0);
    assert_eq!(element(&limited, "out").left, n);
    let close = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() < 1e-9,
        _ => false,
    };
    assert!(
        close(
            element(&with, "out_b").mean_time_in_system,
            element(&limited, "out").mean_time_in_system
        ),
        "B's times are the service stream's first draws"
    );
}

fn diagnosed(b: &Builder) -> Vec<Problem> {
    b.build(Experiment::default())
        .validate()
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.problem)
        .collect()
}

/// Entries without a property, a property no element sets, one value twice and a time with a
/// negative mean are errors.
#[test]
fn bad_tables_are_diagnosed() {
    let model = |property: &str, by: Vec<(&str, Dist)>| {
        let mut b = Builder::new();
        let src = b.source("parts", Dist::Constant(10.0), None);
        b.property(src, "kind", None, PropertyValue::Text("A".into()));
        let m = b.server("m", 1, Dist::Constant(1.0));
        b.processing_by(m, property, by);
        let out = b.sink("out");
        b.connect(src, m);
        b.connect(m, out);
        diagnosed(&b)
    };
    let one = || vec![("A", Dist::Constant(1.0))];
    assert_eq!(model("kind", one()), vec![]);
    assert!(
        model(" ", one()).contains(&Problem::TimesByValueWithoutProperty),
        "no property"
    );
    assert!(
        model("colour", one()).contains(&Problem::UnknownProperty {
            name: "colour".into()
        }),
        "unknown property"
    );
    assert!(
        model(
            "kind",
            vec![("A", Dist::Constant(1.0)), ("A", Dist::Constant(2.0))]
        )
        .contains(&Problem::DuplicateTimeByValue { value: "A".into() }),
        "one value twice"
    );
    assert!(
        model(
            "kind",
            vec![(
                "A",
                Dist::Normal {
                    mean: -1.0,
                    std_dev: 1.0
                }
            )]
        )
        .contains(&Problem::TimeByValueNegative { value: "A".into() }),
        "negative mean"
    );
    // A bad distribution in the table is reported as the task's service time's would be.
    assert!(
        model("kind", vec![("A", Dist::Uniform { min: 2.0, max: 1.0 })])
            .iter()
            .any(|p| matches!(p, Problem::BadDistribution { .. })),
        "bad distribution"
    );
}

/// The task's own time (mean 100) would overload the server; the table's times do not, so no
/// overload warning.
#[test]
fn the_load_check_uses_the_least_time() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Exponential { mean: 2.0 }, None);
    kinds(&mut b, src);
    let m = b.server("m", 1, Dist::Exponential { mean: 100.0 });
    b.processing_by(
        m,
        "kind",
        vec![
            ("A", Dist::Exponential { mean: 0.6 }),
            ("B", Dist::Exponential { mean: 2.0 }),
        ],
    );
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let warned = b
        .build(Experiment::default())
        .validate()
        .into_iter()
        .any(|d| matches!(d.problem, Problem::Overloaded { .. }));
    assert!(!warned);
}

const REPLICATIONS: u64 = 20;

fn summary(b: &Builder) -> ExperimentSummary {
    let model = b.build(Experiment {
        warmup: 5_000.0,
        duration: 200_000.0,
        replications: REPLICATIONS as u32,
        ..Experiment::default()
    });
    let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
    let reports: Vec<Report> = (0..REPLICATIONS)
        .map(|r| {
            compiled
                .run(Rng::new(40_000 + r))
                .unwrap_or_else(|e| panic!("{e}"))
        })
        .collect();
    summarise(&reports)
}

fn covers(s: &ExperimentSummary, element: &str, metric: Metric, theory: f64) {
    let got = s.get(element, metric);
    eprintln!("{element} {}: {got:?} (theory {theory})", metric.name());
    assert!(
        got.is_some_and(|g| g.n == REPLICATIONS && g.contains(theory)),
        "{element} {}: {got:?} vs {theory}",
        metric.name()
    );
}

/// The M/G/1 server of the module's notes, with `end` after it.
fn mg1(end: impl Fn(&mut Builder, stochcraft_blocks::BlockId)) -> Builder {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(0.5), None);
    kinds(&mut b, arrivals);
    let desk = b.server("desk", 1, Dist::Exponential { mean: 100.0 });
    b.processing_by(
        desk,
        "kind",
        vec![
            ("A", Dist::Exponential { mean: 0.6 }),
            ("B", Dist::Exponential { mean: 2.0 }),
        ],
    );
    b.connect(arrivals, desk);
    end(&mut b, desk);
    b
}

const WQ: f64 = 1.481_632_653_1;

#[test]
#[ignore = "long runs against Pollaczek-Khinchine; CI runs it in release with the ignored tests"]
fn hyperexponential_service_matches_pollaczek_khinchine() {
    let s = summary(&mg1(|b, desk| {
        let exit = b.sink("exit");
        b.connect(desk, exit);
    }));
    covers(&s, "desk", Metric::Utilisation, 0.51);
    covers(&s, "desk", Metric::MeanWait, WQ);
    covers(&s, "desk", Metric::MeanQueueLength, 0.740_816_326_5);
    covers(&s, "desk", Metric::MeanWorkInProgress, 1.250_816_326_5);
    covers(&s, "exit", Metric::MeanTimeInSystem, 2.501_632_653_1);
}

#[test]
#[ignore = "long runs against Pollaczek-Khinchine; CI runs it in release with the ignored tests"]
fn each_kind_s_time_in_system_matches_pollaczek_khinchine() {
    let s = summary(&mg1(split));
    covers(&s, "out_a", Metric::MeanTimeInSystem, WQ + 0.6);
    covers(&s, "out_b", Metric::MeanTimeInSystem, WQ + 2.0);
}
