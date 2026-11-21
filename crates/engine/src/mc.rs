//! Monte Carlo models through the engine (ROADMAP M4.1): the commands `mc.get`, `mc.set`,
//! `mc.input.set`, `mc.input.remove`, `mc.output.set`, `mc.output.remove`,
//! `mc.correlation.set`, `mc.inputs.import`, `mc.inputs.export` and `mc.run`.
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
use stochcraft_mc::{
    BATCH, Block, BlockJob, Compiled, IntervalBasis, MAX_TRIALS, McResults, OutputResult, Problem,
    RunOptions, Stepper, table,
};
use stochcraft_model::montecarlo::DEFAULT_TRIALS;
use stochcraft_model::{
    Dist, McCorrelation, McInput, McOutput, McStop, McTarget, MonteCarlo, Sampling,
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
}

impl From<Sampling> for SamplingSpec {
    fn from(s: Sampling) -> Self {
        match s {
            Sampling::Random => SamplingSpec::Random,
            Sampling::LatinHypercube => SamplingSpec::LatinHypercube,
        }
    }
}

impl From<SamplingSpec> for Sampling {
    fn from(s: SamplingSpec) -> Self {
        match s {
            SamplingSpec::Random => Sampling::Random,
            SamplingSpec::LatinHypercube => Sampling::LatinHypercube,
        }
    }
}

impl SamplingSpec {
    fn name(self) -> &'static str {
        match self {
            SamplingSpec::Random => "random",
            SamplingSpec::LatinHypercube => "latin_hypercube",
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
    let place = match p.position {
        Some(i) => usize::try_from(i)
            .unwrap_or(usize::MAX)
            .min(m.outputs.len()),
        None => at.unwrap_or(m.outputs.len()),
    };
    m.outputs.insert(place, out);
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
        let compiled = stochcraft_mc::compile(&m, seed).map_err(|ps| CommandError {
            code: ErrorCode::InvalidModel,
            message: "the Monte Carlo model has errors and cannot run".into(),
            details: Some(problems(&ps)),
        })?;
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
        "random_streams": "trials in batches of 1,000; each input's draws in batch b from the stream keyed mc/<b>/input/<name> under the seed, and the pairing of correlated inputs from mc/<b>/correlation, so results do not depend on the number of threads",
        "correlation": "rank correlations induced by Iman and Conover's method (1982) in each batch, on van der Waerden scores with Pearson correlation 2 sin(πρ/6) for a rank correlation ρ",
        "uncertainty": uncertainty_text(r),
        "influence": "per output and per input that is not a constant, widest swing first. base: the output with every input at its median. low and high: the output with that input at its 10th (p10) and its 90th (p90) percentile and every other input at its median (the tornado of decision analysis, Clemen and Reilly, Making Hard Decisions, 3rd ed., 2014); swing: |high - low|, null if either is not a number. No sampling is used, so these do not vary with the seed, the trials or the sampling, and they leave out what correlated inputs do together. rank_correlation: Spearman's, between the input and the output across the run's trials where the output has a number; for a correlated input it includes the effect of the inputs it is correlated with; null if either has no spread",
        "file_notes": context["file_notes"],
    })
}

/// What the intervals are across, in words: `Student-t across 12 independent Latin hypercube
/// batches of 1000 trials`, or across trials.
pub fn interval_text(r: &McResults) -> String {
    match r.interval {
        IntervalBasis::Trials => format!("Student-t across {} independent trials", r.trials),
        IntervalBasis::Batches { batches, size } => {
            let kind = match r.sampling {
                Sampling::LatinHypercube => "Latin hypercube batches",
                Sampling::Random => "batches",
            };
            format!("Student-t across {batches} independent {kind} of {size} trials")
        }
    }
}

fn uncertainty_text(r: &McResults) -> &'static str {
    match r.interval {
        IntervalBasis::Trials => {
            "plain random sampling with no correlations: the trials are independent, so the mean has the 95% Student-t interval across trials (half_width) and a target's probability Wilson's 95% score interval; percentiles by Hyndman and Fan's definition 7"
        }
        IntervalBasis::Batches { .. } => {
            "Latin hypercube sampling or correlated inputs make the trials within a batch dependent, so each batch is one independent replicate (replicated Latin hypercube sampling; Owen, 1992; Glasserman, 2003, section 4.4): the mean has the 95% Student-t interval on the batch means and a target's probability the 95% Student-t interval on the batch proportions, clamped to [0, 1], with batches - 1 degrees of freedom, and none with fewer than two batches; runs are whole batches; std_dev is across trials; percentiles by Hyndman and Fan's definition 7"
        }
    }
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
         target,target_value,probability,probability_low,probability_high\n",
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
