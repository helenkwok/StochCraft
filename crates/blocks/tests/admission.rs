//! Waiting rooms whose size changes over time, and a way out for those turned away (ROADMAP
//! M2.10): a server's waiting room by timetable or per working server ([`QueueLimit`]), and the
//! connection entities finding no room take at once ([`Connection::turned_away`]).
//!
//! An entity is admitted when those waiting (in the queue or in transfer here) less the idle
//! working units are fewer than the limit now: it gets in when an idle working unit can take it
//! at once, or when fewer than the limit wait. Those already waiting stay when the limit drops.
//!
//! The exact cases are worked by hand. The statistical ones compare two models with the exact
//! results of their continuous-time Markov chains (solved separately, to ten digits):
//!
//! - A, long run, per working server: c = 3 units, arrivals at rate 1.8, exponential service of
//!   mean 1, a health check on each working unit at rate 1 that takes it out when more than 1.5
//!   wait per working unit, restarts of mean 2, retrying the job under way, no patience; 2
//!   waiting per working unit. Those turned away wait (exponential, mean 3) and try again with
//!   probability 0.9, else give up for good. The state is (n at the server, k working, m
//!   waiting to try again); an arrival or a retry is admitted when max(n − k, 0) < 2k.
//! - B, transient, by timetable: c = 2 units, arrivals at rate 1.5, exponential service of mean
//!   1, starting empty; 1 waiting place over [0, 50), 6 over [50, 100), run to 100; those
//!   turned away are lost. The chain is the birth-death chain on the number inside, cut at
//!   2 + 1 and then 2 + 6, solved from time 0.
//!
//! [`QueueLimit`]: stochcraft_model::QueueLimit
//! [`Connection::turned_away`]: stochcraft_model::Connection::turned_away
#![allow(clippy::panic)]

use stochcraft_blocks::{
    Breakdown, Builder, ElementReport, Event, ExperimentSummary, HealthCheck, Metric, OnShiftEnd,
    OnTakeout, PlaceWhileOut, Report, compile, summarise,
};
use stochcraft_model::{DateTime, Diagnostic, Dist, Experiment, Problem, Severity};
use stochcraft_random::Rng;

fn at(h: u32, m: u32, s: u32) -> DateTime {
    DateTime::new(2016, 1, 1, h, m, s).unwrap_or(DateTime(0))
}

/// Once, from `from` to `to` (hours, minutes, seconds of 1 January 2016).
fn once(from: (u32, u32, u32), to: (u32, u32, u32)) -> Event {
    Event {
        start: at(from.0, from.1, from.2),
        end: at(to.0, to.1, to.2),
        rule: None,
    }
}

fn experiment(warmup: f64, duration: f64, replications: u32) -> Experiment {
    Experiment {
        warmup,
        duration,
        replications,
        start: Some(at(0, 0, 0)),
        ..Experiment::default()
    }
}

fn element<'a>(r: &'a Report, id: &str) -> &'a ElementReport {
    r.element(id).unwrap_or_else(|| panic!("no {id} in {r:?}"))
}

fn errors(b: &Builder) -> Vec<Diagnostic> {
    b.build(experiment(0.0, 10.0, 1))
        .validate()
        .into_iter()
        .filter(|d| d.severity == Severity::Error)
        .collect()
}

