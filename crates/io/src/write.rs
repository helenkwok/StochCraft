//! Writing a model StochCraft created as BPMN 2.0 with embedded BPSim 2.0.
//!
//! Everything is written in the order the BPMN 2.0 and BPSim 2.0 schemas require. Ids are the
//! model's; the ids StochCraft adds (the definitions, process, resources, scenario and diagram
//! shapes) are chosen so they never collide with them, since XML ids are unique per file.

use std::collections::{BTreeMap, BTreeSet};

use stochcraft_model::{
    BalkJoin, Balking, Bounds, Breakdown, Calendar, ConveyorShape, DateTime, Demand, Dist,
    ElementKind, Estimate, Experiment, HealthCheck, LengthUnit, Model, MonteCarlo, Need,
    Observation, Observed, OnShiftEnd, OnTakeout, Patience, PatienceUntil, PlaceWhileOut, Power,
    ProcessingBy, ProcessingByResource, Property, PropertyType, PropertyValue, QueueLimit,
    ResourcePool, Routing, Scrap, Sequential, Setup, Stop, TimeInSystem, Timed, Transfer,
};

use stochcraft_model::dmn::DecisionUse;

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
        PropertyValue::Choice(values) => {
            let mut e = s("UserDistribution").with_attr("discrete", "true");
            for (share, v) in values {
                e.push(
                    s("UserDistributionDataPoint")
                        .with_attr("probability", &num(*share))
                        .with_child(s("StringParameter").with_attr("value", v)),
                );
            }
            e
        }
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
    units_param(r.quantity, &r.shifts)
}

/// A resource's `Quantity`: `quantity` units outside its shifts, then each shift's (a pool's,
/// or a task's own by shift).
pub(crate) fn units_param(quantity: u32, shifts: &[Timed<u32>]) -> Element {
    timed_param(
        "Quantity",
        Some(numeric(u64::from(quantity))),
        shifts
            .iter()
            .map(|t| (t.calendar.as_str(), numeric(u64::from(t.value))))
            .collect(),
    )
}

/// What happens to a job under way on a server's unit that goes off shift, as StochCraft's
/// vendor extension ([`crate::bpsim::ON_SHIFT_END`]); `None` for the default (it finishes).
pub(crate) fn on_shift_end_element(o: OnShiftEnd) -> Option<Element> {
    (o != OnShiftEnd::Finish).then(|| vendor_value_element(crate::bpsim::ON_SHIFT_END, o.name()))
}

/// The pool marker on a server's own resource whose units follow shifts, so it reads back as
/// the task's own and not as a pool ([`crate::bpsim::POOL`] `false`); `None` without shifts.
pub(crate) fn own_marker(shifts: &[Timed<u32>]) -> Option<Element> {
    (!shifts.is_empty()).then(|| vendor_value_element(crate::bpsim::POOL, "false"))
}

fn boolean(b: bool) -> Element {
    s("BooleanParameter").with_attr("value", if b { "true" } else { "false" })
}

