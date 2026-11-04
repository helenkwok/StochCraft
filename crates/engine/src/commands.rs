//! The command catalogue: each command's name, summary, parameters (with their JSON schema), an
//! example and its implementation.

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use stochcraft_model::{Connection, Element, ElementId, ElementKind, Model};

use crate::edit::Op;
use crate::run::RunMode;
use crate::wire::{self, DistSpec, KindSpec, ModelDoc, TimeUnitSpec};
use crate::{CommandError, Engine, ErrorCode};

/// What a command does, which decides whether it is undoable and journalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    /// Reads without changing anything. Not journalled.
    Query,
    /// Changes the model. Undoable and journalled.
    Edit,
    /// Works with history, files or runs. Journalled, not undoable itself.
    Action,
}

impl CommandKind {
    pub fn name(self) -> &'static str {
        match self {
            CommandKind::Query => "query",
            CommandKind::Edit => "edit",
            CommandKind::Action => "action",
        }
    }
}

/// One command in the catalogue.
pub struct Command {
    pub name: &'static str,
    pub summary: &'static str,
    pub kind: CommandKind,
    schema: fn() -> Value,
    example: fn() -> Value,
    pub(crate) run: fn(&mut Engine, Value) -> Result<Value, CommandError>,
}

impl Command {
    /// The JSON schema of the command's parameters.
    pub fn params_schema(&self) -> Value {
        (self.schema)()
    }

    /// Example parameters, valid against the schema.
    pub fn example(&self) -> Value {
        (self.example)()
    }

    /// The catalogue entry as JSON: what the CLI's `schema` command and an MCP tool list show.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "summary": self.summary,
            "kind": self.kind.name(),
            "params": self.params_schema(),
            "example": self.example(),
        })
    }
}

fn schema<T: JsonSchema>() -> Value {
    schemars::schema_for!(T).to_value()
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, CommandError> {
    serde_json::from_value(params)
        .map_err(|e| CommandError::new(ErrorCode::InvalidParams, e.to_string()))
}

/// Parameters that may not be left out of a schema'd struct, for commands with none.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// Present-but-null vs absent, for fields that can be cleared.
fn some<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

/// Refuses keys a command does not know, where `deny_unknown_fields` cannot be used (flattened
/// element kinds): a misspelt parameter is an error, not silently ignored.
fn check_keys(v: &Value, extra: &[&str]) -> Result<(), CommandError> {
    let Some(obj) = v.as_object() else {
        return Ok(());
    };
    let kind_keys: &[&str] = match obj.get("kind").and_then(Value::as_str) {
        Some("source") => &["interarrival", "limit"],
        Some("server") => &["capacity", "service"],
        Some("delay") => &["duration"],
        _ => &[],
    };
    for k in obj.keys() {
        if k != "kind" && !extra.contains(&k.as_str()) && !kind_keys.contains(&k.as_str()) {
            return Err(CommandError::new(
                ErrorCode::InvalidParams,
                format!("unknown parameter `{k}`"),
            ));
        }
    }
    Ok(())
}

fn id_param(s: &str) -> Result<ElementId, CommandError> {
    ElementId::new(s).ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidParams,
            format!("`{s}` is not a valid id (a letter or _ first, then letters, digits, . - _)"),
        )
    })
}

fn id_taken(model: &Model, id: &ElementId) -> bool {
    model.elements.iter().any(|e| &e.id == id) || model.connections.iter().any(|c| &c.id == id)
}

/// The id given, which must be new, or a fresh `prefix-N`.
fn new_id(model: &Model, given: Option<&str>, prefix: &str) -> Result<ElementId, CommandError> {
    if let Some(s) = given {
        let id = id_param(s)?;
        if id_taken(model, &id) {
            return Err(CommandError::new(
                ErrorCode::Conflict,
                format!("the id `{id}` is already used"),
            ));
        }
        return Ok(id);
    }
    (1u64..)
        .map(|n| ElementId::unchecked(&format!("{prefix}-{n}")))
        .find(|id| !id_taken(model, id))
        .ok_or_else(|| CommandError::new(ErrorCode::Internal, "no free id"))
}

