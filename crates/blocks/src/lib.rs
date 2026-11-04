//! Flow blocks: turning a [`Model`] into a simulation.
//!
//! [`compile`] checks a [`stochcraft_model::Model`] and returns a [`Compiled`] model, or every
//! problem found. [`Compiled::run`] runs one replication and returns a [`Report`] of statistics
//! per element, collected after the warm-up. [`Compiled`] is `Send + Sync`, so replications can
//! run in parallel (with `stochcraft-experiment`), and [`summarise`] turns their reports into a
//! 95% confidence interval per element and [`Metric`].
//!
//! Entities are created by a source, wait for and occupy a server with one or more identical
//! units, spend time in a delay with no capacity limit, and leave through a sink.
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

use stochcraft_model::{
    Connection, Diagnostic, Element, ElementId, ElementKind, Experiment, Model, Severity,
};
use stochcraft_random::{Dist, Rng};
use stochcraft_stats::Summary;

/// Something that flows through the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entity {
    /// Unique within a run, in creation order.
    pub id: u64,
    /// When its source created it.
    pub created: f64,
    /// When it entered the element it is in now.
    pub entered: f64,
}

/// A model checked and ready to run.
#[derive(Debug, Clone)]
pub struct Compiled {
    model: Model,
    /// For each element, the index of the element it sends entities to.
    outputs: Vec<Option<usize>>,
    warnings: Vec<Diagnostic>,
}

/// Checks `model` and prepares it to run. On any error, returns every diagnostic (errors and
/// warnings) so all of them can be fixed at once.
pub fn compile(model: &Model) -> Result<Compiled, Vec<Diagnostic>> {
    let diagnostics = model.validate();
    if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        return Err(diagnostics);
    }
    // Validation guarantees every non-sink element has exactly one known output.
    let index = |id: &ElementId| model.elements.iter().position(|e| &e.id == id);
    let outputs = model
        .elements
        .iter()
        .map(|e| {
            model
                .connections
                .iter()
                .find(|c| c.from == e.id)
                .and_then(|c| index(&c.to))
        })
        .collect();
    Ok(Compiled {
        model: model.clone(),
        outputs,
        warnings: diagnostics,
    })
}

impl Compiled {
    pub fn model(&self) -> &Model {
        &self.model
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
}

impl Metric {
    pub const ALL: [Metric; 7] = [
        Metric::Entered,
        Metric::Left,
        Metric::MeanQueueLength,
        Metric::MeanWait,
        Metric::Utilisation,
        Metric::MeanContents,
        Metric::MeanTimeInSystem,
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
                ElementSummary {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    kind: e.kind,
                    metrics,
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
    elements: Vec<(String, ElementKind)>,
    /// `from` → `to`; a later connection from the same element replaces the earlier one.
    connections: Vec<(BlockId, BlockId)>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, name: &str, kind: ElementKind) -> BlockId {
        self.elements.push((name.to_owned(), kind));
        BlockId(self.elements.len() - 1)
    }

    /// Creates entities with the given time between arrivals, up to `limit` if one is given.
    pub fn source(&mut self, name: &str, interarrival: Dist, limit: Option<u64>) -> BlockId {
        self.add(
            name,
            ElementKind::Source {
                interarrival,
                limit,
            },
        )
    }

    /// `capacity` identical units that each serve one entity at a time, first come first served,
    /// for a time drawn from `service`.
    pub fn server(&mut self, name: &str, capacity: u32, service: Dist) -> BlockId {
        self.add(name, ElementKind::Server { capacity, service })
    }

    /// Holds every entity for a time drawn from `duration`, with no capacity limit.
    pub fn delay(&mut self, name: &str, duration: Dist) -> BlockId {
        self.add(name, ElementKind::Delay { duration })
    }

    /// Removes entities from the model and records their time in system.
    pub fn sink(&mut self, name: &str) -> BlockId {
        self.add(name, ElementKind::Sink)
    }

    /// Sends `from`'s entities to `to`. A later call for the same `from` replaces the earlier one.
    pub fn connect(&mut self, from: BlockId, to: BlockId) {
        self.connections.retain(|(f, _)| *f != from);
        self.connections.push((from, to));
    }

    /// The model, with the given experiment settings. Names that are not valid ids are kept as
    /// written in the model's diagnostics.
    pub fn build(&self, experiment: Experiment) -> Model {
        let id = |i: usize| {
            let name = self.elements.get(i).map_or("", |(n, _)| n.as_str());
            ElementId::new(name).unwrap_or_else(|| ElementId::unchecked(name))
        };
        let elements = self
            .elements
            .iter()
            .enumerate()
            .map(|(i, (name, kind))| Element {
                id: id(i),
                name: name.clone(),
                kind: kind.clone(),
            })
            .collect();
        let mut connections: Vec<&(BlockId, BlockId)> = self.connections.iter().collect();
        connections.sort();
        let connections = connections
            .into_iter()
            .enumerate()
            .map(|(n, (from, to))| Connection {
                id: ElementId::unchecked(&format!("flow-{}", n + 1)),
                from: id(from.0),
                to: id(to.0),
            })
            .collect();
        Model {
            name: String::new(),
            elements,
            connections,
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
