//! Routing to the shortest queue with room ([`stochcraft_model::Routing::ShortestQueue`]),
//! worked by hand: ties take turns, a full way is passed over, the element sending into the
//! gateway is the one that waits (blocked) while no way has room, and a source loses its
//! arrival then.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, Compiled, Report, compile};
use stochcraft_model::{Dist, Experiment, Model};
use stochcraft_random::Rng;

fn compiled(model: &Model) -> Compiled {
    assert_eq!(model.validate(), vec![]);
    match compile(model) {
        Ok(c) => c,
        Err(e) => panic!("{e:?}"),
    }
}

fn run(c: &Compiled) -> Report {
    c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

fn window(duration: f64) -> Experiment {
    Experiment {
        warmup: 0.0,
        duration,
        ..Experiment::default()
    }
}

fn entered(r: &Report, id: &str) -> u64 {
    r.element(id).map_or(0, |e| e.entered)
}

fn close(got: Option<f64>, want: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() < 1e-9),
        "{got:?} vs {want}"
    );
}

/// Parts every 2 from 2 into a fast way `a` (cycle 1) and a slow one `b` (cycle 100), each with
/// five waiting places. Neither has a part waiting at 2, 4, 6 and 8, so those take turns: `a`,
/// `b`, `a`, `b`. From then on `b` has one waiting and `a` none, so the rest go to `a`: six for
/// `a`, two for `b`.
#[test]
fn ties_take_turns_and_the_shorter_queue_wins() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(2.0), Some(8));
    let pick = m.shortest_queue("pick");
    let a = m.buffered_server("a", 1, Dist::Constant(1.0), 5);
    let b = m.buffered_server("b", 1, Dist::Constant(100.0), 5);
    let out = m.sink("out");
    m.connect(src, pick);
    m.connect(pick, a);
    m.connect(pick, b);
    m.connect(a, out);
    m.connect(b, out);
    let x = window(30.0);
    let r = run(&compiled(&m.build(x)));
    assert_eq!(entered(&r, "a"), 6, "{r:?}");
    assert_eq!(entered(&r, "b"), 2, "{r:?}");
    let pick = r.element("pick");
    assert_eq!(pick.map(|e| (e.entered, e.left)), Some((8, 8)));
}

/// Six parts at 0 into machine `h` (cycle 1), which sends them through the gateway to `a`
/// (cycle 10) or `b` (cycle 15), neither with any waiting room. The first goes to `a` at 1, the
/// second to `b` at 2 (`a` is full). The third, done at 3, finds both full: `h` holds it,
/// blocked, until `a` frees at 11. The fourth, done at 12, waits until `b` frees at 17; the
/// fifth, done at 18, until `a` frees at 21. So `a` gets three and `b` two by 21.5, and `h` was
/// blocked 8 + 5 + 3 = 16 over three parts.
#[test]
fn the_sender_waits_for_room_on_any_way() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(0.0), Some(6));
    let h = m.server("h", 1, Dist::Constant(1.0));
    let pick = m.shortest_queue("pick");
    let a = m.buffered_server("a", 1, Dist::Constant(10.0), 0);
    let b = m.buffered_server("b", 1, Dist::Constant(15.0), 0);
    let out = m.sink("out");
    m.connect(src, h);
    m.connect(h, pick);
    m.connect(pick, a);
    m.connect(pick, b);
    m.connect(a, out);
    m.connect(b, out);
    let r = run(&compiled(&m.build(window(21.5))));
    assert_eq!(entered(&r, "a"), 3, "{r:?}");
    assert_eq!(entered(&r, "b"), 2, "{r:?}");
    let h = r.element("h");
    assert_eq!(h.map(|e| e.left), Some(5));
    // Blocked 8, 5 and 3 waiting for room; the first two never waited.
    close(h.and_then(|e| e.mean_blocked_time), 16.0 / 5.0);
    close(h.and_then(|e| e.blocking_probability), 16.0 / 21.5);
    // Nothing waits before the gateway.
    assert_eq!(r.element("pick").map(|e| (e.entered, e.left)), Some((5, 5)));
}

