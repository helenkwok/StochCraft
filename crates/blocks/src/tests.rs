use stochcraft_model::Problem;

use super::*;

fn problems(r: Result<Report, Failed>) -> Vec<Problem> {
    let Err(Failed::Invalid(ds)) = r else {
        return Vec::new();
    };
    ds.into_iter().map(|d| d.problem).collect()
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
    with_orphan.elements.push(Element::new(
        ElementId::unchecked("spare"),
        "spare",
        ElementKind::Delay {
            duration: Dist::Constant(1.0),
        },
    ));
    with_orphan.connections.push(Connection::new(
        ElementId::unchecked("flow-9"),
        ElementId::unchecked("spare"),
        ElementId::unchecked("exit"),
    ));
    let compiled = compile(&with_orphan);
    assert!(
        compiled.as_ref().is_ok_and(|c| c.warnings().len() == 1),
        "{compiled:?}"
    );
    let Ok(compiled) = compile(&model) else {
        return;
    };
    assert!(compiled.warnings().is_empty());
    let Ok(r) = compiled.run(Rng::new(3)) else {
        unreachable!()
    };
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
    assert_eq!(names.len(), 22);
}

#[test]
fn summarise_leaves_out_replications_missing_a_metric() {
    // Random service, so the desk's utilisation differs from one replication to the next.
    let mut b = Builder::new();
    let src = b.source("arrivals", Dist::Exponential { mean: 1.0 }, None);
    let srv = b.server("desk", 1, Dist::Exponential { mean: 0.5 });
    let out = b.sink("exit");
    b.connect(src, srv);
    b.connect(srv, out);
    let mut reports: Vec<Report> = (1..=4)
        .filter_map(|i| b.run(Rng::new(i), 0.25, 100.0).ok())
        .collect();
    assert_eq!(reports.len(), 4);
    let utilisation: Vec<Option<f64>> = reports
        .iter()
        .map(|r| r.element("desk").and_then(|d| d.utilisation))
        .collect();
    let values: Vec<f64> = utilisation.iter().flatten().copied().collect();
    assert_eq!(values.len(), 4, "{utilisation:?}");
    let entered: Vec<f64> = reports
        .iter()
        .filter_map(|r| r.element("desk").map(|d| d.entered as f64))
        .collect();
    // Remove the desk's utilisation from replications 2 and 4 (indices 1 and 3) after the run.
    for k in [1, 3] {
        let desk = reports
            .get_mut(k)
            .and_then(|r| r.elements.iter_mut().find(|e| e.id.as_str() == "desk"));
        if let Some(d) = desk {
            d.utilisation = None;
        }
    }
    let s = summarise(&reports);
    let u = s.get("desk", Metric::Utilisation);
    // Only replications 1 and 3 count: the mean is over those two, not all four.
    let (u0, u2) = (values[0], values[2]);
    assert!(
        u.is_some_and(|u| u.n == 2 && (u.mean - (u0 + u2) / 2.0).abs() < 1e-12),
        "{u:?}"
    );
    let samples = s
        .elements
        .iter()
        .find(|e| e.id.as_str() == "desk")
        .and_then(|e| e.samples(Metric::Utilisation));
    assert_eq!(samples, Some(&[Some(u0), None, Some(u2), None][..]));
    // Other metrics of the same element still count every replication.
    let n = entered.len() as f64;
    let mean_entered = entered.iter().sum::<f64>() / n;
    assert!(
        s.get("desk", Metric::Entered)
            .is_some_and(|e| e.n == 4 && (e.mean - mean_entered).abs() < 1e-9)
    );
}

fn mean_assigned(r: &Report, element: &str, property: &str) -> Option<f64> {
    r.element(element)?
        .properties
        .iter()
        .find(|(n, _)| n == property)?
        .1
}

