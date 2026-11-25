//! The command catalogue: each command's name, summary, parameters (with their JSON schema), an
//! example and its implementation.

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use stochcraft_model::{
    Bounds, Connection, Element, ElementId, ElementKind, Model, Need, Property, QueueLimit,
    ResourcePool, Transfer, balance, calendar::Calendar, capacity,
};

use crate::edit::Op;
use crate::run::RunMode;
use crate::wire::{
    self, AvailabilityDoc, BalkingSpec, BreakdownSpec, ConveyorShapeSpec, DemandSpec, DistSpec,
    EstimateDoc, EventDoc, FamilySpec, HealthCheckSpec, KindSpec, LengthUnitSpec, MobilityDoc,
    ModelDoc, NeedSpec, OnShiftEndSpec, PatienceSpec, PowerSpec, ProcessingByResourceSpec,
    ProcessingBySpec, PropertyDoc, PropertyTypeSpec, PropertyValueSpec, QueueShiftDoc, RoutingSpec,
    ScrapSpec, SetupSpec, ShiftDoc, Stamp, StopSpec, TimeUnitSpec, TimedDist,
};
use crate::{CommandError, Engine, ErrorCode, HistoryEntry, JournalStart, StartKind, Undo};

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

pub(crate) fn parse<T: DeserializeOwned>(params: Value) -> Result<T, CommandError> {
    serde_json::from_value(params)
        .map_err(|e| CommandError::new(ErrorCode::InvalidParams, e.to_string()))
}

/// Parameters that may not be left out of a schema'd struct, for commands with none.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// Present-but-null vs absent, for fields that can be cleared.
pub(crate) fn some<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
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
        Some("server") => &[
            "capacity",
            "service",
            "processing_by",
            "queue_capacity",
            "queue_per_working_server",
            "queue_shifts",
            "breakdown",
            "transfer",
            "transfer_holds_sender",
            "available",
            "availability",
            "power",
            "setup",
            "planned_run_time",
            "patience",
            "balking",
            "health_check",
            "shifts",
            "on_shift_end",
        ],
        Some("sink") => &["scrap", "demand"],
        Some("shared_task") => &[
            "service",
            "processing_by",
            "processing_by_resource",
            "needs",
            "priority",
        ],
        Some("delay") => &["duration", "processing_by"],
        Some("exclusive_gateway") => &["default", "routing"],
        Some("batch") => &["size"],
        Some("conveyor") => &[
            "length",
            "speed",
            "accumulating",
            "item_length",
            "gap",
            "shape",
        ],
        Some("transport") => &["pool", "load", "unload"],
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

/// The capacity table's parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapacityParams {
    /// The utilisation above which a station is congested, between 0 and 1 (default 0.85).
    #[serde(default)]
    pub target: Option<f64>,
}

/// One work element of an assembly line: its time and the elements that must come before it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkElementSpec {
    pub id: String,
    /// The time to do the element, in the same unit as the cycle time. Positive and finite.
    pub time: f64,
    /// The elements that must be done before this one.
    #[serde(default)]
    pub after: Vec<String>,
}

/// One of the three line-balancing heuristics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeuristicSpec {
    LargestCandidate,
    KilbridgeWester,
    RankedPositionalWeight,
}

impl From<HeuristicSpec> for balance::Heuristic {
    fn from(h: HeuristicSpec) -> Self {
        match h {
            HeuristicSpec::LargestCandidate => balance::Heuristic::LargestCandidate,
            HeuristicSpec::KilbridgeWester => balance::Heuristic::KilbridgeWester,
            HeuristicSpec::RankedPositionalWeight => balance::Heuristic::RankedPositionalWeight,
        }
    }
}

/// Parameters for `line.balance`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BalanceParams {
    pub elements: Vec<WorkElementSpec>,
    /// The cycle time (the longest any station may take), in the same unit as the element times.
    pub cycle: f64,
    /// One heuristic (`largest_candidate`, `kilbridge_wester` or `ranked_positional_weight`);
    /// when left out, all three are run and compared.
    #[serde(default)]
    pub heuristic: Option<HeuristicSpec>,
}

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

/// The result an observed value or bound is for, to remove it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservationRemove {
    /// The id of the block or pool.
    pub element: String,
    /// The result's name, such as `mean_wait`.
    pub metric: String,
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
    /// Servers, shared tasks and delays: processing times for entities with particular values
    /// of a property (a time by product type): `{property, by_value: [{value, time}]}`, values
    /// as text; an entity whose value has a time takes it, any other the task's own `service`
    /// (or `duration`). `null` removes them.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<ProcessingBySpec>")]
    pub processing_by: Option<Option<ProcessingBySpec>>,
    /// Shared tasks: service times by the pool whose units serve the entity (servers that differ
    /// in speed): `{by_resource: [{resource, time}]}`, each pool once and one of the task's
    /// needs; an entity served by a pool with a time takes it, ahead of `processing_by` and
    /// the task's own `service`. When a take spans pools, the pool that supplied its first unit
    /// counts. `null` removes them.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<ProcessingByResourceSpec>")]
    pub processing_by_resource: Option<Option<ProcessingByResourceSpec>>,
    /// Servers: a finite buffer, the most entities that wait for a unit (not counting those in
    /// service); 0 means no waiting room. While it is full, a server or delay sending here
    /// holds its finished entity (blocking after service), a gateway waits, and a source's
    /// arrival is lost. `null` removes the limit.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<u32>")]
    pub queue_capacity: Option<Option<u32>>,
    /// Servers with a finite buffer: `queue_capacity` is the most waiting per unit working now
    /// (up, not taken out and on shift), so the limit follows failures, health checks and
    /// shifts (true), or a fixed number (false). Setting it true clears `queue_shifts`. An
    /// entity is admitted when those waiting less the idle working units are fewer than the
    /// limit; those already waiting stay when it drops.
    #[serde(default)]
    pub queue_per_working_server: Option<bool>,
    /// Servers with a finite buffer: replaces the most waiting while each calendar is active
    /// (the first active one applying; `queue_capacity` outside them), admitted as with
    /// `queue_per_working_server`, which setting these clears. `[]` removes them.
    #[serde(default)]
    pub queue_shifts: Option<Vec<QueueShiftDoc>>,
    /// Servers: each unit fails after a time to failure and is repaired; `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<BreakdownSpec>")]
    pub breakdown: Option<Option<BreakdownSpec>>,
    /// Servers: the time each part takes to get here from the element that sends it (it takes
    /// its place in the buffer as the transfer starts and joins the queue as it ends; a sending
    /// server keeps its unit, blocked, through the transfer); `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<DistSpec>")]
    pub transfer: Option<Option<DistSpec>>,
    /// Servers: whether a sending server keeps its unit, blocked, through the transfer (true),
    /// or is free at once, the transfer being a minimum time in the buffer (false). Needs a
    /// transfer.
    #[serde(default)]
    pub transfer_holds_sender: Option<bool>,
    /// Servers: whether the units are available (on shift) outside the calendars of
    /// `availability`.
    #[serde(default)]
    pub available: Option<bool>,
    /// Servers: replaces the availability while each calendar is active (the first active one
    /// applying). A unit checks it as it takes its next entity; failures and repairs go on
    /// through the breaks.
    #[serde(default)]
    pub availability: Option<Vec<AvailabilityDoc>>,
    /// Servers: the power each unit draws in each state, in kilowatts (processing and idle;
    /// blocked, failed and off_shift default to idle); `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<PowerSpec>")]
    pub power: Option<Option<PowerSpec>>,
    /// Servers: a setup before every job, or with `on_change_of` a changeover when that
    /// property's value changes; `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<SetupSpec>")]
    pub setup: Option<Option<SetupSpec>>,
    /// Servers: the planned time to make one item, for the OEE results (its mean is the plan;
    /// not simulated); `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<DistSpec>")]
    pub planned_run_time: Option<Option<DistSpec>>,
    /// Servers: how long an entity waits before it gives up, and whether it gives up only
    /// before its service starts (`until`: `start`, the default) or also before it is done
    /// (`done`); `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<PatienceSpec>")]
    pub patience: Option<Option<PatienceSpec>>,
    /// Servers: whether an arriving entity may not join the queue (balking): by a table of the
    /// chance of joining from each number found (`type`: `table`, with `join` rows of
    /// `waiting` and `probability`), or 1/(n + 1) (`reciprocal`); n counts those waiting, or
    /// with `counts_in_service` also those in service; `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<BalkingSpec>")]
    pub balking: Option<Option<BalkingSpec>>,
    /// Servers: each unit is checked every `interval` and taken out for `restart` when more
    /// entities wait than `limit` per working unit, keeping at least `keep` working; a job
    /// under way on a unit taken out is retried at once (`on_takeout`: `retry`, the default)
    /// or resumes after the restart (`resume`); with a finite buffer, a unit out keeps a place
    /// of it for itself (`place_while_out`: `reserved`, the default) or holds none (`open`);
    /// `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<HealthCheckSpec>")]
    pub health_check: Option<Option<HealthCheckSpec>>,
    /// Servers: replaces the units while each calendar is active (a shift, the first active one
    /// applying); `capacity` applies outside them. `[]` removes them.
    #[serde(default)]
    pub shifts: Option<Vec<ShiftDoc>>,
    /// Servers: what happens to a job under way on a unit that goes off shift: `finish` (it
    /// finishes first), `retry` (back to the head of the queue at once, its work wasted) or
    /// `resume` (it waits on the unit until the unit is on shift again).
    #[serde(default)]
    pub on_shift_end: Option<OnShiftEndSpec>,
    /// Sinks: what reaches it is scrap, counted against `charged_to`, a server, or else the
    /// last server each entity finished at; `null`: it is not.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<ScrapSpec>")]
    pub scrap: Option<Option<ScrapSpec>>,
    /// Sinks: the customer demand it serves, for takt; `null` removes it.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<DemandSpec>")]
    pub demand: Option<Option<DemandSpec>>,
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
    /// Exclusive gateways: how the way is chosen, `standard` (conditions, else probabilities)
    /// or `shortest_queue` (the way with the shortest queue that has room, the sender waiting
    /// while none has).
    #[serde(default)]
    pub routing: Option<RoutingSpec>,
    /// A batch: how many entities it collects before sending them on together.
    #[serde(default)]
    pub size: Option<u32>,
    /// Conveyors: the length, in the experiment's `length_unit`; above 0.
    #[serde(default)]
    pub length: Option<f64>,
    /// Conveyors: the distance an entity travels in one model time unit; above 0.
    #[serde(default)]
    pub speed: Option<f64>,
    /// Conveyors: whether entities stop behind a stopped one ahead while the belt keeps moving
    /// (true), or the whole belt stops (false).
    #[serde(default)]
    pub accumulating: Option<bool>,
    /// Conveyors: the length of belt an entity takes; above 0.
    #[serde(default)]
    pub item_length: Option<f64>,
    /// Conveyors: the least free belt between one entity and the next; 0 or more.
    #[serde(default)]
    pub gap: Option<f64>,
    /// Conveyors: the drawn shape (`straight`, `curve`, or `spiral`).
    #[serde(default)]
    pub shape: Option<ConveyorShapeSpec>,
    /// Transports: the id of the mobile pool whose units carry the items.
    #[serde(default)]
    pub pool: Option<String>,
    /// Transports: the time to load an item at the pickup.
    #[serde(default)]
    pub load: Option<DistSpec>,
    /// Transports: the time to unload an item at the drop-off.
    #[serde(default)]
    pub unload: Option<DistSpec>,
    /// Servers, shared tasks, delays and batches: how many entities are inside as each run
    /// starts, instead of an empty start. They join the queue at time 0 in order: as many as
    /// there are free units start a fresh service at once, the rest wait (with a finite waiting
    /// room, at most the units plus the room). `null` or 0: it starts empty.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<u32>")]
    pub starting: Option<Option<u32>>,
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
    /// Makes the units mobile, for `transport` blocks: speeds, the home element and what a
    /// unit does after a job.
    #[serde(default)]
    pub mobility: Option<MobilityDoc>,
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
    /// The pool's mobility (see `resource.add`); `null` makes its units stay where they are.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<MobilityDoc>")]
    pub mobility: Option<Option<MobilityDoc>>,
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
    /// Whether entities taking it are rework, counted against the last server they finished
    /// at, for the OEE quality results.
    #[serde(default)]
    pub rework: Option<bool>,
    /// Out of a server with a patience: whether this is the way entities that give up take,
    /// besides its ordinary way out.
    #[serde(default)]
    pub abandoned: Option<bool>,
    /// Out of a server with a finite waiting room: whether this is the way entities finding no
    /// room take at once, besides its ordinary way out.
    #[serde(default)]
    pub turned_away: Option<bool>,
    /// Out of a server with balking: whether this is the way entities not joining its queue
    /// take at once, besides its ordinary way out.
    #[serde(default)]
    pub balked: Option<bool>,
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
    /// The unit lengths (a conveyor's) are in. Changing it does not convert lengths already in
    /// the model.
    #[serde(default)]
    pub length_unit: Option<LengthUnitSpec>,
    /// Length units per drawing unit of the canvas, above 0: a distance between two elements
    /// (for a transport) is the straight line between their centres, times this.
    #[serde(default)]
    pub layout_scale: Option<f64>,
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
    /// Sequential replications: run `replications` first, then add more until every key
    /// measure (or the one `measure`) is precise enough, or `max` is reached; `null` runs a
    /// fixed number again.
    #[serde(default, deserialize_with = "some")]
    pub sequential: Option<Option<wire::SequentialSpec>>,
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
    /// Record results over time (`series`) and suggest a warm-up from them
    /// (`warmup_suggestion`, and the `no_warmup` warning); default true. Recording changes no
    /// result; it costs a few per cent of run time on larger models, and the results grow.
    #[serde(default)]
    pub series: Option<bool>,
    /// Sequential replications for this run only (as `experiment.set` takes them): `null` runs
    /// exactly `replications`; by default the experiment's own setting. A sequential run's
    /// results say how many replications ran and why it stopped (`sequential`).
    #[serde(default, deserialize_with = "some")]
    pub sequential: Option<Option<wire::SequentialSpec>>,
    /// Run although the file had errors StochCraft could not read: the model then has what
    /// StochCraft substituted for each, which the run record lists (`approximations`). Without
    /// this, a file read with errors does not run.
    #[serde(default)]
    pub allow_approximations: bool,
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
    /// to the caller's own workers (see `Engine::worker_task`). A sequential run on workers
    /// sets out to have more replications after each look: `run.status` then gives a larger
    /// `total`, and the caller hands those out too.
    #[serde(default)]
    pub mode: Option<RunMode>,
    /// Record results over time (`series`) and suggest a warm-up from them
    /// (`warmup_suggestion`, and the `no_warmup` warning); default true. Recording changes no
    /// result; it costs a few per cent of run time on larger models, and the results grow.
    #[serde(default)]
    pub series: Option<bool>,
    /// Sequential replications for this run only, as for `run`.
    #[serde(default, deserialize_with = "some")]
    pub sequential: Option<Option<wire::SequentialSpec>>,
    /// As for `run`: accept what StochCraft substituted for errors in the file.
    #[serde(default)]
    pub allow_approximations: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStatus {
    /// For a run in slices: the most replications to finish before reporting (default 1).
    #[serde(default)]
    pub step: Option<u64>,
    /// For a run in slices: the most events to run before reporting (default 5,000, a slice
    /// the browser runs between frames); a replication not finished then carries on at the next
    /// call, with exactly the same results.
    #[serde(default)]
    pub events: Option<u64>,
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
    /// Replace the file even though it holds what StochCraft does not keep (a second diagram, a
    /// note the reader made about it): without this such a save is refused and says what it
    /// would leave out.
    #[serde(default)]
    pub allow_loss: bool,
}

