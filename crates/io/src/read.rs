//! Reading a process model (BPMN 2.0 or XPDL 2.2) with its BPSim scenario into a
//! [`stochcraft_model::Model`].
//!
//! The process is first read into a neutral list of nodes and flows, the same for both formats,
//! and then mapped onto the model with the chosen scenario's parameters.

use std::collections::{BTreeMap, BTreeSet};

use stochcraft_model::{
    Bounds, Breakdown, Calendar, Connection, DateTime, Dist, Element as ModelElement, ElementId,
    ElementKind, Estimate, Experiment, Family, Model, Need, Property, PropertyType, PropertyValue,
    ResourcePool, Stop, ThreePoint, TimeUnit, Timed, Upper,
};

use crate::bpsim::{self, NSS, Scenario, Value};
use crate::xml::Element;
use crate::{Format, Note};

pub const BPMN: &str = "http://www.omg.org/spec/BPMN/20100524/MODEL";
pub const XPDL: [&str; 2] = [
    "http://www.wfmc.org/2009/XPDL2.2",
    "http://www.wfmc.org/2008/XPDL2.1",
];

#[derive(Debug, Clone, PartialEq)]
enum NodeKind {
    Start,
    End,
    Task,
    /// An exclusive gateway, with its default flow. Also the start and end events of a
    /// sub-process simulated in line, which pass entities straight through.
    Exclusive(Option<String>),
    Parallel,
    /// Anything StochCraft cannot simulate yet, with a description for the note.
    Unsupported(String),
}

#[derive(Debug, Clone)]
struct Node {
    id: String,
    name: String,
    kind: NodeKind,
    /// Resources named as the task's performers.
    performers: Vec<String>,
}

#[derive(Debug, Clone)]
struct Flow {
    id: String,
    from: String,
    to: String,
}

#[derive(Debug, Default)]
struct Process {
    nodes: Vec<Node>,
    flows: Vec<Flow>,
    /// Resource id → name.
    resources: BTreeMap<String, String>,
    name: String,
    /// Sub-processes simulated in line: their id → (their start event, their end event). Flows
    /// into a sub-process go to its start event, and flows out of it leave from its end event.
    inline: BTreeMap<String, (String, String)>,
}

impl Process {
    /// Points flows into and out of in-line sub-processes at their start and end events.
    fn rewire(&mut self) {
        for f in &mut self.flows {
            if let Some((start, _)) = self.inline.get(&f.to) {
                f.to = start.clone();
            }
            if let Some((_, end)) = self.inline.get(&f.from) {
                f.from = end.clone();
            }
        }
    }
}

const BPMN_TASKS: [&str; 8] = [
    "task",
    "userTask",
    "manualTask",
    "serviceTask",
    "scriptTask",
    "businessRuleTask",
    "sendTask",
    "receiveTask",
];

fn read_bpmn(root: &Element, notes: &mut Vec<Note>) -> Process {
    let ns = [BPMN];
    let mut p = Process {
        name: root.attr("name").unwrap_or_default(),
        ..Process::default()
    };
    for r in root.children_named(&ns, "resource") {
        if let Some(id) = r.attr("id") {
            p.resources.insert(id.clone(), r.attr("name").unwrap_or(id));
        }
    }
    let message_flows = root
        .descendants()
        .into_iter()
        .filter(|e| e.is(BPMN, "messageFlow"))
        .count();
    if message_flows > 0 {
        notes.push(Note::warning(
            None,
            format!(
                "{message_flows} message flows between pools are ignored (they carry no entities)"
            ),
        ));
    }
    for process in root.children_named(&ns, "process") {
        if p.name.is_empty() {
            p.name = process.attr("name").unwrap_or_default();
        }
        read_bpmn_container(process, false, &mut p, notes);
    }
    p.rewire();
    p
}

/// A start event with no trigger, or a message or timer trigger: an arrival.
fn plain_start(e: &Element) -> bool {
    e.elements().all(|c| {
        !c.local().ends_with("EventDefinition")
            || c.local() == "messageEventDefinition"
            || c.local() == "timerEventDefinition"
    })
}

/// Reads the nodes and flows of a BPMN process or sub-process into `p`. Inside a sub-process
/// (`inner`), its start and end events pass entities straight through.
fn read_bpmn_container(container: &Element, inner: bool, p: &mut Process, notes: &mut Vec<Note>) {
    let ns = [BPMN];
    for e in container.elements() {
        let Some(id) = e.attr("id") else { continue };
        let name = e.attr("name").unwrap_or_default();
        if e.ns.as_deref() != Some(BPMN) {
            continue;
        }
        let local = e.local();
        let kind = match local {
            "sequenceFlow" => {
                if let (Some(from), Some(to)) = (e.attr("sourceRef"), e.attr("targetRef")) {
                    p.flows.push(Flow { id, from, to });
                }
                continue;
            }
            "startEvent" if inner && plain_start(e) => NodeKind::Exclusive(None),
            "endEvent" if inner => NodeKind::Exclusive(None),
            "startEvent" if plain_start(e) => NodeKind::Start,
            "endEvent" => NodeKind::End,
            l if BPMN_TASKS.contains(&l) => NodeKind::Task,
            "exclusiveGateway" => NodeKind::Exclusive(e.attr("default")),
            "parallelGateway" => NodeKind::Parallel,
            "inclusiveGateway" | "eventBasedGateway" | "complexGateway" => {
                NodeKind::Unsupported(format!("{local} (not supported yet)"))
            }
            "subProcess" if e.attr("triggeredByEvent").as_deref() != Some("true") => {
                if let Some((start, end)) = inline_ends(e) {
                    notes.push(Note::info(
                        Some(&id),
                        format!(
                            "sub-process simulated in line: its start event `{start}` and end event `{end}` pass entities straight through"
                        ),
                    ));
                    p.inline.insert(id.clone(), (start, end));
                    read_bpmn_container(e, true, p, notes);
                    continue;
                }
                NodeKind::Unsupported(
                    "sub-process without exactly one plain start event and one end event (not supported yet)"
                        .into(),
                )
            }
            "subProcess" => NodeKind::Unsupported("event sub-process (not supported yet)".into()),
            "callActivity" | "transaction" | "adHocSubProcess" => {
                NodeKind::Unsupported(format!("{local} (sub-processes are not supported yet)"))
            }
            "startEvent"
            | "intermediateCatchEvent"
            | "intermediateThrowEvent"
            | "boundaryEvent" => NodeKind::Unsupported(format!(
                "{local} (event triggers other than arrivals are not supported yet)"
            )),
            "laneSet"
            | "dataObject"
            | "dataObjectReference"
            | "dataStoreReference"
            | "textAnnotation"
            | "association"
            | "documentation"
            | "extensionElements"
            | "property"
            | "ioSpecification"
            | "incoming"
            | "outgoing"
            | "multiInstanceLoopCharacteristics"
            | "standardLoopCharacteristics"
            | "dataInputAssociation"
            | "dataOutputAssociation" => continue,
            other => NodeKind::Unsupported(format!("{other} (not supported yet)")),
        };
        let performers = e
            .elements()
            .filter(|r| {
                r.is(BPMN, "performer")
                    || r.is(BPMN, "humanPerformer")
                    || r.is(BPMN, "potentialOwner")
            })
            .filter_map(|r| r.child(&ns, "resourceRef").map(Element::text))
            .collect();
        p.nodes.push(Node {
            id,
            name,
            kind,
            performers,
        });
    }
}

