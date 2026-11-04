//! Reading a process model (BPMN 2.0 or XPDL 2.2) with its BPSim scenario into a
//! [`stochcraft_model::Model`].
//!
//! The process is first read into a neutral list of nodes and flows, the same for both formats,
//! and then mapped onto the model with the chosen scenario's parameters.

use std::collections::{BTreeMap, BTreeSet};

use stochcraft_model::{
    Connection, Dist, Element as ModelElement, ElementId, ElementKind, Experiment, Model, TimeUnit,
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
        for e in process.elements() {
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
                "startEvent"
                    if e.elements().all(|c| {
                        !c.local().ends_with("EventDefinition")
                            || c.local() == "messageEventDefinition"
                            || c.local() == "timerEventDefinition"
                    }) =>
                {
                    NodeKind::Start
                }
                "endEvent" => NodeKind::End,
                l if BPMN_TASKS.contains(&l) => NodeKind::Task,
                "exclusiveGateway" | "parallelGateway" | "inclusiveGateway"
                | "eventBasedGateway" | "complexGateway" => {
                    NodeKind::Unsupported(format!("{local} (gateways arrive in M2.3)"))
                }
                "subProcess" | "callActivity" | "transaction" | "adHocSubProcess" => {
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
                | "ioSpecification" => continue,
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
    p
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
    let mut activity_sets = 0;
    for wp in root
        .descendants()
        .into_iter()
        .filter(|e| e.is_any(&ns, "WorkflowProcess"))
    {
        activity_sets += wp
            .child(&ns, "ActivitySets")
            .map_or(0, |s| s.elements().count());
        for a in wp
            .child(&ns, "Activities")
            .into_iter()
            .flat_map(|acts| acts.children_named(&ns, "Activity"))
        {
            let Some(id) = a.attr("Id") else { continue };
            let name = a.attr("Name").unwrap_or_default();
            let event = a.child(&ns, "Event");
            let kind = if let Some(ev) = event {
                if ev.child(&ns, "StartEvent").is_some() {
                    NodeKind::Start
                } else if ev.child(&ns, "EndEvent").is_some() {
                    NodeKind::End
                } else {
                    NodeKind::Unsupported("intermediate event (not supported yet)".into())
                }
            } else if let Some(r) = a.child(&ns, "Route") {
                let g = r.attr("GatewayType").unwrap_or_else(|| "Exclusive".into());
                NodeKind::Unsupported(format!("{g} gateway (gateways arrive in M2.3)"))
            } else if a.child(&ns, "BlockActivity").is_some() || a.child(&ns, "SubFlow").is_some() {
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
        for t in wp
            .child(&ns, "Transitions")
            .into_iter()
            .flat_map(|ts| ts.children_named(&ns, "Transition"))
        {
            if let (Some(id), Some(from), Some(to)) = (t.attr("Id"), t.attr("From"), t.attr("To")) {
                p.flows.push(Flow { id, from, to });
            }
        }
    }
    if activity_sets > 0 {
        notes.push(Note::error(None, format!("{activity_sets} sub-process activity sets are not supported yet; their contents are not simulated")));
    }
    p
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
                format!("{what} is an expression (`{e}`); expressions arrive in M2.2"),
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
        });
    }
    let mut connections = Vec::new();
    for f in &p.flows {
        for key in [
            "ControlParameters/Probability",
            "ControlParameters/Condition",
        ] {
            if param(&f.id, key).is_some() && kept.contains(&f.from) {
                notes.push(Note::warning(
                    Some(&f.id),
                    "branch probabilities and conditions arrive with gateways in M2.3",
                ));
            }
        }
        if kept.contains(&f.from) && kept.contains(&f.to) {
            connections.push(Connection {
                id: ElementId::unchecked(&f.id),
                from: ElementId::unchecked(&f.from),
                to: ElementId::unchecked(&f.to),
            });
        }
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
    if scenario.as_ref().is_some_and(|s| {
        s.params
            .values()
            .any(|p| p.keys().any(|k| k.contains("/Property/")))
    }) {
        notes.push(Note::warning(
            None,
            "token properties and the expressions that use them arrive in M2.2; they are ignored",
        ));
    }
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
