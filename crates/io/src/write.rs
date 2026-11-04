//! Writing a model StochCraft created as BPMN 2.0 with embedded BPSim 2.0.
//!
//! Everything is written in the order the BPMN 2.0 and BPSim 2.0 schemas require. Ids are the
//! model's; the ids StochCraft adds (the definitions, process, resources, scenario and diagram
//! shapes) are chosen so they never collide with them, since XML ids are unique per file.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use stochcraft_model::{Dist, ElementKind, Model};

use crate::bpsim::NS2;
use crate::duration::trim_float;
use crate::read::BPMN;
use crate::xml::{self, Document, Element};

const BPMNDI: &str = "http://www.omg.org/spec/BPMN/20100524/DI";
const DC: &str = "http://www.omg.org/spec/DD/20100524/DC";
const DI: &str = "http://www.omg.org/spec/DD/20100524/DI";

/// Hands out ids that are not already used in the file.
struct Ids(BTreeSet<String>);

impl Ids {
    fn fresh(&mut self, base: &str) -> String {
        let mut id = base.to_owned();
        let mut n = 1;
        while self.0.contains(&id) {
            n += 1;
            id = format!("{base}_{n}");
        }
        self.0.insert(id.clone());
        id
    }
}

fn b(local: &str) -> Element {
    Element::new(&format!("bpmn:{local}"), Some(BPMN))
}

fn s(local: &str) -> Element {
    Element::new(&format!("bpsim:{local}"), Some(NS2))
}

fn num(v: f64) -> String {
    trim_float(v)
}

/// The BPSim value element for a distribution.
pub fn dist_element(d: &Dist) -> Element {
    match d {
        Dist::Constant(v) => s("FloatingParameter").with_attr("value", &num(*v)),
        Dist::Exponential { mean } => {
            s("NegativeExponentialDistribution").with_attr("mean", &num(*mean))
        }
        Dist::Uniform { min, max } => s("UniformDistribution")
            .with_attr("min", &num(*min))
            .with_attr("max", &num(*max)),
        Dist::Triangular { min, mode, max } => s("TriangularDistribution")
            .with_attr("mode", &num(*mode))
            .with_attr("min", &num(*min))
            .with_attr("max", &num(*max)),
        Dist::Normal { mean, std_dev } => s("NormalDistribution")
            .with_attr("mean", &num(*mean))
            .with_attr("standardDeviation", &num(*std_dev)),
        Dist::LogNormal { mean, std_dev } => s("LogNormalDistribution")
            .with_attr("mean", &num(*mean))
            .with_attr("standardDeviation", &num(*std_dev)),
        Dist::TruncatedNormal {
            mean,
            std_dev,
            min,
            max,
        } => s("TruncatedNormalDistribution")
            .with_attr("mean", &num(*mean))
            .with_attr("standardDeviation", &num(*std_dev))
            .with_attr("min", &num(*min))
            .with_attr("max", &num(*max)),
        Dist::Gamma { shape, scale } => s("GammaDistribution")
            .with_attr("shape", &num(*shape))
            .with_attr("scale", &num(*scale)),
        Dist::Beta { shape, scale } => s("BetaDistribution")
            .with_attr("shape", &num(*shape))
            .with_attr("scale", &num(*scale)),
        Dist::Weibull { shape, scale } => s("WeibullDistribution")
            .with_attr("shape", &num(*shape))
            .with_attr("scale", &num(*scale)),
        Dist::Erlang { mean, k } => s("ErlangDistribution")
            .with_attr("mean", &num(*mean))
            .with_attr("k", &k.to_string()),
        Dist::Poisson { mean } => s("PoissonDistribution").with_attr("mean", &num(*mean)),
        Dist::Binomial {
            probability,
            trials,
        } => s("BinomialDistribution")
            .with_attr("probability", &num(*probability))
            .with_attr("trials", &trials.to_string()),
        Dist::User { points, discrete } => {
            let mut e = s("UserDistribution")
                .with_attr("discrete", if *discrete { "true" } else { "false" });
            for (p, v) in points {
                e.push(
                    s("UserDistributionDataPoint")
                        .with_attr("probability", &num(*p))
                        .with_child(s("FloatingParameter").with_attr("value", &num(*v))),
                );
            }
            e
        }
    }
}

fn param(name: &str, value: Element) -> Element {
    s(name).with_child(value)
}