// ---------------------------------------------------------------------------------------------
// Implementations

fn model_get(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    Ok(wire::model(e.model()))
}

fn model_validate(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    Ok(json!({ "diagnostics": crate::diagnostics(e.model()) }))
}

fn model_capacity(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: CapacityParams = parse(p)?;
    let target = p.target.unwrap_or(capacity::DEFAULT_TARGET);
    if !(target > 0.0 && target < 1.0) {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!("the target must be between 0 and 1, not {target}"),
        ));
    }
    Ok(wire::capacity(e.model(), &e.model().capacity(target)))
}

fn line_balance(_: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: BalanceParams = parse(p)?;
    let mut elements = Vec::with_capacity(p.elements.len());
    for e in p.elements {
        if e.id.trim().is_empty() {
            return Err(CommandError::new(
                ErrorCode::InvalidParams,
                "a work element's `id` must not be empty",
            ));
        }
        elements.push(balance::WorkElement {
            id: ElementId::unchecked(&e.id),
            time: e.time,
            predecessors: e.after.iter().map(|a| ElementId::unchecked(a)).collect(),
        });
    }
    let mut result = balance::compare(&elements, p.cycle)
        .map_err(|err| CommandError::new(ErrorCode::InvalidParams, err.to_string()))?;
    if let Some(h) = p.heuristic {
        let wanted: balance::Heuristic = h.into();
        result.balances.retain(|b| b.heuristic == wanted);
    }
    Ok(wire::line_balance(&result))
}

fn model_new(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: ModelNew = parse(p)?;
    let start = JournalStart::new(StartKind::New, Some(p.name.clone()));
    e.new_document(
        Model {
            name: p.name,
            ..Model::default()
        },
        start,
    );
    Ok(json!({}))
}

/// Refuses `transfer_holds_sender` set to false on a server with no transfer, which would
/// otherwise be dropped without a word (as `block.set` refuses it).
fn check_transfer(kind: &KindSpec) -> Result<(), CommandError> {
    if matches!(
        kind,
        KindSpec::Server {
            transfer: None,
            transfer_holds_sender: false,
            ..
        }
    ) {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            "`transfer_holds_sender` needs a transfer: give `transfer` too",
        ));
    }
    Ok(())
}

fn model_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    if let Some(elements) = p
        .get("model")
        .and_then(|m| m.get("elements"))
        .and_then(Value::as_array)
    {
        for el in elements {
            check_keys(
                el,
                &["id", "name", "properties", "starting", "bounds", "estimate"],
            )?;
        }
    }
    let p: ModelSet = parse(p)?;
    for el in &p.model.elements {
        check_transfer(&el.kind)?;
        // An estimate the model could not keep is refused, not dropped on the way in.
        if let Some(estimate) = &el.estimate
            && let Err(why) = estimate.to_estimate()
        {
            return Err(CommandError::new(
                ErrorCode::InvalidParams,
                format!("the estimate of `{}`: {why}", el.id),
            ));
        }
    }
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
    check_transfer(&p.kind)?;
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
    // Records an optimisation in the batch adds are taken back with it if a step fails.
    let records = e.sensitivity.len();
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
                        e.revision += 1;
                        if let Undo::Model(op) = entry.op {
                            let _ = crate::edit::apply(&mut e.model, op);
                        }
                    }
                }
                e.redo = redo;
                e.sensitivity.truncate(records);
                return Err(CommandError {
                    message: format!("step {} (`{}`): {}", i + 1, step.command, err.message),
                    ..err
                });
            }
        }
    }
    let ops: Vec<Op> = e
        .undo
        .drain(start..)
        .rev()
        .filter_map(|h| match h.op {
            Undo::Model(op) => Some(op),
            Undo::Sd(_) => None,
        })
        .collect();
    if !ops.is_empty() {
        e.undo.push(HistoryEntry {
            label,
            op: Undo::Model(Op::Batch(ops)),
        });
    }
    Ok(json!({ "results": results }))
}

/// The edits that clear an exclusive gateway's `default` where it is one of `gone` (connections
/// about to be removed), so no gateway keeps pointing at a flow that no longer exists; applied
/// first, each is undone last. `keep` is a gateway that goes with them, which needs none.
fn clear_dangling_defaults(model: &Model, gone: &[ElementId], keep: Option<&ElementId>) -> Vec<Op> {
    model
        .elements
        .iter()
        .filter(|el| Some(&el.id) != keep)
        .filter_map(|el| match &el.kind {
            ElementKind::ExclusiveGateway {
                default: Some(d),
                routing,
            } if gone.contains(d) => Some(Op::SetKind(
                el.id.clone(),
                ElementKind::ExclusiveGateway {
                    default: None,
                    routing: *routing,
                },
            )),
            _ => None,
        })
        .collect()
}