/// A pool's or a server's resource's `Availability` (`available` outside the calendars of
/// `availability`), when it is not always available.
pub(crate) fn availability_param(available: bool, availability: &[Timed<bool>]) -> Option<Element> {
    if available && availability.is_empty() {
        return None;
    }
    Some(timed_param(
        "Availability",
        Some(boolean(available)),
        availability
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

/// The BPMN element an element kind is written as.
pub(crate) fn bpmn_local(kind: &ElementKind) -> &'static str {
    match kind {
        ElementKind::Source { .. } => "startEvent",
        ElementKind::Sink { .. } => "endEvent",
        ElementKind::Server { .. }
        | ElementKind::SharedTask { .. }
        | ElementKind::Delay { .. }
        | ElementKind::Batch { .. }
        | ElementKind::Unbatch
        | ElementKind::Conveyor { .. }
        | ElementKind::Transport { .. } => "task",
        ElementKind::ExclusiveGateway { .. } => "exclusiveGateway",
        ElementKind::ParallelGateway => "parallelGateway",
    }
}

/// A connection's BPSim `ControlParameters` values: its probability and condition.
/// A server's finite buffer as a StochCraft vendor extension: `n` places waiting, or how it is
/// set otherwise (`limit`): per working server, or by timetable (the values `validFor` their
/// calendars in a `stochcraft:byShift`, `n` outside them).
pub(crate) fn queue_capacity_element(n: u32, limit: &QueueLimit) -> Element {
    let e = vendor_value_element(crate::bpsim::QUEUE_CAPACITY, &n.to_string());
    match limit {
        QueueLimit::Fixed => e,
        QueueLimit::PerWorkingServer => e.with_attr("stochcraft:perWorkingServer", "true"),
        QueueLimit::ByShift(timed) => {
            let mut by = Element::new("stochcraft:byShift", Some(crate::bpsim::STOCHCRAFT_NS));
            for t in timed {
                by.push(numeric(u64::from(t.value)).with_attr("validFor", t.calendar.as_str()));
            }
            e.with_child(by)
        }
    }
}

/// The scenario parameters that say experiment `x`: runs, seed, time unit, result frequency,
/// start, duration (the warm-up included) and warm-up, and a Monte Carlo model's inputs.
pub(crate) fn scenario_parameters(x: &Experiment, mc: Option<&MonteCarlo>) -> Element {
    let mut sp = s("ScenarioParameters")
        .with_attr("replication", &x.replications.to_string())
        // `xs:long` is signed; the seed's 64 bits are kept and read back unchanged.
        .with_attr("seed", &(x.seed as i64).to_string())
        .with_attr("baseTimeUnit", x.time_unit.bpsim_name());
    if let Some(f) = x.result_frequency {
        sp = sp.with_attr("baseResultFrequency", &crate::duration::format_seconds(f));
    }
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
    // A Monte Carlo model's inputs: the scenario's own properties, after its times (schema
    // order).
    if let Some(mc) = mc {
        sp.push(crate::mc::properties_element(mc));
    }
    sp
}

/// The scenario's vendor extensions that say experiment `x`: the length unit and layout scale
/// (unless the default) and sequential runs. BPSim allows vendor extensions only as children of
/// the `Scenario`, after its calendars, never inside `ScenarioParameters`.
pub(crate) fn scenario_extensions(x: &Experiment) -> Vec<Element> {
    let mut out = Vec::new();
    if let Some(u) = length_unit_element(x.length_unit) {
        out.push(u);
    }
    if let Some(u) = layout_scale_element(x.layout_scale) {
        out.push(u);
    }
    if let Some(q) = &x.sequential {
        out.push(sequential_element(q));
    }
    if !x.observed.is_empty() {
        out.push(observed_element(&x.observed));
    }
    out
}

/// A StochCraft vendor extension holding one value.
pub(crate) fn vendor_value_element(name: &str, value: &str) -> Element {
    s("VendorExtension")
        .with_attr("name", name)
        .with_attr("xmlns:stochcraft", crate::bpsim::STOCHCRAFT_NS)
        .with_attr("stochcraft:value", value)
}

/// The decision table a block uses, as StochCraft's vendor extension ([`crate::bpsim::DECISION`]).
pub(crate) fn decision_element(u: &DecisionUse) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:decision", Some(ns))
        .with_attr("file", &u.file)
        .with_attr("decision", &u.decision);
    if let Some(r) = &u.route {
        body = body.with_attr("route", r);
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::DECISION)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// A block's starting contents as a StochCraft vendor extension: `n` entities inside as each
/// replication starts.
pub(crate) fn starting_element(n: u32) -> Element {
    vendor_value_element(crate::bpsim::STARTING_CONTENTS, &n.to_string())
}

/// The vendor extension that makes a task a batch or a split, if `kind` is one.
pub(crate) fn batching_element(kind: &ElementKind) -> Option<Element> {
    match kind {
        ElementKind::Batch { size } => Some(vendor_value_element(
            crate::bpsim::BATCH_SIZE,
            &size.to_string(),
        )),
        ElementKind::Unbatch => Some(vendor_value_element(crate::bpsim::UNBATCH, "true")),
        _ => None,
    }
}

/// A conveyor as StochCraft's vendor extension ([`crate::bpsim::CONVEYOR`]), if `kind` is one.
pub(crate) fn conveyor_element(kind: &ElementKind) -> Option<Element> {
    let ElementKind::Conveyor {
        length,
        speed,
        accumulating,
        item_length,
        gap,
        shape,
    } = kind
    else {
        return None;
    };
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:conveyor", Some(ns))
        .with_attr("length", &num(*length))
        .with_attr("speed", &num(*speed))
        .with_attr("accumulating", if *accumulating { "true" } else { "false" })
        .with_attr("itemLength", &num(*item_length))
        .with_attr("gap", &num(*gap));
    match shape {
        ConveyorShape::Straight => {}
        ConveyorShape::Curve { radius, angle } => {
            body = body
                .with_attr("shape", "curve")
                .with_attr("radius", &num(*radius))
                .with_attr("angle", &num(*angle));
        }
        ConveyorShape::Spiral {
            radius,
            rise,
            turns,
        } => {
            body = body
                .with_attr("shape", "spiral")
                .with_attr("radius", &num(*radius))
                .with_attr("rise", &num(*rise))
                .with_attr("turns", &num(*turns));
        }
    }
    Some(
        s("VendorExtension")
            .with_attr("name", crate::bpsim::CONVEYOR)
            .with_attr("xmlns:stochcraft", ns)
            .with_child(body),
    )
}

/// The scenario's length unit as StochCraft's vendor extension ([`crate::bpsim::LENGTH_UNIT`]),
/// unless it is the default, metres.
pub(crate) fn length_unit_element(unit: LengthUnit) -> Option<Element> {
    (unit != LengthUnit::default())
        .then(|| vendor_value_element(crate::bpsim::LENGTH_UNIT, unit.name()))
}

/// The scenario's layout scale as StochCraft's vendor extension ([`crate::bpsim::LAYOUT_SCALE`]),
/// unless it is the default, 1.
pub(crate) fn layout_scale_element(scale: f64) -> Option<Element> {
    ((scale - 1.0).abs() > f64::EPSILON)
        .then(|| vendor_value_element(crate::bpsim::LAYOUT_SCALE, &num(scale)))
}

/// Sequential replications as StochCraft's vendor extension ([`crate::bpsim::SEQUENTIAL`]):
/// every setting written, so the file reads back the same whatever the defaults become.
pub(crate) fn sequential_element(x: &Sequential) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:sequential", Some(ns))
        .with_attr("relative", &num(x.relative))
        .with_attr("points", &num(x.points))
        .with_attr("max", &x.max.to_string());
    if let Some(m) = &x.measure {
        body = body
            .with_attr("element", m.element.as_str())
            .with_attr("metric", &m.metric);
        if let Some(h) = m.half_width {
            body = body.with_attr("halfWidth", &num(h));
        }
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::SEQUENTIAL)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// The observed data as StochCraft's vendor extension ([`crate::bpsim::OBSERVED`]): one
/// `observation` for each result, with its observed value (and the half-width of its own
/// interval), or its bound.
pub(crate) fn observed_element(list: &[Observation]) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:observed", Some(ns));
    for o in list {
        let mut e = Element::new("stochcraft:observation", Some(ns))
            .with_attr("element", o.element.as_str())
            .with_attr("metric", &o.metric);
        e = match o.target {
            Observed::Value {
                value,
                half_width,
                tolerance,
            } => {
                let mut e = e.with_attr("value", &num(value));
                if let Some(h) = half_width {
                    e = e.with_attr("halfWidth", &num(h));
                }
                if let Some(t) = tolerance {
                    e = e.with_attr("tolerance", &num(t));
                }
                e
            }
            Observed::AtMost(x) => e.with_attr("atMost", &num(x)),
            Observed::AtLeast(x) => e.with_attr("atLeast", &num(x)),
        };
        body.push(e);
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::OBSERVED)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// What clears the observed data an inherited scenario sets.
pub(crate) fn observed_off_element() -> Element {
    vendor_value_element(crate::bpsim::OBSERVED, "off")
}

/// What turns off the sequential replications an inherited scenario sets.
pub(crate) fn sequential_off_element() -> Element {
    vendor_value_element(crate::bpsim::SEQUENTIAL, "off")
}

/// A transport as StochCraft's vendor extension ([`crate::bpsim::TRANSPORT`]), if `kind` is one.
pub(crate) fn transport_element(kind: &ElementKind) -> Option<Element> {
    let ElementKind::Transport { pool, load, unload } = kind else {
        return None;
    };
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let part = |local: &str, d: &Dist| {
        Element::new(&format!("stochcraft:{local}"), Some(ns)).with_child(dist_element(d))
    };
    let body = Element::new("stochcraft:transport", Some(ns))
        .with_attr("pool", pool.as_str())
        .with_child(part("load", load))
        .with_child(part("unload", unload));
    Some(
        s("VendorExtension")
            .with_attr("name", crate::bpsim::TRANSPORT)
            .with_attr("xmlns:stochcraft", ns)
            .with_child(body),
    )
}

/// A pool's mobility as StochCraft's vendor extension ([`crate::bpsim::MOBILITY`]), if it is
/// mobile.
pub(crate) fn mobility_element(pool: &ResourcePool) -> Option<Element> {
    let m = pool.mobility.as_ref()?;
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let body = Element::new("stochcraft:mobility", Some(ns))
        .with_attr("speedEmpty", &num(m.speed_empty))
        .with_attr("speedLoaded", &num(m.speed_loaded))
        .with_attr("home", m.home.as_str())
        .with_attr("afterJob", m.after_job.name());
    Some(
        s("VendorExtension")
            .with_attr("name", crate::bpsim::MOBILITY)
            .with_attr("xmlns:stochcraft", ns)
            .with_child(body),
    )
}

/// A connection's bill-of-materials quantity as a vendor extension.
pub(crate) fn quantity_element(q: u32) -> Element {
    vendor_value_element(crate::bpsim::QUANTITY, &q.to_string())
}

/// A server's breakdowns as StochCraft's vendor extension ([`crate::bpsim::BREAKDOWN`]).
pub(crate) fn breakdown_element(b: &Breakdown) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let part = |local: &str, d: &Dist| {
        Element::new(&format!("stochcraft:{local}"), Some(ns)).with_child(dist_element(d))
    };
    let mut body = Element::new("stochcraft:breakdown", Some(ns));
    if !b.hands_on_while_down {
        body = body.with_attr("handsOnWhileDown", "false");
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::BREAKDOWN)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(
            body.with_child(part("timeToFailure", &b.time_to_failure))
                .with_child(part("repairTime", &b.repair)),
        )
}

/// A server's power per state as StochCraft's vendor extension ([`crate::bpsim::POWER`]).
pub(crate) fn power_element(p: &Power) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:power", Some(ns))
        .with_attr("processing", &num(p.processing))
        .with_attr("idle", &num(p.idle));
    for (name, x) in [
        ("blocked", p.blocked),
        ("failed", p.failed),
        ("offShift", p.off_shift),
    ] {
        if let Some(x) = x {
            body = body.with_attr(name, &num(x));
        }
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::POWER)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// A server's changeovers as StochCraft's vendor extension ([`crate::bpsim::CHANGEOVER`]).
pub(crate) fn changeover_element(x: &Setup) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:changeover", Some(ns))
        .with_attr("property", x.on_change_of.as_deref().unwrap_or_default())
        .with_child(Element::new("stochcraft:time", Some(ns)).with_child(dist_element(&x.time)));
    for c in &x.by_change {
        body.push(
            Element::new("stochcraft:change", Some(ns))
                .with_attr("from", &c.from)
                .with_attr("to", &c.to)
                .with_child(dist_element(&c.time)),
        );
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::CHANGEOVER)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// A task's processing times by property value as StochCraft's vendor extension
/// ([`crate::bpsim::PROCESSING_TIME_BY`]).
pub(crate) fn processing_by_element(x: &ProcessingBy) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body =
        Element::new("stochcraft:processingTimeBy", Some(ns)).with_attr("property", &x.property);
    for b in &x.by_value {
        body.push(
            Element::new("stochcraft:time", Some(ns))
                .with_attr("value", &b.value)
                .with_child(dist_element(&b.time)),
        );
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::PROCESSING_TIME_BY)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// A shared task's service times by serving pool as StochCraft's vendor extension
/// ([`crate::bpsim::PROCESSING_TIME_BY_RESOURCE`]).
pub(crate) fn processing_by_resource_element(x: &ProcessingByResource) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:processingTimeByResource", Some(ns));
    for b in &x.by_resource {
        body.push(
            Element::new("stochcraft:time", Some(ns))
                .with_attr("resource", b.resource.as_str())
                .with_child(dist_element(&b.time)),
        );
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::PROCESSING_TIME_BY_RESOURCE)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// A sink's customer demand as StochCraft's vendor extension ([`crate::bpsim::DEMAND`]).
pub(crate) fn demand_element(d: &Demand) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:demand", Some(ns))
        .with_attr("quantity", &num(d.quantity))
        .with_attr("per", d.per.name());
    if let Some(c) = &d.calendar {
        body = body.with_attr("calendar", c.as_str());
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::DEMAND)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// A sink's time in system by percentile as StochCraft's vendor extension
/// ([`crate::bpsim::TIME_IN_SYSTEM`]): the target time, if any, as a BPSim value in the
/// scenario's time unit.
pub(crate) fn time_in_system_element(x: &TimeInSystem) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:timeInSystem", Some(ns));
    if let Some(c) = &x.calendar {
        body = body.with_attr("calendar", c.as_str());
    }
    if let Some(w) = x.within {
        body.push(
            Element::new("stochcraft:within", Some(ns))
                .with_child(dist_element(&Dist::Constant(w))),
        );
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::TIME_IN_SYSTEM)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// A sink's scrap as StochCraft's vendor extension ([`crate::bpsim::SCRAP`]).
pub(crate) fn scrap_element(x: &Scrap) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:scrap", Some(ns));
    if let Some(c) = &x.charged_to {
        body = body.with_attr("chargedTo", c.as_str());
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::SCRAP)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// Rework on a connection as StochCraft's vendor extension ([`crate::bpsim::REWORK`]).
pub(crate) fn rework_element() -> Element {
    vendor_value_element(crate::bpsim::REWORK, "true")
}

/// A way for entities that give up as StochCraft's vendor extension
/// ([`crate::bpsim::ABANDONED`]).
pub(crate) fn abandoned_element() -> Element {
    vendor_value_element(crate::bpsim::ABANDONED, "true")
}

/// A way for entities turned away as StochCraft's vendor extension
/// ([`crate::bpsim::TURNED_AWAY`]).
pub(crate) fn turned_away_element() -> Element {
    vendor_value_element(crate::bpsim::TURNED_AWAY, "true")
}

/// A way for entities that balk as StochCraft's vendor extension ([`crate::bpsim::BALKED`]).
pub(crate) fn balked_element() -> Element {
    vendor_value_element(crate::bpsim::BALKED, "true")
}

/// A server's balking as StochCraft's vendor extension ([`crate::bpsim::BALKING`]). `rule` is
/// written only for `reciprocal`, and `countsInService` only when true.
pub(crate) fn balking_element(b: &Balking) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:balking", Some(ns));
    if b.counts_in_service {
        body = body.with_attr("countsInService", "true");
    }
    match &b.join {
        BalkJoin::Reciprocal => body = body.with_attr("rule", "reciprocal"),
        BalkJoin::Table(rows) => {
            for r in rows {
                body = body.with_child(
                    Element::new("stochcraft:join", Some(ns))
                        .with_attr("waiting", &r.waiting.to_string())
                        .with_attr("probability", &num(r.probability)),
                );
            }
        }
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::BALKING)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
}

