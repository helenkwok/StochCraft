//! Monte Carlo models in files (decision D15): a BPMN file with no process, whose BPSim
//! scenario holds the inputs as its own properties and the rest in StochCraft's
//! `stochcraft:monteCarlo` vendor extension ([`crate::bpsim::MONTE_CARLO`]).
//!
//! ```xml
//! <bpsim:Scenario id="scenario" name="Project cost">
//!   <bpsim:ScenarioParameters replication="10000" seed="1" baseTimeUnit="min">
//!     <bpsim:PropertyParameters>
//!       <bpsim:Property name="labour" type="double">
//!         <bpsim:TriangularDistribution mode="100" min="80" max="150"/>
//!       </bpsim:Property>
//!       <bpsim:Property name="rate" type="double">
//!         <bpsim:FloatingParameter value="45"/>
//!       </bpsim:Property>
//!     </bpsim:PropertyParameters>
//!   </bpsim:ScenarioParameters>
//!   <bpsim:VendorExtension name="stochcraft:monteCarlo" xmlns:stochcraft="…">
//!     <stochcraft:monteCarlo sampling="latinHypercube">
//!       <stochcraft:input name="labour" unit="h"/>
//!       <stochcraft:output name="cost" expression="$labour * $rate" unit="USD" atMost="7000"/>
//!       <stochcraft:correlation a="labour" b="rate" rank="0.5"/>
//!       <stochcraft:stop relative="0.01" max="1000000"/>
//!     </stochcraft:monteCarlo>
//!   </bpsim:VendorExtension>
//! </bpsim:Scenario>
//! ```
//!
//! The number of trials is the scenario's `replication` and the seed its `seed`, as for a
//! process model. A scenario that inherits from another takes its inputs, replacing those it
//! names again, and the whole vendor extension of the nearest scenario that has one.

use stochcraft_model::{
    Dist, McCorrelation, McInput, McOutput, McStop, McTarget, MonteCarlo, Sampling, TimeUnit,
};

use crate::Note;
use crate::bpsim::{self, MONTE_CARLO, NS2, NSS, STOCHCRAFT_NS};
use crate::duration::trim_float;
use crate::xml::Element;

fn s(local: &str) -> Element {
    Element::new(&format!("bpsim:{local}"), Some(NS2))
}

fn mc(local: &str) -> Element {
    Element::new(&format!("stochcraft:{local}"), Some(STOCHCRAFT_NS))
}

/// The scenario's `PropertyParameters` holding the inputs, for its `ScenarioParameters`.
pub(crate) fn properties_element(m: &MonteCarlo) -> Element {
    let mut pp = s("PropertyParameters");
    for i in &m.inputs {
        pp.push(
            s("Property")
                .with_attr("name", &i.name)
                .with_attr("type", "double")
                .with_child(crate::write::dist_element(&i.dist)),
        );
    }
    pp
}

/// The `stochcraft:monteCarlo` vendor extension: inputs' units, outputs, correlations, the
/// sampling and the stop.
pub(crate) fn extension_element(m: &MonteCarlo) -> Element {
    let mut body = mc("monteCarlo").with_attr("sampling", m.sampling.file_name());
    for i in m.inputs.iter().filter(|i| !i.unit.is_empty()) {
        body.push(
            mc("input")
                .with_attr("name", &i.name)
                .with_attr("unit", &i.unit),
        );
    }
    for o in &m.outputs {
        let mut e = mc("output")
            .with_attr("name", &o.name)
            .with_attr("expression", &o.expression);
        if !o.unit.is_empty() {
            e = e.with_attr("unit", &o.unit);
        }
        match o.target {
            Some(McTarget::AtMost(x)) => e = e.with_attr("atMost", &trim_float(x)),
            Some(McTarget::AtLeast(x)) => e = e.with_attr("atLeast", &trim_float(x)),
            None => {}
        }
        body.push(e);
    }
    for c in &m.correlations {
        body.push(
            mc("correlation")
                .with_attr("a", &c.a)
                .with_attr("b", &c.b)
                .with_attr("rank", &trim_float(c.rank)),
        );
    }
    if let Some(stop) = m.stop {
        body.push(
            mc("stop")
                .with_attr("relative", &trim_float(stop.relative))
                .with_attr("max", &stop.max.to_string()),
        );
    }
    s("VendorExtension")
        .with_attr("name", MONTE_CARLO)
        .with_attr("xmlns:stochcraft", STOCHCRAFT_NS)
        .with_child(body)
}

