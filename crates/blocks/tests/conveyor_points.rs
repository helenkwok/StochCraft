//! Points along a conveyor section and junctions between sections (ROADMAP M5.5, L5), with
//! answers worked by hand: the time to a point is s/v; an item passing a point rides on exactly
//! as on the plain conveyor; a divert by probability gives binomial shares; what leaves one
//! section arrives on the next; a section with points holds as many as the plain one.
#![allow(clippy::panic)]

use stochcraft_blocks::{BlockId, Builder, Report, compile};
use stochcraft_model::{
    Dist, ElementId, Experiment, Frame, Junction, JunctionKind, Model, Path, Place, Segment,
    Severity, Track,
};
use stochcraft_random::Rng;

fn id(s: &str) -> ElementId {
    ElementId::unchecked(s)
}

fn experiment(duration: f64) -> Experiment {
    Experiment {
        warmup: 0.0,
        duration,
        ..Experiment::default()
    }
}

/// A straight track of `length` metres for conveyor `belt`.
fn lay(m: &mut Model, belt: &str, length: f64) {
    let path = Path::new(Frame::default(), vec![Segment::Line { length, rise: 0.0 }]);
    m.site
        .paths
        .insert(id(belt), Track::new(path, m.experiment.length_unit));
}

/// `block` placed `s` along conveyor `belt`.
fn place(m: &mut Model, block: &str, belt: &str, s: f64) {
    m.site
        .places
        .insert(id(block), Place::Along { path: id(belt), s });
}

/// The id of the flow from `from` to `to`.
fn flow(m: &Model, from: &str, to: &str) -> ElementId {
    m.connections
        .iter()
        .find(|c| c.from.as_str() == from && c.to.as_str() == to)
        .map(|c| c.id.clone())
        .unwrap_or_else(|| panic!("no flow {from} -> {to}"))
}

fn run(m: &Model) -> Report {
    let errors: Vec<String> = m
        .validate()
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.plain())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let compiled = compile(m).unwrap_or_else(|e| panic!("{e:?}"));
    compiled.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

fn element<'a>(r: &'a Report, id: &str) -> &'a stochcraft_blocks::ElementReport {
    r.element(id).unwrap_or_else(|| panic!("no {id}"))
}

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-9)
}

/// A source every `every` (at most `limit`) onto `belt` (length, speed, accumulating, item
/// length, gap), which sends into the divert `divert`; `lane` takes probability `p` from it and
/// `end` is its default way. The builder, the lane and the end.
fn line(
    every: f64,
    limit: Option<u64>,
    belt: (f64, f64, bool, f64, f64),
    lane: impl FnOnce(&mut Builder) -> BlockId,
    end: impl FnOnce(&mut Builder) -> BlockId,
    p: f64,
) -> (Builder, BlockId, BlockId) {
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(every), limit);
    let (length, speed, accumulating, item, gap) = belt;
    let conveyor = b.conveyor("belt", length, speed, accumulating, item, gap);
    let divert = b.divert("divert");
    let lane = lane(&mut b);
    let end = end(&mut b);
    b.connect(src, conveyor);
    b.connect(conveyor, divert);
    b.route(divert, lane, Some(p), None);
    b.route(divert, end, None, None);
    b.default_route(divert, end);
    (b, lane, end)
}

/// `line`, with the machine `end` sending into a sink.
fn into_machine(every: f64, belt: (f64, f64, bool, f64, f64), service: f64, p: f64) -> Builder {
    let (mut b, _, machine) = line(
        every,
        None,
        belt,
        |b| b.sink("lane"),
        |b| b.buffered_server("machine", 1, Dist::Constant(service), 0),
        p,
    );
    let out = b.sink("out");
    b.connect(machine, out);
    b
}

#[test]
fn the_time_to_a_point_is_its_distance_over_the_speed() {
    // 10 m at 0.5 m a unit of time; a divert 3 m along sends every item down its lane, so each
    // is on the belt 3 / 0.5 = 6, never the 20 the whole length takes.
    for accumulating in [false, true] {
        let (b, ..) = line(
            5.0,
            Some(20),
            (10.0, 0.5, accumulating, 0.5, 0.5),
            |b| b.sink("lane"),
            |b| b.sink("end"),
            1.0,
        );
        let mut m = b.build(experiment(1_000.0));
        lay(&mut m, "belt", 10.0);
        place(&mut m, "divert", "belt", 3.0);
        let r = run(&m);
        let belt = element(&r, "belt");
        assert_eq!((belt.entered, belt.left), (20, 20), "{accumulating}");
        assert!(close(belt.mean_time_on, 6.0), "{:?}", belt.mean_time_on);
        assert_eq!(element(&r, "lane").entered, 20);
        assert_eq!(element(&r, "end").entered, 0);
        // The divert counted each item in and out as it met it.
        let d = element(&r, "divert");
        assert_eq!((d.entered, d.left), (20, 20));
    }
}

