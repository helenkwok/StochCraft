//! Conveyors (ROADMAP M5.1), worked by hand with constant times: the time on a conveyor, how
//! many it holds with its exit shut, accumulating or not, and the most it carries.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, Report, compile};
use stochcraft_model::{Dist, Experiment};
use stochcraft_random::Rng;

fn run(m: &Builder, duration: f64) -> Report {
    run_warned(m, duration, &[])
}

/// As [`run`], with the warnings `warned` (problem codes) expected from the model.
fn run_warned(m: &Builder, duration: f64, warned: &[&str]) -> Report {
    let model = m.build(Experiment {
        warmup: 0.0,
        duration,
        ..Experiment::default()
    });
    let codes: Vec<&str> = model.validate().iter().map(|d| d.problem.code()).collect();
    assert_eq!(codes, warned);
    let compiled = compile(&model).unwrap_or_else(|e| panic!("{e:?}"));
    compiled.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-9)
}

#[test]
fn a_conveyor_never_blocked_takes_its_length_over_its_speed() {
    for accumulating in [false, true] {
        // One every 5 onto 10 at 1 a unit of time: each item 10 on it.
        let mut m = Builder::new();
        let src = m.source("in", Dist::Constant(5.0), Some(20));
        let belt = m.conveyor("belt", 10.0, 1.0, accumulating, 0.5, 0.5);
        let out = m.sink("out");
        m.connect(src, belt);
        m.connect(belt, out);
        let r = run(&m, 1_000.0);
        let e = r.element("belt").unwrap_or_else(|| panic!("no belt"));
        assert_eq!((e.entered, e.left), (20, 20));
        assert!(close(e.mean_time_on, 10.0), "{:?}", e.mean_time_on);
        assert!(
            close(e.blocking_probability, 0.0),
            "{:?}",
            e.blocking_probability
        );
    }
}

#[test]
fn items_queue_a_pitch_apart_behind_a_slower_machine() {
    // Five items, one every 3, onto 4 at 1 a unit of time with pitch 1, into a machine of 5
    // with no waiting room. Item i reaches the end at max(tᵢ + 4, dᵢ₋₁ + 1) and leaves when the
    // machine takes it: at 4, 9, 14, 19 and 24, after getting on at 0, 3, 6, 9 and 12. Times on
    // 4, 6, 8, 10 and 12: a mean of 8. Not accumulating, only the front item ever waits, so the
    // belt stops just as long and the times are the same.
    for accumulating in [true, false] {
        let mut m = Builder::new();
        let src = m.source("in", Dist::Constant(3.0), Some(5));
        let belt = m.conveyor("belt", 4.0, 1.0, accumulating, 1.0, 0.0);
        let machine = m.buffered_server("machine", 1, Dist::Constant(5.0), 0);
        let out = m.sink("out");
        m.connect(src, belt);
        m.connect(belt, machine);
        m.connect(machine, out);
        let r = run(&m, 100.0);
        let e = r.element("belt").unwrap_or_else(|| panic!("no belt"));
        assert_eq!(e.left, 5);
        assert!(
            close(e.mean_time_on, 8.0),
            "{accumulating}: {:?}",
            e.mean_time_on
        );
    }
}

#[test]
fn an_accumulating_conveyor_with_its_exit_shut_fills_and_one_that_is_not_stops_short() {
    // 10 at 1 a unit of time, pitch 2: five places. One arrives every 3; the first goes into a
    // machine that never finishes, so the exit is shut from the second on, which reaches the
    // end at 13. Accumulating, items keep getting on until five are on it. Not accumulating,
    // the belt stops at 13: the item that got on at 12 has moved 1, less than a pitch, so the
    // entrance stays shut with four on it (got on at 3, 6, 9 and 12).
    for (accumulating, held) in [(true, 5), (false, 4)] {
        let mut m = Builder::new();
        let src = m.source("in", Dist::Constant(3.0), None);
        let belt = m.conveyor("belt", 10.0, 1.0, accumulating, 1.5, 0.5);
        let machine = m.buffered_server("machine", 1, Dist::Constant(1e9), 0);
        let out = m.sink("out");
        m.connect(src, belt);
        m.connect(belt, machine);
        m.connect(machine, out);
        let r = run(&m, 1_000.0);
        let e = r.element("belt").unwrap_or_else(|| panic!("no belt"));
        assert_eq!(
            e.work_in_progress,
            Some(held),
            "accumulating {accumulating}"
        );
        assert_eq!(e.left, 1);
    }
}

