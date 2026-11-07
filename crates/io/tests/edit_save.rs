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
        Element::new(
            id(new),
            "Walk to the exit",
            ElementKind::Delay {
                duration: Dist::Uniform { min: 0.5, max: 1.5 },
            },
        ),
    );
    let c = m
        .connections
        .iter()
        .position(|c| c.from.as_str() == from && c.to.as_str() == to)
        .unwrap_or(0);
    m.connections.remove(c);
    m.connections.insert(
        c,
        Connection::new(id(&format!("{new}-in")), id(from), id(new)),
    );
    m.connections.insert(
        c + 1,
        Connection::new(id(&format!("{new}-out")), id(new), id(to)),
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
            queue_capacity: None,
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
            queue_capacity: None,
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
fn values_tied_to_calendars_survive_edits() {
    // A processing time and a property, each with a value for the "lunch" calendar besides the
    // default one. StochCraft ignores calendar values on reading (M2.6), so the read-back check
    // cannot see them: saving must keep them by itself.
    let original = text(&file("mm1.bpmn"))
        .replace(
            r#"<bpsim:NegativeExponentialDistribution mean="1"/>"#,
            r#"<bpsim:NegativeExponentialDistribution mean="1"/>
                <bpsim:NegativeExponentialDistribution mean="3" validFor="lunch"/>"#,
        )
        .replace(
            r#"<bpsim:SetupTime><bpsim:FloatingParameter value="0"/></bpsim:SetupTime>
            </bpsim:TimeParameters>"#,
            r#"<bpsim:SetupTime><bpsim:FloatingParameter value="0"/></bpsim:SetupTime>
            </bpsim:TimeParameters>
            <bpsim:PropertyParameters>
              <bpsim:Property name="size"><bpsim:NumericParameter value="9" validFor="lunch"/><bpsim:NumericParameter value="1"/></bpsim:Property>
            </bpsim:PropertyParameters>"#,
        );
    assert!(original.contains(r#"mean="3" validFor="lunch""#) && original.contains("\"size\""));
    let loaded = read_scenario(original.as_bytes(), None).unwrap_or_else(|e| panic!("{e}"));
    let mut m = loaded.model.clone();
    let serve = element(&mut m, "serve");
    if let ElementKind::Server { service, .. } = &mut serve.kind {
        *service = Dist::Exponential { mean: 2.0 };
    }
    serve.properties[0].value = stochcraft_model::PropertyValue::Number(4.0);
    let saved = loaded.opened.save(&m).unwrap_or_else(|e| panic!("{e}"));
    let t = text(&saved);
    for kept in [
        r#"<bpsim:NegativeExponentialDistribution mean="3" validFor="lunch"/>"#,
        r#"<bpsim:NumericParameter value="9" validFor="lunch"/>"#,
    ] {
        assert!(t.contains(kept), "{kept} lost:\n{t}");
    }
    let back = read_scenario(&saved, None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.model, m);
}

#[test]
fn edited_saves_keep_the_byte_order_mark_and_doctype() {
    let original = format!(
        "\u{feff}{}",
        text(&file("mm1.bpmn")).replace(
            "<semantic:definitions ",
            "<!DOCTYPE semantic:definitions [\n  <!ENTITY tool \"StochCraft\">\n]>\n<semantic:definitions ",
        )
    );
    let loaded = read_scenario(original.as_bytes(), None).unwrap_or_else(|e| panic!("{e}"));
    let mut m = loaded.model.clone();
    m.experiment.replications = 3;
    let saved = loaded.opened.save(&m).unwrap_or_else(|e| panic!("{e}"));
    let t = text(&saved);
    assert!(t.starts_with("\u{feff}<?xml "), "{t}");
    assert!(
        t.contains("<!DOCTYPE semantic:definitions [\n  <!ENTITY tool \"StochCraft\">\n]>\n<semantic:definitions "),
        "{t}"
    );
    assert_eq!(changed_lines(original.as_bytes(), &saved), 2, "{t}");
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

fn set_queue(m: &mut Model, at: &str, q: Option<u32>) {
    if let ElementKind::Server { queue_capacity, .. } = &mut element(m, at).kind {
        *queue_capacity = q;
    }
}

/// A finite buffer is a StochCraft vendor extension in the task's element parameters: adding
/// one adds one line, changing it changes that line, and removing it gives back the file as it
/// was, in BPMN with BPSim 2.0 and in XPDL with BPSim 1.0.
#[test]
fn finite_buffers_save_as_a_vendor_extension() {
    for (name, task, ns) in [
        ("mm1.bpmn", "serve", "bpsim"),
        ("tandem.xpdl", "weld", "b1"),
    ] {
        let original = file(name);
        let (three, n) = edit_and_save(name, None, |m| set_queue(m, task, Some(3)));
        assert_eq!(n, 1, "{name}:\n{}", diff(&original, &three));
        let t = text(&three);
        let ext = format!(
            r#"<{ns}:VendorExtension name="stochcraft:queueCapacity" xmlns:stochcraft="https://helenkwok.github.io/StochCraft/bpsim/1" stochcraft:value="3"/>"#
        );
        assert!(t.contains(&ext), "{name}:\n{t}");
        let loaded = read_scenario(&three, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut model = loaded.model.clone();
        set_queue(&mut model, task, Some(0));
        let zero = loaded
            .opened
            .save(&model)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(changed_lines(&three, &zero), 2, "{}", diff(&three, &zero));
        assert!(text(&zero).contains(r#"stochcraft:value="0""#));
        set_queue(&mut model, task, None);
        let none = loaded
            .opened
            .save(&model)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(text(&none), text(&original), "{name}");
    }
}

/// A server added to a file, or a delay made a server, takes its buffer along; one made a delay
/// leaves it behind.
#[test]
fn new_and_changed_servers_carry_their_buffer() {
    let (saved, _) = edit_and_save("tandem.xpdl", None, |m| {
        element(m, "cool").kind = ElementKind::Server {
            capacity: 2,
            service: Dist::Constant(1.0),
            queue_capacity: Some(4),
        };
        set_queue(m, "weld", Some(1));
    });
    let t = text(&saved);
    assert!(t.contains(r#"stochcraft:value="4""#), "{t}");
    let (back, _) = edit_and_save("tandem.xpdl", None, |m| {
        set_queue(m, "weld", Some(2));
    });
    let loaded = read_scenario(&back, None).unwrap_or_else(|e| panic!("{e}"));
    let mut model = loaded.model.clone();
    element(&mut model, "weld").kind = ElementKind::Delay {
        duration: Dist::Constant(1.0),
    };
    let delay = loaded.opened.save(&model).unwrap_or_else(|e| panic!("{e}"));
    assert!(!text(&delay).contains("queueCapacity"), "{}", text(&delay));
}

/// What the reader makes of buffers it cannot use: a bad value, a buffer on a task without a
/// resource, and StochCraft extensions this version does not know (kept all the same).
#[test]
fn buffers_that_cannot_be_used_are_reported() {
    let original = text(&file("mm1.bpmn"));
    let ext = |name: &str, value: &str| {
        format!(
            r#"<bpsim:VendorExtension name="{name}" xmlns:sc="https://helenkwok.github.io/StochCraft/bpsim/1" sc:value="{value}"/>
          </bpsim:ElementParameters>"#
        )
    };
    let with = |name: &str, value: &str| {
        original.replacen("</bpsim:ElementParameters>", &ext(name, value), 1)
    };
    // Any prefix is read; the value is whole and at least 0.
    let first = original
        .split("elementRef=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap_or_default()
        .to_owned();
    for (name, value, want) in [
        ("stochcraft:queueCapacity", "-1", "must be a whole number"),
        ("stochcraft:queueCapacity", "two", "must be a whole number"),
        ("stochcraft:breakdowns", "1", "not known to this version"),
    ] {
        let bytes = with(name, value);
        let l = read_scenario(bytes.as_bytes(), None).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            l.notes.iter().any(|n| n.message.contains(want)),
            "{name}={value} on {first}: {:?}",
            l.notes
        );
        assert!(l.model.elements.iter().all(|e| !matches!(
            e.kind,
            ElementKind::Server {
                queue_capacity: Some(_),
                ..
            }
        )));
    }
}

#[test]
fn a_buffer_on_a_task_without_a_resource_is_ignored() {
    let original = text(&file("tandem.xpdl"));
    let bytes = original.replacen(
        r#"<b1:ElementParameters elementRef="cool">"#,
        r#"<b1:ElementParameters elementRef="cool"><b1:VendorExtension name="stochcraft:queueCapacity" xmlns:stochcraft="https://helenkwok.github.io/StochCraft/bpsim/1" stochcraft:value="2"/>"#,
        1,
    );
    let l = read_scenario(bytes.as_bytes(), None).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        l.notes.iter().any(|n| n.at.as_deref() == Some("cool")
            && n.message.contains("only to a task with a resource")),
        "{:?}",
        l.notes
    );
    assert!(matches!(
        l.model
            .elements
            .iter()
            .find(|e| e.id.as_str() == "cool")
            .map(|e| &e.kind),
        Some(ElementKind::Delay { .. })
    ));
}
