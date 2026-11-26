#![allow(clippy::panic)]

//! The capacity table against textbook numbers (Gross, Shortle, Thompson and Harris,
//! *Fundamentals of Queueing Theory*, 4th ed.) and one test for each reason a number is not
//! given.

use super::*;
use crate::{
    BalkJoin, Balking, Breakdown, Experiment, Need, OnShiftEnd, Patience, QueueLimit, Requirement,
    ResourcePool, Setup,
};

fn id(s: &str) -> ElementId {
    ElementId(s.to_owned())
}

fn el(i: &str, kind: ElementKind) -> Element {
    Element::new(id(i), i, kind)
}

fn source(i: &str, interarrival: Dist) -> Element {
    el(
        i,
        ElementKind::Source {
            interarrival,
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
        },
    )
}

fn gateway(i: &str) -> Element {
    el(
        i,
        ElementKind::ExclusiveGateway {
            default: None,
            routing: Routing::Standard,
        },
    )
}

fn link(from: &str, to: &str) -> Connection {
    Connection::new(id(&format!("{from}-{to}")), id(from), id(to))
}

fn chance(from: &str, to: &str, p: f64) -> Connection {
    Connection {
        probability: Some(p),
        ..link(from, to)
    }
}

fn model(elements: Vec<Element>, connections: Vec<Connection>) -> Model {
    Model {
        name: "capacity".into(),
        elements,
        connections,
        resources: Vec::new(),
        calendars: Vec::new(),
        experiment: Experiment::default(),
        layout: BTreeMap::new(),
        estimates: BTreeMap::new(),
        starting: BTreeMap::new(),
        monte_carlo: None,
        site: Default::default(),
        agents: None,
        decisions: BTreeMap::new(),
        dmn: BTreeMap::new(),
    }
}

/// Source → server → sink.
fn line(arrival_rate: f64, units: u32, service: Dist) -> Model {
    model(
        vec![
            source("in", Dist::exponential_rate(arrival_rate)),
            server("s", units, service),
            sink("out"),
        ],
        vec![link("in", "s"), link("s", "out")],
    )
}