fn existing_element(model: &Model, s: &str) -> Result<ElementId, CommandError> {
    let id = ElementId::unchecked(s);
    if model.element(&id).is_none() {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            format!("no element with id `{s}`"),
        ));
    }
    Ok(id)
}

// ---------------------------------------------------------------------------------------------
// Parameters

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelNew {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelSet {
    pub model: ModelDoc,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Rename {
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BlockAdd {
    /// A new id; by default `<kind>-N`.
    #[serde(default)]
    pub id: Option<String>,
    /// The display name; by default the id.
    #[serde(default)]
    pub name: Option<String>,
    /// Position in the model's element list; by default the end.
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(flatten)]
    pub kind: KindSpec,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Id {
    pub id: String,
}

/// The parameters to change; each must apply to the element's kind.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlockSet {
    pub id: String,
    /// Sources: time between arrivals.
    #[serde(default)]
    pub interarrival: Option<DistSpec>,
    /// Sources: stop after this many arrivals; `null` removes the limit.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<u64>")]
    pub limit: Option<Option<u64>>,
    /// Servers: number of identical units.
    #[serde(default)]
    pub capacity: Option<u32>,
    /// Servers: time to serve one entity.
    #[serde(default)]
    pub service: Option<DistSpec>,
    /// Delays: time each entity spends.
    #[serde(default)]
    pub duration: Option<DistSpec>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlockRename {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Connect {
    pub from: String,
    pub to: String,
    /// A new id; by default `flow-N`.
    #[serde(default)]
    pub id: Option<String>,
}

/// The connection's id, or the elements it joins.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Disconnect {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

/// The settings to change. Changing `time_unit` does not convert times already in the model.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExperimentSet {
    #[serde(default)]
    pub time_unit: Option<TimeUnitSpec>,
    /// Time before statistics are collected.
    #[serde(default)]
    pub warmup: Option<f64>,
    /// Time over which statistics are collected, after the warm-up.
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub replications: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
}

/// Overrides of the model's experiment for this run only.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Run {
    #[serde(default)]
    pub replications: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Worker threads; by default all available. Results do not depend on it.
    #[serde(default)]
    pub threads: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStart {
    #[serde(default)]
    pub replications: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub threads: Option<u32>,
    /// `threads` natively, `slices` in the browser by default.
    #[serde(default)]
    pub mode: Option<RunMode>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStatus {
    /// For a run in slices: replications to run before reporting (default 1).
    #[serde(default)]
    pub step: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileOpen {
    pub path: String,
    /// The BPSim scenario to read; by default the first.
    #[serde(default)]
    pub scenario: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileSave {
    /// Where to save; by default the file the model was opened from or last saved to.
    #[serde(default)]
    pub path: Option<String>,
    /// Write a new BPMN file from the model alone, instead of saving into the opened file.
    #[serde(default)]
    pub as_new: bool,
}

// ---------------------------------------------------------------------------------------------
// Implementations

fn model_get(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    Ok(wire::model(e.model()))
}

fn model_validate(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    Ok(json!({ "diagnostics": wire::diagnostics(&e.model().validate()) }))
}

fn model_new(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: ModelNew = parse(p)?;
    e.new_document(Model {
        name: p.name,
        ..Model::default()
    });
    Ok(json!({}))
}

fn model_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    if let Some(elements) = p
        .get("model")
        .and_then(|m| m.get("elements"))
        .and_then(Value::as_array)
    {
        for el in elements {
            check_keys(el, &["id", "name"])?;
        }
    }
    let p: ModelSet = parse(p)?;
    e.edit("model.set", Op::SetModel(Box::new(p.model.into())))?;
    Ok(json!({}))
}

fn model_rename(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Rename = parse(p)?;
    e.edit("model.rename", Op::RenameModel(p.name))?;
    Ok(json!({}))
}

fn block_add(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    check_keys(&p, &["id", "name", "index"])?;
    let p: BlockAdd = parse(p)?;
    let kind: ElementKind = p.kind.into();
    let id = new_id(e.model(), p.id.as_deref(), kind.name())?;
    let index = p
        .index
        .map_or(e.model().elements.len(), |i| i as usize)
        .min(e.model().elements.len());
    let element = Element {
        name: p.name.unwrap_or_else(|| id.to_string()),
        id: id.clone(),
        kind,
    };
    e.edit(
        "block.add",
        Op::AddElement {
            index,
            element,
            connections: vec![],
        },
    )?;
    Ok(json!({ "id": id.as_str() }))
}

fn block_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Id = parse(p)?;
    let id = existing_element(e.model(), &p.id)?;
    e.edit("block.remove", Op::RemoveElement(id))?;
    Ok(json!({}))
}

fn block_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: BlockSet = parse(p)?;
    let id = existing_element(e.model(), &p.id)?;
    let mut kind = e
        .model()
        .element(&id)
        .map_or(ElementKind::Sink, |el| el.kind.clone());
    let given = [
        ("interarrival", p.interarrival.is_some()),
        ("limit", p.limit.is_some()),
        ("capacity", p.capacity.is_some()),
        ("service", p.service.is_some()),
        ("duration", p.duration.is_some()),
    ];
    let applies: &[&str] = match kind {
        ElementKind::Source { .. } => &["interarrival", "limit"],
        ElementKind::Server { .. } => &["capacity", "service"],
        ElementKind::Delay { .. } => &["duration"],
        ElementKind::Sink => &[],
    };
    if let Some((name, _)) = given.iter().find(|(n, g)| *g && !applies.contains(n)) {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!("`{name}` does not apply to a {}", kind.name()),
        ));
    }
    if !given.iter().any(|(_, g)| *g) {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            "nothing to set: give at least one parameter",
        ));
    }
    match &mut kind {
        ElementKind::Source {
            interarrival,
            limit,
        } => {
            if let Some(d) = p.interarrival {
                *interarrival = d.into();
            }
            if let Some(l) = p.limit {
                *limit = l;
            }
        }
        ElementKind::Server { capacity, service } => {
            if let Some(c) = p.capacity {
                *capacity = c;
            }
            if let Some(d) = p.service {
                *service = d.into();
            }
        }
        ElementKind::Delay { duration } => {
            if let Some(d) = p.duration {
                *duration = d.into();
            }
        }
        ElementKind::Sink => {}
    }
    e.edit("block.set", Op::SetKind(id, kind))?;
    Ok(json!({}))
}

