//! Monte Carlo models through the engine (ROADMAP M4.1): the commands `mc.get`, `mc.set`,
//! `mc.input.set`, `mc.input.remove`, `mc.output.set`, `mc.output.remove`,
//! `mc.correlation.set`, `mc.inputs.import`, `mc.inputs.export`, `mc.run` and `mc.sobol`.
//!
//! A Monte Carlo model is the model's [`MonteCarlo`] section on a model with no process; its
//! trials are the experiment's `replications` and its seed the experiment's. Each edit replaces
//! the whole section, so it is undone exactly. The first edit makes the model a Monte Carlo one
//! and sets its trials to [`DEFAULT_TRIALS`], undone with it. The runs themselves are
//! `stochcraft-mc`'s, with its batches made on worker threads by `stochcraft-experiment`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_experiment::{Control, Experiment};
use stochcraft_mc::sobol::{DEFAULT_BOOTSTRAP, DEFAULT_ROWS};
use stochcraft_mc::{
    BATCH, Block, BlockJob, Compiled, Estimate, IntervalBasis, MAX_TRIALS, McResults, OutputResult,
    Problem, RunOptions, SobolError, SobolOptions, SobolResults, SobolRun, Stepper, table,
};
use stochcraft_model::montecarlo::DEFAULT_TRIALS;
use stochcraft_model::{
    Control as McControl, Dist, McCorrelation, McInput, McOutput, McStop, McTarget, MonteCarlo,
    Sampling,
};

use crate::commands::{parse, some};
use crate::edit::Op;
use crate::wire::DistSpec;
use crate::{CommandError, Engine, ErrorCode};

/// How inputs are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SamplingSpec {
    /// Each draw independent.
    #[default]
    Random,
    /// Latin hypercube: within each batch of 1,000 trials, one draw from each of as many equally
    /// likely strata of each input as there are trials.
    LatinHypercube,
    /// Antithetic pairs: trials come in pairs that use each input's uniform u and 1 - u, so
    /// an output that rises with the inputs has the pair's errors cancel.
    Antithetic,
    /// Randomised quasi-Monte Carlo: within each batch of 1,000 trials, Sobol points with a
    /// random linear matrix scramble and digital shift, one dimension per input (at most 64
    /// inputs).
    Sobol,
}

impl From<Sampling> for SamplingSpec {
    fn from(s: Sampling) -> Self {
        match s {
            Sampling::Random => SamplingSpec::Random,
            Sampling::LatinHypercube => SamplingSpec::LatinHypercube,
            Sampling::Antithetic => SamplingSpec::Antithetic,
            Sampling::Sobol => SamplingSpec::Sobol,
        }
    }
}

impl From<SamplingSpec> for Sampling {
    fn from(s: SamplingSpec) -> Self {
        match s {
            SamplingSpec::Random => Sampling::Random,
            SamplingSpec::LatinHypercube => Sampling::LatinHypercube,
            SamplingSpec::Antithetic => Sampling::Antithetic,
            SamplingSpec::Sobol => Sampling::Sobol,
        }
    }
}

impl SamplingSpec {
    fn name(self) -> &'static str {
        match self {
            SamplingSpec::Random => "random",
            SamplingSpec::LatinHypercube => "latin_hypercube",
            SamplingSpec::Antithetic => "antithetic",
            SamplingSpec::Sobol => "sobol",
        }
    }
}

/// An uncertain input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputDoc {
    pub name: String,
    pub distribution: DistSpec,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
}

/// A result each trial computes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputDoc {
    pub name: String,
    /// A BPSim expression over the inputs and the outputs before it, each as `$name` or
    /// `getProperty('name')`.
    pub expression: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// A target: the probability that the output is at most this is reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_most: Option<f64>,
    /// A target: the probability that the output is at least this is reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<f64>,
    /// Control variates that narrow this output's interval: inputs (their own means are known) or
    /// earlier outputs with a mean you know.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<ControlDoc>,
}

/// A control variate: `{ "input": "x" }` (the input's own mean is known), or `{ "output": "y",
/// "expected": 12.5 }` (an earlier output, with the mean you state; a wrong mean biases the
/// controlled result). An object with the fields of both forms is refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum ControlDoc {
    Input { input: String },
    Output { output: String, expected: f64 },
}

impl From<&McControl> for ControlDoc {
    fn from(c: &McControl) -> Self {
        match c {
            McControl::Input { name } => ControlDoc::Input {
                input: name.clone(),
            },
            McControl::Output { name, expected } => ControlDoc::Output {
                output: name.clone(),
                expected: *expected,
            },
        }
    }
}

impl From<ControlDoc> for McControl {
    fn from(c: ControlDoc) -> Self {
        match c {
            ControlDoc::Input { input } => McControl::Input { name: input },
            ControlDoc::Output { output, expected } => McControl::Output {
                name: output,
                expected,
            },
        }
    }
}

/// A rank (Spearman) correlation between two inputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrelationDoc {
    pub a: String,
    pub b: String,
    pub rank: f64,
}

/// A sequential stop: more trials until every output's mean has a 95% half-width within
/// `relative` of itself (0.01 is ±1%), or `max` trials have run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StopDoc {
    #[serde(default = "default_relative")]
    pub relative: f64,
    #[serde(default = "default_max")]
    pub max: u32,
}

fn default_relative() -> f64 {
    McStop::default().relative
}

fn default_max() -> u32 {
    McStop::default().max
}

impl From<McStop> for StopDoc {
    fn from(s: McStop) -> Self {
        Self {
            relative: s.relative,
            max: s.max,
        }
    }
}

impl From<StopDoc> for McStop {
    fn from(s: StopDoc) -> Self {
        Self {
            relative: s.relative,
            max: s.max,
        }
    }
}

/// A Monte Carlo model's inputs, outputs and sampling, as `model.get` and `model.set` give them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonteCarloDoc {
    #[serde(default)]
    pub inputs: Vec<InputDoc>,
    #[serde(default)]
    pub outputs: Vec<OutputDoc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub correlations: Vec<CorrelationDoc>,
    #[serde(default)]
    pub sampling: SamplingSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<StopDoc>,
}

fn target_of(at_most: Option<f64>, at_least: Option<f64>) -> Option<McTarget> {
    at_most
        .map(McTarget::AtMost)
        .or(at_least.map(McTarget::AtLeast))
}

