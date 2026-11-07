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
use stochcraft_model::{
    Connection, Dist, Element, ElementId, ElementKind, Experiment, Model, Property, PropertyType,
    PropertyValue, TimeUnit,
};

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
fn reference_properties_and_expressions_read() {
    // The guide's car repair models set a count of issues on arrival and change it as a car is
    // repaired (in a sub-process, left out until it is supported), and branch on it. Every
    // expression in their properties and branch conditions parses; every property that reaches
    // the model is valid.
    let Some(paths) = models() else { return };
    let (mut with_properties, mut expressions) = (0, 0);
    for p in &paths {
        let b = bytes(p);
        let doc = xml::parse(&b).unwrap_or_else(|e| panic!("{}: {e}", name(p)));
        let Some(data) = bpsim::find_data(&doc.root) else {
            continue;
        };
        for (id, _) in bpsim::scenario_list(data) {
            let mut notes = Vec::new();
            let Some(s) = bpsim::read_scenario(data, Some(&id), &mut notes) else {
                continue;
            };
            for (element, params) in &s.params {
                for (key, param) in params {
                    if !(key.contains("/Property/") || key.ends_with("/Condition")) {
                        continue;
                    }
                    for v in param
                        .elements()
                        .filter(|v| v.local() == "ExpressionParameter")
                    {
                        let src = v.attr("value").unwrap_or_default();
                        let parsed = stochcraft_model::expr::parse(&src);
                        assert!(
                            parsed.is_ok(),
                            "{} [{id}] {element} {key}: `{src}`: {parsed:?}",
                            name(p)
                        );
                        expressions += 1;
                    }
                }
            }
        }
        let Ok(first) = read(&b) else { continue };
        for (id, _) in &first.opened.scenarios {
            let l = read_scenario(&b, Some(id)).unwrap_or_else(|e| panic!("{} {id}: {e}", name(p)));
            let props: Vec<&Property> = l
                .model
                .elements
                .iter()
                .flat_map(|e| &e.properties)
                .collect();
            if props.is_empty() {
                continue;
            }
            with_properties += 1;
            let at = format!("{} [{id}]", name(p));
            let property_problems: Vec<String> = l
                .model
                .validate()
                .iter()
                .filter(|d| {
                    matches!(
                        d.problem.code(),
                        "bad_expression"
                            | "unknown_property"
                            | "bad_property"
                            | "property_type_conflict"
                            | "property_value_type"
                    )
                })
                .map(ToString::to_string)
                .collect();
            assert!(property_problems.is_empty(), "{at}: {property_problems:#?}");
            let types = l.model.property_types();
            assert!(
                types.values().all(|t| t.is_numeric()),
                "{at}: a count of issues is a number: {types:?}"
            );
        }
    }
    assert!(with_properties > 0, "no reference model sets properties");
    assert!(expressions >= 20, "only {expressions} expressions found");
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

/// Checks the BPSim data in a written file against the official schema: the errors, if any.
fn schema_errors(written: &[u8], label: &str, xsd: &Path) -> Option<String> {
    let doc = xml::parse(written).unwrap_or_else(|e| panic!("{label}: {e}"));
    let mut data = bpsim::find_data(&doc.root)
        .unwrap_or_else(|| panic!("{label}: no BPSimData"))
        .clone();
    if data.ns.as_deref() != Some(NS2) {
        return None;
    }
    // The prefixes are declared on the file's root; the extracted part declares them itself.
    for (n, v) in doc.root.attrs.iter().rev() {
        if n.starts_with("xmlns") && !data.attrs.iter().any(|(d, _)| d == n) {
            data.attrs.insert(0, (n.clone(), v.clone()));
        }
    }
    let out =
        std::env::temp_dir().join(format!("stochcraft-xsd-{}-{label}.xml", std::process::id()));
    let text = xml::write(&xml::Document::new(data));
    std::fs::write(&out, text).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    // The BPSim schema with StochCraft's vendor extension schema next to it, which declares the
    // values of its `VendorExtension`s (the BPSim schema checks them strictly).
    let ours = Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/stochcraft-bpsim-1.xsd");
    let both = std::env::temp_dir().join(format!(
        "stochcraft-xsd-{}-{label}-schemas.xsd",
        std::process::id()
    ));
    let wrapper = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <xs:import namespace="{NS2}" schemaLocation="{}"/>
  <xs:import namespace="{}" schemaLocation="{}"/>
</xs:schema>
"#,
        xsd.display(),
        bpsim::STOCHCRAFT_NS,
        ours.display()
    );
    std::fs::write(&both, wrapper).unwrap_or_else(|e| panic!("{}: {e}", both.display()));
    let result = Command::new("xmllint")
        .arg("--noout")
        .arg("--schema")
        .arg(&both)
        .arg(&out)
        .output();
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&both);
    let result = result.unwrap_or_else(|e| panic!("xmllint: {e}"));
    (!result.status.success()).then(|| String::from_utf8_lossy(&result.stderr).into_owned())
}

