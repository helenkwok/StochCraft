//! The command catalogue: each command's name, summary, parameters (with their JSON schema), an
//! example and its implementation.

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use stochcraft_model::{
    Bounds, Connection, Element, ElementId, ElementKind, Model, Need, Property, ResourcePool,
    calendar::Calendar,
};

use crate::edit::Op;
use crate::run::RunMode;
use crate::wire::{
    self, AvailabilityDoc, BreakdownSpec, DistSpec, EstimateDoc, EventDoc, FamilySpec, KindSpec,
    ModelDoc, NeedSpec, PropertyDoc, PropertyTypeSpec, PropertyValueSpec, ShiftDoc, Stamp,
    StopSpec, TimeUnitSpec, TimedDist,
};
use crate::{CommandError, Engine, ErrorCode, HistoryEntry};

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
        Some("source") => &["interarrival", "limit", "timed", "idle_outside"],
        Some("server") => &["capacity", "service", "queue_capacity", "breakdown"],
        Some("shared_task") => &["service", "needs", "priority"],
        Some("delay") => &["duration"],
        Some("exclusive_gateway") => &["default"],
        Some("batch") => &["size"],
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

/// Whether `id` is used by the model, or by the opened file for something it keeps but the model
/// does not have (an element StochCraft does not simulate, a lane, a scenario, a diagram shape).
fn id_taken(e: &Engine, id: &ElementId) -> bool {
    let model = e.model();
    model.elements.iter().any(|x| &x.id == id)
        || model.connections.iter().any(|c| &c.id == id)
        || model.resources.iter().any(|r| &r.id == id)
        || model.calendars.iter().any(|c| &c.id == id)
        || e.file_uses_id(id.as_str())
}

/// The id given, which must be new, or a fresh `prefix-N`.
fn new_id(e: &Engine, given: Option<&str>, prefix: &str) -> Result<ElementId, CommandError> {
    if let Some(s) = given {
        let id = id_param(s)?;
        if id_taken(e, &id) {
            return Err(CommandError::new(
                ErrorCode::Conflict,
                format!("the id `{id}` is already used"),
            ));
        }
        return Ok(id);
    }
    (1u64..)
        .map(|n| ElementId::unchecked(&format!("{prefix}-{n}")))
        .find(|id| !id_taken(e, id))
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
    /// Where to draw it: its shape's left edge, in the diagram's units. By default, in a model
    /// that is laid out, to the right of the element before it.
    #[serde(default)]
    pub x: Option<f64>,
    /// Its shape's top edge (y grows downwards).
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(flatten)]
    pub kind: KindSpec,
    /// Properties the element sets on entities.
    #[serde(default)]
    pub properties: Vec<PropertyDoc>,
}

/// Sets one property on every entity entering an element, adding it or replacing its value.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PropertySet {
    /// The element's id.
    pub element: String,
    pub name: String,
    /// The declared type; left out, an existing property keeps its own; `null` removes it.
    #[serde(default, rename = "type", deserialize_with = "some")]
    #[schemars(with = "Option<PropertyTypeSpec>")]
    pub ty: Option<Option<PropertyTypeSpec>>,
    pub value: PropertyValueSpec,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PropertyRemove {
    /// The element's id.
    pub element: String,
    pub name: String,
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
    /// Sources: replaces the times between arrivals that apply while a calendar is active
    /// (the first active one applying; `interarrival` outside them).
    #[serde(default)]
    pub timed: Option<Vec<TimedDist>>,
    /// Sources: no arrivals outside the calendars of `timed`.
    #[serde(default)]
    pub idle_outside: Option<bool>,
    /// Servers: number of identical units.
    #[serde(default)]
    pub capacity: Option<u32>,
    /// Servers: time to serve one entity.
    #[serde(default)]
    pub service: Option<DistSpec>,
    /// Servers: a finite buffer, the most entities that wait for a unit (not counting those in
    /// service); 0 means no waiting room. While it is full, a server or delay sending here
    /// holds its finished entity (blocking after service), a gateway waits, and a source's
    /// arrival is lost. `null` removes the limit.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<u32>")]
    pub queue_capacity: Option<Option<u32>>,
    /// Servers: each unit fails after a time to failure and is repaired; `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<BreakdownSpec>")]
    pub breakdown: Option<Option<BreakdownSpec>>,
    /// Tasks with shared resources: what each entity needs, the alternatives in order.
    #[serde(default)]
    pub needs: Option<Vec<NeedSpec>>,
    /// Tasks with shared resources: who gets freed units first, higher first.
    #[serde(default)]
    pub priority: Option<f64>,
    /// Delays: time each entity spends.
    #[serde(default)]
    pub duration: Option<DistSpec>,
    /// Exclusive gateways: the id of the outgoing connection taken when no condition holds;
    /// `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<String>")]
    pub default: Option<Option<String>>,
    /// A batch: how many entities it collects before sending them on together.
    #[serde(default)]
    pub size: Option<u32>,
}

