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
    // default one. StochCraft uses calendar values only for arrivals and pool units, so the
    // read-back check cannot see these: saving must keep them by itself.
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
fn removing_a_block_leaves_no_references_to_it() {
    // A lane listing the task, a note attached to it and a timer on its boundary.
    let original = text(&file("mm1.bpmn"))
        .replace(
            r#"<semantic:process id="p1" name="Counter" isExecutable="false">"#,
            r#"<semantic:process id="p1" name="Counter" isExecutable="false">
    <semantic:laneSet id="ls1">
      <semantic:lane id="desk" name="Desk">
        <semantic:flowNodeRef>arrive</semantic:flowNodeRef>
        <semantic:flowNodeRef>serve</semantic:flowNodeRef>
        <semantic:flowNodeRef>leave</semantic:flowNodeRef>
      </semantic:lane>
    </semantic:laneSet>"#,
        )
        .replace(
            r#"<semantic:sequenceFlow id="f2" sourceRef="serve" targetRef="leave"/>"#,
            r#"<semantic:sequenceFlow id="f2" sourceRef="serve" targetRef="leave"/>
    <semantic:boundaryEvent id="late" attachedToRef="serve">
      <semantic:outgoing>f3</semantic:outgoing>
      <semantic:timerEventDefinition id="late-timer"/>
    </semantic:boundaryEvent>
    <semantic:sequenceFlow id="f3" sourceRef="late" targetRef="leave"/>
    <semantic:textAnnotation id="note"><semantic:text>Busy at noon</semantic:text></semantic:textAnnotation>
    <semantic:association id="a1" sourceRef="note" targetRef="serve"/>
    <semantic:association id="a2" sourceRef="note" targetRef="f2"/>"#,
        )
        .replace(
            "<semantic:incoming>f2</semantic:incoming>",
            "<semantic:incoming>f2</semantic:incoming>\n      <semantic:incoming>f3</semantic:incoming>",
        );
    assert!(original.contains("attachedToRef") && original.contains("flowNodeRef"));
    let loaded = read_scenario(original.as_bytes(), None).unwrap_or_else(|e| panic!("{e}"));
    let mut m = loaded.model.clone();
    m.elements.retain(|e| e.id.as_str() != "serve");
    m.connections.clear();
    m.connections
        .push(Connection::new(id("f0"), id("arrive"), id("leave")));
    let saved = loaded.opened.save(&m).unwrap_or_else(|e| panic!("{e}"));
    let t = text(&saved);
    for gone in [
        "<semantic:flowNodeRef>serve</semantic:flowNodeRef>",
        r#"attachedToRef="serve""#,
        r#"targetRef="serve""#,
        r#"id="f3""#,
        "<semantic:incoming>f3</semantic:incoming>",
        r#"targetRef="f2""#,
    ] {
        assert!(!t.contains(gone), "{gone} left behind:\n{t}");
    }
    // What does not refer to the task stays.
    for kept in [
        "<semantic:flowNodeRef>arrive</semantic:flowNodeRef>",
        r#"<semantic:textAnnotation id="note">"#,
    ] {
        assert!(t.contains(kept), "{kept} lost:\n{t}");
    }
    let back = read_scenario(&saved, None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.model, m);

    // A task written again as another kind (an end event) keeps its place in the lane and its
    // note: it is still there.
    let mut m = loaded.model.clone();
    element(&mut m, "serve").kind = ElementKind::Sink;
    m.connections.retain(|c| c.id.as_str() != "f2");
    let saved = loaded.opened.save(&m).unwrap_or_else(|e| panic!("{e}"));
    let t = text(&saved);
    for kept in [
        "<semantic:flowNodeRef>serve</semantic:flowNodeRef>",
        r#"targetRef="serve""#,
    ] {
        assert!(t.contains(kept), "{kept} lost:\n{t}");
    }
}

