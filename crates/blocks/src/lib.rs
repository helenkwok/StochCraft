//! Flow blocks: turning a [`Model`] into a simulation.
//!
//! [`compile`] checks a [`stochcraft_model::Model`] and returns a [`Compiled`] model, or every
//! problem found. [`Compiled::run`] runs one replication and returns a [`Report`] of statistics
//! per element, collected after the warm-up. [`Compiled`] is `Send + Sync`, so replications can
//! run in parallel (with `stochcraft-experiment`), and [`summarise`] turns their reports into a
//! 95% confidence interval per element and [`Metric`].
//!
//! Entities are created by a source, wait for and occupy a server with one or more identical
//! units, spend time in a delay with no capacity limit, and leave through a sink. Gateways route
//! them at once: an exclusive gateway down one outgoing connection (by probability, or the first
//! condition that holds), a parallel gateway down all of them as copies, after waiting for one on
//! each incoming connection when it has several (a join). Each copy is a new entity with the
//! original's creation time and properties. Each exclusive gateway draws from its own stream.
//!
//! Entities carry BPSim properties. Each element's [`stochcraft_model::Property`] assignments
//! are made as an entity enters it (at a source, as it is created), all together, from the
//! values the entity arrived with. Each element reports the mean of the values it assigned to
//! each property (numbers as they are, booleans as 0 or 1). Values drawn for properties come
//! from streams of their own, so adding a property never changes the other random numbers.
//!
//! [`Builder`] writes the same models from code, as in the tests and examples:
//!
//! ```
//! use stochcraft_blocks::Builder;
//! use stochcraft_random::{Dist, Rng};
//!
//! let mut m = Builder::new();
//! let arrivals = m.source("arrivals", Dist::exponential_rate(0.75), None);
//! let machine = m.server("machine", 1, Dist::exponential_rate(1.0));
//! let exit = m.sink("exit");
//! m.connect(arrivals, machine);
//! m.connect(machine, exit);
//! let report = m.run(Rng::new(1), 1_000.0, 10_000.0).expect("valid model");
//! assert!(report.element("machine").and_then(|e| e.utilisation).is_some());
//! ```

mod runtime;

pub use stochcraft_model::Breakdown;
pub use stochcraft_model::calendar::{Calendar, Event, Rule};
use stochcraft_model::expr;
use stochcraft_model::{
    Connection, Diagnostic, Element, ElementId, ElementKind, Experiment, Model, Need, Property,
    PropertyType, PropertyValue, ResourcePool, Severity, Timed,
};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::Summary;

/// A property value an entity holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Held {
    Long(i64),
    Double(f64),
    Boolean(bool),
    Text(String),
}

impl Held {
    /// As an expression value.
    pub fn value(&self) -> expr::Value {
        match self {
            Held::Long(n) => expr::Value::Number(*n as f64),
            Held::Double(n) => expr::Value::Number(*n),
            Held::Boolean(b) => expr::Value::Boolean(*b),
            Held::Text(s) => expr::Value::Text(s.clone()),
        }
    }

    /// `v` as a property of type `ty`: a long is rounded to the nearest whole number (halves
    /// away from zero), and is not set at all when `v` is not a finite number.
    pub fn of(v: &expr::Value, ty: PropertyType) -> Option<Held> {
        Some(match ty {
            PropertyType::Long => {
                let n = v.number();
                if !n.is_finite() {
                    return None;
                }
                // `as` saturates at the ends of i64's range.
                Held::Long(n.round() as i64)
            }
            PropertyType::Double | PropertyType::Duration => Held::Double(v.number()),
            PropertyType::Boolean => Held::Boolean(v.boolean()),
            PropertyType::String | PropertyType::DateTime => Held::Text(v.text()),
        })
    }

