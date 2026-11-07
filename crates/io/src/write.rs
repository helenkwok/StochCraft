//! Writing a model StochCraft created as BPMN 2.0 with embedded BPSim 2.0.
//!
//! Everything is written in the order the BPMN 2.0 and BPSim 2.0 schemas require. Ids are the
//! model's; the ids StochCraft adds (the definitions, process, resources, scenario and diagram
//! shapes) are chosen so they never collide with them, since XML ids are unique per file.

use std::collections::{BTreeMap, BTreeSet};

use stochcraft_model::{
    Bounds, Dist, ElementKind, Model, Need, Property, PropertyType, PropertyValue,
};

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

/// A BPSim whole-number value.
pub(crate) fn numeric(n: u64) -> Element {
    s("NumericParameter").with_attr("value", &n.to_string())
}

/// A property's `Property` parameter: its name, its declared type and its value (a duration
/// as a number in the scenario's time unit).
pub(crate) fn property_element(p: &Property) -> Element {
    let mut e = s("Property").with_attr("name", &p.name);
    if let Some(t) = p.ty {
        e = e.with_attr("type", t.bpsim_name());
    }
    let value = match &p.value {
        PropertyValue::Number(n) if p.ty == Some(PropertyType::Long) && n.fract() == 0.0 => {
            s("NumericParameter").with_attr("value", &num(*n))
        }
        PropertyValue::Number(n) => s("FloatingParameter").with_attr("value", &num(*n)),
        PropertyValue::Boolean(b) => {
            s("BooleanParameter").with_attr("value", if *b { "true" } else { "false" })
        }
        PropertyValue::Text(t) if p.ty == Some(PropertyType::DateTime) => {
            s("DateTimeParameter").with_attr("value", t)
        }
        PropertyValue::Text(t) => s("StringParameter").with_attr("value", t),
        PropertyValue::Random(d) => dist_element(d),
        PropertyValue::Expression(x) => s("ExpressionParameter").with_attr("value", x),
    };
    e.with_child(value)
}

fn param(name: &str, value: Element) -> Element {
    s(name).with_child(value)
}

/// The BPMN element an element kind is written as.
pub(crate) fn bpmn_local(kind: &ElementKind) -> &'static str {
    match kind {
        ElementKind::Source { .. } => "startEvent",
        ElementKind::Sink => "endEvent",
        ElementKind::Server { .. } | ElementKind::SharedTask { .. } | ElementKind::Delay { .. } => {
            "task"
        }
        ElementKind::ExclusiveGateway { .. } => "exclusiveGateway",
        ElementKind::ParallelGateway => "parallelGateway",
    }
}

/// A connection's BPSim `ControlParameters` values: its probability and condition.
/// A server's finite buffer as a StochCraft vendor extension.
pub(crate) fn queue_capacity_element(n: u32) -> Element {
    s("VendorExtension")
        .with_attr("name", crate::bpsim::QUEUE_CAPACITY)
        .with_attr("xmlns:stochcraft", crate::bpsim::STOCHCRAFT_NS)
        .with_attr("stochcraft:value", &n.to_string())
}

pub(crate) fn probability_element(p: f64) -> Element {
    s("FloatingParameter").with_attr("value", &num(p))
}

pub(crate) fn condition_element(c: &str) -> Element {
    s("ExpressionParameter").with_attr("value", c)
}

pub(crate) fn size(kind: &ElementKind) -> (f64, f64) {
    kind.shape_size()
}

/// A shape's `dc:Bounds`.
pub(crate) fn bounds_element(b: &Bounds) -> Element {
    Element::new("dc:Bounds", Some(DC))
        .with_attr("x", &num(b.x))
        .with_attr("y", &num(b.y))
        .with_attr("width", &num(b.width))
        .with_attr("height", &num(b.height))
}

/// Where an edge between two shapes starts and ends: from the middle of the first's right side
/// to the middle of the second's left, or between top and bottom when one is wholly above the
/// other.
pub fn edge_ends(a: &Bounds, b: &Bounds) -> ((f64, f64), (f64, f64)) {
    let (ax, ay) = a.centre();
    let (bx, by) = b.centre();
    if b.x >= a.x + a.width || b.x + b.width <= a.x {
        if bx >= ax {
            ((a.x + a.width, ay), (b.x, by))
        } else {
            ((a.x, ay), (b.x + b.width, by))
        }
    } else if by >= ay {
        ((ax, a.y + a.height), (bx, b.y))
    } else {
        ((ax, a.y), (bx, b.y + b.height))
    }
}