fn row<'a>(c: &'a Capacity, i: &str) -> &'a Station {
    match c.stations.iter().find(|s| s.id.as_str() == i) {
        Some(s) => s,
        None => panic!("no row {i} in {c:#?}"),
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

fn formula(s: &Station) -> &Formula {
    match &s.formula {
        Ok(f) => f,
        Err(w) => panic!("no formula at {}: {w:?}", s.id),
    }
}

fn reason(s: &Station) -> &Why {
    match &s.formula {
        Ok(f) => panic!("a formula at {}: {f:?}", s.id),
        Err(w) => w,
    }
}

#[test]
fn mm1_matches_the_textbook() {
    // λ = 0.75, μ = 1: ρ = 0.75, Wq = ρ/(μ − λ) = 3, Lq = 2.25.
    let c = line(0.75, 1, Dist::exponential_rate(1.0)).capacity(DEFAULT_TARGET);
    let s = row(&c, "s");
    assert_eq!(s.arrival_rate, Value::exact(0.75));
    assert!(near(s.utilisation.exact_value().unwrap_or(0.0), 0.75));
    assert_eq!(s.status, Status::Fine);
    let f = formula(s);
    assert_eq!(f.name, "M/M/1");
    assert!(near(f.mean_wait, 3.0), "{f:?}");
    assert!(near(f.mean_queue, 2.25), "{f:?}");
    assert!(near(f.wait_probability.unwrap_or(0.0), 0.75));
    assert_eq!(c.bottleneck, Some(id("s")));
}

#[test]
fn mm2_matches_erlang_c() {
    // λ = 1.5, μ = 1, c = 2: Erlang C = 9/14, Wq = 9/7.
    let c = line(1.5, 2, Dist::exponential_rate(1.0)).capacity(DEFAULT_TARGET);
    let f = formula(row(&c, "s"));
    assert!(near(f.mean_wait, 9.0 / 7.0), "{f:?}");
    assert!(near(f.wait_probability.unwrap_or(0.0), 9.0 / 14.0));
}

#[test]
fn erlang_c_holds_for_many_units() {
    // 500 units at 98%: the recursion stays finite, and the wait is short and positive.
    let c = line(490.0, 500, Dist::exponential_rate(1.0)).capacity(0.99);
    let f = formula(row(&c, "s"));
    assert!(
        f.mean_wait.is_finite() && f.mean_wait > 0.0 && f.mean_wait < 1.0,
        "{f:?}"
    );
}

#[test]
fn mg1_matches_pollaczek_khinchine() {
    // Constant service 1, λ = 0.5: Wq = λ E[S²] / (2 (1 − ρ)) = 0.5.
    let c = line(0.5, 1, Dist::Constant(1.0)).capacity(DEFAULT_TARGET);
    let f = formula(row(&c, "s"));
    assert_eq!(f.name, "M/G/1");
    assert!(near(f.mean_wait, 0.5), "{f:?}");
}

#[test]
fn a_user_distribution_service_is_mg1() {
    // Service uniform on [0, 2] given as a table (an atom of 0 at 0, then even to 2): mean 1,
    // variance 1/3. At λ = 0.5, Wq = λ (variance + mean²) / (2 (1 − ρ)) = 2/3.
    let table = Dist::User {
        points: vec![(0.0, 0.0), (1.0, 2.0)],
        discrete: false,
    };
    let c = line(0.5, 1, table).capacity(DEFAULT_TARGET);
    let f = formula(row(&c, "s"));
    assert_eq!(f.name, "M/G/1");
    assert!(near(f.mean_wait, 2.0 / 3.0), "{f:?}");
}

#[test]
fn a_pert_service_is_mg1() {
    // PERT(10, 20, 60) is a beta on [10, 60] with shapes 1.8 and 4.2: mean 25 and variance
    // 50² · 1.8 · 4.2 / (6² · 7) = 75. At λ = 0.02 (ρ = 0.5), Wq = 0.02 · (75 + 625) / 1 = 14;
    // the fitted table is within a few percent of that.
    let fitted = crate::ThreePoint {
        family: crate::Family::Pert,
        min: 10.0,
        mode: 20.0,
        upper: 60.0,
        upper_is: crate::Upper::Max,
    }
    .fit();
    let Ok(pert) = fitted else {
        panic!("a PERT fit: {fitted:?}")
    };
    let c = line(0.02, 1, pert).capacity(DEFAULT_TARGET);
    let f = formula(row(&c, "s"));
    assert_eq!(f.name, "M/G/1");
    assert!((f.mean_wait / 14.0 - 1.0).abs() < 0.1, "{f:?}");
}

#[test]
fn a_setup_before_every_job_is_part_of_the_service() {
    // Service 0.6 constant plus setup 0.4 constant: M/G/1 with S = 1.
    let mut m = line(0.5, 1, Dist::Constant(0.6));
    if let ElementKind::Server { setup, .. } = &mut m.elements[1].kind {
        *setup = Some(Setup::every_job(Dist::Constant(0.4)));
    }
    let c = m.capacity(DEFAULT_TARGET);
    let s = row(&c, "s");
    assert_eq!(s.mean_service, Value::exact(1.0));
    assert!(near(formula(s).mean_wait, 0.5));
}

#[test]
fn a_finite_room_fed_by_a_source_is_mm1k() {
    // λ = μ = 1, one unit, two places to wait (K = 3): each state 1/4, so a quarter are turned
    // away, Lq = 0.75 and Wq = Lq / (0.75 λ) = 1.
    let mut m = line(1.0, 1, Dist::exponential_rate(1.0));
    if let ElementKind::Server { queue_capacity, .. } = &mut m.elements[1].kind {
        *queue_capacity = Some(2);
    }
    let c = m.capacity(DEFAULT_TARGET);
    let s = row(&c, "s");
    let f = formula(s);
    assert_eq!(f.name, "M/M/1/3");
    assert!(near(f.lost_share.unwrap_or(0.0), 0.25), "{f:?}");
    assert!(near(f.mean_queue, 0.75), "{f:?}");
    assert!(near(f.mean_wait, 1.0), "{f:?}");
    // Not unstable at 100% offered: the full room turns the rest away. Utilisation is what
    // is served.
    assert_ne!(s.status, Status::Unstable);
    assert!(near(s.utilisation.exact_value().unwrap_or(0.0), 0.75));
}

#[test]
fn patience_until_service_starts_is_erlang_a() {
    // M/M/1+M with θ = μ = 1 and λ = 1: the number inside is Poisson(1), so Lq = e⁻¹ and the
    // share giving up θ Lq / λ = e⁻¹.
    let mut m = line(1.0, 1, Dist::exponential_rate(1.0));
    if let ElementKind::Server { patience, .. } = &mut m.elements[1].kind {
        *patience = Some(Patience {
            time: Dist::exponential_rate(1.0),
            until: PatienceUntil::Start,
        });
    }
    let c = m.capacity(DEFAULT_TARGET);
    let f = formula(row(&c, "s"));
    assert_eq!(f.name, "M/M/1+M");
    let e1 = stochcraft_random::math::exp(-1.0);
    assert!(near(f.mean_queue, e1), "{f:?}");
    assert!(near(f.abandoned_share.unwrap_or(0.0), e1), "{f:?}");
}

#[test]
fn balking_is_a_birth_death_queue() {
    // b(n) = 1/(n + 1) of those waiting, one unit, λ = μ = 1: p₁ = p₀, p_{n+1} = p_n / n, so
    // p_n ∝ 1/(n − 1)! for n ≥ 1 and the total is 1 + e; Lq = Σ (n − 1) p_n = e / (1 + e).
    let mut m = line(1.0, 1, Dist::exponential_rate(1.0));
    if let ElementKind::Server { balking, .. } = &mut m.elements[1].kind {
        *balking = Some(Balking {
            join: BalkJoin::Reciprocal,
            counts_in_service: false,
        });
    }
    let c = m.capacity(DEFAULT_TARGET);
    let f = formula(row(&c, "s"));
    let e = stochcraft_random::math::exp(1.0);
    assert!(near(f.mean_queue, e / (1.0 + e)), "{f:?}");
}

#[test]
fn a_rework_loop_is_solved_and_mmc_holds_in_it() {
    // 1 per minute, a fifth sent back: the server sees 1/0.8 = 1.25 (a Jackson network).
    let m = model(
        vec![
            source("in", Dist::exponential_rate(1.0)),
            server("s", 2, Dist::exponential_rate(1.0)),
            gateway("check"),
            sink("out"),
        ],
        vec![
            link("in", "s"),
            link("s", "check"),
            chance("check", "s", 0.2),
            chance("check", "out", 0.8),
        ],
    );
    let c = m.capacity(DEFAULT_TARGET);
    let s = row(&c, "s");
    assert!(
        near(s.arrival_rate.exact_value().unwrap_or(0.0), 1.25),
        "{s:?}"
    );
    assert_eq!(formula(s).name, "M/M/2");

    // With a constant time it would be M/G/1, but the loop makes arrivals not Poisson.
    let mut m = m;
    m.elements[1] = server("s", 1, Dist::Constant(0.5));
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(reason(row(&c, "s")), &Why::Loop);
}

#[test]
fn a_loop_sending_everything_back_has_no_rate() {
    let m = model(
        vec![
            source("in", Dist::exponential_rate(1.0)),
            el(
                "wait",
                ElementKind::Delay {
                    duration: Dist::Constant(1.0),
                    processing_by: None,
                },
            ),
            gateway("again"),
            sink("out"),
        ],
        vec![
            link("in", "wait"),
            link("wait", "again"),
            chance("again", "wait", 1.0),
            chance("again", "out", 0.0),
        ],
    );
    let c = m.capacity(DEFAULT_TARGET);
    assert!(matches!(
        row(&c, "wait").arrival_rate,
        Value::Unknown(Why::NoWayOut(_))
    ));
}

#[test]
fn routing_by_condition_is_not_guessed() {
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(1.0)),
            gateway("which"),
            server("a", 1, Dist::exponential_rate(2.0)),
            server("b", 1, Dist::exponential_rate(2.0)),
            sink("out"),
        ],
        vec![
            link("in", "which"),
            link("which", "a"),
            link("which", "b"),
            link("a", "out"),
            link("b", "out"),
        ],
    );
    m.connections[1].condition = Some("getProperty('x') > 1".into());
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(
        row(&c, "a").arrival_rate,
        Value::Unknown(Why::ByCondition(id("which")))
    );
    assert_eq!(row(&c, "b").status, Status::Unknown);

    // By probability, each way gets its share, and those without one split what is left.
    m.connections[1].condition = None;
    m.connections[1].probability = Some(0.7);
    let c = m.capacity(DEFAULT_TARGET);
    assert!(near(
        row(&c, "a").arrival_rate.exact_value().unwrap_or(0.0),
        0.7
    ));
    assert!(near(
        row(&c, "b").arrival_rate.exact_value().unwrap_or(0.0),
        0.3
    ));
    // A probability split of Poisson arrivals is Poisson: M/M/1 at both.
    assert_eq!(formula(row(&c, "b")).name, "M/M/1");
}