#[test]
fn a_conveyor_carries_at_most_its_speed_over_its_pitch() {
    // One every 0.5 onto a belt at 1 a unit of time with pitch 2: the entrance frees every 2,
    // so it takes one in every four arrivals (the source loses the rest) and carries 0.5 a unit
    // of time.
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(0.5), None);
    let belt = m.conveyor("belt", 10.0, 1.0, true, 2.0, 0.0);
    let out = m.sink("out");
    m.connect(src, belt);
    m.connect(belt, out);
    // The capacity table sees it before the run: offered 2 a unit of time, it carries 0.5.
    let r = run_warned(&m, 1_000.0, &["overloaded"]);
    let e = r.element("belt").unwrap_or_else(|| panic!("no belt"));
    // One gets on every 2 (from the first arrival, at 0 or 0.5): 500 in 1,000. Those that got
    // on in the last 10 are still on it, four or five as the first arrival falls.
    assert_eq!(e.entered, 500);
    assert!((495..=496).contains(&e.left), "{}", e.left);
    assert!(close(e.mean_time_on, 10.0));
}

#[test]
fn a_divert_sends_an_item_whose_lane_is_full_round_the_default_way() {
    // One a unit of time onto a belt of 2 at 1 (pitch 0.5, so the entrance is always free again
    // before the next arrival, never at the same instant), then a divert choosing the lane for
    // every item (probability 1); the lane is a machine of 2.5 with no waiting room. Items reach
    // the divert a unit apart: the lane takes one, the next two find it busy and go round (the
    // default way), and so on: of nine, three take the lane and six go round.
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), Some(9));
    let belt = m.conveyor("belt", 2.0, 1.0, true, 0.5, 0.0);
    let divert = m.divert("divert");
    let lane = m.buffered_server("lane", 1, Dist::Constant(2.5), 0);
    let packed = m.sink("packed");
    let round = m.sink("round");
    m.connect(src, belt);
    m.connect(belt, divert);
    m.route(divert, lane, Some(1.0), None);
    m.route(divert, round, None, None);
    m.default_route(divert, round);
    m.connect(lane, packed);
    let r = run(&m, 100.0);
    let left = |id: &str| r.element(id).map(|e| e.entered);
    assert_eq!(left("belt"), Some(9));
    assert_eq!(left("packed"), Some(3));
    assert_eq!(left("round"), Some(6));
    // The belt never waits: a full lane sends the item round at once.
    let e = r.element("belt").unwrap_or_else(|| panic!("no belt"));
    assert!(
        close(e.blocking_probability, 0.0),
        "{:?}",
        e.blocking_probability
    );
}

#[test]
fn a_divert_needs_a_default_way() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), Some(1));
    let divert = m.divert("divert");
    let out = m.sink("out");
    m.connect(src, divert);
    m.route(divert, out, Some(1.0), None);
    let model = m.build(Experiment::default());
    let codes: Vec<&str> = model.validate().iter().map(|d| d.problem.code()).collect();
    assert!(codes.contains(&"divert_no_default"), "{codes:?}");
}

#[test]
fn the_times_on_a_belt_all_in_one_bin_when_they_are_all_equal() {
    // One every 5 onto 10 at 1 a unit of time: every item 10 on it, so one bin holds them all.
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(5.0), Some(20));
    let belt = m.conveyor("belt", 10.0, 1.0, false, 0.5, 0.5);
    let out = m.sink("out");
    m.connect(src, belt);
    m.connect(belt, out);
    let r = run(&m, 1_000.0);
    let e = r.element("belt").unwrap_or_else(|| panic!("no belt"));
    let h = e
        .time_on_histogram
        .as_ref()
        .unwrap_or_else(|| panic!("no histogram"));
    assert_eq!(h.counts.iter().sum::<u64>(), e.left);
    assert_eq!(h.counts.iter().filter(|n| **n > 0).count(), 1);
    assert!((h.start - 10.0).abs() < 1e-9, "{}", h.start);
    // Nothing else reports one.
    let src_report = r.element("in").unwrap_or_else(|| panic!("no source"));
    assert!(src_report.time_on_histogram.is_none());
}

#[test]
fn two_different_times_on_a_belt_fill_two_bins() {
    // Two items, at 0 and 3, onto 4 at 1 a unit of time into a machine of 5 with no waiting
    // room: the first leaves the belt at 4 (4 on it), the second reaches the end at 7, waits for
    // the machine until 9 (6 on it).
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(3.0), Some(2));
    let belt = m.conveyor("belt", 4.0, 1.0, false, 1.0, 0.0);
    let machine = m.buffered_server("machine", 1, Dist::Constant(5.0), 0);
    let out = m.sink("out");
    m.connect(src, belt);
    m.connect(belt, machine);
    m.connect(machine, out);
    let r = run(&m, 100.0);
    let e = r.element("belt").unwrap_or_else(|| panic!("no belt"));
    let h = e
        .time_on_histogram
        .as_ref()
        .unwrap_or_else(|| panic!("no histogram"));
    assert_eq!(h.counts.len(), 20);
    assert_eq!(h.counts.iter().filter(|n| **n > 0).count(), 2);
    assert_eq!((h.counts[0], h.counts[19]), (1, 1));
    assert!((h.start - 4.0).abs() < 1e-9 && (h.width - 0.1).abs() < 1e-9);
}