impl From<&MonteCarlo> for MonteCarloDoc {
    fn from(m: &MonteCarlo) -> Self {
        Self {
            inputs: m
                .inputs
                .iter()
                .map(|i| InputDoc {
                    name: i.name.clone(),
                    distribution: DistSpec::from(&i.dist),
                    unit: i.unit.clone(),
                })
                .collect(),
            outputs: m
                .outputs
                .iter()
                .map(|o| OutputDoc {
                    name: o.name.clone(),
                    expression: o.expression.clone(),
                    unit: o.unit.clone(),
                    at_most: match o.target {
                        Some(McTarget::AtMost(x)) => Some(x),
                        _ => None,
                    },
                    at_least: match o.target {
                        Some(McTarget::AtLeast(x)) => Some(x),
                        _ => None,
                    },
                    controls: o.controls.iter().map(ControlDoc::from).collect(),
                })
                .collect(),
            correlations: m
                .correlations
                .iter()
                .map(|c| CorrelationDoc {
                    a: c.a.clone(),
                    b: c.b.clone(),
                    rank: c.rank,
                })
                .collect(),
            sampling: m.sampling.into(),
            stop: m.stop.map(Into::into),
        }
    }
}

impl From<MonteCarloDoc> for MonteCarlo {
    fn from(m: MonteCarloDoc) -> Self {
        Self {
            inputs: m
                .inputs
                .into_iter()
                .map(|i| McInput {
                    name: i.name,
                    dist: Dist::from(i.distribution),
                    unit: i.unit,
                })
                .collect(),
            outputs: m
                .outputs
                .into_iter()
                .map(|o| McOutput {
                    target: target_of(o.at_most, o.at_least),
                    controls: o.controls.into_iter().map(McControl::from).collect(),
                    name: o.name,
                    expression: o.expression,
                    unit: o.unit,
                })
                .collect(),
            correlations: m
                .correlations
                .into_iter()
                .map(|c| McCorrelation {
                    a: c.a,
                    b: c.b,
                    rank: c.rank,
                })
                .collect(),
            sampling: m.sampling.into(),
            stop: m.stop.map(Into::into),
        }
    }
}

/// Problems as JSON, in the shape of the model's diagnostics: `severity`, `at`, `code` and a
/// `message` that starts with the severity.
pub fn problems(ps: &[Problem]) -> Value {
    ps.iter()
        .map(|p| {
            let severity = if p.error { "error" } else { "warning" };
            json!({
                "severity": severity,
                "at": p.at,
                "code": p.code,
                "message": format!("{severity}: {}", p.message),
            })
        })
        .collect()
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParams, message)
}

fn not_found(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::NotFound, message)
}

fn check_name(name: &str) -> Result<(), CommandError> {
    if stochcraft_mc::valid_name(name) {
        Ok(())
    } else {
        Err(invalid(format!(
            "`{name}` cannot be a name: use letters, digits and _, starting with a letter or _, so that $name reads it"
        )))
    }
}

impl Engine {
    /// The model's Monte Carlo section, or an empty one.
    fn monte_carlo(&self) -> MonteCarlo {
        self.model.monte_carlo.clone().unwrap_or_default()
    }

    /// Replaces the Monte Carlo section in one undoable step; the first one also sets the
    /// trials to [`DEFAULT_TRIALS`].
    fn set_monte_carlo(&mut self, label: &'static str, m: MonteCarlo) -> Result<(), CommandError> {
        let op = Op::SetMonteCarlo(Some(Box::new(m)));
        if self.model.monte_carlo.is_some() {
            return self.edit(label, op);
        }
        let mut x = self.model.experiment.clone();
        x.replications = DEFAULT_TRIALS;
        self.edit(label, Op::Batch(vec![op, Op::SetExperiment(x)]))
    }
}

/// The inputs of `m` that can be a control variate: those that vary and whose draws have exactly the
/// mean their distribution states (`mc.output.set` refuses the others, saying why).
pub fn control_inputs(m: &MonteCarlo) -> Vec<String> {
    m.inputs
        .iter()
        .filter(|i| {
            i.dist.check().is_ok()
                && !stochcraft_mc::control::is_constant(&i.dist)
                && stochcraft_mc::control::exact_mean(&i.dist).is_ok()
        })
        .map(|i| i.name.clone())
        .collect()
}

/// `mc.get`: no parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Get {}

pub(crate) fn get(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<Get>(p)?;
    let m = e.model.monte_carlo.as_ref();
    let x = &e.model.experiment;
    Ok(json!({
        "monte_carlo": m.map(MonteCarloDoc::from),
        "is_monte_carlo": e.model.is_monte_carlo(),
        "trials": x.replications,
        "seed": x.seed,
        "problems": m.map_or(Value::Array(Vec::new()), |m| problems(&stochcraft_mc::check(m))),
    }))
}

/// `mc.set`: how the model is sampled and run.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Set {
    #[serde(default)]
    pub sampling: Option<SamplingSpec>,
    /// Run sequentially, until the outputs' means are precise enough; `null` runs a fixed
    /// number of trials.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<StopDoc>")]
    pub stop: Option<Option<StopDoc>>,
    /// The trials (a sequential run's first); the experiment's `replications`.
    #[serde(default)]
    pub trials: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
}

pub(crate) fn set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Set = parse(p)?;
    let mut m = e.monte_carlo();
    if let Some(s) = p.sampling {
        m.sampling = s.into();
    }
    if let Some(stop) = p.stop {
        if let Some(s) = stop
            && !(s.relative.is_finite() && s.relative > 0.0 && s.max >= 1)
        {
            return Err(invalid(
                "a stop needs `relative` above 0 (0.01 is ±1%) and `max` of at least 1",
            ));
        }
        m.stop = stop.map(Into::into);
    }
    if p.trials == Some(0) || p.trials.is_some_and(|t| u64::from(t) > MAX_TRIALS) {
        return Err(invalid(format!("`trials` must be from 1 to {MAX_TRIALS}")));
    }
    // A most below one batch, where runs are whole batches, is refused, not raised.
    if (p.sampling.is_some() || p.stop.is_some())
        && let Some(bad) = stochcraft_mc::check(&m)
            .into_iter()
            .find(|x| x.code == "bad_stop")
    {
        return Err(CommandError {
            code: ErrorCode::InvalidParams,
            message: bad.message.clone(),
            details: Some(problems(&[bad])),
        });
    }
    let created = e.model.monte_carlo.is_none();
    let mut ops = vec![Op::SetMonteCarlo(Some(Box::new(m)))];
    let mut x = e.model.experiment.clone();
    if created {
        x.replications = DEFAULT_TRIALS;
    }
    if let Some(t) = p.trials {
        x.replications = t;
    }
    if let Some(s) = p.seed {
        x.seed = s;
    }
    if x != e.model.experiment {
        ops.push(Op::SetExperiment(x));
    }
    e.edit("mc.set", Op::Batch(ops))?;
    Ok(json!({}))
}