fn run_once(b: &Builder, duration: f64) -> Report {
    assert_eq!(errors(b), vec![]);
    let compiled =
        compile(&b.build(experiment(0.0, duration, 1))).unwrap_or_else(|d| panic!("{d:?}"));
    compiled.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

/// `replications` runs from seeds `seed`, `seed + 1`, and so on.
fn run(b: &Builder, warmup: f64, duration: f64, replications: u64, seed: u64) -> ExperimentSummary {
    let model = b.build(experiment(warmup, duration, replications as u32));
    let compiled = compile(&model).unwrap_or_else(|d| panic!("{d:?}"));
    let reports: Vec<Report> = (0..replications)
        .map(|r| {
            compiled
                .run(Rng::new(seed + r))
                .unwrap_or_else(|e| panic!("{e}"))
        })
        .collect();
    summarise(&reports)
}

/// Whether the 95% interval of `metric` at `element` over the replications holds `theory`
/// (`scale` times the metric: per time unit, or units for a share); each is printed.
fn covers(s: &ExperimentSummary, element: &str, metric: Metric, scale: f64, theory: f64) -> bool {
    let got = s.get(element, metric);
    let shown = got.map(|g| (g.mean * scale, g.half_width * scale));
    let ok = got.is_some_and(|g| g.contains(theory / scale));
    eprintln!(
        "{element} {} (x{scale}): {shown:?} (theory {theory}) {}",
        metric.name(),
        if ok { "inside" } else { "OUTSIDE" }
    );
    ok
}

/// What entered either left, gave up or is still inside, at every element; a source's arrivals
/// turned away where it sends count as lost there (a server's never entered).
fn balanced(r: &Report) {
    for e in &r.elements {
        let lost = if e.kind == "source" {
            e.lost.unwrap_or(0)
        } else {
            0
        };
        let out = e.left + lost + e.abandoned.unwrap_or(0) + e.work_in_progress.unwrap_or(0);
        assert_eq!(e.entered, out, "{} does not balance: {e:?}", e.id);
    }
}

/// One unit, service 10, four arrivals at 0, a waiting room of 1 per working unit. The first
/// gets the unit, the second waits (one fewer waiting than the limit, once the idle unit takes
/// the first, whichever comes first), the third and fourth find one waiting with the unit busy,
/// at the limit: lost.
#[test]
fn an_entity_is_admitted_below_the_limit_and_turned_away_at_it() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::Constant(0.0), Some(4));
    let desk = b.server("desk", 1, Dist::Constant(10.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.queue_per_working_server(desk, 1);
    let r = run_once(&b, 15.0);
    let d = element(&r, "desk");
    assert_eq!(d.entered, 2, "{d:?}");
    assert_eq!(d.lost, Some(2), "{d:?}");
    assert_eq!(d.loss_probability, Some(0.5), "{d:?}");
    assert_eq!(element(&r, "arrivals").lost, Some(2));
    assert_eq!(d.left, 1, "{d:?}");
    // No unit, no room: with a limit of 0, only an idle working unit lets one in.
    b.queue_per_working_server(desk, 0);
    let r = run_once(&b, 15.0);
    assert_eq!(element(&r, "desk").entered, 1);
    assert_eq!(element(&r, "desk").lost, Some(3));
}

/// Two units, service 100, a job every minute from 1, two waiting per working unit, and one
/// unit on shift from 5.5 (finishing its job first, but off shift for the next entity at
/// once). Jobs 1 and 2 get the units, 3 to 5 wait (three, below the limit of four with two
/// working). At 5.5 one unit goes off: the limit is now 2, but the three waiting stay; jobs 6
/// to 9 are turned away. Run to 9.5: three still waiting, none removed.
#[test]
fn those_waiting_stay_when_the_limit_drops() {
    let mut b = Builder::new();
    b.calendar("low", vec![once((0, 5, 30), (2, 0, 0))]);
    let arrivals = b.source("arrivals", Dist::Constant(1.0), Some(9));
    let desk = b.server("desk", 2, Dist::Constant(100.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.server_shifts(desk, 2, &[("low", 1)], OnShiftEnd::Finish);
    b.queue_per_working_server(desk, 2);
    let r = run_once(&b, 9.5);
    let d = element(&r, "desk");
    assert_eq!(d.entered, 5, "{d:?}");
    assert_eq!(d.lost, Some(4), "{d:?}");
    assert_eq!(d.work_in_progress, Some(5), "{d:?}");
    // Three waiting from 5 to 9.5, two from 4 to 5, one from 3 to 4.
    let queue = (1.0 + 2.0 + 3.0 * 4.5) / 9.5;
    assert!(
        d.mean_queue_length
            .is_some_and(|q| (q - queue).abs() < 1e-12),
        "{d:?}"
    );
    balanced(&r);
}

/// The same by timetable: one unit, service 100, a job every minute from 1, at most 4 waiting
/// outside the quiet time, 1 within it, from 4.5 to 7.5. Job 1 gets the unit, 2 to 4 wait;
/// 5 to 7 find three waiting with a limit of 1: turned away, the three stay. From 7.5 the limit
/// is 4 again, so job 8 waits.
#[test]
fn a_timetable_sets_the_limit_and_those_waiting_stay() {
    let mut b = Builder::new();
    b.calendar("quiet", vec![once((0, 4, 30), (0, 7, 30))]);
    let arrivals = b.source("arrivals", Dist::Constant(1.0), Some(8));
    let desk = b.server("desk", 1, Dist::Constant(100.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.queue_by_shift(desk, 4, &[("quiet", 1)]);
    let r = run_once(&b, 8.5);
    let d = element(&r, "desk");
    assert_eq!(d.entered, 5, "{d:?}");
    assert_eq!(d.lost, Some(3), "{d:?}");
    // A limit rising to above those waiting does not empty the queue: they were never sent
    // away.
    assert_eq!(d.work_in_progress, Some(5), "{d:?}");
    balanced(&r);
}

/// A sender blocked by a full room by timetable gets in once the limit rises: a delay of 0
/// feeds one unit (service 100) with no room to wait until 4.5, then 1. Job 1 (at 1) gets the
/// unit; job 2 (at 2) waits in the delay, blocked, until 4.5.
#[test]
fn a_blocked_sender_gets_in_when_the_timetable_opens_the_room() {
    let mut b = Builder::new();
    b.calendar("open", vec![once((0, 4, 30), (1, 0, 0))]);
    let arrivals = b.source("arrivals", Dist::Constant(1.0), Some(2));
    let hall = b.delay("hall", Dist::Constant(0.0));
    let desk = b.server("desk", 1, Dist::Constant(100.0));
    let exit = b.sink("exit");
    b.connect(arrivals, hall);
    b.connect(hall, desk);
    b.connect(desk, exit);
    b.queue_by_shift(desk, 0, &[("open", 1)]);
    let r = run_once(&b, 10.0);
    let h = element(&r, "hall");
    assert_eq!(h.left, 2, "{h:?}");
    assert!(
        h.mean_blocked_time
            .is_some_and(|t| (t - 2.5 / 2.0).abs() < 1e-12),
        "{h:?}"
    );
    assert_eq!(element(&r, "desk").lost, Some(0));
}

/// Per working server, a failure lowers the limit and a repair raises it: two units, service
/// 100, three waiting per working unit, a job every minute from 1. Both units fail 0.5 after
/// each repair, repairs taking 3.2: down from 0.5 to 3.7, so jobs 1 to 3 find none working and
/// are turned away; job 4 finds both up and idle and gets one; both fail again at 4.2 (the job
/// stops, still in service, on its unit), so job 5 is turned away.
#[test]
fn per_working_server_follows_the_units_working() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::Constant(1.0), Some(5));
    let desk = b.server("desk", 2, Dist::Constant(100.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.queue_per_working_server(desk, 3);
    b.breakdown(
        desk,
        Breakdown {
            time_to_failure: Dist::Constant(0.5),
            repair: Dist::Constant(3.2),
            interruptible: true,
            hands_on_while_down: true,
        },
    );
    let r = run_once(&b, 5.5);
    let d = element(&r, "desk");
    assert_eq!(d.lost, Some(4), "{d:?}");
    assert_eq!(d.entered, 1, "{d:?}");
}

/// A server taken out by a health check lowers the limit per working server; the place-in-room
/// rule changes nothing (no places are kept). One unit taken out at 2 (anyone waiting fails the
/// check), restart 10: jobs from 3 on find no unit working and are turned away.
#[test]
fn a_takeout_lowers_the_limit_and_the_place_rule_changes_nothing() {
    let reports: Vec<Report> = [PlaceWhileOut::Reserved, PlaceWhileOut::Open]
        .into_iter()
        .map(|place| {
            let mut b = Builder::new();
            let arrivals = b.source("arrivals", Dist::Constant(1.0), Some(6));
            let desk = b.server("desk", 1, Dist::Constant(100.0));
            let exit = b.sink("exit");
            b.connect(arrivals, desk);
            b.connect(desk, exit);
            b.queue_per_working_server(desk, 5);
            b.health_check(
                desk,
                HealthCheck {
                    interval: Dist::Constant(2.5),
                    limit: 0.0,
                    restart: Dist::Constant(10.0),
                    keep: None,
                    on_takeout: OnTakeout::Retry,
                    place_while_out: place,
                },
            );
            run_once(&b, 6.5)
        })
        .collect();
    for r in &reports {
        let d = element(r, "desk");
        assert_eq!(d.taken_out, Some(1), "{d:?}");
        // Jobs 1 and 2 got in (2 waiting at 2.5); 3 to 6 found no unit working.
        assert_eq!(d.entered, 2, "{d:?}");
        assert_eq!(d.lost, Some(4), "{d:?}");
    }
    assert_eq!(
        format!("{:?}", reports.first()),
        format!("{:?}", reports.get(1))
    );
}

/// One unit, service 10, no waiting room (fixed, 0), arrivals at 1, 2 and 3: the second and
/// third find no room and take the way for those turned away at once, to a sink of their own.
/// The source counts them as lost (they never got in where it sends), and the server too.
#[test]
fn the_turned_away_way_takes_arrivals_from_a_source() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::Constant(1.0), Some(3));
    let desk = b.buffered_server("desk", 1, Dist::Constant(10.0), 0);
    let exit = b.sink("exit");
    let away = b.sink("away");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.turned_away(desk, away);
    let r = run_once(&b, 5.0);
    let d = element(&r, "desk");
    assert_eq!(d.entered, 1, "{d:?}");
    assert_eq!(d.lost, Some(2), "{d:?}");
    assert_eq!(element(&r, "arrivals").lost, Some(2));
    let a = element(&r, "away");
    assert_eq!(a.left, 2, "{a:?}");
    assert_eq!(a.mean_time_in_system, Some(0.0), "{a:?}");
}

/// A gateway feeding a full room with a way for those turned away does not wait: the
/// entities take that way at once. Without the way, the gateway holds the second until the
/// first is done at 11.
#[test]
fn the_turned_away_way_takes_entities_from_a_gateway_which_never_waits() {
    let build = |way: bool| {
        let mut b = Builder::new();
        let arrivals = b.source("arrivals", Dist::Constant(1.0), Some(3));
        let door = b.exclusive("door");
        let desk = b.buffered_server("desk", 1, Dist::Constant(10.0), 0);
        let exit = b.sink("exit");
        b.connect(arrivals, door);
        b.connect(door, desk);
        b.connect(desk, exit);
        if way {
            let away = b.sink("away");
            b.turned_away(desk, away);
        }
        b
    };
    let r = run_once(&build(true), 5.0);
    let d = element(&r, "desk");
    assert_eq!(d.lost, Some(2), "{d:?}");
    let a = element(&r, "away");
    assert_eq!(a.left, 2, "{a:?}");
    assert_eq!(a.mean_time_in_system, Some(0.0), "{a:?}");
    assert_eq!(element(&r, "door").left, 3);
    // Without the way, the second waits at the gateway (counted as gone its way), the third
    // behind it.
    let r = run_once(&build(false), 5.0);
    assert_eq!(element(&r, "desk").lost, Some(0));
    assert_eq!(element(&r, "desk").entered, 1);
    assert_eq!(element(&r, "door").left, 2);
}

/// A server feeding a full room with a way for those turned away is never blocked: its
/// finished entity takes that way at once.
#[test]
fn the_turned_away_way_takes_entities_from_a_server_which_is_never_blocked() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::Constant(1.0), Some(3));
    let first = b.server("first", 1, Dist::Constant(0.5));
    let desk = b.server("desk", 1, Dist::Constant(10.0));
    let exit = b.sink("exit");
    let away = b.sink("away");
    b.connect(arrivals, first);
    b.connect(first, desk);
    b.connect(desk, exit);
    b.queue_per_working_server(desk, 0);
    b.turned_away(desk, away);
    let r = run_once(&b, 5.0);
    let f = element(&r, "first");
    assert_eq!(f.left, 3, "{f:?}");
    assert!(f.mean_blocked_time.is_none_or(|t| t == 0.0), "{f:?}");
    assert_eq!(element(&r, "desk").lost, Some(2));
    assert_eq!(element(&r, "away").left, 2);
}

/// At most one way for those turned away, only out of a task with a finite waiting room; no
/// warning of a gateway feeding a finite room that turns away; a limit with no number is
/// ignored with a warning.
#[test]
fn turned_away_ways_are_checked() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(1.0), None);
    let door = b.exclusive("door");
    let desk = b.buffered_server("desk", 1, Dist::exponential_rate(2.0), 2);
    let exit = b.sink("exit");
    let away = b.sink("away");
    b.connect(arrivals, door);
    b.connect(door, desk);
    b.connect(desk, exit);
    let model = b.build(experiment(0.0, 10.0, 1));
    assert!(
        model
            .validate()
            .iter()
            .any(|d| matches!(d.problem, Problem::BufferBehindGateway { .. }))
    );
    b.turned_away(desk, away);
    let model = b.build(experiment(0.0, 10.0, 1));
    let diagnostics = model.validate();
    assert!(
        !diagnostics
            .iter()
            .any(|d| matches!(d.problem, Problem::BufferBehindGateway { .. })),
        "{diagnostics:?}"
    );
    assert_eq!(errors(&b), vec![]);
    b.turned_away(desk, exit);
    assert!(
        errors(&b)
            .iter()
            .any(|d| matches!(d.problem, Problem::SeveralTurnedAwayWays { count: 2 }))
    );
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(1.0), None);
    let desk = b.server("desk", 1, Dist::exponential_rate(2.0));
    let exit = b.sink("exit");
    let away = b.sink("away");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.turned_away(desk, away);
    assert!(
        errors(&b)
            .iter()
            .any(|d| matches!(d.problem, Problem::TurnedAwayWithoutRoom { .. }))
    );
}