    /// The number recorded in an element's statistics: numbers as they are, booleans as 0 or 1.
    fn statistic(&self) -> Option<f64> {
        match self {
            Held::Long(n) => Some(*n as f64),
            Held::Double(n) if n.is_finite() => Some(*n),
            Held::Boolean(b) => Some(f64::from(u8::from(*b))),
            Held::Double(_) | Held::Text(_) => None,
        }
    }
}

/// Something that flows through the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    /// Unique within a run, in creation order.
    pub id: u64,
    /// When its source created it.
    pub created: f64,
    /// When it entered the element it is in now.
    pub entered: f64,
    /// Its properties, by the slot [`Compiled::property_names`] gives each name (`None`: not
    /// set). Empty when the model has no properties.
    pub properties: Vec<Option<Held>>,
}

/// Where an assigned value comes from.
#[derive(Debug, Clone)]
enum Source {
    Fixed(Option<Held>),
    Random(Dist),
    Expr(expr::Expr),
}

/// One property assignment, ready to run.
#[derive(Debug, Clone)]
struct Assign {
    name: String,
    slot: usize,
    ty: PropertyType,
    source: Source,
}

/// One of the connections an element sends entities down.
#[derive(Debug, Clone)]
struct Route {
    /// The element it leads to.
    to: usize,
    /// Which of that element's inputs it arrives on: a join has one per incoming connection, in
    /// model order; every other element has one.
    input: usize,
    /// Out of an exclusive gateway routing by probability: this connection's share.
    weight: f64,
    /// Out of an exclusive gateway routing by condition: what must hold to take it.
    condition: Option<expr::Expr>,
    /// The gateway's default connection.
    default: bool,
}

/// A model checked and ready to run.
#[derive(Debug, Clone)]
pub struct Compiled {
    model: Model,
    /// For each element, the connections it sends entities down, in model order.
    routes: Vec<Vec<Route>>,
    /// For each element, how many inputs it has (more than one only for a join).
    inputs: Vec<usize>,
    warnings: Vec<Diagnostic>,
    /// Property names, sorted: an entity's slots.
    slots: Vec<String>,
    /// For each element, its property assignments.
    assigns: Vec<Vec<Assign>>,
    /// Sources create their first entity at time 0 rather than after the first interval.
    first_arrival_at_zero: bool,
}

fn compile_assign(p: &Property, slots: &[String], ty: PropertyType) -> Option<Assign> {
    let source = match &p.value {
        PropertyValue::Number(n) => Source::Fixed(Held::of(&expr::Value::Number(*n), ty)),
        PropertyValue::Boolean(b) => Source::Fixed(Held::of(&expr::Value::Boolean(*b), ty)),
        PropertyValue::Text(s) => Source::Fixed(Held::of(&expr::Value::Text(s.clone()), ty)),
        PropertyValue::Random(d) => Source::Random(d.clone()),
        PropertyValue::Expression(src) => Source::Expr(expr::parse(src).ok()?),
    };
    Some(Assign {
        name: p.name.clone(),
        slot: slots.binary_search(&p.name).ok()?,
        ty,
        source,
    })
}

