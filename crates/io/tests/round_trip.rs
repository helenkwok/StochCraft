//! Writing a model and reading it back gives the same model; opened files save unchanged.

use stochcraft_blocks::{BlockId, Builder, Event, Rule};
use stochcraft_io::{Format, read, write_new};
use stochcraft_model::{
    DateTime, Dist, ElementKind, Experiment, Model, Need, Property, PropertyType, PropertyValue,
    Routing, TimeUnit,
};

/// A model using every element kind and every distribution.
pub fn showcase() -> Model {
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
    let cut = m.buffered_server(
        "cut",
        2,
        Dist::Gamma {
            shape: 2.5,
            scale: 0.4,
        },
        6,
    );
    m.breakdown(
        cut,
        stochcraft_blocks::Breakdown {
            time_to_failure: Dist::Exponential { mean: 120.0 },
            repair: Dist::Triangular {
                min: 1.0,
                mode: 2.0,
                max: 6.0,
            },
            interruptible: true,
            hands_on_while_down: true,
        },
    );
    m.transfer(cut, Dist::Uniform { min: 0.5, max: 1.5 });
    m.setup(
        cut,
        stochcraft_blocks::Setup {
            time: Dist::Constant(5.0),
            on_change_of: Some("issues".into()),
            by_change: vec![stochcraft_blocks::Change {
                from: "0".into(),
                to: "2".into(),
                time: Dist::Uniform { min: 6.0, max: 9.0 },
            }],
        },
    );
    m.power(
        cut,
        stochcraft_blocks::Power {
            processing: 12.5,
            idle: 1.25,
            blocked: None,
            failed: Some(0.0),
            off_shift: Some(0.5),
        },
    );
    let mut previous = cut;
    m.connect(src, cut);
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
    // Every kind of property value and every type, in name order (as files are read).
    let props: [(BlockId, &str, Option<PropertyType>, PropertyValue); 9] = [
        (
            src,
            "arrived",
            Some(PropertyType::DateTime),
            PropertyValue::Text("2026-11-04T09:00:00".into()),
        ),
        (
            src,
            "issues",
            Some(PropertyType::Long),
            PropertyValue::Random(Dist::Poisson { mean: 2.0 }),
        ),
        (
            src,
            "label",
            None,
            PropertyValue::Text("Gold & <VIP> \"customer\"".into()),
        ),
        (
            src,
            "patience",
            Some(PropertyType::Duration),
            PropertyValue::Number(1.5),
        ),
        (
            src,
            "rush",
            Some(PropertyType::Boolean),
            PropertyValue::Boolean(false),
        ),
        (
            src,
            "units",
            Some(PropertyType::Long),
            PropertyValue::Number(4.0),
        ),
        (
            cut,
            "issues",
            None,
            PropertyValue::Expression("getProperty('issues') - 1".into()),
        ),
        (
            cut,
            "weight",
            Some(PropertyType::Double),
            PropertyValue::Random(Dist::Uniform { min: 1.0, max: 2.0 }),
        ),
        (
            out,
            "fixed",
            None,
            PropertyValue::Expression(
                "getProperty('issues') <= 0 and getProperty('label') != 'x'".into(),
            ),
        ),
    ];
    for (block, name, ty, value) in props {
        m.property(block, name, ty, value);
    }
    // Calendars: a faster rate on weekday mornings, and no arrivals on Sundays' night shift.
    let at = |d: u32, h: u32| DateTime::new(2016, 1, d, h, 0, 0).unwrap_or(DateTime(0));
    m.calendar(
        "mornings",
        vec![Event {
            start: at(4, 8),
            end: at(4, 12),
            rule: Rule::parse("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE,FR;WKST=SU;COUNT=30").ok(),
        }],
    );
    m.calendar(
        "once",
        vec![Event {
            start: at(5, 22),
            end: at(6, 3),
            rule: None,
        }],
    );
    m.timed_arrivals(
        src,
        vec![
            ("mornings", Dist::Exponential { mean: 0.25 }),
            ("once", Dist::Constant(2.0)),
        ],
        false,
    );
    m.availability(cut, true, &[("once", false)]);
    let mut model = m.build(Experiment {
        time_unit: TimeUnit::Hours,
        warmup: 2.5,
        duration: 40.0,
        replications: 7,
        seed: u64::MAX - 3,
        stop: None,
        start: Some(at(4, 6)),
    });
    model.name = "Showcase & <friends>".into();
    model
}