/// A server's patience as StochCraft's vendor extension ([`crate::bpsim::PATIENCE`]). `until`
/// is written only when it is not the default (`start`).
pub(crate) fn patience_element(p: &Patience) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:patience", Some(ns));
    if p.until != PatienceUntil::Start {
        body = body.with_attr("until", p.until.name());
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::PATIENCE)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body.with_child(dist_element(&p.time)))
}

/// A server's health check as StochCraft's vendor extension ([`crate::bpsim::HEALTH_CHECK`]).
/// `keep`, `onTakeout` and `placeWhileOut` are written only when set and not the default
/// (`retry`, `reserved`); a limit of infinity is written `INF`, as XML Schema spells it.
pub(crate) fn health_check_element(h: &HealthCheck) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let part = |local: &str, d: &Dist| {
        Element::new(&format!("stochcraft:{local}"), Some(ns)).with_child(dist_element(d))
    };
    let limit = if h.limit == f64::INFINITY {
        "INF".to_owned()
    } else {
        num(h.limit)
    };
    let mut body = Element::new("stochcraft:healthCheck", Some(ns)).with_attr("limit", &limit);
    if let Some(k) = h.keep {
        body = body.with_attr("keep", &k.to_string());
    }
    if h.on_takeout != OnTakeout::Retry {
        body = body.with_attr("onTakeout", h.on_takeout.name());
    }
    if h.place_while_out != PlaceWhileOut::Reserved {
        body = body.with_attr("placeWhileOut", h.place_while_out.name());
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::HEALTH_CHECK)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(
            body.with_child(part("interval", &h.interval))
                .with_child(part("restart", &h.restart)),
        )
}