fn block_rename(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: BlockRename = parse(p)?;
    let id = existing_element(e.model(), &p.id)?;
    e.edit("block.rename", Op::RenameElement(id, p.name))?;
    Ok(json!({}))
}

fn connect(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Connect = parse(p)?;
    let from = existing_element(e.model(), &p.from)?;
    let to = existing_element(e.model(), &p.to)?;
    let id = new_id(e.model(), p.id.as_deref(), "flow")?;
    let index = e.model().connections.len();
    e.edit(
        "connect",
        Op::Connect(
            index,
            Connection {
                id: id.clone(),
                from,
                to,
            },
        ),
    )?;
    Ok(json!({ "id": id.as_str() }))
}

fn disconnect(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Disconnect = parse(p)?;
    let id = match (p.id, p.from, p.to) {
        (Some(id), None, None) => ElementId::unchecked(&id),
        (None, Some(from), Some(to)) => e
            .model()
            .connections
            .iter()
            .find(|c| c.from.as_str() == from && c.to.as_str() == to)
            .map(|c| c.id.clone())
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::NotFound,
                    format!("no connection from `{from}` to `{to}`"),
                )
            })?,
        _ => {
            return Err(CommandError::new(
                ErrorCode::InvalidParams,
                "give either `id`, or both `from` and `to`",
            ));
        }
    };
    e.edit("disconnect", Op::Disconnect(id.clone()))?;
    Ok(json!({ "id": id.as_str() }))
}