#[test]
fn copies_joins_and_batches_pass_rates_on() {
    let m = model(
        vec![
            source("in", Dist::exponential_rate(2.0)),
            el("split", ElementKind::ParallelGateway),
            server("a", 1, Dist::exponential_rate(4.0)),
            el("pack", ElementKind::Batch { size: 4 }),
            server("b", 1, Dist::exponential_rate(1.0)),
            el("open", ElementKind::Unbatch),
            server("c", 1, Dist::Constant(0.1)),
            el("join", ElementKind::ParallelGateway),
            server("d", 1, Dist::Constant(0.1)),
            sink("out"),
        ],
        vec![
            link("in", "split"),
            link("split", "a"),
            link("split", "pack"),
            link("pack", "b"),
            link("b", "open"),
            link("open", "c"),
            link("a", "join"),
            link("c", "join"),
            link("join", "d"),
            link("d", "out"),
        ],
    );
    let c = m.capacity(DEFAULT_TARGET);
    let rate = |i: &str| row(&c, i).arrival_rate.exact_value().unwrap_or(f64::NAN);
    assert!(near(rate("a"), 2.0));
    assert!(near(rate("b"), 0.5));
    assert!(near(rate("c"), 2.0));
    assert!(near(rate("d"), 2.0));
    // After a batch, arrivals are not Poisson.
    assert_eq!(reason(row(&c, "c")), &Why::After(id("open")));
}

#[test]
fn a_server_feeding_a_finite_room_is_not_a_formula() {
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(0.5)),
            server("first", 1, Dist::exponential_rate(1.0)),
            server("second", 1, Dist::exponential_rate(1.0)),
            sink("out"),
        ],
        vec![
            link("in", "first"),
            link("first", "second"),
            link("second", "out"),
        ],
    );
    if let ElementKind::Server { queue_capacity, .. } = &mut m.elements[2].kind {
        *queue_capacity = Some(3);
    }
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(reason(row(&c, "second")), &Why::BlocksSender);
    // Nor is the first: the full room can hold up its unit.
    assert_eq!(reason(row(&c, "first")), &Why::HeldUp(id("second")));
}

#[test]
fn a_tandem_of_exponential_servers_is_mm1_twice() {
    // Burke: an M/M/1's departures are Poisson, so the second is M/M/1 too.
    let m = model(
        vec![
            source("in", Dist::exponential_rate(0.5)),
            server("first", 1, Dist::exponential_rate(1.0)),
            server("second", 1, Dist::Constant(1.0)),
            sink("out"),
        ],
        vec![
            link("in", "first"),
            link("first", "second"),
            link("second", "out"),
        ],
    );
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(formula(row(&c, "second")).name, "M/G/1");

    // After a server with constant times, arrivals are not Poisson.
    let mut m = m;
    m.elements[1] = server("first", 1, Dist::Constant(1.0));
    m.elements[2] = server("second", 1, Dist::exponential_rate(1.5));
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(reason(row(&c, "second")), &Why::After(id("first")));
}

#[test]
fn several_units_with_general_times_have_no_formula() {
    let c = line(1.0, 2, Dist::Constant(1.0)).capacity(DEFAULT_TARGET);
    assert_eq!(reason(row(&c, "s")), &Why::SeveralUnitsGeneral);
}

#[test]
fn non_exponential_arrivals_have_no_formula() {
    let mut m = line(1.0, 1, Dist::exponential_rate(2.0));
    m.elements[0] = source("in", Dist::Constant(1.0));
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(reason(row(&c, "s")), &Why::NotPoisson(id("in")));
    // The rate is still known.
    assert_eq!(row(&c, "s").arrival_rate, Value::exact(1.0));
}

#[test]
fn an_overloaded_station_is_unstable_and_caps_what_follows() {
    // 3 per minute into one unit of 1 a minute: 300%, three units at 85% need 4 (3 / 0.85).
    let m = model(
        vec![
            source("in", Dist::exponential_rate(3.0)),
            server("first", 1, Dist::exponential_rate(1.0)),
            server("second", 1, Dist::exponential_rate(2.0)),
            sink("out"),
        ],
        vec![
            link("in", "first"),
            link("first", "second"),
            link("second", "out"),
        ],
    );
    let c = m.capacity(DEFAULT_TARGET);
    let first = row(&c, "first");
    assert_eq!(first.status, Status::Unstable);
    assert_eq!(first.units_for_target, Some(4));
    assert_eq!(reason(first), &Why::Unstable);
    // At most 1 a minute gets through.
    assert_eq!(
        row(&c, "second").arrival_rate,
        Value::Known {
            value: 1.0,
            bound: Bound::AtMost
        }
    );
    assert_eq!(c.bottleneck, Some(id("first")));
}

#[test]
fn breakdowns_leave_utilisation_as_the_results_count_it() {
    // Up 9, repaired in 1: available 90%. λ s = 0.8: utilisation 80%, load 0.8 / 0.9 ≈ 89%.
    let mut m = line(0.8, 1, Dist::exponential_rate(1.0));
    if let ElementKind::Server { breakdown, .. } = &mut m.elements[1].kind {
        *breakdown = Some(Breakdown {
            time_to_failure: Dist::exponential_rate(1.0 / 9.0),
            repair: Dist::exponential_rate(1.0),
            interruptible: true,
            hands_on_while_down: true,
        });
    }
    let c = m.capacity(DEFAULT_TARGET);
    let s = row(&c, "s");
    assert_eq!(s.availability, Some(Value::exact(0.9)));
    assert!(near(s.utilisation.exact_value().unwrap_or(0.0), 0.8));
    assert!(near(s.load.exact_value().unwrap_or(0.0), 0.8 / 0.9));
    assert_eq!(s.status, Status::Congested);
    assert_eq!(s.units_for_target, Some(2));
    assert_eq!(reason(s), &Why::Breakdowns);
}