/// Columns by distance from the sources (breadth first), rows in model order within a column.
fn layout(model: &Model) -> BTreeMap<String, (f64, f64)> {
    let mut next: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for c in &model.connections {
        next.entry(c.from.as_str()).or_default().push(c.to.as_str());
    }
    let mut depth: BTreeMap<&str, usize> = BTreeMap::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    for e in &model.elements {
        if matches!(e.kind, ElementKind::Source { .. }) {
            depth.insert(e.id.as_str(), 0);
            queue.push_back(e.id.as_str());
        }
    }
    while let Some(id) = queue.pop_front() {
        let d = depth.get(id).copied().unwrap_or(0);
        for n in next.get(id).into_iter().flatten() {
            if !depth.contains_key(n) {
                depth.insert(n, d + 1);
                queue.push_back(n);
            }
        }
    }
    let last = depth.values().copied().max().map_or(0, |m| m + 1);
    let mut rows: BTreeMap<usize, usize> = BTreeMap::new();
    let mut centres = BTreeMap::new();
    for e in &model.elements {
        let col = depth.get(e.id.as_str()).copied().unwrap_or(last);
        let row = rows.entry(col).or_insert(0);
        centres.insert(
            e.id.as_str().to_owned(),
            (100.0 + 180.0 * col as f64, 100.0 + 140.0 * *row as f64),
        );
        *row += 1;
    }
    centres
}

fn size(kind: &ElementKind) -> (f64, f64) {
    match kind {
        ElementKind::Source { .. } | ElementKind::Sink => (36.0, 36.0),
        _ => (100.0, 80.0),
    }
}