/// A new shared resource pool.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceAdd {
    /// A new id; by default `resource-N`.
    #[serde(default)]
    pub id: Option<String>,
    /// Display name; by default the id.
    #[serde(default)]
    pub name: Option<String>,
    /// How many identical units; 1 by default.
    #[serde(default = "one_unit")]
    pub quantity: u32,
    /// The roles its units play, for tasks that need units by role.
    #[serde(default)]
    pub roles: Vec<String>,
}

fn one_unit() -> u32 {
    1
}

/// A pool's settings to change.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceSet {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub quantity: Option<u32>,
    /// Replaces the pool's roles.
    #[serde(default)]
    pub roles: Option<Vec<String>>,
    /// Replaces the pool's shifts: its units while each calendar is active (the first active
    /// one applying; `quantity` outside them).
    #[serde(default)]
    pub shifts: Option<Vec<ShiftDoc>>,
    /// Whether the units are available outside the calendars of `availability`.
    #[serde(default)]
    pub available: Option<bool>,
    /// Replaces the pool's availability while each calendar is active.
    #[serde(default)]
    pub availability: Option<Vec<AvailabilityDoc>>,
}

/// A new calendar.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalendarAdd {
    /// A new id; by default `calendar-N`.
    #[serde(default)]
    pub id: Option<String>,
    /// Display name; by default the id.
    #[serde(default)]
    pub name: Option<String>,
    /// Its periods; a calendar is active while any is under way.
    #[serde(default)]
    pub events: Vec<EventDoc>,
}

/// A calendar's settings to change.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalendarSet {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Replaces its periods.
    #[serde(default)]
    pub events: Option<Vec<EventDoc>>,
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
    /// Out of an exclusive gateway: the chance an entity takes this connection.
    #[serde(default)]
    pub probability: Option<f64>,
    /// Out of an exclusive gateway: a BPSim expression that must hold to take this connection,
    /// such as `getProperty('issues') > 0`.
    #[serde(default)]
    pub condition: Option<String>,
    /// Into a parallel gateway: how many entities from this connection go into each one it
    /// sends on (an assembly's bill of materials); by default 1.
    #[serde(default)]
    pub quantity: Option<u32>,
}

/// How an exclusive gateway chooses a connection: the parameters to change.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowSet {
    /// The connection's id.
    pub id: String,
    /// The chance an entity takes this connection, from 0 to 1. Connections out of the same
    /// gateway without one share evenly what the others leave. `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<f64>")]
    pub probability: Option<Option<f64>>,
    /// A BPSim expression that must hold for an entity to take this connection, such as
    /// `getProperty('issues') > 0`. If any connection out of a gateway has one, the gateway
    /// routes by condition. `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<String>")]
    pub condition: Option<Option<String>>,
    /// Into a parallel gateway: how many entities from this connection go into each one it
    /// sends on (an assembly's bill of materials). `null` removes it (1).
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<u32>")]
    pub quantity: Option<Option<u32>>,
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
    /// The date and time model time 0 stands for, such as `2016-01-01T06:00:00`, which places
    /// the calendars; `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<String>")]
    pub start: Option<Option<Stamp>>,
    /// What ends each replication early (a terminating experiment), the warm-up plus the
    /// duration then being its time limit; `null` removes it.
    #[serde(default, deserialize_with = "some")]
    pub stop: Option<Option<StopSpec>>,
}

/// A three-point estimate to fit an element's time distribution to: a source's time between
/// arrivals, a task's service time or a delay's duration. The element then uses the fitted
/// distribution (a standard BPSim one), and keeps the estimate as its pedigree.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EstimateSet {
    pub id: String,
    pub family: FamilySpec,
    pub min: f64,
    /// The most likely value.
    pub mode: f64,
    /// The largest value possible (triangular and PERT); give this or `p95`.
    #[serde(default)]
    pub max: Option<f64>,
    /// The value exceeded one time in twenty.
    #[serde(default)]
    pub p95: Option<f64>,
    /// Who gave the estimate or where it comes from.
    #[serde(default)]
    pub source: String,
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
    /// `threads` natively, `slices` in the browser by default; `workers` hands the replications
    /// to the caller's own workers (see `Engine::worker_task`).
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
            check_keys(el, &["id", "name", "properties"])?;
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
    check_keys(&p, &["id", "name", "index", "properties", "x", "y"])?;
    let p: BlockAdd = parse(p)?;
    let kind: ElementKind = p.kind.into();
    let id = new_id(e, p.id.as_deref(), kind.name())?;
    let index = p
        .index
        .map_or(e.model().elements.len(), |i| i as usize)
        .min(e.model().elements.len());
    let element = Element {
        name: p.name.unwrap_or_else(|| id.to_string()),
        id: id.clone(),
        kind,
        properties: wire::sorted_properties(p.properties.into_iter().map(Into::into)),
    };
    let bounds = new_shape(e.model(), &element.kind, index, p.x, p.y)?;
    e.edit(
        "block.add",
        Op::AddElement {
            index,
            element,
            connections: vec![],
            bounds,
        },
    )?;
    Ok(json!({ "id": id.as_str() }))
}