#[test]
fn times_by_property_are_not_guessed() {
    let mut m = line(1.0, 1, Dist::exponential_rate(2.0));
    if let ElementKind::Server { processing_by, .. } = &mut m.elements[1].kind {
        *processing_by = Some(crate::ProcessingBy {
            property: "type".into(),
            by_value: Vec::new(),
        });
    }
    let c = m.capacity(DEFAULT_TARGET);
    // At least the least mean (here the only one, 0.5), as the overloaded check takes it.
    assert_eq!(
        row(&c, "s").mean_service,
        Value::Known {
            value: 0.5,
            bound: Bound::AtLeast
        }
    );
    assert_eq!(row(&c, "s").status, Status::Unknown);
    assert_eq!(reason(row(&c, "s")), &Why::ByProperty);
}

#[test]
fn a_pool_carries_its_tasks_load() {
    let task = |i: &str, mean: f64, quantity: u32| {
        el(
            i,
            ElementKind::SharedTask {
                service: Dist::exponential_rate(1.0 / mean),
                processing_by: None,
                processing_by_resource: None,
                needs: vec![Requirement::new(vec![Need::Resource {
                    id: id("staff"),
                    quantity,
                }])],
                priority: 0.0,
            },
        )
    };
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(1.0)),
            task("a", 1.0, 1),
            task("b", 0.5, 2),
            sink("out"),
        ],
        vec![link("in", "a"), link("a", "b"), link("b", "out")],
    );
    m.resources = vec![ResourcePool::new(id("staff"), "Staff", 4, Vec::new())];
    let c = m.capacity(DEFAULT_TARGET);
    // 1 × 1 × 1 + 1 × 0.5 × 2 = 2 units' work on 4: 50%.
    let pool = row(&c, "staff");
    assert_eq!(pool.kind, StationKind::Pool);
    assert!(near(pool.utilisation.exact_value().unwrap_or(0.0), 0.5));
    assert_eq!(reason(row(&c, "a")), &Why::SharedPool);
    // Each task shows its own share: b takes 1 × 0.5 × 2 = 1 unit of 4.
    assert!(near(
        row(&c, "b").utilisation.exact_value().unwrap_or(0.0),
        0.25
    ));

    // Alone on its pool, one unit at a time, a task is M/M/c.
    m.elements.remove(2);
    m.connections = vec![link("in", "a"), link("a", "out")];
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(formula(row(&c, "a")).name, "M/M/4");
}

#[test]
fn a_delay_holds_rate_times_duration() {
    let m = model(
        vec![
            source("in", Dist::Constant(0.5)),
            el(
                "cool",
                ElementKind::Delay {
                    duration: Dist::Uniform { min: 2.0, max: 4.0 },
                    processing_by: None,
                },
            ),
            sink("out"),
        ],
        vec![link("in", "cool"), link("cool", "out")],
    );
    let c = m.capacity(DEFAULT_TARGET);
    // 2 a minute for 3 minutes each: 6 inside on average (Little's law).
    assert!(near(formula(row(&c, "cool")).mean_inside, 6.0));
}

#[test]
fn bounds_go_the_right_way() {
    assert_eq!(Bound::AtMost.with(Bound::Exact, false), Some(Bound::AtMost));
    assert_eq!(Bound::AtMost.with(Bound::AtMost, true), None);
    assert_eq!(
        Bound::AtMost.with(Bound::AtLeast, true),
        Some(Bound::AtMost)
    );
    assert_eq!(Bound::Exact.with(Bound::AtMost, true), Some(Bound::AtLeast));
}

#[test]
fn a_server_a_full_room_can_hold_up_has_no_formula() {
    // Weld feeds Paint's two places through a gateway: a full room at Paint blocks Weld, so its
    // M/M/1/K numbers would be wrong, and Weld's departures are not Poisson.
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(0.5)),
            server("weld", 1, Dist::exponential_rate(1.0)),
            gateway("pass"),
            server("paint", 1, Dist::exponential_rate(1.0)),
            sink("out"),
        ],
        vec![
            link("in", "weld"),
            link("weld", "pass"),
            link("pass", "paint"),
            link("paint", "out"),
        ],
    );
    if let ElementKind::Server { queue_capacity, .. } = &mut m.elements[3].kind {
        *queue_capacity = Some(2);
    }
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(reason(row(&c, "weld")), &Why::HeldUp(id("paint")));
    assert_eq!(reason(row(&c, "paint")), &Why::BlocksSender);

    // With a way for those turned away, Paint never holds Weld up.
    m.elements.push(sink("away"));
    m.connections.push(Connection {
        turned_away: true,
        ..link("paint", "away")
    });
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(formula(row(&c, "weld")).name, "M/M/1");
}

#[test]
fn a_delay_after_an_unknown_rate_is_not_called_fine() {
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(1.0)),
            gateway("which"),
            el(
                "wait",
                ElementKind::Delay {
                    duration: Dist::Constant(1.0),
                    processing_by: None,
                },
            ),
            sink("out"),
        ],
        vec![
            link("in", "which"),
            link("which", "wait"),
            link("which", "out"),
            link("wait", "out"),
        ],
    );
    m.connections[1].condition = Some("getProperty('x') > 1".into());
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(row(&c, "wait").status, Status::Unknown);
}

#[test]
fn erlang_a_waits_are_those_of_the_entities_served() {
    // M/M/1+M, λ = μ = θ = 1. From place q in the queue an entity is served with chance
    // Π_{i ≤ q} i/(i + 1) = 1/(q + 1), after Σ_{i ≤ q} 1/(1 + i). States are Poisson(1):
    // p_n = e⁻¹/n!, an arrival finding n ≥ 1 joins at place n. So the served share is
    // Σ_n p_n/(n + 1) = 1 − e⁻¹ and their waits sum to Σ_{n ≥ 1} p_n H'(n)/(n + 1), with
    // H'(n) = Σ_{i=1..n} 1/(1 + i).
    let mut m = line(1.0, 1, Dist::exponential_rate(1.0));
    if let ElementKind::Server { patience, .. } = &mut m.elements[1].kind {
        *patience = Some(Patience {
            time: Dist::exponential_rate(1.0),
            until: PatienceUntil::Start,
        });
    }
    let f = formula(row(&m.capacity(DEFAULT_TARGET), "s")).clone();
    let e1 = stochcraft_random::math::exp(-1.0);
    let (mut served, mut waits, mut pn, mut h) = (e1, 0.0, e1, 0.0);
    for n in 1..60 {
        pn /= f64::from(n);
        h += 1.0 / f64::from(1 + n);
        served += pn / f64::from(n + 1);
        waits += pn * h / f64::from(n + 1);
    }
    assert!(near(f.served_share, 1.0 - e1), "{f:?}");
    assert!(near(f.mean_wait, waits / served), "{f:?}");
}

