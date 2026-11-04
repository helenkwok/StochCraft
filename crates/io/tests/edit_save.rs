//! Saving edits into opened files: each edit must read back as the edited model, keep the rest of
//! the file as it was, and change only the lines it needs to.

#![allow(clippy::panic)] // A test file that cannot be read stops the test.

use std::collections::BTreeMap;

use stochcraft_io::{SaveError, read_scenario};
use stochcraft_model::{Connection, Dist, Element, ElementId, ElementKind, Model, TimeUnit};

fn file(name: &str) -> Vec<u8> {
    let path = format!("{}/../../tests/models/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn id(s: &str) -> ElementId {
    ElementId::unchecked(s)
}

fn element<'a>(m: &'a mut Model, i: &str) -> &'a mut Element {
    m.elements
        .iter_mut()
        .find(|e| e.id.as_str() == i)
        .unwrap_or_else(|| panic!("no element {i}"))
}

/// Lines in one text and not the other (as multisets): how much of the file an edit touched.
fn changed_lines(a: &[u8], b: &[u8]) -> usize {
    let count = |t: &[u8]| {
        let mut m: BTreeMap<String, i64> = BTreeMap::new();
        for l in String::from_utf8_lossy(t).lines() {
            *m.entry(l.to_owned()).or_default() += 1;
        }
        m
    };
    let (ca, cb) = (count(a), count(b));
    let mut keys: Vec<&String> = ca.keys().chain(cb.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.iter()
        .map(|k| {
            (ca.get(*k).copied().unwrap_or(0) - cb.get(*k).copied().unwrap_or(0)).unsigned_abs()
                as usize
        })
        .sum()
}

/// Opens `name` (with `scenario`), applies `edit`, saves, and checks the result. Returns the
/// saved bytes and the number of changed lines.
fn edit_and_save(
    name: &str,
    scenario: Option<&str>,
    edit: impl Fn(&mut Model),
) -> (Vec<u8>, usize) {
    let original = file(name);
    let loaded = read_scenario(&original, scenario).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut model = loaded.model.clone();
    edit(&mut model);
    let saved = loaded
        .opened
        .save(&model)
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    let back = read_scenario(&saved, scenario).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(back.model, model, "{name}");
    // The other scenarios are all still there.
    assert_eq!(back.opened.scenarios, loaded.opened.scenarios, "{name}");
    // A second save of the reopened, unchanged model gives the same bytes.
    assert_eq!(
        back.opened.save(&back.model).ok().as_deref(),
        Some(&saved[..])
    );
    let n = changed_lines(&original, &saved);
    (saved, n)
}

/// The lines only in `a` (-) or only in `b` (+).
fn diff(a: &[u8], b: &[u8]) -> String {
    let (ta, tb) = (text(a), text(b));
    let (la, lb): (Vec<&str>, Vec<&str>) = (ta.lines().collect(), tb.lines().collect());
    let mut out = String::new();
    for l in &la {
        if !lb.contains(l) {
            out.push_str(&format!("- {l}\n"));
        }
    }
    for l in &lb {
        if !la.contains(l) {
            out.push_str(&format!("+ {l}\n"));
        }
    }
    out
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn parameter_edits_change_only_their_own_lines() {
    for (name, edit, at_most) in [
        (
            "mm1.bpmn",
            Box::new(|m: &mut Model| {
                if let ElementKind::Server { service, .. } = &mut element(m, "serve").kind {
                    *service = Dist::Triangular {
                        min: 0.5,
                        mode: 1.0,
                        max: 2.0,
                    };
                }
            }) as Box<dyn Fn(&mut Model)>,
            2,
        ),
        (
            "mm1.bpmn",
            Box::new(|m: &mut Model| {
                if let ElementKind::Server { capacity, .. } = &mut element(m, "serve").kind {
                    *capacity = 3;
                }
            }),
            2,
        ),
        (
            "mm1.bpmn",
            Box::new(|m: &mut Model| {
                if let ElementKind::Source { interarrival, .. } = &mut element(m, "arrive").kind {
                    *interarrival = Dist::Exponential { mean: 2.0 };
                }
            }),
            2,
        ),
        (
            "mm1.bpmn",
            Box::new(|m: &mut Model| element(m, "serve").name = "Serve & smile".into()),
            2,
        ),
        (
            "mm1.bpmn",
            Box::new(|m: &mut Model| m.name = "Counter".into()),
            2,
        ),
        (
            "mm1.bpmn",
            Box::new(|m: &mut Model| m.experiment.replications = 4),
            2,
        ),
        (
            "mm1.bpmn",
            Box::new(|m: &mut Model| m.experiment.seed = u64::MAX),
            2,
        ),
        (
            "mm1.bpmn",
            Box::new(|m: &mut Model| m.experiment.duration = 500.0),
            2,
        ),
        // The warm-up is part of BPSim's duration, so both change.
        (
            "mm1.bpmn",
            Box::new(|m: &mut Model| m.experiment.warmup = 30.0),
            4,
        ),
        (
            "tandem.xpdl",
            Box::new(|m: &mut Model| {
                if let ElementKind::Source { limit, .. } = &mut element(m, "in").kind {
                    *limit = None;
                }
            }),
            1,
        ),
        (
            "tandem.xpdl",
            Box::new(|m: &mut Model| {
                if let ElementKind::Delay { duration } = &mut element(m, "cool").kind {
                    *duration = Dist::Constant(0.5);
                }
            }),
            2,
        ),
        (
            "tandem.xpdl",
            Box::new(|m: &mut Model| {
                if let ElementKind::Server { capacity, .. } = &mut element(m, "weld").kind {
                    *capacity = 1;
                }
            }),
            2,
        ),
        (
            "tandem.xpdl",
            Box::new(|m: &mut Model| element(m, "weld").name = "Weld seams".into()),
            2,
        ),
        // A new Warmup parameter takes three lines, and the duration changes.
        (
            "tandem.xpdl",
            Box::new(|m: &mut Model| m.experiment.warmup = 2.0),
            5,
        ),
    ] {
        let (saved, n) = edit_and_save(name, None, edit);
        assert!(
            n <= at_most,
            "{name}: {n} lines changed:\n{}",
            diff(&file(name), &saved)
        );
    }
}

#[test]
fn an_inherited_scenario_gets_its_own_value() {
    // "two-clerks" inherits everything but the clerk quantity from "base".
    let (saved, _) = edit_and_save("mm1.bpmn", Some("two-clerks"), |m| {
        if let ElementKind::Server { service, .. } = &mut element(m, "serve").kind {
            *service = Dist::Constant(0.9);
        }
        m.experiment.replications = 5;
    });
    // The base scenario is unchanged.
    let base = read_scenario(&saved, Some("base")).unwrap_or_else(|e| panic!("{e}"));
    let original = read_scenario(&file("mm1.bpmn"), Some("base")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(base.model, original.model);
}

#[test]
fn unknown_content_survives_edits() {
    let (saved, _) = edit_and_save("mm1.bpmn", None, |m| m.experiment.replications = 3);
    let t = text(&saved);
    for kept in [
        "<!-- M/M/1, written by hand",
        r##"<other:colour value="#ffcc00"/>"##,
        "<bpsim:ResultRequest>mean</bpsim:ResultRequest>",
        r#"<bpsim:SetupTime><bpsim:FloatingParameter value="0"/></bpsim:SetupTime>"#,
        r#"<bpsim:NegativeExponentialDistribution mean="80" timeUnit="s"/>"#,
    ] {
        assert!(t.contains(kept), "{kept} lost:\n{t}");
    }
}

fn add_delay_between(m: &mut Model, new: &str, from: &str, to: &str) {
    let i = m
        .elements
        .iter()
        .position(|e| e.id.as_str() == from)
        .unwrap_or(0)
        + 1;
    m.elements.insert(
        i,
        Element {
            id: id(new),
            name: "Walk to the exit".into(),
            kind: ElementKind::Delay {
                duration: Dist::Uniform { min: 0.5, max: 1.5 },
            },
        },
    );
    let c = m
        .connections
        .iter()
        .position(|c| c.from.as_str() == from && c.to.as_str() == to)
        .unwrap_or(0);
    m.connections.remove(c);
    m.connections.insert(
        c,
        Connection {
            id: id(&format!("{new}-in")),
            from: id(from),
            to: id(new),
        },
    );
    m.connections.insert(
        c + 1,
        Connection {
            id: id(&format!("{new}-out")),
            from: id(new),
            to: id(to),
        },
    );
}

#[test]
fn blocks_and_connections_can_be_added_and_removed() {
    for (name, from, to) in [
        ("mm1.bpmn", "serve", "leave"),
        ("tandem.xpdl", "cool", "out"),
    ] {
        let (saved, _) = edit_and_save(name, None, |m| add_delay_between(m, "walk", from, to));
        let t = text(&saved);
        assert!(t.contains("walk-in") && t.contains("walk-out"), "{t}");
        // Added elements are indented like their neighbours.
        assert!(
            !t.contains("><semantic:task") && !t.contains("></Activity><Activity"),
            "{t}"
        );

        // Removing a block takes its connections, shapes and parameters with it.
        let (saved, _) = edit_and_save(name, None, |m| {
            m.elements.retain(|e| e.id.as_str() != from);
            m.connections
                .retain(|c| c.from.as_str() != from && c.to.as_str() != from);
        });
        let t = text(&saved);
        assert!(!t.contains(&format!("elementRef=\"{from}\"")), "{t}");
    }
    // A server added as a new block gets a resource of its own.
    let (saved, _) = edit_and_save("mm1.bpmn", None, |m| {
        add_delay_between(m, "pay", "serve", "leave");
        element(m, "pay").kind = ElementKind::Server {
            capacity: 2,
            service: Dist::Exponential { mean: 0.5 },
        };
    });
    assert!(text(&saved).contains("pay_resource"));
    // Kinds change: a server becomes a delay and back, a sink becomes a source's target.
    edit_and_save("mm1.bpmn", None, |m| {
        element(m, "serve").kind = ElementKind::Delay {
            duration: Dist::Constant(1.0),
        };
    });
    edit_and_save("tandem.xpdl", None, |m| {
        element(m, "cool").kind = ElementKind::Server {
            capacity: 2,
            service: Dist::Constant(0.75),
        };
    });
    edit_and_save("tandem.xpdl", None, |m| {
        element(m, "weld").kind = ElementKind::Delay {
            duration: Dist::Constant(0.2),
        };
    });
}

#[test]
fn new_names_in_a_latin_1_file_are_character_references() {
    let original = text(&file("mm1.bpmn")).replace("encoding=\"UTF-8\"", "encoding=\"ISO-8859-1\"");
    let loaded = read_scenario(original.as_bytes(), None).unwrap_or_else(|e| panic!("{e}"));
    let mut m = loaded.model.clone();
    element(&mut m, "serve").name = "Café".into();
    let saved = loaded.opened.save(&m).unwrap_or_else(|e| panic!("{e}"));
    assert!(text(&saved).contains("Caf&#233;") && !text(&saved).contains("Café"));
    let back = read_scenario(&saved, None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.model, m);
}

#[test]
fn edits_that_cannot_be_saved_say_why() {
    // Changing the time unit would change what the values with their own units mean.
    let loaded = read_scenario(&file("mm1.bpmn"), None).unwrap_or_else(|e| panic!("{e}"));
    let mut m = loaded.model.clone();
    m.experiment.time_unit = TimeUnit::Hours;
    let r = loaded.opened.save(&m);
    assert!(matches!(r, Err(SaveError::Unsupported(_))), "{r:?}");
    // Reordering elements would change the model's random streams.
    let mut m = loaded.model.clone();
    m.elements.reverse();
    assert!(loaded.opened.save(&m).is_err());
}