/// The model with every element drawn where a new file places it.
fn laid_out(mut m: Model) -> Model {
    m.layout = m.placed();
    m
}

#[test]
fn written_models_read_back_identically() {
    let model = showcase();
    let bytes = write_new(&model);
    let loaded = read(&bytes);
    assert!(loaded.is_ok(), "{loaded:?}");
    let Ok(loaded) = loaded else { return };
    assert_eq!(loaded.opened.format, Format::Bpmn);
    assert!(loaded.notes.is_empty(), "{:#?}", loaded.notes);
    assert_eq!(loaded.model, laid_out(model.clone()));
    assert!(model.validate().is_empty());
}

#[test]
fn added_ids_never_collide_with_model_ids() {
    let mut m = Builder::new();
    // Names that are also the ids StochCraft adds to the file.
    let src = m.source("definitions", Dist::Constant(1.0), None);
    let srv = m.server("process", 1, Dist::Constant(0.5));
    let out = m.sink("scenario");
    m.connect(src, srv);
    m.connect(srv, out);
    let model = m.build(Experiment::default());
    let text = String::from_utf8(write_new(&model)).unwrap_or_default();
    let mut ids: Vec<&str> = text
        .split(" id=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate ids in:\n{text}");
    assert_eq!(
        read(text.as_bytes()).map(|l| l.model).ok(),
        Some(laid_out(model))
    );
}

#[test]
fn opened_files_save_unchanged_and_with_edits() {
    let bytes = write_new(&showcase());
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert_eq!(loaded.opened.save(&loaded.model), Ok(bytes.clone()));
    let mut edited = loaded.model.clone();
    edited.experiment.replications += 1;
    edited.experiment.warmup = 0.0;
    if let Some(e) = edited.elements.pop() {
        edited.layout.remove(&e.id);
    }
    edited.connections.pop();
    let saved = loaded.opened.save(&edited);
    let back = saved.as_deref().map(|b| read(b).map(|l| l.model));
    assert_eq!(back, Ok(Ok(edited)));
}

#[test]
fn property_edits_save_into_the_opened_file() {
    let bytes = write_new(&showcase());
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    let mut edited = loaded.model.clone();
    let props = &mut edited.elements[0].properties;
    // Change a value and a type, remove one, add one (kept in name order, as files read).
    if let Some(p) = props.iter_mut().find(|p| p.name == "units") {
        p.value = PropertyValue::Expression("2 * 3".into());
        p.ty = Some(PropertyType::Double);
    }
    props.retain(|p| p.name != "rush");
    props.push(Property {
        name: "zone".into(),
        ty: None,
        value: PropertyValue::Text("Nord".into()),
    });
    // And a property on an element that had none.
    edited.elements[2].properties.push(Property {
        name: "checked".into(),
        ty: Some(PropertyType::Boolean),
        value: PropertyValue::Boolean(true),
    });
    let saved = loaded.opened.save(&edited);
    assert!(saved.is_ok(), "{saved:?}");
    let Ok(saved) = saved else { return };
    assert_eq!(read(&saved).map(|l| l.model).ok(), Some(edited));
    // Only the changed parameters differ: the other properties' text is untouched.
    let text = String::from_utf8_lossy(&saved);
    assert!(text.contains("name=\"issues\" type=\"long\""), "{text}");
    assert!(!text.contains("name=\"rush\""));
}

