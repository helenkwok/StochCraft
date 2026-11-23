//! Populations of agents on one clock (ROADMAP M4.3, stage A2): a [`Model`] of populations,
//! an optional environment, statistics and an optional stepped mode, run for one replication
//! by a [`Simulation`].
//!
//! **Populations.** A [`Population`] is agents of one chart, created at the start (a count), by
//! arrivals at a rate (exponential gaps from the population's own stream), or by another
//! agent's [`Action::Create`](crate::Action::Create). Each gets the next serial number in its
//! population (0, 1, 2, ...; never reused) and the key `<population>/<serial>`, so its streams
//! are keyed by the seed, its population, its serial and their purpose (D18) and adding an agent
//! never changes another's numbers. An agent leaves its population when its chart reaches a
//! top-level final state, or when another agent removes it
//! ([`Action::Remove`](crate::Action::Remove)), which ends its chart as SCXML's
//! `exitInterpreter` does (exit actions run).
//!
//! **Events.** Every event delivered to an agent is one SCXML external event, handled by one
//! complete macrostep. Messages ([`To`](crate::To)), creations and removals are each an event of
//! their own at the current time, after the current event, in the order the sender's actions ran
//! (a broadcast to its recipients in serial order). Events at one time run first scheduled,
//! first (D18's default order).
//!
//! **Conditions on populations.** After each event, every population it changed (an agent
//! created, removed, or taking a transition) has the agents whose eventless conditions read it
//! ([`Chart::waits_on`]) tested again, in population, then serial order, then the environment;
//! a re-test that takes a transition changes its own population in turn. More than
//! [`DEFAULT_RETEST_LIMIT`] such changes after one event stop the run
//! ([`RunError::RetestLimit`]).
//!
//! **Stepped mode** ([`Stepping`], D18): every `every` time units, from `every` on, the
//! environment gets `before-step`, then every agent `before-step`, then every agent `step`,
//! then the environment `after-step`, each as one macrostep (and the re-tests that follow it).
//! The agents are those alive when the step begins, in population, then serial order, or in
//! one random order per step (the same for both rounds) drawn from the stream
//! `stepping/order`. For a synchronous update, an agent's `before-step` stores its next value
//! and its `step` copies it in.
//!
//! **Networks and grids** (stage A3, [`network`](crate::network)): a population's
//! [`Network`] is built before its first agents are created and its [`Grid`] places each agent
//! as it is created, so a distance network's position and a cell are in the agent's variables
//! (`x` and `y`, `row` and `column`, which its chart must declare) before its data's other first
//! values are worked out. Messages along links or to grid neighbours go to the agents alive then,
//! in serial order, each as its own event like the others; one sent by an agent that is not of
//! that population reaches no one and is counted as undelivered. Links made or removed and moves
//! ([`Action::connect`](crate::Action::connect), [`Action::move_to`](crate::Action::move_to),
//! ...) take effect when the acting agent's macrostep ends, in the order its actions ran (so in
//! the stepped mode an agent that moves sees the cell taken by an agent that moved before it in
//! the same step), and mark the population changed for the re-tests. An agent that leaves loses
//! its links and its cell once its last messages are sent.
//!
//! **Statistics** ([`Statistic`]) are recorded at the start, every `save_every`, and at the end,
//! each after every event at that time. [`bands`] turns many replications into the mean, median
//! and 5% to 95% band at each time.
//!
//! Choices this stage makes where the ROADMAP leaves them open: a message to other agents takes
//! no delay (an agent sends itself a delayed event and the message when it arrives); a broadcast
//! goes to every agent of the population but the sender, as a random pick never picks the
//! sender; only messages from agents are counted when no transition takes them (an agent's own
//! timeouts are not); the environment is one agent, started after the populations' first
//! agents, which messages cannot address; and a rate may not read a population or the time.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use stochcraft_des::EventQueue;
use stochcraft_model::expr::Value;
use stochcraft_random::{Rng, math};
use stochcraft_stats::Summary;
use stochcraft_stats::spread::quantile;

use crate::chart::{Chart, ChartError, Code, PopRef, RefWhat};
use crate::interp::{Agent, Out, Recipient, RunError, World};
use crate::network::{self, Grid, Net, Network, Space, pick};

/// The re-tests after one event that may take a transition before the run is stopped as
/// looping.
pub const DEFAULT_RETEST_LIMIT: u64 = 1_000_000;

/// A population: agents of one chart.
#[derive(Debug, Clone)]
pub struct Population {
    pub name: String,
    pub chart: Arc<Chart>,
    /// The agents created at the start.
    pub count: u64,
    /// Agents arriving at this rate (exponential gaps), from the start.
    pub arrival_rate: Option<f64>,
    /// The links between its agents ([`network`](crate::network)).
    pub network: Option<Network>,
    /// The grid its agents are on ([`network`](crate::network)).
    pub grid: Option<Grid>,
}

impl Population {
    pub fn new(name: &str, chart: Arc<Chart>) -> Self {
        Self {
            name: name.to_string(),
            chart,
            count: 0,
            arrival_rate: None,
            network: None,
            grid: None,
        }
    }

    pub fn count(mut self, n: u64) -> Self {
        self.count = n;
        self
    }

    pub fn arrivals(mut self, rate: f64) -> Self {
        self.arrival_rate = Some(rate);
        self
    }

    /// Links its agents by `network`, built at the start.
    pub fn network(mut self, network: Network) -> Self {
        self.network = Some(network);
        self
    }

    /// Puts its agents on `grid`.
    pub fn grid(mut self, grid: Grid) -> Self {
        self.grid = Some(grid);
        self
    }
}

/// What a [`Statistic`] computes over the agents that meet its condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    /// How many there are.
    Count,
    /// The sum of the expression (0 over none).
    Sum,
    /// The mean (NaN over none).
    Mean,
    /// The least value (NaN over none).
    Min,
    /// The greatest value (NaN over none).
    Max,
}