/// The model as a BPMN 2.0 file with embedded BPSim 2.0 data.
pub fn write_new(model: &Model) -> Vec<u8> {
    let mut ids = Ids(model
        .elements
        .iter()
        .map(|e| e.id.as_str().to_owned())
        .chain(model.connections.iter().map(|c| c.id.as_str().to_owned()))
        .chain(model.resources.iter().map(|r| r.id.as_str().to_owned()))
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

    // The shared pools, under their own ids.
    for r in &model.resources {
        defs.push(
            b("resource")
                .with_attr("id", r.id.as_str())
                .with_attr("name", &r.name),
        );
    }
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
        let mut node = b(bpmn_local(&e.kind))
            .with_attr("id", e.id.as_str())
            .with_attr("name", &e.name);
        if let ElementKind::ExclusiveGateway { default: Some(d) } = &e.kind {
            node = node.with_attr("default", d.as_str());
        }
        for c in model.connections.iter().filter(|c| c.to == e.id) {
            node.push(b("incoming").with_text(c.id.as_str()));
        }
        for c in model.connections.iter().filter(|c| c.from == e.id) {
            node.push(b("outgoing").with_text(c.id.as_str()));
        }
        if let Some(rid) = resource_of.get(e.id.as_str()) {
            node.push(b("performer").with_child(b("resourceRef").with_text(rid)));
        }
        // A shared task's pools named outright, as performers, for other BPMN tools; what it
        // needs is its selection.
        if let ElementKind::SharedTask { needs, .. } = &e.kind {
            let mut named: Vec<&str> = Vec::new();
            for n in needs {
                if let Need::Resource { id, .. } = n
                    && !named.contains(&id.as_str())
                {
                    named.push(id.as_str());
                    node.push(b("performer").with_child(b("resourceRef").with_text(id.as_str())));
                }
            }
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
    let placed = model.placed();
    let mut plane = Element::new("bpmndi:BPMNPlane", Some(BPMNDI))
        .with_attr("id", &ids.fresh("plane"))
        .with_attr("bpmnElement", &process_id);
    for e in &model.elements {
        let Some(b) = placed.get(&e.id) else { continue };
        plane.push(
            Element::new("bpmndi:BPMNShape", Some(BPMNDI))
                .with_attr("id", &ids.fresh(&format!("{}_di", e.id)))
                .with_attr("bpmnElement", e.id.as_str())
                .with_child(bounds_element(b)),
        );
    }
    for c in &model.connections {
        let (Some(a), Some(b)) = (placed.get(&c.from), placed.get(&c.to)) else {
            continue;
        };
        let waypoint = |(x, y): (f64, f64)| {
            Element::new("di:waypoint", Some(DI))
                .with_attr("x", &num(x))
                .with_attr("y", &num(y))
        };
        let (from, to) = edge_ends(a, b);
        plane.push(
            Element::new("bpmndi:BPMNEdge", Some(BPMNDI))
                .with_attr("id", &ids.fresh(&format!("{}_di", c.id)))
                .with_attr("bpmnElement", c.id.as_str())
                .with_child(waypoint(from))
                .with_child(waypoint(to)),
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
                    cp.push(param("TriggerCount", numeric(*n)));
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
            ElementKind::SharedTask {
                service,
                needs,
                priority,
            } => {
                ep.push(
                    s("TimeParameters").with_child(param("ProcessingTime", dist_element(service))),
                );
                let selection = condition_element(&crate::selection::format(needs));
                ep.push(s("ResourceParameters").with_child(param("Selection", selection)));
                if *priority != 0.0 {
                    ep.push(
                        s("PriorityParameters")
                            .with_child(param("Priority", probability_element(*priority))),
                    );
                }
            }
            ElementKind::Delay { duration } => {
                ep.push(
                    s("TimeParameters").with_child(param("ProcessingTime", dist_element(duration))),
                );
            }
            ElementKind::Sink
            | ElementKind::ExclusiveGateway { .. }
            | ElementKind::ParallelGateway => {}
        }
        if !e.properties.is_empty() {
            let mut pp = s("PropertyParameters");
            for p in &e.properties {
                pp.push(property_element(p));
            }
            ep.push(pp);
        }
        if let ElementKind::Server {
            queue_capacity: Some(q),
            ..
        } = e.kind
        {
            ep.push(queue_capacity_element(q));
        }
        if ep.elements().next().is_none() {
            continue;
        }
        scenario.push(ep);
    }
    for c in &model.connections {
        if c.probability.is_none() && c.condition.is_none() {
            continue;
        }
        let mut cp = s("ControlParameters");
        if let Some(p) = c.probability {
            cp.push(param("Probability", probability_element(p)));
        }
        if let Some(x) = &c.condition {
            cp.push(param("Condition", condition_element(x)));
        }
        scenario.push(
            s("ElementParameters")
                .with_attr("elementRef", c.id.as_str())
                .with_child(cp),
        );
    }
    for e in &model.elements {
        if let (ElementKind::Server { capacity, .. }, Some(rid)) =
            (&e.kind, resource_of.get(e.id.as_str()))
        {
            let quantity = numeric(u64::from(*capacity));
            scenario.push(
                s("ElementParameters")
                    .with_attr("elementRef", rid)
                    .with_child(s("ResourceParameters").with_child(param("Quantity", quantity))),
            );
        }
    }
    for r in &model.resources {
        let mut rp =
            s("ResourceParameters").with_child(param("Quantity", numeric(u64::from(r.quantity))));
        for role in &r.roles {
            rp.push(param("Role", s("StringParameter").with_attr("value", role)));
        }
        scenario.push(
            s("ElementParameters")
                .with_attr("elementRef", r.id.as_str())
                .with_child(rp),
        );
    }
    let relationship = b("relationship")
        .with_attr("type", "BPSimData")
        .with_child(b("extensionElements").with_child(s("BPSimData").with_child(scenario)))
        .with_child(b("source").with_text(&defs_id))
        .with_child(b("target").with_text(&defs_id));
    defs.push(relationship);
    xml::write(&Document::new(defs)).into_bytes()
}