#[test]
fn a_one_metre_item_passes_a_point_half_a_metre_in_as_on_the_plain_conveyor() {
    // A 4 m belt at 1 m a unit of time with 1 m items and no gap, offered one every 0.5 (so the
    // entrance turns some away) into a machine of 1.5 with no waiting room, so items queue at
    // the end. A divert 0.5 m in, which sends nothing down its lane, is spanned by each item as
    // it passes and changes nothing: every count and time is the plain conveyor's.
    for accumulating in [false, true] {
        let plain = {
            let mut b = Builder::new();
            let src = b.source("in", Dist::Constant(0.5), None);
            let belt = b.conveyor("belt", 4.0, 1.0, accumulating, 1.0, 0.0);
            let machine = b.buffered_server("machine", 1, Dist::Constant(1.5), 0);
            let out = b.sink("out");
            b.connect(src, belt);
            b.connect(belt, machine);
            b.connect(machine, out);
            let mut m = b.build(experiment(500.0));
            lay(&mut m, "belt", 4.0);
            run(&m)
        };
        let pointed = {
            let mut b = Builder::new();
            let src = b.source("in", Dist::Constant(0.5), None);
            let belt = b.conveyor("belt", 4.0, 1.0, accumulating, 1.0, 0.0);
            let divert = b.divert("divert");
            let lane = b.sink("lane");
            let machine = b.buffered_server("machine", 1, Dist::Constant(1.5), 0);
            let out = b.sink("out");
            b.connect(src, belt);
            b.connect(belt, divert);
            b.route(divert, lane, Some(0.0), None);
            b.route(divert, machine, None, None);
            b.default_route(divert, machine);
            b.connect(machine, out);
            let mut m = b.build(experiment(500.0));
            lay(&mut m, "belt", 4.0);
            place(&mut m, "divert", "belt", 0.5);
            run(&m)
        };
        for id in ["belt", "machine", "out"] {
            let (a, b) = (element(&plain, id), element(&pointed, id));
            assert_eq!(
                (a.entered, a.left),
                (b.entered, b.left),
                "{id} {accumulating}"
            );
            assert_eq!(a.lost, b.lost, "{id} {accumulating}");
            for (x, y) in [
                (a.mean_time_on, b.mean_time_on),
                (a.blocking_probability, b.blocking_probability),
                (a.mean_contents, b.mean_contents),
            ] {
                assert_eq!(x.is_some(), y.is_some(), "{id}");
                if let (Some(x), Some(y)) = (x, y) {
                    assert!((x - y).abs() < 1e-9, "{id} {accumulating}: {x} {y}");
                }
            }
        }
        assert_eq!(element(&pointed, "lane").entered, 0);
        // Every item that reached 0.5 m met the divert.
        let belt = element(&pointed, "belt");
        let d = element(&pointed, "divert");
        assert!(d.entered >= belt.left && d.entered <= belt.entered);
    }
}

#[test]
fn a_divert_by_probability_gives_binomial_shares_and_loses_nothing() {
    // 2,000 items, one a unit of time, onto 10 m at 1 m a unit of time; a divert 2 m along sends
    // each down its lane with probability 0.3. The number diverted D is binomial(2000, 0.3):
    // mean 600, standard deviation √(2000 · 0.3 · 0.7) ≈ 20.49; within four of them, 600 ± 82.
    // Every item is either diverted or rides to the end: D + the rest = 2,000, and the mean
    // time on the belt is (2 D + 10 (2000 − D)) / 2000 exactly.
    for accumulating in [false, true] {
        let (b, ..) = line(
            1.0,
            Some(2_000),
            (10.0, 1.0, accumulating, 0.4, 0.1),
            |b| b.sink("lane"),
            |b| b.sink("end"),
            0.3,
        );
        let mut m = b.build(experiment(3_000.0));
        lay(&mut m, "belt", 10.0);
        place(&mut m, "divert", "belt", 2.0);
        let r = run(&m);
        let diverted = element(&r, "lane").entered;
        let through = element(&r, "end").entered;
        assert_eq!(diverted + through, 2_000, "{accumulating}");
        assert!((518..=682).contains(&diverted), "{diverted}");
        let belt = element(&r, "belt");
        assert_eq!((belt.entered, belt.left), (2_000, 2_000));
        let d = diverted as f64;
        let mean = (2.0 * d + 10.0 * (2_000.0 - d)) / 2_000.0;
        assert!(
            close(belt.mean_time_on, mean),
            "{:?} {mean}",
            belt.mean_time_on
        );
    }
}

