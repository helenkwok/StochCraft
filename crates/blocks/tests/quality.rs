//! ISO 22400-2's quality ratio, scrap ratio, rework ratio and OEE index from the scrap sinks
//! and rework connections a model marks. A part counts against the last server it finished at,
//! or the one a scrap sink names; each pass through a server counts as produced.
#![allow(clippy::panic)]

use stochcraft_blocks::{Builder, ElementReport, Report};
use stochcraft_model::Dist;
use stochcraft_random::Rng;

fn element(r: &Report, id: &str) -> ElementReport {
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

/// A machine whose parts are scrapped one time in ten. Its scrap ratio is exactly what reached
/// the scrap sink over what it made, near 0.1; quality is the rest; OEE is availability times
/// effectiveness times quality.
#[test]
fn scrap_counts_against_the_last_server() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Exponential { mean: 10.0 }, None);
    let m = b.server("machine", 1, Dist::Constant(4.0));
    b.planned_run_time(m, Dist::Constant(3.0));
    let check = b.exclusive("check");
    let good = b.sink("good");
    let scrap = b.sink("scrap");
    b.scrap(scrap, None);
    b.connect(src, m);
    b.connect(m, check);
    b.route(check, good, Some(0.9), None);
    b.route(check, scrap, Some(0.1), None);
    let Ok(r) = b.run(Rng::new(7), 0.0, 1_000_000.0) else {
        panic!("no report")
    };
    let e = element(&r, "machine");
    let made = e.left as f64;
    let scrapped = element(&r, "scrap").left as f64;
    close(e.scrap_ratio, scrapped / made, 1e-12);
    close(e.scrap_ratio, 0.1, 0.005);
    close(e.rework_ratio, 0.0, 0.0);
    close(e.quality_ratio, 1.0 - scrapped / made, 1e-12);
    let (Some(a), Some(eff), Some(q)) = (e.oee_availability, e.oee_effectiveness, e.quality_ratio)
    else {
        panic!("{e:?}")
    };
    close(e.oee, a * eff * q, 1e-12);
    // The gateway and sinks have no OEE rows.
    assert_eq!(element(&r, "check").quality_ratio, None);
}

/// An inspection that rejects the machine's work names the machine: the scrap counts against
/// the machine, not the inspection, whose quality stays 1.
#[test]
fn a_scrap_sink_can_name_the_server_it_counts_against() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Exponential { mean: 10.0 }, None);
    let m = b.server("machine", 1, Dist::Constant(4.0));
    let inspect = b.server("inspect", 1, Dist::Constant(1.0));
    let check = b.exclusive("check");
    let good = b.sink("good");
    let scrap = b.sink("scrap");
    b.scrap(scrap, Some(m));
    b.connect(src, m);
    b.connect(m, inspect);
    b.connect(inspect, check);
    b.route(check, good, Some(0.8), None);
    b.route(check, scrap, Some(0.2), None);
    let Ok(r) = b.run(Rng::new(3), 0.0, 1_000_000.0) else {
        panic!("no report")
    };
    let scrapped = element(&r, "scrap").left as f64;
    let machine = element(&r, "machine");
    close(machine.scrap_ratio, scrapped / machine.left as f64, 1e-12);
    close(machine.scrap_ratio, 0.2, 0.01);
    close(element(&r, "inspect").quality_ratio, 1.0, 0.0);
}

/// One part in five goes back to the machine for rework. Each pass counts as produced, so the
/// machine makes 1.25 passes per part out and its rework ratio is near 0.2; quality is 0.8.
#[test]
fn rework_counts_each_pass() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Exponential { mean: 10.0 }, None);
    let m = b.server("machine", 1, Dist::Constant(4.0));
    let check = b.exclusive("check");
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, check);
    b.route(check, out, Some(0.8), None);
    b.route(check, m, Some(0.2), None);
    b.rework(check, m);
    let Ok(r) = b.run(Rng::new(11), 0.0, 1_000_000.0) else {
        panic!("no report")
    };
    let e = element(&r, "machine");
    let done = element(&r, "out").left as f64;
    close(e.rework_ratio, 0.2, 0.01);
    close(e.scrap_ratio, 0.0, 0.0);
    close(e.quality_ratio, 0.8, 0.01);
    // Passes: what left plus what went back; within the parts still inside at the end.
    assert!((e.left as f64 - done * 1.25).abs() < done * 0.01, "{e:?}");
}

/// A model that marks no scrap or rework has no quality rows; its OEE counts quality as 1.
#[test]
fn without_marks_oee_counts_quality_as_one() {
    let mut b = Builder::new();
    let src = b.source("parts", Dist::Constant(10.0), None);
    let m = b.server("machine", 1, Dist::Constant(4.0));
    b.planned_run_time(m, Dist::Constant(3.0));
    let out = b.sink("out");
    b.connect(src, m);
    b.connect(m, out);
    let Ok(r) = b.run(Rng::new(1), 0.0, 100_000.0) else {
        panic!("no report")
    };
    let e = element(&r, "machine");
    assert_eq!(e.quality_ratio, None);
    assert_eq!(e.scrap_ratio, None);
    assert_eq!(e.rework_ratio, None);
    // Availability 0.4, effectiveness 3/4.
    close(e.oee, 0.4 * 0.75, 1e-3);
}