/// Arrivals every 1 into two ways with no waiting room and cycle 10: the first two get a way,
/// the next three find both full and are lost, counted by the gateway and by the source.
#[test]
fn a_source_loses_its_arrival_when_no_way_has_room() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(1.0), Some(5));
    let pick = m.shortest_queue("pick");
    let a = m.buffered_server("a", 1, Dist::Constant(10.0), 0);
    let b = m.buffered_server("b", 1, Dist::Constant(10.0), 0);
    let out = m.sink("out");
    m.connect(src, pick);
    m.connect(pick, a);
    m.connect(pick, b);
    m.connect(a, out);
    m.connect(b, out);
    let r = run(&compiled(&m.build(window(9.5))));
    assert_eq!((entered(&r, "a"), entered(&r, "b")), (1, 1), "{r:?}");
    assert_eq!(r.element("pick").and_then(|e| e.lost), Some(3));
    assert_eq!(r.element("in").and_then(|e| e.lost), Some(3));
}

/// Four parts at 0 into `h` (cycle 1), routed to `a` or `b` (cycle 10, five waiting places
/// each) across a transfer of 2. With a transfer that holds the sender, `h` works [0, 1),
/// [3, 4), [6, 7), [9, 10) and carries each part [1, 3), [4, 6), [7, 9), [10, 12): over 20,
/// busy 4 and blocked 8, never waiting for room. Ties take turns: `a`, `b`, `a`, `b`. With a
/// minimum time in the buffer instead, `h` is never blocked and works [0, 4).
#[test]
fn a_transfer_into_the_way_taken_holds_the_sender_or_not() {
    for holds in [true, false] {
        let mut m = Builder::new();
        let src = m.source("in", Dist::Constant(0.0), Some(4));
        let h = m.server("h", 1, Dist::Constant(1.0));
        let pick = m.shortest_queue("pick");
        let a = m.buffered_server("a", 1, Dist::Constant(10.0), 5);
        let b = m.buffered_server("b", 1, Dist::Constant(10.0), 5);
        for w in [a, b] {
            if holds {
                m.transfer(w, Dist::Constant(2.0));
            } else {
                m.min_stay(w, Dist::Constant(2.0));
            }
        }
        let out = m.sink("out");
        m.connect(src, h);
        m.connect(h, pick);
        m.connect(pick, a);
        m.connect(pick, b);
        m.connect(a, out);
        m.connect(b, out);
        let r = run(&compiled(&m.build(window(20.0))));
        assert_eq!(
            (entered(&r, "a"), entered(&r, "b")),
            (2, 2),
            "{holds}: {r:?}"
        );
        let h = r.element("h");
        close(h.and_then(|e| e.utilisation), 4.0 / 20.0);
        close(
            h.and_then(|e| e.blocking_probability),
            if holds { 8.0 / 20.0 } else { 0.0 },
        );
        close(h.and_then(|e| e.mean_blocked_time), 0.0);
    }
}

/// Finite buffers behind a shortest-queue gateway are not warned about (the sender waits, not
/// the gateway), unlike behind a standard one; a probability on its way out has no effect and
/// is warned about.
#[test]
fn checks_know_the_sender_waits() {
    let codes = |shortest: bool| {
        let mut m = Builder::new();
        let src = m.source("in", Dist::Constant(1.0), None);
        let h = m.server("h", 1, Dist::Constant(0.5));
        let pick = if shortest {
            m.shortest_queue("pick")
        } else {
            m.exclusive("pick")
        };
        let a = m.buffered_server("a", 1, Dist::Constant(1.0), 2);
        let b = m.buffered_server("b", 1, Dist::Constant(1.0), 2);
        let out = m.sink("out");
        m.connect(src, h);
        m.connect(h, pick);
        m.route(pick, a, Some(0.5), None);
        m.route(pick, b, None, None);
        m.connect(a, out);
        m.connect(b, out);
        m.build(Experiment::default())
            .validate()
            .iter()
            .map(|d| d.problem.code())
            .collect::<Vec<_>>()
    };
    assert_eq!(codes(true), vec!["routing_ignored"]);
    assert_eq!(
        codes(false),
        vec!["buffer_behind_gateway", "buffer_behind_gateway"]
    );
}