fn block_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Id = parse(p)?;
    let id = existing_element(e.model(), &p.id)?;
    // Its estimate and starting contents go with it, and a stop condition watching it.
    let gone: Vec<ElementId> = e
        .model()
        .connections
        .iter()
        .filter(|c| c.from == id || c.to == id)
        .map(|c| c.id.clone())
        .collect();
    let mut ops = clear_dangling_defaults(e.model(), &gone, Some(&id));
    if e.model().estimates.contains_key(&id) {
        ops.push(Op::SetEstimate(id.clone(), None));
    }
    if e.model().starting.contains_key(&id) {
        ops.push(Op::SetStarting(id.clone(), None));
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
    let mut kind = e.model().element(&id).map_or(
        ElementKind::Sink {
            scrap: None,
            demand: None,
        },
        |el| el.kind.clone(),
    );
    match &mut kind {
        ElementKind::Source {
            interarrival: d, ..
        }
        | ElementKind::Server { service: d, .. }
        | ElementKind::SharedTask { service: d, .. }
        | ElementKind::Delay { duration: d, .. } => *d = fitted.clone(),
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

fn estimate_fit(_: &mut Engine, p: Value) -> Result<Value, CommandError> {
    crate::fit::run(&parse(p)?)
}

fn experiment_sweep(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::decide::SweepParams = parse(p)?;
    let approximations = e.approximations(p.allow_approximations)?;
    let mut v = crate::decide::sweep(e.model(), &p)?;
    note_approximations(&mut v, approximations);
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
    let approximations = e.approximations(p.allow_approximations)?;
    let mut v = crate::decide::sensitivity(e.model(), &p)?;
    note_approximations(&mut v, approximations);
    e.sensitivity.push(json!({
        "kind": "one at a time",
        "model_digest": crate::model_digest(e.model()),
        "change": v["change"],
        "effects": v["effects"],
    }));
    Ok(v)
}

fn experiment_design(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::design::DesignParams = parse(p)?;
    let approximations = e.approximations(p.allow_approximations)?;
    let mut v = crate::design::design(e.model(), &p)?;
    note_approximations(&mut v, approximations);
    let mut record = json!({
        "kind": "design",
        "model_digest": crate::model_digest(e.model()),
    });
    // What reproduces the design, and its findings: only the keys this kind of design has.
    for key in [
        "design",
        "factors",
        "replications",
        "seed",
        "main_effects",
        "interactions",
        "correlations",
        "batches",
        "points_per_batch",
        "result_means",
    ] {
        if let Some(x) = v.get(key) {
            record[key] = x.clone();
        }
    }
    if v["design"] == "latin_hypercube" || v["design"] == "sobol" {
        record["points"] = json!(v["points"].as_array().map_or(0, Vec::len));
    }
    e.sensitivity.push(record);
    Ok(v)
}

/// Evaluates given values of the decisions for a range of replications. Not recorded in later
/// runs' records, unlike a sweep or a design: an outside optimiser evaluates thousands of
/// candidates, and the record would grow with each.
fn experiment_evaluate(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::evaluate::EvaluateParams = parse(p)?;
    let approximations = e.approximations(p.allow_approximations)?;
    let mut v = crate::evaluate::evaluate(e.model(), &p)?;
    note_approximations(&mut v, approximations);
    Ok(v)
}

/// An answer that came from a run of a file read with errors says so: the substitutions the run
/// made, as the run record's `approximations`.
fn note_approximations(answer: &mut Value, approximations: Vec<Value>) {
    if !approximations.is_empty() {
        answer["approximations"] = Value::Array(approximations);
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DecisionsSet {
    /// Each decision and its value, as `experiment.evaluate` takes them (`{id, field, value}`).
    decisions: Vec<crate::evaluate::Decision>,
}

/// Sets decisions' values as one undo step: what `experiment.optimise`'s `apply` does, and the
/// step its journal records for it, so an edits-only replay makes the same change.
fn decisions_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: DecisionsSet = parse(p)?;
    let (m, decisions) = crate::evaluate::decided(e.model(), &p.decisions)?;
    let changed = &m != e.model();
    if changed {
        e.edit("decisions.set", Op::SetModel(Box::new(m)))?;
    }
    Ok(json!({ "decisions": decisions, "changed": changed }))
}

/// Searches the decisions (counts, rules and means) for the best design (M8, stages O2 and O4)
/// and cleans up the finalists; `apply` makes the chosen design the model's, as one undo step, when the answer is
/// proven or compared exactly. Recorded in later runs' records with the model's digest.
fn experiment_optimise(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::optimize::OptimiseParams = parse(p)?;
    let approximations = e.approximations(p.allow_approximations)?;
    let (mut v, chosen) = crate::optimize::optimise(e.model(), &p)?;
    note_approximations(&mut v, approximations);
    let mut record = json!({
        "kind": "optimisation",
        "model_digest": crate::model_digest(e.model()),
        "decisions": crate::optimize::recorded(&p),
        "cost_cap": p.cost_cap,
        "constraints": v["constraints"],
        "indifference": p.indifference,
    });
    for key in [
        "outcome",
        "confidence",
        "chosen",
        "designs_compared",
        "designs_tried",
        "all_designs",
        "runs",
        "seed",
    ] {
        if let Some(x) = v.get(key) {
            record[key] = x.clone();
        }
    }
    e.sensitivity.push(record);
    if p.apply {
        match chosen {
            None => {
                v["applied"] = json!(false);
                v["not_applied"] = json!("the answer is not proven, so the model is unchanged");
            }
            Some(m) => {
                if &m != e.model() {
                    e.edit("experiment.optimise", Op::SetModel(Box::new(m)))?;
                }
                v["applied"] = json!(true);
            }
        }
    }
    Ok(v)
}

fn experiment_size(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: crate::decide::SizeParams = parse(p)?;
    let approximations = e.approximations(p.allow_approximations)?;
    let (mut v, sized) = crate::decide::size(e.model(), &p)?;
    note_approximations(&mut v, approximations);
    if p.apply {
        // The last size tried is the answer only if it met the target share: otherwise the
        // search ran out of units to add, and the model keeps its own sizes unless asked.
        if v["stochastic"]["reached"] != true && !p.apply_unreached {
            v["applied"] = json!(false);
            v["not_applied"] = json!(
                "the target share was not reached, so the model is unchanged; `apply_unreached` applies the last size tried anyway"
            );
        } else if &sized != e.model() {
            // The change as decisions, which the journal keeps as a `decisions.set` edit.
            v["applied_decisions"] = json!(crate::decide::changed_units(e.model(), &sized));
            e.edit("experiment.size", Op::SetModel(Box::new(sized)))?;
            v["applied"] = json!(true);
        }
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
    let mut kind = e.model().element(&id).map_or(
        ElementKind::Sink {
            scrap: None,
            demand: None,
        },
        |el| el.kind.clone(),
    );
    let given = [
        ("interarrival", p.interarrival.is_some()),
        ("limit", p.limit.is_some()),
        ("timed", p.timed.is_some()),
        ("idle_outside", p.idle_outside.is_some()),
        ("capacity", p.capacity.is_some()),
        ("service", p.service.is_some()),
        ("processing_by", p.processing_by.is_some()),
        ("processing_by_resource", p.processing_by_resource.is_some()),
        ("queue_capacity", p.queue_capacity.is_some()),
        (
            "queue_per_working_server",
            p.queue_per_working_server.is_some(),
        ),
        ("queue_shifts", p.queue_shifts.is_some()),
        ("breakdown", p.breakdown.is_some()),
        ("transfer", p.transfer.is_some()),
        ("transfer_holds_sender", p.transfer_holds_sender.is_some()),
        ("available", p.available.is_some()),
        ("availability", p.availability.is_some()),
        ("power", p.power.is_some()),
        ("setup", p.setup.is_some()),
        ("planned_run_time", p.planned_run_time.is_some()),
        ("patience", p.patience.is_some()),
        ("balking", p.balking.is_some()),
        ("health_check", p.health_check.is_some()),
        ("shifts", p.shifts.is_some()),
        ("on_shift_end", p.on_shift_end.is_some()),
        ("scrap", p.scrap.is_some()),
        ("demand", p.demand.is_some()),
        ("needs", p.needs.is_some()),
        ("priority", p.priority.is_some()),
        ("duration", p.duration.is_some()),
        ("default", p.default.is_some()),
        ("routing", p.routing.is_some()),
        ("size", p.size.is_some()),
        ("length", p.length.is_some()),
        ("speed", p.speed.is_some()),
        ("accumulating", p.accumulating.is_some()),
        ("item_length", p.item_length.is_some()),
        ("gap", p.gap.is_some()),
        ("shape", p.shape.is_some()),
        ("pool", p.pool.is_some()),
        ("load", p.load.is_some()),
        ("unload", p.unload.is_some()),
        ("starting", p.starting.is_some()),
    ];
    let applies: &[&str] = match kind {
        ElementKind::Source { .. } => &["interarrival", "limit", "timed", "idle_outside"],
        ElementKind::Server { .. } => &[
            "capacity",
            "service",
            "processing_by",
            "queue_capacity",
            "queue_per_working_server",
            "queue_shifts",
            "breakdown",
            "transfer",
            "transfer_holds_sender",
            "available",
            "availability",
            "power",
            "setup",
            "planned_run_time",
            "patience",
            "balking",
            "health_check",
            "shifts",
            "on_shift_end",
            "starting",
        ],
        ElementKind::SharedTask { .. } => &[
            "service",
            "processing_by",
            "processing_by_resource",
            "needs",
            "priority",
            "starting",
        ],
        ElementKind::Delay { .. } => &["duration", "processing_by", "starting"],
        ElementKind::ExclusiveGateway { .. } => &["default", "routing"],
        ElementKind::Batch { .. } => &["size", "starting"],
        ElementKind::Sink { .. } => &["scrap", "demand"],
        ElementKind::Conveyor { .. } => &[
            "length",
            "speed",
            "accumulating",
            "item_length",
            "gap",
            "shape",
        ],
        ElementKind::Transport { .. } => &["pool", "load", "unload"],
        ElementKind::ParallelGateway | ElementKind::Unbatch => &[],
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
    if let Some(x) = p.processing_by
        && let Some(by) = kind.processing_by_mut()
    {
        *by = wire::processing_by(x);
    }
    if let Some(x) = p.processing_by_resource
        && let Some(by) = kind.processing_by_resource_mut()
    {
        *by = wire::processing_by_resource(x);
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
            queue_limit,
            breakdown,
            transfer,
            available,
            availability,
            power,
            setup,
            planned_run_time,
            patience,
            balking,
            health_check,
            shifts,
            on_shift_end,
            ..
        } => {
            if let Some(s) = p.shifts {
                *shifts = s.into_iter().map(Into::into).collect();
            }
            if let Some(o) = p.on_shift_end {
                *on_shift_end = o.into();
            }
            if let Some(x) = p.patience {
                *patience = x.map(Into::into);
            }
            if let Some(x) = p.balking {
                *balking = x.map(Into::into);
            }
            if let Some(x) = p.health_check {
                *health_check = x.map(Into::into);
            }
            if let Some(s) = p.setup {
                *setup = s.map(Into::into);
            }
            if let Some(x) = p.planned_run_time {
                *planned_run_time = x.map(Into::into);
            }
            if let Some(b) = p.breakdown {
                *breakdown = b.map(Into::into);
            }
            if let Some(t) = p.transfer {
                // A new time keeps whether the transfer held the sender.
                let holds = transfer.as_ref().is_none_or(|x| x.holds_sender);
                *transfer = t.map(|d| Transfer {
                    time: d.into(),
                    holds_sender: holds,
                });
            }
            if let Some(h) = p.transfer_holds_sender {
                match transfer {
                    Some(t) => t.holds_sender = h,
                    None => {
                        return Err(CommandError::new(
                            ErrorCode::InvalidParams,
                            "`transfer_holds_sender` needs a transfer: give `transfer` too",
                        ));
                    }
                }
            }
            if let Some(a) = p.available {
                *available = a;
            }
            if let Some(a) = p.availability {
                *availability = a.into_iter().map(Into::into).collect();
            }
            if let Some(x) = p.power {
                *power = x.map(Into::into);
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
            if p.queue_per_working_server == Some(true) && p.queue_shifts.is_some() {
                return Err(CommandError::new(
                    ErrorCode::InvalidParams,
                    "a waiting room is either per working server or by calendar: give `queue_per_working_server` or `queue_shifts`, not both",
                ));
            }
            match (p.queue_per_working_server, p.queue_shifts) {
                (Some(true), _) => *queue_limit = QueueLimit::PerWorkingServer,
                (_, Some(s)) => *queue_limit = wire::queue_limit(false, s),
                (Some(false), None) => {
                    if matches!(queue_limit, QueueLimit::PerWorkingServer) {
                        *queue_limit = QueueLimit::Fixed;
                    }
                }
                (None, None) => {}
            }
            // An unlimited room has no size to set: fixed again if it is ever finite.
            if queue_capacity.is_none() {
                *queue_limit = QueueLimit::Fixed;
            }
        }
        ElementKind::SharedTask {
            service,
            needs,
            priority,
            ..
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
        ElementKind::Delay { duration, .. } => {
            if let Some(d) = p.duration {
                *duration = d.into();
            }
        }
        ElementKind::ExclusiveGateway { default, routing } => {
            if let Some(d) = p.default {
                *default = d.as_deref().map(ElementId::unchecked);
            }
            if let Some(r) = p.routing {
                *routing = r.into();
            }
        }
        ElementKind::Batch { size } => {
            if let Some(n) = p.size {
                *size = n;
            }
        }
        ElementKind::Sink { scrap, demand } => {
            if let Some(d) = p.demand {
                *demand = d.map(Into::into);
            }
            if let Some(s) = p.scrap {
                *scrap = s.map(|s| stochcraft_model::Scrap {
                    charged_to: s.charged_to.as_deref().map(ElementId::unchecked),
                });
            }
        }
        ElementKind::Conveyor {
            length,
            speed,
            accumulating,
            item_length,
            gap,
            shape,
        } => {
            if let Some(x) = p.length {
                *length = x;
            }
            if let Some(x) = p.speed {
                *speed = x;
            }
            if let Some(x) = p.accumulating {
                *accumulating = x;
            }
            if let Some(x) = p.item_length {
                *item_length = x;
            }
            if let Some(x) = p.gap {
                *gap = x;
            }
            if let Some(x) = p.shape {
                *shape = x.into();
            }
        }
        ElementKind::Transport { pool, load, unload } => {
            if let Some(x) = p.pool {
                *pool = ElementId::unchecked(&x);
            }
            if let Some(d) = p.load {
                *load = d.into();
            }
            if let Some(d) = p.unload {
                *unload = d.into();
            }
        }
        ElementKind::ParallelGateway | ElementKind::Unbatch => {}
    }
    // Starting contents are kept beside the element (`Model::starting`), so changing them with
    // other parameters is two edits, undone as one.
    let op = match p.starting {
        None => Op::SetKind(id, kind),
        Some(n) => {
            let starting = Op::SetStarting(id.clone(), n.filter(|n| *n > 0));
            if given.iter().any(|(name, g)| *g && *name != "starting") {
                Op::Batch(vec![Op::SetKind(id, kind), starting])
            } else {
                starting
            }
        }
    };
    e.edit("block.set", op)?;
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
    let mut pool = ResourcePool::new(
        id.clone(),
        p.name.unwrap_or_else(|| id.to_string()),
        p.quantity,
        p.roles,
    );
    pool.mobility = p.mobility.map(Into::into);
    pools.push(pool);
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
        && p.mobility.is_none()
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
        if let Some(m) = p.mobility {
            r.mobility = m.map(Into::into);
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
        .filter(|el| match &el.kind {
            ElementKind::Source { timed, .. } => timed.iter().any(|t| t.calendar.as_str() == p.id),
            ElementKind::Server {
                availability,
                shifts,
                queue_limit,
                ..
            } => {
                availability.iter().any(|t| t.calendar.as_str() == p.id)
                    || shifts.iter().any(|t| t.calendar.as_str() == p.id)
                    || queue_limit.calendars().any(|c| c.as_str() == p.id)
            }
            ElementKind::Sink {
                demand: Some(d), ..
            } => d.calendar.as_ref().is_some_and(|c| c.as_str() == p.id),
            _ => false,
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
            ElementKind::Transport { pool, .. } => pool.as_str() == p.id,
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
                rework: false,
                abandoned: false,
                turned_away: false,
                balked: false,
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
    if p.probability.is_none()
        && p.condition.is_none()
        && p.quantity.is_none()
        && p.rework.is_none()
        && p.abandoned.is_none()
        && p.turned_away.is_none()
        && p.balked.is_none()
    {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            "nothing to set: give `probability`, `condition`, `quantity`, `rework`, `abandoned`, `turned_away` or `balked`",
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
        ops.push(Op::SetQuantity(id.clone(), q));
    }
    if let Some(r) = p.rework {
        ops.push(Op::SetRework(id.clone(), r));
    }
    if let Some(a) = p.abandoned {
        ops.push(Op::SetAbandoned(id.clone(), a));
    }
    if let Some(t) = p.turned_away {
        ops.push(Op::SetTurnedAway(id.clone(), t));
    }
    if let Some(b) = p.balked {
        ops.push(Op::SetBalked(id, b));
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
    // A gateway whose default flow this is loses the default (undo gives it back).
    let mut ops = clear_dangling_defaults(e.model(), std::slice::from_ref(&id), None);
    ops.push(Op::Disconnect(id.clone()));
    let op = if ops.len() == 1 {
        ops.remove(0)
    } else {
        Op::Batch(ops)
    };
    e.edit("disconnect", op)?;
    Ok(json!({ "id": id.as_str() }))
}

fn experiment_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: ExperimentSet = parse(p)?;
    let mut x = e.model().experiment.clone();
    if let Some(t) = p.time_unit {
        x.time_unit = t.into();
    }
    if let Some(u) = p.length_unit {
        x.length_unit = u.into();
    }
    x.layout_scale = p.layout_scale.unwrap_or(x.layout_scale);
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
    if let Some(q) = &p.sequential {
        x.sequential = sequential(e.model(), q.clone())?;
    }
    let given = [
        p.sequential.is_some(),
        p.stop.is_some(),
        p.start.is_some(),
        p.time_unit.is_some(),
        p.length_unit.is_some(),
        p.layout_scale.is_some(),
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

/// The index in the experiment's observed data of the one for `element`'s `metric`.
fn observation_index(model: &Model, element: &str, metric: &str) -> Option<usize> {
    model
        .experiment
        .observed
        .iter()
        .position(|o| o.element.as_str() == element && o.metric == metric)
}

fn observation_add(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: wire::ObservationSpec = parse(p)?;
    if observation_index(e.model(), p.element(), p.metric()).is_some() {
        return Err(CommandError::new(
            ErrorCode::Conflict,
            format!(
                "`{}` already has observed data for `{}`: change it with observation.set",
                p.element(),
                p.metric()
            ),
        ));
    }
    let observation = crate::observed::from_spec(e.model(), p)?;
    let mut x = e.model().experiment.clone();
    x.observed.push(observation);
    e.edit("observation.add", Op::SetExperiment(x))?;
    Ok(json!({}))
}

fn observation_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: wire::ObservationSpec = parse(p)?;
    let Some(i) = observation_index(e.model(), p.element(), p.metric()) else {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            format!(
                "`{}` has no observed data for `{}`: add it with observation.add",
                p.element(),
                p.metric()
            ),
        ));
    };
    let observation = crate::observed::from_spec(e.model(), p)?;
    let mut x = e.model().experiment.clone();
    x.observed[i] = observation;
    e.edit("observation.set", Op::SetExperiment(x))?;
    Ok(json!({}))
}

fn observation_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: ObservationRemove = parse(p)?;
    // Not checked against the model's elements: observed data for a block that was removed is
    // the very thing to remove.
    let Some(i) = observation_index(e.model(), &p.element, &p.metric) else {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            format!("`{}` has no observed data for `{}`", p.element, p.metric),
        ));
    };
    let mut x = e.model().experiment.clone();
    x.observed.remove(i);
    e.edit("observation.remove", Op::SetExperiment(x))?;
    Ok(json!({}))
}

/// The schema of an observation's parameters: its variants are objects, which is said at the top
/// as for every other command's parameters.
fn observation_schema() -> Value {
    let mut s = schema::<wire::ObservationSpec>();
    s["type"] = json!("object");
    s
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

/// Sequential replications as given to `experiment.set`, `run` or `run.start`, checked against
/// `model`: targets above 0, at least one replication, and a watched result on an element the
/// model has, by the name of a metric. `None` (`null`) for a fixed number of replications.
fn sequential(
    model: &Model,
    spec: Option<wire::SequentialSpec>,
) -> Result<Option<stochcraft_model::Sequential>, CommandError> {
    let Some(spec) = spec else {
        return Ok(None);
    };
    let bad = |what: String| CommandError::new(ErrorCode::InvalidParams, what);
    for (name, v) in [("relative", spec.relative), ("points", spec.points)] {
        if !(v.is_finite() && v > 0.0) {
            return Err(bad(format!("sequential `{name}` must be above 0, not {v}")));
        }
    }
    if spec.max == 0 {
        return Err(bad("sequential `max` must be at least 1".into()));
    }
    if let Some(m) = &spec.measure {
        existing_element(model, &m.element)?;
        if stochcraft_blocks::Metric::from_name(&m.metric).is_none() {
            return Err(bad(format!(
                "sequential `metric`: no metric named `{}`",
                m.metric
            )));
        }
        if let Some(h) = m.half_width
            && !(h.is_finite() && h > 0.0)
        {
            return Err(bad(format!(
                "sequential `half_width` must be above 0, not {h}"
            )));
        }
    }
    Ok(Some(spec.into()))
}

/// The sequential override of a `run` or `run.start`: absent keeps the experiment's.
fn sequential_override(
    model: &Model,
    given: Option<Option<wire::SequentialSpec>>,
) -> Result<Option<Option<stochcraft_model::Sequential>>, CommandError> {
    given.map(|q| sequential(model, q)).transpose()
}

fn run(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Run = parse(p)?;
    let sequential = sequential_override(e.model(), p.sequential)?;
    e.plan(
        p.replications,
        p.seed,
        p.threads,
        p.series.unwrap_or(true),
        sequential,
        p.allow_approximations,
    )?
    .run_now()
}

fn run_start(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: RunStart = parse(p)?;
    let sequential = sequential_override(e.model(), p.sequential)?;
    let plan = e.plan(
        p.replications,
        p.seed,
        p.threads,
        p.series.unwrap_or(true),
        sequential,
        p.allow_approximations,
    )?;
    e.start_run(plan, p.mode.unwrap_or_else(RunMode::default_for_target))
}

fn run_status(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: RunStatus = parse(p)?;
    Ok(e.poll_run(
        p.step.unwrap_or(1),
        p.events.unwrap_or(crate::run::SLICE_EVENTS),
    ))
}

fn run_cancel(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<NoParams>(p)?;
    Ok(e.cancel_run())
}

fn file_open(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: FileOpen = parse(p)?;
    let bytes = std::fs::read(&p.path)
        .map_err(|err| CommandError::new(ErrorCode::Io, format!("{}: {err}", p.path)))?;
    // An agent-based model's charts and lists of links are read from beside it.
    let dir = std::path::Path::new(&p.path)
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    e.open_bytes_with(&p.path, &bytes, p.scenario.as_deref(), &mut |href: &str| {
        let path = crate::agents::beside(&dir, href)?;
        std::fs::read(path).map_err(|err| err.to_string())
    })
}

/// A file to write beside the model: its reference, where, its bytes, and what was there when the
/// save looked (`None`: nothing).
type Sibling = (String, std::path::PathBuf, Vec<u8>, Option<Vec<u8>>);

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
    let loss = e.save_loss(&path, p.as_new);
    if !loss.is_empty() && !p.allow_loss {
        return Err(CommandError {
            code: ErrorCode::Conflict,
            message: format!(
                "saving into {path} would leave out what StochCraft does not keep of it: {}; `\"allow_loss\": true` saves anyway",
                loss.join("; ")
            ),
            details: Some(json!({ "would_lose": loss })),
        });
    }
    let bytes = e.save_bytes(p.as_new).map_err(|mut err| {
        if err.code == ErrorCode::Unsupported {
            err.message += "; `\"as_new\": true` writes the model as a new BPMN file instead, without the content StochCraft does not understand";
        }
        err
    })?;
    // An agent-based model's charts and lists of links go beside it, each written only where
    // there is no file of its name yet. One that is there and differs (another model's, or one
    // edited since the model was opened), or that cannot be read, is never replaced, and the
    // model is never saved over one of them: the save is refused before anything is written.
    let model_path = std::path::Path::new(&path);
    let dir = model_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    // The model's file as `beside` gives a companion's: in the folder with its links followed.
    let model_beside = std::fs::canonicalize(if dir.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        dir.as_path()
    })
    .ok()
    .zip(model_path.file_name())
    .map(|(d, n)| d.join(n));
    let model_is = |to: &std::path::Path| {
        model_beside.as_deref() == Some(to)
            || same_file::is_same_file(model_path, to).unwrap_or(false)
    };
    let model_over = |href: &str| {
        CommandError::new(
            ErrorCode::InvalidParams,
            format!(
                "{path} is the model's file `{href}`: the model cannot be saved over a file it reads; give another path"
            ),
        )
    };
    // Nor over a file the opened model reads where it was opened from, when the save is to
    // another folder.
    let opened_dir = e
        .path()
        .and_then(|p| std::path::Path::new(p).parent())
        .map(std::path::Path::to_path_buf);
    // The files the model reads beside it: an agent-based model's charts and lists of links,
    // and a process model's decision tables, each with the bytes it may replace (a `.dmn` file
    // as it was opened or last saved, when saving into that folder: edited tables are written
    // back into it). That folder is the one the tables were read from or written to, which a
    // save of the model's file alone into another folder does not change.
    let tables_dir = e
        .dmn_at
        .as_deref()
        .and_then(|p| std::path::Path::new(p).parent())
        .map(std::path::Path::to_path_buf);
    let same_dir = tables_dir.as_ref().is_some_and(|from| {
        let canon = |d: &std::path::Path| {
            std::fs::canonicalize(if d.as_os_str().is_empty() {
                std::path::Path::new(".")
            } else {
                d
            })
            .ok()
        };
        canon(from).is_some() && canon(from) == canon(&dir)
    });
    // A new table file without an id or namespace is written with generated ones: the model
    // takes them first, so it records what is written.
    e.dmn_as_written();
    let mut files: Vec<crate::dmn::Companion> = e
        .agent_files()
        .into_iter()
        .map(|(href, content)| (href, content, None))
        .collect();
    files.extend(
        e.dmn_files()?
            .into_iter()
            .map(|(href, content, was)| (href, content, was.filter(|_| same_dir))),
    );
    if let Some(from) = &opened_dir {
        for (href, _, _) in &files {
            if let Ok(there) = crate::agents::beside(from, href)
                && model_is(there.as_path())
            {
                return Err(model_over(href.as_str()));
            }
        }
    }
    let written: Vec<(String, Vec<u8>)> = e
        .dmn_files()?
        .into_iter()
        .map(|(href, content, _)| (href, content))
        .collect();
    // Two references can name one file: names that differ only in case, on a file system that
    // ignores case, or a link in the model's folder. Holding the same, it is written once (the
    // second recorded with the first); holding different contents, one would overwrite the
    // other, so the save is refused before anything is written.
    let mut placed: Vec<(String, std::path::PathBuf, Vec<u8>)> = Vec::new();
    let mut twins: Vec<String> = Vec::new();
    let mut unique = Vec::new();
    for (href, content, replaceable) in files {
        let to = crate::agents::beside(&dir, &href)
            .map_err(|m| CommandError::new(ErrorCode::InvalidParams, m))?;
        if let Some((first, _, held)) = placed.iter().find(|(_, t, _)| same_target(t, &to)) {
            if *held != content {
                return Err(CommandError::new(
                    ErrorCode::Conflict,
                    format!(
                        "`{first}` and `{href}` name one file, {}, but the model gives them different contents: make them one reference",
                        to.display()
                    ),
                ));
            }
            twins.push(href);
            continue;
        }
        placed.push((href.clone(), to, content.clone()));
        unique.push((href, content, replaceable));
    }
    let mut companions = Vec::new();
    // Each file to write, with what was there when the save looked (`None`: nothing).
    let mut siblings: Vec<Sibling> = Vec::new();
    // The `.dmn` files already as the model would write them.
    let mut same = Vec::new();
    for (href, content, replaceable) in unique {
        let to = crate::agents::beside(&dir, &href)
            .map_err(|m| CommandError::new(ErrorCode::InvalidParams, m))?;
        if model_is(to.as_path()) {
            return Err(model_over(href.as_str()));
        }
        match std::fs::read(&to) {
            Ok(there) if there == content => same.push(href.clone()),
            // A decision table file as it was read, unchanged on disk since: edited, it is
            // written back.
            Ok(there) if replaceable.as_deref() == Some(there.as_slice()) => {
                siblings.push((href.clone(), to.clone(), content, Some(there)));
            }
            Ok(_) => {
                return Err(CommandError::new(
                    ErrorCode::Conflict,
                    format!(
                        "{} is already there and differs from the model's `{href}`: save the model in another folder, or move that file first",
                        to.display()
                    ),
                ));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                siblings.push((href.clone(), to.clone(), content, None));
            }
            Err(err) => {
                return Err(CommandError::new(
                    ErrorCode::Io,
                    format!("{}: {err}", to.display()),
                ));
            }
        }
        companions.push((href, to));
    }
    // A `.dmn` file written into the folder the model was opened from is recorded as written at
    // once, so that a save failing after it (writing the model's own file, say) leaves the
    // record matching the disk: saving again, or after undoing the edit, then finds the file
    // as this engine left it rather than changed by another. (Written into another folder, it
    // describes no file the model reads, so the record waits for the whole save.)
    let record = |e: &mut Engine, href: &str| {
        if same_dir && let Some(w) = written.iter().find(|(h, _)| h == href) {
            e.dmn_saved(vec![w.clone()]);
        }
    };
    for href in &same {
        record(e, href.as_str());
    }
    // The model's files first, so one that cannot be written leaves the model's old file as it
    // was (and a save again keeps those already written, being the same). Each is written to a
    // temporary file beside it and renamed over it, so a crash or a full disk leaves the old file
    // whole rather than half written.
    for (href, to, content, seen) in siblings {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                CommandError::new(ErrorCode::Io, format!("{}: {err}", parent.display()))
            })?;
        }
        write_beside(&to, &content, seen.as_deref())?;
        record(e, href.as_str());
    }
    for href in &twins {
        record(e, href.as_str());
    }
    // Every one is there now, so a name that differs only in case on a file system that ignores
    // it is found too.
    if let Some((href, _)) = companions.iter().find(|(_, to)| model_is(to.as_path())) {
        return Err(model_over(href.as_str()));
    }
    craft_atomic::atomic_write(model_path, &bytes)
        .map_err(|err| CommandError::new(ErrorCode::Io, format!("{path}: {err}")))?;
    e.dmn_saved(written);
    let result = e.record_saved(&path, &bytes)?;
    // The tables are beside the model's file now.
    e.dmn_at = Some(path);
    e.dmn_away = false;
    Ok(result)
}

/// Whether `a` and `b`, two files beside the model, may be one file. Both there: whether they
/// are the same file (links followed). Only one there: not, as a name the file system takes for
/// the other would be there too. Neither there yet: whether they are the same path once their
/// nearest folders that are there are resolved, compared ignoring case, as a file system may.
fn same_target(a: &std::path::Path, b: &std::path::Path) -> bool {
    if a == b {
        return true;
    }
    let resolved = |p: &std::path::Path| {
        let mut existing = p;
        let mut rest = Vec::new();
        while !existing.exists() {
            match (existing.parent(), existing.file_name()) {
                (Some(parent), Some(name)) => {
                    rest.push(name.to_owned());
                    existing = parent;
                }
                _ => break,
            }
        }
        let mut out = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
        for name in rest.iter().rev() {
            out.push(name);
        }
        out.to_string_lossy().to_lowercase()
    };
    match (a.exists(), b.exists()) {
        (true, true) => {
            same_file::is_same_file(a, b).unwrap_or_else(|_| resolved(a) == resolved(b))
        }
        (false, false) => resolved(a) == resolved(b),
        _ => false,
    }
}

/// Writes `content` to `to`, a file beside the model, if it is still as the save found it
/// (`seen`; `None`: no file there): a file another program changed (or made) since the save
/// looked is never written over. Another program can still write it between this look and the
/// rename that puts `content` there; that window is the write's own.
pub(crate) fn write_beside(
    to: &std::path::Path,
    content: &[u8],
    seen: Option<&[u8]>,
) -> Result<(), CommandError> {
    let now = match std::fs::read(to) {
        Ok(b) => Some(b),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(CommandError::new(
                ErrorCode::Io,
                format!("{}: {err}", to.display()),
            ));
        }
    };
    if now.as_deref() != seen {
        return Err(CommandError::new(
            ErrorCode::Conflict,
            format!(
                "{} changed while the model was being saved: save again, or move that file first",
                to.display()
            ),
        ));
    }
    craft_atomic::atomic_write(to, content)
        .map_err(|err| CommandError::new(ErrorCode::Io, format!("{}: {err}", to.display())))
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
        name: "dmn.set",
        summary: "Set a decision table (DMN 1.6, D30): the decision `decision` of the .dmn file `file` beside the model, added (with the file) if it is not there yet; a decision cannot be added yet to a .dmn file the model read from disk, so put a new one in a new .dmn file. Every cell is FEEL text: input entries are S-FEEL unary tests (`< 10`, `[1..5]`, `\"A\", \"B\"`, `not(...)`, `-`), output entries a number, a string in double quotes, true or false; the hit policy is UNIQUE, FIRST, PRIORITY or COLLECT (with an optional aggregation). A table that cannot run is refused with the cell and the reason. Returns `findings`: rules that overlap under UNIQUE and inputs no rule covers, each with an example, or `unchecked` for a table with too many combinations of inputs to check in full.",
        kind: CommandKind::Edit,
        schema: schema::<crate::dmn::DmnSet>,
        example: || {
            json!({
                "file": "routing.dmn",
                "decision": "priority",
                "table": {
                    "hit_policy": "UNIQUE",
                    "outputs": [{ "name": "priority" }],
                    "rules": [{ "inputs": [], "outputs": ["1"] }]
                }
            })
        },
        run: crate::dmn::set,
    },
    Command {
        name: "dmn.use",
        summary: "Make a task or an exclusive gateway use a decision table the model has (`file` and `decision`), or, with neither, stop using one. As an entity enters, the table is evaluated on its properties and the outputs are written back as properties; a gateway none of whose ways out has a condition takes the way whose flow id, target id or target name is the output (`route` names the output when there are several).",
        kind: CommandKind::Edit,
        schema: schema::<crate::dmn::DmnUse>,
        example: || json!({ "element": "desk" }),
        run: crate::dmn::use_table,
    },
    Command {
        name: "dmn.get",
        summary: "The decision tables: which block uses which decision of which .dmn file, and every file's decisions with their tables, each cell as its FEEL text.",
        kind: CommandKind::Query,
        schema: schema::<crate::dmn::DmnGet>,
        example: || json!({}),
        run: crate::dmn::get,
    },
    Command {
        name: "model.capacity",
        summary: "The capacity table, without running: each server's, shared task's, delay's and pool's arrival rate, mean service time, units, utilisation and load (marked fine, congested above `target`, default 0.85, or unstable at 100%), the units that would bring it under the target, and its mean wait where a queueing formula holds exactly (M/M/c, M/M/c/K, Erlang A, balking, M/G/1), or why not; rates that depend on the run are not guessed. Rates and times in the model's time unit.",
        kind: CommandKind::Query,
        schema: schema::<CapacityParams>,
        example: || json!({ "target": 0.85 }),
        run: model_capacity,
    },
    Command {
        name: "line.balance",
        summary: "Balance assembly work elements across stations for a cycle time without opening a model, using the largest candidate rule, Kilbridge and Wester, and ranked positional weights (or one chosen heuristic), with the theoretical minimum, station times, balance efficiency, delay and smoothness.",
        kind: CommandKind::Query,
        schema: schema::<BalanceParams>,
        example: || {
            json!({
                "elements": [
                    { "id": "A", "time": 4.0 },
                    { "id": "B", "time": 4.0, "after": ["A"] },
                    { "id": "C", "time": 5.0 }
                ],
                "cycle": 10.0
            })
        },
        run: line_balance,
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
        name: "estimate.fit",
        summary: "Fit measured values (at least 20) to each distribution family by maximum likelihood, ranked by Anderson–Darling A² (smaller fits better) with the Kolmogorov–Smirnov distance beside it; families needing values above 0 are skipped with the reason. Changes nothing: apply a fit's `distribution` with block.set as the element's `interarrival`, `service` or `duration`.",
        kind: CommandKind::Query,
        schema: schema::<crate::fit::Params>,
        example: || json!({ "data": [1.9, 0.6, 2.4, 1.1, 0.8, 3.7, 1.5, 0.4, 1.2, 2.9, 0.9, 1.7, 0.7, 2.2, 1.3, 5.1, 1.0, 0.5, 1.6, 2.6, 1.4, 0.3, 1.8, 1.1] }),
        run: estimate_fit,
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
        summary: "Change the experiment: time unit, warm-up, duration, replications, seed, start, an event that ends each replication early, and sequential replications (more until the key measures are precise enough).",
        kind: CommandKind::Edit,
        schema: schema::<ExperimentSet>,
        example: || json!({ "replications": 5, "duration": 240.0 }),
        run: experiment_set,
    },
    Command {
        name: "observation.add",
        summary: "Add real data for a result to be checked against (black-box validation): an observed value (`check` `value`, with the half-width of its own 95% interval if known) or a bound (`at_most` or `at_least`), in the result's own unit (a share as a fraction). The results then say, for each, whether the run agrees: an observed value agrees when the 95% interval of the difference holds 0, and a bound passes or fails only when the whole simulated 95% interval does. One for each result (a block or pool id and a metric name).",
        kind: CommandKind::Edit,
        schema: observation_schema,
        example: || json!({ "check": "value", "element": "desk", "metric": "mean_wait", "value": 3.2, "half_width": 0.4 }),
        run: observation_add,
    },
    Command {
        name: "observation.set",
        summary: "Change the observed data for a result that has some (the same parameters as observation.add).",
        kind: CommandKind::Edit,
        schema: observation_schema,
        example: || json!({ "check": "at_most", "element": "desk", "metric": "mean_wait", "bound": 4.0 }),
        run: observation_set,
    },
    Command {
        name: "observation.remove",
        summary: "Remove the observed data for a result.",
        kind: CommandKind::Edit,
        schema: schema::<ObservationRemove>,
        example: || json!({ "element": "desk", "metric": "mean_wait" }),
        run: observation_remove,
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
        summary: "Run the experiment at each value of one parameter (a task's or pool's units, a buffer, a time's mean) with the same seed, and report the results asked for at each and how much they moved. Each value comes with the warnings its run gives, as a single run's without results over time (`warnings`).",
        kind: CommandKind::Action,
        schema: schema::<crate::decide::SweepParams>,
        example: || json!({ "parameter": { "id": "desk", "field": "capacity" }, "values": [1, 2, 3], "results": [{ "element": "desk", "metric": "mean_wait" }], "replications": 4 }),
        run: experiment_sweep,
    },
    Command {
        name: "experiment.sensitivity",
        summary: "Move each parameter down and up in turn and rank which results it drives (their elasticities); recorded in later runs' records. The runs that give warnings, as a single run's without results over time, are listed with them (`warnings`).",
        kind: CommandKind::Action,
        schema: schema::<crate::decide::SensitivityParams>,
        example: || json!({ "parameters": [{ "id": "desk", "field": "mean" }, { "id": "arrivals", "field": "mean" }], "results": [{ "element": "exit", "metric": "mean_time_in_system" }], "replications": 4 }),
        run: experiment_sensitivity,
    },
    Command {
        name: "experiment.design",
        summary: "Run a designed experiment with several factors at once: every combination of their levels (`full_factorial`, the default), a Latin hypercube of their ranges (`latin_hypercube`, `points`) or a randomised Sobol design (`sobol`: `batches` batches, default 4, of `points` points each, a power of two, each batch with its own random shift, so the batches are independent replicates), each point with the same seed; reports each result at each point, the factors' main effects and two-way interactions with paired intervals (or rank correlations for a Latin hypercube or Sobol design; a Sobol design adds each correlation's value per batch and each result's mean per batch, with 95% intervals across the batches). At most 10,000 runs (points × replications). Each point comes with the warnings its run gives, as a single run's without results over time (`warnings`).",
        kind: CommandKind::Action,
        schema: schema::<crate::design::DesignParams>,
        example: || json!({ "factors": [{ "id": "arrivals", "field": "mean", "levels": [2.0, 3.0] }, { "id": "desk", "field": "mean", "levels": [0.8, 1.2] }], "results": [{ "element": "desk", "metric": "mean_wait" }], "replications": 4 }),
        run: experiment_design,
    },
    Command {
        name: "experiment.evaluate",
        summary: "Evaluate given values of the decisions (each a parameter as a sweep takes it, `{id, field}`, with its `value` and optionally the `min` and `max` it must lie in) for replications `first` (counting from 0, default 0) to `first` + `count` − 1 (default: the experiment's replications), on the model's seed or the one given: each result's value in every replication, with their mean and 95% interval. Replication i depends only on the seed and i, so replications evaluated in stages give exactly what they give at once, and every evaluation meets the same random numbers (common random numbers). An unknown decision, a value of the wrong kind or out of its range, or a decision given twice is refused with the decision named. The model is unchanged; for an optimiser of your own driving the runs (the command line's `evaluate` reads and writes JSON lines). At most 10,000 replications at a time.",
        kind: CommandKind::Action,
        schema: schema::<crate::evaluate::EvaluateParams>,
        example: || json!({ "decisions": [{ "id": "desk", "field": "mean", "value": 1.2, "min": 0.5, "max": 2.0 }], "results": [{ "element": "desk", "metric": "mean_wait" }], "first": 0, "count": 4 }),
        run: experiment_evaluate,
    },
    Command {
        name: "decisions.set",
        summary: "Set decisions' values (each `{id, field, value}`, the field one of `capacity`, `quantity`, `queue_capacity` and `mean` with a number, or a rule, `routing`, `on_shift_end` or `after_job`, with its level's name, as `experiment.evaluate` takes them) as one undo step. An unknown decision, a value of the wrong kind or out of its range, or a decision given twice is refused with the decision named. `experiment.optimise` with `apply` journals its change as this step, so a replay of the edits makes it without searching again.",
        kind: CommandKind::Edit,
        schema: schema::<DecisionsSet>,
        example: || json!({ "decisions": [{ "id": "desk", "field": "capacity", "value": 1 }] }),
        run: decisions_set,
    },
    Command {
        name: "experiment.optimise",
        summary: "Find the best decisions (`decisions`, each `{id, field}` starting from the model's value or `start`: a count, `field` one of `capacity`, `quantity`, `queue_capacity`, from `min` to `max` in steps of `step`; a rule, `routing` of an exclusive gateway, `on_shift_end` of a task with shifts or `after_job` of a mobile pool, one of its `values` by name (default: `standard` and `shortest_queue` for a routing, every level for the others); or a `mean`, a number from `min` to `max`, the distribution rescaled with its shape kept). Rules are never moved by COMPASS: every combination of their levels (at most 8) is searched with the counts, and all meet in one clean-up. One or two means are searched by Nelder–Mead with Barton and Ivey's changes for noise, for each combination of the other decisions' values (at most 8), a local search that the answer says is local. The objective is Σ weight × result plus Σ cost × decision (`objective`: `results` each `{element, metric, weight}`, `sense` minimise or maximise, `costs` each `{id, field, cost}`); `cost_cap` caps the decisions' costs, and designs over it are never run; `constraints` keep results' means `at_most` or `at_least` a limit, each with a `tolerance`. The search is COMPASS from the starting design and a few Latin hypercube `starts`. A space is not searched, and every design is compared, when it has at most `all_designs_up_to` (50) designs, it can be counted (its ranges hold at most a million points), and the budget pays for the clean-up's first stage (10 replications) of every design. The finalists are compared again on fresh replications by Kim and Nelson's KN (after Batur and Kim's feasibility check with constraints), so the answer says, with `confidence` (default 0.95), that the chosen design is the best or within `indifference` (δ, required, in the objective's unit) of the best of the designs compared at the end, or of all designs when every one was compared; \"best found, not proven best\" when the budget (`max_runs`, default 10,000, of which `reserve`, 30%, is kept for the clean-up) runs out first, and \"compared exactly\" only for a model in which nothing is drawn, or among the designs that meet the constraints when the objective is the decisions' costs alone (equal values from a model that draws are never taken as exact: two designs with the same difference in every first-stage replication of the clean-up leave the answer not proven, and the claim says why). Beside it: the chosen design's objective with its interval, the paired difference against the starting design, each constraint's decision, the finalists, the search's trace and the runs used. `time_limit` (seconds) stops the search; its answer's `repeat` gives the `max_runs` and `reserve_runs` that repeat it exactly. `apply` makes the chosen design the model's, as one undo step, journalled as `decisions.set` so an edits-only replay repeats the change without the search.",
        kind: CommandKind::Edit,
        schema: schema::<crate::optimize::OptimiseParams>,
        example: || json!({ "decisions": [{ "id": "desk", "field": "capacity", "min": 1, "max": 3 }], "objective": { "results": [{ "element": "desk", "metric": "mean_wait", "weight": 1.0 }], "costs": [{ "id": "desk", "field": "capacity", "cost": 2.0 }] }, "indifference": 1.0, "max_runs": 300 }),
        run: experiment_optimise,
    },
    Command {
        name: "experiment.size",
        summary: "Add units at the bottleneck until the share of replications meeting a target reaches `target_share` (`confidence` is its old name), with the answer from mean times beside it; `apply` makes it the model's when the target share was reached (`apply_unreached` also applies the last size tried when it was not). A greedy heuristic: it need not find the fewest units. Each step comes with the warnings its run gives, as a single run's without results over time (`warnings`).",
        kind: CommandKind::Edit,
        schema: schema::<crate::decide::SizeParams>,
        example: || json!({ "target": { "element": "desk", "metric": "mean_wait", "at_most": 1.0 }, "target_share": 0.7, "replications": 4, "apply": true }),
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
        summary: "Save the model: into the file it was opened from, or as a new BPMN file with BPSim. A system dynamics model is written as an XMILE file, with its diagram.",
        kind: CommandKind::Action,
        schema: schema::<FileSave>,
        example: || json!({ "path": "help-desk.bpmn" }),
        run: file_save,
    },
    Command {
        name: "file.open",
        summary: "Open a BPMN or XPDL file with BPSim data, or an XMILE system dynamics model (run with sd.run), replacing the model, clearing the history and starting a new journal.",
        kind: CommandKind::Action,
        schema: schema::<FileOpen>,
        example: || json!({ "path": "help-desk.bpmn" }),
        run: file_open,
    },
    Command {
        name: "model.new",
        summary: "Start a new, empty model, clearing the history and starting a new journal.",
        kind: CommandKind::Action,
        schema: schema::<ModelNew>,
        example: || json!({ "name": "Warehouse" }),
        run: model_new,
    },
    Command {
        name: "mc.input.set",
        summary: "Monte Carlo: add an uncertain input (a distribution, with an optional unit) or change one; the first one makes the model a Monte Carlo model, a calculation with no process, with 10,000 trials.",
        kind: CommandKind::Edit,
        schema: schema::<crate::mc::InputSet>,
        example: || json!({ "name": "labour", "distribution": { "type": "triangular", "min": 80.0, "mode": 100.0, "max": 150.0 }, "unit": "h" }),
        run: crate::mc::input_set,
    },
    Command {
        name: "mc.inputs.import",
        summary: "Monte Carlo: inputs from a CSV table with the header `name,distribution,parameters,unit` (parameters as `mean=10;std_dev=2`); each replaces the input of its name, or with `replace` all of them.",
        kind: CommandKind::Edit,
        schema: schema::<crate::mc::InputsImport>,
        example: || {
            json!({ "csv": "name,distribution,parameters,unit
rate,uniform,min=40;max=50,USD/h
materials,normal,mean=500;std_dev=50,USD
" })
        },
        run: crate::mc::inputs_import,
    },
    Command {
        name: "mc.output.set",
        summary: "Monte Carlo: add an output (a BPSim expression over the inputs and earlier outputs, named as $name, with arithmetic, min, max, sum, abs, sqrt, exp, log, pow, round, sin, cos and if) or change one, with an optional unit and target (`at_most` or `at_least`) whose probability is reported. `controls` names control variates that narrow the interval of the output's mean and target probability: inputs (`{ \"input\": \"x\" }`, whose own mean is known) or earlier outputs with a mean you know (`{ \"output\": \"y\", \"expected\": 12.5 }`); the coefficient is fitted on a pilot that the controlled estimate leaves out: the first batch of 1,000 trials, or the first half of a random run under 2,000 trials (whole pairs for antithetic sampling); Latin hypercube, Sobol and correlated-input runs reserve the whole first batch, so one batch gives no controlled estimate (two batches are needed, three for its interval); an estimate holds for a fixed number of trials, not for a run that stops at a precision target; the results report the controlled mean and the variance reduction beside the plain ones. Counts with a very large spread are refused as controls.",
        kind: CommandKind::Edit,
        schema: schema::<crate::mc::OutputSet>,
        example: || json!({ "name": "cost", "expression": "$labour * $rate + $materials", "unit": "USD", "at_most": 6000.0, "controls": [{ "input": "labour" }] }),
        run: crate::mc::output_set,
    },
    Command {
        name: "mc.sobol",
        summary: "Monte Carlo: how much of each output's variation each input accounts for, by Sobol indices: `first` alone and `total` with all its interactions, each as a share of the variance with a 95% bootstrap interval, from `rows` rows of two base samples (1,000 by default; the model is evaluated rows × (inputs that vary + 2) times). Needs independent inputs: a model with correlated inputs is refused. An estimate from few rows can fall a little below 0 or above 1.",
        kind: CommandKind::Action,
        schema: schema::<crate::mc::SobolParams>,
        example: || json!({ "rows": 200, "seed": 1, "bootstrap": 100 }),
        run: crate::mc::sobol,
    },
    Command {
        name: "mc.correlation.set",
        summary: "Monte Carlo: the rank (Spearman) correlation between two inputs, strictly between -1 and 1, induced by Iman and Conover's method; 0 removes it. Correlations that cannot all hold together are refused.",
        kind: CommandKind::Edit,
        schema: schema::<crate::mc::CorrelationSet>,
        example: || json!({ "a": "labour", "b": "materials", "rank": 0.5 }),
        run: crate::mc::correlation_set,
    },
    Command {
        name: "mc.set",
        summary: "Monte Carlo: the sampling (`random`, `latin_hypercube`, `antithetic` or `sobol`), the trials and seed, and a sequential stop (more trials until every output's mean is within a share of itself, such as ±1%).",
        kind: CommandKind::Edit,
        schema: schema::<crate::mc::Set>,
        example: || json!({ "sampling": "latin_hypercube", "trials": 2000 }),
        run: crate::mc::set,
    },
    Command {
        name: "mc.get",
        summary: "Monte Carlo: the model's inputs, outputs, correlations, sampling, trials and seed, and what is wrong with them.",
        kind: CommandKind::Query,
        schema: schema::<crate::mc::Get>,
        example: || json!({}),
        run: crate::mc::get,
    },
    Command {
        name: "mc.inputs.export",
        summary: "Monte Carlo: the inputs as a CSV table (`name,distribution,parameters,unit`), as mc.inputs.import reads it.",
        kind: CommandKind::Query,
        schema: schema::<crate::mc::Get>,
        example: || json!({}),
        run: crate::mc::inputs_export,
    },
    Command {
        name: "mc.run",
        summary: "Monte Carlo: run the trials now. Per output: the mean with its 95% interval, standard deviation, percentiles, minimum and maximum, the probability of meeting its target with its interval, a histogram and the cumulative curve; which inputs matter (`influence`: a tornado of swings from each input's 10th to its 90th percentile with the others at their medians, and the rank correlation of each input with the output); with the rank correlations achieved and a run record. With Latin hypercube or Sobol sampling or correlated inputs the trials within a batch of 1,000 are dependent, so the run is whole batches and the intervals are across batches; with antithetic sampling the trials come in pairs and the intervals are across pairs (`interval` says which). Results do not depend on the number of threads.",
        kind: CommandKind::Action,
        schema: schema::<crate::mc::Run>,
        example: || json!({ "trials": 2000, "seed": 1 }),
        run: crate::mc::run,
    },
    Command {
        name: "mc.output.remove",
        summary: "Monte Carlo: remove an output.",
        kind: CommandKind::Edit,
        schema: schema::<crate::mc::Name>,
        example: || json!({ "name": "cost" }),
        run: crate::mc::output_remove,
    },
    Command {
        name: "mc.input.remove",
        summary: "Monte Carlo: remove an input, with its correlations.",
        kind: CommandKind::Edit,
        schema: schema::<crate::mc::Name>,
        example: || json!({ "name": "materials" }),
        run: crate::mc::input_remove,
    },
    Command {
        name: "sd.open",
        summary: "System dynamics: open an XMILE 1.0 model from its text (a file on disk is opened with file.open), replacing the model, clearing the history and starting a new journal. Stocks, flows, auxiliaries, graphical functions, delays and smooths are read; arrays, modules, macros and the like are refused by name. Edit it with the sd.variable, sd.specs and sd.layout commands, and save it with file.save.",
        kind: CommandKind::Action,
        schema: schema::<crate::sd::Open>,
        example: || json!({ "text": crate::sd::EXAMPLE_XMILE }),
        run: crate::sd::open,
    },
    Command {
        name: "sd.run",
        summary: "System dynamics: run the open model. Every variable over time at the save interval, as the mean (the value itself for one run), and, for a model with a random function (NORMAL, RANDOM), run in `replications` (default 100) with a `seed`, also the median and the 5th and 95th percentiles at each time. A model with no random function runs once. `dt` runs it at another time step; `dt_check` runs it again at half the step and reports each stock's largest relative difference between the two; `time_unit` reports times in another time unit. Euler or fourth-order Runge-Kutta, as the file asks.",
        kind: CommandKind::Action,
        schema: schema::<crate::sd::Run>,
        example: || json!({ "dt_check": true }),
        run: crate::sd::run,
    },
    Command {
        name: "sd.get",
        summary: "System dynamics: the open model's settings, variables (kind, equation, units, flows, where each is drawn and the pipes of the flows), the information links and what the checks found, and with `variable` its values from the last sd.run (marked stale when the model was edited since).",
        kind: CommandKind::Query,
        schema: schema::<crate::sd::Get>,
        example: || json!({ "variable": "Population" }),
        run: crate::sd::get,
    },
    Command {
        name: "sd.new",
        summary: "System dynamics: start a new, empty model (a name, the start and stop times, the time step, `euler` or `rk4`, the time unit), replacing the document, clearing the history and starting a new journal. Add variables with sd.variable.add.",
        kind: CommandKind::Action,
        schema: schema::<crate::sd_edit::New>,
        example: || json!({ "name": "Growth", "stop": 10.0, "dt": 1.0, "time_units": "Years" }),
        run: crate::sd_edit::new,
    },
    Command {
        name: "sd.variable.add",
        summary: "System dynamics: add a stock, a flow or an auxiliary to the open model: its name, its equation (a stock's is its initial value), units, a graphical function, a stock's `inflows` and `outflows`, and where to draw it (otherwise in the first free place under the diagram). A flow can be attached to a stock with `into_stock` or `from_stock`. An equation can only name variables that are already there. The edit is undone with undo; one that would not compile (an unknown name, an algebraic loop) is refused and changes nothing.",
        kind: CommandKind::Action,
        schema: schema::<crate::sd_edit::VariableAdd>,
        example: || json!({ "kind": "stock", "name": "Reserve", "equation": "50", "units": "units", "position": { "x": 400.0, "y": 300.0 } }),
        run: crate::sd_edit::variable_add,
    },
    Command {
        name: "sd.variable.set",
        summary: "System dynamics: change a variable of the open model: its equation, units, documentation, a stock's inflows and outflows, `non_negative`, its graphical function, or `rename` it (every equation and flow list that names it is rewritten, so a run gives the same numbers). Undone with undo; an edit that would not compile is refused and changes nothing.",
        kind: CommandKind::Action,
        schema: schema::<crate::sd_edit::VariableSet>,
        example: || json!({ "name": "Reserve", "equation": "60", "rename": "Buffer" }),
        run: crate::sd_edit::variable_set,
    },
    Command {
        name: "sd.specs.set",
        summary: "System dynamics: change the open model's simulation settings: start, stop, the time step `dt` (with `dt_reciprocal` true, the step is 1/dt), the method (`euler` or `rk4`), the time unit and the save step (`null` for the step itself). Undone with undo.",
        kind: CommandKind::Action,
        schema: schema::<crate::sd_edit::SpecsSet>,
        example: || json!({ "stop": 20.0 }),
        run: crate::sd_edit::specs_set,
    },
    Command {
        name: "sd.layout.set",
        summary: "System dynamics: change the diagram of the open model: move shapes (the centre `x`, `y`, a size, a flow's pipe `pts`; the pipes of flows follow a stock that moves), and add or remove information links (`from`, `to`, `angle`). Undone with undo.",
        kind: CommandKind::Action,
        schema: schema::<crate::sd_edit::LayoutSet>,
        example: || json!({ "shapes": [{ "name": "Buffer", "x": 300.0, "y": 200.0 }] }),
        run: crate::sd_edit::layout_set,
    },
    Command {
        name: "sd.variable.remove",
        summary: "System dynamics: remove a variable of the open model. It is refused, naming who uses it, while another equation names it or a stock lists it as a flow; `force` takes it out of the stocks' flow lists (never out of an equation).",
        kind: CommandKind::Action,
        schema: schema::<crate::sd_edit::VariableRemove>,
        example: || json!({ "name": "Buffer" }),
        run: crate::sd_edit::variable_remove,
    },
    Command {
        name: "agents.open",
        summary: "Agents: open an agent-based model from its text (a BPMN file with no process and the stochcraft:agents extension) and the texts of the files it names (each agent type's SCXML 1.0 state chart, each CSV list of links) in `files`, by their references, replacing the model, clearing the history and starting a new journal. A file on disk is opened with file.open, which reads those files from beside it. Run it with agents.run; file.save writes the model and its files.",
        kind: CommandKind::Action,
        schema: schema::<crate::agents::Open>,
        example: || json!({ "text": crate::agents::EXAMPLE_BPMN, "files": { "person.scxml": crate::agents::EXAMPLE_SCXML } }),
        run: crate::agents::open,
    },
    Command {
        name: "agents.get",
        summary: "Agents: the open agent-based model: the run's length, replications, seed and time unit; each agent type with its chart's states and variables; the populations with their networks and grids; the environment; the statistics; the stepped mode; the files it reads; any problem that stops it running; and whether the last agents.run's results are stale.",
        kind: CommandKind::Query,
        schema: schema::<crate::agents::Get>,
        example: || json!({}),
        run: crate::agents::get,
    },
    Command {
        name: "agents.run",
        summary: "Agents: run the open agent-based model in its replications (or `replications`, at most 100,000) from its seed (or `seed`) to the end of its warm-up and duration (or `end`). Each statistic over time at the save interval: the mean, and over more than one replication the median and the 5th and 95th percentiles at each time; each population's agents created, removed and alive at the end (means over the replications) and the messages dropped and undelivered; and the run record. A run that stops (a chart that loops, a rate below zero) is refused with the agent and the replication.",
        kind: CommandKind::Action,
        schema: schema::<crate::agents::RunParams>,
        example: || json!({ "replications": 3, "seed": 7 }),
        run: crate::agents::run,
    },
];

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;

    #[test]
    fn a_file_beside_the_model_is_written_only_as_the_save_found_it() {
        let dir = std::env::temp_dir().join(format!("stochcraft-beside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        let to = dir.join("sorting.dmn");
        // Not there when the save looked, and still not: written.
        assert!(write_beside(&to, b"new", None).is_ok());
        assert_eq!(std::fs::read(&to).ok().as_deref(), Some(&b"new"[..]));
        // There as the save found it: written over.
        assert!(write_beside(&to, b"edited", Some(b"new")).is_ok());
        // Changed since: refused, and left as it is.
        std::fs::write(&to, b"theirs").unwrap_or_else(|e| panic!("{e}"));
        let refused = write_beside(&to, b"mine", Some(b"edited"));
        assert_eq!(refused.map_err(|e| e.code), Err(ErrorCode::Conflict));
        assert_eq!(std::fs::read(&to).ok().as_deref(), Some(&b"theirs"[..]));
        // Made since the save found nothing there: refused too.
        let made = dir.join("made.dmn");
        std::fs::write(&made, b"theirs").unwrap_or_else(|e| panic!("{e}"));
        let refused = write_beside(&made, b"mine", None);
        assert_eq!(refused.map_err(|e| e.code), Err(ErrorCode::Conflict));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
