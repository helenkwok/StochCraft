//! Setups against results worked by hand: a setup before every job (BPSim's SetUpTime) takes
//! its share of the unit's time and counts in its utilisation; a changeover happens only when
//! the watched property's value changes, its first part needs none, and a time for a particular
//! change replaces the default for that change only.
#![allow(clippy::panic)]

use stochcraft_blocks::{Breakdown, Builder, Change, Report, Setup};
use stochcraft_model::{Dist, PropertyValue};
use stochcraft_random::Rng;

fn element(r: &Report, id: &str) -> stochcraft_blocks::ElementReport {
    r.element(id)
        .cloned()
        .unwrap_or_else(|| panic!("no element {id}"))
}

fn close(got: Option<f64>, want: f64, tol: f64) {
    assert!(
        got.is_some_and(|g| (g - want).abs() <= tol),
        "{got:?} vs {want} ± {tol}"
    );
}

/// A part every 10 minutes; the machine sets up for 2 then works 3. Over a long run it is
/// setting up 20% of the time and busy (setup and work) 50%.
#[test]
fn a_setup_before_every_job_takes_its_share_of_the_time() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    let m = b.server("machine", 1, Dist::Constant(3.0));
    b.setup(m, Setup::every_job(Dist::Constant(2.0)));
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let r = b.run(Rng::new(1), 0.0, 100_000.0);
    let Ok(r) = r else { panic!("{r:?}") };
    let e = element(&r, "machine");
    close(e.time_setup, 0.2, 1e-3);
    close(e.utilisation, 0.5, 1e-3);
    // Each part spends setup and work in the machine: 5 minutes.
    close(element(&r, "out").mean_time_in_system, 5.0, 1e-9);
}

/// Parts of one type only: a changeover never happens (the first part needs none).
#[test]
fn one_type_needs_no_changeover() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    b.property(src, "type", None, PropertyValue::Text("A".into()));
    let m = b.server("machine", 1, Dist::Constant(3.0));
    b.setup(
        m,
        Setup {
            time: Dist::Constant(50.0),
            on_change_of: Some("type".into()),
            by_change: Vec::new(),
        },
    );
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(1), 0.0, 10_000.0) else {
        panic!("no report")
    };
    close(element(&r, "machine").time_setup, 0.0, 0.0);
}

/// Types A and B half each, drawn independently, a part every 10 minutes, work 1. The type
/// changes with probability 1/2 (A→B and B→A a quarter each). A changeover takes 1 minute, but
/// 3 from A to B: on average 0.25 × 3 + 0.25 × 1 = 1 minute per part, a share of 0.1.
#[test]
fn a_changeover_happens_only_when_the_type_changes_with_its_own_time() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    b.property(
        src,
        "type",
        None,
        PropertyValue::Choice(vec![(1.0, "A".into()), (1.0, "B".into())]),
    );
    let m = b.server("machine", 1, Dist::Constant(1.0));
    b.setup(
        m,
        Setup {
            time: Dist::Constant(1.0),
            on_change_of: Some("type".into()),
            by_change: vec![Change {
                from: "A".into(),
                to: "B".into(),
                time: Dist::Constant(3.0),
            }],
        },
    );
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(5), 0.0, 400_000.0) else {
        panic!("no report")
    };
    // 40,000 parts: the share's standard deviation is about 0.0005.
    close(element(&r, "machine").time_setup, 0.1, 0.003);
}

/// A failure during a setup interrupts it like a job: the setup resumes after the repair, so
/// with a setup of 2 and a job of 3 every part still spends 5 minutes of setup and work, plus
/// any repairs it waits through.
#[test]
fn a_failure_interrupts_a_setup_and_it_resumes() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    let m = b.server("machine", 1, Dist::Constant(3.0));
    b.setup(m, Setup::every_job(Dist::Constant(2.0)));
    b.breakdown(
        m,
        Breakdown {
            time_to_failure: Dist::Exponential { mean: 50.0 },
            repair: Dist::Constant(1.0),
            interruptible: true,
            hands_on_while_down: true,
        },
    );
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(3), 0.0, 200_000.0) else {
        panic!("no report")
    };
    let e = element(&r, "machine");
    // Setting up 20% of the time still: a repair stops the clock on the setup.
    close(e.time_setup, 0.2, 0.005);
    assert!(
        element(&r, "out")
            .mean_time_in_system
            .is_some_and(|t| t > 5.0),
        "repairs add to the time in the machine"
    );
}