/// A statistic of one population over time.
#[derive(Debug, Clone, PartialEq)]
pub struct Statistic {
    pub name: String,
    pub population: String,
    pub measure: Measure,
    /// The expression over each agent (none for a count): its variables, `In()`, `serial()`
    /// and `time()`.
    pub expr: Option<String>,
    /// Only the agents for which this holds; all if `None`.
    pub condition: Option<String>,
}

impl Statistic {
    fn new(name: &str, population: &str, measure: Measure, expr: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            population: population.to_string(),
            measure,
            expr: expr.map(str::to_string),
            condition: None,
        }
    }

    pub fn count(name: &str, population: &str) -> Self {
        Self::new(name, population, Measure::Count, None)
    }

    pub fn sum(name: &str, population: &str, expr: &str) -> Self {
        Self::new(name, population, Measure::Sum, Some(expr))
    }

    pub fn mean(name: &str, population: &str, expr: &str) -> Self {
        Self::new(name, population, Measure::Mean, Some(expr))
    }

    pub fn min(name: &str, population: &str, expr: &str) -> Self {
        Self::new(name, population, Measure::Min, Some(expr))
    }

    pub fn max(name: &str, population: &str, expr: &str) -> Self {
        Self::new(name, population, Measure::Max, Some(expr))
    }

    /// Over the agents for which `condition` holds, such as `In('working')`.
    pub fn when(mut self, condition: &str) -> Self {
        self.condition = Some(condition.to_string());
        self
    }
}

/// The order agents take their turn in a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    /// Population, then serial order.
    #[default]
    Fixed,
    /// A random order each step, from the stream `stepping/order`.
    Random,
}

/// The stepped mode: a step every `every` time units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stepping {
    pub every: f64,
    pub order: Order,
}

impl Stepping {
    pub fn every(every: f64) -> Self {
        Self {
            every,
            order: Order::Fixed,
        }
    }

    pub fn random_order(mut self) -> Self {
        self.order = Order::Random;
        self
    }
}

/// Why a model cannot run, or why its run stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelError {
    /// The model is not valid: where, and what is wrong.
    Invalid(ChartError),
    Run(RunError),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::Invalid(e) => e.fmt(f),
            ModelError::Run(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ModelError {}

impl From<RunError> for ModelError {
    fn from(e: RunError) -> Self {
        ModelError::Run(e)
    }
}

fn invalid<T>(place: &str, message: impl Into<String>) -> Result<T, ModelError> {
    Err(ModelError::Invalid(ChartError {
        place: place.to_string(),
        message: message.into(),
    }))
}

/// Populations, an environment, statistics and an optional stepped mode: an agent model, with
/// no files or window (ROADMAP M4.3 A2).
#[derive(Debug, Clone)]
pub struct Model {
    pub populations: Vec<Population>,
    /// One agent outside the populations: it gets the stepped mode's `before-step` and
    /// `after-step`, and may create agents and message them.
    pub environment: Option<Arc<Chart>>,
    pub statistics: Vec<Statistic>,
    pub stepping: Option<Stepping>,
    /// The statistics are recorded at the start, every `save_every` (if set) and at the end.
    pub save_every: Option<f64>,
    pub microstep_limit: u64,
    pub retest_limit: u64,
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    pub fn new() -> Self {
        Self {
            populations: Vec::new(),
            environment: None,
            statistics: Vec::new(),
            stepping: None,
            save_every: None,
            microstep_limit: crate::DEFAULT_MICROSTEP_LIMIT,
            retest_limit: DEFAULT_RETEST_LIMIT,
        }
    }

    pub fn population(mut self, p: Population) -> Self {
        self.populations.push(p);
        self
    }

    pub fn environment(mut self, chart: Arc<Chart>) -> Self {
        self.environment = Some(chart);
        self
    }

    pub fn statistic(mut self, s: Statistic) -> Self {
        self.statistics.push(s);
        self
    }

    pub fn stepped(mut self, s: Stepping) -> Self {
        self.stepping = Some(s);
        self
    }

    pub fn save_every(mut self, every: f64) -> Self {
        self.save_every = Some(every);
        self
    }

    pub fn microstep_limit(mut self, limit: u64) -> Self {
        self.microstep_limit = limit;
        self
    }

    pub fn retest_limit(mut self, limit: u64) -> Self {
        self.retest_limit = limit;
        self
    }

    /// Runs one replication from `seed`, from time 0 to `end`, and returns its statistics and
    /// counts.
    pub fn run(&self, seed: u64, end: f64) -> Result<Run, ModelError> {
        let mut sim = Simulation::new(self, seed)?;
        sim.run_until(end)?;
        Ok(sim.into_run())
    }
}

/// One statistic over one replication.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub name: String,
    /// One value per recorded time ([`Run::times`]).
    pub values: Vec<f64>,
}

/// What happened to one population in a replication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counts {
    pub population: String,
    pub created: u64,
    pub removed: u64,
    /// Alive at the end.
    pub alive: u64,
    /// Messages to its agents that no transition took.
    pub dropped: u64,
    /// Messages to it that reached no agent: no agent of that serial (or it had gone), or no
    /// other agent to pick or broadcast to.
    pub undelivered: u64,
}

/// One replication's results.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub times: Vec<f64>,
    pub series: Vec<Series>,
    pub counts: Vec<Counts>,
}

/// One statistic across replications.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    pub name: String,
    /// At each time: the mean of the replications' finite values, and their median and 5th
    /// and 95th percentiles (`stochcraft_stats::spread::quantile`); NaN where none is finite.
    pub mean: Vec<f64>,
    pub median: Vec<f64>,
    pub low: Vec<f64>,
    pub high: Vec<f64>,
}

/// Every statistic across replications.
#[derive(Debug, Clone, PartialEq)]
pub struct Bands {
    pub times: Vec<f64>,
    pub series: Vec<Band>,
}