fn finite(v: f64, what: &str) -> Result<f64, CommandError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!("{what} must be a finite number"),
        ))
    }
}

/// A new element's shape: where it was asked to go, or in a laid-out model to the right of the
/// element before it, centred on it; in a model with no layout, none.
fn new_shape(
    model: &Model,
    kind: &ElementKind,
    index: usize,
    x: Option<f64>,
    y: Option<f64>,
) -> Result<Option<Bounds>, CommandError> {
    let (width, height) = kind.shape_size();
    let placed = model.placed();
    let prev = index
        .checked_sub(1)
        .and_then(|i| model.elements.get(i))
        .and_then(|p| placed.get(&p.id));
    let auto = match prev {
        Some(p) if !model.layout.is_empty() => {
            Some((p.x + p.width + 50.0, p.y + p.height / 2.0 - height / 2.0))
        }
        _ => None,
    };
    let (x, y) = match (x, y, auto) {
        (Some(x), Some(y), _) => (finite(x, "x")?, finite(y, "y")?),
        (None, None, None) => return Ok(None),
        (x, y, auto) => {
            let (ax, ay) = auto.unwrap_or((40.0, 40.0));
            (finite(x.unwrap_or(ax), "x")?, finite(y.unwrap_or(ay), "y")?)
        }
    };
    Ok(Some(Bounds {
        x,
        y,
        width,
        height,
    }))
}

/// Moves or resizes elements' shapes, all in one undoable step.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayoutSet {
    pub shapes: Vec<ShapeSet>,
}

/// One element's shape: what is left out stays as it is (a shape not placed yet starts where
/// it is drawn now, at its kind's size).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShapeSet {
    /// The element's id.
    pub id: String,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
}

fn layout_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: LayoutSet = parse(p)?;
    let placed = e.model().placed();
    let mut shapes = Vec::with_capacity(p.shapes.len());
    for s in p.shapes {
        let id = existing_element(e.model(), &s.id)?;
        let Some(now) = placed.get(&id).copied() else {
            return Err(CommandError::new(
                ErrorCode::Internal,
                format!("`{id}` has no place"),
            ));
        };
        let size = |v: Option<f64>, old: f64, what: &str| -> Result<f64, CommandError> {
            let v = finite(v.unwrap_or(old), what)?;
            if v <= 0.0 {
                return Err(CommandError::new(
                    ErrorCode::InvalidParams,
                    format!("{what} must be more than 0"),
                ));
            }
            Ok(v)
        };
        let b = Bounds {
            x: finite(s.x.unwrap_or(now.x), "x")?,
            y: finite(s.y.unwrap_or(now.y), "y")?,
            width: size(s.width, now.width, "width")?,
            height: size(s.height, now.height, "height")?,
        };
        shapes.push((id, Some(b)));
    }
    e.edit("layout.set", Op::SetLayout(shapes))?;
    Ok(json!({}))
}

/// Several editing commands as one step: undone and redone together, and if one fails none of
/// them is applied.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub commands: Vec<BatchStep>,
    /// What the step is called in the undo history: `paste`, `delete`, `align` or `duplicate`;
    /// by default `batch`.
    #[serde(default)]
    pub label: Option<String>,
}

/// One command of a batch.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchStep {
    /// An editing command's name, such as `block.add`.
    pub command: String,
    #[serde(default)]
    pub params: Value,
}

/// The names a batch can be given in the history.
const BATCH_LABELS: [&str; 5] = ["batch", "paste", "delete", "align", "duplicate"];

fn batch(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Batch = parse(p)?;
    let label = match p.label.as_deref() {
        None => "batch",
        Some(l) => BATCH_LABELS
            .iter()
            .copied()
            .find(|b| *b == l)
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::InvalidParams,
                    format!("a batch's label is one of {BATCH_LABELS:?}, not `{l}`"),
                )
            })?,
    };
    let start = e.undo.len();
    let redo = e.redo.clone();
    let mut results = Vec::with_capacity(p.commands.len());
    for (i, step) in p.commands.into_iter().enumerate() {
        let params = if step.params.is_null() {
            json!({})
        } else {
            step.params
        };
        let result = match COMMANDS.iter().find(|c| c.name == step.command) {
            Some(c) if c.kind == CommandKind::Edit => (c.run)(e, params),
            Some(_) => Err(CommandError::new(
                ErrorCode::InvalidParams,
                format!("`{}` is not an editing command", step.command),
            )),
            None => Err(CommandError::new(
                ErrorCode::UnknownCommand,
                format!("no command `{}`", step.command),
            )),
        };
        match result {
            Ok(v) => results.push(v),
            Err(err) => {
                // Take back the steps already done, so the model is as it was.
                while e.undo.len() > start {
                    if let Some(entry) = e.undo.pop() {
                        let _ = crate::edit::apply(&mut e.model, entry.op);
                    }
                }
                e.redo = redo;
                return Err(CommandError {
                    message: format!("step {} (`{}`): {}", i + 1, step.command, err.message),
                    ..err
                });
            }
        }
    }
    let ops: Vec<Op> = e.undo.drain(start..).rev().map(|h| h.op).collect();
    if !ops.is_empty() {
        e.undo.push(HistoryEntry {
            label,
            op: Op::Batch(ops),
        });
    }
    Ok(json!({ "results": results }))
}