/// A sub-process's start and end events, when it has exactly one plain start event and one
/// end event of its own (event sub-processes inside it do not count).
fn inline_ends(sub: &Element) -> Option<(String, String)> {
    let mine = |local: &str| -> Vec<&Element> {
        sub.elements()
            .filter(|e| e.is(BPMN, local) && e.attr("id").is_some())
            .collect()
    };
    let starts = mine("startEvent");
    let ends = mine("endEvent");
    match (starts.as_slice(), ends.as_slice()) {
        ([s], [e]) if plain_start(s) => Some((s.attr("id")?, e.attr("id")?)),
        _ => None,
    }
}

fn read_xpdl(root: &Element, notes: &mut Vec<Note>) -> Process {
    let ns = XPDL;
    let mut p = Process {
        name: root.attr("Name").unwrap_or_default(),
        ..Process::default()
    };
    for part in root
        .descendants()
        .into_iter()
        .filter(|e| e.is_any(&ns, "Participant"))
    {
        let is_resource = part
            .child(&ns, "ParticipantType")
            .and_then(|t| t.attr("Type"))
            .as_deref()
            == Some("RESOURCE");
        if let (true, Some(id)) = (is_resource, part.attr("Id")) {
            p.resources
                .insert(id.clone(), part.attr("Name").unwrap_or(id));
        }
    }
    for wp in root
        .descendants()
        .into_iter()
        .filter(|e| e.is_any(&ns, "WorkflowProcess"))
    {
        let sets: Vec<&Element> = wp
            .child(&ns, "ActivitySets")
            .map(|s| s.children_named(&ns, "ActivitySet").collect())
            .unwrap_or_default();
        read_xpdl_set(wp, &sets, false, &mut p, notes);
    }
    p.rewire();
    p
}

/// Reads the activities and transitions of an XPDL process or activity set into `p`. Inside an
/// activity set simulated in line (`inner`), its start and end events pass entities straight
/// through.
fn read_xpdl_set(
    holder: &Element,
    sets: &[&Element],
    inner: bool,
    p: &mut Process,
    notes: &mut Vec<Note>,
) {
    let ns = XPDL;
    let event_of = |a: &Element, which: &str| {
        a.child(&ns, "Event")
            .is_some_and(|ev| ev.child(&ns, which).is_some())
    };
    for a in holder
        .child(&ns, "Activities")
        .into_iter()
        .flat_map(|acts| acts.children_named(&ns, "Activity"))
    {
        let Some(id) = a.attr("Id") else { continue };
        let name = a.attr("Name").unwrap_or_default();
        let kind = if a.child(&ns, "Event").is_some() {
            if event_of(a, "StartEvent") {
                if inner {
                    NodeKind::Exclusive(None)
                } else {
                    NodeKind::Start
                }
            } else if event_of(a, "EndEvent") {
                if inner {
                    NodeKind::Exclusive(None)
                } else {
                    NodeKind::End
                }
            } else {
                NodeKind::Unsupported("intermediate event (not supported yet)".into())
            }
        } else if let Some(r) = a.child(&ns, "Route") {
            let g = r.attr("GatewayType").unwrap_or_else(|| "Exclusive".into());
            match g.as_str() {
                "Exclusive" | "XOR" => NodeKind::Exclusive(None),
                "Parallel" | "AND" => NodeKind::Parallel,
                _ => NodeKind::Unsupported(format!("{g} gateway (not supported yet)")),
            }
        } else if let Some(b) = a.child(&ns, "BlockActivity") {
            let set = b
                .attr("ActivitySetId")
                .and_then(|sid| sets.iter().find(|s| s.attr("Id").as_deref() == Some(&sid)));
            match set {
                Some(s) if s.attr("TriggeredByEvent").as_deref() == Some("true") => {
                    NodeKind::Unsupported("event sub-process (not supported yet)".into())
                }
                Some(s) => {
                    let mine = |which: &str| -> Vec<String> {
                        s.child(&ns, "Activities")
                            .into_iter()
                            .flat_map(|acts| acts.children_named(&ns, "Activity"))
                            .filter(|x| event_of(x, which))
                            .filter_map(|x| x.attr("Id"))
                            .collect()
                    };
                    match (mine("StartEvent").as_slice(), mine("EndEvent").as_slice()) {
                        ([start], [end]) => {
                            notes.push(Note::info(
                                Some(&id),
                                format!(
                                    "sub-process simulated in line: its start event `{start}` and end event `{end}` pass entities straight through"
                                ),
                            ));
                            p.inline.insert(id.clone(), (start.clone(), end.clone()));
                            read_xpdl_set(s, sets, true, p, notes);
                            continue;
                        }
                        _ => NodeKind::Unsupported(
                            "sub-process without exactly one start event and one end event (not supported yet)"
                                .into(),
                        ),
                    }
                }
                None => NodeKind::Unsupported(
                    "sub-process with no activity set (not supported yet)".into(),
                ),
            }
        } else if a.child(&ns, "SubFlow").is_some() {
            NodeKind::Unsupported("sub-process (not supported yet)".into())
        } else {
            NodeKind::Task
        };
        let performers = a
            .child(&ns, "Performers")
            .map(|ps| {
                ps.children_named(&ns, "Performer")
                    .map(Element::text)
                    .collect()
            })
            .unwrap_or_default();
        p.nodes.push(Node {
            id,
            name,
            kind,
            performers,
        });
    }
    for t in holder
        .child(&ns, "Transitions")
        .into_iter()
        .flat_map(|ts| ts.children_named(&ns, "Transition"))
    {
        if let (Some(id), Some(from), Some(to)) = (t.attr("Id"), t.attr("From"), t.attr("To")) {
            // XPDL marks a gateway's default transition with an OTHERWISE condition.
            let otherwise = t
                .child(&ns, "Condition")
                .and_then(|c| c.attr("Type"))
                .as_deref()
                == Some("OTHERWISE");
            if otherwise
                && let Some(n) = p.nodes.iter_mut().find(|n| n.id == from)
                && let NodeKind::Exclusive(d) = &mut n.kind
            {
                *d = Some(id.clone());
            }
            p.flows.push(Flow { id, from, to });
        }
    }
}

