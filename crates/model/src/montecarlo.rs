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
    /// Whether it answers the model's question or is a step towards an answer.
    pub role: OutputRole,
    /// A summary reported beside the mean, such as the root mean square of a squared error.
    pub summary: Option<OutputSummary>,
}

/// What an output is for. Results list the key outputs first and fold the working ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputRole {
    /// An answer to the model's question.
    #[default]
    Key,
    /// A step towards an answer (a distance, an intermediate estimate), kept to be checked.
    Working,
}

impl OutputRole {
    /// Its name in files (`stochcraft:output`'s `role`).
    pub fn file_name(self) -> &'static str {
        match self {
            OutputRole::Key => "key",
            OutputRole::Working => "working",
        }
    }

    pub fn from_file_name(s: &str) -> Option<Self> {
        match s {
            "key" => Some(OutputRole::Key),
            "working" => Some(OutputRole::Working),
            _ => None,
        }
    }
}

/// A summary of an output reported beside its mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSummary {
    /// The output is a squared quantity (a squared error); its root mean square √mean is
    /// reported, in the unit of the quantity, with a 95% half-width by the delta method,
    /// half-width / (2 √mean).
    Rms,
}

impl OutputSummary {
    /// Its name in files (`stochcraft:output`'s `summary`).
    pub fn file_name(self) -> &'static str {
        match self {
            OutputSummary::Rms => "rms",
        }
    }

    pub fn from_file_name(s: &str) -> Option<Self> {
        match s {
            "rms" => Some(OutputSummary::Rms),
            _ => None,
        }
    }

    /// The summary's value and 95% half-width from the output's mean and half-width: `None`
    /// for a mean that is negative or not a number, and no half-width for a mean of 0.
    pub fn of(self, mean: f64, half_width: Option<f64>) -> Option<(f64, Option<f64>)> {
        match self {
            OutputSummary::Rms => {
                if !mean.is_finite() || mean < 0.0 {
                    return None;
                }
                let rms = mean.sqrt();
                let hw = half_width.filter(|_| rms > 0.0).map(|h| h / (2.0 * rms));
                Some((rms, hw))
            }
        }
    }

    /// The summary's unit from the output's: a squared unit (`m²`, `m^2`) loses its square, any
    /// other is put under a root (`√(USD)`), and none stays none.
    pub fn unit(self, unit: &str) -> String {
        match self {
            OutputSummary::Rms => {
                let u = unit.trim();
                if u.is_empty() {
                    String::new()
                } else if let Some(base) = u.strip_suffix('²').or_else(|| u.strip_suffix("^2")) {
                    base.trim_end().to_owned()
                } else {
                    format!("√({u})")
                }
            }
        }
    }
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
    /// linear matrix scramble and digital shift.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_root_mean_square_has_the_unit_and_interval_of_the_quantity() {
        let rms = OutputSummary::Rms;
        assert_eq!(rms.unit("m²"), "m");
        assert_eq!(rms.unit("m^2"), "m");
        assert_eq!(rms.unit("USD"), "√(USD)");
        assert_eq!(rms.unit(""), "");
        // √38.0689 = 6.17; the delta method: 0.8 / (2 × 6.17).
        let (v, hw) = rms.of(38.0689, Some(0.8)).unwrap_or_default();
        assert!((v - 6.17).abs() < 1e-12);
        assert!((hw.unwrap_or_default() - 0.8 / 12.34).abs() < 1e-12);
        assert_eq!(rms.of(0.0, Some(1.0)), Some((0.0, None)));
        assert_eq!(rms.of(-1.0, None), None);
        assert_eq!(rms.of(f64::NAN, None), None);
    }

    #[test]
    fn roles_and_summaries_have_file_names_both_ways() {
        for r in [OutputRole::Key, OutputRole::Working] {
            assert_eq!(OutputRole::from_file_name(r.file_name()), Some(r));
        }
        assert_eq!(OutputRole::default(), OutputRole::Key);
        assert_eq!(
            OutputSummary::from_file_name(OutputSummary::Rms.file_name()),
            Some(OutputSummary::Rms)
        );
        assert_eq!(OutputSummary::from_file_name("mean"), None);
    }
}