#[test]
fn the_bottleneck_is_named_only_when_sure() {
    // Paint's load is only at most 86.5% (Weld's room can lose arrivals): it could be above
    // Weld's 61.5%, so no bottleneck is named.
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(1.0 / 6.5)),
            server("weld", 1, Dist::exponential_rate(0.25)),
            server("paint", 1, Dist::exponential_rate(0.2)),
            sink("out"),
        ],
        vec![
            link("in", "weld"),
            link("weld", "paint"),
            link("paint", "out"),
        ],
    );
    if let ElementKind::Server { queue_capacity, .. } = &mut m.elements[1].kind {
        *queue_capacity = Some(3);
    }
    let c = m.capacity(DEFAULT_TARGET);
    assert!(matches!(
        row(&c, "paint").load,
        Value::Known {
            bound: Bound::AtMost,
            ..
        }
    ));
    assert_eq!(c.bottleneck, None);
    // With every load exact, the highest is named.
    m.elements[1] = server("weld", 1, Dist::exponential_rate(0.25));
    assert_eq!(m.capacity(DEFAULT_TARGET).bottleneck, Some(id("paint")));
}

#[test]
fn a_room_turning_entities_away_passes_on_at_most_its_arrivals() {
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(0.5)),
            server("first", 1, Dist::exponential_rate(1.0)),
            gateway("pass"),
            server("second", 1, Dist::exponential_rate(1.0)),
            server("third", 1, Dist::exponential_rate(1.0)),
            sink("out"),
            sink("away"),
        ],
        vec![
            link("in", "first"),
            link("first", "pass"),
            link("pass", "second"),
            link("second", "third"),
            link("third", "out"),
            Connection {
                turned_away: true,
                ..link("second", "away")
            },
        ],
    );
    if let ElementKind::Server { queue_capacity, .. } = &mut m.elements[3].kind {
        *queue_capacity = Some(1);
    }
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(
        row(&c, "third").arrival_rate,
        Value::Known {
            value: 0.5,
            bound: Bound::AtMost
        }
    );
}

#[test]
fn a_transfer_that_keeps_the_sender_holds_it_up() {
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(0.5)),
            server("first", 1, Dist::exponential_rate(1.0)),
            server("second", 1, Dist::exponential_rate(1.0)),
            sink("out"),
        ],
        vec![
            link("in", "first"),
            link("first", "second"),
            link("second", "out"),
        ],
    );
    if let ElementKind::Server { transfer, .. } = &mut m.elements[2].kind {
        *transfer = Some(crate::Transfer::holding(Dist::Constant(0.5)));
    }
    let c = m.capacity(DEFAULT_TARGET);
    let first = row(&c, "first");
    assert_eq!(reason(first), &Why::HeldUp(id("second")));
    // Busy as the results count it is still exact; the unit is occupied at least that long.
    assert!(near(first.utilisation.exact_value().unwrap_or(0.0), 0.5));
    assert!(matches!(
        first.load,
        Value::Known {
            bound: Bound::AtLeast,
            ..
        }
    ));
    assert_eq!(reason(row(&c, "second")), &Why::Transfer);
}

fn shared_task(i: &str, mean: f64) -> Element {
    el(
        i,
        ElementKind::SharedTask {
            service: Dist::exponential_rate(1.0 / mean),
            processing_by: None,
            processing_by_resource: None,
            needs: vec![Requirement::new(vec![Need::Resource {
                id: id("staff"),
                quantity: 1,
            }])],
            priority: 0.0,
        },
    )
}

/// With service times by pool there is no single service time: the task's mean service is at
/// least the least of its own and the entries' (never their average), and no formula is given.
#[test]
fn service_times_by_pool_get_no_formula_and_a_lower_bound() {
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(0.2)),
            shared_task("a", 2.0),
            sink("out"),
        ],
        vec![link("in", "a"), link("a", "out")],
    );
    m.resources = vec![ResourcePool::new(id("staff"), "Staff", 1, Vec::new())];
    // Without entries the task alone on its pool has the M/M/1 formula.
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(formula(row(&c, "a")).name, "M/M/1");
    if let ElementKind::SharedTask {
        processing_by_resource,
        ..
    } = &mut m.elements[1].kind
    {
        *processing_by_resource = Some(crate::ProcessingByResource {
            by_resource: vec![crate::ByResource {
                resource: id("staff"),
                time: Dist::exponential_rate(1.0 / 0.5),
            }],
        });
    }
    let c = m.capacity(DEFAULT_TARGET);
    let a = row(&c, "a");
    assert!(
        matches!(
            a.mean_service,
            Value::Known { value, bound: Bound::AtLeast } if near(value, 0.5)
        ),
        "{:?}",
        a.mean_service
    );
    assert_eq!(reason(a), &Why::ByResource);
    assert_eq!(Why::ByResource.code(), "by_resource");
}

#[test]
fn an_overloaded_pool_bounds_what_follows() {
    // Two a minute on one unit of a minute each: at most one a minute gets through.
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(2.0)),
            shared_task("a", 1.0),
            server("next", 4, Dist::exponential_rate(1.0)),
            sink("out"),
        ],
        vec![link("in", "a"), link("a", "next"), link("next", "out")],
    );
    m.resources = vec![ResourcePool::new(id("staff"), "Staff", 1, Vec::new())];
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(row(&c, "staff").status, Status::Unstable);
    assert_eq!(
        row(&c, "next").arrival_rate,
        Value::Known {
            value: 1.0,
            bound: Bound::AtMost
        }
    );

    // Two tasks each under the pool alone but over it together: what follows is bounded.
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(0.6)),
            shared_task("a", 1.0),
            shared_task("b", 1.0),
            server("next", 4, Dist::exponential_rate(1.0)),
            sink("out"),
        ],
        vec![
            link("in", "a"),
            link("a", "b"),
            link("b", "next"),
            link("next", "out"),
        ],
    );
    m.resources = vec![ResourcePool::new(id("staff"), "Staff", 1, Vec::new())];
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(row(&c, "staff").status, Status::Unstable);
    assert!(matches!(
        row(&c, "next").arrival_rate,
        Value::Known {
            bound: Bound::AtMost,
            ..
        }
    ));
}

