//! The BPSim reference models published by WfMC with the Implementer's Guide 2.0, and the BPSim
//! 2.0 XML Schema. They carry no licence of their own, so they are not committed:
//! `cargo xtask fetch-bpsim` downloads them (pinned SHA-256) into `target/bpsim/`.
//!
//! Locally these tests skip with a message when the files (or `xmllint`) are missing. In CI,
//! where `STOCHCRAFT_REQUIRE_BPSIM=1`, missing files fail the tests instead, so they can never
//! silently stop running.

#![allow(clippy::panic)] // A reference file that cannot be read fails the test.

use std::path::{Path, PathBuf};
use std::process::Command;

use stochcraft_blocks::Builder;
use stochcraft_io::bpsim::{self, NS2};
use stochcraft_io::{read, read_scenario, write_new, xml};
use stochcraft_model::{Dist, Experiment, Model, TimeUnit};

fn required() -> bool {
    std::env::var("STOCHCRAFT_REQUIRE_BPSIM").is_ok_and(|v| v == "1")
}

fn missing(what: &str) {
    assert!(
        !required(),
        "{what} (STOCHCRAFT_REQUIRE_BPSIM=1); run `cargo xtask fetch-bpsim`"
    );
    eprintln!("skipped: {what}; run `cargo xtask fetch-bpsim` to include these tests");
}

fn bpsim_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bpsim")
}

fn find_examples(dir: &Path, depth: u32) -> Option<PathBuf> {
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() || entry.file_name() == "__MACOSX" {
            continue;
        }
        if entry.file_name() == "Examples" {
            return Some(path);
        }
        if depth > 0
            && let Some(found) = find_examples(&path, depth - 1)
        {
            return Some(found);
        }
    }
    None
}

/// The reference files with the given extension, sorted.
fn files(ext: &str) -> Option<Vec<PathBuf>> {
    let Some(dir) = find_examples(&bpsim_dir().join("guide"), 2) else {
        missing("the BPSim reference models are not in target/bpsim");
        return None;
    };
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == ext))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    assert!(!out.is_empty(), "no .{ext} files in {}", dir.display());
    Some(out)
}

fn models() -> Option<Vec<PathBuf>> {
    let mut all = files("bpmn")?;
    all.extend(files("xpdl")?);
    Some(all)
}

fn bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[test]
fn every_reference_model_and_scenario_reads() {
    let Some(paths) = models() else { return };
    assert_eq!(paths.len(), 14, "7 models, each as BPMN and XPDL");
    for p in &paths {
        let b = bytes(p);
        let first = read(&b).unwrap_or_else(|e| panic!("{}: {e}", name(p)));
        assert!(
            !first.opened.scenarios.is_empty(),
            "{}: no scenarios",
            name(p)
        );
        assert!(!first.model.elements.is_empty(), "{}: no elements", name(p));
        for (id, _) in &first.opened.scenarios {
            let l = read_scenario(&b, Some(id)).unwrap_or_else(|e| panic!("{} {id}: {e}", name(p)));
            assert_eq!(l.opened.scenario.as_deref(), Some(id.as_str()));
        }
    }
}

#[test]
fn unchanged_saves_are_byte_identical_and_the_xml_tree_round_trips() {
    let Some(mut paths) = models() else { return };
    paths.extend(files("bpsim").unwrap_or_default());
    for p in &paths {
        let b = bytes(p);
        let doc = xml::parse(&b).unwrap_or_else(|e| panic!("{}: {e}", name(p)));
        let again =
            xml::parse(xml::write(&doc).as_bytes()).unwrap_or_else(|e| panic!("{}: {e}", name(p)));
        assert_eq!(
            again.root,
            doc.root,
            "{}: the XML tree changed on writing",
            name(p)
        );
        if p.extension().is_some_and(|e| e != "bpsim") {
            let l = read(&b).unwrap_or_else(|e| panic!("{}: {e}", name(p)));
            assert_eq!(l.opened.save(&l.model).ok(), Some(b), "{}", name(p));
        }
    }
}

#[test]
fn bpmn_and_xpdl_versions_give_the_same_model() {
    let Some(bpmn) = files("bpmn") else { return };
    for b in &bpmn {
        let x = b.with_extension("xpdl");
        let (lb, lx) = (read(&bytes(b)), read(&bytes(&x)));
        let (Ok(mut mb), Ok(mut mx)) = (lb.map(|l| l.model), lx.map(|l| l.model)) else {
            panic!("{}: could not read both versions", name(b));
        };
        // Model names come from different places in the two formats.
        mb.name.clear();
        mx.name.clear();
        let key = |m: &Model| {
            let mut c: Vec<(String, String)> = m
                .connections
                .iter()
                .map(|c| (c.from.to_string(), c.to.to_string()))
                .collect();
            c.sort();
            let mut e: Vec<String> = m
                .elements
                .iter()
                .map(|e| format!("{} {:?}", e.id, e.kind))
                .collect();
            e.sort();
            (e, c, m.experiment.clone())
        };
        assert_eq!(key(&mb), key(&mx), "{}: BPMN and XPDL disagree", name(b));
    }
}