/// A server's planned time to make one item as StochCraft's vendor extension
/// ([`crate::bpsim::PLANNED_RUN_TIME`]).
pub(crate) fn planned_element(d: &Dist) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    s("VendorExtension")
        .with_attr("name", crate::bpsim::PLANNED_RUN_TIME)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(Element::new("stochcraft:plannedRunTime", Some(ns)).with_child(dist_element(d)))
}

/// A server's transfer time as StochCraft's vendor extension ([`crate::bpsim::TRANSFER_TIME`]).
pub(crate) fn transfer_element(t: &Transfer) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let mut body = Element::new("stochcraft:transferTime", Some(ns));
    if !t.holds_sender {
        body = body.with_attr("holdsSender", "false");
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::TRANSFER_TIME)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body.with_child(dist_element(&t.time)))
}

/// An exclusive gateway's routing rule as StochCraft's vendor extension
/// ([`crate::bpsim::ROUTING`]), if it is not BPMN's own.
pub(crate) fn routing_element(routing: Routing) -> Option<Element> {
    match routing {
        Routing::Standard => None,
        Routing::ShortestQueue => {
            Some(vendor_value_element(crate::bpsim::ROUTING, "shortestQueue"))
        }
        Routing::Divert => Some(vendor_value_element(crate::bpsim::ROUTING, "divert")),
    }
}