#[test]
fn a_section_with_points_holds_as_many_as_the_plain_one() {
    // As M5.1's exit-shut check: 10 m at 1, pitch 2, one every 3, the first item into a machine
    // that never finishes. A divert 5 m along that sends nothing changes nothing: accumulating,
    // five wait on the belt (⌊10 / 2⌋); not accumulating, four, as the belt stops short.
    for (accumulating, held) in [(true, 5), (false, 4)] {
        let b = into_machine(3.0, (10.0, 1.0, accumulating, 1.5, 0.5), 1e9, 0.0);
        let mut m = b.build(experiment(1_000.0));
        lay(&mut m, "belt", 10.0);
        place(&mut m, "divert", "belt", 5.0);
        let r = run(&m);
        let belt = element(&r, "belt");
        assert_eq!(belt.work_in_progress, Some(held), "{accumulating}");
        assert_eq!(belt.left, 1);
        assert_eq!(element(&r, "lane").entered, 0);
    }
}

#[test]
fn only_the_items_whose_front_gets_there_meet_a_point_behind_a_shut_exit() {
    // 10 m at 1 with 1 m items and no gap, one every 3, the first into a machine that never
    // finishes; a divert 5 m along sends nothing. Not accumulating, the belt stops when the
    // second item reaches the end, 13 after the first got on: the items that got on 0, 3 and 6
    // after it have passed 5 m by then, the one at 9 is at 4 m. Accumulating, the items queue
    // a metre apart from the end, at 10, 9, 8, 7, 6 and 5 m: with the first, seven fronts reach
    // 5 m, and the one stopping at 4 m never does.
    for (accumulating, met) in [(false, 3), (true, 7)] {
        let b = into_machine(3.0, (10.0, 1.0, accumulating, 1.0, 0.0), 1e9, 0.0);
        let mut m = b.build(experiment(1_000.0));
        lay(&mut m, "belt", 10.0);
        place(&mut m, "divert", "belt", 5.0);
        let r = run(&m);
        assert_eq!(element(&r, "divert").entered, met, "{accumulating}");
    }
}

#[test]
fn items_on_an_accumulating_belt_with_a_point_never_come_closer_than_a_pitch() {
    // One every 0.7 onto 8 m at 1 with a pitch of 1, a divert 5 m along taking half of them,
    // the rest into a machine of 1.6 with no waiting room, so items queue past the point. At
    // every look, the items on the belt are in order, a pitch apart or more.
    let b = into_machine(0.7, (8.0, 1.0, true, 0.8, 0.2), 1.6, 0.5);
    let mut m = b.build(experiment(200.0));
    lay(&mut m, "belt", 8.0);
    place(&mut m, "divert", "belt", 5.0);
    let compiled = compile(&m).unwrap_or_else(|e| panic!("{e:?}"));
    let mut live = compiled.live(Rng::new(3));
    let mut t = 0.0;
    while t < 200.0 {
        t += 0.37;
        live.advance_to(t).unwrap_or_else(|e| panic!("{e}"));
        let items = live.conveyor_items("belt");
        for pair in items.windows(2) {
            let gap = f64::from(pair[0].0 - pair[1].0) * 8.0;
            assert!(gap >= 1.0 - 1e-4, "at {t}: {items:?}");
        }
    }
}