/// Checks `model` and prepares it to run. On any error, returns every diagnostic (errors and
/// warnings) so all of them can be fixed at once.
pub fn compile(model: &Model) -> Result<Compiled, Vec<Diagnostic>> {
    let diagnostics = model.validate();
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(diagnostics);
    }
    // Validation guarantees every non-sink element has an output, only gateways more than one,
    // and every connection joins known elements.
    let index = |id: &ElementId| model.elements.iter().position(|e| &e.id == id);
    let incoming = |e: &Element| -> Vec<&Connection> {
        model.connections.iter().filter(|c| c.to == e.id).collect()
    };
    let is_join =
        |e: &Element| matches!(e.kind, ElementKind::ParallelGateway) && incoming(e).len() > 1;
    let inputs = model
        .elements
        .iter()
        .map(|e| if is_join(e) { incoming(e).len() } else { 1 })
        .collect();
    let routes = model
        .elements
        .iter()
        .map(|e| {
            let outs: Vec<&Connection> = model
                .connections
                .iter()
                .filter(|c| c.from == e.id)
                .collect();
            let default = match &e.kind {
                ElementKind::ExclusiveGateway { default } => default.as_ref(),
                _ => None,
            };
            let by_condition = outs.iter().any(|c| c.condition.is_some());
            // Probabilities: those not given share evenly what the given ones leave; if
            // nothing is left to share, every connection counts the same.
            let given: f64 = outs.iter().filter_map(|c| c.probability).sum();
            let unset = outs.iter().filter(|c| c.probability.is_none()).count();
            let share = if unset > 0 {
                (1.0 - given).max(0.0) / unset as f64
            } else {
                0.0
            };
            let mut weights: Vec<f64> = outs
                .iter()
                .map(|c| c.probability.unwrap_or(share))
                .collect();
            if weights.iter().sum::<f64>() <= 0.0 {
                weights.iter_mut().for_each(|w| *w = 1.0);
            }
            outs.iter()
                .zip(weights)
                .filter_map(|(c, weight)| {
                    let to = index(&c.to)?;
                    let target = model.elements.get(to)?;
                    let input = if is_join(target) {
                        incoming(target).iter().position(|x| x.id == c.id)?
                    } else {
                        0
                    };
                    Some(Route {
                        to,
                        input,
                        weight,
                        condition: if by_condition {
                            c.condition.as_deref().and_then(|s| expr::parse(s).ok())
                        } else {
                            None
                        },
                        default: default == Some(&c.id),
                    })
                })
                .collect()
        })
        .collect();
    let types = model.property_types();
    let slots: Vec<String> = types.keys().cloned().collect();
    // Validation guarantees every expression parses and every property has a type.
    let assigns = model
        .elements
        .iter()
        .map(|e| {
            e.properties
                .iter()
                .filter_map(|p| compile_assign(p, &slots, *types.get(&p.name)?))
                .collect()
        })
        .collect();
    Ok(Compiled {
        model: model.clone(),
        routes,
        inputs,
        warnings: diagnostics,
        slots,
        assigns,
        first_arrival_at_zero: false,
    })
}

impl Compiled {
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Makes sources create their first entity at time 0 instead of after the first interval
    /// (decision D8 in ROADMAP.md is open; the default is after the first interval). For
    /// cross-checks against simulators that start at time 0.
    #[doc(hidden)]
    pub fn first_arrival_at_zero(mut self, yes: bool) -> Self {
        self.first_arrival_at_zero = yes;
        self
    }

    /// The names of the properties entities can carry, in slot order.
    pub fn property_names(&self) -> &[String] {
        &self.slots
    }

    /// Warnings found when compiling (the model runs, but perhaps not as intended).
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }
}

/// A statistic an element reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Metric {
    /// Entities that entered (for a source: created).
    Entered,
    /// Entities that left (for a sink: removed).
    Left,
    /// Servers: time-average number waiting for a unit.
    MeanQueueLength,
    /// Servers: mean time from arrival to start of service.
    MeanWait,
    /// Servers: time-average fraction of units busy.
    Utilisation,
    /// Servers and delays: time-average number of entities being served or delayed.
    MeanContents,
    /// Sinks: mean time from creation to arrival.
    MeanTimeInSystem,
    /// Servers and delays sending to a finite buffer: mean time an entity that left waited,
    /// after its service or delay, for room in the next buffer (zero when there was room).
    MeanBlockedTime,
    /// Servers sending to a finite buffer: time-average fraction of units blocked (holding a
    /// served entity while the next buffer is full).
    BlockingProbability,
    /// Servers with a finite buffer: arrivals from sources lost because the buffer was full.
    Lost,
    /// Servers with a finite buffer: the fraction of arrivals lost (lost over lost plus
    /// entered).
    LossProbability,
    /// Servers whose units fail: time-average fraction of units up (not failed or under
    /// repair).
    Availability,
}