/// The mean and the 5% to 95% band of each statistic at each time, across replications of one
/// model (their times are the first run's; a run with fewer values counts only where it has
/// one).
pub fn bands(runs: &[Run]) -> Bands {
    let times = runs.first().map(|r| r.times.clone()).unwrap_or_default();
    let names: Vec<String> = runs
        .first()
        .map(|r| r.series.iter().map(|s| s.name.clone()).collect())
        .unwrap_or_default();
    let series = names
        .into_iter()
        .enumerate()
        .map(|(i, name)| {
            let mut b = Band {
                name,
                mean: Vec::with_capacity(times.len()),
                median: Vec::with_capacity(times.len()),
                low: Vec::with_capacity(times.len()),
                high: Vec::with_capacity(times.len()),
            };
            for k in 0..times.len() {
                let mut at: Vec<f64> = runs
                    .iter()
                    .filter_map(|r| r.series.get(i).and_then(|s| s.values.get(k)).copied())
                    .filter(|v| v.is_finite())
                    .collect();
                b.mean.push(Summary::of(&at).map_or(f64::NAN, |s| s.mean));
                at.sort_by(f64::total_cmp);
                b.median.push(quantile(&at, 0.5).unwrap_or(f64::NAN));
                b.low.push(quantile(&at, 0.05).unwrap_or(f64::NAN));
                b.high.push(quantile(&at, 0.95).unwrap_or(f64::NAN));
            }
            b
        })
        .collect();
    Bands { times, series }
}

/// Which agent: the environment, or a population's agent by its serial number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    Env,
    Agent(u32, u64),
}

#[derive(Debug, Clone)]
enum Ev {
    Fire(Who, u64),
    Message {
        to: (u32, u64),
        event: String,
        data: Option<f64>,
    },
    Create(u32),
    Remove(u32, u64),
    Arrival(u32),
    Step,
}

#[derive(Debug, Clone, Copy)]
enum Op<'a> {
    Start,
    Fire(u64),
    Deliver(&'a str, Option<f64>),
    Retest,
    Stop,
}

#[derive(Debug)]
struct Pop {
    name: String,
    chart: Arc<Chart>,
    /// By serial number; `None` once gone (or while it runs).
    agents: Vec<Option<Agent>>,
    /// The serial numbers alive, ascending.
    live: Vec<u64>,
    arrivals: Option<(f64, Rng)>,
    counts: Counts,
    net: Option<Net>,
    space: Option<Space>,
}

impl Pop {
    fn is_live(&self, s: u64) -> bool {
        self.live.binary_search(&s).is_ok()
    }

    /// The live agents linked to `s`, ascending.
    fn linked(&self, s: u64) -> Vec<u64> {
        self.net
            .as_ref()
            .map(|n| n.linked(s))
            .unwrap_or_default()
            .into_iter()
            .filter(|&x| self.is_live(x))
            .collect()
    }

    /// The agents in `s`'s grid neighbourhood, ascending.
    fn neighbours(&self, s: u64) -> Vec<u64> {
        self.space
            .as_ref()
            .map(|g| g.neighbours(s))
            .unwrap_or_default()
    }
}

#[derive(Debug)]
struct Stat {
    population: u32,
    measure: Measure,
    expr: Option<Code>,
    condition: Option<Code>,
}

/// What an agent's expressions see of the populations while it runs: `me` is the agent
/// running, taken out of its population's slot meanwhile.
struct View<'a> {
    pops: &'a [Pop],
    who: Option<Who>,
}

impl View<'_> {
    fn agent<'b>(&'b self, p: u32, s: u64, me: &'b Agent) -> Option<&'b Agent> {
        if self.who == Some(Who::Agent(p, s)) {
            return Some(me);
        }
        self.pops
            .get(p as usize)
            .and_then(|pop| pop.agents.get(index(s)?))
            .and_then(Option::as_ref)
    }
}

impl World for View<'_> {
    fn call(&self, name: &str, args: &[Value], me: &Agent) -> Value {
        let empty = Value::Text(String::new());
        if name == "serial" {
            return match self.who {
                Some(Who::Agent(_, s)) => Value::Number(s as f64),
                _ => Value::Number(f64::NAN),
            };
        }
        let text = |i: usize| args.get(i).map(Value::text).unwrap_or_default();
        let pop_name = text(0);
        let Some(p) = self.pops.iter().position(|q| q.name == pop_name) else {
            return empty;
        };
        let p = p as u32;
        let Some(pop) = self.pops.get(p as usize) else {
            return empty;
        };
        let agents = || pop.live.iter().filter_map(move |&s| self.agent(p, s, me));
        if crate::data::NEIGHBOURHOOD_FUNCTIONS.contains(&name) {
            let nan = Value::Number(f64::NAN);
            let Some(Who::Agent(q, s)) = self.who else {
                return nan;
            };
            if q != p {
                return nan;
            }
            let grid = name.starts_with("neighbour");
            if grid && pop.space.as_ref().and_then(|g| g.cell_of(s)).is_none() {
                return nan;
            }
            let others = if grid {
                pop.neighbours(s)
            } else {
                pop.linked(s)
            };
            let others = others.into_iter().filter_map(|x| self.agent(p, x, me));
            return match name {
                "degree" | "neighbours" => Value::Number(others.count() as f64),
                "linked-count" | "neighbour-count" => {
                    let state = text(1);
                    Value::Number(others.filter(|a| a.is_in(&state)).count() as f64)
                }
                _ => {
                    let var = text(1);
                    Value::Number(
                        others
                            .map(|a| a.value(&var).map_or(f64::NAN, Value::number))
                            .sum(),
                    )
                }
            };
        }
        match name {
            "population-size" => Value::Number(pop.live.len() as f64),
            "population-count" => {
                let state = text(1);
                Value::Number(agents().filter(|a| a.is_in(&state)).count() as f64)
            }
            "population-sum" | "population-mean" => {
                let var = text(1);
                let (mut n, mut sum) = (0u64, 0.0);
                for a in agents() {
                    n += 1;
                    sum += a.value(&var).map_or(f64::NAN, Value::number);
                }
                if name == "population-sum" {
                    Value::Number(sum)
                } else if n == 0 {
                    Value::Number(f64::NAN)
                } else {
                    Value::Number(sum / n as f64)
                }
            }
            "agent-value" => {
                let serial = args.get(1).map_or(f64::NAN, Value::number);
                let var = text(2);
                whole(serial)
                    .and_then(|s| self.agent(p, s, me))
                    .and_then(|a| a.value(&var).cloned())
                    .unwrap_or(empty)
            }
            _ => empty,
        }
    }
}

