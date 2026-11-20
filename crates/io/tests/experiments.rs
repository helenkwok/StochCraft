//! Terminating experiments, three-point estimates (M2.8) and starting contents (M2.9) in files:
//! StochCraft's vendor extensions `stochcraft:stopCount`, `stochcraft:stopWhenEmpty`,
//! `stochcraft:estimate` and `stochcraft:startingContents` written, read back, edited and saved.

#![allow(clippy::panic)]

use stochcraft_blocks::Builder;
use stochcraft_io::{read, read_scenario, write_new};
use stochcraft_model::{
    Dist, ElementId, ElementKind, Estimate, Experiment, Family, Model, Stop, ThreePoint, Upper,
};

fn estimate(family: Family, min: f64, mode: f64, upper: f64, upper_is: Upper) -> Estimate {
    Estimate {
        three_point: ThreePoint {
            family,
            min,
            mode,
            upper,
            upper_is,
        },
        source: "Delphi, 3 experts, round 2 — médiane".into(),
    }
}

/// Orders, a task timed from a PERT estimate, and a run that stops at the 50th order done.
fn model() -> Model {
    let mut b = Builder::new();
    let src = b.source("orders", Dist::Exponential { mean: 5.0 }, None);
    let pack = b.server("pack", 1, Dist::Constant(1.0));
    let ship = b.delay("ship", Dist::Constant(1.0));
    let done = b.sink("done");
    b.connect(src, pack);
    b.connect(pack, ship);
    b.connect(ship, done);
    let mut m = b.build(Experiment {
        warmup: 0.0,
        duration: 10_000.0,
        replications: 5,
        stop: Some(Stop::Count {
            element: ElementId::unchecked("done"),
            count: 50,
        }),
        ..Experiment::default()
    });
    for (id, e) in [
        ("pack", estimate(Family::Pert, 1.0, 2.0, 6.0, Upper::Max)),
        ("ship", estimate(Family::Weibull, 0.5, 2.0, 5.0, Upper::P95)),
    ] {
        let fitted = e.three_point.fit().unwrap_or_else(|err| panic!("{err}"));
        let id = ElementId::unchecked(id);
        if let Some(el) = m.elements.iter_mut().find(|x| x.id == id) {
            match &mut el.kind {
                ElementKind::Server { service: d, .. } | ElementKind::Delay { duration: d, .. } => {
                    *d = fitted;
                }
                _ => {}
            }
        }
        m.estimates.insert(id, e);
    }
    m.layout = m.placed();
    m
}

#[test]
fn a_new_file_keeps_the_stop_and_the_estimates() {
    let m = model();
    assert!(m.validate().is_empty(), "{:?}", m.validate());
    let bytes = write_new(&m);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("name=\"stochcraft:stopCount\""), "{text}");
    assert!(text.contains("<stochcraft:estimate family=\"pert\" min=\"1\" mode=\"2\" max=\"6\""));
    // The PERT is a standard BPSim user distribution.
    assert!(text.contains("UserDistribution"));
    let back = read(&bytes).unwrap_or_else(|e| panic!("{e}"));
    assert!(back.notes.is_empty(), "{:#?}", back.notes);
    assert_eq!(back.model, m);
}

#[test]
fn stops_and_estimates_save_into_an_opened_file() {
    let m = model();
    let original = write_new(&m);
    let loaded = read(&original).unwrap_or_else(|e| panic!("{e}"));
    // Stop when the packing task empties instead, change one estimate, forget the other.
    let mut edited = loaded.model.clone();
    edited.experiment.stop = Some(Stop::Empty {
        element: ElementId::unchecked("pack"),
    });
    let new = estimate(Family::Triangular, 1.0, 1.5, 4.0, Upper::P95);
    if let Some(el) = edited.elements.iter_mut().find(|x| x.id.as_str() == "pack")
        && let ElementKind::Server { service, .. } = &mut el.kind
    {
        *service = new.three_point.fit().unwrap_or(Dist::Constant(0.0));
    }
    edited
        .estimates
        .insert(ElementId::unchecked("pack"), new.clone());
    edited.estimates.remove(&ElementId::unchecked("ship"));
    let saved = loaded
        .opened
        .save(&edited)
        .unwrap_or_else(|e| panic!("{e}"));
    let text = String::from_utf8_lossy(&saved);
    assert!(!text.contains("stopCount"), "{text}");
    assert!(text.contains("stochcraft:stopWhenEmpty"));
    assert!(!text.contains("weibull"));
    let back = read_scenario(&saved, None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.model.experiment.stop, edited.experiment.stop);
    assert_eq!(back.model.estimates, edited.estimates);
    // And removing both leaves no trace.
    let mut bare = back.model.clone();
    bare.experiment.stop = None;
    bare.estimates.clear();
    let saved = back.opened.save(&bare).unwrap_or_else(|e| panic!("{e}"));
    let text = String::from_utf8_lossy(&saved);
    assert!(!text.contains("stochcraft:stop") && !text.contains("stochcraft:estimate"));
}

