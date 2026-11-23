//! An agent-based model as a file holds it (ROADMAP M4.3, stage A4, decision D16): populations of
//! agents, each agent type's state chart, the environment, statistics over time and the stepped
//! mode. This module only holds the model's description; `stochcraft-agents` builds and runs it,
//! and `stochcraft-io` reads and writes it.
//!
//! In a file it is a BPMN file with no process. Its scenario gives the run's length (the
//! warm-up plus the duration), the replications, the seed and the time unit
//! ([`crate::Experiment`]); the rest is the `stochcraft:agents` vendor extension on the scenario.
//! Each agent type's state chart is a W3C SCXML 1.0 document of its own, which the extension
//! names by a relative reference (`chart="person.scxml"`), as a network read from a list of links
//! names its CSV file. Rates, intervals and the stepped mode's step are in the experiment's time
//! unit.

/// The agent-based part of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Agents {
    /// The agent types, each with its state chart.
    pub types: Vec<AgentType>,
    pub populations: Vec<PopulationSpec>,
    /// The agent type of the environment, one agent outside the populations; `None` for none.
    pub environment: Option<String>,
    pub statistics: Vec<StatisticSpec>,
    pub stepping: Option<SteppingSpec>,
    /// The statistics are recorded at the start, every `save_every` (if set) and at the end.
    pub save_every: Option<f64>,
    /// The most microsteps in one macrostep before a run stops as looping; `None` for the
    /// default.
    pub microstep_limit: Option<u64>,
    /// The most re-tests that take a transition after one event before a run stops as looping;
    /// `None` for the default.
    pub retest_limit: Option<u64>,
}

impl Agents {
    pub fn agent_type(&self, name: &str) -> Option<&AgentType> {
        self.types.iter().find(|t| t.name == name)
    }

    /// The relative references of the files the model reads besides its own: the state charts,
    /// then the lists of links, each once, in order.
    pub fn files(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        let networks = self.populations.iter().filter_map(|p| match &p.network {
            Some(NetworkSpec::File { href }) => Some(href.as_str()),
            _ => None,
        });
        for href in self.types.iter().map(|t| t.chart.as_str()).chain(networks) {
            if !out.contains(&href) {
                out.push(href);
            }
        }
        out
    }
}

/// An agent type: a name and the relative reference of its SCXML state chart.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentType {
    pub name: String,
    pub chart: String,
}

/// A population: agents of one type.
#[derive(Debug, Clone, PartialEq)]
pub struct PopulationSpec {
    pub name: String,
    /// The name of its [`AgentType`].
    pub agent_type: String,
    /// The agents created at the start.
    pub count: u64,
    /// Agents arriving at this rate (exponential gaps), from the start.
    pub arrival_rate: Option<f64>,
    pub network: Option<NetworkSpec>,
    pub grid: Option<GridSpec>,
}

/// The links between a population's agents, built at the start.
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkSpec {
    /// No links at the start (agents link during the run).
    Empty,
    /// Each pair linked with the probability that gives this mean degree.
    Random { mean_degree: f64 },
    /// A ring lattice: each agent linked to `each_side` neighbours on each side.
    Ring { each_side: u64 },
    /// Watts and Strogatz's small world: the ring lattice with each link rewired with
    /// probability `rewire`.
    SmallWorld { each_side: u64, rewire: f64 },
    /// Barabási and Albert's preferential attachment: a complete network of `initial`, then
    /// each agent linked to `links` earlier ones.
    ScaleFree { initial: u64, links: u64 },
    /// Positions placed at random in a `width` by `height` rectangle, linked within `range`.
    Distance { width: f64, height: f64, range: f64 },
    /// These links between serial numbers.
    Links(Vec<(u64, u64)>),
    /// The links in a CSV file, by its relative reference.
    File { href: String },
}

impl NetworkSpec {
    /// The `kind` the file writes.
    pub fn file_name(&self) -> &'static str {
        match self {
            NetworkSpec::Empty => "empty",
            NetworkSpec::Random { .. } => "random",
            NetworkSpec::Ring { .. } => "ring",
            NetworkSpec::SmallWorld { .. } => "smallWorld",
            NetworkSpec::ScaleFree { .. } => "scaleFree",
            NetworkSpec::Distance { .. } => "distance",
            NetworkSpec::Links(_) => "links",
            NetworkSpec::File { .. } => "file",
        }
    }
}

/// A grid of cells for a population's agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSpec {
    pub rows: u64,
    pub columns: u64,
    /// Von Neumann neighbours (4) rather than Moore's (8).
    pub von_neumann: bool,
    /// The edges wrap round (a torus).
    pub wrap: bool,
    /// Many agents may share a cell; otherwise at most one.
    pub many_per_cell: bool,
    /// Agents are placed in random free cells; otherwise in order, row by row.
    pub random_placement: bool,
}

impl GridSpec {
    /// `rows` × `columns`, Moore neighbours, not wrapped, one agent per cell, placed in order.
    pub fn new(rows: u64, columns: u64) -> Self {
        Self {
            rows,
            columns,
            von_neumann: false,
            wrap: false,
            many_per_cell: false,
            random_placement: false,
        }
    }
}

/// What a statistic computes over the agents that meet its condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasureSpec {
    Count,
    Sum,
    Mean,
    Min,
    Max,
}

impl MeasureSpec {
    pub const ALL: [MeasureSpec; 5] = [
        MeasureSpec::Count,
        MeasureSpec::Sum,
        MeasureSpec::Mean,
        MeasureSpec::Min,
        MeasureSpec::Max,
    ];

    pub fn file_name(self) -> &'static str {
        match self {
            MeasureSpec::Count => "count",
            MeasureSpec::Sum => "sum",
            MeasureSpec::Mean => "mean",
            MeasureSpec::Min => "min",
            MeasureSpec::Max => "max",
        }
    }

    pub fn from_file_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.file_name() == s)
    }
}

/// A statistic of one population over time.
#[derive(Debug, Clone, PartialEq)]
pub struct StatisticSpec {
    pub name: String,
    pub population: String,
    pub measure: MeasureSpec,
    /// The expression over each agent (none for a count).
    pub expression: Option<String>,
    /// Only the agents for which this holds; all if `None`.
    pub condition: Option<String>,
}

/// The stepped mode: a step every `every` time units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SteppingSpec {
    pub every: f64,
    /// The agents take their turns in one random order per step; otherwise in population, then
    /// serial order.
    pub random_order: bool,
}