/// A serial number: a whole number of 0 or more that a u64 holds.
fn whole(v: f64) -> Option<u64> {
    (v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v < 18_446_744_073_709_551_616.0)
        .then_some(v as u64)
}

/// A serial number as an index, if this target's `usize` holds it (on wasm32 one of 2^32 or
/// more is no agent rather than another agent's index).
fn index(serial: u64) -> Option<usize> {
    usize::try_from(serial).ok()
}

/// All of `others`, or with a draw the one it picks (none if there are none).
fn one_or_all(others: Vec<u64>, draw: Option<u64>) -> Vec<u64> {
    match draw {
        None => others,
        Some(d) => others
            .get(pick(d, others.len()))
            .copied()
            .into_iter()
            .collect(),
    }
}

fn check_name(name: &str, place: &str) -> Result<(), ModelError> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        invalid(
            place,
            "a population's name is letters, digits, `_` and `-`, and not empty",
        )
    }
}

/// One replication of a [`Model`] running: its populations, environment, clock and statistics.
pub struct Simulation {
    pops: Vec<Pop>,
    env: Option<Agent>,
    stats: Vec<Stat>,
    stat_names: Vec<String>,
    /// For each population, the populations with an agent type that waits on it.
    readers: Vec<Vec<u32>>,
    env_reads: Vec<bool>,
    dirty: BTreeSet<u32>,
    queue: EventQueue<Ev>,
    seed: u64,
    stepping: Option<(Stepping, Rng)>,
    save_every: Option<f64>,
    saved: u64,
    times: Vec<f64>,
    values: Vec<Vec<f64>>,
    microstep_limit: u64,
    retest_limit: u64,
}

impl fmt::Debug for Simulation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Simulation")
            .field("now", &self.queue.now())
            .field("queued", &self.queue.len())
            .field("counts", &self.counts())
            .field("times", &self.times)
            .finish()
    }
}

impl Simulation {
    /// Checks `model` and starts a replication from `seed` at time 0: each population's first
    /// agents are created and started in population, then serial order, then the environment;
    /// then the first arrivals and step are put on the clock.
    pub fn new(model: &Model, seed: u64) -> Result<Self, ModelError> {
        let names: Vec<&str> = model.populations.iter().map(|p| p.name.as_str()).collect();
        for (i, p) in model.populations.iter().enumerate() {
            let place = format!("population `{}`", p.name);
            check_name(&p.name, &place)?;
            if names[..i].contains(&p.name.as_str()) {
                return invalid(&place, "another population has this name");
            }
            if let Some(r) = p.arrival_rate
                && !(r.is_finite() && r > 0.0)
            {
                return invalid(&place, format!("the arrival rate {r} is not above 0"));
            }
        }
        let find = |n: &str| model.populations.iter().position(|p| p.name == n);
        let check_refs = |refs: &[PopRef]| -> Result<(), ModelError> {
            for r in refs {
                let Some(q) = find(&r.population).and_then(|q| model.populations.get(q)) else {
                    return invalid(
                        &r.place,
                        format!("there is no population `{}`", r.population),
                    );
                };
                match &r.what {
                    RefWhat::Population => {}
                    RefWhat::State(s) => {
                        if !q.chart.ids.contains_key(s) {
                            return invalid(
                                &r.place,
                                format!("population `{}` has no state `{s}`", q.name),
                            );
                        }
                    }
                    RefWhat::Variable(v) => {
                        if !q.chart.var_index.contains_key(v) {
                            return invalid(
                                &r.place,
                                format!("population `{}` has no variable `{v}`", q.name),
                            );
                        }
                    }
                    RefWhat::Network => {
                        if q.network.is_none() {
                            return invalid(
                                &r.place,
                                format!("population `{}` has no network", q.name),
                            );
                        }
                    }
                    RefWhat::Grid => {
                        if q.grid.is_none() {
                            return invalid(
                                &r.place,
                                format!("population `{}` has no grid", q.name),
                            );
                        }
                    }
                }
            }
            Ok(())
        };
        for p in &model.populations {
            check_refs(&p.chart.refs)?;
            let declares = |vars: [&str; 2], what: &str| {
                if vars.iter().all(|v| p.chart.var_index.contains_key(*v)) {
                    Ok(())
                } else {
                    invalid(
                        &format!("population `{}`", p.name),
                        format!(
                            "its agents' {what} is kept in the variables `{}` and `{}`, which \
                             its chart `{}` must declare",
                            vars[0], vars[1], p.chart.name
                        ),
                    )
                }
            };
            if matches!(p.network, Some(Network::Distance { .. })) {
                declares(["x", "y"], "position")?;
            }
            if p.grid.is_some() {
                declares(["row", "column"], "cell")?;
            }
        }
        if let Some(env) = &model.environment {
            check_refs(&env.refs)?;
        }
        if let Some(s) = model.stepping
            && !(s.every.is_finite() && s.every > 0.0)
        {
            return invalid(
                "stepping",
                format!("a step every {} is not above 0", s.every),
            );
        }
        if let Some(e) = model.save_every
            && !(e.is_finite() && e > 0.0)
        {
            return invalid("save_every", format!("saving every {e} is not above 0"));
        }
        let mut stats = Vec::new();
        let mut stat_names: Vec<String> = Vec::new();
        for s in &model.statistics {
            let place = format!("statistic `{}`", s.name);
            if stat_names.contains(&s.name) {
                return invalid(&place, "another statistic has this name");
            }
            let Some(p) = find(&s.population) else {
                return invalid(&place, format!("there is no population `{}`", s.population));
            };
            let chart = &model.populations[p].chart;
            let compile = |e: &str| -> Result<Code, ModelError> {
                let (code, refs) = chart
                    .agent_expression(e, &place)
                    .map_err(ModelError::Invalid)?;
                check_refs(&refs)?;
                Ok(code)
            };
            let expr = match (&s.expr, s.measure) {
                (None, Measure::Count) => None,
                (Some(e), m) if m != Measure::Count => Some(compile(e)?),
                _ => {
                    return invalid(&place, "a count has no expression; the others have one");
                }
            };
            let condition = match &s.condition {
                Some(c) => Some(compile(c)?),
                None => None,
            };
            stat_names.push(s.name.clone());
            stats.push(Stat {
                population: p as u32,
                measure: s.measure,
                expr,
                condition,
            });
        }
        let readers: Vec<Vec<u32>> = model
            .populations
            .iter()
            .map(|p| {
                (0..model.populations.len() as u32)
                    .filter(|&q| {
                        model.populations[q as usize]
                            .chart
                            .waits_on
                            .contains(&p.name)
                    })
                    .collect()
            })
            .collect();
        let env_reads = model
            .populations
            .iter()
            .map(|p| {
                model
                    .environment
                    .as_ref()
                    .is_some_and(|e| e.waits_on.contains(&p.name))
            })
            .collect();
        let mut pops = Vec::new();
        for p in &model.populations {
            let place = format!("population `{}`", p.name);
            let net = match &p.network {
                Some(n) => {
                    let key = format!("network/{}:{}", p.name.len(), p.name);
                    let net = Net::new(n, p.count, Rng::keyed(seed, &key))
                        .or_else(|m| invalid(&format!("the network of {place}"), m))?;
                    Some(net)
                }
                None => None,
            };
            let space = match p.grid {
                Some(g) => {
                    let key = format!("grid/{}:{}", p.name.len(), p.name);
                    let space = Space::new(g, p.count, Rng::keyed(seed, &key))
                        .or_else(|m| invalid(&format!("the grid of {place}"), m))?;
                    Some(space)
                }
                None => None,
            };
            pops.push(Pop {
                net,
                space,
                name: p.name.clone(),
                chart: Arc::clone(&p.chart),
                agents: Vec::new(),
                live: Vec::new(),
                arrivals: p.arrival_rate.map(|r| {
                    let key = format!("arrivals/{}:{}", p.name.len(), p.name);
                    (r, Rng::keyed(seed, &key))
                }),
                counts: Counts {
                    population: p.name.clone(),
                    created: 0,
                    removed: 0,
                    alive: 0,
                    dropped: 0,
                    undelivered: 0,
                },
            });
        }
        let mut sim = Simulation {
            pops,
            env: None,
            values: vec![Vec::new(); stats.len()],
            stats,
            stat_names,
            readers,
            env_reads,
            dirty: BTreeSet::new(),
            queue: EventQueue::new(0.0),
            seed,
            stepping: model
                .stepping
                .map(|s| (s, Rng::keyed(seed, "stepping/order"))),
            save_every: model.save_every,
            saved: 0,
            times: Vec::new(),
            microstep_limit: model.microstep_limit,
            retest_limit: model.retest_limit,
        };
        for (p, pop) in model.populations.iter().enumerate() {
            for _ in 0..pop.count {
                sim.create(p as u32)?;
                sim.propagate()?;
            }
        }
        if let Some(env) = &model.environment {
            let agent = Agent::new(Arc::clone(env), seed, "environment")
                .with_microstep_limit(model.microstep_limit);
            sim.env = Some(agent);
            sim.act(Who::Env, Op::Start)?;
            sim.propagate()?;
        }
        for p in 0..sim.pops.len() as u32 {
            sim.schedule_arrival(p)?;
        }
        if let Some((s, _)) = &sim.stepping {
            let every = s.every;
            sim.schedule(every, Ev::Step, "stepping")?;
        }
        Ok(sim)
    }