/// The other time parameters StochCraft does not simulate yet.
const OTHER_TIMES: [&str; 6] = [
    "TransferTime",
    "QueueTime",
    "WaitTime",
    "SetUpTime",
    "ValidationTime",
    "ReworkTime",
];

fn dist_of(v: Option<Value>, what: &str, at: &str, notes: &mut Vec<Note>) -> Option<Dist> {
    match v {
        Some(Value::Number(n)) => Some(Dist::Constant(n)),
        Some(Value::Dist(d)) => Some(d),
        Some(Value::Expression(e)) => {
            notes.push(Note::error(
                Some(at),
                format!(
                    "{what} is an expression (`{e}`); only token properties take expressions so far"
                ),
            ));
            None
        }
        Some(other) => {
            notes.push(Note::error(
                Some(at),
                format!("{what} must be a time or a distribution, not {other:?}"),
            ));
            None
        }
        None => None,
    }
}

/// A property's `type` attribute. The schema's types are read as they are; common spellings
/// outside it (the guide's example models write `int`) are read as the nearest BPSim type with
/// a warning.
fn property_type(raw: &str, name: &str, at: &str, notes: &mut Vec<Note>) -> Option<PropertyType> {
    if let Some(t) = PropertyType::from_bpsim_name(raw) {
        return Some(t);
    }
    let near = match raw {
        "int" | "integer" | "short" | "byte" => Some(PropertyType::Long),
        "float" | "decimal" | "number" => Some(PropertyType::Double),
        "date" | "time" => Some(PropertyType::DateTime),
        "bool" => Some(PropertyType::Boolean),
        _ => None,
    };
    match near {
        Some(t) => notes.push(Note::warning(
            Some(at),
            format!(
                "property `{name}` has type `{raw}`, which BPSim does not define; read as {}",
                t.bpsim_name()
            ),
        )),
        None => notes.push(Note::warning(
            Some(at),
            format!(
                "property `{name}` has the unknown type `{raw}`; its type is taken from its values"
            ),
        )),
    }
    near
}

/// The properties an element sets in the scenario (`PropertyParameters`), by name.
fn properties(
    scenario: Option<&Scenario>,
    id: &str,
    unit: TimeUnit,
    notes: &mut Vec<Note>,
) -> Vec<Property> {
    let Some(params) = scenario.and_then(|s| s.params.get(id)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, p) in params {
        let Some(name) = key.strip_prefix("PropertyParameters/Property/") else {
            continue;
        };
        let ty = p
            .attr("type")
            .and_then(|t| property_type(t.trim(), name, id, notes));
        let value = match bpsim::value(p, unit, notes, Some(id)) {
            Some(Value::Number(n)) | Some(Value::Dist(Dist::Constant(n))) => {
                PropertyValue::Number(n)
            }
            Some(Value::Bool(b)) => PropertyValue::Boolean(b),
            Some(Value::Text(t)) => PropertyValue::Text(t),
            Some(Value::Dist(d)) => PropertyValue::Random(d),
            Some(Value::Expression(e)) => PropertyValue::Expression(e),
            // Only result requests: nothing is assigned.
            None => continue,
        };
        out.push(Property {
            name: name.to_owned(),
            ty,
            value,
        });
    }
    out
}

/// Which resources each task uses, and the tasks using each resource.
#[derive(Debug, Default)]
pub(crate) struct Resources {
    /// Tasks with a resource of their own, one unit at a time (servers): task → resource.
    pub of_task: BTreeMap<String, String>,
    /// Tasks that share pools, or need roles, several units or alternatives: task → needs.
    pub shared: BTreeMap<String, Vec<Need>>,
    /// The tasks that can use each resource.
    pub users: BTreeMap<String, Vec<String>>,
    /// Each resource's roles (BPSim `role`), in file order.
    pub roles: BTreeMap<String, Vec<String>>,
}