/// Scenario `id` and those it inherits from, the root first (a loop stops at its first repeat).
fn chain<'a>(data: &'a Element, id: &str) -> Vec<&'a Element> {
    let scenarios: Vec<&Element> = data.children_named(&NSS, "Scenario").collect();
    let by_id = |want: &str| {
        scenarios
            .iter()
            .copied()
            .find(|s| s.attr("id").as_deref() == Some(want))
    };
    let mut out: Vec<&Element> = by_id(id).into_iter().collect();
    while let Some(parent) = out.last().and_then(|s| s.attr("inherits")) {
        match by_id(&parent) {
            Some(p) if !out.iter().any(|x| std::ptr::eq(*x, p)) => out.push(p),
            _ => break,
        }
    }
    out.reverse();
    out
}

fn number(e: &Element, name: &str, at: &str, notes: &mut Vec<Note>) -> Option<f64> {
    let raw = e.attr(name)?;
    match raw.trim().parse::<f64>() {
        Ok(v) if v.is_finite() => Some(v),
        _ => {
            notes.push(Note::warning(
                Some(at),
                format!("`{name}` `{raw}` is not a number and is ignored"),
            ));
            None
        }
    }
}

/// The Monte Carlo model scenario `id` of `data` holds, or `None` if neither it nor a scenario
/// it inherits from has the `stochcraft:monteCarlo` extension (a process model's scenario
/// properties are not inputs).
pub(crate) fn read(
    data: &Element,
    id: &str,
    unit: TimeUnit,
    notes: &mut Vec<Note>,
) -> Option<MonteCarlo> {
    let chain = chain(data, id);
    let body = chain
        .iter()
        .rev()
        .copied()
        .flat_map(|s| s.children_named(&NSS, "VendorExtension"))
        .find(|x| x.attr("name").as_deref() == Some(MONTE_CARLO))?
        .elements()
        .find(|e| e.local() == "monteCarlo");
    let mut m = MonteCarlo::default();
    for scenario in &chain {
        let Some(props) = scenario
            .child(&NSS, "ScenarioParameters")
            .and_then(|p| p.child(&NSS, "PropertyParameters"))
        else {
            continue;
        };
        for p in props.children_named(&NSS, "Property") {
            let Some(name) = p.attr("name") else {
                notes.push(Note::warning(
                    None,
                    "a scenario property without a name is ignored",
                ));
                continue;
            };
            let dist = match bpsim::default_value(p, unit, notes, Some(name.as_str())) {
                Some(bpsim::Value::Dist(d)) => d,
                Some(bpsim::Value::Number(v)) => Dist::Constant(v),
                _ => {
                    notes.push(Note::warning(
                        Some(name.as_str()),
                        "this input is not a number or a distribution and is ignored",
                    ));
                    continue;
                }
            };
            match m.inputs.iter_mut().find(|i| i.name == name) {
                Some(i) => i.dist = dist,
                None => m.inputs.push(McInput {
                    name,
                    dist,
                    unit: String::new(),
                }),
            }
        }
    }
    let Some(body) = body else {
        return Some(m);
    };
    if let Some(raw) = body.attr("sampling") {
        match Sampling::from_file_name(raw.trim()) {
            Some(x) => m.sampling = x,
            None => notes.push(Note::warning(
                None,
                format!("sampling `{raw}` is not random, latinHypercube, antithetic or sobol; random is used"),
            )),
        }
    }
    for e in body.elements() {
        match e.local() {
            "input" => {
                let name = e.attr("name").unwrap_or_default();
                match m.inputs.iter_mut().find(|i| i.name == name) {
                    Some(i) => i.unit = e.attr("unit").unwrap_or_default(),
                    None => notes.push(Note::warning(
                        Some(name.as_str()),
                        "the unit of an input the scenario does not have is ignored",
                    )),
                }
            }
            "output" => {
                let name = e.attr("name").unwrap_or_default();
                let at_most = number(e, "atMost", &name, notes);
                let at_least = number(e, "atLeast", &name, notes);
                let target = match (at_most, at_least) {
                    (Some(x), None) => Some(McTarget::AtMost(x)),
                    (None, Some(x)) => Some(McTarget::AtLeast(x)),
                    (None, None) => None,
                    (Some(x), Some(_)) => {
                        notes.push(Note::warning(
                            Some(name.as_str()),
                            "an output has one target, atMost or atLeast; atMost is used",
                        ));
                        Some(McTarget::AtMost(x))
                    }
                };
                m.outputs.push(McOutput {
                    expression: e.attr("expression").unwrap_or_default(),
                    unit: e.attr("unit").unwrap_or_default(),
                    target,
                    name,
                });
            }
            "correlation" => {
                let (a, b) = (
                    e.attr("a").unwrap_or_default(),
                    e.attr("b").unwrap_or_default(),
                );
                let at = format!("{a}~{b}");
                if let Some(rank) = number(e, "rank", &at, notes) {
                    m.correlations.push(McCorrelation { a, b, rank });
                }
            }
            "stop" => {
                let mut stop = McStop::default();
                if let Some(r) = number(e, "relative", "stop", notes) {
                    stop.relative = r;
                }
                if let Some(raw) = e.attr("max") {
                    match raw.trim().parse::<u32>() {
                        Ok(v) if v > 0 => stop.max = v,
                        _ => notes.push(Note::warning(
                            None,
                            format!(
                                "stop `max` `{raw}` is not a whole number above 0 and is ignored"
                            ),
                        )),
                    }
                }
                m.stop = Some(stop);
            }
            other => notes.push(Note::warning(
                None,
                format!("`{other}` in a Monte Carlo model is not known and is ignored"),
            )),
        }
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use stochcraft_model::Model;

    use super::*;

    fn model() -> Model {
        let mut m = Model {
            name: "Project cost".into(),
            monte_carlo: Some(MonteCarlo {
                inputs: vec![
                    McInput {
                        name: "labour".into(),
                        dist: Dist::Triangular {
                            min: 80.0,
                            mode: 100.0,
                            max: 150.0,
                        },
                        unit: "h".into(),
                    },
                    McInput {
                        name: "rate".into(),
                        dist: Dist::Constant(45.0),
                        unit: "USD/h".into(),
                    },
                    McInput {
                        name: "rebate".into(),
                        dist: Dist::Constant(-250.0),
                        unit: String::new(),
                    },
                    McInput {
                        name: "materials".into(),
                        dist: Dist::User {
                            points: vec![(0.25, 400.0), (0.75, 600.0)],
                            discrete: true,
                        },
                        unit: "USD".into(),
                    },
                ],
                outputs: vec![
                    McOutput {
                        name: "cost".into(),
                        expression: "$labour * $rate + $materials + $rebate".into(),
                        unit: "USD".into(),
                        target: Some(McTarget::AtMost(5_500.0)),
                    },
                    McOutput {
                        name: "over".into(),
                        expression: "if($cost > 6000, 1, 0) & 'x' < \"y\"".into(),
                        unit: String::new(),
                        target: Some(McTarget::AtLeast(0.0)),
                    },
                ],
                correlations: vec![McCorrelation {
                    a: "labour".into(),
                    b: "materials".into(),
                    rank: 0.5,
                }],
                sampling: Sampling::LatinHypercube,
                stop: Some(McStop {
                    relative: 0.005,
                    max: 200_000,
                }),
            }),
            ..Model::default()
        };
        m.experiment.replications = 10_000;
        m.experiment.seed = 42;
        m
    }

    #[test]
    fn a_monte_carlo_model_is_a_bpmn_file_with_no_process() {
        let m = model();
        let bytes = crate::write_new(&m);
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("bpmn:process"), "{text}");
        assert!(!text.contains("BPMNDiagram"), "{text}");
        assert!(text.contains("stochcraft:monteCarlo"), "{text}");
        assert!(
            text.contains(r#"<bpsim:Property name="labour" type="double">"#),
            "{text}"
        );
        let loaded = crate::read(&bytes).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(loaded.model, m, "{text}");
        assert!(loaded.model.is_monte_carlo());
        // Saved unchanged, it is the same bytes; edited, it reads back as the edit.
        assert_eq!(loaded.opened.save(&m).as_deref(), Ok(&bytes[..]));
        let mut edited = m.clone();
        if let Some(mc) = edited.monte_carlo.as_mut() {
            mc.inputs[0].dist = Dist::Normal {
                mean: 100.0,
                std_dev: 10.0,
            };
            mc.outputs.remove(1);
            mc.correlations.clear();
            mc.stop = None;
            mc.sampling = Sampling::Random;
        }
        edited.experiment.replications = 5_000;
        let saved = loaded
            .opened
            .save(&edited)
            .unwrap_or_else(|e| unreachable!("{e}"));
        let back = crate::read(&saved).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(back.model, edited);
        // Saved again unchanged from there, byte for byte.
        assert_eq!(back.opened.save(&edited).as_deref(), Ok(&saved[..]));
    }

    /// Every sampling is saved under its file name and read back, and a name this reader does
    /// not know is a warning that falls back to random sampling (what a reader from before the
    /// antithetic and Sobol modes does with a file that has them).
    #[test]
    fn every_sampling_is_saved_and_read_back() {
        for sampling in [
            Sampling::Random,
            Sampling::LatinHypercube,
            Sampling::Antithetic,
            Sampling::Sobol,
        ] {
            let mut m = model();
            if let Some(mc) = m.monte_carlo.as_mut() {
                mc.sampling = sampling;
            }
            let bytes = crate::write_new(&m);
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                text.contains(&format!(r#"sampling="{}""#, sampling.file_name())),
                "{text}"
            );
            let loaded = crate::read(&bytes).unwrap_or_else(|e| unreachable!("{e}"));
            assert_eq!(loaded.model, m, "{text}");
        }
        let text = String::from_utf8_lossy(&crate::write_new(&model()))
            .replace(r#"sampling="latinHypercube""#, r#"sampling="quasiRandom""#);
        let loaded = crate::read(text.as_bytes()).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            loaded.model.monte_carlo.map(|mc| mc.sampling),
            Some(Sampling::Random)
        );
        assert!(
            loaded
                .notes
                .iter()
                .any(|n| n.message.contains("quasiRandom")),
            "{:?}",
            loaded.notes
        );
    }

    #[test]
    fn a_process_model_is_not_a_monte_carlo_one() {
        let bytes = crate::write_new(&Model::default());
        let loaded = crate::read(&bytes).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(loaded.model.monte_carlo, None);
        // The extension alone makes one, with no inputs yet.
        let m = Model {
            monte_carlo: Some(MonteCarlo::default()),
            ..Model::default()
        };
        let back = crate::read(&crate::write_new(&m)).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(back.model.monte_carlo, Some(MonteCarlo::default()));
    }

    #[test]
    fn an_inheriting_scenario_replaces_inputs_by_name() {
        let xml = format!(
            r#"<bpmn:definitions xmlns:bpmn="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:bpsim="{NS2}" xmlns:stochcraft="{STOCHCRAFT_NS}" id="d">
  <bpmn:relationship type="BPSimData"><bpmn:extensionElements><bpsim:BPSimData>
    <bpsim:Scenario id="base">
      <bpsim:ScenarioParameters replication="100"><bpsim:PropertyParameters>
        <bpsim:Property name="a"><bpsim:FloatingParameter value="1"/></bpsim:Property>
        <bpsim:Property name="b"><bpsim:FloatingParameter value="2"/></bpsim:Property>
      </bpsim:PropertyParameters></bpsim:ScenarioParameters>
      <bpsim:VendorExtension name="stochcraft:monteCarlo"><stochcraft:monteCarlo>
        <stochcraft:output name="s" expression="$a + $b"/>
        <stochcraft:frobnicate/>
      </stochcraft:monteCarlo></bpsim:VendorExtension>
    </bpsim:Scenario>
    <bpsim:Scenario id="high" inherits="base">
      <bpsim:ScenarioParameters><bpsim:PropertyParameters>
        <bpsim:Property name="b"><bpsim:UniformDistribution min="5" max="6"/></bpsim:Property>
      </bpsim:PropertyParameters></bpsim:ScenarioParameters>
    </bpsim:Scenario>
  </bpsim:BPSimData></bpmn:extensionElements><bpmn:source>d</bpmn:source><bpmn:target>d</bpmn:target></bpmn:relationship>
</bpmn:definitions>"#
        );
        let loaded = crate::read_scenario(xml.as_bytes(), Some("high"))
            .unwrap_or_else(|e| unreachable!("{e}"));
        let m = loaded.model.monte_carlo.unwrap_or_default();
        let names: Vec<(&str, &Dist)> = m
            .inputs
            .iter()
            .map(|i| (i.name.as_str(), &i.dist))
            .collect();
        assert_eq!(
            names,
            vec![
                ("a", &Dist::Constant(1.0)),
                ("b", &Dist::Uniform { min: 5.0, max: 6.0 })
            ]
        );
        assert_eq!(m.outputs.len(), 1);
        assert_eq!(loaded.model.experiment.replications, 100);
        assert!(
            loaded
                .notes
                .iter()
                .any(|n| n.message.contains("frobnicate")),
            "{:?}",
            loaded.notes
        );
    }
}