#[test]
fn result_scenarios_from_another_simulator_parse() {
    let Some(results) = files("bpsim") else {
        return;
    };
    assert_eq!(results.len(), 3);
    for p in &results {
        let doc = xml::parse(&bytes(p)).unwrap_or_else(|e| panic!("{}: {e}", name(p)));
        let data =
            bpsim::find_data(&doc.root).unwrap_or_else(|| panic!("{}: no BPSimData", name(p)));
        let scenarios: Vec<_> = data.children_named(&bpsim::NSS, "Scenario").collect();
        assert!(
            scenarios.iter().all(|s| s.attr("result").is_some()),
            "{}: every scenario is a result",
            name(p)
        );
        let values = data
            .descendants()
            .into_iter()
            .filter(|e| e.attr("result").is_some() && e.attr("instance").is_some())
            .count();
        assert!(values > 0, "{}: no per-replication values", name(p));
    }
    // The car repair arrivals: PT24M over PT60H is 150 intervals, and L-SIM counts 151 in every
    // replication, so it fires the start event at time 0 as well (ROADMAP.md, open question).
    let car = results
        .iter()
        .find(|p| name(p).starts_with("Car Repair"))
        .unwrap_or_else(|| panic!("no car repair results"));
    let doc = xml::parse(&bytes(car)).unwrap_or_else(|e| panic!("{e}"));
    let arrivals: Vec<String> = doc
        .root
        .descendants()
        .into_iter()
        .filter(|e| {
            e.attr("elementRef").as_deref() == Some("_51BDA265-2FF5-40CB-B68D-1FBF9DAAA74C")
        })
        .flat_map(|e| e.descendants())
        .filter(|v| v.attr("result").as_deref() == Some("count"))
        .filter_map(|v| v.attr("value"))
        .collect();
    // Two result scenarios (S1 and S2), three replications each.
    assert_eq!(arrivals.len(), 6, "{arrivals:?}");
    assert!(arrivals.iter().all(|v| v == "151"), "{arrivals:?}");
}

/// Validates the BPSim data in a written BPMN file against the official schema.
fn validate(written: &[u8], label: &str, xsd: &Path) {
    let doc = xml::parse(written).unwrap_or_else(|e| panic!("{label}: {e}"));
    let mut data = bpsim::find_data(&doc.root)
        .unwrap_or_else(|| panic!("{label}: no BPSimData"))
        .clone();
    // The prefix is declared on the BPMN root; the extracted part declares it itself.
    data.attrs.insert(0, ("xmlns:bpsim".into(), NS2.into()));
    let out =
        std::env::temp_dir().join(format!("stochcraft-xsd-{}-{label}.xml", std::process::id()));
    let text = xml::write(&xml::Document {
        decl: None,
        prolog: vec![],
        root: data,
    });
    std::fs::write(&out, text).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    let result = Command::new("xmllint")
        .arg("--noout")
        .arg("--schema")
        .arg(xsd)
        .arg(&out)
        .output();
    let _ = std::fs::remove_file(&out);
    let result = result.unwrap_or_else(|e| panic!("xmllint: {e}"));
    assert!(
        result.status.success(),
        "{label}: BPSim data is not schema-valid:\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn showcase() -> Model {
    let mut m = Builder::new();
    let src = m.source(
        "arrivals",
        Dist::Triangular {
            min: 1.0,
            mode: 2.0,
            max: 4.5,
        },
        Some(500),
    );
    let mut previous = m.server(
        "cut",
        2,
        Dist::Gamma {
            shape: 2.5,
            scale: 0.4,
        },
    );
    m.connect(src, previous);
    let delays = [
        Dist::Constant(1.25),
        Dist::Exponential { mean: 0.75 },
        Dist::Uniform { min: 0.5, max: 1.5 },
        Dist::Normal {
            mean: 3.0,
            std_dev: 0.5,
        },
        Dist::LogNormal {
            mean: 2.0,
            std_dev: 0.3,
        },
        Dist::TruncatedNormal {
            mean: 2.0,
            std_dev: 1.0,
            min: 1.0,
            max: 1000.0,
        },
        Dist::Beta {
            shape: 2.0,
            scale: 5.0,
        },
        Dist::Weibull {
            shape: 1.5,
            scale: 2.0,
        },
        Dist::Erlang { mean: 6.0, k: 3 },
        Dist::Poisson { mean: 4.0 },
        Dist::Binomial {
            probability: 0.3,
            trials: 10,
        },
        Dist::User {
            points: vec![(0.2, 1.0), (0.5, 4.0), (0.3, 10.0)],
            discrete: true,
        },
        Dist::User {
            points: vec![(0.0, 0.0), (0.5, 2.0), (0.5, 10.0)],
            discrete: false,
        },
    ];
    for (i, d) in delays.into_iter().enumerate() {
        let step = m.delay(&format!("step-{i}"), d);
        m.connect(previous, step);
        previous = step;
    }
    let out = m.sink("exit");
    m.connect(previous, out);
    m.build(Experiment {
        time_unit: TimeUnit::Hours,
        warmup: 2.5,
        duration: 40.0,
        replications: 7,
        seed: u64::MAX - 3,
    })
}

#[test]
fn everything_stochcraft_writes_is_schema_valid() {
    let xsd = bpsim_dir().join("BPSim-2.0.xsd");
    if !xsd.exists() {
        return missing("the BPSim 2.0 XML Schema is not in target/bpsim");
    }
    if Command::new("xmllint").arg("--version").output().is_err() {
        return missing("xmllint is not installed (libxml2-utils)");
    }
    validate(&write_new(&showcase()), "showcase", &xsd);
    // Each reference model, as StochCraft reads it, written again as a new BPMN file.
    let Some(paths) = models() else { return };
    for p in &paths {
        let b = bytes(p);
        let Ok(first) = read(&b) else { continue };
        for (id, _) in &first.opened.scenarios {
            if let Ok(l) = read_scenario(&b, Some(id)) {
                let label: String = format!("{}-{id}", name(p))
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect();
                validate(&write_new(&l.model), &label, &xsd);
            }
        }
    }
}