#[test]
fn bad_extensions_are_reported_and_ignored() {
    let text = String::from_utf8_lossy(&write_new(&model()))
        .replace("family=\"pert\"", "family=\"gumbel\"")
        .replace(
            "name=\"stochcraft:stopCount\" xmlns:stochcraft=\"https://helenkwok.github.io/StochCraft/bpsim/1\" stochcraft:value=\"50\"",
            "name=\"stochcraft:stopCount\" xmlns:stochcraft=\"https://helenkwok.github.io/StochCraft/bpsim/1\" stochcraft:value=\"none\"",
        );
    let back = read(text.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.model.experiment.stop, None);
    assert_eq!(back.model.estimates.len(), 1);
    let messages: Vec<&str> = back.notes.iter().map(|n| n.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("stop count")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("three-point estimate")),
        "{messages:?}"
    );
}

/// The orders model with entities inside its task and its delay at the start.
fn started() -> Model {
    let mut m = model();
    m.starting.insert(ElementId::unchecked("pack"), 3);
    m.starting.insert(ElementId::unchecked("ship"), 2);
    m
}

#[test]
fn a_new_file_keeps_the_starting_contents() {
    let m = started();
    assert!(m.validate().is_empty(), "{:?}", m.validate());
    let text = String::from_utf8_lossy(&write_new(&m)).into_owned();
    assert!(
        text.contains("name=\"stochcraft:startingContents\""),
        "{text}"
    );
    let back = read(text.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert!(back.notes.is_empty(), "{:#?}", back.notes);
    assert_eq!(back.model, m);
}

#[test]
fn starting_contents_save_into_an_opened_file() {
    let loaded = read(&write_new(&started())).unwrap_or_else(|e| panic!("{e}"));
    // More inside the task, none in the delay.
    let mut edited = loaded.model.clone();
    edited.starting.insert(ElementId::unchecked("pack"), 5);
    edited.starting.remove(&ElementId::unchecked("ship"));
    let saved = loaded
        .opened
        .save(&edited)
        .unwrap_or_else(|e| panic!("{e}"));
    let back = read_scenario(&saved, None).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.model.starting, edited.starting);
    // And with none left, no trace of them.
    let mut bare = back.model.clone();
    bare.starting.clear();
    let saved = back.opened.save(&bare).unwrap_or_else(|e| panic!("{e}"));
    let text = String::from_utf8_lossy(&saved);
    assert!(!text.contains("startingContents"), "{text}");
}

#[test]
fn a_scenario_inherits_starting_contents_and_can_empty_them() {
    let text = String::from_utf8_lossy(&write_new(&started())).into_owned();
    let base = text
        .split("<bpsim:Scenario id=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no scenario in {text}"))
        .to_owned();
    let text = text.replace(
        "</bpsim:BPSimData>",
        &format!(
            r#"<bpsim:Scenario id="later" name="Later" inherits="{base}"></bpsim:Scenario>
        <bpsim:Scenario id="emptied" name="Emptied" inherits="{base}">
          <bpsim:ElementParameters elementRef="pack">
            <bpsim:VendorExtension name="stochcraft:startingContents" xmlns:stochcraft="https://helenkwok.github.io/StochCraft/bpsim/1" stochcraft:value="0"/>
          </bpsim:ElementParameters>
        </bpsim:Scenario>
      </bpsim:BPSimData>"#
        ),
    );
    let later = read_scenario(text.as_bytes(), Some("later")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(later.model.starting, started().starting);
    let emptied = read_scenario(text.as_bytes(), Some("emptied")).unwrap_or_else(|e| panic!("{e}"));
    assert!(emptied.notes.is_empty(), "{:#?}", emptied.notes);
    let only_ship: Vec<(&str, u32)> = emptied
        .model
        .starting
        .iter()
        .map(|(id, n)| (id.as_str(), *n))
        .collect();
    assert_eq!(only_ship, vec![("ship", 2)]);
}

#[test]
fn bad_starting_contents_are_reported_and_ignored() {
    let text = String::from_utf8_lossy(&write_new(&started()))
        .replace(
            "name=\"stochcraft:startingContents\" xmlns:stochcraft=\"https://helenkwok.github.io/StochCraft/bpsim/1\" stochcraft:value=\"3\"",
            "name=\"stochcraft:startingContents\" xmlns:stochcraft=\"https://helenkwok.github.io/StochCraft/bpsim/1\" stochcraft:value=\"three\"",
        );
    let back = read(text.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.model.starting.len(), 1);
    assert!(
        back.notes
            .iter()
            .any(|n| n.message.contains("starting contents")),
        "{:#?}",
        back.notes
    );
}