/// A model with every kind of routing: a probability split, a fork and join, and conditions
/// with a default.
fn routed() -> Model {
    let mut m = Builder::new();
    let src = m.source("arrivals", Dist::Exponential { mean: 2.0 }, None);
    m.property(
        src,
        "issues",
        Some(PropertyType::Long),
        PropertyValue::Random(Dist::Poisson { mean: 2.0 }),
    );
    let check = m.exclusive("check");
    let scrap = m.sink("scrap");
    let split = m.parallel("split");
    let left = m.delay("left", Dist::Constant(1.0));
    let right = m.delay("right", Dist::Constant(2.0));
    let join = m.parallel("join");
    let triage = m.exclusive("triage");
    let repair = m.delay("repair", Dist::Constant(1.0));
    let out = m.sink("exit");
    m.connect(src, check);
    m.route(check, split, Some(0.9), None);
    m.route(check, scrap, Some(0.1), None);
    m.connect(split, left);
    m.connect(split, right);
    m.connect(left, join);
    m.connect(right, join);
    m.connect(join, triage);
    m.route(triage, repair, None, Some("getProperty('issues') > 0"));
    m.route(triage, out, None, None);
    m.default_route(triage, out);
    m.connect(repair, out);
    m.build(Experiment::default())
}

#[test]
fn gateways_and_routing_write_read_and_save() {
    let model = routed();
    assert!(model.validate().is_empty(), "{:#?}", model.validate());
    let bytes = write_new(&model);
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert!(loaded.notes.is_empty(), "{:#?}", loaded.notes);
    assert_eq!(loaded.model, laid_out(model));
    assert_eq!(loaded.opened.save(&loaded.model), Ok(bytes.clone()));

    // Change a probability, a condition and the default; remove a probability.
    let mut edited = loaded.model.clone();
    let at = |m: &Model, from: &str, to: &str| {
        m.connections
            .iter()
            .position(|c| c.from.as_str() == from && c.to.as_str() == to)
    };
    let (Some(to_split), Some(to_scrap), Some(to_repair)) = (
        at(&edited, "check", "split"),
        at(&edited, "check", "scrap"),
        at(&edited, "triage", "repair"),
    ) else {
        unreachable!()
    };
    edited.connections[to_split].probability = Some(0.75);
    edited.connections[to_scrap].probability = None;
    edited.connections[to_repair].condition = Some("getProperty('issues') >= 2".into());
    let repair_flow = edited.connections[to_repair].id.clone();
    if let Some(e) = edited
        .elements
        .iter_mut()
        .find(|e| e.id.as_str() == "triage")
    {
        e.kind = ElementKind::ExclusiveGateway {
            default: Some(repair_flow),
            routing: Routing::Standard,
        };
    }
    let saved = loaded.opened.save(&edited);
    assert!(saved.is_ok(), "{saved:?}");
    let Ok(saved) = saved else { return };
    assert_eq!(read(&saved).map(|l| l.model).ok(), Some(edited));
}

/// Batches, splits and an assembly's bill of materials are written as StochCraft's vendor
/// extensions on plain BPMN tasks and flows, read back as they were, and saved in place when
/// they change.
#[test]
fn batches_and_assemblies_write_read_and_save() {
    let mut m = Builder::new();
    let parts = m.source("parts", Dist::Constant(1.0), None);
    let frames = m.source("frames", Dist::Constant(2.0), None);
    let assemble = m.parallel("assemble");
    let pack = m.batch("pack", 4);
    let ship = m.delay("ship", Dist::Constant(3.0));
    let unpack = m.unbatch("unpack");
    let out = m.sink("out");
    m.connect(parts, assemble);
    m.connect(frames, assemble);
    m.quantity(parts, assemble, 2);
    m.connect(assemble, pack);
    m.connect(pack, ship);
    m.connect(ship, unpack);
    m.connect(unpack, out);
    let model = m.build(Experiment::default());
    assert!(model.validate().is_empty(), "{:#?}", model.validate());
    let bytes = write_new(&model);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("stochcraft:batchSize") && text.contains("stochcraft:quantity"));
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert!(loaded.notes.is_empty(), "{:#?}", loaded.notes);
    assert_eq!(loaded.model, laid_out(model));
    assert_eq!(loaded.opened.save(&loaded.model), Ok(bytes.clone()));

    // A bigger batch, the split becomes a delay, the delay a batch, and the quantity goes.
    let mut edited = loaded.model.clone();
    for e in &mut edited.elements {
        match e.id.as_str() {
            "pack" => e.kind = ElementKind::Batch { size: 6 },
            "unpack" => {
                e.kind = ElementKind::Delay {
                    duration: Dist::Constant(1.0),
                }
            }
            "ship" => e.kind = ElementKind::Batch { size: 2 },
            _ => {}
        }
    }
    for c in &mut edited.connections {
        c.quantity = None;
    }
    let saved = loaded.opened.save(&edited);
    let back = saved.as_deref().map(|b| read(b).map(|l| l.model));
    assert_eq!(back, Ok(Ok(edited)));
}