    /// The current time.
    pub fn now(&self) -> f64 {
        self.queue.now()
    }

    /// The agent `serial` of `population`, if it is alive.
    pub fn agent(&self, population: &str, serial: u64) -> Option<&Agent> {
        self.pops
            .iter()
            .find(|p| p.name == population)
            .and_then(|p| p.agents.get(index(serial)?))
            .and_then(Option::as_ref)
    }

    /// The serial numbers alive in `population`, ascending.
    pub fn alive(&self, population: &str) -> &[u64] {
        self.pops
            .iter()
            .find(|p| p.name == population)
            .map_or(&[], |p| p.live.as_slice())
    }

    pub fn environment(&self) -> Option<&Agent> {
        self.env.as_ref()
    }

    fn pop(&self, population: &str) -> Option<&Pop> {
        self.pops.iter().find(|p| p.name == population)
    }

    /// Every link of `population`'s network now, the lower serial first, sorted (empty if it
    /// has no network).
    pub fn links(&self, population: &str) -> Vec<(u64, u64)> {
        self.pop(population)
            .and_then(|p| p.net.as_ref())
            .map(Net::links)
            .unwrap_or_default()
    }

    /// The agents linked to agent `serial` of `population`, ascending.
    pub fn linked(&self, population: &str, serial: u64) -> Vec<u64> {
        self.pop(population)
            .map(|p| p.linked(serial))
            .unwrap_or_default()
    }

    /// The position of agent `serial` in `population`'s distance network.
    pub fn position(&self, population: &str, serial: u64) -> Option<(f64, f64)> {
        self.pop(population)
            .and_then(|p| p.net.as_ref())
            .and_then(|n| n.positions.get(&serial).copied())
    }

    /// The cell (row, column) of agent `serial` on `population`'s grid.
    pub fn cell(&self, population: &str, serial: u64) -> Option<(u64, u64)> {
        let space = self.pop(population)?.space.as_ref()?;
        space.cell_of(serial).map(|c| space.row_column(c))
    }

    /// The agents in the grid neighbourhood of agent `serial` of `population`, ascending.
    pub fn neighbours(&self, population: &str, serial: u64) -> Vec<u64> {
        self.pop(population)
            .map(|p| p.neighbours(serial))
            .unwrap_or_default()
    }

    /// Each population's counts so far.
    pub fn counts(&self) -> Vec<Counts> {
        self.pops
            .iter()
            .map(|p| Counts {
                alive: p.live.len() as u64,
                ..p.counts.clone()
            })
            .collect()
    }