fn experiment_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: ExperimentSet = parse(p)?;
    let mut x = e.model().experiment.clone();
    if let Some(t) = p.time_unit {
        x.time_unit = t.into();
    }
    x.warmup = p.warmup.unwrap_or(x.warmup);
    x.duration = p.duration.unwrap_or(x.duration);
    x.replications = p.replications.unwrap_or(x.replications);
    x.seed = p.seed.unwrap_or(x.seed);
    let given = [
        p.time_unit.is_some(),
        p.warmup.is_some(),
        p.duration.is_some(),
        p.replications.is_some(),
        p.seed.is_some(),
    ];
    if !given.contains(&true) {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            "nothing to set: give at least one setting",
        ));
    }
    e.edit("experiment.set", Op::SetExperiment(x))?;
    Ok(json!({}))
}

fn undo(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    let label = e.undo()?;
    Ok(json!({ "undone": label }))
}

fn redo(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    let label = e.redo()?;
    Ok(json!({ "redone": label }))
}

fn history(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    Ok(e.history_json())
}

fn run(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Run = parse(p)?;
    e.plan(p.replications, p.seed, p.threads)?.run_now()
}

fn run_start(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: RunStart = parse(p)?;
    let plan = e.plan(p.replications, p.seed, p.threads)?;
    e.start_run(plan, p.mode.unwrap_or_else(RunMode::default_for_target))
}

fn run_status(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: RunStatus = parse(p)?;
    Ok(e.poll_run(p.step.unwrap_or(1)))
}

fn run_cancel(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    Ok(e.cancel_run())
}

fn file_open(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: FileOpen = parse(p)?;
    let bytes = std::fs::read(&p.path)
        .map_err(|err| CommandError::new(ErrorCode::Io, format!("{}: {err}", p.path)))?;
    e.open_bytes(&p.path, &bytes, p.scenario.as_deref())
}

fn file_save(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: FileSave = parse(p)?;
    let path = p
        .path
        .or_else(|| e.path().map(str::to_owned))
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidParams,
                "the model has no file yet: give a `path`",
            )
        })?;
    let bytes = e.save_bytes(p.as_new)?;
    std::fs::write(&path, &bytes)
        .map_err(|err| CommandError::new(ErrorCode::Io, format!("{path}: {err}")))?;
    e.saved_to(&path, &bytes)
}

fn example_model() -> Value {
    json!({
        "name": "Help desk",
        "elements": [
            {"id": "arrivals", "kind": "source", "interarrival": {"type": "exponential", "mean": 2.0}},
            {"id": "desk", "kind": "server", "capacity": 1, "service": {"type": "exponential", "mean": 1.5}},
            {"id": "exit", "kind": "sink"}
        ],
        "connections": [
            {"id": "flow-1", "from": "arrivals", "to": "desk"},
            {"id": "flow-2", "from": "desk", "to": "exit"}
        ],
        "experiment": {"time_unit": "min", "warmup": 60.0, "duration": 480.0, "replications": 10, "seed": 1}
    })
}

