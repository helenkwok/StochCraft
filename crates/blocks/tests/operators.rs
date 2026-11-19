//! Operators who walk or drive (ROADMAP M5.2), worked by hand with constant times: the times
//! travelling empty, loading, travelling loaded and unloading, the unit chosen for a request,
//! going home, and the shares of time adding up to 1.
//!
//! Every line here is `in` -> `pickup` (a server taking 1) -> `haul` (the transport) -> `out`,
//! on a straight line, with a pool `ops` whose units carry items from `pickup` to `out`.
#![allow(clippy::panic)]

use stochcraft_blocks::{AfterJob, Builder, OperatorState, Report, checks, compile};
use stochcraft_model::{Dist, Experiment, Model};
use stochcraft_random::Rng;

/// A line of the shape above, and how its pool moves.
struct Line {
    interarrival: Dist,
    limit: Option<u64>,
    units: u32,
    speed_empty: f64,
    speed_loaded: f64,
    after: AfterJob,
    /// Whether the units start at `in` (else at `out`, where they put an item down).
    home_at_source: bool,
    /// Where `in`, `pickup` and `out` are, in drawing units (the scale is 1).
    at: [(f64, f64); 3],
    load: Dist,
    unload: Dist,
}

impl Line {
    fn builder(&self) -> Builder {
        let mut m = Builder::new();
        let src = m.source("in", self.interarrival.clone(), self.limit);
        let pickup = m.server("pickup", 1, Dist::Constant(1.0));
        let haul = m.transport("haul", "ops", self.load.clone(), self.unload.clone());
        let out = m.sink("out");
        m.connect(src, pickup);
        m.connect(pickup, haul);
        m.connect(haul, out);
        m.pool("ops", self.units, &[]);
        let home = if self.home_at_source { src } else { out };
        m.mobile("ops", self.speed_empty, self.speed_loaded, home, self.after);
        for (block, (x, y)) in [src, pickup, out].into_iter().zip(self.at) {
            m.place(block, x, y);
        }
        m
    }
}

fn model(m: &Builder, warmup: f64, duration: f64) -> Model {
    m.build(Experiment {
        warmup,
        duration,
        ..Experiment::default()
    })
}

fn run(m: &Builder, warmup: f64, duration: f64) -> (Model, Report) {
    let model = model(m, warmup, duration);
    let compiled = compile(&model).unwrap_or_else(|e| panic!("{e:?}"));
    let report = compiled.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"));
    (model, report)
}

fn near(a: Option<f64>, b: f64, tol: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() <= tol)
}

/// The pool's shares of time, in the order idle, empty, loading, loaded, unloading.
fn shares(r: &Report) -> [Option<f64>; 5] {
    let p = r.element("ops").unwrap_or_else(|| panic!("no pool"));
    [
        p.time_idle,
        p.time_travel_empty,
        p.time_loading,
        p.time_travel_loaded,
        p.time_unloading,
    ]
}

fn assert_shares(r: &Report, want: [f64; 5], tol: f64) {
    for (got, want) in shares(r).into_iter().zip(want) {
        assert!(near(got, want, tol), "{:?} against {want}", shares(r));
    }
}

fn sum_of_shares(r: &Report) -> f64 {
    shares(r).into_iter().flatten().sum()
}

/// One unit, `out` 10 from `pickup`, 1 a unit of time empty and 2 loaded, load 1, unload 1,
/// staying where it puts the item down (which is where it starts), with items every `every`.
fn one_unit_10_apart(every: f64) -> Line {
    Line {
        interarrival: Dist::Constant(every),
        limit: None,
        units: 1,
        speed_empty: 1.0,
        speed_loaded: 2.0,
        after: AfterJob::Stay,
        home_at_source: false,
        at: [(-50.0, 0.0), (0.0, 0.0), (10.0, 0.0)],
        load: Dist::Constant(1.0),
        unload: Dist::Constant(1.0),
    }
}