    /// Handles every event due no later than `end`, recording the statistics at each saving
    /// time on the way (after every event at that time), then moves the clock to `end`.
    pub fn run_until(&mut self, end: f64) -> Result<(), RunError> {
        loop {
            let next = self.next_save();
            let horizon = match next {
                Some(t) if t <= end => t,
                _ => end,
            };
            while self.queue.peek_time().is_some_and(|t| t <= horizon) {
                let Some(ev) = self.queue.pop() else { break };
                self.handle(ev.payload)?;
            }
            self.queue.advance_to(horizon);
            if next.is_some_and(|t| t <= end) {
                self.record(horizon);
                self.saved += 1;
            } else {
                return Ok(());
            }
        }
    }

    /// The results: the statistics recorded so far, and once more now if the last record was
    /// earlier.
    pub fn into_run(mut self) -> Run {
        let now = self.queue.now();
        if self.times.last().is_none_or(|&t| t < now) {
            self.record(now);
        }
        let counts = self.counts();
        let series = self
            .stat_names
            .into_iter()
            .zip(self.values)
            .map(|(name, values)| Series { name, values })
            .collect();
        Run {
            times: self.times,
            series,
            counts,
        }
    }

    fn next_save(&self) -> Option<f64> {
        if self.saved == 0 {
            return Some(0.0);
        }
        self.save_every.map(|e| self.saved as f64 * e)
    }

    fn record(&mut self, t: f64) {
        self.times.push(t);
        for (i, st) in self.stats.iter().enumerate() {
            let Some(pop) = self.pops.get(st.population as usize) else {
                continue;
            };
            let (mut n, mut sum, mut lo, mut hi) = (0u64, 0.0, f64::INFINITY, f64::NEG_INFINITY);
            for &s in &pop.live {
                let Some(a) = index(s)
                    .and_then(|i| pop.agents.get(i))
                    .and_then(Option::as_ref)
                else {
                    continue;
                };
                let view = View {
                    pops: &self.pops,
                    who: Some(Who::Agent(st.population, s)),
                };
                if let Some(c) = &st.condition
                    && !a.evaluate(c, t, &view).boolean()
                {
                    continue;
                }
                n += 1;
                if let Some(e) = &st.expr {
                    let x = a.evaluate(e, t, &view).number();
                    sum += x;
                    lo = lo.min(x);
                    hi = hi.max(x);
                }
            }
            let v = match st.measure {
                Measure::Count => n as f64,
                Measure::Sum => sum,
                _ if n == 0 => f64::NAN,
                Measure::Mean => sum / n as f64,
                Measure::Min => lo,
                Measure::Max => hi,
            };
            if let Some(col) = self.values.get_mut(i) {
                col.push(v);
            }
        }
    }

    fn key(&self, who: Who) -> String {
        match who {
            Who::Env => "environment".to_string(),
            Who::Agent(p, s) => format!(
                "{}/{s}",
                self.pops.get(p as usize).map_or("", |q| q.name.as_str())
            ),
        }
    }

    fn schedule(&mut self, delay: f64, ev: Ev, who: &str) -> Result<(), RunError> {
        let now = self.queue.now();
        self.queue
            .schedule_in(delay, 0, ev)
            .map(|_| ())
            .map_err(|_| RunError::Schedule {
                agent: who.to_string(),
                time: now,
                delay,
            })
    }

    fn schedule_arrival(&mut self, p: u32) -> Result<(), RunError> {
        let Some(pop) = self.pops.get_mut(p as usize) else {
            return Ok(());
        };
        let Some((rate, rng)) = &mut pop.arrivals else {
            return Ok(());
        };
        let gap = (0.0 - math::ln(rng.uniform_open0())) / *rate;
        let who = format!("arrivals of {}", pop.name);
        self.schedule(gap, Ev::Arrival(p), &who)
    }

    /// Creates the next agent of population `p` and starts it now.
    fn create(&mut self, p: u32) -> Result<(), RunError> {
        let limit = self.microstep_limit;
        let seed = self.seed;
        let Some(pop) = self.pops.get_mut(p as usize) else {
            return Ok(());
        };
        let serial = pop.agents.len() as u64;
        let key = format!("{}/{serial}", pop.name);
        let mut agent = Agent::new(Arc::clone(&pop.chart), seed, key).with_microstep_limit(limit);
        if let Some(net) = &mut pop.net
            && let (Some(range), Some((w, h))) = (net.range, net.size)
        {
            let at = match net.positions.get(&serial) {
                Some(&at) => at,
                None => {
                    // Created during the run: a position of its own, linked within the range.
                    let at = (net.rng.uniform() * w, net.rng.uniform() * h);
                    let near: Vec<u64> = pop
                        .live
                        .iter()
                        .copied()
                        .filter(|o| {
                            net.positions
                                .get(o)
                                .is_some_and(|&q| network::within(at, q, range))
                        })
                        .collect();
                    for o in near {
                        net.connect(serial, o);
                    }
                    net.positions.insert(serial, at);
                    at
                }
            };
            agent.preset("x", Value::Number(at.0));
            agent.preset("y", Value::Number(at.1));
        }
        if let Some(space) = &mut pop.space {
            let (row, column) = match space.choose(serial) {
                Some(cell) => {
                    space.place(serial, cell);
                    let (r, c) = space.row_column(cell);
                    (r as f64, c as f64)
                }
                None => (-1.0, -1.0),
            };
            agent.preset("row", Value::Number(row));
            agent.preset("column", Value::Number(column));
        }
        pop.agents.push(Some(agent));
        pop.live.push(serial);
        pop.counts.created += 1;
        self.act(Who::Agent(p, serial), Op::Start)?;
        Ok(())
    }

