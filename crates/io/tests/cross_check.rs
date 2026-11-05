//! The car repair model against another BPSim simulator's published results (L-SIM, in the
//! BPSim Implementer's Guide's `Car Repair Process v2.0.0_results.bpsim`).
//!
//! The reference files are downloaded by `cargo xtask fetch-bpsim`, never committed; without
//! them the test skips (CI sets `STOCHCRAFT_REQUIRE_BPSIM=1`, which makes a missing file fail).
//!
//! The model routes by probability (whether the estimate is accepted; whether an inspection finds
//! a new issue) and by condition (repair while `noOfIssues > 0`), with its repair loop inside a
//! sub-process that is simulated in line. Its tasks take no time. L-SIM reports three
//! replications of scenario S1, so its counts are compared with the spread StochCraft's own
//! replications give for a mean of three.
//!
//! It also gathers the evidence for decision D8 (ROADMAP.md): the first arrival at time 0, as
//! L-SIM does, or after the first interval, as the specification reads and StochCraft does.

#![allow(clippy::panic)] // A reference file that cannot be read fails the test.

use std::path::{Path, PathBuf};

use stochcraft_blocks::{Report, compile};
use stochcraft_io::{read_scenario, xml};
use stochcraft_random::Rng;

const ARRIVE: &str = "_51BDA265-2FF5-40CB-B68D-1FBF9DAAA74C";
const START_REPAIRS: &str = "_FA77D7D8-9C02-4006-942A-B003AEDA5E3C";
const REPAIR_ISSUE: &str = "_071B5D0A-5225-4E55-9105-8D15169DAC96";
const NOT_ACCEPTED: &str = "_87E52808-857B-45B0-93E8-F6D7AC0B4B3E";
const REPAIRS_DONE: &str = "_B67579E4-E038-4D9D-AA46-A3F9628CADA1";
const LEAVES: &str = "_4CFC937C-2E42-4671-859E-42E710931963";

fn required() -> bool {
    std::env::var("STOCHCRAFT_REQUIRE_BPSIM").is_ok_and(|v| v == "1")
}

fn examples() -> Option<PathBuf> {
    fn find(dir: &Path, depth: u32) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if !path.is_dir() || entry.file_name() == "__MACOSX" {
                continue;
            }
            if entry.file_name() == "Examples" {
                return Some(path);
            }
            if depth > 0
                && let Some(found) = find(&path, depth - 1)
            {
                return Some(found);
            }
        }
        None
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bpsim/guide");
    let found = find(&dir, 2);
    if found.is_none() {
        assert!(
            !required(),
            "the BPSim reference models are missing (STOCHCRAFT_REQUIRE_BPSIM=1); run `cargo xtask fetch-bpsim`"
        );
        eprintln!("skipped: run `cargo xtask fetch-bpsim` to include this test");
    }
    found
}

/// L-SIM's per-replication counts for `element` in result scenario `scenario`.
fn lsim(results: &[u8], scenario: &str, element: &str) -> Vec<f64> {
    let doc = xml::parse(results).unwrap_or_else(|e| panic!("{e}"));
    doc.root
        .descendants()
        .into_iter()
        .filter(|s| s.local() == "Scenario" && s.attr("id").as_deref() == Some(scenario))
        .flat_map(|s| s.descendants())
        .filter(|e| e.attr("elementRef").as_deref() == Some(element))
        .flat_map(|e| e.descendants())
        .filter(|v| v.attr("result").as_deref() == Some("count"))
        .filter_map(|v| v.attr("value")?.parse().ok())
        .collect()
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

fn sd(v: &[f64]) -> f64 {
    let m = mean(v);
    let var = v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() as f64 - 1.0);
    var.sqrt()
}

fn entered(r: &Report, id: &str) -> f64 {
    r.element(id).map_or(f64::NAN, |e| e.entered as f64)
}

#[test]
fn car_repair_matches_lsim() {
    let Some(dir) = examples() else { return };
    let model =
        std::fs::read(dir.join("Car Repair Process v2.0.0.bpmn")).unwrap_or_else(|e| panic!("{e}"));
    let results = std::fs::read(dir.join("Car Repair Process v2.0.0_results.bpsim"))
        .unwrap_or_else(|e| panic!("{e}"));
    let loaded = read_scenario(&model, Some("S1")).unwrap_or_else(|e| panic!("{e}"));
    let compiled = compile(&loaded.model).unwrap_or_else(|d| panic!("{d:?}"));
    const REPS: u64 = 400;
    for at_zero in [false, true] {
        let c = compiled.clone().first_arrival_at_zero(at_zero);
        let reports: Vec<Report> = (0..REPS).map(|r| c.run(Rng::new(1 + r))).collect();
        eprintln!(
            "first arrival {}:",
            if at_zero {
                "at time 0"
            } else {
                "after the first interval"
            }
        );
        for (label, id) in [
            ("arrivals", ARRIVE),
            ("estimate not accepted", NOT_ACCEPTED),
            ("repairs started", START_REPAIRS),
            ("issues repaired", REPAIR_ISSUE),
            ("repairs completed", REPAIRS_DONE),
            ("customers leave", LEAVES),
        ] {
            let ours: Vec<f64> = reports.iter().map(|r| entered(r, id)).collect();
            let theirs = lsim(&results, "S1_results", id);
            assert_eq!(theirs.len(), 3, "{label}: {theirs:?}");
            // L-SIM's mean of three against the spread of a mean of three of ours: within
            // three standard errors of both (a two-sample check that rarely fails by chance).
            let se = (sd(&ours).powi(2) / 3.0 + sd(&theirs).powi(2) / 3.0).sqrt();
            let gap = mean(&theirs) - mean(&ours);
            eprintln!(
                "  {label:>22}: StochCraft {:8.2} (sd {:6.2}), L-SIM {:8.2} {theirs:?}, gap {:6.2} = {:5.2} se",
                mean(&ours),
                sd(&ours),
                mean(&theirs),
                gap,
                if se > 0.0 { gap / se } else { 0.0 }
            );
            if id == ARRIVE {
                // Deterministic: PT24M over PT60H.
                let want = if at_zero { 151.0 } else { 150.0 };
                assert!(ours.iter().all(|&n| n == want), "{label}: {ours:?}");
                continue;
            }
            assert!(
                gap.abs() <= 3.0 * se,
                "{label} (first arrival at 0: {at_zero}): ours {} vs L-SIM {}",
                mean(&ours),
                mean(&theirs)
            );
        }
    }
}