#[test]
fn with_items_always_waiting_each_cycle_is_walk_back_load_carry_unload() {
    // An item reaches `haul` at 3 (one every 2, then 1 at `pickup`) and more follow faster
    // than one in 17, so one is always waiting. The unit starts at `out`, 10 from `pickup`.
    // Each cycle: walk back 10 at 1 = 10, load 1, carry 10 at 2 = 5, unload 1: 17, every one,
    // the first included. Measured from 3 for 51,008.5 = 3,000 cycles and 8.5 of a 3,001st
    // (still walking: it set out at 51,003), the time of that one unit is:
    //   travelling empty 30,008.5, loading 3,000, travelling loaded 15,000, unloading 3,000,
    //   idle 0 (a unit freed takes the next request at once).
    // Over 51,008.5 those are close to 10/17, 1/17, 5/17, 1/17 and 0,
    // to within a cycle's share of the run (1/3,000).
    let line = one_unit_10_apart(2.0);
    let (model, r) = run(&line.builder(), 3.0, 51_008.5);
    let w = 51_008.5;
    assert_shares(
        &r,
        [0.0, 30_008.5 / w, 3_000.0 / w, 15_000.0 / w, 3_000.0 / w],
        1e-9,
    );
    assert_shares(
        &r,
        [0.0, 10.0 / 17.0, 1.0 / 17.0, 5.0 / 17.0, 1.0 / 17.0],
        1e-3,
    );
    assert!((sum_of_shares(&r) - 1.0).abs() < 1e-9, "{:?}", shares(&r));
    let pool = r.element("ops").unwrap_or_else(|| panic!("no pool"));
    // Distance is speed times time moving: 30,008.5 empty at 1, and 15,000 loaded at 2.
    assert!(near(pool.distance_empty, 30_008.5, 1e-6), "{pool:?}");
    assert!(near(pool.distance_loaded, 30_000.0, 1e-6), "{pool:?}");
    let haul = r.element("haul").unwrap_or_else(|| panic!("no haul"));
    // 3,000 items delivered in 51,008.5: the throughput is 1/17 to within a cycle of 3,000.
    assert_eq!(haul.left, 3_000);
    assert!((haul.left as f64 / w - 1.0 / 17.0).abs() < 1e-4);
    // Load 1, carry 5, unload 1.
    assert!(near(haul.mean_time_carried, 7.0, 1e-9), "{haul:?}");
    // Nothing the behaviour checks look at is off.
    assert!(checks::check_report(&model, &r).is_empty());
}

#[test]
fn with_an_item_every_40_the_unit_is_idle_23_fortieths_and_each_item_waits_the_walk() {
    // An item every 40 reaches `haul` at 41, 81, ... (1 at `pickup`). The unit is at `out`
    // each time: walk 10, load 1, carry 5, unload 1 = 17 busy of 40. Measured from 41 for 2,000
    // (50 whole periods, items 1 to 50 delivered by 40k + 18):
    //   idle 1,150 = 23/40, empty 500 = 10/40, loading 50 = 1/40, loaded 250 = 5/40,
    //   unloading 50 = 1/40.
    // An item waits for transport until loading starts: the walk back, 10, every time, so
    // the mean wait is 10 and it is waiting 10 of every 40 (mean queue 0.25). It is carried
    // 1 + 5 + 1 = 7. The unit travelled empty 500 and loaded 250 at 2 = 500.
    let line = one_unit_10_apart(40.0);
    let (model, r) = run(&line.builder(), 41.0, 2_000.0);
    assert_shares(&r, [0.575, 0.25, 0.025, 0.125, 0.025], 1e-9);
    let haul = r.element("haul").unwrap_or_else(|| panic!("no haul"));
    assert_eq!(haul.left, 50);
    assert!(near(haul.mean_wait, 10.0, 1e-9), "{haul:?}");
    assert!(near(haul.mean_queue_length, 0.25, 1e-9), "{haul:?}");
    assert!(near(haul.mean_time_carried, 7.0, 1e-9), "{haul:?}");
    let pool = r.element("ops").unwrap_or_else(|| panic!("no pool"));
    assert!(near(pool.distance_empty, 500.0, 1e-9), "{pool:?}");
    assert!(near(pool.distance_loaded, 500.0, 1e-9), "{pool:?}");
    assert!(near(pool.utilisation, 0.425, 1e-9), "{pool:?}");
    assert!(checks::check_report(&model, &r).is_empty());
}