/// Validates the BPSim data in a written file against the official schema.
fn validate(written: &[u8], label: &str, xsd: &Path) {
    if let Some(errors) = schema_errors(written, label, xsd) {
        panic!("{label}: BPSim data is not schema-valid:\n{errors}");
    }
}

fn label(p: &Path, scenario: &str) -> String {
    format!("{}-{scenario}", name(p))
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

/// The edits tried on every reference model: each must save, read back as the edited model and
/// keep a schema-valid file valid.
/// A named change to a model.
type Edit = (&'static str, Box<dyn Fn(&mut Model)>);

fn edits() -> Vec<Edit> {
    let first = |m: &Model, f: fn(&ElementKind) -> bool| m.elements.iter().position(|e| f(&e.kind));
    vec![
        (
            "move and place shapes",
            Box::new(|m: &mut Model| {
                // Move every shape the file has, and place the first element without one.
                for b in m.layout.values_mut() {
                    b.x += 40.0;
                    b.y += 30.5;
                }
                let placed = m.placed();
                if let Some(e) = m.elements.iter().find(|e| !m.layout.contains_key(&e.id))
                    && let Some(b) = placed.get(&e.id)
                {
                    m.layout.insert(e.id.clone(), *b);
                }
            }),
        ),
        (
            "experiment",
            Box::new(|m: &mut Model| {
                m.experiment.replications += 1;
                m.experiment.seed = 99;
                m.experiment.duration += 60.0;
                m.experiment.warmup += 30.0;
            }),
        ),
        (
            "names",
            Box::new(|m: &mut Model| {
                m.name = format!("{} (edited)", m.name);
                if let Some(e) = m.elements.first_mut() {
                    e.name = "Renamed & checked".into();
                }
            }),
        ),
        (
            "times",
            Box::new(move |m: &mut Model| {
                for e in &mut m.elements {
                    match &mut e.kind {
                        ElementKind::Server { service: d, .. }
                        | ElementKind::SharedTask { service: d, .. }
                        | ElementKind::Delay { duration: d } => {
                            *d = Dist::Triangular {
                                min: 1.0,
                                mode: 2.0,
                                max: 5.0,
                            };
                        }
                        ElementKind::Source {
                            interarrival,
                            limit,
                        } => {
                            *interarrival = Dist::Exponential { mean: 3.0 };
                            *limit = Some(25);
                        }
                        ElementKind::Sink
                        | ElementKind::ExclusiveGateway { .. }
                        | ElementKind::ParallelGateway => {}
                    }
                }
            }),
        ),
        (
            "add",
            Box::new(move |m: &mut Model| {
                // A new delay on the first connection (or on its own if there is none).
                let new = ElementId::unchecked("stochcraft-added");
                let at = first(m, |k| matches!(k, ElementKind::Source { .. })).map_or(0, |i| i + 1);
                m.elements.insert(
                    at,
                    Element::new(
                        new.clone(),
                        "Added",
                        ElementKind::Delay {
                            duration: Dist::Constant(2.0),
                        },
                    ),
                );
                if let Some(c) = m.connections.first().cloned() {
                    m.connections[0].to = new.clone();
                    m.connections.insert(
                        1,
                        Connection::new(ElementId::unchecked("stochcraft-added-out"), new, c.to),
                    );
                }
            }),
        ),
        (
            "remove",
            Box::new(move |m: &mut Model| {
                if let Some(i) = first(m, |k| matches!(k, ElementKind::Sink)) {
                    let id = m.elements.remove(i).id;
                    m.connections.retain(|c| c.from != id && c.to != id);
                    m.layout.remove(&id);
                }
            }),
        ),
        (
            "server becomes delay",
            Box::new(move |m: &mut Model| {
                if let Some(i) = first(m, |k| matches!(k, ElementKind::Server { .. })) {
                    m.elements[i].kind = ElementKind::Delay {
                        duration: Dist::Constant(1.5),
                    };
                }
            }),
        ),
        (
            "properties",
            Box::new(move |m: &mut Model| {
                // Change every property, drop the last of each element's, and give the first
                // source a new one (properties read back in name order).
                for e in &mut m.elements {
                    for p in &mut e.properties {
                        p.value = match &p.value {
                            PropertyValue::Expression(x) => {
                                PropertyValue::Expression(format!("({x}) * 1"))
                            }
                            PropertyValue::Number(n) => PropertyValue::Number(n + 1.0),
                            _ => PropertyValue::Number(2.0),
                        };
                    }
                    if e.properties.len() > 1 {
                        e.properties.pop();
                    }
                }
                if let Some(i) = first(m, |k| matches!(k, ElementKind::Source { .. })) {
                    let props = &mut m.elements[i].properties;
                    props.push(Property {
                        name: "stochcraftTier".into(),
                        ty: Some(PropertyType::String),
                        value: PropertyValue::Text("gold".into()),
                    });
                    props.sort_by(|a, b| a.name.cmp(&b.name));
                }
            }),
        ),
        (
            "delay becomes server",
            Box::new(move |m: &mut Model| {
                if let Some(i) = first(m, |k| matches!(k, ElementKind::Delay { .. })) {
                    m.elements[i].kind = ElementKind::Server {
                        capacity: 2,
                        service: Dist::Constant(1.5),
                        queue_capacity: None,
                    };
                }
            }),
        ),
    ]
}

/// Whether scenario `child` inherits, directly or not, from `ancestor`.
fn inherits_from(file: &[u8], child: &str, ancestor: &str) -> bool {
    let doc = xml::parse(file).unwrap_or_else(|e| panic!("{e}"));
    let Some(data) = bpsim::find_data(&doc.root) else {
        return false;
    };
    let parent = |id: &str| {
        data.elements()
            .find(|s| s.attr("id").as_deref() == Some(id))
            .and_then(|s| s.attr("inherits"))
    };
    let mut at = parent(child);
    for _ in 0..32 {
        match at {
            Some(p) if p == ancestor => return true,
            Some(p) => at = parent(&p),
            None => return false,
        }
    }
    false
}

#[test]
fn edits_save_into_every_reference_model_and_scenario() {
    let Some(paths) = models() else { return };
    let xsd = bpsim_dir().join("BPSim-2.0.xsd");
    let check_schema = xsd.exists() && Command::new("xmllint").arg("--version").output().is_ok();
    if !check_schema {
        missing("the BPSim 2.0 XML Schema or xmllint is missing");
    }
    let (mut saves, mut checked) = (0, 0);
    for p in &paths {
        let b = bytes(p);
        let Ok(first) = read(&b) else { continue };
        for (scenario, _) in &first.opened.scenarios {
            let l =
                read_scenario(&b, Some(scenario)).unwrap_or_else(|e| panic!("{}: {e}", name(p)));
            let valid_before =
                check_schema && schema_errors(&b, &label(p, scenario), &xsd).is_none();
            for (what, edit) in edits() {
                let mut m = l.model.clone();
                edit(&mut m);
                if m == l.model {
                    continue;
                }
                let at = format!("{} [{scenario}] {what}", name(p));
                let saved = l.opened.save(&m).unwrap_or_else(|e| panic!("{at}: {e}"));
                let back =
                    read_scenario(&saved, Some(scenario)).unwrap_or_else(|e| panic!("{at}: {e}"));
                // A file with sub-processes keeps flows in several places, so connections are
                // compared by id (saving checks the order of each element's outgoing ones).
                let by_id = |m: &Model| {
                    let mut m = m.clone();
                    m.connections.sort_by(|a, b| a.id.cmp(&b.id));
                    m
                };
                // Elements added without a shape are given one in the file.
                let mut want = by_id(&m);
                for (id, b) in &back.model.layout {
                    want.layout.entry(id.clone()).or_insert(*b);
                }
                assert_eq!(by_id(&back.model), want, "{at}");
                assert_eq!(back.opened.scenarios, l.opened.scenarios, "{at}");
                // Every other scenario reads as it did.
                for (other, _) in l.opened.scenarios.iter().filter(|(o, _)| o != scenario) {
                    let before = read_scenario(&b, Some(other)).map(|x| x.model);
                    let after = read_scenario(&saved, Some(other)).map(|x| x.model);
                    // Removed and added elements are gone from (or new in) every scenario, and a
                    // scenario that inherits from the edited one sees its edits.
                    if matches!(what, "experiment" | "times" | "names")
                        && !inherits_from(&b, other, scenario)
                    {
                        assert_eq!(
                            after.ok().map(|m| m.experiment),
                            before.ok().map(|m| m.experiment),
                            "{at}: scenario {other} changed"
                        );
                    }
                }
                if valid_before {
                    checked += 1;
                    if let Some(errors) = schema_errors(&saved, &label(p, scenario), &xsd) {
                        panic!("{at}: the edit made a schema-valid file invalid:\n{errors}");
                    }
                }
                saves += 1;
            }
        }
    }
    eprintln!("{saves} edited saves, {checked} of them checked against the schema");
    assert!(saves > 50, "{saves} saves");
    assert!(
        !check_schema || checked > 0,
        "no file was schema-valid to begin with"
    );
}

/// The technical support models share their agents between tasks: they read as shared pools,
/// and a pool's quantity, a shared task's needs and its priority save back into the file.
#[test]
fn shared_pools_read_and_save_into_the_reference_models() {
    let Some(paths) = models() else { return };
    let mut shared = 0;
    for p in &paths {
        let b = bytes(p);
        let Ok(l) = read(&b) else { continue };
        if l.model.resources.is_empty() {
            continue;
        }
        shared += 1;
        let mut m = l.model.clone();
        if let Some(r) = m.resources.first_mut() {
            r.quantity += 2;
        }
        if let Some(ElementKind::SharedTask {
            needs, priority, ..
        }) = m
            .elements
            .iter_mut()
            .map(|e| &mut e.kind)
            .find(|k| matches!(k, ElementKind::SharedTask { .. }))
        {
            *priority = 2.0;
            if let Some(stochcraft_model::Need::Resource { quantity, .. }) = needs.first_mut() {
                *quantity = 2;
            }
        }
        let saved = l.opened.save(&m);
        let Ok(saved) = saved else {
            panic!("{}: {saved:?}", name(p))
        };
        assert_eq!(read(&saved).map(|x| x.model).ok(), Some(m), "{}", name(p));
    }
    assert!(shared > 0, "no reference model shares a resource");
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
    let mut previous = m.buffered_server(
        "cut",
        2,
        Dist::Gamma {
            shape: 2.5,
            scale: 0.4,
        },
        6,
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
    // Routing: a probability split, a fork and join, and a condition with a default.
    let check = m.exclusive("check");
    let scrap = m.sink("scrap");
    let split = m.parallel("split");
    let (left, right) = (
        m.delay("left", Dist::Constant(1.0)),
        m.delay("right", Dist::Constant(2.0)),
    );
    let join = m.parallel("join");
    let triage = m.exclusive("triage");
    let out = m.sink("exit");
    let other = m.sink("other-exit");
    m.connect(previous, check);
    m.route(check, split, Some(0.9), None);
    m.route(check, scrap, Some(0.1), None);
    m.connect(split, left);
    m.connect(split, right);
    m.connect(left, join);
    m.connect(right, join);
    // Shared pools: by id with an alternative by roles, several units, and a priority.
    m.pool("crew", 3, &["fitter"]);
    m.pool("seniors", 2, &["fitter", "lead"]);
    let fit = m.shared_task(
        "fit",
        Dist::Exponential { mean: 1.5 },
        vec![
            stochcraft_model::Need::Resource {
                id: stochcraft_model::ElementId::unchecked("crew"),
                quantity: 2,
            },
            stochcraft_model::Need::Roles {
                roles: vec!["fitter".into(), "lead".into()],
                quantity: 1,
            },
        ],
        3.5,
    );
    m.connect(join, fit);
    m.connect(fit, triage);
    m.route(triage, out, None, Some("1 < 2 and not(false())"));
    m.route(triage, other, None, None);
    m.default_route(triage, other);
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
                validate(&write_new(&l.model), &label(p, id), &xsd);
            }
        }
    }
}