#[test]
fn several_points_are_met_in_turn_along_the_belt() {
    // 10 m at 1; a divert at 3 m whose lane is a machine of 100 with no waiting room, then a
    // divert at 6 m whose lane always has room, each sending every item it can. Of 20 items, one
    // every 5, the first takes the first lane (the machine is then busy past the last), the
    // other 19 the second. Times on the belt: 3 for one and 6 for 19, a mean of 5.85.
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(5.0), Some(20));
    let belt = b.conveyor("belt", 10.0, 1.0, false, 0.5, 0.5);
    let first = b.divert("first");
    let second = b.divert("second");
    let lane1 = b.buffered_server("lane1", 1, Dist::Constant(100.0), 0);
    let lane2 = b.sink("lane2");
    let end = b.sink("end");
    let done = b.sink("done");
    b.connect(src, belt);
    b.connect(belt, first);
    b.route(first, lane1, Some(1.0), None);
    b.route(first, second, None, None);
    b.default_route(first, second);
    b.route(second, lane2, Some(1.0), None);
    b.route(second, end, None, None);
    b.default_route(second, end);
    b.connect(lane1, done);
    let build = |s1: f64, s2: f64| {
        let mut m = b.build(experiment(1_000.0));
        lay(&mut m, "belt", 10.0);
        place(&mut m, "first", "belt", s1);
        place(&mut m, "second", "belt", s2);
        m
    };
    let r = run(&build(3.0, 6.0));
    assert_eq!(element(&r, "lane1").entered, 1);
    assert_eq!(element(&r, "lane2").entered, 19);
    assert_eq!(element(&r, "end").entered, 0);
    assert!(close(element(&r, "belt").mean_time_on, 5.85));
    // Placed the other way round, the flows and the track disagree: an error.
    let codes: Vec<&str> = build(6.0, 3.0)
        .validate()
        .iter()
        .map(|d| d.problem.code())
        .collect();
    assert!(codes.contains(&"points_out_of_order"), "{codes:?}");
}

#[test]
fn a_divert_placed_off_its_belt_is_an_error() {
    let (b, ..) = line(
        5.0,
        Some(1),
        (10.0, 1.0, true, 0.5, 0.5),
        |b| b.sink("lane"),
        |b| b.sink("end"),
        1.0,
    );
    let mut m = b.build(experiment(100.0));
    lay(&mut m, "belt", 10.0);
    place(&mut m, "divert", "belt", 12.0);
    let errors: Vec<&str> = m
        .validate()
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.problem.code())
        .collect();
    assert_eq!(errors, ["place_off_path"]);
}

#[test]
fn a_divert_at_the_end_of_its_belt_works_as_before() {
    // M5.1's divert check, with the divert placed at the very end of the belt's track: the same
    // three of nine take the lane and six go round.
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(1.0), Some(9));
    let belt = b.conveyor("belt", 2.0, 1.0, true, 0.5, 0.0);
    let divert = b.divert("divert");
    let lane = b.buffered_server("lane", 1, Dist::Constant(2.5), 0);
    let packed = b.sink("packed");
    let round = b.sink("round");
    b.connect(src, belt);
    b.connect(belt, divert);
    b.route(divert, lane, Some(1.0), None);
    b.route(divert, round, None, None);
    b.default_route(divert, round);
    b.connect(lane, packed);
    let mut m = b.build(experiment(100.0));
    lay(&mut m, "belt", 2.0);
    place(&mut m, "divert", "belt", 2.0);
    let r = run(&m);
    assert_eq!(element(&r, "packed").entered, 3);
    assert_eq!(element(&r, "round").entered, 6);
}

/// Conveyor `a` (6 m at 1) into conveyor `b` (4 m at 0.5), one item every 5, twenty of them.
fn two_sections(junction: Option<Junction>) -> Report {
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(5.0), Some(20));
    let first = b.conveyor("a", 6.0, 1.0, true, 0.5, 0.5);
    let second = b.conveyor("b", 4.0, 0.5, true, 0.5, 0.5);
    let out = b.sink("out");
    b.connect(src, first);
    b.connect(first, second);
    b.connect(second, out);
    let mut m = b.build(experiment(1_000.0));
    lay(&mut m, "a", 6.0);
    lay(&mut m, "b", 4.0);
    if let Some(j) = junction {
        let f = flow(&m, "a", "b");
        m.site.junctions.insert(f, j);
    }
    run(&m)
}

#[test]
fn a_junction_adds_its_transfer_time_and_loses_nothing() {
    // Never held up, an item takes 6 / 1 = 6 on the first section and 4 / 0.5 = 8 on the second:
    // 14 in all, and 2 more across a turntable that takes 2. A junction of 0 changes nothing.
    // Everything that leaves the first section arrives on the second, and leaves it.
    for (junction, total) in [
        (None, 14.0),
        (Some(Junction::default()), 14.0),
        (
            Some(Junction {
                kind: JunctionKind::Turntable,
                transfer: 2.0,
            }),
            16.0,
        ),
    ] {
        let r = two_sections(junction);
        let (a, b, out) = (element(&r, "a"), element(&r, "b"), element(&r, "out"));
        assert_eq!((a.entered, a.left, b.entered, b.left), (20, 20, 20, 20));
        assert_eq!(out.entered, 20);
        assert!(close(a.mean_time_on, 6.0), "{:?}", a.mean_time_on);
        assert!(close(b.mean_time_on, 8.0), "{:?}", b.mean_time_on);
        assert!(
            close(out.mean_time_in_system, total),
            "{junction:?}: {:?}",
            out.mean_time_in_system
        );
    }
}