/// `mc.input.set`: add an input or change one.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputSet {
    pub name: String,
    /// What each trial draws; needed for a new input.
    #[serde(default)]
    pub distribution: Option<DistSpec>,
    /// Free text, such as `h` or `USD`.
    #[serde(default)]
    pub unit: Option<String>,
}

pub(crate) fn input_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: InputSet = parse(p)?;
    check_name(&p.name)?;
    let mut m = e.monte_carlo();
    if m.output(&p.name).is_some() {
        return Err(CommandError::new(
            ErrorCode::Conflict,
            format!(
                "`{}` is an output; an input needs a name of its own",
                p.name
            ),
        ));
    }
    match m.inputs.iter_mut().find(|i| i.name == p.name) {
        Some(i) => {
            if let Some(d) = p.distribution {
                i.dist = d.into();
            }
            if let Some(u) = p.unit {
                i.unit = u;
            }
        }
        None => {
            let d = p.distribution.ok_or_else(|| {
                invalid(format!(
                    "`{}` is a new input: give its `distribution`",
                    p.name
                ))
            })?;
            m.inputs.push(McInput {
                name: p.name.clone(),
                dist: d.into(),
                unit: p.unit.unwrap_or_default(),
            });
        }
    }
    e.set_monte_carlo("mc.input.set", m)?;
    Ok(json!({ "name": p.name }))
}

/// A name, for the commands that remove one.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Name {
    pub name: String,
}

pub(crate) fn input_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Name = parse(p)?;
    let mut m = e.monte_carlo();
    let before = m.inputs.len();
    m.inputs.retain(|i| i.name != p.name);
    if m.inputs.len() == before {
        return Err(not_found(format!("no input `{}`", p.name)));
    }
    // Its correlations go with it.
    m.correlations.retain(|c| c.a != p.name && c.b != p.name);
    // So do the controls that named it.
    for o in &mut m.outputs {
        o.controls.retain(|c| c.name() != p.name);
    }
    e.set_monte_carlo("mc.input.remove", m)?;
    Ok(json!({}))
}

/// `mc.output.set`: add an output or change one.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputSet {
    pub name: String,
    /// A BPSim expression over the inputs and earlier outputs, each as `$name` or
    /// `getProperty('name')`; needed for a new output.
    #[serde(default)]
    pub expression: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
    /// The probability that the output is at most this is reported; `null` removes the target.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<f64>")]
    pub at_most: Option<Option<f64>>,
    /// The probability that the output is at least this is reported; `null` removes the target.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<f64>")]
    pub at_least: Option<Option<f64>>,
    /// The output's control variates, replacing its present ones (`[]` removes them): inputs, as
    /// `{ "input": "x" }` (the input's own mean is known; refused where the sampled mean is not
    /// exactly it), or earlier outputs with a mean you know, as `{ "output": "y", "expected": 12.5 }`.
    #[serde(default)]
    pub controls: Option<Vec<ControlDoc>>,
    /// Where it goes among the outputs (from 0); outputs are evaluated in order, each using
    /// those before it. A new output goes last by default.
    #[serde(default)]
    pub position: Option<u32>,
}

pub(crate) fn output_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: OutputSet = parse(p)?;
    check_name(&p.name)?;
    if let (Some(Some(_)), Some(Some(_))) = (p.at_most, p.at_least) {
        return Err(invalid("give `at_most` or `at_least`, not both"));
    }
    if let Some(Some(x)) = p.at_most.or(p.at_least)
        && !x.is_finite()
    {
        return Err(invalid("a target must be a finite number"));
    }
    let mut m = e.monte_carlo();
    let control_problems = |m: &MonteCarlo| -> Vec<(Option<String>, &'static str)> {
        stochcraft_mc::check(m)
            .into_iter()
            .filter(|x| x.code.contains("control"))
            .map(|x| (x.at, x.code))
            .collect()
    };
    let before = control_problems(&m);
    if m.input(&p.name).is_some() {
        return Err(CommandError::new(
            ErrorCode::Conflict,
            format!(
                "`{}` is an input; an output needs a name of its own",
                p.name
            ),
        ));
    }
    if let Some(x) = &p.expression
        && let Err(err) = stochcraft_model::expr::parse_with_variables(x)
    {
        return Err(invalid(format!("the expression cannot be read: {err}")));
    }
    let at = m.outputs.iter().position(|o| o.name == p.name);
    let mut out = match at {
        Some(i) => m.outputs.remove(i),
        None => McOutput {
            name: p.name.clone(),
            expression: p.expression.clone().ok_or_else(|| {
                invalid(format!(
                    "`{}` is a new output: give its `expression`",
                    p.name
                ))
            })?,
            unit: String::new(),
            target: None,
            controls: Vec::new(),
        },
    };
    if let Some(x) = p.expression {
        out.expression = x;
    }
    if let Some(u) = p.unit {
        out.unit = u;
    }
    match (p.at_most, p.at_least) {
        (Some(Some(x)), _) => out.target = Some(McTarget::AtMost(x)),
        (_, Some(Some(x))) => out.target = Some(McTarget::AtLeast(x)),
        (Some(None), _) | (_, Some(None)) => out.target = None,
        (None, None) => {}
    }
    if let Some(controls) = p.controls {
        out.controls = controls.into_iter().map(McControl::from).collect();
    }
    let place = match p.position {
        Some(i) => usize::try_from(i)
            .unwrap_or(usize::MAX)
            .min(m.outputs.len()),
        None => at.unwrap_or(m.outputs.len()),
    };
    m.outputs.insert(place, out);
    // Controls that cannot be used are refused, saying why: those of the edited output, and any
    // the edit leaves another output with (moving an output after one that controls on it).
    // Problems that were there before the edit do not stop an unrelated one.
    if let Some(bad) = stochcraft_mc::check(&m).into_iter().find(|x| {
        x.code.contains("control")
            && (x.at.as_deref() == Some(p.name.as_str())
                || !before.contains(&(x.at.clone(), x.code)))
    }) {
        return Err(CommandError {
            code: ErrorCode::InvalidParams,
            message: bad.message.clone(),
            details: Some(problems(&[bad])),
        });
    }
    e.set_monte_carlo("mc.output.set", m)?;
    Ok(json!({ "name": p.name }))
}

