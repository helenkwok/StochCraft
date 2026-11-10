//! Transfers into a server with a finite buffer, worked by hand: a part takes its place in the
//! buffer as its transfer starts, the sending machine stays occupied (blocked) through the
//! transfer, and the part joins the queue as the transfer ends.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, Compiled, Report, compile};
use stochcraft_model::{Dist, Experiment};
use stochcraft_random::Rng;

/// `parts` parts all at time 0 into machine `m1` (cycle `c1`, unlimited queue), which sends
/// them across a transfer of `transfer` into `m2` (cycle `c2`, `queue` waiting places), then
/// out.
fn line(parts: u64, c1: f64, transfer: f64, c2: f64, queue: u32, x: Experiment) -> Compiled {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(0.0), Some(parts));
    let m1 = m.server("m1", 1, Dist::Constant(c1));
    let m2 = m.buffered_server("m2", 1, Dist::Constant(c2), queue);
    m.transfer(m2, Dist::Constant(transfer));
    let out = m.sink("out");
    m.connect(src, m1);
    m.connect(m1, m2);
    m.connect(m2, out);
    let model = m.build(x);
    assert_eq!(model.validate(), vec![]);
    match compile(&model) {
        Ok(c) => c,
        Err(e) => panic!("{e:?}"),
    }
}

fn run(c: &Compiled) -> Report {
    c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

fn window(warmup: f64, duration: f64) -> Experiment {
    Experiment {
        warmup,
        duration,
        ..Experiment::default()
    }
}

fn left(r: &Report, id: &str) -> u64 {
    r.element(id).map_or(0, |e| e.left)
}

fn close(got: Option<f64>, want: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() < 1e-9),
        "{got:?} vs {want}"
    );
}

/// The second machine is the bottleneck (cycle 4 against 2 + 1 for the first and its transfer),
/// and its buffer never empties: after the first part (out at 7) one leaves every 4, so 20 in
/// the 80 time units after a warm-up of 21 (out at 23, 27, …, 99): throughput exactly 1/4.
#[test]
fn the_slower_machine_sets_the_pace() {
    let r = run(&line(40, 2.0, 1.0, 4.0, 2, window(21.0, 80.0)));
    assert_eq!(left(&r, "m2"), 20, "{r:?}");
    // The second machine never waits for work.
    close(r.element("m2").and_then(|e| e.utilisation), 1.0);
    // The first is held up, its unit blocked waiting for room or in transfer, part of the time.
    let blocked = r.element("m1").and_then(|e| e.blocking_probability);
    assert!(blocked.is_some_and(|b| b > 0.0 && b < 1.0), "{blocked:?}");
}

/// The first machine and its transfer are the bottleneck (3 + 2 against 2): the first machine
/// works [5k, 5k + 3) and holds each part through its transfer [5k + 3, 5k + 5), never waiting
/// for room; the second serves it [5k + 5, 5k + 7). After a warm-up of 21, the parts out at 22,
/// 27, …, 97: 16 in 80, throughput exactly 1/5. The first machine is blocked for its transfers,
/// 2 in every 5, and busy 3 in every 5; the second is busy 2 in every 5.
#[test]
fn the_transfer_holds_the_sending_machine() {
    let r = run(&line(40, 3.0, 2.0, 2.0, 2, window(21.0, 80.0)));
    assert_eq!(left(&r, "m2"), 16, "{r:?}");
    let m1 = r.element("m1");
    close(m1.and_then(|e| e.blocking_probability), 2.0 / 5.0);
    close(m1.and_then(|e| e.utilisation), 3.0 / 5.0);
    close(r.element("m2").and_then(|e| e.utilisation), 2.0 / 5.0);
    // Never waiting for room, the first machine's parts are not counted as blocked waiting.
    close(m1.and_then(|e| e.mean_blocked_time), 0.0);
    // The second machine's wait counts from the end of the transfer: none.
    close(r.element("m2").and_then(|e| e.mean_wait), 0.0);
}

/// The same line stopped at 99, with the 20th part in transfer [98, 100): it has left the
/// first machine and is inside the second (counted once, there), whose counts balance: 20
/// entered, 19 left, 1 inside.
#[test]
fn a_part_in_transfer_counts_once_at_the_receiving_machine() {
    let r = run(&line(20, 3.0, 2.0, 2.0, 2, window(0.0, 99.0)));
    let (m1, m2) = (r.element("m1"), r.element("m2"));
    assert_eq!(m1.map(|e| (e.entered, e.left)), Some((20, 20)), "{r:?}");
    assert_eq!(m1.and_then(|e| e.work_in_progress), Some(0), "{r:?}");
    assert_eq!(m2.map(|e| (e.entered, e.left)), Some((20, 19)), "{r:?}");
    assert_eq!(m2.and_then(|e| e.work_in_progress), Some(1), "{r:?}");
    // In transfer, the part is not in the queue or in service.
    close(m2.and_then(|e| e.mean_contents), 19.0 * 2.0 / 99.0);
    // The first machine: 19 whole transfers and one under way, 39 of 99.
    close(m1.and_then(|e| e.blocking_probability), 39.0 / 99.0);
    // Time-average work in progress, each part counted once: part k (from 0) is the first
    // machine's from 0 until its transfer starts at 5k + 3, then the second's until it leaves
    // at 5k + 7 (the 20th, from 98, until the end at 99).
    let first: f64 = (0..20).map(|k| 5.0 * f64::from(k) + 3.0).sum();
    close(m1.and_then(|e| e.mean_work_in_progress), first / 99.0);
    close(
        m2.and_then(|e| e.mean_work_in_progress),
        (19.0 * 4.0 + 1.0) / 99.0,
    );
}

