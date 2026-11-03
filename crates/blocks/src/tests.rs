use stochcraft_model::Problem;

use super::*;

fn problems(r: Result<Report, Vec<Diagnostic>>) -> Vec<Problem> {
    r.err()
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.problem)
        .collect()
}

#[test]
fn deterministic_line_has_exact_times() {
    // Arrivals every 1.0, a 0.5 machine, 2.0 of transport, a 0.5 machine: no queueing, so every
    // entity takes exactly 3.0. The warm-up ends at 10.25 so that no event falls on the boundary:
    // exits at 11, 12, …, 110 are the 100 counted.
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), None);
    let a = m.server("cut", 1, Dist::Constant(0.5));
    let t = m.delay("conveyor", Dist::Constant(2.0));
    let b = m.server("weld", 1, Dist::Constant(0.5));
    let out = m.sink("out");
    m.connect(src, a);
    m.connect(a, t);
    m.connect(t, b);
    m.connect(b, out);
    let r = m.run(Rng::new(1), 10.25, 100.0).ok();
    let get = |id: &str| r.as_ref().and_then(|r| r.element(id)).cloned();
    let close = |v: Option<f64>, x: f64| v.is_some_and(|v| (v - x).abs() < 1e-9);
    let (sink, cut, conveyor) = (get("out"), get("cut"), get("conveyor"));
    assert!(
        close(sink.as_ref().and_then(|s| s.mean_time_in_system), 3.0),
        "{sink:?}"
    );
    assert!(
        close(cut.as_ref().and_then(|c| c.utilisation), 0.5),
        "{cut:?}"
    );
    assert!(close(cut.as_ref().and_then(|c| c.mean_wait), 0.0));
    // Little's law: 1 entity per unit time for 2.0 each.
    assert!(
        close(conveyor.as_ref().and_then(|c| c.mean_contents), 2.0),
        "{conveyor:?}"
    );
    assert_eq!(sink.map(|s| s.left), Some(100));
}

#[test]
fn source_limit_stops_arrivals() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), Some(5));
    let out = m.sink("out");
    m.connect(src, out);
    let r = m.run(Rng::new(1), 0.0, 100.0).ok();
    assert_eq!(
        r.as_ref().and_then(|r| r.element("out")).map(|e| e.left),
        Some(5)
    );
}

#[test]
fn invalid_models_are_refused_with_every_problem() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), None);
    let s = m.server("m", 1, Dist::Constant(1.0));
    m.connect(src, s);
    assert!(problems(m.run(Rng::new(1), 0.0, 1.0)).contains(&Problem::NoOutput));
    m.connect(s, src);
    let p = problems(m.run(Rng::new(1), 0.0, 1.0));
    assert!(
        p.iter().any(|p| matches!(p, Problem::IntoSource { .. })),
        "{p:?}"
    );
    let out = m.sink("in");
    m.connect(s, out);
    let p = problems(m.run(Rng::new(1), f64::NAN, -1.0));
    assert!(
        p.iter().any(|p| matches!(p, Problem::DuplicateId { .. })),
        "{p:?}"
    );
    assert_eq!(
        p.iter()
            .filter(|p| matches!(p, Problem::BadExperiment { .. }))
            .count(),
        2,
        "{p:?}"
    );
    let mut named = Builder::new();
    let a = named.source("1st station", Dist::Constant(1.0), None);
    let z = named.sink("z");
    named.connect(a, z);
    let p = problems(named.run(Rng::new(1), 0.0, 1.0));
    assert!(
        p.contains(&Problem::InvalidId {
            id: "1st station".into()
        }),
        "{p:?}"
    );
}

#[test]
fn same_seed_same_report() {
    let build = || {
        let mut m = Builder::new();
        let src = m.source("in", Dist::exponential_rate(0.8), None);
        let s = m.server(
            "m",
            2,
            Dist::Triangular {
                min: 0.5,
                mode: 1.5,
                max: 3.0,
            },
        );
        let out = m.sink("out");
        m.connect(src, s);
        m.connect(s, out);
        m.run(Rng::new(77), 50.0, 2_000.0)
    };
    assert!(build().is_ok());
    assert_eq!(build(), build());
}

#[test]
fn compiling_a_model_directly() {
    let mut b = Builder::new();
    let src = b.source("arrivals", Dist::exponential_rate(0.5), None);
    let srv = b.server("desk", 1, Dist::exponential_rate(1.0));
    let out = b.sink("exit");
    b.connect(src, srv);
    b.connect(srv, out);
    let model = b.build(Experiment {
        warmup: 10.0,
        duration: 500.0,
        ..Experiment::default()
    });
    // An orphan element is a warning, not an error: the model still compiles.
    let mut with_orphan = model.clone();
    with_orphan.elements.push(Element {
        id: ElementId::unchecked("spare"),
        name: "spare".into(),
        kind: ElementKind::Delay {
            duration: Dist::Constant(1.0),
        },
    });
    with_orphan.connections.push(Connection {
        id: ElementId::unchecked("flow-9"),
        from: ElementId::unchecked("spare"),
        to: ElementId::unchecked("exit"),
    });
    let compiled = compile(&with_orphan);
    assert!(
        compiled.as_ref().is_ok_and(|c| c.warnings().len() == 1),
        "{compiled:?}"
    );
    let Ok(compiled) = compile(&model) else {
        return;
    };
    assert!(compiled.warnings().is_empty());
    let r = compiled.run(Rng::new(3));
    assert_eq!((r.warmup, r.duration), (10.0, 500.0));
    assert_eq!(
        r.elements.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["arrivals", "desk", "exit"]
    );
    // The builder and the compiled model give the same results for the same seed.
    assert_eq!(b.run(Rng::new(3), 10.0, 500.0).ok(), Some(r));
}

#[test]
fn compiled_models_can_be_shared_across_threads() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Compiled>();
}

#[test]
fn summarise_gives_intervals_per_element_and_metric() {
    let mut b = Builder::new();
    let src = b.source("arrivals", Dist::Constant(1.0), None);
    let srv = b.server("desk", 1, Dist::Constant(0.5));
    let out = b.sink("exit");
    b.connect(src, srv);
    b.connect(srv, out);
    let reports: Vec<Report> = (0..5)
        .filter_map(|i| b.run(Rng::new(i), 0.25, 100.0).ok())
        .collect();
    let s = summarise(&reports);
    assert_eq!(s.replications, 5);
    // Over (0.25, 100.25] the desk serves jobs 1–99 fully (0.5 each) and job 100 for 0.25 of
    // its 0.5: (99 × 0.5 + 0.25) / 100 = 0.4975, the same in every replication.
    let u = s.get("desk", Metric::Utilisation);
    assert!(
        u.is_some_and(|u| (u.mean - 0.4975).abs() < 1e-9 && u.half_width.abs() < 1e-9),
        "{u:?}"
    );
    assert!(
        s.get("exit", Metric::MeanTimeInSystem)
            .is_some_and(|w| (w.mean - 0.5).abs() < 1e-9)
    );
    // Metrics an element does not report are absent, not zero.
    assert_eq!(s.get("arrivals", Metric::Utilisation), None);
    assert_eq!(s.get("exit", Metric::MeanWait), None);
    assert_eq!(summarise(&[]).elements, vec![]);
    let names: Vec<&str> = Metric::ALL.iter().map(|m| m.name()).collect();
    assert_eq!(names.len(), 7);
}