pub(crate) fn output_remove(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Name = parse(p)?;
    let mut m = e.monte_carlo();
    let before = m.outputs.len();
    m.outputs.retain(|o| o.name != p.name);
    if m.outputs.len() == before {
        return Err(not_found(format!("no output `{}`", p.name)));
    }
    for o in &mut m.outputs {
        o.controls.retain(|c| c.name() != p.name);
    }
    e.set_monte_carlo("mc.output.remove", m)?;
    Ok(json!({}))
}

/// `mc.correlation.set`: the rank correlation between two inputs; 0 removes it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CorrelationSet {
    pub a: String,
    pub b: String,
    /// Spearman's rank correlation, strictly between -1 and 1.
    pub rank: f64,
}

pub(crate) fn correlation_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: CorrelationSet = parse(p)?;
    let mut m = e.monte_carlo();
    for name in [&p.a, &p.b] {
        if m.input(name).is_none() {
            return Err(not_found(format!("no input `{name}`")));
        }
    }
    if p.a == p.b {
        return Err(invalid("an input cannot be correlated with itself"));
    }
    if !(p.rank.is_finite() && p.rank.abs() < 1.0) {
        return Err(invalid(
            "a rank correlation must be strictly between -1 and 1",
        ));
    }
    let same = |c: &McCorrelation| (c.a == p.a && c.b == p.b) || (c.a == p.b && c.b == p.a);
    match m.correlations.iter().position(same) {
        Some(i) if p.rank != 0.0 => {
            if let Some(c) = m.correlations.get_mut(i) {
                c.rank = p.rank;
            }
        }
        Some(i) => {
            m.correlations.remove(i);
        }
        None if p.rank != 0.0 => m.correlations.push(McCorrelation {
            a: p.a.clone(),
            b: p.b.clone(),
            rank: p.rank,
        }),
        None => return Ok(json!({})),
    }
    // Correlations that cannot all hold are refused, saying why.
    if let Some(bad) = stochcraft_mc::check(&m)
        .into_iter()
        .find(|x| x.code == "inconsistent_correlations")
    {
        return Err(CommandError {
            code: ErrorCode::InvalidParams,
            message: bad.message.clone(),
            details: Some(problems(&[bad])),
        });
    }
    e.set_monte_carlo("mc.correlation.set", m)?;
    Ok(json!({}))
}

/// `mc.inputs.import`: inputs from a CSV table (`name,distribution,parameters,unit`).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputsImport {
    /// The table's text, as `mc.inputs.export` writes it.
    pub csv: String,
    /// Replace every input with the table's; by default an input in the table replaces the one
    /// with its name, and the others are added.
    #[serde(default)]
    pub replace: bool,
}

pub(crate) fn inputs_import(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: InputsImport = parse(p)?;
    let rows = table::read(&p.csv).map_err(invalid)?;
    let mut m = e.monte_carlo();
    for i in &rows {
        check_name(&i.name)?;
        if m.output(&i.name).is_some() {
            return Err(CommandError::new(
                ErrorCode::Conflict,
                format!(
                    "`{}` is an output; an input needs a name of its own",
                    i.name
                ),
            ));
        }
    }
    if p.replace {
        m.inputs.clear();
    }
    let count = rows.len();
    for row in rows {
        match m.inputs.iter_mut().find(|i| i.name == row.name) {
            Some(i) => *i = row,
            None => m.inputs.push(row),
        }
    }
    let names: Vec<String> = m.inputs.iter().map(|i| i.name.clone()).collect();
    m.correlations
        .retain(|c| names.contains(&c.a) && names.contains(&c.b));
    // An input that is gone is no control any more.
    for o in &mut m.outputs {
        o.controls
            .retain(|c| !matches!(c, McControl::Input { name } if !names.contains(name)));
    }
    e.set_monte_carlo("mc.inputs.import", m)?;
    Ok(json!({ "inputs": count }))
}

pub(crate) fn inputs_export(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    parse::<Get>(p)?;
    Ok(json!({ "csv": table::write(&e.monte_carlo().inputs) }))
}

/// `mc.run`: run the model now.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Run {
    /// Trials (a sequential run's first); by default the experiment's `replications`.
    #[serde(default)]
    pub trials: Option<u64>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// By default the model's own.
    #[serde(default)]
    pub sampling: Option<SamplingSpec>,
    /// Worker threads; by default all available. Results do not depend on it.
    #[serde(default)]
    pub threads: Option<u32>,
    /// A sequential stop for this run only; `null` runs exactly `trials`; by default the
    /// model's own.
    #[serde(default, deserialize_with = "some")]
    #[schemars(with = "Option<StopDoc>")]
    pub stop: Option<Option<StopDoc>>,
    /// Percentiles (0 to 100) to report besides 5, 10, 50, 90 and 95.
    #[serde(default)]
    pub percentiles: Option<Vec<f64>>,
}

pub(crate) fn run(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let mut job = e.mc_start(p)?;
    while !job.step(usize::MAX)? {}
    e.mc_results(&mut job)
        .ok_or_else(|| CommandError::new(ErrorCode::Internal, "the run ended without results"))
}

/// A Monte Carlo run made a few batches at a time ([`Engine::mc_start`]), so the window can draw
/// between them and a long run never holds up the page: each [`McJob::step`] makes some of the
/// batches the run wants next, and [`Engine::mc_results`] gives the results once it has them.
/// `mc.run` is the same run with every batch it wants made at once, so the results are the
/// same, however the batches are spread over frames or threads.
pub struct McJob {
    compiled: Compiled,
    stepper: Stepper,
    /// The model as run (with the run's own sampling and stop), for the run record.
    model: MonteCarlo,
    threads: Option<u32>,
    /// The engine's model and file as they were when the run started (digest, file, format,
    /// scenario, unsaved changes, notes), for the run record: the model may be edited while the
    /// batches are made.
    context: Value,
}

impl McJob {
    /// Makes up to `batches` (at least one) of the batches the run wants next, on worker threads
    /// where there are several (none on wasm32). True once the run has its results.
    pub fn step(&mut self, batches: usize) -> Result<bool, CommandError> {
        if self.stepper.is_done() {
            return Ok(true);
        }
        if self.stepper.is_summarising() {
            // Every batch is in: one piece of the results a step, so a window can draw between.
            self.stepper.summarise_step(&self.compiled);
            return Ok(self.stepper.is_done());
        }
        let wanted: Vec<BlockJob> = self
            .stepper
            .wanted()
            .iter()
            .take(batches.max(1))
            .copied()
            .collect();
        let blocks = make_blocks(&self.compiled, &wanted, self.threads)?;
        self.stepper.deliver(&self.compiled, blocks);
        Ok(self.stepper.is_done())
    }