fn assign_resources(p: &Process, scenario: Option<&Scenario>, notes: &mut Vec<Note>) -> Resources {
    let unit = scenario.map_or(TimeUnit::Minutes, |s| s.settings.base_time_unit);
    let val = |id: &str, key: &str, notes: &mut Vec<Note>| {
        scenario
            .and_then(|s| s.param(id, key))
            .and_then(|p| bpsim::value(p, unit, notes, Some(id)))
    };
    let mut roles: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in p.resources.keys() {
        let mut list = Vec::new();
        let params: Vec<&Element> = scenario
            .map(|s| {
                s.params_with_prefix(r, "ResourceParameters/Role/")
                    .collect()
            })
            .unwrap_or_default();
        for p in params {
            match bpsim::value(p, unit, notes, Some(r)) {
                Some(Value::Text(t)) if !list.contains(&t) => list.push(t),
                Some(Value::Text(_)) | None => {}
                Some(other) => notes.push(Note::warning(
                    Some(r),
                    format!("a role must be text, not {other:?}; it is ignored"),
                )),
            }
        }
        roles.insert(r.clone(), list);
    }
    let mut users: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut needs_of: Vec<(String, Vec<Need>)> = Vec::new();
    for n in p.nodes.iter().filter(|n| n.kind == NodeKind::Task) {
        let selection = val(&n.id, "ResourceParameters/Selection", notes);
        let needs = match selection {
            Some(Value::Expression(e)) => match crate::selection::parse(&e) {
                Ok(needs) => needs,
                Err(why) => {
                    notes.push(Note::error(
                        Some(&n.id),
                        format!("resource selection `{e}`: {why}"),
                    ));
                    continue;
                }
            },
            Some(Value::Text(r)) => vec![Need::Resource {
                id: ElementId::unchecked(&r),
                quantity: 1,
            }],
            _ => match n.performers.first() {
                Some(r) => vec![Need::Resource {
                    id: ElementId::unchecked(r),
                    quantity: 1,
                }],
                None => continue,
            },
        };
        for need in &needs {
            let pools: Vec<&String> = match need {
                Need::Resource { id, .. } => p
                    .resources
                    .get_key_value(id.as_str())
                    .map(|(k, _)| k)
                    .into_iter()
                    .collect(),
                Need::Roles { roles: want, .. } => roles
                    .iter()
                    .filter(|(_, have)| want.iter().all(|w| have.contains(w)))
                    .map(|(r, _)| r)
                    .collect(),
            };
            for r in pools {
                let u = users.entry(r.clone()).or_default();
                if !u.contains(&n.id) {
                    u.push(n.id.clone());
                }
            }
        }
        needs_of.push((n.id.clone(), needs));
    }
    let mut of_task = BTreeMap::new();
    let mut shared = BTreeMap::new();
    // A resource whose units follow a calendar is a pool, even for one task.
    let timed = |r: &str| {
        scenario.is_some_and(|s| {
            s.param(r, "ResourceParameters/Availability").is_some()
                || s.param(r, "ResourceParameters/Quantity")
                    .is_some_and(|q| q.elements().any(|v| v.attr("validFor").is_some()))
        })
    };
    for (task, needs) in needs_of {
        let own = match needs.as_slice() {
            [Need::Resource { id, quantity: 1 }]
                if users.get(id.as_str()).is_none_or(|u| u.len() <= 1) && !timed(id.as_str()) =>
            {
                Some(id.to_string())
            }
            _ => None,
        };
        match own {
            Some(r) => {
                of_task.insert(task, r);
            }
            None => {
                shared.insert(task, needs);
            }
        }
    }
    Resources {
        of_task,
        shared,
        users,
        roles,
    }
}

/// Looks up a parameter's default value: (element, key, notes).
type ParamValue<'a> = dyn Fn(&str, &str, &mut Vec<Note>) -> Option<Value> + 'a;

/// A resource's units (BPSim `quantity`, default 1), for a task's own resource.
fn quantity_of(r: &str, val: &ParamValue<'_>, notes: &mut Vec<Note>) -> u32 {
    match val(r, "ResourceParameters/Quantity", notes) {
        Some(Value::Number(q)) if q >= 1.0 && q.fract() == 0.0 && q <= f64::from(u32::MAX) => {
            q as u32
        }
        Some(other) => {
            notes.push(Note::error(
                Some(r),
                format!("resource quantity must be a whole number of at least 1, not {other:?}"),
            ));
            1
        }
        None => 1,
    }
}

/// A whole number of units, at least 0.
fn units(v: &Value) -> Option<u32> {
    match v {
        Value::Number(q) if *q >= 0.0 && q.fract() == 0.0 && *q <= f64::from(u32::MAX) => {
            Some(*q as u32)
        }
        _ => None,
    }
}

/// A shared pool: its quantity outside its shifts (BPSim `quantity`, default 1), each shift's
/// (values `validFor` a calendar), and its availability (BPSim `availability`, default true).
fn pool_of(
    r: &str,
    name: &str,
    roles: Vec<String>,
    scenario: Option<&Scenario>,
    notes: &mut Vec<Note>,
) -> ResourcePool {
    let unit = scenario.map_or(TimeUnit::Minutes, |s| s.settings.base_time_unit);
    let param = |key: &str| scenario.and_then(|s| s.param(r, key));
    let mut pool = ResourcePool::new(ElementId::unchecked(r), name, 1, roles);
    if let Some(p) = param("ResourceParameters/Quantity") {
        let shifts = bpsim::timed_values(p, unit, notes, Some(r));
        match bpsim::default_value(p, unit, notes, Some(r)) {
            Some(v) => match units(&v) {
                Some(q) => pool.quantity = q,
                None => notes.push(Note::error(
                    Some(r),
                    format!("resource quantity must be a whole number, not {v:?}; 1 is used"),
                )),
            },
            // Only shift values: no units outside them.
            None if !shifts.is_empty() => pool.quantity = 0,
            None => {}
        }
        for (c, v) in shifts {
            match units(&v) {
                Some(q) => pool.shifts.push(Timed {
                    calendar: ElementId::unchecked(&c),
                    value: q,
                }),
                None => notes.push(Note::error(
                    Some(r),
                    format!("resource quantity for calendar `{c}` must be a whole number, not {v:?}; it is ignored"),
                )),
            }
        }
    }
    if let Some(p) = param("ResourceParameters/Availability") {
        let timed = bpsim::timed_values(p, unit, notes, Some(r));
        match bpsim::default_value(p, unit, notes, Some(r)) {
            Some(Value::Bool(b)) => pool.available = b,
            // Only calendar values: unavailable outside them.
            None if !timed.is_empty() => pool.available = false,
            None => {}
            Some(other) => notes.push(Note::error(
                Some(r),
                format!("availability must be true or false, not {other:?}; it is ignored"),
            )),
        }
        for (c, v) in timed {
            match v {
                Value::Bool(b) => pool.availability.push(Timed {
                    calendar: ElementId::unchecked(&c),
                    value: b,
                }),
                other => notes.push(Note::error(
                    Some(r),
                    format!("availability for calendar `{c}` must be true or false, not {other:?}; it is ignored"),
                )),
            }
        }
    }
    pool
}

