//! Reading a process model (BPMN 2.0 or XPDL 2.2) with its BPSim scenario into a
//! [`stochcraft_model::Model`].
//!
//! The process is first read into a neutral list of nodes and flows, the same for both formats,
//! and then mapped onto the model with the chosen scenario's parameters.

use std::collections::{BTreeMap, BTreeSet};

use stochcraft_model::{
    Connection, Dist, Element as ModelElement, ElementId, ElementKind, Experiment, Model, Property,
    PropertyType, PropertyValue, TimeUnit,
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

/// `getResource('id', n)` or `bpsim:getResource("id", n)`: the resource id and quantity.
fn get_resource(expr: &str) -> Option<(String, f64)> {
    let e = expr.trim();
    let e = e.strip_prefix("bpsim:").unwrap_or(e);
    let args = e
        .strip_prefix("getResource")?
        .trim()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let (name, qty) = args.rsplit_once(',')?;
    let name = name
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
        .to_owned();
    Some((name, qty.trim().parse().ok()?))
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

/// Which resource each task uses, and the tasks using each resource.
#[derive(Debug, Default)]
pub(crate) struct Resources {
    pub of_task: BTreeMap<String, String>,
    pub users: BTreeMap<String, Vec<String>>,
}

fn assign_resources(p: &Process, scenario: Option<&Scenario>, notes: &mut Vec<Note>) -> Resources {
    let unit = scenario.map_or(TimeUnit::Minutes, |s| s.settings.base_time_unit);
    let val = |id: &str, key: &str, notes: &mut Vec<Note>| {
        scenario
            .and_then(|s| s.param(id, key))
            .and_then(|p| bpsim::value(p, unit, notes, Some(id)))
    };
    let mut users: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut task_resource: BTreeMap<String, String> = BTreeMap::new();
    for n in p.nodes.iter().filter(|n| n.kind == NodeKind::Task) {
        let selection = val(&n.id, "ResourceParameters/Selection", notes);
        let chosen = match selection {
            Some(Value::Expression(e)) => match get_resource(&e) {
                Some((r, qty)) => {
                    if qty != 1.0 {
                        notes.push(Note::warning(
                            Some(&n.id),
                            format!("needs {qty} units of `{r}` per job; one is used until M2.5"),
                        ));
                    }
                    Some(r)
                }
                None => {
                    notes.push(Note::error(
                        Some(&n.id),
                        format!("resource selection `{e}` is not supported until M2.5"),
                    ));
                    None
                }
            },
            Some(Value::Text(r)) => Some(r),
            _ => n.performers.first().cloned(),
        };
        if let Some(r) = chosen {
            users.entry(r.clone()).or_default().push(n.id.clone());
            task_resource.insert(n.id.clone(), r);
        }
    }
    Resources {
        of_task: task_resource,
        users,
    }
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
        users,
    } = assign_resources(&p, scenario, notes);
    for (r, tasks) in users.iter().filter(|(_, t)| t.len() > 1) {
        notes.push(Note::error(
            Some(r),
            format!(
                "resource shared by {} tasks ({}); shared resource pools arrive in M2.5",
                tasks.len(),
                tasks.join(", ")
            ),
        ));
    }

    let mut elements = Vec::new();
    let mut kept = BTreeSet::new();
    for n in &p.nodes {
        let id = n.id.as_str();
        let kind = match &n.kind {
            NodeKind::Start => {
                let interarrival = dist_of(
                    val(id, "ControlParameters/InterTriggerTimer", notes),
                    "the inter-trigger timer",
                    id,
                    notes,
                );
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
                    },
                    None => {
                        notes.push(Note::info(Some(id), "no inter-trigger timer, so this start event never fires (BPSim's default)"));
                        ElementKind::Source {
                            interarrival: Dist::Constant(0.0),
                            limit: Some(0),
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
                match task_resource.get(id) {
                    Some(r) => {
                        let quantity = match val(r, "ResourceParameters/Quantity", notes) {
                            Some(Value::Number(q))
                                if q >= 1.0 && q.fract() == 0.0 && q <= f64::from(u32::MAX) =>
                            {
                                q as u32
                            }
                            Some(Value::Number(0.0)) => {
                                notes.push(Note::error(
                                    Some(r),
                                    "has no units outside its calendar shifts; shifts arrive in M2.6, so 1 unit is used",
                                ));
                                1
                            }
                            Some(other) => {
                                notes.push(Note::error(Some(r), format!("resource quantity must be a whole number of at least 1, not {other:?}")));
                                1
                            }
                            None => 1,
                        };
                        ElementKind::Server {
                            capacity: quantity,
                            service: time,
                        }
                    }
                    None => ElementKind::Delay { duration: time },
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
        });
    }
    Model {
        name: p.name,
        elements,
        connections,
        experiment: experiment(scenario, notes),
    }
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
    Experiment {
        time_unit: unit,
        warmup,
        duration,
        replications: s.replication.unwrap_or(1),
        seed,
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
    let model = to_model(process, scenario.as_ref(), notes);
    Ok((format, scenario, model))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_resource_expressions() {
        assert_eq!(
            get_resource("getResource('resource_Front_Office', 1)"),
            Some(("resource_Front_Office".into(), 1.0))
        );
        assert_eq!(
            get_resource(" bpsim:getResource(\"m\", 2) "),
            Some(("m".into(), 2.0))
        );
        assert_eq!(get_resource("getResourceByRoles(['a'], 1)"), None);
        assert_eq!(get_resource("getResource('m')"), None);
    }
}