#[test]
fn throughput_is_conserved_across_a_junction_into_a_slower_machine() {
    // One every 0.5 onto a 10 m section that hands over across a junction of 0.5 to a 5 m one,
    // into a machine of 2 with no waiting room: the machine sets the pace, 0.5 a unit of time.
    // Every item the first section lets go is counted on the second, and every one the second
    // lets go reaches the machine.
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(0.5), None);
    let first = b.conveyor("a", 10.0, 1.0, true, 0.8, 0.2);
    let second = b.conveyor("b", 5.0, 1.0, true, 0.8, 0.2);
    let machine = b.buffered_server("machine", 1, Dist::Constant(2.0), 0);
    let out = b.sink("out");
    b.connect(src, first);
    b.connect(first, second);
    b.connect(second, machine);
    b.connect(machine, out);
    let mut m = b.build(experiment(1_000.0));
    lay(&mut m, "a", 10.0);
    lay(&mut m, "b", 5.0);
    let f = flow(&m, "a", "b");
    m.site.junctions.insert(
        f,
        Junction {
            kind: JunctionKind::Inline,
            transfer: 0.5,
        },
    );
    let r = run(&m);
    let (a, b, machine, out) = (
        element(&r, "a"),
        element(&r, "b"),
        element(&r, "machine"),
        element(&r, "out"),
    );
    assert_eq!(a.left, b.entered);
    assert_eq!(b.left, machine.entered);
    assert_eq!(machine.left, out.entered);
    // 1,000 at one every 2: at most 500, less the start-up (some 17 for the first to arrive).
    assert!((480..=500).contains(&out.entered), "{}", out.entered);
}

#[test]
fn a_junction_with_a_negative_time_is_an_error_and_one_off_a_conveyor_is_dropped() {
    let mut b = Builder::new();
    let src = b.source("in", Dist::Constant(5.0), Some(1));
    let first = b.conveyor("a", 6.0, 1.0, true, 0.5, 0.5);
    let second = b.conveyor("b", 4.0, 0.5, true, 0.5, 0.5);
    let out = b.sink("out");
    b.connect(src, first);
    b.connect(first, second);
    b.connect(second, out);
    let mut m = b.build(experiment(100.0));
    let ab = flow(&m, "a", "b");
    let into = flow(&m, "b", "out");
    m.site.junctions.insert(
        ab,
        Junction {
            kind: JunctionKind::Side,
            transfer: -1.0,
        },
    );
    m.site.junctions.insert(into, Junction::default());
    let codes: Vec<&str> = m
        .validate()
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.problem.code())
        .collect();
    assert_eq!(codes, ["bad_junction_time"]);
    // A junction belongs between two conveyors: the one into the sink is not kept.
    let kept = m.site.kept(&m);
    assert_eq!(kept.junctions.len(), 1);
}

#[test]
fn an_item_sent_round_onto_the_same_belt_rides_it_again_from_the_start() {
    // 10 m at 1 with a pitch of 1; a divert 2 m along sends every item straight back onto the
    // start of the same belt. One item: each ride takes it 2 m, 2 units of time, and the ride
    // before it, now off the belt ahead, never holds it up. Accumulating, the rides are told
    // apart even though the entity is the same.
    for accumulating in [false, true] {
        let mut b = Builder::new();
        let src = b.source("in", Dist::Constant(1.0), Some(1));
        let belt = b.conveyor("belt", 10.0, 1.0, accumulating, 0.5, 0.5);
        let divert = b.divert("divert");
        let end = b.sink("end");
        b.connect(src, belt);
        b.connect(belt, divert);
        b.route(divert, belt, Some(1.0), None);
        b.route(divert, end, None, None);
        b.default_route(divert, end);
        let mut m = b.build(experiment(21.5));
        lay(&mut m, "belt", 10.0);
        place(&mut m, "divert", "belt", 2.0);
        let r = run(&m);
        let belt = element(&r, "belt");
        assert!(belt.left >= 9, "{accumulating}: {}", belt.left);
        assert_eq!(element(&r, "divert").left, belt.left);
        assert!(close(belt.mean_time_on, 2.0), "{:?}", belt.mean_time_on);
        assert_eq!(element(&r, "end").entered, 0);
    }
}