fn block_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Id = parse(p)?;
    let id = existing_element(e.model(), &p.id)?;
    // Its estimate goes with it, and a stop condition watching it.
    let mut ops = Vec::new();
    if e.model().estimates.contains_key(&id) {
        ops.push(Op::SetEstimate(id.clone(), None));
    }
    let x = &e.model().experiment;
    if x.stop.as_ref().is_some_and(|s| s.element() == &id) {
        let mut x = x.clone();
        x.stop = None;
        ops.push(Op::SetExperiment(x));
    }
    ops.push(Op::RemoveElement(id));
    e.edit("block.remove", Op::Batch(ops))?;
    Ok(json!({}))
}

fn estimate_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: EstimateSet = parse(p)?;
    let id = existing_element(e.model(), &p.id)?;
    let estimate = EstimateDoc {
        family: p.family,
        min: p.min,
        mode: p.mode,
        max: p.max,
        p95: p.p95,
        source: p.source,
    }
    .to_estimate()
    .map_err(|m| CommandError::new(ErrorCode::InvalidParams, m))?;
    let fitted = estimate
        .three_point
        .fit()
        .map_err(|err| CommandError::new(ErrorCode::InvalidParams, err.to_string()))?;
    let mut kind = e
        .model()
        .element(&id)
        .map_or(ElementKind::Sink, |el| el.kind.clone());
    match &mut kind {
        ElementKind::Source {
            interarrival: d, ..
        }
        | ElementKind::Server { service: d, .. }
        | ElementKind::SharedTask { service: d, .. }
        | ElementKind::Delay { duration: d } => *d = fitted.clone(),
        _ => {
            return Err(CommandError::new(
                ErrorCode::InvalidParams,
                format!("`{id}` has no time distribution to fit an estimate to"),
            ));
        }
    }
    e.edit(
        "estimate.set",
        Op::Batch(vec![
            Op::SetKind(id.clone(), kind),
            Op::SetEstimate(id, Some(estimate)),
        ]),
    )?;
    Ok(json!({ "distribution": DistSpec::from(&fitted), "mean": fitted.mean() }))
}

fn estimate_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Id = parse(p)?;
    let id = existing_element(e.model(), &p.id)?;
    if !e.model().estimates.contains_key(&id) {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            format!("`{id}` has no three-point estimate"),
        ));
    }
    e.edit("estimate.remove", Op::SetEstimate(id, None))?;
    Ok(json!({}))
}

fn experiment_sweep(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::decide::SweepParams = parse(p)?;
    let v = crate::decide::sweep(e.model(), &p)?;
    e.sensitivity.push(json!({
        "kind": "sweep",
        "model_digest": crate::model_digest(e.model()),
        "parameter": v["parameter"],
        "values": p.values,
        "effects": v["effects"],
    }));
    Ok(v)
}

fn experiment_sensitivity(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::decide::SensitivityParams = parse(p)?;
    let v = crate::decide::sensitivity(e.model(), &p)?;
    e.sensitivity.push(json!({
        "kind": "one at a time",
        "model_digest": crate::model_digest(e.model()),
        "change": v["change"],
        "effects": v["effects"],
    }));
    Ok(v)
}

fn experiment_size(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::decide::SizeParams = parse(p)?;
    let (mut v, sized) = crate::decide::size(e.model(), &p)?;
    if p.apply && &sized != e.model() {
        e.edit("experiment.size", Op::SetModel(Box::new(sized)))?;
        v["applied"] = json!(true);
    }
    Ok(v)
}

fn results_distribution(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::distribution::Params = parse(p)?;
    let results = e.last_results.as_ref().ok_or_else(|| {
        CommandError::new(
            ErrorCode::NotFound,
            "no run has finished yet: run the experiment first",
        )
    })?;
    crate::distribution::describe(results, &p)
}