impl Metric {
    pub const ALL: [Metric; 12] = [
        Metric::Entered,
        Metric::Left,
        Metric::MeanQueueLength,
        Metric::MeanWait,
        Metric::Utilisation,
        Metric::MeanContents,
        Metric::MeanTimeInSystem,
        Metric::MeanBlockedTime,
        Metric::BlockingProbability,
        Metric::Lost,
        Metric::LossProbability,
        Metric::Availability,
    ];

    /// A stable name for tables, files and agents.
    pub fn name(self) -> &'static str {
        match self {
            Metric::Entered => "entered",
            Metric::Left => "left",
            Metric::MeanQueueLength => "mean_queue_length",
            Metric::MeanWait => "mean_wait",
            Metric::Utilisation => "utilisation",
            Metric::MeanContents => "mean_contents",
            Metric::MeanTimeInSystem => "mean_time_in_system",
            Metric::MeanBlockedTime => "mean_blocked_time",
            Metric::BlockingProbability => "blocking_probability",
            Metric::Lost => "lost",
            Metric::LossProbability => "loss_probability",
            Metric::Availability => "availability",
        }
    }
}

/// Statistics for one element over the reporting period of one replication.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementReport {
    pub id: ElementId,
    pub name: String,
    /// "source", "server", "delay" or "sink".
    pub kind: &'static str,
    pub entered: u64,
    pub left: u64,
    pub mean_queue_length: Option<f64>,
    pub mean_wait: Option<f64>,
    pub utilisation: Option<f64>,
    pub mean_contents: Option<f64>,
    pub mean_time_in_system: Option<f64>,
    /// Only where a finite buffer is involved (`None` otherwise, so models without one report
    /// as before): see [`Metric::MeanBlockedTime`] and the metrics after it.
    pub mean_blocked_time: Option<f64>,
    pub blocking_probability: Option<f64>,
    /// Servers whose units fail: [`Metric::Availability`].
    pub availability: Option<f64>,
    pub lost: Option<u64>,
    pub loss_probability: Option<f64>,
    /// For each property the element assigns, in the element's order: the mean of the values
    /// it assigned (`None` when it assigned none, or the property holds text).
    pub properties: Vec<(String, Option<f64>)>,
}

impl ElementReport {
    /// The value of `metric`, if this element reports it.
    pub fn metric(&self, metric: Metric) -> Option<f64> {
        match metric {
            Metric::Entered => Some(self.entered as f64),
            Metric::Left => Some(self.left as f64),
            Metric::MeanQueueLength => self.mean_queue_length,
            Metric::MeanWait => self.mean_wait,
            Metric::Utilisation => self.utilisation,
            Metric::MeanContents => self.mean_contents,
            Metric::MeanTimeInSystem => self.mean_time_in_system,
            Metric::MeanBlockedTime => self.mean_blocked_time,
            Metric::BlockingProbability => self.blocking_probability,
            Metric::Lost => self.lost.map(|n| n as f64),
            Metric::LossProbability => self.loss_probability,
            Metric::Availability => self.availability,
        }
    }
}

/// The results of one replication.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub warmup: f64,
    pub duration: f64,
    /// In model order.
    pub elements: Vec<ElementReport>,
}

impl Report {
    /// The element with this id.
    pub fn element(&self, id: &str) -> Option<&ElementReport> {
        self.elements.iter().find(|e| e.id.as_str() == id)
    }
}

/// One element's results across replications.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementSummary {
    pub id: ElementId,
    pub name: String,
    pub kind: &'static str,
    /// Each metric the element reports, in [`Metric::ALL`] order.
    pub metrics: Vec<(Metric, Summary)>,
    /// The mean value assigned to each property, as in [`ElementReport::properties`].
    pub properties: Vec<(String, Summary)>,
}