    /// Runs one macrostep of agent `who` (if it is alive), then sends out what it sent, takes
    /// it out of its population if its chart ended, and marks its population changed if it
    /// was created, ended or took a transition. Returns whether a transition took the event
    /// (for [`Op::Deliver`]), or `None` if there is no such agent.
    fn act(&mut self, who: Who, op: Op) -> Result<Option<bool>, RunError> {
        let now = self.queue.now();
        let taken = match who {
            Who::Env => self.env.take(),
            Who::Agent(p, s) => self
                .pops
                .get_mut(p as usize)
                .and_then(|pop| pop.agents.get_mut(index(s)?))
                .and_then(Option::take),
        };
        let Some(mut agent) = taken else {
            return Ok(None);
        };
        let result = {
            let view = View {
                pops: &self.pops,
                who: Some(who),
            };
            match op {
                Op::Start => agent.start_in(now, &view).map(|()| (true, true)),
                Op::Fire(token) => agent
                    .fire_in(token, now, &view)
                    .map(|fired| (fired, fired && agent.took_steps())),
                Op::Deliver(event, data) => agent
                    .handle_in(event, data, now, &view)
                    .map(|taken| (taken, agent.took_steps())),
                Op::Retest => agent.retest(now, &view).map(|changed| (changed, changed)),
                Op::Stop => {
                    agent.stop(now, &view);
                    Ok((true, true))
                }
            }
        };
        let out = agent.take_out();
        let ended = !agent.is_running();
        match who {
            Who::Env => {
                if !ended {
                    self.env = Some(agent);
                }
            }
            Who::Agent(p, s) => {
                if let Some(pop) = self.pops.get_mut(p as usize) {
                    if ended {
                        if let Ok(i) = pop.live.binary_search(&s) {
                            pop.live.remove(i);
                        }
                        pop.counts.removed += 1;
                    } else if let Some(slot) = index(s).and_then(|i| pop.agents.get_mut(i)) {
                        *slot = Some(agent);
                    }
                }
            }
        }
        let (matched, changed) = result?;
        self.dispatch(who, out)?;
        if let (Who::Agent(p, s), true) = (who, ended)
            && let Some(pop) = self.pops.get_mut(p as usize)
        {
            if let Some(net) = &mut pop.net {
                net.forget(s);
            }
            if let Some(space) = &mut pop.space {
                space.remove(s);
            }
        }
        if let Who::Agent(p, _) = who
            && (changed || ended)
        {
            self.dirty.insert(p);
        }
        Ok(Some(matched))
    }