    /// The trials made so far, and those asked for so far (a sequential run asks for more after
    /// each look).
    pub fn progress(&self) -> (u64, u64) {
        (self.stepper.made(), self.stepper.asked())
    }
}

/// Makes `jobs` with `compiled`, on worker threads (`threads`, or all available), in order.
fn make_blocks(
    compiled: &Compiled,
    jobs: &[BlockJob],
    threads: Option<u32>,
) -> Result<Vec<Block>, CommandError> {
    let count = jobs.len() as u64;
    let mut runner = Experiment::new(count, compiled.seed());
    if let Some(t) = threads {
        runner = runner.threads(t as usize);
    }
    runner
        .run_range(
            0..count,
            |rep| {
                let job = usize::try_from(rep.index)
                    .ok()
                    .and_then(|i| jobs.get(i))
                    .copied()
                    .unwrap_or(BlockJob {
                        index: rep.index,
                        trials: 0,
                    });
                compiled.block(job)
            },
            &Control::new(),
        )
        .map_err(|err| CommandError::new(ErrorCode::Internal, err.to_string()))
}

impl Engine {
    /// Gets a Monte Carlo run ready, with `mc.run`'s parameters, without making any trials yet:
    /// the window makes it a few batches a frame with [`McJob::step`].
    pub fn mc_start(&self, params: Value) -> Result<McJob, CommandError> {
        let p: Run = parse(params)?;
        let mut m = self.model.monte_carlo.clone().ok_or_else(|| {
            not_found(
                "the model is not a Monte Carlo model: add inputs and outputs with mc.input.set and mc.output.set",
            )
        })?;
        if let Some(s) = p.sampling {
            m.sampling = s.into();
        }
        if let Some(stop) = p.stop {
            m.stop = stop.map(Into::into);
        }
        let x = &self.model.experiment;
        let seed = p.seed.unwrap_or(x.seed);
        let trials = p.trials.unwrap_or(u64::from(x.replications));
        if trials == 0 || trials > MAX_TRIALS {
            return Err(invalid(format!("`trials` must be from 1 to {MAX_TRIALS}")));
        }
        let percentiles = p.percentiles.unwrap_or_default();
        if let Some(bad) = percentiles.iter().find(|q| !(0.0..=100.0).contains(*q)) {
            return Err(invalid(format!("percentile {bad} is not from 0 to 100")));
        }
        let compiled = compile_model(&m, seed)?;
        let options = RunOptions {
            trials,
            stop: m.stop,
            percentiles,
        };
        let stepper = compiled.stepper(&options);
        Ok(McJob {
            compiled,
            stepper,
            model: m,
            threads: p.threads,
            context: self.mc_context(),
        })
    }

    /// A finished run's results, as `mc.run` gives them, with its warnings and run record; `None`
    /// before the run has them (and after they were taken).
    pub fn mc_results(&self, job: &mut McJob) -> Option<Value> {
        let results = job.stepper.take_results()?;
        let mut v = results_json(&results);
        v["warnings"] = problems(&job.compiled.warnings);
        v["record"] = mc_record(&job.context, &results, &job.model);
        Some(v)
    }

    /// What a run's record says of the model and its file, as they are now.
    fn mc_context(&self) -> Value {
        let file = self.file.as_ref();
        json!({
            "model_digest": crate::model_digest(&self.model),
            "file": file.and_then(|f| f.path.as_deref()),
            "format": file.map(|f| crate::format_name(f.opened.format)),
            "scenario": file.and_then(|f| f.opened.scenario.as_deref()),
            "unsaved_changes": file.is_some() && self.is_dirty(),
            "file_notes": file.map_or_else(Vec::new, |f| f.notes.clone()),
        })
    }
}

/// The run record of a Monte Carlo run: enough to reproduce it. `context` is the engine's
/// ([`Engine::mc_context`]) when the run started.
fn mc_record(context: &Value, r: &McResults, m: &MonteCarlo) -> Value {
    json!({
        "stochcraft": env!("CARGO_PKG_VERSION"),
        "kind": "monte_carlo",
        "model_digest": context["model_digest"],
        "file": context["file"],
        "format": context["format"],
        "scenario": context["scenario"],
        "unsaved_changes": context["unsaved_changes"],
        "seed": r.seed,
        "trials": r.trials,
        "trials_requested": r.requested_trials,
        "sampling": SamplingSpec::from(r.sampling).name(),
        "interval": interval_text(r),
        "batch": BATCH,
        "stop": m.stop.map(StopDoc::from),
        "random_streams": "trials in batches of 1,000; each input's draws in batch b from the stream keyed mc/<b>/input/<name> under the seed (with Sobol sampling its random linear matrix scramble and digital shift, from mc/qmc/<b>/<name>, the points being the first of Joe and Kuo's Sobol sequence with one dimension per input in declaration order), and the pairing of correlated inputs from mc/<b>/correlation, so results do not depend on the number of threads",
        "correlation": "rank correlations induced by Iman and Conover's method (1982) in each batch, on van der Waerden scores with Pearson correlation 2 sin(πρ/6) for a rank correlation ρ",
        "uncertainty": uncertainty_text(r),
        "controls": "an output with controls has a controlled estimate beside its results (controlled): the coefficients b of Y - b(C - mu) are fitted by least squares on a pilot, the first batch of 1,000 trials (the first half for a run under 2,000 trials), whose trials are left out of the estimate; for a run of a fixed number of trials that makes it unbiased (a run that stops at a precision target can still be biased by the stopping rule, which judges the same trials); where trials are grouped by batch (Latin hypercube, Sobol, correlated inputs) the pilot is the whole first batch, so one batch gives no controlled estimate (two are needed, three for an interval), and antithetic runs round the pilot to whole pairs; a trial with no number in the output or a control withdraws the controlled estimate; the controlled mean and target probability, with their 95% intervals on the run's interval basis, come from the remaining trials; variance_ratio and narrowing compare the controlled interval with the uncontrolled one on those same trials. Percentiles, histogram and extremes are not controlled; a sequential stop is judged on the uncontrolled mean",
        "influence": "per output and per input that is not a constant, widest swing first. base: the output with every input at its median. low and high: the output with that input at its 10th (p10) and its 90th (p90) percentile and every other input at its median (the tornado of decision analysis, Clemen and Reilly, Making Hard Decisions, 3rd ed., 2014); swing: |high - low|, null if either is not a number. No sampling is used, so these do not vary with the seed, the trials or the sampling, and they leave out what correlated inputs do together. rank_correlation: Spearman's, between the input and the output across the run's trials where the output has a number; for a correlated input it includes the effect of the inputs it is correlated with; null if either has no spread",
        "file_notes": context["file_notes"],
    })
}

