//! Writing a model and reading it back gives the same model; opened files save unchanged.

use stochcraft_blocks::{BlockId, Builder, Event, Rule};
use stochcraft_io::{Format, read, write_new};
use stochcraft_model::{
    DateTime, Dist, ElementKind, Experiment, Model, Property, PropertyType, PropertyValue, TimeUnit,
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
    let mut model = m.build(Experiment {
        time_unit: TimeUnit::Hours,
        warmup: 2.5,
        duration: 40.0,
        replications: 7,
        seed: u64::MAX - 3,
        start: Some(at(4, 6)),
    });
    model.name = "Showcase & <friends>".into();
    model
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
    assert_eq!(loaded.model, model);
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
    assert_eq!(read(text.as_bytes()).map(|l| l.model).ok(), Some(model));
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
    edited.elements.pop();
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
    assert_eq!(loaded.model, model);
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
        };
    }
    let saved = loaded.opened.save(&edited);
    assert!(saved.is_ok(), "{saved:?}");
    let Ok(saved) = saved else { return };
    assert_eq!(read(&saved).map(|l| l.model).ok(), Some(edited));
}