/// Every command, in the order the documentation lists them. The examples, run in this order on
/// a new engine, all succeed (a test checks it).
pub static COMMANDS: &[Command] = &[
    Command {
        name: "model.set",
        summary: "Replace the whole model with the one given.",
        kind: CommandKind::Edit,
        schema: schema::<ModelSet>,
        example: || json!({ "model": example_model() }),
        run: model_set,
    },
    Command {
        name: "model.get",
        summary: "The model: elements, connections and experiment.",
        kind: CommandKind::Query,
        schema: schema::<NoParams>,
        example: || json!({}),
        run: model_get,
    },
    Command {
        name: "model.validate",
        summary: "Every problem with the model, errors first, each with a stable code.",
        kind: CommandKind::Query,
        schema: schema::<NoParams>,
        example: || json!({}),
        run: model_validate,
    },
    Command {
        name: "model.rename",
        summary: "Rename the model.",
        kind: CommandKind::Edit,
        schema: schema::<Rename>,
        example: || json!({ "name": "Help desk, two shifts" }),
        run: model_rename,
    },
    Command {
        name: "block.add",
        summary: "Add a source, server, delay or sink. Returns its id.",
        kind: CommandKind::Edit,
        schema: schema::<BlockAdd>,
        example: || json!({ "kind": "delay", "id": "walk", "duration": {"type": "triangular", "min": 1.0, "mode": 2.0, "max": 4.0} }),
        run: block_add,
    },
    Command {
        name: "block.set",
        summary: "Change an element's parameters.",
        kind: CommandKind::Edit,
        schema: schema::<BlockSet>,
        example: || json!({ "id": "desk", "capacity": 2 }),
        run: block_set,
    },
    Command {
        name: "block.rename",
        summary: "Change an element's display name (its id stays).",
        kind: CommandKind::Edit,
        schema: schema::<BlockRename>,
        example: || json!({ "id": "desk", "name": "Front desk" }),
        run: block_rename,
    },
    Command {
        name: "connect",
        summary: "Connect one element to another. Returns the connection's id.",
        kind: CommandKind::Edit,
        schema: schema::<Connect>,
        example: || json!({ "from": "walk", "to": "exit" }),
        run: connect,
    },
    Command {
        name: "disconnect",
        summary: "Remove a connection, by id or by the elements it joins.",
        kind: CommandKind::Edit,
        schema: schema::<Disconnect>,
        example: || json!({ "from": "walk", "to": "exit" }),
        run: disconnect,
    },
    Command {
        name: "block.remove",
        summary: "Remove an element and its connections.",
        kind: CommandKind::Edit,
        schema: schema::<Id>,
        example: || json!({ "id": "walk" }),
        run: block_remove,
    },
    Command {
        name: "experiment.set",
        summary: "Change the experiment: time unit, warm-up, duration, replications, seed.",
        kind: CommandKind::Edit,
        schema: schema::<ExperimentSet>,
        example: || json!({ "replications": 5, "duration": 240.0 }),
        run: experiment_set,
    },
    Command {
        name: "undo",
        summary: "Undo the last edit.",
        kind: CommandKind::Action,
        schema: schema::<NoParams>,
        example: || json!({}),
        run: undo,
    },
    Command {
        name: "redo",
        summary: "Redo the last undone edit.",
        kind: CommandKind::Action,
        schema: schema::<NoParams>,
        example: || json!({}),
        run: redo,
    },
    Command {
        name: "history",
        summary: "The edits that can be undone and redone, and whether the model has unsaved changes.",
        kind: CommandKind::Query,
        schema: schema::<NoParams>,
        example: || json!({}),
        run: history,
    },
    Command {
        name: "run",
        summary: "Run the experiment now and return its results: a 95% confidence interval per element and metric.",
        kind: CommandKind::Action,
        schema: schema::<Run>,
        example: || json!({ "replications": 4 }),
        run,
    },
    Command {
        name: "run.start",
        summary: "Start the experiment in the background; follow it with run.status.",
        kind: CommandKind::Action,
        schema: schema::<RunStart>,
        example: || json!({ "replications": 4, "mode": "slices" }),
        run: run_start,
    },
    Command {
        name: "run.status",
        summary: "Progress of the background run, and its results once finished.",
        kind: CommandKind::Query,
        schema: schema::<RunStatus>,
        example: || json!({ "step": 4 }),
        run: run_status,
    },
    Command {
        name: "run.cancel",
        summary: "Stop the background run.",
        kind: CommandKind::Action,
        schema: schema::<NoParams>,
        example: || json!({}),
        run: run_cancel,
    },
    Command {
        name: "file.save",
        summary: "Save the model: into the file it was opened from, or as a new BPMN file with BPSim.",
        kind: CommandKind::Action,
        schema: schema::<FileSave>,
        example: || json!({ "path": "help-desk.bpmn" }),
        run: file_save,
    },
    Command {
        name: "file.open",
        summary: "Open a BPMN or XPDL file with BPSim data, replacing the model and clearing the history.",
        kind: CommandKind::Action,
        schema: schema::<FileOpen>,
        example: || json!({ "path": "help-desk.bpmn" }),
        run: file_open,
    },
    Command {
        name: "model.new",
        summary: "Start a new, empty model, clearing the history.",
        kind: CommandKind::Action,
        schema: schema::<ModelNew>,
        example: || json!({ "name": "Warehouse" }),
        run: model_new,
    },
];