/// The buffer's capacity (one unit and two waiting places) counts parts in transfer: stepping
/// through a run where the first machine keeps it full, the second machine never holds more
/// than three parts, in transfer, waiting or in service, and does hold three.
#[test]
fn the_capacity_is_never_exceeded_transit_included() {
    let c = line(40, 2.0, 1.0, 4.0, 2, window(0.0, 100.0));
    let mut live = c.live(Rng::new(1));
    let mut most = 0;
    let mut with_transit = false;
    while live.step().unwrap_or_else(|e| panic!("{e}")).is_some() {
        let r = live.report();
        let inside = r.element("m2").and_then(|e| e.work_in_progress);
        let Some(n) = inside else {
            panic!("no work in progress: {r:?}")
        };
        assert!(n <= 3, "{n} inside m2 at {}", live.now());
        most = most.max(n);
        // Inside but not waiting or in service: in transfer, at full capacity.
        let state = live
            .states()
            .into_iter()
            .find(|s| s.id.as_str() == "m2")
            .unwrap_or_else(|| panic!("no m2"));
        if n == 3 && state.waiting + state.busy < 3.0 {
            with_transit = true;
        }
    }
    assert_eq!(most, 3);
    assert!(
        with_transit,
        "the buffer was never full with a part in transfer"
    );
}

/// A source sending into a server with a transfer is held by nothing: each arrival sets off at
/// once and reaches the server a transfer later. Arrivals every 2 from 0, transfer 5, service
/// 1: each part is served [2k + 5, 2k + 6), so by 50, 23 have left (the last at 49).
#[test]
fn a_source_sends_parts_on_their_way() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(2.0), None);
    let srv = m.server("m", 1, Dist::Constant(1.0));
    m.transfer(srv, Dist::Constant(5.0));
    let out = m.sink("out");
    m.connect(src, srv);
    m.connect(srv, out);
    let model = m.build(window(0.0, 50.0));
    let c = match compile(&model) {
        Ok(c) => c.first_arrival_at_zero(true),
        Err(e) => panic!("{e:?}"),
    };
    let r = run(&c);
    assert_eq!(left(&r, "m"), 23, "{r:?}");
    // Arrivals at 0, 2, …, 50: 26 made, all sent (none lost, no buffer).
    assert_eq!(r.element("in").map(|e| (e.entered, e.left)), Some((26, 26)));
    close(r.element("m").and_then(|e| e.mean_wait), 0.0);
}

/// A minimum time in the buffer instead (the transfer does not hold the sender): with the
/// first machine and its transfer the bottleneck held (3 + 2 against 2), the first machine now
/// works without a break, [3k, 3k + 3), each part reaching the queue 2 later and served
/// [3k + 5, 3k + 7). After a warm-up of 21, the parts out at 22, 25, …, 100: 27 in 80,
/// throughput 1/3 where holding gave 1/5. The first machine is never blocked, and the part
/// still counts at the second machine from the moment it leaves the first.
#[test]
fn a_minimum_stay_frees_the_sending_machine_at_once() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(0.0), Some(40));
    let m1 = m.server("m1", 1, Dist::Constant(3.0));
    let m2 = m.buffered_server("m2", 1, Dist::Constant(2.0), 2);
    m.min_stay(m2, Dist::Constant(2.0));
    let out = m.sink("out");
    m.connect(src, m1);
    m.connect(m1, m2);
    m.connect(m2, out);
    let model = m.build(window(21.0, 80.0));
    assert_eq!(model.validate(), vec![]);
    let c = compile(&model).unwrap_or_else(|e| panic!("{e:?}"));
    let r = run(&c);
    assert_eq!(left(&r, "m2"), 27, "{r:?}");
    let m1 = r.element("m1");
    close(m1.and_then(|e| e.utilisation), 1.0);
    close(m1.and_then(|e| e.blocking_probability), 0.0);
    // Busy [3k + 5, 3k + 7): half of k = 5, all of k = 6 to 31, 53 of 80.
    close(r.element("m2").and_then(|e| e.utilisation), 53.0 / 80.0);
    // Part k is the second machine's [3k + 3, 3k + 7): 1 of k = 5, 4 each of k = 6 to 31, 2 of
    // k = 32: 107 over 80.
    close(
        r.element("m2").and_then(|e| e.mean_work_in_progress),
        107.0 / 80.0,
    );
}