impl ElementSummary {
    pub fn metric(&self, metric: Metric) -> Option<&Summary> {
        self.metrics
            .iter()
            .find(|(m, _)| *m == metric)
            .map(|(_, s)| s)
    }
}

/// The results of an experiment: a confidence interval per element and metric.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperimentSummary {
    pub replications: u64,
    pub elements: Vec<ElementSummary>,
}

impl ExperimentSummary {
    pub fn get(&self, id: &str, metric: Metric) -> Option<&Summary> {
        self.elements
            .iter()
            .find(|e| e.id.as_str() == id)?
            .metric(metric)
    }
}

/// Summarises replication reports of the same model (as [`Compiled::run`] returns them, in any
/// order). A metric is left out of an element when no replication reported a value for it, for
/// example the mean wait of a server that no entity reached.
pub fn summarise(reports: &[Report]) -> ExperimentSummary {
    let elements = reports.first().map_or_else(Vec::new, |first| {
        first
            .elements
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let metrics = Metric::ALL
                    .iter()
                    .filter_map(|&m| {
                        let values: Vec<f64> = reports
                            .iter()
                            .filter_map(|r| r.elements.get(i)?.metric(m))
                            .collect();
                        Summary::of(&values).map(|s| (m, s))
                    })
                    .collect();
                let properties = e
                    .properties
                    .iter()
                    .enumerate()
                    .filter_map(|(j, (name, _))| {
                        let values: Vec<f64> = reports
                            .iter()
                            .filter_map(|r| r.elements.get(i)?.properties.get(j)?.1)
                            .collect();
                        Summary::of(&values).map(|s| (name.clone(), s))
                    })
                    .collect();
                ElementSummary {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    kind: e.kind,
                    metrics,
                    properties,
                }
            })
            .collect()
    });
    ExperimentSummary {
        replications: reports.len() as u64,
        elements,
    }
}

/// An element added to a [`Builder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(usize);

