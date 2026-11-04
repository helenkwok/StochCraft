//! StochCraft's own hand-written model files (tests/models/), read as other tools write them.

#![allow(clippy::panic)] // A test file that cannot be read stops the test.

use stochcraft_blocks::{Metric, compile, summarise};
use stochcraft_io::{Format, Severity, read_scenario};
use stochcraft_model::{Dist, ElementKind, Model, TimeUnit};
use stochcraft_random::Rng;

fn load(name: &str, scenario: Option<&str>) -> stochcraft_io::Loaded {
    let path = format!("{}/../../tests/models/{name}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    read_scenario(&bytes, scenario).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn kind<'a>(m: &'a Model, id: &str) -> &'a ElementKind {
    match m.elements.iter().find(|e| e.id.as_str() == id) {
        Some(e) => &e.kind,
        None => panic!("no element {id}"),
    }
}

#[test]
fn bpmn_written_by_another_tool() {
    let l = load("mm1.bpmn", None);
    assert_eq!(l.opened.format, Format::Bpmn);
    assert_eq!(l.opened.scenario.as_deref(), Some("base"));
    assert_eq!(l.opened.scenarios.len(), 2);
    // Warnings at most (the set-up time is noted as not simulated yet).
    assert!(
        l.notes.iter().all(|n| n.severity < Severity::Error),
        "{:#?}",
        l.notes
    );
    let m = &l.model;
    assert_eq!(m.name, "M/M/1");
    assert_eq!(
        kind(m, "arrive"),
        &ElementKind::Source {
            interarrival: Dist::Exponential { mean: 80.0 / 60.0 },
            limit: None
        }
    );
    assert_eq!(
        kind(m, "serve"),
        &ElementKind::Server {
            capacity: 1,
            service: Dist::Exponential { mean: 1.0 }
        }
    );
    assert_eq!(kind(m, "leave"), &ElementKind::Sink);
    let x = &m.experiment;
    assert_eq!(
        (x.time_unit, x.replications, x.seed),
        (TimeUnit::Minutes, 16, 2026)
    );
    // BPSim's duration (21,000) includes the warm-up (1,000).
    assert_eq!((x.warmup, x.duration), (1_000.0, 20_000.0));
    assert!(m.validate().is_empty(), "{:?}", m.validate());
}

#[test]
fn inherited_scenario_changes_only_what_it_says() {
    let l = load("mm1.bpmn", Some("two-clerks"));
    assert_eq!(
        kind(&l.model, "serve"),
        &ElementKind::Server {
            capacity: 2,
            service: Dist::Exponential { mean: 1.0 }
        }
    );
    assert_eq!(l.model.experiment.replications, 16);
}

#[test]
fn a_file_runs_and_matches_queueing_theory() {
    // λ = 0.75, μ = 1 per minute: W = 1 / (μ − λ) = 4 minutes, utilisation 0.75.
    let l = load("mm1.bpmn", None);
    let compiled = compile(&l.model).unwrap_or_else(|d| panic!("{d:?}"));
    let x = &l.model.experiment;
    let reports: Vec<_> = (0..u64::from(x.replications))
        .map(|r| compiled.run(Rng::new(x.seed + r)))
        .collect();
    let s = summarise(&reports);
    let w = s.get("leave", Metric::MeanTimeInSystem);
    let u = s.get("serve", Metric::Utilisation);
    eprintln!("W {w:?}\nU {u:?}");
    assert!(w.is_some_and(|w| w.contains(4.0)), "{w:?}");
    assert!(u.is_some_and(|u| u.contains(0.75)), "{u:?}");
}

#[test]
fn xpdl_with_bpsim_1() {
    let l = load("tandem.xpdl", None);
    assert_eq!(l.opened.format, Format::Xpdl);
    assert!(
        l.notes.iter().any(|n| n.message.contains("BPSim 1.0")),
        "{:#?}",
        l.notes
    );
    assert!(
        l.notes.iter().all(|n| n.severity < Severity::Warning),
        "{:#?}",
        l.notes
    );
    let m = &l.model;
    // Minutes converted to the scenario's hours; k written as 2.0; a duration in hours.
    assert_eq!(
        kind(m, "in"),
        &ElementKind::Source {
            interarrival: Dist::Uniform {
                min: 10.0 / 60.0,
                max: 20.0 / 60.0
            },
            limit: Some(40)
        }
    );
    assert_eq!(
        kind(m, "weld"),
        &ElementKind::Server {
            capacity: 3,
            service: Dist::Erlang { mean: 0.2, k: 2 }
        }
    );
    assert_eq!(
        kind(m, "cool"),
        &ElementKind::Delay {
            duration: Dist::Constant(0.75)
        }
    );
    let x = &m.experiment;
    assert_eq!(
        (x.time_unit, x.replications, x.seed, x.warmup, x.duration),
        (TimeUnit::Hours, 10, 7, 0.0, 24.0)
    );
    assert!(m.validate().is_empty(), "{:?}", m.validate());
}

#[test]
fn unknown_content_is_kept_when_saving_unchanged() {
    let path = format!("{}/../../tests/models/mm1.bpmn", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_default();
    let l = load("mm1.bpmn", None);
    let saved = l.opened.save(&l.model).unwrap_or_default();
    assert_eq!(saved, bytes);
    assert!(String::from_utf8_lossy(&saved).contains("other:colour"));
}

#[test]
fn not_a_model() {
    for bad in [&b"<html/>"[..], b"not xml", b""] {
        assert!(stochcraft_io::read(bad).is_err());
    }
}

#[test]
fn truncated_files_never_panic() {
    let path = format!("{}/../../tests/models/mm1.bpmn", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_default();
    assert!(!bytes.is_empty());
    for end in 0..bytes.len() {
        // Every prefix is either rejected or read; neither may panic.
        let _ = stochcraft_io::read(&bytes[..end]);
        let mut corrupted = bytes.clone();
        corrupted[end] = b'<';
        let _ = stochcraft_io::read(&corrupted);
    }
}