#[test]
fn a_delay_by_property_has_no_exact_occupancy() {
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(1.0)),
            el(
                "wait",
                ElementKind::Delay {
                    duration: Dist::Constant(1.0),
                    processing_by: Some(crate::ProcessingBy {
                        property: "type".into(),
                        by_value: vec![crate::ByValue {
                            value: "slow".into(),
                            time: Dist::Constant(10.0),
                        }],
                    }),
                },
            ),
            sink("out"),
        ],
        vec![link("in", "wait"), link("wait", "out")],
    );
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(reason(row(&c, "wait")), &Why::ByProperty);
    // And a server with such times gets no number of units: all could be slow.
    m.elements[1] = server("wait", 1, Dist::Constant(1.0));
    if let ElementKind::Server { processing_by, .. } = &mut m.elements[1].kind {
        *processing_by = Some(crate::ProcessingBy {
            property: "type".into(),
            by_value: vec![crate::ByValue {
                value: "slow".into(),
                time: Dist::Constant(10.0),
            }],
        });
    }
    let c = m.capacity(DEFAULT_TARGET);
    let s = row(&c, "wait");
    assert_eq!(s.status, Status::Unstable);
    assert_eq!(s.units_for_target, None);
}

#[test]
fn availability_is_exact_only_for_failures_that_interrupt() {
    let failing = |interruptible: bool, time_to_failure: Dist| {
        let mut m = line(0.5, 1, Dist::exponential_rate(1.0));
        if let ElementKind::Server { breakdown, .. } = &mut m.elements[1].kind {
            *breakdown = Some(Breakdown {
                time_to_failure,
                repair: Dist::Constant(1.0),
                interruptible,
                hands_on_while_down: true,
            });
        }
        m.capacity(DEFAULT_TARGET)
    };
    // A failure waiting for the job to end keeps the unit up longer: at least 90%, so the load
    // is at most 0.5 / 0.9.
    let c = failing(false, Dist::Constant(9.0));
    let s = row(&c, "s");
    assert_eq!(
        s.availability,
        Some(Value::Known {
            value: 0.9,
            bound: Bound::AtLeast
        })
    );
    assert!(matches!(
        s.load,
        Value::Known {
            bound: Bound::AtMost,
            ..
        }
    ));
    // A time to failure with no usable mean: the load is not known, not taken at 100% up.
    let c = failing(
        true,
        Dist::Normal {
            mean: 9.0,
            std_dev: 1.0,
        },
    );
    let s = row(&c, "s");
    assert_eq!(s.availability, Some(Value::Unknown(Why::NoMean)));
    assert_eq!(s.load, Value::Unknown(Why::NoMean));
    assert_eq!(s.status, Status::Unknown);
}

#[test]
fn balking_that_still_takes_in_too_much_is_unstable() {
    // Three a minute, half join whatever the queue: 150% of one unit.
    let mut m = line(3.0, 1, Dist::exponential_rate(1.0));
    if let ElementKind::Server { balking, .. } = &mut m.elements[1].kind {
        *balking = Some(Balking {
            join: BalkJoin::Table(vec![crate::BalkRow {
                waiting: 0,
                probability: 0.5,
            }]),
            counts_in_service: false,
        });
    }
    let c = m.capacity(DEFAULT_TARGET);
    let s = row(&c, "s");
    assert_eq!(s.status, Status::Unstable);
    assert_eq!(reason(s), &Why::Unstable);
}

#[test]
fn a_later_balking_row_that_takes_in_more_is_not_cut_off() {
    // Nearly all balk from 1 inside, all join from 30, none from 100: the queue sits near 100,
    // far past where the weights first fall away.
    let mut m = line(10.0, 1, Dist::exponential_rate(1.0));
    if let ElementKind::Server { balking, .. } = &mut m.elements[1].kind {
        *balking = Some(Balking {
            join: BalkJoin::Table(vec![
                crate::BalkRow {
                    waiting: 1,
                    probability: 0.001,
                },
                crate::BalkRow {
                    waiting: 30,
                    probability: 1.0,
                },
                crate::BalkRow {
                    waiting: 100,
                    probability: 0.0,
                },
            ]),
            counts_in_service: true,
        });
    }
    let c = m.capacity(DEFAULT_TARGET);
    let f = formula(row(&c, "s"));
    assert!(f.mean_queue > 90.0, "{f:?}");
}

#[test]
fn the_overloaded_warning_follows_the_table() {
    let overloaded = |m: &Model| -> Vec<String> {
        m.validate()
            .into_iter()
            .filter(|d| matches!(d.problem, crate::Problem::Overloaded { .. }))
            .filter_map(|d| d.at.map(|a| a.to_string()))
            .collect()
    };
    // Through a gateway: 0.7 of 2 a minute onto one unit of a minute each.
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(2.0)),
            gateway("which"),
            server("a", 1, Dist::exponential_rate(1.0)),
            server("b", 1, Dist::exponential_rate(1.0)),
            sink("out"),
        ],
        vec![
            link("in", "which"),
            chance("which", "a", 0.7),
            chance("which", "b", 0.3),
            link("a", "out"),
            link("b", "out"),
        ],
    );
    assert_eq!(overloaded(&m), vec!["a".to_owned()]);
    // A pool its tasks overload together.
    m.elements[2] = shared_task("a", 1.0);
    m.elements[3] = shared_task("b", 1.0);
    m.resources = vec![ResourcePool::new(id("staff"), "Staff", 1, Vec::new())];
    assert_eq!(overloaded(&m), vec!["staff".to_owned()]);
    // A source with a limit stops: no warning.
    if let ElementKind::Source { limit, .. } = &mut m.elements[0].kind {
        *limit = Some(10);
    }
    assert_eq!(overloaded(&m), Vec::<String>::new());
}

#[test]
fn a_station_of_uncertain_capacity_bounds_what_follows() {
    // s → next: with failures of exact availability, or at least an availability that
    // certainly carries what s gets, all of it reaches next; with an availability not known, s
    // may be over 100%, so next gets at most that.
    let failing = |interruptible: bool, time_to_failure: Dist| {
        let mut m = model(
            vec![
                source("in", Dist::exponential_rate(0.5)),
                server("s", 1, Dist::exponential_rate(1.0)),
                server("next", 4, Dist::exponential_rate(1.0)),
                sink("out"),
            ],
            vec![link("in", "s"), link("s", "next"), link("next", "out")],
        );
        if let ElementKind::Server { breakdown, .. } = &mut m.elements[1].kind {
            *breakdown = Some(Breakdown {
                time_to_failure,
                repair: Dist::Constant(1.0),
                interruptible,
                hands_on_while_down: true,
            });
        }
        row(&m.capacity(DEFAULT_TARGET), "next")
            .arrival_rate
            .clone()
    };
    assert_eq!(failing(true, Dist::Constant(9.0)), Value::exact(0.5));
    let at_most = Value::Known {
        value: 0.5,
        bound: Bound::AtMost,
    };
    assert_eq!(failing(false, Dist::Constant(9.0)), Value::exact(0.5));
    // At least 1/3 up: 0.5 a minute may be more than s passes on.
    assert_eq!(failing(false, Dist::Constant(0.5)), at_most);
    let no_mean = Dist::Normal {
        mean: 9.0,
        std_dev: 1.0,
    };
    assert_eq!(failing(true, no_mean), at_most);
}

