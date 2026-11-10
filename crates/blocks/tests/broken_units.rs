//! Broken units and finite buffers, worked by hand with fixed times: a unit that is down and
//! empty keeps its own place in the buffer free (the buffer holds only its waiting room), and a
//! unit that fails while holding a finished part hands it on under repair, or keeps it until
//! repaired ([`Breakdown::hands_on_while_down`]).
#![allow(clippy::panic)]

use stochcraft_blocks::{Breakdown, Builder, Report, compile};
use stochcraft_model::{Dist, Experiment, Model};
use stochcraft_random::Rng;

fn run(model: &Model) -> Report {
    assert_eq!(model.validate(), vec![]);
    let c = compile(model).unwrap_or_else(|e| panic!("{e:?}"));
    c.run(Rng::new(1)).unwrap_or_else(|e| panic!("{e}"))
}

fn window(duration: f64) -> Experiment {
    Experiment {
        warmup: 0.0,
        duration,
        ..Experiment::default()
    }
}

fn fails(after: f64, repair: f64, hands_on_while_down: bool) -> Breakdown {
    Breakdown {
        time_to_failure: Dist::Constant(after),
        repair: Dist::Constant(repair),
        interruptible: true,
        hands_on_while_down,
    }
}

/// Arrivals every 2 into a machine of cycle 1 with one waiting place, which fails at 5, empty,
/// and is repaired at 15. The parts of 2 and 4 are served at once; the one of 6 takes the
/// waiting place; with the broken machine's own place kept free, those of 8, 10, 12 and 14 find
/// no room and are lost (with it taken, the part of 8 would have got in).
#[test]
fn a_broken_empty_unit_keeps_its_place_free() {
    let mut m = Builder::new();
    let src = m.source("in", Dist::Constant(2.0), None);
    let b = m.buffered_server("b", 1, Dist::Constant(1.0), 1);
    m.breakdown(b, fails(5.0, 10.0, true));
    let out = m.sink("out");
    m.connect(src, b);
    m.connect(b, out);
    let r = run(&m.build(window(15.5)));
    let b = r.element("b");
    assert_eq!(b.map(|e| (e.entered, e.lost)), Some((3, Some(4))), "{r:?}");
}

/// Three parts at 0 into machine `a` (cycle 1), which fails at 3 and is repaired at 13, then
/// `b` (cycle 5, no waiting room). The first part reaches `b` at 1; the second, done at 2,
/// waits for room until `b` frees at 6. By default `a`, though broken, hands it on then; set to
/// keep it until repaired, it hands it on at 13. By 12.5, `b` has taken two or one.
#[test]
fn a_broken_unit_hands_on_its_finished_part_or_keeps_it() {
    for (hands_on, taken) in [(true, 2), (false, 1)] {
        let mut m = Builder::new();
        let src = m.source("in", Dist::Constant(0.0), Some(3));
        let a = m.server("a", 1, Dist::Constant(1.0));
        m.breakdown(a, fails(3.0, 10.0, hands_on));
        let b = m.buffered_server("b", 1, Dist::Constant(5.0), 0);
        let out = m.sink("out");
        m.connect(src, a);
        m.connect(a, b);
        m.connect(b, out);
        let r = run(&m.build(window(12.5)));
        assert_eq!(
            r.element("b").map(|e| e.entered),
            Some(taken),
            "{hands_on}: {r:?}"
        );
    }
}
