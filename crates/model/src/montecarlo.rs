//! A Monte Carlo model: a calculation, not a process (ROADMAP M4.1, decision D15).
//!
//! Inputs are distributions (a cost, a duration, a demand) and outputs are BPSim expressions
//! over them (a total cost, a finish date, a profit). Each trial draws every input once and
//! evaluates every output in order. There is no clock and no queue. This module only holds the
//! model; `stochcraft-mc` checks and runs it.
//!
//! In a file it is a BPMN file with no process: the inputs are the scenario's own properties
//! (`ScenarioParameters/PropertyParameters`, each a BPSim distribution) and the rest is the
//! `stochcraft:monteCarlo` vendor extension on the scenario. The number of trials is the
//! scenario's `replication`, and the seed its `seed` ([`crate::Experiment`]).

use stochcraft_random::Dist;

/// The trials a model runs when it becomes a Monte Carlo model ([`crate::Experiment`]'s
/// `replications`).
pub const DEFAULT_TRIALS: u32 = 10_000;

/// A Monte Carlo model's inputs, outputs and how they are sampled.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MonteCarlo {
    pub inputs: Vec<McInput>,
    /// Evaluated in this order; each may use the inputs and the outputs before it.
    pub outputs: Vec<McOutput>,
    /// Rank correlations between pairs of inputs; pairs not listed are independent.
    pub correlations: Vec<McCorrelation>,
    pub sampling: Sampling,
    /// Run until the outputs' means are precise enough, instead of a fixed number of trials.
    pub stop: Option<McStop>,
}

/// An uncertain input.
#[derive(Debug, Clone, PartialEq)]
pub struct McInput {
    /// Letters, digits and `_`, starting with a letter or `_`; unique among inputs and outputs.
    pub name: String,
    /// What a trial draws. A constant is its value as given, negative or not.
    pub dist: Dist,
    /// Free text (`h`, `USD`); empty for none.
    pub unit: String,
}

/// A result each trial computes.
#[derive(Debug, Clone, PartialEq)]
pub struct McOutput {
    /// As an input's.
    pub name: String,
    /// A BPSim expression (XPath 1.0 subset, `crate::expr`) over the inputs and earlier outputs,
    /// each named as `$name` or `getProperty('name')`.
    pub expression: String,
    pub unit: String,
    /// A target whose probability of being met is reported, with its interval.
    pub target: Option<McTarget>,
    /// Control variates: quantities with a known mean whose deviation from it is used to
    /// narrow this output's interval (Law, *Simulation Modeling and Analysis*, 5th ed., §11.3).
    pub controls: Vec<Control>,
}

/// A control variate for an output.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// An input, whose known mean is its distribution's exact mean.
    Input { name: String },
    /// An earlier output whose mean the modeller knows and types.
    Output { name: String, expected: f64 },
}

impl Control {
    /// The input or output it names.
    pub fn name(&self) -> &str {
        match self {
            Control::Input { name } | Control::Output { name, .. } => name,
        }
    }
}

/// A target for an output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum McTarget {
    /// The output should be at most this.
    AtMost(f64),
    /// The output should be at least this.
    AtLeast(f64),
}

impl McTarget {
    /// Whether `value` meets the target.
    pub fn met(self, value: f64) -> bool {
        match self {
            McTarget::AtMost(x) => value <= x,
            McTarget::AtLeast(x) => value >= x,
        }
    }

    pub fn value(self) -> f64 {
        match self {
            McTarget::AtMost(x) | McTarget::AtLeast(x) => x,
        }
    }
}

/// A rank (Spearman) correlation between two inputs, strictly between -1 and 1.
#[derive(Debug, Clone, PartialEq)]
pub struct McCorrelation {
    pub a: String,
    pub b: String,
    pub rank: f64,
}

/// How inputs are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sampling {
    /// Each draw independent.
    #[default]
    Random,
    /// Latin hypercube: within each batch of trials, each input's range is cut into as many
    /// equally likely strata as there are trials, and each stratum is drawn once.
    LatinHypercube,
    /// Antithetic pairs: trials come in pairs that use each input's uniform u and 1 - u.
    Antithetic,
    /// Randomised quasi-Monte Carlo: within each batch of trials, Sobol points with a random
    /// digital shift.
    Sobol,
}

impl Sampling {
    /// Its name in files (`stochcraft:monteCarlo`'s `sampling`).
    pub fn file_name(self) -> &'static str {
        match self {
            Sampling::Random => "random",
            Sampling::LatinHypercube => "latinHypercube",
            Sampling::Antithetic => "antithetic",
            Sampling::Sobol => "sobol",
        }
    }

    pub fn from_file_name(s: &str) -> Option<Self> {
        match s {
            "random" => Some(Sampling::Random),
            "latinHypercube" => Some(Sampling::LatinHypercube),
            "antithetic" => Some(Sampling::Antithetic),
            "sobol" => Some(Sampling::Sobol),
            _ => None,
        }
    }
}

/// A sequential stop: more trials until every output's mean has a 95% half-width within
/// `relative` of itself, or `max` trials have run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct McStop {
    pub relative: f64,
    pub max: u32,
}

impl Default for McStop {
    fn default() -> Self {
        Self {
            relative: 0.01,
            max: 1_000_000,
        }
    }
}

impl MonteCarlo {
    pub fn input(&self, name: &str) -> Option<&McInput> {
        self.inputs.iter().find(|i| i.name == name)
    }

    pub fn output(&self, name: &str) -> Option<&McOutput> {
        self.outputs.iter().find(|o| o.name == name)
    }
}