/// What the intervals are across, in words: `Student-t across 12 independent Latin hypercube
/// batches of 1000 trials`, across independent antithetic pairs, or across trials.
pub fn interval_text(r: &McResults) -> String {
    match r.interval {
        IntervalBasis::Trials => format!("Student-t across {} independent trials", r.trials),
        IntervalBasis::Batches { batches, size } => {
            let kind = match r.sampling {
                Sampling::LatinHypercube => "Latin hypercube batches",
                Sampling::Sobol => "randomised Sobol batches",
                Sampling::Random | Sampling::Antithetic => "batches",
            };
            format!("Student-t across {batches} independent {kind} of {size} trials")
        }
        IntervalBasis::Pairs { pairs } => {
            format!("Student-t across {pairs} independent antithetic pairs of trials")
        }
    }
}

fn uncertainty_text(r: &McResults) -> &'static str {
    match r.interval {
        IntervalBasis::Trials => {
            "plain random sampling with no correlations: the trials are independent, so the mean has the 95% Student-t interval across trials (half_width) and a target's probability Wilson's 95% score interval; percentiles by Hyndman and Fan's definition 7"
        }
        IntervalBasis::Batches { .. } => {
            "Latin hypercube or Sobol sampling or correlated inputs make the trials within a batch dependent, so each batch is one independent replicate (replicated Latin hypercube sampling, Owen, 1992; Glasserman, 2003, section 4.4; Sobol sampling is randomised quasi-Monte Carlo with an independent random linear matrix scramble and digital shift in each batch, Matousek, 1998, and Owen, 2003): the mean has the 95% Student-t interval on the batch means and a target's probability the 95% Student-t interval on the batch proportions, clamped to [0, 1], with batches - 1 degrees of freedom, and none with fewer than two batches; runs are whole batches; std_dev is across trials; percentiles by Hyndman and Fan's definition 7"
        }
        IntervalBasis::Pairs { .. } => {
            "antithetic sampling makes the two trials of a pair (u and 1 - u for every input) dependent, and the pairs independent (Glasserman, 2003, section 4.2): the mean has the 95% Student-t interval on the pair means and a target's probability the 95% Student-t interval on the pair proportions, clamped to [0, 1], with pairs - 1 degrees of freedom; runs are whole pairs; std_dev is across trials, so it is not the spread the interval is made from; percentiles by Hyndman and Fan's definition 7"
        }
    }
}

/// An output's control variates and controlled estimate as JSON: the controls with their known
/// means and coefficients, and the estimate (`null` with the reason in `unavailable` when there
/// is none). `narrowing` is how much narrower the controlled interval is than the uncontrolled
/// one on the same trials (negative: wider); `helped` is whether that is at least 10%.
fn controlled_json(c: &stochcraft_mc::Controlled) -> Value {
    let estimate = c.estimate.as_ref().map(|e| {
        let target = e.target.as_ref().map(|t| {
            json!({
                "coefficients": t.coefficients,
                "probability": t.probability,
                "interval": t.interval.map(|(lo, hi)| [lo, hi]),
                "uncontrolled_probability": t.uncontrolled_probability,
                "uncontrolled_interval": t.uncontrolled_interval.map(|(lo, hi)| [lo, hi]),
                "variance_ratio": t.variance_ratio,
                "narrowing": t.narrowing,
                "helped": t.helped(),
            })
        });
        json!({
            "trials": e.trials,
            "interval_count": e.units,
            "mean": e.mean,
            "half_width": e.half_width,
            "uncontrolled_mean": e.uncontrolled_mean,
            "uncontrolled_half_width": e.uncontrolled_half_width,
            "variance_ratio": e.variance_ratio,
            "narrowing": e.narrowing,
            "helped": e.helped(),
            "target": target,
        })
    });
    json!({
        "controls": c.controls.iter().map(|u| json!({
            "name": u.name,
            "kind": if u.input { "input" } else { "output" },
            "mean": u.mean,
            "coefficient": u.coefficient,
            "sample_mean": u.sample_mean,
            "mean_mismatch": u.mismatch,
        })).collect::<Vec<_>>(),
        "pilot_trials": c.pilot_trials,
        "correlation": c.correlation,
        "unavailable": c.unavailable,
        "estimate": estimate,
    })
}

fn output_json(o: &OutputResult) -> Value {
    let percentiles: serde_json::Map<String, Value> = o
        .percentiles
        .iter()
        .map(|(p, v)| (format!("{p}"), json!(v)))
        .collect();
    let mut v = json!({
        "name": o.name,
        "unit": o.unit,
        "valid": o.valid,
        "invalid": o.invalid,
        "mean": o.mean,
        "half_width": o.half_width,
        "interval_count": o.interval_count,
        "std_dev": o.std_dev,
        "min": o.min,
        "max": o.max,
        "percentiles": percentiles,
        "histogram": { "edges": o.histogram.edges, "counts": o.histogram.counts },
        "cdf": o.cdf.iter().map(|(x, p)| json!([x, p])).collect::<Vec<_>>(),
        "influence": {
            "base": o.influence.base,
            "inputs": o.influence.inputs.iter().map(|i| json!({
                "input": i.input,
                "p10": i.p10,
                "p90": i.p90,
                "low": i.low,
                "high": i.high,
                "swing": i.swing(),
                "rank_correlation": i.rank_correlation,
            })).collect::<Vec<_>>(),
        },
    });
    if let Some(c) = &o.controlled {
        v["controlled"] = controlled_json(c);
    }
    if let Some(t) = o.target {
        let (kind, x) = match t.target {
            McTarget::AtMost(x) => ("at_most", x),
            McTarget::AtLeast(x) => ("at_least", x),
        };
        v["target"] = json!({
            kind: x,
            "met": t.met,
            "of": t.of,
            "probability": t.probability,
            "interval": t.interval.map(|(lo, hi)| [lo, hi]),
        });
    }
    v
}