/// Writes a [`Model`] from code. Each element's name is also its id, so names must be valid ids
/// (a letter or `_` first, then letters, digits, `.`, `-` or `_`) and unique; problems are
/// reported when the model is built or run.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    elements: Vec<(String, ElementKind, Vec<Property>)>,
    /// `from` → `to`, with a probability and a condition; a later connection from the same
    /// element replaces the earlier one unless `from` is a gateway.
    connections: Vec<(BlockId, BlockId, Option<f64>, Option<String>)>,
    /// Shared resource pools.
    pools: Vec<ResourcePool>,
    calendars: Vec<Calendar>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, name: &str, kind: ElementKind) -> BlockId {
        self.elements.push((name.to_owned(), kind, Vec::new()));
        BlockId(self.elements.len() - 1)
    }

    /// Makes `block` set property `name` on every entity that enters it (or that it creates).
    pub fn property(
        &mut self,
        block: BlockId,
        name: &str,
        ty: Option<PropertyType>,
        value: PropertyValue,
    ) {
        if let Some((_, _, props)) = self.elements.get_mut(block.0) {
            props.push(Property {
                name: name.to_owned(),
                ty,
                value,
            });
        }
    }

    /// Creates entities with the given time between arrivals, up to `limit` if one is given.
    pub fn source(&mut self, name: &str, interarrival: Dist, limit: Option<u64>) -> BlockId {
        self.add(
            name,
            ElementKind::Source {
                interarrival,
                limit,
                idle_outside: false,
                timed: Vec::new(),
            },
        )
    }

    /// `capacity` identical units that each serve one entity at a time, first come first served,
    /// for a time drawn from `service`.
    pub fn server(&mut self, name: &str, capacity: u32, service: Dist) -> BlockId {
        self.add(
            name,
            ElementKind::Server {
                capacity,
                service,
                queue_capacity: None,
                breakdown: None,
            },
        )
    }

    /// As [`Builder::server`], with a finite buffer: at most `queue_capacity` entities wait for
    /// a unit. When it is full, a server or delay sending here waits holding its entity
    /// (blocking after service) and a source loses its arrival.
    pub fn buffered_server(
        &mut self,
        name: &str,
        capacity: u32,
        service: Dist,
        queue_capacity: u32,
    ) -> BlockId {
        self.add(
            name,
            ElementKind::Server {
                capacity,
                service,
                queue_capacity: Some(queue_capacity),
                breakdown: None,
            },
        )
    }

    /// A pool of `quantity` identical units, playing `roles`, that shared tasks draw on. Its
    /// name is its id.
    pub fn pool(&mut self, name: &str, quantity: u32, roles: &[&str]) {
        self.pools.push(ResourcePool::new(
            ElementId::new(name).unwrap_or_else(|| ElementId::unchecked(name)),
            name,
            quantity,
            roles.iter().map(|r| (*r).to_owned()).collect(),
        ));
    }

    /// Makes server `block`'s units fail and be repaired.
    pub fn breakdown(&mut self, block: BlockId, breakdown: Breakdown) {
        if let Some((_, ElementKind::Server { breakdown: b, .. }, _)) =
            self.elements.get_mut(block.0)
        {
            *b = Some(breakdown);
        }
    }

    /// A calendar of `events`; its name is its id.
    pub fn calendar(&mut self, name: &str, events: Vec<Event>) {
        self.calendars.push(Calendar {
            id: ElementId::unchecked(name),
            name: name.to_owned(),
            events,
        });
    }

    /// Pool `pool`'s units while each calendar is active (the first active one applying); its
    /// quantity applies outside them.
    pub fn shifts(&mut self, pool: &str, shifts: &[(&str, u32)]) {
        if let Some(p) = self.pools.iter_mut().find(|p| p.name == pool) {
            p.shifts = shifts
                .iter()
                .map(|(c, q)| Timed {
                    calendar: ElementId::unchecked(c),
                    value: *q,
                })
                .collect();
        }
    }

    /// Source `block`'s time between arrivals while each calendar is active (the first active
    /// one applying); outside them, its own interarrival time, or none with `idle_outside`.
    pub fn timed_arrivals(&mut self, block: BlockId, timed: Vec<(&str, Dist)>, idle_outside: bool) {
        if let Some((
            _,
            ElementKind::Source {
                timed: t,
                idle_outside: i,
                ..
            },
            _,
        )) = self.elements.get_mut(block.0)
        {
            *t = timed
                .into_iter()
                .map(|(c, d)| Timed {
                    calendar: ElementId::unchecked(c),
                    value: d,
                })
                .collect();
            *i = idle_outside;
        }
    }

    /// A task whose units come from the shared pools: each entity takes what the first of
    /// `needs` that can be met asks for, ahead of entities of tasks with a lower `priority`.
    pub fn shared_task(
        &mut self,
        name: &str,
        service: Dist,
        needs: Vec<Need>,
        priority: f64,
    ) -> BlockId {
        self.add(
            name,
            ElementKind::SharedTask {
                service,
                needs,
                priority,
            },
        )
    }

    /// Holds every entity for a time drawn from `duration`, with no capacity limit.
    pub fn delay(&mut self, name: &str, duration: Dist) -> BlockId {
        self.add(name, ElementKind::Delay { duration })
    }

    /// Removes entities from the model and records their time in system.
    pub fn sink(&mut self, name: &str) -> BlockId {
        self.add(name, ElementKind::Sink)
    }

    /// Sends each entity down one outgoing connection: by condition if any connection has one
    /// ([`Builder::route`]), else by the connections' probabilities.
    pub fn exclusive(&mut self, name: &str) -> BlockId {
        self.add(name, ElementKind::ExclusiveGateway { default: None })
    }

    /// Makes the connection from exclusive gateway `gateway` to `to` its default.
    pub fn default_route(&mut self, gateway: BlockId, to: BlockId) {
        // Connection ids are only known when the model is built: keep the target's name.
        let target = self
            .elements
            .get(to.0)
            .map(|(n, _, _)| n.clone())
            .unwrap_or_default();
        if let Some((_, kind, _)) = self.elements.get_mut(gateway.0) {
            *kind = ElementKind::ExclusiveGateway {
                default: Some(ElementId::unchecked(&format!("->{target}"))),
            };
        }
    }

    /// Copies each entity down every outgoing connection; with several incoming connections,
    /// waits for one entity on each first.
    pub fn parallel(&mut self, name: &str) -> BlockId {
        self.add(name, ElementKind::ParallelGateway)
    }

    fn is_gateway(&self, b: BlockId) -> bool {
        self.elements
            .get(b.0)
            .is_some_and(|(_, k, _)| k.is_gateway())
    }

    /// Sends `from`'s entities to `to`. A later call for the same `from` replaces the earlier one,
    /// unless `from` is a gateway.
    pub fn connect(&mut self, from: BlockId, to: BlockId) {
        self.route(from, to, None, None);
    }

    /// Connects exclusive gateway `from` to `to`, taken with `probability`, or when `condition`
    /// (a BPSim expression) holds.
    pub fn route(
        &mut self,
        from: BlockId,
        to: BlockId,
        probability: Option<f64>,
        condition: Option<&str>,
    ) {
        if !self.is_gateway(from) {
            self.connections.retain(|(f, ..)| *f != from);
        }
        self.connections
            .push((from, to, probability, condition.map(str::to_owned)));
    }

    /// The model, with the given experiment settings. Names that are not valid ids are kept as
    /// written in the model's diagnostics.
    pub fn build(&self, experiment: Experiment) -> Model {
        let id = |i: usize| {
            let name = self.elements.get(i).map_or("", |(n, _, _)| n.as_str());
            ElementId::new(name).unwrap_or_else(|| ElementId::unchecked(name))
        };
        let mut connections: Vec<&(BlockId, BlockId, Option<f64>, Option<String>)> =
            self.connections.iter().collect();
        connections.sort_by_key(|(from, to, ..)| (*from, *to));
        let connections: Vec<Connection> = connections
            .into_iter()
            .enumerate()
            .map(|(n, (from, to, probability, condition))| Connection {
                id: ElementId::unchecked(&format!("flow-{}", n + 1)),
                from: id(from.0),
                to: id(to.0),
                probability: *probability,
                condition: condition.clone(),
            })
            .collect();
        let elements = self
            .elements
            .iter()
            .enumerate()
            .map(|(i, (name, kind, properties))| {
                // A default route names its target (`->name`) until connection ids exist.
                let kind = match kind {
                    ElementKind::ExclusiveGateway {
                        default: Some(d), ..
                    } => ElementKind::ExclusiveGateway {
                        default: d.as_str().strip_prefix("->").and_then(|t| {
                            connections
                                .iter()
                                .find(|c| c.from == id(i) && c.to.as_str() == t)
                                .map(|c| c.id.clone())
                        }),
                    },
                    other => other.clone(),
                };
                Element {
                    id: id(i),
                    name: name.clone(),
                    kind,
                    properties: properties.clone(),
                }
            })
            .collect();
        Model {
            name: String::new(),
            elements,
            connections,
            resources: self.pools.clone(),
            calendars: self.calendars.clone(),
            experiment,
        }
    }

    /// Builds, compiles and runs one replication for `warmup + duration` time units.
    pub fn run(&self, rng: Rng, warmup: f64, duration: f64) -> Result<Report, Vec<Diagnostic>> {
        let experiment = Experiment {
            warmup,
            duration,
            replications: 1,
            ..Experiment::default()
        };
        Ok(compile(&self.build(experiment))?.run(rng))
    }
}

#[cfg(test)]
mod tests;
