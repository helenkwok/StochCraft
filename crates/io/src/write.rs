//! Writing a model StochCraft created as BPMN 2.0 with embedded BPSim 2.0.
//!
//! Everything is written in the order the BPMN 2.0 and BPSim 2.0 schemas require. Ids are the
//! model's; the ids StochCraft adds (the definitions, process, resources, scenario and diagram
//! shapes) are chosen so they never collide with them, since XML ids are unique per file.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use stochcraft_model::{
    Breakdown, Calendar, DateTime, Dist, ElementKind, Model, Need, Property, PropertyType,
    PropertyValue, ResourcePool, Timed,
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

/// Parameter `name` with a default value (if any), then one value valid for each calendar.
fn timed_param(name: &str, default: Option<Element>, timed: Vec<(&str, Element)>) -> Element {
    let mut p = s(name);
    if let Some(d) = default {
        p.push(d);
    }
    for (c, v) in timed {
        p.push(v.with_attr("validFor", c));
    }
    p
}

/// A source's `InterTriggerTimer`: its time between arrivals, then those of its calendars.
pub(crate) fn timer_param(
    interarrival: &Dist,
    timed: &[Timed<Dist>],
    idle_outside: bool,
) -> Element {
    timed_param(
        "InterTriggerTimer",
        (!idle_outside).then(|| dist_element(interarrival)),
        timed
            .iter()
            .map(|t| (t.calendar.as_str(), dist_element(&t.value)))
            .collect(),
    )
}

/// A pool's `Quantity`: its units outside its shifts, then each shift's.
pub(crate) fn quantity_param(r: &ResourcePool) -> Element {
    timed_param(
        "Quantity",
        Some(numeric(u64::from(r.quantity))),
        r.shifts
            .iter()
            .map(|t| (t.calendar.as_str(), numeric(u64::from(t.value))))
            .collect(),
    )
}

fn boolean(b: bool) -> Element {
    s("BooleanParameter").with_attr("value", if b { "true" } else { "false" })
}

/// A pool's `Availability`, when it is not always available.
pub(crate) fn availability_param(r: &ResourcePool) -> Option<Element> {
    if r.available && r.availability.is_empty() {
        return None;
    }
    Some(timed_param(
        "Availability",
        Some(boolean(r.available)),
        r.availability
            .iter()
            .map(|t| (t.calendar.as_str(), boolean(t.value)))
            .collect(),
    ))
}

/// A BPSim `Calendar` with its events as iCalendar text.
pub(crate) fn calendar_element(c: &Calendar) -> Element {
    let mut e = s("Calendar").with_attr("id", c.id.as_str());
    if !c.name.is_empty() {
        e.set_attr("name", &c.name);
    }
    e.with_text(&stochcraft_model::calendar::write(&c.events))
}

/// The scenario's `Start`.
pub(crate) fn start_param(t: DateTime) -> Element {
    param("Start", s("DateTimeParameter").with_attr("value", &t.iso()))
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

/// A server's breakdowns as StochCraft's vendor extension ([`crate::bpsim::BREAKDOWN`]).
pub(crate) fn breakdown_element(b: &Breakdown) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let part = |local: &str, d: &Dist| {
        Element::new(&format!("stochcraft:{local}"), Some(ns)).with_child(dist_element(d))
    };
    s("VendorExtension")
        .with_attr("name", crate::bpsim::BREAKDOWN)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(
            Element::new("stochcraft:breakdown", Some(ns))
                .with_child(part("timeToFailure", &b.time_to_failure))
                .with_child(part("repairTime", &b.repair)),
        )
}

/// BPSim's `Interruptible`, for a server whose failures interrupt the job under way.
pub(crate) fn interruptible_param(yes: bool) -> Element {
    param("Interruptible", boolean(yes))
}

pub(crate) fn probability_element(p: f64) -> Element {
    s("FloatingParameter").with_attr("value", &num(p))
}

pub(crate) fn condition_element(c: &str) -> Element {
    s("ExpressionParameter").with_attr("value", c)
}

pub(crate) fn size(kind: &ElementKind) -> (f64, f64) {
    match kind {
        ElementKind::Source { .. } | ElementKind::Sink => (36.0, 36.0),
        ElementKind::ExclusiveGateway { .. } | ElementKind::ParallelGateway => (50.0, 50.0),
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
    if let Some(t) = x.start {
        sp.push(start_param(t));
    }
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
                timed,
                idle_outside,
            } => {
                let mut cp = s("ControlParameters").with_child(timer_param(
                    interarrival,
                    timed,
                    *idle_outside,
                ));
                if let Some(n) = limit {
                    cp.push(param("TriggerCount", numeric(*n)));
                }
                ep.push(cp);
            }
            ElementKind::Server {
                service, breakdown, ..
            } => {
                ep.push(
                    s("TimeParameters").with_child(param("ProcessingTime", dist_element(service))),
                );
                let rid = resource_of.get(e.id.as_str()).cloned().unwrap_or_default();
                let selection = s("ExpressionParameter")
                    .with_attr("value", &format!("getResource('{rid}', 1)"));
                ep.push(s("ResourceParameters").with_child(param("Selection", selection)));
                if breakdown.as_ref().is_some_and(|b| b.interruptible) {
                    ep.push(s("PriorityParameters").with_child(interruptible_param(true)));
                }
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
        if let ElementKind::Server {
            breakdown: Some(b), ..
        } = &e.kind
        {
            ep.push(breakdown_element(b));
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
        let mut rp = s("ResourceParameters");
        if let Some(a) = availability_param(r) {
            rp.push(a);
        }
        rp.push(quantity_param(r));
        for role in &r.roles {
            rp.push(param("Role", s("StringParameter").with_attr("value", role)));
        }
        scenario.push(
            s("ElementParameters")
                .with_attr("elementRef", r.id.as_str())
                .with_child(rp),
        );
    }
    for c in &model.calendars {
        scenario.push(calendar_element(c));
    }
    let relationship = b("relationship")
        .with_attr("type", "BPSimData")
        .with_child(b("extensionElements").with_child(s("BPSimData").with_child(scenario)))
        .with_child(b("source").with_text(&defs_id))
        .with_child(b("target").with_text(&defs_id));
    defs.push(relationship);
    xml::write(&Document::new(defs)).into_bytes()
}