/// The scenario's calendars, read from their iCalendar text.
fn calendars(scenario: Option<&Scenario>, notes: &mut Vec<Note>) -> Vec<Calendar> {
    let Some(s) = scenario else {
        return Vec::new();
    };
    s.calendars
        .iter()
        .map(|c| {
            let parsed = stochcraft_model::calendar::parse(&c.text);
            for why in parsed.errors {
                notes.push(Note::error(
                    Some(&c.id),
                    format!("{why}; this event is left out of the calendar"),
                ));
            }
            for what in parsed.ignored {
                notes.push(Note::warning(
                    Some(&c.id),
                    format!("{what} in the calendar is not simulated and is ignored"),
                ));
            }
            Calendar {
                id: ElementId::unchecked(&c.id),
                name: c.name.clone().unwrap_or_default(),
                events: parsed.events,
            }
        })
        .collect()
}

/// The resources of the tasks in the model in `root`, as [`read_model`] assigns them.
pub(crate) fn resources(root: &Element, scenario_id: Option<&str>) -> Resources {
    let mut notes = Vec::new();
    let process = if root.is(BPMN, "definitions") {
        read_bpmn(root, &mut notes)
    } else {
        read_xpdl(root, &mut notes)
    };
    let scenario =
        bpsim::find_data(root).and_then(|d| bpsim::read_scenario(d, scenario_id, &mut notes));
    assign_resources(&process, scenario.as_ref(), &mut notes)
}