/// A terminating experiment's stop condition, as a vendor extension on the element it watches.
pub(crate) fn stop_element(stop: &Stop) -> Element {
    match stop {
        Stop::Count { count, .. } => {
            vendor_value_element(crate::bpsim::STOP_COUNT, &count.to_string())
        }
        Stop::Empty { .. } => vendor_value_element(crate::bpsim::STOP_WHEN_EMPTY, "true"),
    }
}

/// A three-point estimate as StochCraft's vendor extension ([`crate::bpsim::ESTIMATE`]).
pub(crate) fn estimate_element(e: &Estimate) -> Element {
    let ns = crate::bpsim::STOCHCRAFT_NS;
    let t = &e.three_point;
    let mut body = Element::new("stochcraft:estimate", Some(ns))
        .with_attr("family", t.family.name())
        .with_attr("min", &num(t.min))
        .with_attr("mode", &num(t.mode))
        .with_attr(t.upper_is.name(), &num(t.upper));
    if !e.source.is_empty() {
        body = body.with_attr("source", &e.source);
    }
    s("VendorExtension")
        .with_attr("name", crate::bpsim::ESTIMATE)
        .with_attr("xmlns:stochcraft", ns)
        .with_child(body)
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
        if let ElementKind::ExclusiveGateway {
            default: Some(d), ..
        } = &e.kind
        {
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
    // A Monte Carlo model is a file with no process (D15), and so no diagram; so is an
    // agent-based model (D16).
    let monte_carlo = model.is_monte_carlo() || model.is_agents();
    if !monte_carlo {
        defs.push(process);
    }

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
    if !monte_carlo {
        defs.push(
            Element::new("bpmndi:BPMNDiagram", Some(BPMNDI))
                .with_attr("id", &ids.fresh("diagram"))
                .with_child(plane),
        );
    }

    // The simulation data, in BPSim schema order.
    let sp = scenario_parameters(&model.experiment, model.monte_carlo.as_ref());
    // The layout as far as the model can hold it (`Site::kept`).
    let site = model.site.kept(model);
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
                service,
                breakdown,
                setup,
                ..
            } => {
                // A setup before every job is BPSim's own SetUpTime (a changeover is the vendor
                // extension below).
                let mut tp = s("TimeParameters");
                if let Some(x) = setup.as_ref().filter(|x| x.on_change_of.is_none()) {
                    tp.push(param("SetUpTime", dist_element(&x.time)));
                }
                ep.push(tp.with_child(param("ProcessingTime", dist_element(service))));
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
                ..
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
            ElementKind::Delay { duration, .. } => {
                ep.push(
                    s("TimeParameters").with_child(param("ProcessingTime", dist_element(duration))),
                );
            }
            ElementKind::Sink { .. }
            | ElementKind::ExclusiveGateway { .. }
            | ElementKind::ParallelGateway
            | ElementKind::Batch { .. }
            | ElementKind::Unbatch
            | ElementKind::Conveyor { .. }
            | ElementKind::Transport { .. } => {}
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
            queue_limit,
            ..
        } = &e.kind
        {
            ep.push(queue_capacity_element(*q, queue_limit));
        }
        if let ElementKind::Server {
            breakdown: Some(b), ..
        } = &e.kind
        {
            ep.push(breakdown_element(b));
        }
        if let ElementKind::Server {
            transfer: Some(t), ..
        } = &e.kind
        {
            ep.push(transfer_element(t));
        }
        if let ElementKind::ExclusiveGateway { routing, .. } = &e.kind
            && let Some(x) = routing_element(*routing)
        {
            ep.push(x);
        }
        if let ElementKind::Server { power: Some(p), .. } = &e.kind {
            ep.push(power_element(p));
        }
        if let ElementKind::Server {
            planned_run_time: Some(d),
            ..
        } = &e.kind
        {
            ep.push(planned_element(d));
        }
        if let ElementKind::Server {
            patience: Some(p), ..
        } = &e.kind
        {
            ep.push(patience_element(p));
        }
        if let ElementKind::Server {
            balking: Some(b), ..
        } = &e.kind
        {
            ep.push(balking_element(b));
        }
        if let ElementKind::Server {
            health_check: Some(h),
            ..
        } = &e.kind
        {
            ep.push(health_check_element(h));
        }
        if let ElementKind::Sink { scrap: Some(x), .. } = &e.kind {
            ep.push(scrap_element(x));
        }
        if let ElementKind::Sink {
            time_in_system: Some(x),
            ..
        } = &e.kind
        {
            ep.push(time_in_system_element(x));
        }
        if let ElementKind::Sink {
            demand: Some(d), ..
        } = &e.kind
        {
            ep.push(demand_element(d));
        }
        if let ElementKind::Server { setup: Some(x), .. } = &e.kind
            && x.on_change_of.is_some()
        {
            ep.push(changeover_element(x));
        }
        if let Some(x) = e.kind.processing_by() {
            ep.push(processing_by_element(x));
        }
        if let Some(x) = e.kind.processing_by_resource() {
            ep.push(processing_by_resource_element(x));
        }
        if let ElementKind::Server { on_shift_end, .. } = &e.kind
            && let Some(x) = on_shift_end_element(*on_shift_end)
        {
            ep.push(x);
        }
        if let Some(x) = batching_element(&e.kind) {
            ep.push(x);
        }
        if let Some(x) = conveyor_element(&e.kind) {
            ep.push(x);
        }
        if let Some(x) = transport_element(&e.kind) {
            ep.push(x);
        }
        if let Some(stop) = model
            .experiment
            .stop
            .as_ref()
            .filter(|s| s.element() == &e.id)
        {
            ep.push(stop_element(stop));
        }
        if let Some(x) = model.estimates.get(&e.id) {
            ep.push(estimate_element(x));
        }
        if let Some(n) = model
            .starting
            .get(&e.id)
            .filter(|n| **n > 0 && e.kind.holds_at_start())
        {
            ep.push(starting_element(*n));
        }
        if let Some(u) = model.decisions.get(&e.id) {
            ep.push(decision_element(u));
        }
        // The physical layout (D24): a conveyor's track and a block's place.
        if let Some(t) = site.paths.get(&e.id) {
            ep.push(crate::layout::path_element(t));
        }
        if let Some(p) = site.places.get(&e.id) {
            ep.push(crate::layout::place_element(p));
        }
        if ep.elements().next().is_none() {
            continue;
        }
        scenario.push(ep);
    }
    for c in &model.connections {
        let junction = site.junctions.get(&c.id);
        if c.probability.is_none()
            && c.condition.is_none()
            && c.quantity.is_none()
            && !c.rework
            && !c.abandoned
            && !c.turned_away
            && !c.balked
            && junction.is_none()
        {
            continue;
        }
        let mut ep = s("ElementParameters").with_attr("elementRef", c.id.as_str());
        if c.probability.is_some() || c.condition.is_some() {
            let mut cp = s("ControlParameters");
            if let Some(p) = c.probability {
                cp.push(param("Probability", probability_element(p)));
            }
            if let Some(x) = &c.condition {
                cp.push(param("Condition", condition_element(x)));
            }
            ep.push(cp);
        }
        if let Some(q) = c.quantity {
            ep.push(quantity_element(q));
        }
        if c.rework {
            ep.push(rework_element());
        }
        if c.abandoned {
            ep.push(abandoned_element());
        }
        if c.turned_away {
            ep.push(turned_away_element());
        }
        if c.balked {
            ep.push(balked_element());
        }
        if let Some(j) = junction {
            ep.push(crate::layout::junction_element(j));
        }
        scenario.push(ep);
    }
    for e in &model.elements {
        if let (
            ElementKind::Server {
                capacity,
                available,
                availability,
                shifts,
                ..
            },
            Some(rid),
        ) = (&e.kind, resource_of.get(e.id.as_str()))
        {
            let mut rp = s("ResourceParameters");
            if let Some(a) = availability_param(*available, availability) {
                rp.push(a);
            }
            rp.push(units_param(*capacity, shifts));
            let mut ep = s("ElementParameters")
                .with_attr("elementRef", rid)
                .with_child(rp);
            if let Some(m) = own_marker(shifts) {
                ep.push(m);
            }
            scenario.push(ep);
        }
    }
    for r in &model.resources {
        let mut rp = s("ResourceParameters");
        if let Some(a) = availability_param(r.available, &r.availability) {
            rp.push(a);
        }
        rp.push(quantity_param(r));
        for role in &r.roles {
            rp.push(param("Role", s("StringParameter").with_attr("value", role)));
        }
        let mut ep = s("ElementParameters")
            .with_attr("elementRef", r.id.as_str())
            .with_child(rp)
            .with_child(vendor_value_element(crate::bpsim::POOL, "true"));
        if let Some(x) = mobility_element(r) {
            ep.push(x);
        }
        scenario.push(ep);
    }
    for c in &model.calendars {
        scenario.push(calendar_element(c));
    }
    // The scenario's vendor extensions, after its calendars (BPSim's schema order).
    for x in scenario_extensions(&model.experiment) {
        scenario.push(x);
    }
    if let Some(x) = crate::layout::scene_element(&model.site.scene) {
        scenario.push(x);
    }
    // The rest of a Monte Carlo model, last, where BPSim puts a scenario's vendor extensions.
    if let Some(mc) = &model.monte_carlo {
        scenario.push(crate::mc::extension_element(mc));
    }
    // The populations of an agent-based model, likewise.
    if let Some(a) = &model.agents {
        scenario.push(crate::agents::extension_element(a));
    }
    let relationship = b("relationship")
        .with_attr("type", "BPSimData")
        .with_child(b("extensionElements").with_child(s("BPSimData").with_child(scenario)))
        .with_child(b("source").with_text(&defs_id))
        .with_child(b("target").with_text(&defs_id));
    defs.push(relationship);
    xml::write(&Document::new(defs)).into_bytes()
}