fn results_compare(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::compare::Params = parse(p)?;
    let need = |r: Option<&Value>, which: &str| {
        r.cloned().ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                format!(
                    "no {which} run to compare: run the experiment, change the model, run it again, then compare (or give `a` and `b`)"
                ),
            )
        })
    };
    let b = match p.b {
        Some(b) => b,
        None => need(e.last_results.as_ref(), "last")?,
    };
    let a = match p.a {
        Some(a) => a,
        None => need(e.previous_results.as_ref(), "earlier")?,
    };
    crate::compare::compare(&a, &b, p.results.as_deref())
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
        ("timed", p.timed.is_some()),
        ("idle_outside", p.idle_outside.is_some()),
        ("capacity", p.capacity.is_some()),
        ("service", p.service.is_some()),
        ("queue_capacity", p.queue_capacity.is_some()),
        ("breakdown", p.breakdown.is_some()),
        ("needs", p.needs.is_some()),
        ("priority", p.priority.is_some()),
        ("duration", p.duration.is_some()),
        ("default", p.default.is_some()),
        ("size", p.size.is_some()),
    ];
    let applies: &[&str] = match kind {
        ElementKind::Source { .. } => &["interarrival", "limit", "timed", "idle_outside"],
        ElementKind::Server { .. } => &["capacity", "service", "queue_capacity", "breakdown"],
        ElementKind::SharedTask { .. } => &["service", "needs", "priority"],
        ElementKind::Delay { .. } => &["duration"],
        ElementKind::ExclusiveGateway { .. } => &["default"],
        ElementKind::Batch { .. } => &["size"],
        ElementKind::Sink | ElementKind::ParallelGateway | ElementKind::Unbatch => &[],
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
            timed,
            idle_outside,
        } => {
            if let Some(d) = p.interarrival {
                *interarrival = d.into();
            }
            if let Some(l) = p.limit {
                *limit = l;
            }
            if let Some(t) = p.timed {
                *timed = t.into_iter().map(Into::into).collect();
            }
            if let Some(i) = p.idle_outside {
                *idle_outside = i;
            }
        }
        ElementKind::Server {
            capacity,
            service,
            queue_capacity,
            breakdown,
        } => {
            if let Some(b) = p.breakdown {
                *breakdown = b.map(Into::into);
            }
            if let Some(c) = p.capacity {
                *capacity = c;
            }
            if let Some(d) = p.service {
                *service = d.into();
            }
            if let Some(q) = p.queue_capacity {
                *queue_capacity = q;
            }
        }
        ElementKind::SharedTask {
            service,
            needs,
            priority,
        } => {
            if let Some(d) = p.service {
                *service = d.into();
            }
            if let Some(n) = p.needs {
                *needs = n.into_iter().map(Into::into).collect();
            }
            if let Some(x) = p.priority {
                *priority = x;
            }
        }
        ElementKind::Delay { duration } => {
            if let Some(d) = p.duration {
                *duration = d.into();
            }
        }
        ElementKind::ExclusiveGateway { default } => {
            if let Some(d) = p.default {
                *default = d.as_deref().map(ElementId::unchecked);
            }
        }
        ElementKind::Batch { size } => {
            if let Some(n) = p.size {
                *size = n;
            }
        }
        ElementKind::Sink | ElementKind::ParallelGateway | ElementKind::Unbatch => {}
    }
    e.edit("block.set", Op::SetKind(id, kind))?;
    Ok(json!({}))
}

fn existing_pool(model: &Model, s: &str) -> Result<usize, CommandError> {
    model
        .resources
        .iter()
        .position(|r| r.id.as_str() == s)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                format!("no resource pool with id `{s}`"),
            )
        })
}

fn resource_add(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: ResourceAdd = parse(p)?;
    let id = new_id(e, p.id.as_deref(), "resource")?;
    let mut pools = e.model().resources.clone();
    pools.push(ResourcePool::new(
        id.clone(),
        p.name.unwrap_or_else(|| id.to_string()),
        p.quantity,
        p.roles,
    ));
    e.edit("resource.add", Op::SetResources(pools))?;
    Ok(json!({ "id": id.as_str() }))
}

fn resource_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: ResourceSet = parse(p)?;
    let i = existing_pool(e.model(), &p.id)?;
    if p.name.is_none()
        && p.quantity.is_none()
        && p.roles.is_none()
        && p.shifts.is_none()
        && p.available.is_none()
        && p.availability.is_none()
    {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            "nothing to set: give at least one parameter",
        ));
    }
    let mut pools = e.model().resources.clone();
    if let Some(r) = pools.get_mut(i) {
        if let Some(n) = p.name {
            r.name = n;
        }
        if let Some(q) = p.quantity {
            r.quantity = q;
        }
        if let Some(roles) = p.roles {
            r.roles = roles;
        }
        if let Some(sh) = p.shifts {
            r.shifts = sh.into_iter().map(Into::into).collect();
        }
        if let Some(a) = p.available {
            r.available = a;
        }
        if let Some(a) = p.availability {
            r.availability = a.into_iter().map(Into::into).collect();
        }
    }
    e.edit("resource.set", Op::SetResources(pools))?;
    Ok(json!({}))
}

fn existing_calendar(model: &Model, s: &str) -> Result<usize, CommandError> {
    model
        .calendars
        .iter()
        .position(|c| c.id.as_str() == s)
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, format!("no calendar with id `{s}`")))
}