#[test]
fn a_request_takes_the_nearest_idle_unit() {
    // Two units start at `in`, 20 from `pickup` (`out` is 5 from it, on the other side from
    // `in`: at 0, 5 and 20 on the line), speed 1 empty and loaded, load 1, unload 1, staying
    // where they put an item down. Items reach `haul` at 101 and 201.
    //  - At 101 both units are 20 away: the first (lowest index) goes. It reaches `pickup` at
    //    121, loads to 122, carries 5 to 127, unloads to 128, and is idle at `out`, 5 away.
    //  - At 201 unit 0 is 5 away and unit 1 20 away: unit 0 goes again.
    // At 110, 9 of the 20 into its first walk, unit 0 is 0.45 of the way from `in` to `pickup`.
    // At 203 it is 2 of the 5 into its walk from `out` to `pickup` (0.4), and unit 1 has not
    // moved. By 300 unit 0 has travelled 20 + 5 = 25 empty and 5 + 5 = 10 loaded, unit 1 none:
    // a mean of 12.5 empty and 5 loaded per unit (had unit 1 taken the second, 20 empty).
    let line = Line {
        interarrival: Dist::Constant(100.0),
        limit: Some(2),
        units: 2,
        speed_empty: 1.0,
        speed_loaded: 1.0,
        after: AfterJob::Stay,
        home_at_source: true,
        at: [(20.0, 0.0), (0.0, 0.0), (5.0, 0.0)],
        load: Dist::Constant(1.0),
        unload: Dist::Constant(1.0),
    };
    let m = model(&line.builder(), 0.0, 300.0);
    let compiled = compile(&m).unwrap_or_else(|e| panic!("{e:?}"));
    let mut live = compiled.live(Rng::new(1));
    let unit = |live: &stochcraft_blocks::Live<'_>, n: usize| {
        live.operators()
            .into_iter()
            .find(|o| o.unit == n)
            .unwrap_or_else(|| panic!("no unit {n}"))
    };

    live.advance_to(50.0).unwrap_or_else(|e| panic!("{e}"));
    for n in 0..2 {
        let u = unit(&live, n);
        assert_eq!(
            (u.state, u.from.as_str(), u.to.as_str()),
            (OperatorState::Idle, "in", "in")
        );
    }

    live.advance_to(110.0).unwrap_or_else(|e| panic!("{e}"));
    let u = unit(&live, 0);
    assert_eq!(u.state, OperatorState::TravelEmpty);
    assert_eq!((u.from.as_str(), u.to.as_str()), ("in", "pickup"));
    assert!((u.progress - 0.45).abs() < 1e-9, "{u:?}");
    assert_eq!(unit(&live, 1).state, OperatorState::Idle);

    live.advance_to(150.0).unwrap_or_else(|e| panic!("{e}"));
    let u = unit(&live, 0);
    assert_eq!(
        (u.state, u.from.as_str(), u.to.as_str()),
        (OperatorState::Idle, "out", "out")
    );

    live.advance_to(203.0).unwrap_or_else(|e| panic!("{e}"));
    let u = unit(&live, 0);
    assert_eq!(u.state, OperatorState::TravelEmpty);
    assert_eq!((u.from.as_str(), u.to.as_str()), ("out", "pickup"));
    assert!((u.progress - 0.4).abs() < 1e-9, "{u:?}");
    let u = unit(&live, 1);
    assert_eq!((u.state, u.from.as_str()), (OperatorState::Idle, "in"));

    live.advance_to(300.0).unwrap_or_else(|e| panic!("{e}"));
    let r = live.report();
    let pool = r.element("ops").unwrap_or_else(|| panic!("no pool"));
    assert!(near(pool.distance_empty, 12.5, 1e-9), "{pool:?}");
    assert!(near(pool.distance_loaded, 5.0, 1e-9), "{pool:?}");
    assert!((sum_of_shares(&r) - 1.0).abs() < 1e-9, "{:?}", shares(&r));
    assert_eq!(r.element("haul").map(|h| h.left), Some(2));
}

#[test]
fn a_unit_going_home_walks_it_empty_and_is_not_free_on_the_way() {
    // `in` (home) at -20, `pickup` at 0 and `out` at 10 on the line; 2 a unit of time empty, 1
    // loaded; load 1, unload 1; one item every 100 (at `haul` from 101, 201, ...). The unit
    // is at home each time: walk 20 at 2 = 10, load 1, carry 10 at 1 = 10, unload 1, then walk
    // home 30 at 2 = 15: 37 of every 100, the walk home counting as travelling empty.
    // Measured from 101 for 2,000 (20 whole periods), per period:
    //   idle 63, empty 10 + 15 = 25, loading 1, loaded 10, unloading 1.
    // An item waits for transport 10 (the walk to the pickup), is carried 1 + 10 + 1 = 12.
    // The unit travelled empty 25 x 20 = 500 at 2 = 1,000 and loaded 10 x 20 = 200.
    let line = Line {
        interarrival: Dist::Constant(100.0),
        limit: None,
        units: 1,
        speed_empty: 2.0,
        speed_loaded: 1.0,
        after: AfterJob::GoHome,
        home_at_source: true,
        at: [(-20.0, 0.0), (0.0, 0.0), (10.0, 0.0)],
        load: Dist::Constant(1.0),
        unload: Dist::Constant(1.0),
    };
    let (model, r) = run(&line.builder(), 101.0, 2_000.0);
    assert_shares(&r, [0.63, 0.25, 0.01, 0.10, 0.01], 1e-9);
    let pool = r.element("ops").unwrap_or_else(|| panic!("no pool"));
    assert!(near(pool.distance_empty, 1_000.0, 1e-9), "{pool:?}");
    assert!(near(pool.distance_loaded, 200.0, 1e-9), "{pool:?}");
    let haul = r.element("haul").unwrap_or_else(|| panic!("no haul"));
    assert_eq!(haul.left, 20);
    assert!(near(haul.mean_wait, 10.0, 1e-9), "{haul:?}");
    assert!(near(haul.mean_time_carried, 12.0, 1e-9), "{haul:?}");
    assert!(checks::check_report(&model, &r).is_empty());
}

