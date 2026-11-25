//! Every `.dmn` file StochCraft writes is valid against OMG's DMN 1.6 XML Schema (fetched by
//! `cargo xtask fetch-dmn`, never committed): a new file with every part a table can have, names
//! left empty included, and an edited file.
//!
//! Locally this skips with a message when the schema (or `xmllint`) is missing. In CI, where
//! `STOCHCRAFT_REQUIRE_BPSIM=1`, a missing schema fails the test instead.

#![allow(clippy::panic)] // A schema or file that cannot be read fails the test.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use stochcraft_io::dmn::{read_dmn, save_dmn, write_dmn};
use stochcraft_model::dmn::{
    Aggregation, Decision, Definitions, HitPolicy, Input, Output, Rule, Table,
};

fn required() -> bool {
    std::env::var("STOCHCRAFT_REQUIRE_BPSIM").is_ok_and(|v| v == "1")
}

fn missing(what: &str) {
    assert!(
        !required(),
        "{what} (STOCHCRAFT_REQUIRE_BPSIM=1); run `cargo xtask fetch-dmn`"
    );
    eprintln!("skipped: {what}; run `cargo xtask fetch-dmn` to include this test");
}

fn schema() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/dmn/DMN16.xsd")
}

/// A folder of its own for one check, removed when the check ends, however it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let d = std::env::temp_dir().join(format!(
                "stochcraft-dmn-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&d) {
                Ok(()) => return Self(d),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("{}: {e}", d.display()),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What xmllint says is wrong with `bytes` against the DMN 1.6 schema (whose imports are read
/// from beside it, never the network).
fn schema_errors(bytes: &[u8], label: &str) -> Option<String> {
    let scratch = Scratch::new();
    let doc = scratch.0.join(format!("{label}.dmn"));
    std::fs::write(&doc, bytes).unwrap_or_else(|e| panic!("{e}"));
    let result = Command::new("xmllint")
        .arg("--noout")
        .arg("--nonet")
        .arg("--schema")
        .arg(schema())
        .arg(&doc)
        .output()
        .unwrap_or_else(|e| panic!("xmllint: {e}"));
    (!result.status.success()).then(|| {
        format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        )
    })
}

/// Two decisions: one with every part a table can have, one whose names are empty.
fn definitions() -> Definitions {
    let full = Decision {
        id: "chute".into(),
        name: "Chute".into(),
        variable: Some("chute".into()),
        table: Table {
            id: Some("t1".into()),
            hit_policy: HitPolicy::Priority,
            aggregation: None,
            inputs: vec![
                Input {
                    id: Some("i1".into()),
                    label: Some("Size".into()),
                    expression: "size".into(),
                    type_ref: Some("number".into()),
                    values: Some("[0..100]".into()),
                },
                Input {
                    expression: "class".into(),
                    ..Input::default()
                },
            ],
            outputs: vec![Output {
                id: Some("o1".into()),
                label: Some("Chute".into()),
                name: Some("chute".into()),
                type_ref: Some("string".into()),
                values: Some("\"a\", \"b\"".into()),
                default: Some("\"b\"".into()),
            }],
            annotations: vec!["Why".into()],
            rules: vec![
                Rule {
                    id: Some("r1".into()),
                    description: Some("small & light".into()),
                    inputs: vec!["< 10".into(), "\"A\"".into()],
                    outputs: vec!["\"a\"".into()],
                    annotations: vec!["fits a tote".into()],
                },
                Rule {
                    inputs: vec!["-".into(), "-".into()],
                    outputs: vec!["\"b\"".into()],
                    annotations: vec![String::new()],
                    ..Rule::default()
                },
            ],
        },
    };
    let unnamed = Decision {
        id: "count".into(),
        name: String::new(),
        variable: None,
        table: Table {
            hit_policy: HitPolicy::Collect,
            aggregation: Some(Aggregation::Count),
            inputs: vec![Input {
                expression: "size".into(),
                ..Input::default()
            }],
            outputs: vec![Output {
                name: Some("matches".into()),
                ..Output::default()
            }],
            rules: vec![Rule {
                inputs: vec!["> 0".into()],
                outputs: vec!["1".into()],
                ..Rule::default()
            }],
            ..Table::default()
        },
    };
    Definitions {
        id: "sorting".into(),
        name: String::new(),
        namespace: "https://example.org/sorting".into(),
        decisions: vec![full, unnamed],
    }
}

#[test]
fn every_dmn_file_written_is_valid_dmn_1_6() {
    if !schema().exists() {
        return missing("OMG's DMN 1.6 XML Schema is not in target/dmn");
    }
    if Command::new("xmllint").arg("--version").output().is_err() {
        return missing("xmllint is not installed (libxml2-utils)");
    }
    let defs = definitions();
    let new = write_dmn(&defs);
    if let Some(errors) = schema_errors(new.as_bytes(), "new") {
        panic!("a new file is not valid DMN 1.6:\n{errors}\n{new}");
    }
    let read = read_dmn(new.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(read, defs);
    // Edited: a rule added and one removed, a column added, a default taken away.
    let mut after = read.clone();
    let t = &mut after.decisions[0].table;
    t.rules.remove(0);
    t.rules.insert(
        0,
        Rule {
            inputs: vec!["> 50".into(), "-".into()],
            outputs: vec!["\"a\"".into()],
            annotations: vec!["big".into()],
            ..Rule::default()
        },
    );
    t.outputs[0].default = None;
    let edited = save_dmn(new.as_bytes(), &read, &after).unwrap_or_else(|e| panic!("{e}"));
    if let Some(errors) = schema_errors(&edited, "edited") {
        panic!(
            "an edited file is not valid DMN 1.6:\n{errors}\n{}",
            String::from_utf8_lossy(&edited)
        );
    }
}
