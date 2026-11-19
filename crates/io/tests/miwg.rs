//! The BPMN Model Interchange Working Group's reference models (BPMN 2.0 interchange tests, with
//! no simulation data): https://github.com/bpmn-miwg/bpmn-miwg-test-suite, CC BY 3.0, ROADMAP.md
//! D14. Only its `Reference/` folder is used, and it is not committed: `cargo xtask fetch-miwg`
//! fetches it at a pinned commit into `target/miwg/suite/`.
//!
//! Locally these tests skip with a message when the files are missing. In CI, where
//! `STOCHCRAFT_REQUIRE_MIWG=1`, missing files fail the tests instead, so they can never silently
//! stop running.
//!
//! The models use much of BPMN that StochCraft does not simulate (sub-processes, message flows,
//! data objects, event types), so the tests ask for no more than that every file reads without a
//! panic, that the XML survives writing and that saving is stable. Files that cannot be read are
//! listed in [`CANNOT_READ`], each with the reason.

#![allow(clippy::panic)] // A reference file that cannot be read fails the test.

use std::path::{Path, PathBuf};

use stochcraft_io::{Severity, read, write_new, xml};

/// The reference models that StochCraft cannot read at all (not well-formed, or not a BPMN 2.0
/// definitions document), by file name, with why. A file listed here must still fail to read, so
/// the list never goes stale.
const CANNOT_READ: &[(&str, &str)] = &[];

fn required() -> bool {
    std::env::var("STOCHCRAFT_REQUIRE_MIWG").is_ok_and(|v| v == "1")
}

/// The reference `.bpmn` files, sorted.
fn models() -> Option<Vec<PathBuf>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/miwg/suite/Reference");
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "bpmn"))
                .collect()
        })
        .unwrap_or_default();
    if out.is_empty() {
        assert!(
            !required(),
            "the BPMN MIWG reference models are missing (STOCHCRAFT_REQUIRE_MIWG=1); run `cargo xtask fetch-miwg`"
        );
        eprintln!("skipped: run `cargo xtask fetch-miwg` to include these tests");
        return None;
    }
    out.sort();
    // The suite has 22 reference models at the pinned commit; a checkout with far fewer is wrong.
    assert!(
        out.len() >= 20,
        "only {} files in {}",
        out.len(),
        dir.display()
    );
    Some(out)
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", name(p)))
}

fn allowed(p: &Path) -> Option<&'static str> {
    CANNOT_READ
        .iter()
        .find(|(file, _)| *file == name(p))
        .map(|(_, why)| *why)
}

#[test]
fn every_reference_model_reads_or_is_listed_with_its_reason() {
    let Some(paths) = models() else { return };
    for p in &paths {
        let result = read(&bytes(p));
        match (allowed(p), result) {
            (None, Ok(l)) => {
                // Reading is lenient: what StochCraft cannot use is a note, not a failure, and the
                // model it makes is checkable.
                let errors = l.notes.iter().filter(|n| n.severity == Severity::Error);
                eprintln!(
                    "{}: {} elements, {} error notes, {} diagnostics",
                    name(p),
                    l.model.elements.len(),
                    errors.count(),
                    l.model.validate().len()
                );
            }
            (None, Err(e)) => panic!(
                "{}: cannot be read: {e}; if StochCraft cannot read this file, list it in CANNOT_READ with the reason",
                name(p)
            ),
            (Some(why), Ok(_)) => panic!("{}: is listed as unreadable ({why}) but reads", name(p)),
            (Some(_), Err(_)) => {}
        }
    }
}

#[test]
fn the_xml_of_every_reference_model_survives_writing() {
    let Some(paths) = models() else { return };
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
    }
}

#[test]
fn saving_a_reference_model_is_stable() {
    let Some(paths) = models() else { return };
    let mut unstable = Vec::new();
    for p in &paths {
        let b = bytes(p);
        let Ok(l) = read(&b) else { continue };
        // Unchanged, the file saves as the very same bytes, whatever in it StochCraft ignores.
        assert_eq!(
            l.opened.save(&l.model).ok().as_deref(),
            Some(b.as_slice()),
            "{}: an unchanged model did not save as the original bytes",
            name(p)
        );
        // As a new file: what is written reads back, and writing that again changes nothing.
        let first = write_new(&l.model);
        let again = read(&first).unwrap_or_else(|e| panic!("{}: written file: {e}", name(p)));
        let second = write_new(&again.model);
        if second != first {
            unstable.push(format!(
                "{}: {}",
                name(p),
                first_difference(&first, &second)
            ));
        }
    }
    assert!(
        unstable.is_empty(),
        "writing, reading and writing again gave a different file:\n{}",
        unstable.join("\n")
    );
}

/// The first line where `a` and `b` differ, both shown.
fn first_difference(a: &[u8], b: &[u8]) -> String {
    let (a, b) = (String::from_utf8_lossy(a), String::from_utf8_lossy(b));
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    for i in 0..la.len().max(lb.len()) {
        let (x, y) = (la.get(i).copied(), lb.get(i).copied());
        if x != y {
            return format!(
                "line {}: {:?} then {:?}",
                i + 1,
                x.unwrap_or("<end>"),
                y.unwrap_or("<end>")
            );
        }
    }
    String::from("same lines, different line endings")
}