#[test]
fn new_ids_never_clash_with_the_files_other_ids() {
    let loaded = read_scenario(&file("mm1.bpmn"), None).unwrap_or_else(|e| panic!("{e}"));
    // Ids the file uses for what StochCraft keeps but does not model: the task's own resource,
    // the process, a scenario.
    for taken in ["clerk", "p1", "two-clerks", "defs"] {
        assert!(loaded.opened.id_in_use(taken), "{taken}");
        let mut m = loaded.model.clone();
        add_delay_between(&mut m, taken, "serve", "leave");
        let r = loaded.opened.save(&m);
        assert!(
            matches!(r, Err(SaveError::Unsupported(_))),
            "{taken}: {r:?}"
        );
    }
    // The model's own ids are its to reuse.
    assert!(!loaded.opened.id_in_use("serve") && !loaded.opened.id_in_use("f1"));
    // Ids StochCraft makes up for the file (a new server's resource) avoid the model's new ids.
    let (saved, _) = edit_and_save("mm1.bpmn", None, |m| {
        add_delay_between(m, "pay", "serve", "leave");
        add_delay_between(m, "pay_resource", "pay", "leave");
        element(m, "pay").kind = ElementKind::Server {
            capacity: 1,
            service: Dist::Constant(1.0),
            queue_capacity: None,
        };
    });
    let t = text(&saved);
    assert_eq!(t.matches(r#"id="pay_resource""#).count(), 1, "{t}");
}

#[test]
fn edits_that_cannot_be_saved_say_why() {
    // "two-clerks" takes its values with their own units from "base": changing its time unit
    // would mean rewriting them in "base", which would change that scenario too.
    let loaded =
        read_scenario(&file("mm1.bpmn"), Some("two-clerks")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = loaded.model.clone();
    m.experiment.time_unit = TimeUnit::Hours;
    let r = loaded.opened.save(&m);
    assert!(
        matches!(&r, Err(SaveError::Unsupported(why)) if why.contains("`base`")),
        "{r:?}"
    );
    // Reordering elements would change the model's random streams.
    let mut m = loaded.model.clone();
    m.elements.reverse();
    assert!(loaded.opened.save(&m).is_err());
}

/// Opens `bytes`, changes only the time unit, saves, and checks that the file reads back as the
/// relabelled model.
fn relabel(bytes: &[u8], unit: TimeUnit) -> Vec<u8> {
    let loaded = read_scenario(bytes, None).unwrap_or_else(|e| panic!("{e}"));
    let mut m = loaded.model.clone();
    m.experiment.time_unit = unit;
    let saved = loaded.opened.save(&m).unwrap_or_else(|e| panic!("{e}"));
    let back = read_scenario(&saved, None).unwrap_or_else(|e| panic!("{e}"));
    assert_within_noise(&back.model, &m);
    saved
}

/// Asserts that two models are equal but for numbers within 1e-12 of each other, which a
/// rescaled value may differ by.
fn assert_within_noise(got: &Model, want: &Model) {
    if got == want {
        return;
    }
    let (g, w) = (format!("{got:?}"), format!("{want:?}"));
    let split = |s: &str| -> Vec<String> {
        s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'e'))
            .map(str::to_owned)
            .collect()
    };
    let same = |a: &String, b: &String| match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(x), Ok(y)) => (x - y).abs() <= 1e-12 * x.abs().max(y.abs()),
        _ => a == b,
    };
    let (sg, sw) = (split(&g), split(&w));
    // The text between the numbers must match exactly.
    let words = |s: &str| -> String {
        s.chars()
            .filter(|c| !c.is_ascii_digit() && *c != '.')
            .collect()
    };
    assert!(
        words(&g) == words(&w)
            && sg.len() == sw.len()
            && sg.iter().zip(&sw).all(|(a, b)| same(a, b)),
        "{got:?}\n!=\n{want:?}"
    );
}