/// A run's results as JSON.
pub fn results_json(r: &McResults) -> Value {
    let basis = match r.interval {
        IntervalBasis::Trials => json!({ "across": "trials", "count": r.trials }),
        IntervalBasis::Batches { batches, size } => {
            json!({ "across": "batches", "count": batches, "size": size })
        }
        IntervalBasis::Pairs { pairs } => json!({ "across": "pairs", "count": pairs, "size": 2 }),
    };
    let mut v = json!({
        "kind": "monte_carlo",
        "trials": r.trials,
        "trials_requested": r.requested_trials,
        "seed": r.seed,
        "sampling": SamplingSpec::from(r.sampling).name(),
        "interval": interval_text(r),
        "interval_basis": basis,
        "outputs": r.outputs.iter().map(output_json).collect::<Vec<_>>(),
        "inputs": r.inputs.iter().map(|i| json!({
            "name": i.name,
            "unit": i.unit,
            "mean": i.mean,
            "std_dev": i.std_dev,
            "min": i.min,
            "max": i.max,
            "expected_mean": i.expected_mean,
        })).collect::<Vec<_>>(),
        "correlations": r.correlations.iter().map(|c| json!({
            "a": c.a,
            "b": c.b,
            "target": c.target,
            "achieved": c.achieved,
        })).collect::<Vec<_>>(),
    });
    if let Some(s) = r.stop.as_ref() {
        v["stop"] = json!({
            "relative": s.relative,
            "max": s.max,
            "reason": s.reason.name(),
            "looks": s.looks,
            "note": "estimated precision after adaptive stopping: the interval's coverage is not guaranteed to be 95%",
        });
    }
    v
}

/// The most bootstrap resamples `mc.sobol` makes, so that a typing error cannot hold it for hours.
pub const MOST_BOOTSTRAP: u32 = 100_000;

/// `mc.sobol`: how much of each output's variation each input accounts for.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SobolParams {
    /// Rows of each of the two base samples, N; the model is evaluated N × (inputs + 2) times
    /// (inputs that vary). By default 1,000; at least 10.
    #[serde(default)]
    pub rows: Option<u64>,
    /// The seed of the draws; by default the model's own.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Bootstrap resamples for the 95% intervals (0 for none); by default 1,000.
    #[serde(default)]
    pub bootstrap: Option<u32>,
}

pub(crate) fn sobol(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let mut job = e.sobol_start(p)?;
    while !job.step(usize::MAX) {}
    job.results()
        .ok_or_else(|| CommandError::new(ErrorCode::Internal, "the run ended without results"))
}

/// Compiles `m` for a run under `seed`, or says what is wrong with it.
fn compile_model(m: &MonteCarlo, seed: u64) -> Result<Compiled, CommandError> {
    stochcraft_mc::compile(m, seed).map_err(|ps| CommandError {
        code: ErrorCode::InvalidModel,
        message: "the Monte Carlo model has errors and cannot run".into(),
        details: Some(problems(&ps)),
    })
}

/// How many of `m`'s inputs vary (are not constants): those a Sobol run makes indices for.
fn varied_inputs(m: &MonteCarlo) -> u64 {
    m.inputs
        .iter()
        .filter(|i| !matches!(i.dist, Dist::Constant(_)))
        .count() as u64
}

/// A Sobol run refused: a model whose inputs cannot have indices is an invalid model, a number
/// asked for that cannot be used invalid parameters.
fn sobol_error(e: &SobolError) -> CommandError {
    let code = match e {
        SobolError::Correlated(_) | SobolError::NoVariedInputs => ErrorCode::InvalidModel,
        SobolError::TooFewRows | SobolError::TooMany { .. } => ErrorCode::InvalidParams,
    };
    CommandError::new(code, e.to_string())
}

/// A Sobol run made a few pieces at a time ([`Engine::sobol_start`]), so the window can draw
/// between them: each [`SobolJob::step`] evaluates some batches of rows or works out an output's
/// indices, and [`SobolJob::results`] gives the results once it is done. `mc.sobol` is the same
/// run with every piece made at once, so the results are the same however the pieces are spread
/// over frames. Unlike a Monte Carlo run's batches these are made on the calling thread.
pub struct SobolJob {
    compiled: Compiled,
    run: SobolRun,
    evaluations: u64,
    /// The results, once the run has made them and until they are taken.
    results: Option<Value>,
    finished: bool,
}

impl SobolJob {
    /// Makes up to `pieces` (at least one) of the pieces of work the run has left. True once the
    /// run is done and has its results.
    pub fn step(&mut self, pieces: usize) -> bool {
        let mut made = 0;
        while !self.finished && made < pieces.max(1) {
            self.run.step(&self.compiled);
            made += 1;
            if self.run.is_done() {
                self.results = self.run.take_results().map(|r| sobol_json(&r));
                self.finished = true;
            }
        }
        self.finished
    }

    /// How far along the run is, from 0 to 1.
    pub fn progress(&self) -> f64 {
        if self.finished {
            1.0
        } else {
            self.run.progress(&self.compiled).clamp(0.0, 1.0)
        }
    }

    /// The model evaluations the run makes in all, N × (inputs + 2).
    pub fn evaluations(&self) -> u64 {
        self.evaluations
    }

    /// The finished run's results, as `mc.sobol` gives them; `None` before it is done (and after
    /// they were taken).
    pub fn results(&mut self) -> Option<Value> {
        self.results.take()
    }
}

impl Engine {
    /// Gets a Sobol run ready, with `mc.sobol`'s parameters, without evaluating the model yet:
    /// the window makes it a few pieces a frame with [`SobolJob::step`].
    pub fn sobol_start(&self, params: Value) -> Result<SobolJob, CommandError> {
        let p: SobolParams = parse(params)?;
        let m = self.model.monte_carlo.as_ref().ok_or_else(|| {
            not_found(
                "the model is not a Monte Carlo model: add inputs and outputs with mc.input.set and mc.output.set",
            )
        })?;
        let seed = p.seed.unwrap_or(self.model.experiment.seed);
        let rows = p.rows.unwrap_or(DEFAULT_ROWS);
        let bootstrap = p.bootstrap.unwrap_or(DEFAULT_BOOTSTRAP);
        if bootstrap > MOST_BOOTSTRAP {
            return Err(invalid(format!(
                "`bootstrap` must be from 0 to {MOST_BOOTSTRAP}"
            )));
        }
        let compiled = compile_model(m, seed)?;
        let run = compiled
            .sobol_run(rows, seed, &SobolOptions { bootstrap })
            .map_err(|err| sobol_error(&err))?;
        Ok(SobolJob {
            compiled,
            run,
            evaluations: rows.saturating_mul(varied_inputs(m) + 2),
            results: None,
            finished: false,
        })
    }

    /// What a Sobol run with the default rows would take in model evaluations, N × (inputs + 2);
    /// `None` when no input varies.
    pub fn sobol_evaluations(&self) -> Option<u64> {
        let varied = varied_inputs(self.model.monte_carlo.as_ref()?);
        (varied > 0).then(|| DEFAULT_ROWS * (varied + 2))
    }