fn calendar_add(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: CalendarAdd = parse(p)?;
    let id = new_id(e, p.id.as_deref(), "calendar")?;
    let mut all = e.model().calendars.clone();
    all.push(Calendar {
        name: p.name.unwrap_or_else(|| id.to_string()),
        id: id.clone(),
        events: p.events.into_iter().map(Into::into).collect(),
    });
    e.edit("calendar.add", Op::SetCalendars(all))?;
    Ok(json!({ "id": id.as_str() }))
}

fn calendar_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: CalendarSet = parse(p)?;
    let i = existing_calendar(e.model(), &p.id)?;
    if p.name.is_none() && p.events.is_none() {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            "nothing to set: give at least one parameter",
        ));
    }
    let mut all = e.model().calendars.clone();
    if let Some(c) = all.get_mut(i) {
        if let Some(n) = p.name {
            c.name = n;
        }
        if let Some(ev) = p.events {
            c.events = ev.into_iter().map(Into::into).collect();
        }
    }
    e.edit("calendar.set", Op::SetCalendars(all))?;
    Ok(json!({}))
}

fn calendar_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Id = parse(p)?;
    let i = existing_calendar(e.model(), &p.id)?;
    let m = e.model();
    let mut users: Vec<&str> = m
        .elements
        .iter()
        .filter(|el| {
            matches!(&el.kind, ElementKind::Source { timed, .. }
                if timed.iter().any(|t| t.calendar.as_str() == p.id))
        })
        .map(|el| el.id.as_str())
        .collect();
    users.extend(
        m.resources
            .iter()
            .filter(|r| r.calendars().any(|c| c.as_str() == p.id))
            .map(|r| r.id.as_str()),
    );
    if !users.is_empty() {
        return Err(CommandError::new(
            ErrorCode::Conflict,
            format!(
                "`{}` is used by {}; remove their values for it first",
                p.id,
                users.join(", ")
            ),
        ));
    }
    let mut all = m.calendars.clone();
    all.remove(i);
    e.edit("calendar.remove", Op::SetCalendars(all))?;
    Ok(json!({}))
}

fn resource_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Id = parse(p)?;
    let i = existing_pool(e.model(), &p.id)?;
    let users: Vec<&str> = e
        .model()
        .elements
        .iter()
        .filter(|el| match &el.kind {
            ElementKind::SharedTask { needs, .. } => needs
                .iter()
                .any(|n| matches!(n, Need::Resource { id, .. } if id.as_str() == p.id)),
            _ => false,
        })
        .map(|el| el.id.as_str())
        .collect();
    if !users.is_empty() {
        return Err(CommandError::new(
            ErrorCode::Conflict,
            format!(
                "`{}` is needed by {}; change their needs first",
                p.id,
                users.join(", ")
            ),
        ));
    }
    let mut pools = e.model().resources.clone();
    pools.remove(i);
    e.edit("resource.remove", Op::SetResources(pools))?;
    Ok(json!({}))
}

fn block_rename(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: BlockRename = parse(p)?;
    let id = existing_element(e.model(), &p.id)?;
    e.edit("block.rename", Op::RenameElement(id, p.name))?;
    Ok(json!({}))
}

fn property_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: PropertySet = parse(p)?;
    let id = existing_element(e.model(), &p.element)?;
    if p.name.trim().is_empty() {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            "a property needs a name",
        ));
    }
    let mut props = e
        .model()
        .element(&id)
        .map_or_else(Vec::new, |el| el.properties.clone());
    let old = props.iter().position(|q| q.name == p.name);
    let ty = match p.ty {
        Some(t) => t.map(Into::into),
        None => old.and_then(|i| props.get(i)).and_then(|q| q.ty),
    };
    let property = Property {
        name: p.name.clone(),
        ty,
        value: p.value.into(),
    };
    match old {
        Some(i) => props[i] = property,
        None => props.push(property),
    }
    e.edit(
        "property.set",
        Op::SetProperties(id, wire::sorted_properties(props)),
    )?;
    Ok(json!({}))
}

fn property_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: PropertyRemove = parse(p)?;
    let id = existing_element(e.model(), &p.element)?;
    let mut props = e
        .model()
        .element(&id)
        .map_or_else(Vec::new, |el| el.properties.clone());
    let before = props.len();
    props.retain(|q| q.name != p.name);
    if props.len() == before {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            format!("`{}` sets no property `{}`", p.element, p.name),
        ));
    }
    e.edit("property.remove", Op::SetProperties(id, props))?;
    Ok(json!({}))
}

/// Every property the model sets: its type and the elements that set it.
fn property_list(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    let m = e.model();
    let list: Vec<Value> = m
        .property_types()
        .into_iter()
        .map(|(name, ty)| {
            let set_by: Vec<&str> = m
                .elements
                .iter()
                .filter(|el| el.property(&name).is_some())
                .map(|el| el.id.as_str())
                .collect();
            json!({ "name": name, "type": PropertyTypeSpec::from(ty), "set_by": set_by })
        })
        .collect();
    Ok(json!({ "properties": list }))
}