#[test]
fn a_unit_walking_home_is_taken_only_once_it_is_home() {
    // As above but items every 20, reaching `haul` at 21, 41, 61, ... (faster than one in 37).
    // The first: walk to `pickup` 21 to 31, load to 32, carry to 42, unload to 43, walk home
    // (30 at 2 = 15) to 58. The second, which came at 41 and waited, is not given the unit on
    // its way home (it is free to be taken only at home): it is taken at 58, walks to 68, loads
    // to 69, carries to 79, unloads to 80 and walks home to 95, where the third (at 61) takes
    // it. Each cycle after the first starts as the last unit got home, 37 after the one
    // before; the unit is idle only for the 21 before the first item.
    let line = Line {
        interarrival: Dist::Constant(20.0),
        limit: None,
        units: 1,
        speed_empty: 2.0,
        speed_loaded: 1.0,
        after: AfterJob::GoHome,
        home_at_source: true,
        at: [(-20.0, 0.0), (0.0, 0.0), (10.0, 0.0)],
        load: Dist::Constant(1.0),
        unload: Dist::Constant(1.0),
    };
    let m = model(&line.builder(), 0.0, 1_000.0);
    let compiled = compile(&m).unwrap_or_else(|e| panic!("{e:?}"));
    let mut live = compiled.live(Rng::new(1));
    let at = |live: &mut stochcraft_blocks::Live<'_>, t: f64| {
        live.advance_to(t).unwrap_or_else(|e| panic!("{e}"));
        live.operators()
            .into_iter()
            .next()
            .unwrap_or_else(|| panic!("no unit"))
    };
    // At 50 it is on its way home from `out` (set out at 43, home at 58), 7 of 15 along.
    let u = at(&mut live, 50.0);
    assert_eq!(u.state, OperatorState::TravelEmpty);
    assert_eq!((u.from.as_str(), u.to.as_str()), ("out", "in"));
    assert!((u.progress - 7.0 / 15.0).abs() < 1e-9, "{u:?}");
    // At 58, home, the oldest request takes it at once: it is walking to `pickup` (68).
    let u = at(&mut live, 60.0);
    assert_eq!(u.state, OperatorState::TravelEmpty);
    assert_eq!((u.from.as_str(), u.to.as_str()), ("in", "pickup"));
    assert!((u.progress - 0.2).abs() < 1e-9, "{u:?}");
    // Items keep it so busy that it is never idle after its first job: the shares over 1,000
    // add up to 1 with idle only the 21 before the first item.
    live.advance_to(1_000.0).unwrap_or_else(|e| panic!("{e}"));
    let r = live.report();
    assert!(
        near(shares(&r)[0], 21.0 / 1_000.0, 1e-9),
        "{:?}",
        shares(&r)
    );
    assert!((sum_of_shares(&r) - 1.0).abs() < 1e-9, "{:?}", shares(&r));
}

#[test]
fn the_states_add_up_to_all_the_time_in_every_run() {
    // Random times, several units, walking home or not, with a warm-up that cuts a leg in
    // two: the five shares of every unit's time always add up to 1.
    for (seed, after, units) in [
        (1, AfterJob::Stay, 1),
        (2, AfterJob::GoHome, 1),
        (3, AfterJob::Stay, 3),
        (4, AfterJob::GoHome, 3),
    ] {
        let line = Line {
            interarrival: Dist::Exponential { mean: 12.0 },
            limit: None,
            units,
            speed_empty: 1.5,
            speed_loaded: 1.0,
            after,
            home_at_source: seed % 2 == 0,
            at: [(0.0, 40.0), (30.0, 0.0), (60.0, 25.0)],
            load: Dist::Exponential { mean: 2.0 },
            unload: Dist::Exponential { mean: 1.0 },
        };
        let m = model(&line.builder(), 333.0, 5_000.0);
        let compiled = compile(&m).unwrap_or_else(|e| panic!("{e:?}"));
        let r = compiled
            .run(Rng::new(seed))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            (sum_of_shares(&r) - 1.0).abs() < 1e-9,
            "seed {seed}: {:?}",
            shares(&r)
        );
        let pool = r.element("ops").unwrap_or_else(|| panic!("no pool"));
        assert!(pool.distance_empty.is_some_and(|d| d > 0.0), "{pool:?}");
        assert!(pool.distance_loaded.is_some_and(|d| d > 0.0), "{pool:?}");
        let haul = r.element("haul").unwrap_or_else(|| panic!("no haul"));
        assert!(haul.left > 0, "{haul:?}");
        assert!(haul.mean_wait.is_some_and(|w| w > 0.0), "{haul:?}");
        assert!(checks::check_report(&m, &r).is_empty(), "seed {seed}");
    }
}
