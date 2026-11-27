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
            q: 0.95,
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

/// The file `started()` writes, with a scenario `later` that inherits everything from it.
fn with_child() -> (String, String) {
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
      </bpsim:BPSimData>"#
        ),
    );
    (text, base)
}

#[test]
fn a_child_scenario_empties_a_block_its_parent_starts_full() {
    let (text, base) = with_child();
    let loaded = read_scenario(text.as_bytes(), Some("later")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(loaded.model.starting, started().starting);
    // The packing task starts empty in the child: saying nothing there would let the parent's 3
    // come back, so the child says 0.
    let mut edited = loaded.model.clone();
    edited.starting.remove(&ElementId::unchecked("pack"));
    let saved = loaded
        .opened
        .save(&edited)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        String::from_utf8_lossy(&saved).contains("stochcraft:value=\"0\""),
        "{}",
        String::from_utf8_lossy(&saved)
    );
    let child = read_scenario(&saved, Some("later")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(child.model.starting, edited.starting);
    assert!(
        !child
            .model
            .starting
            .contains_key(&ElementId::unchecked("pack"))
    );
    let parent = read_scenario(&saved, Some(base.as_str())).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(parent.model.starting, started().starting);
}

#[test]
fn a_block_that_comes_to_hold_entities_loses_a_value_the_file_kept() {
    // A split, which cannot hold entities at the start, with a value the file keeps (read with
    // a note and ignored), made a delay in place: the delay starts empty, as the model says.
    let mut b = Builder::new();
    let src = b.source("orders", Dist::Exponential { mean: 5.0 }, None);
    let split = b.unbatch("split");
    let done = b.sink("done");
    b.connect(src, split);
    b.connect(split, done);
    let m = b.build(Experiment::default());
    let text = String::from_utf8_lossy(&write_new(&m)).replace(
        "</bpsim:Scenario>",
        r#"<bpsim:ElementParameters elementRef="split">
            <bpsim:VendorExtension name="stochcraft:startingContents" xmlns:stochcraft="https://helenkwok.github.io/StochCraft/bpsim/1" stochcraft:value="4"/>
          </bpsim:ElementParameters>
        </bpsim:Scenario>"#,
    );
    let loaded = read(text.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    assert!(loaded.model.starting.is_empty());
    assert!(
        loaded
            .notes
            .iter()
            .any(|n| n.message.contains("starting contents")),
        "{:#?}",
        loaded.notes
    );
    let mut edited = loaded.model.clone();
    if let Some(e) = edited
        .elements
        .iter_mut()
        .find(|e| e.id.as_str() == "split")
    {
        e.kind = ElementKind::Delay {
            duration: Dist::Constant(1.0),
            processing_by: None,
        };
    }
    let saved = loaded
        .opened
        .save(&edited)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        !String::from_utf8_lossy(&saved).contains("startingContents"),
        "{}",
        String::from_utf8_lossy(&saved)
    );
    let back = read(&saved).unwrap_or_else(|e| panic!("{e}"));
    assert!(back.model.starting.is_empty());
}

/// A BPMN file with no BPSim data: the first save that writes the scenario adds one, and every
/// part of the same save goes into it (a time, a waiting room, a starting count and the run's
/// length), not into one new scenario each.
#[test]
fn one_save_into_a_file_without_bpsim_data_adds_one_scenario() {
    let path = format!("{}/../../tests/models/mm1.bpmn", env!("CARGO_MANIFEST_DIR"));
    let original = String::from_utf8(std::fs::read(&path).unwrap_or_else(|e| panic!("{e}")))
        .unwrap_or_else(|e| panic!("{e}"));
    let (from, to) = (
        original
            .find("<semantic:relationship type=\"BPSimData\">")
            .unwrap_or_else(|| panic!("no BPSim data in mm1.bpmn")),
        original
            .find("</semantic:relationship>")
            .unwrap_or_else(|| panic!("no end of BPSim data in mm1.bpmn")),
    );
    let bare = format!(
        "{}{}",
        &original[..from],
        &original[to + "</semantic:relationship>".len()..]
    );
    assert!(!bare.contains("BPSimData"));
    let loaded = read(bare.as_bytes()).unwrap_or_else(|e| panic!("{e}"));
    let mut edited = loaded.model.clone();
    edited.experiment.duration = 120.0;
    edited.starting.insert(ElementId::unchecked("serve"), 2);
    if let Some(e) = edited
        .elements
        .iter_mut()
        .find(|e| e.id.as_str() == "serve")
        && let ElementKind::Server {
            service,
            queue_capacity,
            ..
        } = &mut e.kind
    {
        *service = Dist::Constant(1.5);
        *queue_capacity = Some(5);
    }
    let saved = loaded
        .opened
        .save(&edited)
        .unwrap_or_else(|e| panic!("{e}"));
    let text = String::from_utf8_lossy(&saved);
    assert_eq!(text.matches("Scenario id=").count(), 1, "{text}");
    let back = read(&saved).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.model.experiment.duration, 120.0);
    assert_eq!(back.model.starting, edited.starting);
    assert_eq!(back.model.elements, edited.elements);
}

/// An estimate's upper point may be any quantile: `q` is written beside `p95` when it is not
/// 0.95, read back, and left out at 0.95.
#[test]
fn an_estimate_s_quantile_is_written_and_read() {
    let mut m = model();
    let mut e = estimate(Family::LogNormal, 1.0, 4.0, 10.0, Upper::P95);
    e.three_point.q = 0.9;
    m.estimates.insert(ElementId::unchecked("ship"), e);
    let bytes = write_new(&m);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("p95=\"10\" q=\"0.9\""), "{text}");
    let back = read(&bytes).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(back.model.estimates, m.estimates);
    let text = String::from_utf8_lossy(&write_new(&model())).into_owned();
    assert!(!text.contains(" q=\""), "{text}");
}