/// Changing the time unit relabels the model's times (their numbers stay). Values in the file
/// that name their own unit are rescaled in that unit so they still read as the model says.
#[test]
fn changing_the_time_unit_rescales_values_with_their_own_unit() {
    // mm1: base unit min, arrivals every 80 s, a warm-up of PT1000M, a duration of 21000 (min).
    let (hours, n) = edit_and_save("mm1.bpmn", None, |m| {
        m.experiment.time_unit = TimeUnit::Hours;
    });
    let t = text(&hours);
    for want in [
        r#"baseTimeUnit="hour""#,
        // 80 s became 80 s × 60: 1.333… hours written in seconds.
        r#"<bpsim:NegativeExponentialDistribution mean="4800" timeUnit="s"/>"#,
        // 1000 min became 1000 hours, written in minutes.
        r#"<bpsim:Warmup><bpsim:DurationParameter value="PT60000M"/></bpsim:Warmup>"#,
        // Values in the base unit are relabelled as they are.
        r#"<bpsim:Duration><bpsim:FloatingParameter value="21000"/></bpsim:Duration>"#,
        r#"<bpsim:NegativeExponentialDistribution mean="1"/>"#,
    ] {
        assert!(t.contains(want), "{want} missing:\n{t}");
    }
    // Three lines in "base"; "two-clerks", which inherits from it, gets `baseTimeUnit="min"`
    // and the two rescaled values as they were: eight new lines, four of them the lines "base"
    // had before (which a count of changed lines does not see).
    assert_eq!(n, 10, "{}", diff(&file("mm1.bpmn"), &hours));
    // And back: the original numbers again.
    let t = text(&relabel(&hours, TimeUnit::Minutes));
    for want in [
        r#"<bpsim:NegativeExponentialDistribution mean="80" timeUnit="s"/>"#,
        r#"<bpsim:Warmup><bpsim:DurationParameter value="PT1000M"/></bpsim:Warmup>"#,
    ] {
        assert_eq!(t.matches(want).count(), 2, "{want}:\n{t}");
    }
}