/// Maps a process and scenario onto a model.
fn to_model(p: Process, scenario: Option<&Scenario>, notes: &mut Vec<Note>) -> Model {
    let unit = scenario.map_or(TimeUnit::Minutes, |s| s.settings.base_time_unit);
    let param = |id: &str, key: &str| scenario.and_then(|s| s.param(id, key));
    let val = |id: &str, key: &str, notes: &mut Vec<Note>| {
        param(id, key).and_then(|p| bpsim::value(p, unit, notes, Some(id)))
    };

    let Resources {
        of_task: task_resource,
        shared,
        users,
        roles,
    } = assign_resources(&p, scenario, notes);
    // The pools the shared tasks draw on, in id order.
    let resources: Vec<ResourcePool> = p
        .resources
        .iter()
        .filter(|(r, _)| {
            users
                .get(*r)
                .is_some_and(|u| u.iter().any(|t| shared.contains_key(t)))
        })
        .map(|(r, name)| {
            pool_of(
                r,
                name,
                roles.get(r).cloned().unwrap_or_default(),
                scenario,
                notes,
            )
        })
        .collect();

    let mut elements = Vec::new();
    let mut kept = BTreeSet::new();
    for n in &p.nodes {
        let id = n.id.as_str();
        let queue = queue_capacity(scenario, id, notes);
        let kind = match &n.kind {
            NodeKind::Start => {
                let timer = param(id, "ControlParameters/InterTriggerTimer");
                let default = timer.and_then(|p| bpsim::default_value(p, unit, notes, Some(id)));
                let mut timed = Vec::new();
                for (c, v) in
                    timer.map_or_else(Vec::new, |p| bpsim::timed_values(p, unit, notes, Some(id)))
                {
                    let what = format!("the inter-trigger timer for calendar `{c}`");
                    if let Some(d) = dist_of(Some(v), &what, id, notes) {
                        timed.push(Timed {
                            calendar: ElementId::unchecked(&c),
                            value: d,
                        });
                    }
                }
                let idle_outside = default.is_none() && !timed.is_empty();
                // With no default, the first calendar's value stands in (unused outside them).
                let interarrival = if idle_outside {
                    timed.first().map(|t| t.value.clone())
                } else {
                    dist_of(default, "the inter-trigger timer", id, notes)
                };
                let count = match val(id, "ControlParameters/TriggerCount", notes) {
                    Some(Value::Number(c)) if c >= 0.0 && c.fract() == 0.0 => Some(c as u64),
                    Some(other) => {
                        notes.push(Note::error(
                            Some(id),
                            format!("trigger count must be a whole number, not {other:?}"),
                        ));
                        None
                    }
                    None => None,
                };
                match interarrival {
                    Some(d) => ElementKind::Source {
                        interarrival: d,
                        limit: count,
                        timed,
                        idle_outside,
                    },
                    None => {
                        notes.push(Note::info(Some(id), "no inter-trigger timer, so this start event never fires (BPSim's default)"));
                        ElementKind::Source {
                            interarrival: Dist::Constant(0.0),
                            limit: Some(0),
                            idle_outside: false,
                            timed: Vec::new(),
                        }
                    }
                }
            }
            NodeKind::End => ElementKind::Sink,
            NodeKind::Task => {
                for t in OTHER_TIMES {
                    if val(id, &format!("TimeParameters/{t}"), notes).is_some() {
                        notes.push(Note::warning(
                            Some(id),
                            format!("{t} is not simulated yet and is ignored"),
                        ));
                    }
                }
                let time = dist_of(
                    val(id, "TimeParameters/ProcessingTime", notes),
                    "the processing time",
                    id,
                    notes,
                )
                .unwrap_or(Dist::Constant(0.0));
                let priority = match val(id, "PriorityParameters/Priority", notes) {
                    Some(Value::Number(x)) => Some(x),
                    Some(other) => {
                        notes.push(Note::error(
                            Some(id),
                            format!("priority must be a number, not {other:?}"),
                        ));
                        None
                    }
                    None => None,
                };
                let batching = batching_of(scenario, id, notes);
                match (batching, shared.get(id), task_resource.get(id)) {
                    (Some(kind), shared, own) => {
                        let mut ignored = Vec::new();
                        if val(id, "TimeParameters/ProcessingTime", notes).is_some() {
                            ignored.push("its processing time");
                        }
                        if shared.is_some() || own.is_some() {
                            ignored.push("its resource");
                        }
                        if !ignored.is_empty() {
                            notes.push(Note::warning(
                                Some(id),
                                format!(
                                    "a batch or a split takes no time and no resource; {} is ignored",
                                    ignored.join(" and ")
                                ),
                            ));
                        }
                        kind
                    }
                    (None, Some(needs), _) => ElementKind::SharedTask {
                        service: time,
                        needs: needs.clone(),
                        priority: priority.unwrap_or(0.0),
                    },
                    (None, None, Some(r)) => {
                        if priority.is_some_and(|x| x != 0.0) {
                            notes.push(Note::info(Some(id), "a priority matters only among tasks that share a resource; this task has its own"));
                        }
                        ElementKind::Server {
                            capacity: quantity_of(r, &val, notes),
                            service: time,
                            queue_capacity: queue,
                            breakdown: breakdown_of(scenario, id, unit, notes),
                        }
                    }
                    (None, None, None) => ElementKind::Delay { duration: time },
                }
            }
            NodeKind::Exclusive(default) => ElementKind::ExclusiveGateway {
                default: default
                    .as_ref()
                    .filter(|d| p.flows.iter().any(|f| &f.id == *d && f.from == n.id))
                    .map(|d| ElementId::unchecked(d)),
            },
            NodeKind::Parallel => ElementKind::ParallelGateway,
            NodeKind::Unsupported(what) => {
                notes.push(Note::error(
                    Some(id),
                    format!("{what}; this element is left out"),
                ));
                continue;
            }
        };
        if !matches!(kind, ElementKind::Server { .. })
            && scenario
                .and_then(|s| s.param(id, &format!("VendorExtension/{}", bpsim::BREAKDOWN)))
                .is_some()
        {
            notes.push(Note::warning(
                Some(id),
                "breakdowns (stochcraft:breakdown) apply only to a task with a resource of its own; they are ignored here",
            ));
        }
        if !matches!(kind, ElementKind::Batch { .. } | ElementKind::Unbatch)
            && !matches!(n.kind, NodeKind::Task)
            && [bpsim::BATCH_SIZE, bpsim::UNBATCH].iter().any(|x| {
                scenario
                    .and_then(|s| s.param(id, &format!("VendorExtension/{x}")))
                    .is_some()
            })
        {
            notes.push(Note::warning(
                Some(id),
                "only a task can make or split batches (stochcraft:batchSize, stochcraft:unbatch); this is ignored",
            ));
        }
        if queue.is_some() && !matches!(kind, ElementKind::Server { .. }) {
            notes.push(Note::warning(
                Some(id),
                "a queue capacity (stochcraft:queueCapacity) applies only to a task with a resource; it is ignored here",
            ));
        }
        kept.insert(n.id.clone());
        elements.push(ModelElement {
            id: ElementId::unchecked(&n.id),
            name: n.name.clone(),
            kind,
            properties: properties(scenario, id, unit, notes),
        });
    }
    let mut connections = Vec::new();
    for f in &p.flows {
        if !(kept.contains(&f.from) && kept.contains(&f.to)) {
            continue;
        }
        let id = f.id.as_str();
        let probability = match val(id, "ControlParameters/Probability", notes) {
            Some(Value::Number(x)) => Some(x),
            Some(other) => {
                notes.push(Note::error(
                    Some(id),
                    format!("a probability must be a number, not {other:?}; it is ignored"),
                ));
                None
            }
            None => None,
        };
        let condition = match val(id, "ControlParameters/Condition", notes) {
            Some(Value::Expression(x)) => Some(x),
            Some(Value::Bool(b)) => Some(if b { "true()" } else { "false()" }.to_owned()),
            Some(other) => {
                notes.push(Note::error(
                    Some(id),
                    format!("a condition must be an expression or true/false, not {other:?}; it is ignored"),
                ));
                None
            }
            None => None,
        };
        connections.push(Connection {
            id: ElementId::unchecked(&f.id),
            from: ElementId::unchecked(&f.from),
            to: ElementId::unchecked(&f.to),
            probability,
            condition,
            quantity: vendor_count(scenario, id, bpsim::QUANTITY, "a quantity", notes),
        });
    }
    let mut experiment = experiment(scenario, notes);
    experiment.stop = stop_of(scenario, &elements, notes);
    let estimates = estimates_of(scenario, &elements, notes);
    Model {
        name: p.name,
        elements,
        connections,
        resources,
        calendars: calendars(scenario, notes),
        experiment,
        layout: BTreeMap::new(),
        estimates,
    }
}

/// The stop condition StochCraft's vendor extensions give the experiment, on the element they
/// watch; the first, in model order, if there are several.
fn stop_of(
    scenario: Option<&Scenario>,
    elements: &[ModelElement],
    notes: &mut Vec<Note>,
) -> Option<Stop> {
    let s = scenario?;
    let mut found: Vec<Stop> = Vec::new();
    for e in elements {
        let id = e.id.as_str();
        let element = e.id.clone();
        if let Some(n) = vendor_count(scenario, id, bpsim::STOP_COUNT, "a stop count", notes) {
            found.push(Stop::Count {
                element: element.clone(),
                count: u64::from(n),
            });
        }
        if let Some(x) = s.param(id, &format!("VendorExtension/{}", bpsim::STOP_WHEN_EMPTY)) {
            match bpsim::vendor_value(x).unwrap_or_default().trim() {
                "true" | "1" => found.push(Stop::Empty { element }),
                "false" | "0" => {}
                other => notes.push(Note::error(
                    Some(id),
                    format!(
                        "`{}` must be true or false, not `{other}`; it is ignored",
                        bpsim::STOP_WHEN_EMPTY
                    ),
                )),
            }
        }
    }
    if found.len() > 1 {
        notes.push(Note::error(
            Some(found[0].element().as_str()),
            "the scenario has several stop conditions; the run stops on this one only",
        ));
    }
    found.into_iter().next()
}