#[test]
fn a_task_of_unknown_rate_may_overload_its_pool() {
    // a's load alone is 0.5 of the one unit; b, reached by a condition, adds an unknown load,
    // so a may get less than it is sent and next gets at most that.
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(0.5)),
            shared_task("a", 1.0),
            server("next", 4, Dist::exponential_rate(1.0)),
            source("other", Dist::exponential_rate(1.0)),
            gateway("which"),
            shared_task("b", 1.0),
            sink("out"),
        ],
        vec![
            link("in", "a"),
            link("a", "next"),
            link("next", "out"),
            link("other", "which"),
            link("which", "b"),
            link("which", "out"),
            link("b", "out"),
        ],
    );
    m.resources = vec![ResourcePool::new(id("staff"), "Staff", 1, Vec::new())];
    m.connections[4].condition = Some("getProperty('x') > 1".into());
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(
        row(&c, "next").arrival_rate,
        Value::Known {
            value: 0.5,
            bound: Bound::AtMost
        }
    );
    // By a probability b's load is known (0.5 + 0.2 of the unit): next gets all a gets.
    m.connections[4].condition = None;
    m.connections[4].probability = Some(0.2);
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(row(&c, "next").arrival_rate, Value::exact(0.5));
}

#[test]
fn a_station_of_uncertain_capacity_on_its_own_loop_is_judged_on_what_it_is_offered() {
    // One a minute in, half sent back: two a minute offered to s, whose times by property are
    // at least 0.6. Its load is at least 1.2: unstable, and what follows it at most.
    let back = chance("check", "s", 0.5);
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(1.0)),
            server("s", 1, Dist::exponential_rate(1.0 / 0.6)),
            gateway("check"),
            sink("out"),
        ],
        vec![
            link("in", "s"),
            link("s", "check"),
            back,
            chance("check", "out", 0.5),
        ],
    );
    if let ElementKind::Server { processing_by, .. } = &mut m.elements[1].kind {
        *processing_by = Some(crate::ProcessingBy {
            property: "type".into(),
            by_value: Vec::new(),
        });
    }
    let c = m.capacity(DEFAULT_TARGET);
    let s = row(&c, "s");
    assert_eq!(
        s.arrival_rate.exact_value().map(|r| near(r, 2.0)),
        Some(true)
    );
    assert_eq!(s.status, Status::Unstable);
    assert!(
        m.validate()
            .iter()
            .any(|d| matches!(d.problem, crate::Problem::Overloaded { .. }))
    );
}

fn conveyor(i: &str, length: f64, speed: f64, item_length: f64, gap: f64) -> Element {
    el(
        i,
        ElementKind::Conveyor {
            length,
            speed,
            accumulating: true,
            item_length,
            gap,
            shape: Default::default(),
        },
    )
}

/// Source → conveyor of 10 long at 0.5 per time unit with 1 per item → server → sink.
fn belt_line(arrival_rate: f64) -> Model {
    model(
        vec![
            source("in", Dist::exponential_rate(arrival_rate)),
            conveyor("belt", 10.0, 0.5, 1.0, 0.0),
            server("s", 1, Dist::Constant(0.1)),
            sink("out"),
        ],
        vec![link("in", "belt"), link("belt", "s"), link("s", "out")],
    )
}

#[test]
fn a_conveyor_carries_speed_over_pitch_and_its_time_is_length_over_speed() {
    let c = belt_line(0.4).capacity(DEFAULT_TARGET);
    let b = row(&c, "belt");
    assert_eq!(b.kind, StationKind::Conveyor);
    assert_eq!(b.units, Some(1));
    // Pitch 1 over speed 0.5: 2 per item, so at most 0.5 items per time unit.
    assert_eq!(b.mean_service.exact_value(), Some(2.0));
    assert_eq!(
        b.arrival_rate.exact_value().map(|r| near(r, 0.4)),
        Some(true)
    );
    assert_eq!(b.load.exact_value().map(|l| near(l, 0.8)), Some(true));
    assert_eq!(b.utilisation, b.load);
    assert_eq!(b.status, Status::Fine);
    // Nothing after it can hold it up: 10 / 0.5 exactly, and no queueing formula.
    assert_eq!(b.time_on, Some(Value::exact(20.0)));
    assert_eq!(reason(b), &Why::ConveyorTime);
    assert_eq!(b.units_for_target, None);
    assert!(
        belt_line(0.4)
            .validate()
            .iter()
            .all(|d| !matches!(d.problem, crate::Problem::Overloaded { .. }))
    );
}

#[test]
fn a_conveyor_fed_faster_than_it_carries_is_unstable_and_bounds_what_follows() {
    let m = belt_line(0.6);
    let c = m.capacity(DEFAULT_TARGET);
    let b = row(&c, "belt");
    assert_eq!(b.load.exact_value().map(|l| near(l, 1.2)), Some(true));
    assert_eq!(b.status, Status::Unstable);
    assert_eq!(c.bottleneck, Some(id("belt")));
    match row(&c, "s").arrival_rate {
        Value::Known {
            value,
            bound: Bound::AtMost,
        } => assert!(value <= 0.5 + 1e-12, "{value}"),
        ref other => panic!("not at most 0.5: {other:?}"),
    }
    assert!(
        m.validate()
            .iter()
            .any(|d| matches!(d.problem, crate::Problem::Overloaded { .. }))
    );
}

#[test]
fn a_conveyors_time_is_at_least_its_length_over_its_speed_when_what_follows_can_hold_it_up() {
    let mut m = belt_line(0.4);
    if let ElementKind::Server { queue_capacity, .. } = &mut m.elements[2].kind {
        *queue_capacity = Some(2);
    }
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(
        row(&c, "belt").time_on,
        Some(Value::Known {
            value: 20.0,
            bound: Bound::AtLeast
        })
    );
}