#[test]
fn properties_count_down_through_a_repair_line() {
    // The mechanics of the BPSim guide's car repair loop, unrolled (the loop itself needs the
    // gateways of M2.3): each car arrives with 3 issues, each repair station removes one, and
    // the exit records whether none are left.
    let mut m = Builder::new();
    let src = m.source("cars", Dist::Constant(1.0), Some(20));
    m.property(
        src,
        "issues",
        Some(PropertyType::Long),
        PropertyValue::Number(3.0),
    );
    let mut prev = src;
    for name in ["repair1", "repair2", "repair3"] {
        let r = m.delay(name, Dist::Constant(0.5));
        m.property(
            r,
            "issues",
            None,
            PropertyValue::Expression("getProperty('issues') - 1".into()),
        );
        m.connect(prev, r);
        prev = r;
    }
    let out = m.sink("out");
    m.property(
        out,
        "fixed",
        None,
        PropertyValue::Expression("getProperty('issues') = 0".into()),
    );
    m.connect(prev, out);
    let r = m.run(Rng::new(1), 0.0, 100.0).ok();
    let mean = |e: &str, p: &str| r.as_ref().and_then(|r| mean_assigned(r, e, p));
    assert_eq!(mean("cars", "issues"), Some(3.0));
    assert_eq!(mean("repair1", "issues"), Some(2.0));
    assert_eq!(mean("repair2", "issues"), Some(1.0));
    assert_eq!(mean("repair3", "issues"), Some(0.0));
    assert_eq!(mean("out", "fixed"), Some(1.0));
    assert_eq!(
        r.as_ref().and_then(|r| r.element("out")).map(|e| e.left),
        Some(20)
    );
}

#[test]
fn assignments_at_one_element_see_the_values_it_arrived_with() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), Some(5));
    m.property(src, "a", None, PropertyValue::Number(1.0));
    m.property(src, "b", None, PropertyValue::Number(10.0));
    let swap = m.delay("swap", Dist::Constant(0.0));
    // Together, not one after the other: b gets the old a, a the old b.
    m.property(
        swap,
        "b",
        None,
        PropertyValue::Expression("getProperty('a')".into()),
    );
    m.property(
        swap,
        "a",
        None,
        PropertyValue::Expression("getProperty('b')".into()),
    );
    let out = m.sink("out");
    m.property(
        out,
        "check",
        None,
        PropertyValue::Expression("getProperty('a') * 100 + getProperty('b')".into()),
    );
    m.connect(src, swap);
    m.connect(swap, out);
    let r = m.run(Rng::new(1), 0.0, 10.0).ok();
    assert_eq!(
        r.as_ref().and_then(|r| mean_assigned(r, "out", "check")),
        Some(1001.0)
    );
}

#[test]
fn values_take_their_property_type() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), Some(4));
    m.property(
        src,
        "n",
        Some(PropertyType::Long),
        PropertyValue::Expression("2.5".into()),
    );
    m.property(
        src,
        "flag",
        Some(PropertyType::Boolean),
        PropertyValue::Expression("'yes'".into()),
    );
    m.property(
        src,
        "label",
        Some(PropertyType::String),
        PropertyValue::Expression("concat('car-', 7)".into()),
    );
    let out = m.sink("out");
    m.property(
        out,
        "ok",
        None,
        PropertyValue::Expression(
            "getProperty('n') = 3 and getProperty('flag') and getProperty('label') = 'car-7'"
                .into(),
        ),
    );
    m.property(
        out,
        "never",
        Some(PropertyType::Long),
        PropertyValue::Expression("getProperty('label') + 1".into()),
    );
    m.connect(src, out);
    let r = m.run(Rng::new(1), 0.0, 10.0).ok();
    let mean = |e: &str, p: &str| r.as_ref().and_then(|r| mean_assigned(r, e, p));
    assert_eq!(mean("in", "n"), Some(3.0), "2.5 rounds to 3");
    assert_eq!(mean("out", "ok"), Some(1.0));
    // Text is not a number, so the long is left unset and nothing is recorded.
    assert_eq!(mean("out", "never"), None);
    assert_eq!(mean("in", "label"), None, "text has no mean");
}

#[test]
fn random_properties_have_their_mean_and_change_no_other_result() {
    let build = |with_properties: bool| {
        let mut m = Builder::new();
        let src = m.source("in", Dist::exponential_rate(0.8), None);
        let desk = m.server("desk", 1, Dist::exponential_rate(1.0));
        let out = m.sink("out");
        if with_properties {
            m.property(
                src,
                "issues",
                Some(PropertyType::Long),
                PropertyValue::Random(Dist::Poisson { mean: 2.0 }),
            );
            m.property(
                desk,
                "minutes",
                None,
                PropertyValue::Random(Dist::Uniform { min: 0.0, max: 1.0 }),
            );
        }
        m.connect(src, desk);
        m.connect(desk, out);
        m.run(Rng::new(7), 100.0, 20_000.0).ok()
    };
    let (plain, with) = (build(false), build(true));
    assert!(plain.is_some() && with.is_some());
    let (plain, with) = (plain.unwrap_or_else(empty), with.unwrap_or_else(empty));
    for (a, b) in plain.elements.iter().zip(&with.elements) {
        for metric in Metric::ALL {
            assert_eq!(a.metric(metric), b.metric(metric), "{} {metric:?}", a.id);
        }
    }
    let issues = mean_assigned(&with, "in", "issues").unwrap_or(f64::NAN);
    assert!((issues - 2.0).abs() < 0.05, "{issues}");
    let minutes = mean_assigned(&with, "desk", "minutes").unwrap_or(f64::NAN);
    assert!((minutes - 0.5).abs() < 0.02, "{minutes}");
}