fn connect(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Connect = parse(p)?;
    let from = existing_element(e.model(), &p.from)?;
    let to = existing_element(e.model(), &p.to)?;
    let id = new_id(e, p.id.as_deref(), "flow")?;
    let index = e.model().connections.len();
    e.edit(
        "connect",
        Op::Connect(
            index,
            Connection {
                id: id.clone(),
                from,
                to,
                probability: p.probability,
                condition: p.condition,
                quantity: p.quantity,
            },
        ),
    )?;
    Ok(json!({ "id": id.as_str() }))
}

fn flow_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: FlowSet = parse(p)?;
    let c = e
        .model()
        .connections
        .iter()
        .find(|c| c.id.as_str() == p.id)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                format!("no connection with id `{}`", p.id),
            )
        })?;
    if p.probability.is_none() && p.condition.is_none() && p.quantity.is_none() {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            "nothing to set: give `probability`, `condition` or `quantity`",
        ));
    }
    let id = c.id.clone();
    let mut ops = Vec::new();
    if p.probability.is_some() || p.condition.is_some() {
        let probability = p.probability.unwrap_or(c.probability);
        let condition = p.condition.unwrap_or_else(|| c.condition.clone());
        ops.push(Op::SetRoute(id.clone(), probability, condition));
    }
    if let Some(q) = p.quantity {
        ops.push(Op::SetQuantity(id, q));
    }
    let op = if ops.len() == 1 {
        ops.remove(0)
    } else {
        Op::Batch(ops)
    };
    e.edit("flow.set", op)?;
    Ok(json!({}))
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
    if let Some(s) = p.start {
        x.start = s.map(|s| s.0);
    }
    if let Some(stop) = &p.stop {
        if let Some(s) = stop {
            existing_element(e.model(), s.element())?;
        }
        x.stop = stop.clone().map(Into::into);
    }
    let given = [
        p.stop.is_some(),
        p.start.is_some(),
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
    // Written to a temporary file beside it and renamed over it, so a crash or a full disk
    // leaves the old file whole rather than half written.
    craft_atomic::atomic_write(std::path::Path::new(&path), &bytes)
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
        summary: "Add a source, server, shared task, delay, sink, exclusive gateway or parallel gateway. Returns its id.",
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
        example: || json!({ "id": "desk", "capacity": 2, "queue_capacity": 8 }),
        run: block_set,
    },
    Command {
        name: "estimate.set",
        summary: "Fit an element's time distribution to a three-point estimate (minimum, most likely, maximum or 95th percentile) and keep the estimate as its pedigree. Returns the distribution.",
        kind: CommandKind::Edit,
        schema: schema::<EstimateSet>,
        example: || json!({ "id": "desk", "family": "pert", "min": 0.5, "mode": 1.2, "max": 4.0, "source": "front-desk staff, March" }),
        run: estimate_set,
    },
    Command {
        name: "estimate.remove",
        summary: "Forget an element's three-point estimate; its distribution stays.",
        kind: CommandKind::Edit,
        schema: schema::<Id>,
        example: || json!({ "id": "desk" }),
        run: estimate_remove,
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
        name: "flow.set",
        summary: "Set how an exclusive gateway chooses a connection: its probability, or a condition that must hold.",
        kind: CommandKind::Edit,
        schema: schema::<FlowSet>,
        example: || json!({ "id": "flow-2", "probability": 0.8 }),
        run: flow_set,
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
        name: "layout.set",
        summary: "Move or resize elements' shapes in the diagram, in one step.",
        kind: CommandKind::Edit,
        schema: schema::<LayoutSet>,
        example: || json!({ "shapes": [{ "id": "desk", "x": 240, "y": 80 }] }),
        run: layout_set,
    },
    Command {
        name: "batch",
        summary: "Run several editing commands as one step, undone together; if one fails, none is applied.",
        kind: CommandKind::Edit,
        schema: schema::<Batch>,
        example: || {
            json!({
                "label": "paste",
                "commands": [
                    { "command": "block.add", "params": { "id": "desk-copy", "kind": "delay", "duration": { "type": "constant", "value": 1.0 } } },
                    { "command": "connect", "params": { "from": "desk-copy", "to": "exit" } }
                ]
            })
        },
        run: batch,
    },
    Command {
        name: "property.set",
        summary: "Set a property on every entity entering an element (at a source: every entity it creates), from a number, true or false, text, a distribution or a BPSim expression.",
        kind: CommandKind::Edit,
        schema: schema::<PropertySet>,
        example: || json!({ "element": "arrivals", "name": "issues", "type": "long", "value": {"random": {"type": "poisson", "mean": 2.0}} }),
        run: property_set,
    },
    Command {
        name: "property.list",
        summary: "Every property the model sets, with its type and the elements that set it.",
        kind: CommandKind::Query,
        schema: schema::<NoParams>,
        example: || json!({}),
        run: property_list,
    },
    Command {
        name: "property.remove",
        summary: "Stop an element setting a property.",
        kind: CommandKind::Edit,
        schema: schema::<PropertyRemove>,
        example: || json!({ "element": "arrivals", "name": "issues" }),
        run: property_remove,
    },
    Command {
        name: "experiment.set",
        summary: "Change the experiment: time unit, warm-up, duration, replications, seed, start, and an event that ends each replication early.",
        kind: CommandKind::Edit,
        schema: schema::<ExperimentSet>,
        example: || json!({ "replications": 5, "duration": 240.0 }),
        run: experiment_set,
    },
    Command {
        name: "resource.add",
        summary: "Add a pool of units that tasks share. Returns its id.",
        kind: CommandKind::Edit,
        schema: schema::<ResourceAdd>,
        example: || json!({ "id": "agents", "name": "Support agents", "quantity": 3, "roles": ["agent"] }),
        run: resource_add,
    },
    Command {
        name: "resource.set",
        summary: "Change a shared pool's name, quantity, roles, shifts or availability.",
        kind: CommandKind::Edit,
        schema: schema::<ResourceSet>,
        example: || json!({ "id": "agents", "quantity": 4 }),
        run: resource_set,
    },
    Command {
        name: "resource.remove",
        summary: "Remove a shared pool that no task names.",
        kind: CommandKind::Edit,
        schema: schema::<Id>,
        example: || json!({ "id": "agents" }),
        run: resource_remove,
    },
    Command {
        name: "calendar.add",
        summary: "Add a calendar (shifts, opening hours) that arrival rates and pool units can follow. Returns its id.",
        kind: CommandKind::Edit,
        schema: schema::<CalendarAdd>,
        example: || json!({ "id": "weekdays", "events": [{ "start": "2016-01-04T09:00:00", "end": "2016-01-04T17:00:00", "repeat": "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR" }] }),
        run: calendar_add,
    },
    Command {
        name: "calendar.set",
        summary: "Change a calendar's name or periods.",
        kind: CommandKind::Edit,
        schema: schema::<CalendarSet>,
        example: || json!({ "id": "weekdays", "name": "Opening hours" }),
        run: calendar_set,
    },
    Command {
        name: "calendar.remove",
        summary: "Remove a calendar that no value is valid for.",
        kind: CommandKind::Edit,
        schema: schema::<Id>,
        example: || json!({ "id": "weekdays" }),
        run: calendar_remove,
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
        name: "experiment.sweep",
        summary: "Run the experiment at each value of one parameter (a task's or pool's units, a buffer, a time's mean) with the same seed, and report the results asked for at each and how much they moved.",
        kind: CommandKind::Action,
        schema: schema::<crate::decide::SweepParams>,
        example: || json!({ "parameter": { "id": "desk", "field": "capacity" }, "values": [1, 2, 3], "results": [{ "element": "desk", "metric": "mean_wait" }], "replications": 4 }),
        run: experiment_sweep,
    },
    Command {
        name: "experiment.sensitivity",
        summary: "Move each parameter down and up in turn and rank which results it drives (their elasticities); recorded in later runs' records.",
        kind: CommandKind::Action,
        schema: schema::<crate::decide::SensitivityParams>,
        example: || json!({ "parameters": [{ "id": "desk", "field": "mean" }, { "id": "arrivals", "field": "mean" }], "results": [{ "element": "exit", "metric": "mean_time_in_system" }], "replications": 4 }),
        run: experiment_sensitivity,
    },
    Command {
        name: "experiment.size",
        summary: "Add units at the bottleneck until the share of replications meeting a target reaches the confidence asked for, with the answer from mean times beside it; `apply` makes it the model's.",
        kind: CommandKind::Edit,
        schema: schema::<crate::decide::SizeParams>,
        example: || json!({ "target": { "element": "desk", "metric": "mean_wait", "at_most": 1.0 }, "confidence": 0.7, "replications": 4, "apply": true }),
        run: experiment_size,
    },
    Command {
        name: "results.distribution",
        summary: "The distribution of one result across the last run's replications: percentiles, the empirical distribution with a confidence band, the probability of meeting a target, and the values as a distribution for another model.",
        kind: CommandKind::Query,
        schema: schema::<crate::distribution::Params>,
        example: || json!({ "element": "exit", "metric": "mean_time_in_system", "at_most": 6.0 }),
        run: results_distribution,
    },
    Command {
        name: "results.compare",
        summary: "Compare two runs' results, by default the last run with the one before it: per element and metric both means and the difference with its 95% interval, paired-t when the runs share their seed and replications (common random numbers), Welch's two-sample t otherwise.",
        kind: CommandKind::Query,
        schema: schema::<crate::compare::Params>,
        example: || {
            let run = |wait: [f64; 3]| json!({ "seed": 1, "replications": 3, "values": { "desk": { "mean_wait": wait } } });
            json!({ "a": run([1.2, 0.8, 1.5]), "b": run([0.9, 0.7, 1.1]), "results": [{ "element": "desk", "metric": "mean_wait" }] })
        },
        run: results_compare,
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