    /// Puts what an agent sent out on the clock, in order.
    fn dispatch(&mut self, who: Who, out: Vec<Out>) -> Result<(), RunError> {
        let find = |pops: &[Pop], n: &str| pops.iter().position(|p| p.name == n).map(|p| p as u32);
        for o in out {
            match o {
                Out::Own(s) => {
                    let key = self.key(who);
                    if let Err(e) = self.schedule(s.delay, Ev::Fire(who, s.token), &key) {
                        self.halt(who, "an event it sent would not be at a finite time");
                        return Err(e);
                    }
                }
                Out::Message { to, event, data } => {
                    let population = match &to {
                        Recipient::One { population, .. }
                        | Recipient::Random { population, .. }
                        | Recipient::All { population }
                        | Recipient::Linked { population, .. }
                        | Recipient::Neighbours { population, .. } => population,
                    };
                    let Some(p) = find(&self.pops, population) else {
                        continue;
                    };
                    let sender = match who {
                        Who::Agent(q, s) if q == p => Some(s),
                        _ => None,
                    };
                    let Some(pop) = self.pops.get(p as usize) else {
                        continue;
                    };
                    let recipients: Vec<u64> = match to {
                        Recipient::One { serial, .. } => whole(serial)
                            .filter(|s| pop.live.binary_search(s).is_ok())
                            .into_iter()
                            .collect(),
                        Recipient::Random { draw, .. } => {
                            let at = sender.and_then(|s| pop.live.binary_search(&s).ok());
                            let m = pop.live.len() - usize::from(at.is_some());
                            if m == 0 {
                                Vec::new()
                            } else {
                                let mut k = pick(draw, m);
                                if at.is_some_and(|i| k >= i) {
                                    k += 1;
                                }
                                pop.live.get(k).copied().into_iter().collect()
                            }
                        }
                        Recipient::All { .. } => pop
                            .live
                            .iter()
                            .copied()
                            .filter(|&s| Some(s) != sender)
                            .collect(),
                        Recipient::Linked { draw, .. } => {
                            let others = sender.map(|s| pop.linked(s)).unwrap_or_default();
                            one_or_all(others, draw)
                        }
                        Recipient::Neighbours { draw, .. } => {
                            let others = sender.map(|s| pop.neighbours(s)).unwrap_or_default();
                            one_or_all(others, draw)
                        }
                    };
                    if recipients.is_empty() {
                        if let Some(pop) = self.pops.get_mut(p as usize) {
                            pop.counts.undelivered += 1;
                        }
                        continue;
                    }
                    let key = self.key(who);
                    for s in recipients {
                        let ev = Ev::Message {
                            to: (p, s),
                            event: event.clone(),
                            data,
                        };
                        self.schedule(0.0, ev, &key)?;
                    }
                }
                Out::Create(population) => {
                    if let Some(p) = find(&self.pops, &population) {
                        let key = self.key(who);
                        self.schedule(0.0, Ev::Create(p), &key)?;
                    }
                }
                Out::Remove { population, serial } => {
                    if let (Some(p), Some(s)) = (find(&self.pops, &population), whole(serial)) {
                        let key = self.key(who);
                        self.schedule(0.0, Ev::Remove(p, s), &key)?;
                    }
                }
                Out::Connect { population, serial } => self.link(who, &population, serial, true),
                Out::Disconnect { population, serial } => {
                    self.link(who, &population, serial, false);
                }
                Out::MoveTo {
                    population,
                    row,
                    column,
                } => {
                    if let Some((p, s)) = self.actor(who, &population) {
                        let cell = self.pops.get(p as usize).and_then(|pop| {
                            let space = pop.space.as_ref()?;
                            let cell = space.cell_at(row, column)?;
                            let here = space.cell_of(s) == Some(cell);
                            (here || space.can_enter(cell)).then_some(cell)
                        });
                        if let Some(cell) = cell {
                            self.move_agent(p, s, cell);
                        }
                    }
                }
                Out::MoveRandom { population, draw } => {
                    if let Some((p, s)) = self.actor(who, &population) {
                        let cell = self.pops.get(p as usize).and_then(|pop| {
                            let space = pop.space.as_ref()?;
                            let free: Vec<u64> = space
                                .neighbour_cells(space.cell_of(s)?)
                                .into_iter()
                                .filter(|&c| space.can_enter(c))
                                .collect();
                            free.get(pick(draw, free.len())).copied()
                        });
                        if let Some(cell) = cell {
                            self.move_agent(p, s, cell);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// The acting agent as one of `population`, if it is and is alive.
    fn actor(&self, who: Who, population: &str) -> Option<(u32, u64)> {
        let Who::Agent(p, s) = who else { return None };
        let pop = self.pops.get(p as usize)?;
        (pop.name == population && pop.is_live(s)).then_some((p, s))
    }

    /// Links (or unlinks) the acting agent and agent `serial` of `population`.
    fn link(&mut self, who: Who, population: &str, serial: f64, connect: bool) {
        let Some((p, s)) = self.actor(who, population) else {
            return;
        };
        let Some(pop) = self.pops.get_mut(p as usize) else {
            return;
        };
        let Some(other) = whole(serial).filter(|&o| pop.is_live(o)) else {
            return;
        };
        let Some(net) = &mut pop.net else { return };
        let changed = if connect {
            net.connect(s, other)
        } else {
            net.disconnect(s, other)
        };
        if changed {
            self.dirty.insert(p);
        }
    }

    /// Moves agent `s` of population `p` to `cell`, and writes its `row` and `column`.
    fn move_agent(&mut self, p: u32, s: u64, cell: u64) {
        let Some(pop) = self.pops.get_mut(p as usize) else {
            return;
        };
        let Some(space) = &mut pop.space else { return };
        if space.cell_of(s) == Some(cell) {
            return;
        }
        space.place(s, cell);
        let (r, c) = space.row_column(cell);
        if let Some(a) = index(s)
            .and_then(|i| pop.agents.get_mut(i))
            .and_then(Option::as_mut)
        {
            a.set_value("row", Value::Number(r as f64));
            a.set_value("column", Value::Number(c as f64));
        }
        self.dirty.insert(p);
    }

    /// Stops an agent after a scheduling error, as [`Agent`]'s own driver does.
    fn halt(&mut self, who: Who, reason: &str) {
        let agent = match who {
            Who::Env => self.env.as_mut(),
            Who::Agent(p, s) => self
                .pops
                .get_mut(p as usize)
                .and_then(|pop| pop.agents.get_mut(index(s)?))
                .and_then(Option::as_mut),
        };
        if let Some(a) = agent {
            a.halt(reason.to_string());
        }
    }

    fn handle(&mut self, ev: Ev) -> Result<(), RunError> {
        match ev {
            Ev::Fire(who, token) => {
                self.act(who, Op::Fire(token))?;
            }
            Ev::Message { to, event, data } => {
                let (p, s) = to;
                match self.act(Who::Agent(p, s), Op::Deliver(&event, data))? {
                    Some(true) => {}
                    Some(false) => self.count(p, |c| c.dropped += 1),
                    None => self.count(p, |c| c.undelivered += 1),
                }
            }
            Ev::Create(p) => self.create(p)?,
            Ev::Remove(p, s) => {
                self.act(Who::Agent(p, s), Op::Stop)?;
            }
            Ev::Arrival(p) => {
                self.create(p)?;
                self.propagate()?;
                self.schedule_arrival(p)?;
            }
            Ev::Step => {
                self.step()?;
                if let Some((s, _)) = &self.stepping {
                    let every = s.every;
                    self.schedule(every, Ev::Step, "stepping")?;
                }
            }
        }
        self.propagate()
    }

    fn count(&mut self, p: u32, f: impl FnOnce(&mut Counts)) {
        if let Some(pop) = self.pops.get_mut(p as usize) {
            f(&mut pop.counts);
        }
    }

    /// One step of the stepped mode.
    fn step(&mut self) -> Result<(), RunError> {
        let mut order: Vec<(u32, u64)> = self
            .pops
            .iter()
            .enumerate()
            .flat_map(|(p, pop)| pop.live.iter().map(move |&s| (p as u32, s)))
            .collect();
        if let Some((s, rng)) = &mut self.stepping
            && s.order == Order::Random
        {
            // Fisher–Yates.
            for i in (1..order.len()).rev() {
                let j = pick(rng.next_u64(), i + 1);
                order.swap(i, j);
            }
        }
        self.act(Who::Env, Op::Deliver("before-step", None))?;
        self.propagate()?;
        for event in ["before-step", "step"] {
            for &(p, s) in &order {
                self.act(Who::Agent(p, s), Op::Deliver(event, None))?;
                self.propagate()?;
            }
        }
        self.act(Who::Env, Op::Deliver("after-step", None))?;
        Ok(())
    }

    /// Tests again the agents waiting on each changed population, in population, then serial
    /// order, then the environment, until no re-test changes anything.
    fn propagate(&mut self) -> Result<(), RunError> {
        let mut changes = 0u64;
        while let Some(p) = self.dirty.pop_first() {
            let readers = self.readers.get(p as usize).cloned().unwrap_or_default();
            let mut who: Vec<Who> = Vec::new();
            for q in readers {
                if let Some(pop) = self.pops.get(q as usize) {
                    who.extend(pop.live.iter().map(|&s| Who::Agent(q, s)));
                }
            }
            if self.env_reads.get(p as usize).copied().unwrap_or(false) {
                who.push(Who::Env);
            }
            for w in who {
                if self.act(w, Op::Retest)? == Some(true) {
                    changes += 1;
                    if changes > self.retest_limit {
                        return Err(RunError::RetestLimit {
                            time: self.queue.now(),
                            limit: self.retest_limit,
                        });
                    }
                }
            }
        }
        Ok(())
    }
}