/// Source → transport → sink, one operator walking at 1 empty and 2 loaded, the pickup (the
/// source) and the drop-off (the sink) 10 apart on the canvas, load 1 and unload 1.
fn carry_line(arrival_rate: f64) -> Model {
    let shape = |x| crate::Bounds {
        x,
        y: 0.0,
        width: 10.0,
        height: 10.0,
    };
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(arrival_rate)),
            el(
                "carry",
                ElementKind::Transport {
                    pool: id("walker"),
                    load: Dist::Constant(1.0),
                    unload: Dist::Constant(1.0),
                },
            ),
            sink("out"),
        ],
        vec![link("in", "carry"), link("carry", "out")],
    );
    let mut pool = ResourcePool::new(id("walker"), "Walker", 1, Vec::new());
    pool.mobility = Some(crate::Mobility {
        speed_empty: 1.0,
        speed_loaded: 2.0,
        home: id("in"),
        after_job: crate::AfterJob::Stay,
    });
    m.resources = vec![pool];
    m.layout.insert(id("in"), shape(0.0));
    m.layout.insert(id("out"), shape(10.0));
    m
}

#[test]
fn a_transports_work_is_at_least_load_unload_and_the_loaded_trip() {
    // 1 + 1 + 10 / 2 = 7 per item at 0.2 items per time unit: at least 1.4, so unstable.
    let m = carry_line(0.2);
    assert_eq!(m.distance(&id("in"), &id("out")), Some(10.0));
    let c = m.capacity(DEFAULT_TARGET);
    let t = row(&c, "carry");
    assert_eq!(t.kind, StationKind::Transport);
    assert_eq!(
        t.mean_service,
        Value::Known {
            value: 7.0,
            bound: Bound::AtLeast
        }
    );
    assert_eq!(reason(t), &Why::TransportTrips);
    let p = row(&c, "walker");
    assert_eq!(p.kind, StationKind::Pool);
    match p.load {
        Value::Known {
            value,
            bound: Bound::AtLeast,
        } => assert!(near(value, 1.4), "{value}"),
        ref other => panic!("not at least 1.4: {other:?}"),
    }
    assert_eq!(p.status, Status::Unstable);
    assert_eq!(t.status, Status::Unstable);
}

#[test]
fn a_transport_below_100_percent_is_not_known_before_a_run() {
    // At 0.05 items per time unit the load is at least 0.35; the empty trips could add more.
    let c = carry_line(0.05).capacity(DEFAULT_TARGET);
    let p = row(&c, "walker");
    match p.load {
        Value::Known {
            value,
            bound: Bound::AtLeast,
        } => assert!(near(value, 0.35), "{value}"),
        ref other => panic!("not at least 0.35: {other:?}"),
    }
    assert_eq!(p.status, Status::Unknown);
    assert_eq!(row(&c, "carry").status, Status::Unknown);
}

/// A job shop's job needs a machine and the worker at once: no formula for the task (it holds
/// units of two pools), but each pool carries its load: the worker the whole of it, the two
/// machines half each.
#[test]
fn a_task_holding_several_pools_gets_no_formula_but_loads_each_pool() {
    let need = |pool: &str| {
        Requirement::new(vec![Need::Resource {
            id: id(pool),
            quantity: 1,
        }])
    };
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(1.0)),
            el(
                "job",
                ElementKind::SharedTask {
                    service: Dist::exponential_rate(1.0 / 0.75),
                    processing_by: None,
                    processing_by_resource: None,
                    needs: vec![need("machines"), need("worker")],
                    priority: 0.0,
                },
            ),
            sink("out"),
        ],
        vec![link("in", "job"), link("job", "out")],
    );
    m.resources = vec![
        ResourcePool::new(id("machines"), "Machines", 2, Vec::new()),
        ResourcePool::new(id("worker"), "Worker", 1, Vec::new()),
    ];
    let c = m.capacity(DEFAULT_TARGET);
    let job = row(&c, "job");
    assert_eq!(reason(job), &Why::SeveralPools);
    assert_eq!(job.utilisation, Value::Unknown(Why::SeveralPools));
    assert_eq!(job.units, None);
    let load = |pool: &str| row(&c, pool).utilisation.exact_value();
    assert!(
        load("worker").is_some_and(|u| near(u, 0.75)),
        "{:?}",
        load("worker")
    );
    assert!(
        load("machines").is_some_and(|u| near(u, 0.375)),
        "{:?}",
        load("machines")
    );
    // The task is as congested as its busiest pool: the worker, at 75%, is below the target.
    assert_eq!(job.status, Status::Fine);
    assert_eq!(Why::SeveralPools.code(), "several_pools");
}

/// A pool serves whole jobs only: with 3 units in each of two pools and a job needing 2 of each,
/// one job at a time, not 1.5. Offered 1.2 jobs an hour with a mean of 1 hour, the task passes on
/// at most 1, and what follows it gets at most that.
#[test]
fn a_pool_serves_whole_jobs_only() {
    let need = |pool: &str| {
        Requirement::new(vec![Need::Resource {
            id: id(pool),
            quantity: 2,
        }])
    };
    let mut m = model(
        vec![
            source("in", Dist::exponential_rate(1.2)),
            el(
                "job",
                ElementKind::SharedTask {
                    service: Dist::exponential_rate(1.0),
                    processing_by: None,
                    processing_by_resource: None,
                    needs: vec![need("a"), need("b")],
                    priority: 0.0,
                },
            ),
            server("next", 1, Dist::exponential_rate(10.0)),
            sink("out"),
        ],
        vec![link("in", "job"), link("job", "next"), link("next", "out")],
    );
    m.resources = vec![
        ResourcePool::new(id("a"), "A", 3, Vec::new()),
        ResourcePool::new(id("b"), "B", 3, Vec::new()),
    ];
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(
        row(&c, "next").arrival_rate,
        Value::Known {
            value: 1.0,
            bound: Bound::AtMost
        }
    );
    // One job of one unit each: unchanged, 3 at a time.
    if let ElementKind::SharedTask { needs, .. } = &mut m.elements[1].kind {
        for r in needs.iter_mut() {
            if let Some(Need::Resource { quantity, .. }) = r.alternatives.first_mut() {
                *quantity = 1;
            }
        }
    }
    let c = m.capacity(DEFAULT_TARGET);
    assert_eq!(row(&c, "next").arrival_rate, Value::exact(1.2));
}