/// The three-point estimates StochCraft's vendor extensions record for elements.
fn estimates_of(
    scenario: Option<&Scenario>,
    elements: &[ModelElement],
    notes: &mut Vec<Note>,
) -> BTreeMap<ElementId, Estimate> {
    let mut out = BTreeMap::new();
    let Some(s) = scenario else {
        return out;
    };
    for e in elements {
        let id = e.id.as_str();
        let Some(ext) = s.param(id, &format!("VendorExtension/{}", bpsim::ESTIMATE)) else {
            continue;
        };
        let body = ext.elements().find(|x| x.local() == "estimate");
        let number = |name: &str| body.and_then(|b| b.attr(name)?.trim().parse::<f64>().ok());
        let family = body
            .and_then(|b| b.attr("family"))
            .and_then(|f| Family::from_name(f.trim()));
        let upper = match (number("max"), number("p95")) {
            (Some(x), None) => Some((x, Upper::Max)),
            (None, Some(x)) => Some((x, Upper::P95)),
            _ => None,
        };
        match (family, number("min"), number("mode"), upper) {
            (Some(family), Some(min), Some(mode), Some((upper, upper_is))) => {
                out.insert(
                    e.id.clone(),
                    Estimate {
                        three_point: ThreePoint {
                            family,
                            min,
                            mode,
                            upper,
                            upper_is,
                        },
                        source: body.and_then(|b| b.attr("source")).unwrap_or_default(),
                    },
                );
            }
            _ => notes.push(Note::error(
                Some(id),
                format!(
                    "a three-point estimate (`{}`) needs a family (triangular, pert, lognormal or weibull), a min, a mode and one of max or p95; it is ignored",
                    bpsim::ESTIMATE
                ),
            )),
        }
    }
    out
}

/// The finite buffer StochCraft's vendor extension gives element `id`, if any; and a note for
/// each StochCraft extension this version does not know (kept in the file all the same).
fn queue_capacity(scenario: Option<&Scenario>, id: &str, notes: &mut Vec<Note>) -> Option<u32> {
    let params = scenario.and_then(|s| s.params.get(id))?;
    let mut queue = None;
    for (key, e) in params {
        let Some(name) = key.strip_prefix("VendorExtension/") else {
            continue;
        };
        if name == bpsim::QUEUE_CAPACITY {
            let raw = bpsim::vendor_value(e).unwrap_or_default();
            match raw.trim().parse::<u32>() {
                Ok(q) => queue = Some(q),
                Err(_) => notes.push(Note::error(
                    Some(id),
                    format!(
                        "queue capacity `{raw}` must be a whole number of at least 0; there is no limit"
                    ),
                )),
            }
        } else if [
            bpsim::BREAKDOWN,
            bpsim::BATCH_SIZE,
            bpsim::UNBATCH,
            bpsim::QUANTITY,
            bpsim::STOP_COUNT,
            bpsim::STOP_WHEN_EMPTY,
            bpsim::ESTIMATE,
        ]
        .contains(&name)
        {
            // Read by `breakdown_of`, `batching_of`, `vendor_count`, `stop_of` and
            // `estimates_of`.
        } else if name.starts_with(bpsim::STOCHCRAFT) {
            notes.push(Note::warning(
                Some(id),
                format!("the StochCraft extension `{name}` is not known to this version; it is kept in the file but not used"),
            ));
        }
    }
    queue
}

/// A whole number of at least 1 that StochCraft's vendor extension `name` gives element `id`,
/// if it has one; an error note (and none) for anything else.
fn vendor_count(
    scenario: Option<&Scenario>,
    id: &str,
    name: &str,
    what: &str,
    notes: &mut Vec<Note>,
) -> Option<u32> {
    let e = scenario.and_then(|s| s.param(id, &format!("VendorExtension/{name}")))?;
    let raw = bpsim::vendor_value(e).unwrap_or_default();
    match raw.trim().parse::<u32>() {
        Ok(n) if n >= 1 => Some(n),
        _ => {
            notes.push(Note::error(
                Some(id),
                format!("{what} (`{name}`) must be a whole number of at least 1, not `{raw}`; it is ignored"),
            ));
            None
        }
    }
}

/// The batch or split StochCraft's vendor extensions make task `id`, if any.
fn batching_of(
    scenario: Option<&Scenario>,
    id: &str,
    notes: &mut Vec<Note>,
) -> Option<ElementKind> {
    let batch = vendor_count(scenario, id, bpsim::BATCH_SIZE, "a batch size", notes)
        .map(|size| ElementKind::Batch { size });
    let unbatch = scenario
        .and_then(|s| s.param(id, &format!("VendorExtension/{}", bpsim::UNBATCH)))
        .and_then(|e| {
            let raw = bpsim::vendor_value(e).unwrap_or_default();
            match raw.trim() {
                "true" | "1" => Some(ElementKind::Unbatch),
                "false" | "0" => None,
                other => {
                    notes.push(Note::error(
                        Some(id),
                        format!(
                            "`{}` must be true or false, not `{other}`; it is ignored",
                            bpsim::UNBATCH
                        ),
                    ));
                    None
                }
            }
        });
    match (batch, unbatch) {
        (Some(b), Some(_)) => {
            notes.push(Note::error(
                Some(id),
                "a task cannot both make batches and split them; it makes them",
            ));
            Some(b)
        }
        (b, u) => b.or(u),
    }
}

/// The breakdowns StochCraft's vendor extension gives server `id`, if any; whether a failure
/// interrupts the job is BPSim's `Interruptible` (default false).
fn breakdown_of(
    scenario: Option<&Scenario>,
    id: &str,
    unit: TimeUnit,
    notes: &mut Vec<Note>,
) -> Option<Breakdown> {
    let s = scenario?;
    let ext = s.param(id, &format!("VendorExtension/{}", bpsim::BREAKDOWN))?;
    let body = ext.elements().find(|e| e.local() == "breakdown");
    let part = |local: &str, notes: &mut Vec<Note>| {
        let p = body?.elements().find(|e| e.local() == local)?;
        dist_of(
            bpsim::default_value(p, unit, notes, Some(id)),
            &format!("the breakdown's {local}"),
            id,
            notes,
        )
    };
    let (Some(time_to_failure), Some(repair)) =
        (part("timeToFailure", notes), part("repairTime", notes))
    else {
        notes.push(Note::error(
            Some(id),
            "a breakdown needs a timeToFailure and a repairTime distribution; the units never fail",
        ));
        return None;
    };
    let interruptible = match s
        .param(id, "PriorityParameters/Interruptible")
        .and_then(|p| bpsim::value(p, unit, notes, Some(id)))
    {
        Some(Value::Bool(b)) => b,
        None => false,
        Some(other) => {
            notes.push(Note::error(
                Some(id),
                format!("interruptible must be true or false, not {other:?}; false is used"),
            ));
            false
        }
    };
    Some(Breakdown {
        time_to_failure,
        repair,
        interruptible,
    })
}

