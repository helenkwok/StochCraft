//! A shared task's service time by the pool that serves the entity (servers that differ in
//! speed), and who gets the units that come free at one instant.
//!
//! The cases are worked by hand with constant times. Able and Baker are two pools of one unit
//! each, and the task needs `orResource(able, baker)`: the first that is free serves, Able
//! before Baker.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, Report};
use stochcraft_model::{
    ByResource, Dist, ElementId, ElementKind, Experiment, Need, Problem, ProcessingByResource,
    PropertyValue, Severity,
};
use stochcraft_random::Rng;

fn pool(id: &str) -> Need {
    Need::Resource {
        id: ElementId::unchecked(id),
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

/// Four customers arrive at 1, 2, 3 and 4, and Able serves in 2, Baker in 3 (the task's own
/// time, 100, is never used).
///
/// - The first, at 1, finds both free and goes to Able, until 3.
/// - The second, at 2, finds Able busy: Baker, until 5.
/// - The third, at 3, arrives just as Able finishes: Able again, until 5.
/// - The fourth, at 4, finds both busy and waits. Baker and Able both finish at 5, Baker's job
///   being the one that began first. The fourth takes Able (the first alternative), not Baker,
///   whose unit happened to be handed back first: until 7.
///
/// So Able serves three (busy 2 + 2 + 2 = 6 of 20) and Baker one (busy 3 of 20), and one of the
/// four waited 1.
#[test]
fn units_freed_at_one_instant_go_to_the_first_alternative() {
    let mut b = Builder::new();
    b.pool("able", 1, &[]);
    b.pool("baker", 1, &[]);
    let src = b.source("in", Dist::Constant(1.0), Some(4));
    let t = b.shared_task(
        "serve",
        Dist::Constant(100.0),
        vec![pool("able"), pool("baker")],
        0.0,
    );
    b.processing_by_resource(
        t,
        vec![
            ("able", Dist::Constant(2.0)),
            ("baker", Dist::Constant(3.0)),
        ],
    );
    let out = b.sink("out");
    b.connect(src, t);
    b.connect(t, out);
    let r = b.run(Rng::new(1), 0.0, 20.0);
    let Ok(r) = r else { panic!("{r:?}") };
    assert_eq!(element(&r, "able").entered, 3, "{r:?}");
    assert_eq!(element(&r, "baker").entered, 1, "{r:?}");
    exactly("able", element(&r, "able").utilisation, 6.0 / 20.0);
    exactly("baker", element(&r, "baker").utilisation, 3.0 / 20.0);
    exactly("wait", element(&r, "serve").mean_wait, 0.25);
    assert_eq!(element(&r, "out").entered, 4, "{r:?}");
}

/// An arrival at the instant a unit comes free joins the queue before anyone is served, so it
/// takes the unit of the pool listed first, not the free one it happened to meet: Able serves
/// from 1 to 3 and the second customer arrives at 3, the timer that brings it set long before
/// Able's service ended. Baker has been free all along, and still does not serve it.
#[test]
fn an_arrival_meets_the_units_freed_at_its_instant() {
    let mut b = Builder::new();
    b.pool("able", 1, &[]);
    b.pool("baker", 1, &[]);
    let first = b.source("first", Dist::Constant(1.0), Some(1));
    let second = b.source("second", Dist::Constant(3.0), Some(1));
    let t = b.shared_task(
        "serve",
        Dist::Constant(100.0),
        vec![pool("able"), pool("baker")],
        0.0,
    );
    b.processing_by_resource(
        t,
        vec![
            ("able", Dist::Constant(2.0)),
            ("baker", Dist::Constant(5.0)),
        ],
    );
    let out = b.sink("out");
    b.connect(first, t);
    b.connect(second, t);
    b.connect(t, out);
    let r = b.run(Rng::new(1), 0.0, 20.0);
    let Ok(r) = r else { panic!("{r:?}") };
    assert_eq!(element(&r, "able").entered, 2, "{r:?}");
    assert_eq!(element(&r, "baker").entered, 0, "{r:?}");
    exactly("able", element(&r, "able").utilisation, 4.0 / 20.0);
}

/// Units that come free go to the waiting entities by their task's priority, then by arrival,
/// counting those that arrive at that instant too. Two tasks share one unit: the low task's
/// first customer holds it from 1 to 5 and its second waits from 2; a high-priority customer
/// arrives at 5, through a delay set after the unit's job began, as the unit comes free. It is
/// served first (waiting 0), and the second low customer waits until 6 (4 in all).
#[test]
fn units_freed_go_by_priority_among_those_waiting_at_that_instant() {
    let mut b = Builder::new();
    b.pool("staff", 1, &[]);
    let l1 = b.source("l1", Dist::Constant(1.0), Some(1));
    let l2 = b.source("l2", Dist::Constant(2.0), Some(1));
    let h = b.source("h", Dist::Constant(1.5), Some(1));
    let hold = b.delay("hold", Dist::Constant(3.5));
    let low = b.shared_task("low", Dist::Constant(4.0), vec![pool("staff")], 0.0);
    let high = b.shared_task("high", Dist::Constant(1.0), vec![pool("staff")], 5.0);
    let out = b.sink("out");
    b.connect(l1, low);
    b.connect(l2, low);
    b.connect(h, hold);
    b.connect(hold, high);
    b.connect(low, out);
    b.connect(high, out);
    let r = b.run(Rng::new(1), 0.0, 20.0);
    let Ok(r) = r else { panic!("{r:?}") };
    exactly("high", element(&r, "high").mean_wait, 0.0);
    exactly("low", element(&r, "low").mean_wait, 2.0);
    assert_eq!(element(&r, "out").entered, 3, "{r:?}");
}

/// A pool's entry comes before the entity's time by property, which comes before the task's own
/// time. The task needs `orResource(baker, able)`; Able has an entry (2); the property `kind` has
/// A: 5; the task's own time is 9.
///
/// - A kind A arrives at 0.5: Baker, no entry, so the property's 5 (until 5.5).
/// - A kind B arrives at 1: Baker busy, Able, the entry's 2, not 9 (until 3).
/// - A kind B arrives at 6: Baker is free again and has no entry, B has none by property: the
///   task's own 9 (until 15).
/// - A kind A arrives at 7: Baker busy, Able: the entry's 2, not the property's 5 (until 9).
///
/// Baker is busy 5 + 9 = 14 of 20, Able 2 + 2 = 4 of 20, and Baker served twice, Able twice.
#[test]
fn a_pools_time_comes_before_the_property_and_the_tasks_own() {
    let mut b = Builder::new();
    b.pool("able", 1, &[]);
    b.pool("baker", 1, &[]);
    let kind = |b: &mut Builder, name: &str, at: f64, value: &str| {
        let s = b.source(name, Dist::Constant(at), Some(1));
        b.property(s, "kind", None, PropertyValue::Text(value.into()));
        s
    };
    let a1 = kind(&mut b, "a1", 0.5, "A");
    let b1 = kind(&mut b, "b1", 1.0, "B");
    let b2 = kind(&mut b, "b2", 6.0, "B");
    let a2 = kind(&mut b, "a2", 7.0, "A");
    let t = b.shared_task(
        "serve",
        Dist::Constant(9.0),
        vec![pool("baker"), pool("able")],
        0.0,
    );
    b.processing_by(t, "kind", vec![("A", Dist::Constant(5.0))]);
    b.processing_by_resource(t, vec![("able", Dist::Constant(2.0))]);
    let out = b.sink("out");
    for s in [a1, b1, b2, a2] {
        b.connect(s, t);
    }
    b.connect(t, out);
    let r = b.run(Rng::new(1), 0.0, 20.0);
    let Ok(r) = r else { panic!("{r:?}") };
    exactly("baker", element(&r, "baker").utilisation, 14.0 / 20.0);
    exactly("able", element(&r, "able").utilisation, 4.0 / 20.0);
    assert_eq!(element(&r, "baker").entered, 2, "{r:?}");
    assert_eq!(element(&r, "able").entered, 2, "{r:?}");
}

/// A take that spans pools counts the pool that supplied its first unit, in the order the take
/// was filled: two units from `orResource` by role, from a pool of one and a pool of two.
#[test]
fn a_take_that_spans_pools_counts_the_first_pool() {
    let mut b = Builder::new();
    b.pool("first", 1, &["nurse"]);
    b.pool("second", 2, &["nurse"]);
    let src = b.source("in", Dist::Constant(1.0), Some(1));
    let t = b.shared_task(
        "serve",
        Dist::Constant(100.0),
        vec![Need::Roles {
            roles: vec!["nurse".into()],
            quantity: 2,
        }],
        0.0,
    );
    // The take is one unit of `first`, then one of `second`: `first`'s entry applies.
    b.processing_by_resource(
        t,
        vec![
            ("first", Dist::Constant(4.0)),
            ("second", Dist::Constant(7.0)),
        ],
    );
    let out = b.sink("out");
    b.connect(src, t);
    b.connect(t, out);
    let r = b.run(Rng::new(1), 0.0, 20.0);
    let Ok(r) = r else { panic!("{r:?}") };
    // The customer arrives at 1 and holds one unit of each for 4: `first` is busy 4 of 20, and
    // `second` has 1 of its 2 units busy for 4 of 20.
    exactly("first", element(&r, "first").utilisation, 4.0 / 20.0);
    exactly("second", element(&r, "second").utilisation, 4.0 / 40.0);
}

/// Entries for a pool that never serves leave every result as it was without them: the task's
/// own stream is not advanced by the feature.
#[test]
fn entries_no_entity_uses_change_nothing() {
    let run = |with_entry: bool| {
        let mut b = Builder::new();
        b.pool("main", 1, &[]);
        b.pool("spare", 1, &[]);
        let src = b.source("in", Dist::Constant(100.0), None);
        let t = b.shared_task(
            "serve",
            Dist::Exponential { mean: 1.0 },
            vec![pool("main"), pool("spare")],
            0.0,
        );
        if with_entry {
            b.processing_by_resource(t, vec![("spare", Dist::Exponential { mean: 0.5 })]);
        }
        let out = b.sink("out");
        b.connect(src, t);
        b.connect(t, out);
        let r = b.run(Rng::new(7), 0.0, 1000.0);
        let Ok(r) = r else { panic!("{r:?}") };
        r
    };
    let (without, with) = (run(false), run(true));
    assert_eq!(element(&with, "spare").entered, 0, "{with:?}");
    assert_eq!(without, with);
}

/// An entry for a pool the model lacks, one the task does not need, and one pool twice are
/// errors, never a silent fallback.
#[test]
fn bad_entries_are_diagnosed() {
    let model = |entries: Vec<(&str, Dist)>| {
        let mut b = Builder::new();
        b.pool("able", 1, &[]);
        b.pool("baker", 1, &[]);
        b.pool("cook", 1, &[]);
        let src = b.source("in", Dist::Constant(10.0), None);
        let t = b.shared_task(
            "serve",
            Dist::Constant(1.0),
            vec![pool("able"), pool("baker")],
            0.0,
        );
        b.processing_by_resource(t, entries);
        let out = b.sink("out");
        b.connect(src, t);
        b.connect(t, out);
        b.build(Experiment::default())
            .validate()
            .into_iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.problem)
            .collect::<Vec<_>>()
    };
    let one = |pool: &'static str| vec![(pool, Dist::Constant(1.0))];
    assert_eq!(model(one("able")), vec![]);
    assert_eq!(
        model(vec![
            ("able", Dist::Constant(1.0)),
            ("baker", Dist::Constant(2.0))
        ]),
        vec![]
    );
    assert!(
        model(one("nobody")).contains(&Problem::TimeForUnknownPool {
            id: ElementId::unchecked("nobody")
        }),
        "a pool the model lacks"
    );
    assert!(
        model(one("cook")).contains(&Problem::TimeForPoolNotNeeded {
            id: ElementId::unchecked("cook")
        }),
        "a pool the task does not need"
    );
    assert!(
        model(vec![
            ("able", Dist::Constant(1.0)),
            ("able", Dist::Constant(2.0))
        ])
        .contains(&Problem::DuplicateTimeByPool {
            id: ElementId::unchecked("able")
        }),
        "one pool twice"
    );
}

/// A pool named through a role counts as needed: the entry for a pool with the role is fine.
#[test]
fn a_pool_reached_by_role_is_needed() {
    let mut b = Builder::new();
    b.pool("nurses", 1, &["nurse"]);
    b.pool("doctors", 1, &["doctor"]);
    let src = b.source("in", Dist::Constant(10.0), None);
    let t = b.shared_task(
        "serve",
        Dist::Constant(1.0),
        vec![Need::Roles {
            roles: vec!["nurse".into()],
            quantity: 1,
        }],
        0.0,
    );
    b.processing_by_resource(
        t,
        vec![
            ("nurses", Dist::Constant(1.0)),
            ("doctors", Dist::Constant(2.0)),
        ],
    );
    let out = b.sink("out");
    b.connect(src, t);
    b.connect(t, out);
    let errors: Vec<Problem> = b
        .build(Experiment::default())
        .validate()
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.problem)
        .collect();
    assert_eq!(
        errors,
        vec![Problem::TimeForPoolNotNeeded {
            id: ElementId::unchecked("doctors")
        }]
    );
}

/// The model's own types: an entry is read back by pool, and only a shared task has them.
#[test]
fn the_model_looks_an_entry_up_by_pool() {
    let by = ProcessingByResource {
        by_resource: vec![ByResource {
            resource: ElementId::unchecked("able"),
            time: Dist::Constant(2.0),
        }],
    };
    assert_eq!(
        by.time_for(&ElementId::unchecked("able")),
        Some(&Dist::Constant(2.0))
    );
    assert_eq!(by.time_for(&ElementId::unchecked("baker")), None);
    let mut sink = ElementKind::Sink {
        scrap: None,
        demand: None,
    };
    assert!(sink.processing_by_resource().is_none());
    assert!(sink.processing_by_resource_mut().is_none());
}