/// Two presses behind a shortest-queue gateway, each with a minimum time in its buffer:
/// written, read and saved back unchanged; switching the gateway to standard routing and the
/// transfers to ones that hold the sender, in place, drops their extensions and reads back;
/// switching back reads back as the first model.
#[test]
fn shortest_queue_routing_and_minimum_stays_write_read_and_save() {
    let mut m = Builder::new();
    let src = m.source("arrive", Dist::Exponential { mean: 60.0 }, None);
    let handle = m.server("handle", 1, Dist::Constant(25.0));
    let pick = m.shortest_queue("pick");
    let p1 = m.buffered_server("press1", 1, Dist::Constant(175.0), 3);
    let p2 = m.buffered_server("press2", 1, Dist::Constant(176.0), 3);
    let out = m.sink("done");
    m.min_stay(p1, Dist::Constant(32.0));
    m.min_stay(p2, Dist::Constant(32.0));
    m.breakdown(
        p1,
        stochcraft_blocks::Breakdown {
            time_to_failure: Dist::Exponential { mean: 525.0 },
            repair: Dist::Exponential { mean: 73.0 },
            interruptible: true,
            hands_on_while_down: false,
        },
    );
    m.connect(src, handle);
    m.connect(handle, pick);
    m.connect(pick, p1);
    m.connect(pick, p2);
    m.connect(p1, out);
    m.connect(p2, out);
    let model = m.build(Experiment::default());
    assert!(model.validate().is_empty(), "{:#?}", model.validate());
    let bytes = write_new(&model);
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert!(
        text.contains(r#"stochcraft:value="shortestQueue""#),
        "{text}"
    );
    assert_eq!(text.matches(r#"holdsSender="false""#).count(), 2, "{text}");
    assert!(text.contains(r#"handsOnWhileDown="false""#), "{text}");
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert!(loaded.notes.is_empty(), "{:#?}", loaded.notes);
    assert_eq!(loaded.model, laid_out(model));
    assert_eq!(loaded.opened.save(&loaded.model), Ok(bytes.clone()));

    let mut edited = loaded.model.clone();
    for e in &mut edited.elements {
        match &mut e.kind {
            ElementKind::ExclusiveGateway { routing, .. } => *routing = Routing::Standard,
            ElementKind::Server {
                transfer: Some(t), ..
            } => t.holds_sender = true,
            _ => {}
        }
    }
    let saved = loaded.opened.save(&edited);
    assert!(saved.is_ok(), "{saved:?}");
    let Ok(saved) = saved else { return };
    let after = String::from_utf8_lossy(&saved).into_owned();
    assert!(!after.contains("shortestQueue"), "{after}");
    assert!(!after.contains("holdsSender"), "{after}");
    let Ok(back) = read(&saved) else {
        unreachable!()
    };
    assert_eq!(back.model, edited);
    // Back again: the same model (an extension rewritten in place is written on one line).
    let again = back.opened.save(&loaded.model);
    let back = again.as_deref().map(|b| read(b).map(|l| l.model));
    assert_eq!(back, Ok(Ok(loaded.model)));
}

/// A model whose pools BPMN with BPSim alone could not tell apart from tasks' own resources:
/// one pool no task uses, and one that a single task with priority 0 needs one unit of.
fn lone_pools() -> Model {
    let mut m = Builder::new();
    let src = m.source("arrivals", Dist::Exponential { mean: 2.0 }, None);
    m.pool("agents", 2, &[]);
    m.pool("spare", 3, &[]);
    let phone = m.shared_task(
        "phone",
        Dist::Exponential { mean: 1.5 },
        vec![Need::Resource {
            id: stochcraft_model::ElementId::unchecked("agents"),
            quantity: 1,
        }],
        0.0,
    );
    let out = m.sink("exit");
    m.connect(src, phone);
    m.connect(phone, out);
    m.build(Experiment::default())
}

#[test]
fn pools_one_task_or_none_uses_read_back_as_pools() {
    let model = lone_pools();
    let bytes = write_new(&model);
    let loaded = read(&bytes);
    assert!(loaded.is_ok(), "{loaded:?}");
    let Ok(loaded) = loaded else { return };
    assert_eq!(loaded.model, laid_out(model));
}

#[test]
fn files_without_the_pool_marker_keep_pools_with_a_role_or_a_priority() {
    let mut m = Builder::new();
    let src = m.source("arrivals", Dist::Exponential { mean: 2.0 }, None);
    m.pool("agents", 2, &["agent"]);
    m.pool("clerks", 1, &[]);
    m.pool("spare", 3, &["spare"]);
    let phone = m.shared_task(
        "phone",
        Dist::Exponential { mean: 1.5 },
        vec![Need::Resource {
            id: stochcraft_model::ElementId::unchecked("agents"),
            quantity: 1,
        }],
        0.0,
    );
    let desk = m.shared_task(
        "desk",
        Dist::Exponential { mean: 1.0 },
        vec![Need::Resource {
            id: stochcraft_model::ElementId::unchecked("clerks"),
            quantity: 1,
        }],
        2.0,
    );
    let out = m.sink("exit");
    m.connect(src, phone);
    m.connect(phone, desk);
    m.connect(desk, out);
    let model = m.build(Experiment::default());
    let written = String::from_utf8(write_new(&model)).unwrap_or_default();
    // The file as an older StochCraft wrote it: no pool markers.
    let old = without_extension(&written, "stochcraft:pool");
    assert!(!old.contains("stochcraft:pool"));
    let loaded = read(old.as_bytes());
    assert!(loaded.is_ok(), "{loaded:?}");
    let Ok(loaded) = loaded else { return };
    assert_eq!(loaded.model, laid_out(model));
}

/// `xml` without the `<bpsim:VendorExtension …/>` elements whose name is `name`.
fn without_extension(xml: &str, name: &str) -> String {
    let needle = format!("name=\"{name}\"");
    let mut out = String::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&needle) {
        let start = rest[..at].rfind('<').unwrap_or(at);
        let end = rest[at..].find("/>").map_or(rest.len(), |e| at + e + 2);
        out.push_str(&rest[..start]);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// A need by roles tries pools in model order, so the pools must read back in the order they
/// were written, not in id order.
#[test]
fn pools_read_back_in_model_order() {
    let mut m = Builder::new();
    let src = m.source("arrivals", Dist::Exponential { mean: 2.0 }, None);
    m.pool("zeta", 1, &["agent"]);
    m.pool("alpha", 2, &["agent"]);
    let phone = m.shared_task(
        "phone",
        Dist::Exponential { mean: 1.5 },
        vec![Need::Roles {
            roles: vec!["agent".into()],
            quantity: 1,
        }],
        0.0,
    );
    let out = m.sink("exit");
    m.connect(src, phone);
    m.connect(phone, out);
    let model = m.build(Experiment::default());
    let loaded = read(&write_new(&model));
    assert!(loaded.is_ok(), "{loaded:?}");
    let Ok(loaded) = loaded else { return };
    let ids = |m: &Model| {
        m.resources
            .iter()
            .map(|r| r.id.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&loaded.model), ["zeta", "alpha"]);
    assert_eq!(loaded.model, laid_out(model));
}

/// An XPDL file's resource participants read as pools in the order it declares them.
#[test]
fn xpdl_pools_read_in_declaration_order() {
    let xpdl = r#"<?xml version="1.0" encoding="UTF-8"?>
<Package xmlns="http://www.wfmc.org/2009/XPDL2.2" xmlns:b1="http://www.bpsim.org/schemas/1.0" Id="pkg" Name="Pools">
  <Participants>
    <Participant Id="zeta" Name="Zeta"><ParticipantType Type="RESOURCE"/></Participant>
    <Participant Id="alpha" Name="Alpha"><ParticipantType Type="RESOURCE"/></Participant>
  </Participants>
  <WorkflowProcesses>
    <WorkflowProcess Id="wp">
      <Activities>
        <Activity Id="in"><Event><StartEvent Trigger="None"/></Event></Activity>
        <Activity Id="out"><Event><EndEvent Result="None"/></Event></Activity>
      </Activities>
      <Transitions><Transition Id="t1" From="in" To="out"/></Transitions>
    </WorkflowProcess>
  </WorkflowProcesses>
  <b1:BPSimData>
    <b1:Scenario id="s">
      <b1:ElementParameters elementRef="zeta">
        <b1:ResourceParameters><b1:Role><b1:StringParameter value="agent"/></b1:Role></b1:ResourceParameters>
      </b1:ElementParameters>
      <b1:ElementParameters elementRef="alpha">
        <b1:ResourceParameters><b1:Role><b1:StringParameter value="agent"/></b1:Role></b1:ResourceParameters>
      </b1:ElementParameters>
    </b1:Scenario>
  </b1:BPSimData>
</Package>"#;
    let loaded = read(xpdl.as_bytes());
    assert!(loaded.is_ok(), "{loaded:?}");
    let Ok(loaded) = loaded else { return };
    assert_eq!(loaded.opened.format, Format::Xpdl);
    let ids: Vec<String> = loaded
        .model
        .resources
        .iter()
        .map(|r| r.id.to_string())
        .collect();
    assert_eq!(ids, ["zeta", "alpha"]);
}

#[test]
fn a_mix_is_written_as_text_data_points_and_read_back() {
    let mut m = Builder::new();
    let src = m.source("parts", Dist::Exponential { mean: 1.0 }, None);
    m.property(
        src,
        "type",
        Some(PropertyType::String),
        PropertyValue::Choice(vec![
            (0.5, "A".into()),
            (0.3, "B".into()),
            (0.2, "C".into()),
        ]),
    );
    let out = m.sink("out");
    m.connect(src, out);
    let model = m.build(Experiment::default());
    let bytes = write_new(&model);
    let text = String::from_utf8_lossy(&bytes);
    // BPSim's own form: a discrete user distribution of string parameters (BPSim 2.0, 6.3.5.13).
    let squeezed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        squeezed.contains(r#"<bpsim:UserDistributionDataPoint probability="0.3"> <bpsim:StringParameter value="B"/>"#),
        "{text}"
    );
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert_eq!(loaded.model, laid_out(model));

    // Changed in the opened file, it saves in place and reads back.
    let mut edited = loaded.model.clone();
    edited.elements[0].properties[0].value =
        PropertyValue::Choice(vec![(1.0, "A".into()), (1.0, "D".into())]);
    let saved = loaded.opened.save(&edited);
    let back = saved.as_deref().map(|b| read(b).map(|l| l.model));
    assert_eq!(back, Ok(Ok(edited)));
}

#[test]
fn a_setup_before_every_job_is_bpsims_setuptime() {
    let mut m = Builder::new();
    let src = m.source("parts", Dist::Exponential { mean: 5.0 }, None);
    let machine = m.server("machine", 1, Dist::Constant(3.0));
    m.setup(
        machine,
        stochcraft_blocks::Setup::every_job(Dist::Uniform { min: 1.0, max: 2.0 }),
    );
    let out = m.sink("out");
    m.connect(src, machine);
    m.connect(machine, out);
    let model = m.build(Experiment::default());
    let bytes = write_new(&model);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("<bpsim:SetUpTime>"), "{text}");
    assert!(!text.contains("stochcraft:changeover"), "{text}");
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert_eq!(loaded.model, laid_out(model));

    // Made a changeover in the opened file: SetUpTime goes, the extension comes.
    let mut edited = loaded.model.clone();
    if let ElementKind::Server { setup, .. } = &mut edited.elements[1].kind {
        *setup = Some(stochcraft_blocks::Setup {
            time: Dist::Constant(4.0),
            on_change_of: Some("type".into()),
            by_change: Vec::new(),
        });
    }
    let saved = loaded.opened.save(&edited);
    assert!(saved.is_ok(), "{saved:?}");
    let Ok(saved) = saved else { return };
    let text = String::from_utf8_lossy(&saved);
    assert!(!text.contains("SetUpTime"), "{text}");
    assert!(
        text.contains(r#"<stochcraft:changeover property="type">"#),
        "{text}"
    );
    assert_eq!(read(&saved).map(|l| l.model).ok(), Some(edited));
}

/// A planned run time per item is StochCraft's vendor extension: written, read back, changed
/// and removed in place.
#[test]
fn a_planned_run_time_is_written_saved_and_removed() {
    let mut m = Builder::new();
    let src = m.source("parts", Dist::Exponential { mean: 5.0 }, None);
    let machine = m.server("machine", 1, Dist::Constant(3.0));
    m.planned_run_time(machine, Dist::Constant(2.5));
    let out = m.sink("out");
    m.connect(src, machine);
    m.connect(machine, out);
    let model = m.build(Experiment::default());
    let bytes = write_new(&model);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("<stochcraft:plannedRunTime>"), "{text}");
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert_eq!(loaded.model, laid_out(model));

    let mut edited = loaded.model.clone();
    if let ElementKind::Server {
        planned_run_time, ..
    } = &mut edited.elements[1].kind
    {
        *planned_run_time = Some(Dist::Constant(2.0));
    }
    let saved = loaded.opened.save(&edited);
    assert!(saved.is_ok(), "{saved:?}");
    let Ok(saved) = saved else { return };
    assert_eq!(read(&saved).map(|l| l.model).ok(), Some(edited.clone()));

    if let ElementKind::Server {
        planned_run_time, ..
    } = &mut edited.elements[1].kind
    {
        *planned_run_time = None;
    }
    let Ok(again) = read(&saved) else {
        unreachable!()
    };
    let removed = again.opened.save(&edited);
    let Ok(removed) = removed else { unreachable!() };
    let text = String::from_utf8_lossy(&removed);
    assert!(!text.contains("plannedRunTime"), "{text}");
    assert_eq!(read(&removed).map(|l| l.model).ok(), Some(edited));
}

/// A scrap sink, with the server it counts against, and a rework connection are StochCraft's
/// vendor extensions: written, read back, changed and removed in place.
#[test]
fn scrap_and_rework_are_written_saved_and_removed() {
    let mut m = Builder::new();
    let src = m.source("parts", Dist::Exponential { mean: 5.0 }, None);
    let machine = m.server("machine", 1, Dist::Constant(3.0));
    let check = m.exclusive("check");
    let good = m.sink("good");
    let scrap = m.sink("scrap");
    m.connect(src, machine);
    m.connect(machine, check);
    m.route(check, good, Some(0.8), None);
    m.route(check, scrap, Some(0.1), None);
    m.route(check, machine, Some(0.1), None);
    m.scrap(scrap, Some(machine));
    m.rework(check, machine);
    let model = m.build(Experiment::default());
    let bytes = write_new(&model);
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains(r#"<stochcraft:scrap chargedTo="machine"/>"#),
        "{text}"
    );
    assert!(text.contains(r#"name="stochcraft:rework""#), "{text}");
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert_eq!(loaded.model, laid_out(model));

    // No longer charged to the machine, and the loop no longer rework.
    let mut edited = loaded.model.clone();
    for e in &mut edited.elements {
        if let ElementKind::Sink { scrap: Some(s), .. } = &mut e.kind {
            s.charged_to = None;
        }
    }
    for c in &mut edited.connections {
        c.rework = false;
    }
    let saved = loaded.opened.save(&edited);
    let Ok(saved) = saved else { unreachable!() };
    let text = String::from_utf8_lossy(&saved);
    assert!(!text.contains("chargedTo"), "{text}");
    assert!(!text.contains("stochcraft:rework"), "{text}");
    assert_eq!(read(&saved).map(|l| l.model).ok(), Some(edited.clone()));

    // Not scrap any more.
    for e in &mut edited.elements {
        if let ElementKind::Sink { scrap, .. } = &mut e.kind {
            *scrap = None;
        }
    }
    let Ok(again) = read(&saved) else {
        unreachable!()
    };
    let Ok(removed) = again.opened.save(&edited) else {
        unreachable!()
    };
    let text = String::from_utf8_lossy(&removed);
    assert!(!text.contains("stochcraft:scrap"), "{text}");
    assert_eq!(read(&removed).map(|l| l.model).ok(), Some(edited));
}

/// A customer demand per shift of a calendar is StochCraft's vendor extension: written, read
/// back, changed and removed in place.
#[test]
fn a_demand_is_written_saved_and_removed() {
    let mut m = Builder::new();
    m.calendar(
        "day-shift",
        vec![stochcraft_blocks::Event {
            start: stochcraft_model::DateTime::new(2016, 1, 1, 8, 0, 0)
                .unwrap_or(stochcraft_model::DateTime(0)),
            end: stochcraft_model::DateTime::new(2016, 1, 1, 16, 0, 0)
                .unwrap_or(stochcraft_model::DateTime(0)),
            rule: stochcraft_blocks::Rule::parse("FREQ=DAILY").ok(),
        }],
    );
    let src = m.source("parts", Dist::Exponential { mean: 5.0 }, None);
    let out = m.sink("out");
    m.demand(
        out,
        stochcraft_blocks::Demand {
            quantity: 48.0,
            per: stochcraft_blocks::DemandPer::Shift,
            calendar: Some(stochcraft_model::ElementId::unchecked("day-shift")),
        },
    );
    m.connect(src, out);
    let model = m.build(Experiment::default());
    let bytes = write_new(&model);
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains(r#"<stochcraft:demand quantity="48" per="shift" calendar="day-shift"/>"#),
        "{text}"
    );
    let Ok(loaded) = read(&bytes) else {
        unreachable!()
    };
    assert_eq!(loaded.model, laid_out(model));

    let mut edited = loaded.model.clone();
    for e in &mut edited.elements {
        if let ElementKind::Sink {
            demand: Some(d), ..
        } = &mut e.kind
        {
            d.per = stochcraft_blocks::DemandPer::Week;
            d.calendar = None;
        }
    }
    let Ok(saved) = loaded.opened.save(&edited) else {
        unreachable!()
    };
    assert_eq!(read(&saved).map(|l| l.model).ok(), Some(edited.clone()));

    for e in &mut edited.elements {
        if let ElementKind::Sink { demand, .. } = &mut e.kind {
            *demand = None;
        }
    }
    let Ok(again) = read(&saved) else {
        unreachable!()
    };
    let Ok(removed) = again.opened.save(&edited) else {
        unreachable!()
    };
    let text = String::from_utf8_lossy(&removed);
    assert!(!text.contains("stochcraft:demand"), "{text}");
    assert_eq!(read(&removed).map(|l| l.model).ok(), Some(edited));
}

/// A rework way with no probability, condition or quantity is still written in a new file.
#[test]
fn a_plain_rework_way_is_written_in_a_new_file() {
    let mut m = Builder::new();
    let src = m.source("parts", Dist::Exponential { mean: 5.0 }, None);
    let first = m.server("first", 1, Dist::Constant(3.0));
    let again = m.server("again", 1, Dist::Constant(2.0));
    let out = m.sink("out");
    m.connect(src, first);
    m.connect(first, again);
    m.connect(again, out);
    m.rework(first, again);
    let model = m.build(Experiment::default());
    let bytes = write_new(&model);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains(r#"name="stochcraft:rework""#), "{text}");
    assert_eq!(read(&bytes).map(|l| l.model).ok(), Some(laid_out(model)));
}