/// A scenario that inherits from the opened one means what it meant before the opened one's
/// time unit changed: it keeps the old unit and the values that were rescaled.
#[test]
fn inheriting_scenarios_keep_their_meaning_when_the_time_unit_changes() {
    let original = file("mm1.bpmn");
    let before = read_scenario(&original, Some("two-clerks")).unwrap_or_else(|e| panic!("{e}"));
    for unit in [TimeUnit::Hours, TimeUnit::Seconds, TimeUnit::Days] {
        let saved = relabel(&original, unit);
        let after = read_scenario(&saved, Some("two-clerks")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(after.model, before.model, "{unit:?}:\n{}", text(&saved));
        // Saving it again changes nothing more.
        let again = relabel(&saved, TimeUnit::Minutes);
        let after = read_scenario(&again, Some("two-clerks")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(after.model, before.model, "{unit:?}:\n{}", text(&again));
    }
    // A scenario that runs for the default 8 hours keeps them.
    let no_duration = text(&original).replace(
        r#"
            <bpsim:Duration><bpsim:FloatingParameter value="21000"/></bpsim:Duration>"#,
        "",
    );
    let no_duration = no_duration.replace("PT1000M", "PT100M");
    let before =
        read_scenario(no_duration.as_bytes(), Some("two-clerks")).unwrap_or_else(|e| panic!("{e}"));
    let saved = relabel(no_duration.as_bytes(), TimeUnit::Hours);
    let after = read_scenario(&saved, Some("two-clerks")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(after.model, before.model, "{}", text(&saved));
}

/// A rescaled value that cannot be written as a decimal reading back as exactly the same float
/// still saves: it reads back within 1e-12 of itself.
#[test]
fn rescaled_values_may_differ_by_float_noise() {
    // 2.7 days under minutes is 3888 min; under hours 3888 h, which is 162 days, but
    // 3888 × (86400 / 3600) does not give back 3888 exactly once read as days.
    let original = text(&file("mm1.bpmn"))
        .replace(r#"mean="80" timeUnit="s""#, r#"mean="2.7" timeUnit="day""#);
    let loaded = read_scenario(original.as_bytes(), None).unwrap_or_else(|e| panic!("{e}"));
    let mut m = loaded.model.clone();
    m.experiment.time_unit = TimeUnit::Hours;
    let saved = loaded.opened.save(&m).unwrap_or_else(|e| panic!("{e}"));
    let back = read_scenario(&saved, None).unwrap_or_else(|e| panic!("{e}"));
    let mean = |m: &Model| match &m
        .elements
        .iter()
        .find(|e| e.id.as_str() == "arrive")
        .map(|e| &e.kind)
    {
        Some(ElementKind::Source {
            interarrival: Dist::Exponential { mean },
            ..
        }) => *mean,
        other => panic!("{other:?}"),
    };
    let (got, want) = (mean(&back.model), mean(&m));
    assert_ne!(got, want, "this case was meant to be inexact");
    assert!((got - want).abs() <= 1e-12 * want.abs(), "{got} {want}");
}

/// Without a change of time unit, values with their own units are left exactly as written.
#[test]
fn other_edits_leave_values_with_their_own_unit_alone() {
    let (saved, _) = edit_and_save("mm1.bpmn", None, |m| {
        m.experiment.replications = 3;
        m.experiment.duration = 100.0;
        if let ElementKind::Server { service, .. } = &mut element(m, "serve").kind {
            *service = Dist::Exponential { mean: 2.0 };
        }
    });
    let t = text(&saved);
    for kept in [
        r#"<bpsim:NegativeExponentialDistribution mean="80" timeUnit="s"/>"#,
        r#"<bpsim:DurationParameter value="PT1000M"/>"#,
        r#"baseTimeUnit="min""#,
    ] {
        assert!(t.contains(kept), "{kept} changed:\n{t}");
    }
}

/// Every kind of value with its own unit: the distributions' time parameters (not their
/// shapes), a user distribution's points, durations of several components, a count
/// distribution, calendar values, and the BPSim 1.0 data of an XPDL file.
#[test]
fn every_value_with_its_own_unit_is_rescaled() {
    let original = text(&file("mm1.bpmn"))
        .replace(
            r#"<bpsim:NegativeExponentialDistribution mean="1"/>"#,
            r#"<bpsim:TriangularDistribution min="30" mode="45" max="90" timeUnit="s"/>
                <bpsim:TriangularDistribution min="1" mode="2" max="3" timeUnit="s" validFor="lunch"/>"#,
        )
        .replace("PT1000M", "PT16H40M")
        .replace(
            r#"<bpsim:Quantity><bpsim:NumericParameter value="1"/></bpsim:Quantity></bpsim:ResourceParameters>
          </bpsim:ElementParameters>
        </bpsim:Scenario>"#,
            r#"<bpsim:Quantity><bpsim:NumericParameter value="1"/></bpsim:Quantity></bpsim:ResourceParameters>
          </bpsim:ElementParameters>
          <bpsim:ElementParameters elementRef="leave">
            <bpsim:PropertyParameters>
              <bpsim:Property name="due"><bpsim:FloatingParameter value="2" timeUnit="day"/></bpsim:Property>
              <bpsim:Property name="wait"><bpsim:GammaDistribution shape="2" scale="6" timeUnit="hour"/></bpsim:Property>
              <bpsim:Property name="pick"><bpsim:UserDistribution timeUnit="s"><bpsim:UserDistributionDataPoint probability="0.5"><bpsim:FloatingParameter value="30"/></bpsim:UserDistributionDataPoint><bpsim:UserDistributionDataPoint probability="0.5"><bpsim:DurationParameter value="PT1M"/></bpsim:UserDistributionDataPoint></bpsim:UserDistribution></bpsim:Property>
              <bpsim:Property name="batch"><bpsim:PoissonDistribution mean="4" timeUnit="min"/></bpsim:Property>
            </bpsim:PropertyParameters>
          </bpsim:ElementParameters>
        </bpsim:Scenario>"#,
        );
    let hours = relabel(original.as_bytes(), TimeUnit::Hours);
    let t = text(&hours);
    for want in [
        r#"<bpsim:TriangularDistribution min="1800" mode="2700" max="5400" timeUnit="s"/>"#,
        r#"<bpsim:TriangularDistribution min="60" mode="120" max="180" timeUnit="s" validFor="lunch"/>"#,
        r#"<bpsim:FloatingParameter value="1800"/>"#,
        r#"<bpsim:DurationParameter value="PT60M"/>"#,
        // Hours stay hours: the gamma's scale grows with the change, its shape does not.
        r#"<bpsim:GammaDistribution shape="2" scale="360" timeUnit="hour"/>"#,
        // Several components become one, in the smallest unit among them.
        r#"<bpsim:DurationParameter value="PT60000M"/>"#,
        r#"<bpsim:FloatingParameter value="120" timeUnit="day"/>"#,
        // A count does not scale: its unit label follows the base unit.
        r#"<bpsim:PoissonDistribution mean="4" timeUnit="hour"/>"#,
    ] {
        assert!(t.contains(want), "{want} missing:\n{t}");
    }
    // Back to minutes: the same numbers as before, the durations each in one unit.
    let t = text(&relabel(&hours, TimeUnit::Minutes));
    for want in [
        r#"<bpsim:TriangularDistribution min="30" mode="45" max="90" timeUnit="s"/>"#,
        r#"<bpsim:DurationParameter value="PT1000M"/>"#,
        r#"<bpsim:FloatingParameter value="2" timeUnit="day"/>"#,
        r#"<bpsim:PoissonDistribution mean="4" timeUnit="min"/>"#,
    ] {
        assert!(
            t.contains(want),
            "{want} missing:
{t}"
        );
    }

    // tandem.xpdl (BPSim 1.0, base unit hour): arrivals uniform 10 to 20 min, welding PT45M,
    // a duration of P1D.
    let minutes = relabel(&file("tandem.xpdl"), TimeUnit::Minutes);
    let t = text(&minutes);
    for want in [
        r#"baseTimeUnit="min""#,
        r#"<b1:UniformDistribution min="0.16666666666666666" max="0.3333333333333333" timeUnit="min"/>"#,
        r#"<b1:DurationParameter value="PT0.75M"/>"#,
        r#"<b1:DurationParameter value="P0.016666666666666666D"/>"#,
    ] {
        assert!(t.contains(want), "{want} missing:\n{t}");
    }
    assert_eq!(
        text(&relabel(&minutes, TimeUnit::Hours)),
        text(&file("tandem.xpdl"))
    );
}

/// A scenario without a duration runs for 8 hours; changing its time unit writes the duration
/// down, since 8 hours would be another number in the new unit.
#[test]
fn a_default_duration_is_written_when_the_time_unit_changes() {
    let original = text(&file("mm1.bpmn")).replace(
        r#"
            <bpsim:Warmup><bpsim:DurationParameter value="PT1000M"/></bpsim:Warmup>
            <bpsim:Duration><bpsim:FloatingParameter value="21000"/></bpsim:Duration>"#,
        "",
    );
    let loaded = read_scenario(original.as_bytes(), None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(loaded.model.experiment.duration, 480.0);
    let t = text(&relabel(original.as_bytes(), TimeUnit::Seconds));
    assert!(
        t.contains(r#"<bpsim:FloatingParameter value="480"/>"#),
        "{t}"
    );
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

/// Moving a shape changes its bounds and the edges to and from it, nothing else; a file with no
/// diagram gets one (BPMN DI, or XPDL graphics) once its shapes are placed.
#[test]
fn moved_and_placed_shapes_save_into_the_diagram() {
    let (saved, n) = edit_and_save("distributions.bpmn", None, |m| {
        let id = m.elements[1].id.clone();
        if let Some(b) = m.layout.get_mut(&id) {
            b.x += 25.0;
            b.y -= 10.0;
        }
    });
    // One Bounds line, and the waypoints of the edges in and out (two each).
    assert!(
        (2..=10).contains(&n),
        "{n} lines:\n{}",
        diff(&file("distributions.bpmn"), &saved)
    );

    for name in ["mm1.bpmn", "tandem.xpdl"] {
        let original = file(name);
        let loaded = read_scenario(&original, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(loaded.model.layout.is_empty(), "{name} has no diagram");
        let (saved, _) = edit_and_save(name, None, |m| m.layout = m.placed());
        let back = read_scenario(&saved, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(back.model.layout.len(), back.model.elements.len(), "{name}");
    }
}