    /// The model has correlated inputs, so a Sobol run is refused (the indices assume
    /// independent inputs).
    pub fn sobol_correlated(&self) -> bool {
        self.model
            .monte_carlo
            .as_ref()
            .is_some_and(|m| !m.correlations.is_empty())
    }
}

fn estimate_json(e: &Estimate) -> Value {
    json!({
        "value": e.value,
        "interval": e.interval.map(|(lo, hi)| [lo, hi]),
    })
}

/// A Sobol run's results as JSON: per output its variance, the rows used and dropped, why it has
/// no indices if it has none, and per input its first-order and total index with their 95%
/// intervals (`null` when no bootstrap was asked for).
pub fn sobol_json(r: &SobolResults) -> Value {
    json!({
        "kind": "sobol",
        "rows": r.rows,
        "evaluations": r.evaluations,
        "seed": r.seed,
        "bootstrap": r.bootstrap,
        "inputs": r.inputs,
        "constants": r.constants,
        "outputs": r.outputs.iter().map(|o| json!({
            "name": o.name,
            "variance": o.variance,
            "rows_used": o.rows_used,
            "rows_dropped": o.rows_dropped,
            "unavailable": o.unavailable,
            "inputs": o.indices.iter().map(|i| json!({
                "input": i.input,
                "first": estimate_json(&i.first),
                "total": estimate_json(&i.total),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "method": "Sobol indices from two independent random base samples A and B of `rows` rows each and, for each input that varies, the matrix A_B with that input's column taken from B: rows × (inputs + 2) model evaluations. first: the share of the output's variance the input accounts for alone (Saltelli, Annoni, Azzini, Campolongo, Ratto and Tarantola, 2010). total: the share it accounts for with all its interactions (Jansen, 1999). Intervals are 95% percentile bootstrap intervals over the rows. An estimate from few rows can fall a little below 0 or above 1. The inputs must be independent; constant inputs are left out.",
    })
}

/// A text cell safe to open in a spreadsheet: one that starts with `=`, `+`, `-`, `@`, a tab or a
/// carriage return would be read as a formula, so it gets a `'` in front (OWASP, "CSV Injection").
fn text_cell(text: &str) -> String {
    if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_owned()
    }
}

/// A Monte Carlo run's results (as [`results_json`] gives them) as CSV, a row per output, as the
/// window's Export table (CSV) and `stochcraft-cli run --format csv` write them.
pub fn results_csv(r: &Value) -> String {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let mut out = String::from(
        "output,unit,trials,valid,mean,half_width,std_dev,min,p5,p10,p50,p90,p95,max,\
         target,target_value,probability,probability_low,probability_high,\
         controlled_mean,controlled_half_width,interval_narrowing,controlled_probability,\
         controlled_probability_low,controlled_probability_high\n",
    );
    let num = |v: &Value| v.as_f64().map(|x| x.to_string()).unwrap_or_default();
    for o in r["outputs"].as_array().into_iter().flatten() {
        let t = &o["target"];
        let (kind, value) = if let Some(x) = t["at_most"].as_f64() {
            ("at_most", x.to_string())
        } else if let Some(x) = t["at_least"].as_f64() {
            ("at_least", x.to_string())
        } else {
            ("", String::new())
        };
        let row = [
            crate::wire::csv_field(&text_cell(&s(&o["name"]))),
            crate::wire::csv_field(&text_cell(&s(&o["unit"]))),
            r["trials"].to_string(),
            o["valid"].to_string(),
            num(&o["mean"]),
            num(&o["half_width"]),
            num(&o["std_dev"]),
            num(&o["min"]),
            num(&o["percentiles"]["5"]),
            num(&o["percentiles"]["10"]),
            num(&o["percentiles"]["50"]),
            num(&o["percentiles"]["90"]),
            num(&o["percentiles"]["95"]),
            num(&o["max"]),
            kind.to_owned(),
            value,
            num(&t["probability"]),
            num(&t["interval"][0]),
            num(&t["interval"][1]),
            num(&o["controlled"]["estimate"]["mean"]),
            num(&o["controlled"]["estimate"]["half_width"]),
            num(&o["controlled"]["estimate"]["narrowing"]),
            num(&o["controlled"]["estimate"]["target"]["probability"]),
            num(&o["controlled"]["estimate"]["target"]["interval"][0]),
            num(&o["controlled"]["estimate"]["target"]["interval"][1]),
        ];
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_form_goes_both_ways() {
        let m = MonteCarlo {
            inputs: vec![McInput {
                name: "x".into(),
                dist: Dist::Normal {
                    mean: 1.0,
                    std_dev: 2.0,
                },
                unit: "h".into(),
            }],
            outputs: vec![McOutput {
                name: "y".into(),
                expression: "$x * 2".into(),
                unit: String::new(),
                target: Some(McTarget::AtLeast(3.0)),
                controls: vec![McControl::Input { name: "x".into() }],
            }],
            correlations: vec![],
            sampling: Sampling::LatinHypercube,
            stop: Some(McStop::default()),
        };
        let doc = MonteCarloDoc::from(&m);
        assert_eq!(MonteCarlo::from(doc.clone()), m);
        let text = serde_json::to_value(&doc).unwrap_or_default();
        assert_eq!(text["sampling"], "latin_hypercube");
        assert_eq!(text["outputs"][0]["at_least"], 3.0);
        assert!(text["outputs"][0].get("at_most").is_none());
    }

    /// A name or unit a spreadsheet would read as a formula gets a quote in front; numbers and
    /// ordinary text are as they were.
    #[test]
    fn text_cells_that_look_like_formulas_are_quoted() {
        let r = json!({
            "trials": 10,
            "outputs": [
                { "name": "=1+1", "unit": "@u", "valid": 10, "mean": 1.5 },
                { "name": "-x", "unit": "+y", "valid": 10 },
                { "name": "\tz", "unit": "\rw", "valid": 10 },
                { "name": "cost", "unit": "USD", "valid": 10 },
            ],
        });
        let csv = results_csv(&r);
        let rows: Vec<&str> = csv.split('\n').collect();
        assert!(
            rows.get(1)
                .is_some_and(|l| l.starts_with("'=1+1,'@u,10,10,1.5,"))
        );
        assert!(rows.get(2).is_some_and(|l| l.starts_with("'-x,'+y,10,10,")));
        assert!(rows.get(3).is_some_and(|l| l.starts_with("'\tz,")));
        assert!(
            rows.get(4)
                .is_some_and(|l| l.starts_with("cost,USD,10,10,"))
        );
        assert_eq!(text_cell("a-b"), "a-b");
    }
}