/// The model as a BPMN 2.0 file with embedded BPSim 2.0 data.
pub fn write_new(model: &Model) -> Vec<u8> {
    let mut ids = Ids(model
        .elements
        .iter()
        .map(|e| e.id.as_str().to_owned())
        .chain(model.connections.iter().map(|c| c.id.as_str().to_owned()))
        .collect());
    let defs_id = ids.fresh("definitions");
    let process_id = ids.fresh("process");
    let mut defs = b("definitions")
        .with_attr("xmlns:bpmn", BPMN)
        .with_attr("xmlns:bpmndi", BPMNDI)
        .with_attr("xmlns:dc", DC)
        .with_attr("xmlns:di", DI)
        .with_attr("xmlns:bpsim", NS2)
        .with_attr("id", &defs_id)
        .with_attr("name", &model.name)
        .with_attr(
            "targetNamespace",
            "https://github.com/helenkwok/StochCraft/model",
        )
        .with_attr("exporter", "StochCraft")
        .with_attr("exporterVersion", env!("CARGO_PKG_VERSION"));

    // Each server owns a resource with as many units as its capacity.
    let mut resource_of = BTreeMap::new();
    for e in &model.elements {
        if let ElementKind::Server { .. } = e.kind {
            let rid = ids.fresh(&format!("{}_resource", e.id));
            defs.push(
                b("resource")
                    .with_attr("id", &rid)
                    .with_attr("name", &e.name),
            );
            resource_of.insert(e.id.as_str(), rid);
        }
    }

    let mut process = b("process")
        .with_attr("id", &process_id)
        .with_attr("name", &model.name)
        .with_attr("isExecutable", "false");
    for e in &model.elements {
        let local = match e.kind {
            ElementKind::Source { .. } => "startEvent",
            ElementKind::Sink => "endEvent",
            ElementKind::Server { .. } | ElementKind::Delay { .. } => "task",
        };
        let mut node = b(local)
            .with_attr("id", e.id.as_str())
            .with_attr("name", &e.name);
        for c in model.connections.iter().filter(|c| c.to == e.id) {
            node.push(b("incoming").with_text(c.id.as_str()));
        }
        for c in model.connections.iter().filter(|c| c.from == e.id) {
            node.push(b("outgoing").with_text(c.id.as_str()));
        }
        if let Some(rid) = resource_of.get(e.id.as_str()) {
            node.push(b("performer").with_child(b("resourceRef").with_text(rid)));
        }
        process.push(node);
    }
    for c in &model.connections {
        process.push(
            b("sequenceFlow")
                .with_attr("id", c.id.as_str())
                .with_attr("sourceRef", c.from.as_str())
                .with_attr("targetRef", c.to.as_str()),
        );
    }
    defs.push(process);

    // The diagram.
    let centres = layout(model);
    let mut plane = Element::new("bpmndi:BPMNPlane", Some(BPMNDI))
        .with_attr("id", &ids.fresh("plane"))
        .with_attr("bpmnElement", &process_id);
    for e in &model.elements {
        let (cx, cy) = centres.get(e.id.as_str()).copied().unwrap_or((0.0, 0.0));
        let (w, h) = size(&e.kind);
        plane.push(
            Element::new("bpmndi:BPMNShape", Some(BPMNDI))
                .with_attr("id", &ids.fresh(&format!("{}_di", e.id)))
                .with_attr("bpmnElement", e.id.as_str())
                .with_child(
                    Element::new("dc:Bounds", Some(DC))
                        .with_attr("x", &num(cx - w / 2.0))
                        .with_attr("y", &num(cy - h / 2.0))
                        .with_attr("width", &num(w))
                        .with_attr("height", &num(h)),
                ),
        );
    }
    let half_width = |id: &str| {
        model
            .elements
            .iter()
            .find(|e| e.id.as_str() == id)
            .map_or(0.0, |e| size(&e.kind).0 / 2.0)
    };
    for c in &model.connections {
        let (fx, fy) = centres.get(c.from.as_str()).copied().unwrap_or((0.0, 0.0));
        let (tx, ty) = centres.get(c.to.as_str()).copied().unwrap_or((0.0, 0.0));
        let waypoint = |x: f64, y: f64| {
            Element::new("di:waypoint", Some(DI))
                .with_attr("x", &num(x))
                .with_attr("y", &num(y))
        };
        plane.push(
            Element::new("bpmndi:BPMNEdge", Some(BPMNDI))
                .with_attr("id", &ids.fresh(&format!("{}_di", c.id)))
                .with_attr("bpmnElement", c.id.as_str())
                .with_child(waypoint(fx + half_width(c.from.as_str()), fy))
                .with_child(waypoint(tx - half_width(c.to.as_str()), ty)),
        );
    }
    defs.push(
        Element::new("bpmndi:BPMNDiagram", Some(BPMNDI))
            .with_attr("id", &ids.fresh("diagram"))
            .with_child(plane),
    );

    // The simulation data, in BPSim schema order.
    let x = &model.experiment;
    let mut sp = s("ScenarioParameters")
        .with_attr("replication", &x.replications.to_string())
        // `xs:long` is signed; the seed's 64 bits are kept and read back unchanged.
        .with_attr("seed", &(x.seed as i64).to_string())
        .with_attr("baseTimeUnit", x.time_unit.bpsim_name());
    // BPSim's duration includes the warm-up.
    sp.push(param(
        "Duration",
        s("FloatingParameter").with_attr("value", &num(x.warmup + x.duration)),
    ));
    if x.warmup > 0.0 {
        sp.push(param(
            "Warmup",
            s("FloatingParameter").with_attr("value", &num(x.warmup)),
        ));
    }
    let mut scenario = s("Scenario")
        .with_attr("id", &ids.fresh("scenario"))
        .with_attr(
            "name",
            if model.name.is_empty() {
                "Scenario"
            } else {
                &model.name
            },
        )
        .with_attr("vendor", "StochCraft")
        .with_attr("version", env!("CARGO_PKG_VERSION"))
        .with_child(sp);
    for e in &model.elements {
        let mut ep = s("ElementParameters").with_attr("elementRef", e.id.as_str());
        match &e.kind {
            ElementKind::Source {
                interarrival,
                limit,
            } => {
                let mut cp = s("ControlParameters")
                    .with_child(param("InterTriggerTimer", dist_element(interarrival)));
                if let Some(n) = limit {
                    cp.push(param(
                        "TriggerCount",
                        s("NumericParameter").with_attr("value", &n.to_string()),
                    ));
                }
                ep.push(cp);
            }
            ElementKind::Server { service, .. } => {
                ep.push(
                    s("TimeParameters").with_child(param("ProcessingTime", dist_element(service))),
                );
                let rid = resource_of.get(e.id.as_str()).cloned().unwrap_or_default();
                let selection = s("ExpressionParameter")
                    .with_attr("value", &format!("getResource('{rid}', 1)"));
                ep.push(s("ResourceParameters").with_child(param("Selection", selection)));
            }
            ElementKind::Delay { duration } => {
                ep.push(
                    s("TimeParameters").with_child(param("ProcessingTime", dist_element(duration))),
                );
            }
            ElementKind::Sink => continue,
        }
        scenario.push(ep);
    }
    for e in &model.elements {
        if let (ElementKind::Server { capacity, .. }, Some(rid)) =
            (&e.kind, resource_of.get(e.id.as_str()))
        {
            let quantity = s("NumericParameter").with_attr("value", &capacity.to_string());
            scenario.push(
                s("ElementParameters")
                    .with_attr("elementRef", rid)
                    .with_child(s("ResourceParameters").with_child(param("Quantity", quantity))),
            );
        }
    }
    let relationship = b("relationship")
        .with_attr("type", "BPSimData")
        .with_child(b("extensionElements").with_child(s("BPSimData").with_child(scenario)))
        .with_child(b("source").with_text(&defs_id))
        .with_child(b("target").with_text(&defs_id));
    defs.push(relationship);
    xml::write(&Document {
        decl: None,
        prolog: Vec::new(),
        root: defs,
    })
    .into_bytes()
}