/// Without a finite room, or with a fixed one and no way for those turned away, nothing is
/// new: a way never taken changes no result.
#[test]
fn a_way_never_taken_changes_nothing() {
    let build = |way: bool| {
        let mut b = Builder::new();
        let arrivals = b.source("arrivals", Dist::exponential_rate(1.0), None);
        let desk = b.buffered_server("desk", 2, Dist::exponential_rate(1.0), 1_000);
        let exit = b.sink("exit");
        b.connect(arrivals, desk);
        b.connect(desk, exit);
        if way {
            let away = b.sink("away");
            b.turned_away(desk, away);
        }
        b
    };
    let plain = run_once(&build(false), 1_000.0);
    let with = run_once(&build(true), 1_000.0);
    let desk = |r: &Report| format!("{:?}", element(r, "desk"));
    assert_eq!(desk(&plain), desk(&with));
}

/// Case A (module comment): the long-run results match the chain's.
#[test]
#[ignore = "long runs against the chain; CI runs it in release with the ignored tests"]
fn per_working_server_with_a_health_check_and_retries_matches_the_chain() {
    let mut b = Builder::new();
    let arrivals = b.source("arrivals", Dist::exponential_rate(1.8), None);
    let desk = b.server("desk", 3, Dist::exponential_rate(1.0));
    let exit = b.sink("exit");
    let wait = b.delay("wait", Dist::exponential_rate(1.0 / 3.0));
    let again = b.exclusive("again");
    let quit = b.sink("quit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.queue_per_working_server(desk, 2);
    b.health_check(
        desk,
        HealthCheck {
            interval: Dist::exponential_rate(1.0),
            limit: 1.5,
            restart: Dist::exponential_rate(0.5),
            keep: None,
            on_takeout: OnTakeout::Retry,
            place_while_out: PlaceWhileOut::Reserved,
        },
    );
    b.turned_away(desk, wait);
    b.connect(wait, again);
    b.route(again, desk, Some(0.9), None);
    b.route(again, quit, Some(0.1), None);
    let duration = 100_000.0;
    let s = run(&b, 1_000.0, duration, 16, 52_000);
    let checks = [
        covers(&s, "desk", Metric::MeanQueueLength, 1.0, 1.856_435_639_7),
        covers(&s, "desk", Metric::MeanInService, 1.0, 2.405_832_294_9),
        covers(&s, "desk", Metric::Utilisation, 3.0, 1.600_691_420_1),
        covers(&s, "wait", Metric::MeanContents, 1.0, 5.979_257_396_5),
        covers(&s, "desk", Metric::Lost, 1.0 / duration, 1.993_085_798_8),
        covers(
            &s,
            "arrivals",
            Metric::Lost,
            1.0 / duration,
            0.499_616_575_0,
        ),
        covers(&s, "quit", Metric::Left, 1.0 / duration, 0.199_308_579_9),
        covers(
            &s,
            "desk",
            Metric::TakenOut,
            1.0 / duration,
            0.297_083_852_6,
        ),
        covers(&s, "exit", Metric::Left, 1.0 / duration, 1.600_691_420_1),
    ];
    assert!(checks.iter().all(|c| *c), "{checks:?}");
}

/// Case B (module comment): the transient results over [0, 100] match the chain's.
#[test]
#[ignore = "long runs against the chain; CI runs it in release with the ignored tests"]
fn a_waiting_room_by_timetable_matches_the_transient_chain() {
    let mut b = Builder::new();
    b.calendar("busy", vec![once((0, 50, 0), (1, 40, 0))]);
    let arrivals = b.source("arrivals", Dist::exponential_rate(1.5), None);
    let desk = b.server("desk", 2, Dist::exponential_rate(1.0));
    let exit = b.sink("exit");
    b.connect(arrivals, desk);
    b.connect(desk, exit);
    b.queue_by_shift(desk, 1, &[("busy", 6)]);
    let s = run(&b, 0.0, 100.0, 20_000, 2_000_000);
    let checks = [
        covers(&s, "desk", Metric::Lost, 1.0, 15.862_597_108_1),
        covers(&s, "desk", Metric::MeanQueueLength, 1.0, 0.640_723_918_7),
        covers(&s, "desk", Metric::MeanWorkInProgress, 1.0, 1.955_857_886_2),
        covers(&s, "desk", Metric::WorkInProgress, 1.0, 2.624_005_361_1),
    ];
    assert!(checks.iter().all(|c| *c), "{checks:?}");
}