fn empty() -> Report {
    Report {
        stopped_at: None,
        warmup: 0.0,
        duration: 0.0,
        elements: vec![],
        measured_from: 0.0,
        measured_to: 0.0,
        after_warmup: true,
    }
}

#[test]
fn property_means_are_summarised_across_replications() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), None);
    m.property(
        src,
        "x",
        None,
        PropertyValue::Random(Dist::Uniform { min: 0.0, max: 2.0 }),
    );
    let out = m.sink("out");
    m.connect(src, out);
    let reports: Vec<Report> = (0..5)
        .filter_map(|i| m.run(Rng::new(i), 0.0, 200.0).ok())
        .collect();
    let s = summarise(&reports);
    let x = s.elements.first().and_then(|e| e.properties.first());
    assert!(
        x.is_some_and(|(n, s)| n == "x" && s.n == 5 && (s.mean - 1.0).abs() < 0.1),
        "{x:?}"
    );
    assert!(
        s.elements.get(1).is_some_and(|e| e.properties.is_empty()),
        "the sink assigns nothing"
    );
}

#[test]
fn a_bad_expression_stops_compilation_with_its_position() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), None);
    m.property(
        src,
        "n",
        None,
        PropertyValue::Expression("getProperty('n') +* 2".into()),
    );
    let out = m.sink("out");
    m.connect(src, out);
    let p = problems(m.run(Rng::new(1), 0.0, 1.0));
    assert!(
        matches!(p.as_slice(), [Problem::BadExpression { error, .. }] if error.position == 18),
        "{p:?}"
    );
}

#[test]
fn a_mix_draws_each_value_by_its_share_and_routes_by_it() {
    // A product mix: A three parts in five, B two. A decision sends each type its own way.
    let mut m = Builder::new();
    let src = m.source("parts", Dist::Constant(1.0), Some(20_000));
    m.property(
        src,
        "type",
        None,
        PropertyValue::Choice(vec![(3.0, "A".into()), (2.0, "B".into())]),
    );
    let split = m.exclusive("by_type");
    let a = m.sink("a");
    let b = m.sink("b");
    m.connect(src, split);
    m.route(split, a, None, Some("getProperty('type') = 'A'"));
    m.route(split, b, None, Some("getProperty('type') = 'B'"));
    let r = m.run(Rng::new(7), 0.0, 30_000.0).ok();
    let left = |e: &str| r.as_ref().and_then(|r| r.element(e)).map_or(0, |e| e.left);
    let (na, nb) = (left("a"), left("b"));
    assert_eq!(na + nb, 20_000);
    // 0.6 within five standard deviations of a binomial share (sd 0.0035).
    let share = na as f64 / 20_000.0;
    assert!((share - 0.6).abs() < 0.0175, "share of A {share}");
}

#[test]
fn a_mix_needs_shares_that_add_up_to_more_than_zero() {
    let mut m = Builder::new();
    let src = m.source("parts", Dist::Constant(1.0), Some(5));
    m.property(
        src,
        "type",
        None,
        PropertyValue::Choice(vec![(0.0, "A".into()), (-1.0, "B".into())]),
    );
    let out = m.sink("out");
    m.connect(src, out);
    let model = m.build(Experiment::default());
    assert!(
        model
            .validate()
            .iter()
            .any(|d| matches!(&d.problem, Problem::BadProperty { name, .. } if name == "type")),
        "{:?}",
        model.validate()
    );
}

#[test]
fn a_mix_whose_shares_overflow_is_refused() {
    let mut m = Builder::new();
    let src = m.source("parts", Dist::Constant(1.0), Some(5));
    m.property(
        src,
        "type",
        None,
        PropertyValue::Choice(vec![(1e308, "A".into()), (1e308, "B".into())]),
    );
    let out = m.sink("out");
    m.connect(src, out);
    let model = m.build(Experiment::default());
    assert!(
        model
            .validate()
            .iter()
            .any(|d| matches!(&d.problem, Problem::BadProperty { name, .. } if name == "type"))
    );
}