fn experiment(scenario: Option<&Scenario>, notes: &mut Vec<Note>) -> Experiment {
    let Some(s) = scenario.map(|s| &s.settings) else {
        return Experiment::default();
    };
    let unit = s.base_time_unit;
    let warmup = s.warmup.unwrap_or(0.0);
    // BPSim's duration includes the warm-up; StochCraft's excludes it.
    let duration = match s.duration {
        Some(d) if d > warmup => d - warmup,
        Some(d) => {
            notes.push(Note::error(
                None,
                format!("the scenario duration ({d}) must be longer than its warm-up ({warmup})"),
            ));
            d
        }
        None => {
            let default = unit.convert(8.0, TimeUnit::Hours);
            notes.push(Note::warning(
                None,
                format!(
                    "the scenario has no duration; {default} {} (8 hours) is used",
                    unit.bpsim_name()
                ),
            ));
            default
        }
    };
    let seed = s.seed.unwrap_or_else(|| {
        notes.push(Note::info(
            None,
            "the scenario has no seed; seed 1 is used so results can be reproduced",
        ));
        1
    });
    let start = s.start.as_deref().and_then(|t| {
        let d = DateTime::parse_iso(t);
        if d.is_none() {
            notes.push(Note::warning(
                None,
                format!("the scenario start `{t}` is not a date and time and is ignored"),
            ));
        }
        d
    });
    Experiment {
        time_unit: unit,
        warmup,
        duration,
        replications: s.replication.unwrap_or(1),
        seed,
        start,
        stop: None,
    }
}

/// Reads the model in `root` (a BPMN `definitions` or XPDL `Package`) with scenario `id` (or the
/// first input scenario). Returns the format, the scenario actually used and the model.
pub fn read_model(
    root: &Element,
    scenario_id: Option<&str>,
    notes: &mut Vec<Note>,
) -> Result<(Format, Option<Scenario>, Model), String> {
    let (format, process) = if root.is(BPMN, "definitions") {
        (Format::Bpmn, read_bpmn(root, notes))
    } else if root.is_any(&XPDL, "Package") {
        (Format::Xpdl, read_xpdl(root, notes))
    } else {
        return Err(format!(
            "`{}` is neither a BPMN 2.0 definitions nor an XPDL 2.2 package",
            root.name
        ));
    };
    let scenario = match bpsim::find_data(root) {
        Some(data) => match bpsim::read_scenario(data, scenario_id, notes) {
            Some(s) => {
                if s.version_1 {
                    notes.push(Note::info(None, "BPSim 1.0 data, read as BPSim 2.0"));
                }
                Some(s)
            }
            None if scenario_id.is_some() => {
                return Err(format!("no scenario `{}`", scenario_id.unwrap_or_default()));
            }
            None => {
                notes.push(Note::warning(
                    None,
                    "the file's BPSim data has no input scenario; defaults are used",
                ));
                None
            }
        },
        None => {
            notes.push(Note::warning(
                None,
                "the file has no BPSim simulation data; defaults are used",
            ));
            None
        }
    };
    let _ = NSS;
    let mut model = to_model(process, scenario.as_ref(), notes);
    model.layout = layout(root, format, &model);
    Ok((format, scenario, model))
}

const BPMNDI: &str = "http://www.omg.org/spec/BPMN/20100524/DI";
const DC: &str = "http://www.omg.org/spec/DD/20100524/DC";

/// The shapes the file's diagram gives the model's elements (the first, if it has several):
/// BPMN DI's `BPMNShape` bounds, or XPDL's `NodeGraphicsInfo`.
pub(crate) fn layout(root: &Element, format: Format, model: &Model) -> BTreeMap<ElementId, Bounds> {
    let mut out = BTreeMap::new();
    for e in &model.elements {
        if let Some(b) = shape_bounds(root, format, e.id.as_str()) {
            out.insert(e.id.clone(), b);
        }
    }
    out
}

/// The bounds of the shape drawn for node `id`, if the file has one.
pub(crate) fn shape_bounds(root: &Element, format: Format, id: &str) -> Option<Bounds> {
    let f = |e: &Element, a: &str| e.attr(a).and_then(|v| v.trim().parse::<f64>().ok());
    let finite = |b: Bounds| {
        (b.x.is_finite() && b.y.is_finite() && b.width >= 0.0 && b.height >= 0.0).then_some(b)
    };
    match format {
        Format::Bpmn => {
            let shape = root.descendants().into_iter().find(|e| {
                e.is(BPMNDI, "BPMNShape") && e.attr("bpmnElement").as_deref() == Some(id)
            })?;
            let b = shape.child(&[DC], "Bounds")?;
            finite(Bounds {
                x: f(b, "x")?,
                y: f(b, "y")?,
                width: f(b, "width")?,
                height: f(b, "height")?,
            })
        }
        Format::Xpdl => {
            let act = root
                .descendants()
                .into_iter()
                .find(|e| e.is_any(&XPDL, "Activity") && e.attr("Id").as_deref() == Some(id))?;
            let info = act
                .child(&XPDL, "NodeGraphicsInfos")?
                .child(&XPDL, "NodeGraphicsInfo")?;
            let c = info.child(&XPDL, "Coordinates")?;
            finite(Bounds {
                x: f(c, "XCoordinate")?,
                y: f(c, "YCoordinate")?,
                width: f(info, "Width")?,
                height: f(info, "Height")?,
            })
        }
    }
}
