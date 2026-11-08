//! Experiments for decisions over many runs of the model (M2.8): sweeping a parameter over a
//! range, recording which parameters drive which results, and sizing capacity at the
//! bottleneck until a target share of replications meets a target.
//!
//! Every run of a model variant uses the same seed and so the same random streams (common
//! random numbers), so differences between variants come from the change, not from noise
//! (Law, *Simulation Modeling and Analysis*, 5th ed., section 11.2). Replication i of one
//! variant is paired with replication i of the other, and a difference is judged by the
//! paired-t interval on those differences (Law, section 10.2.1).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_blocks::{ExperimentSummary, Metric, compile, summarise};
use stochcraft_experiment::Experiment;
use stochcraft_model::{Dist, ElementId, ElementKind, Model, ResourcePool};
use stochcraft_stats::{Proportion, difference};

use crate::{CommandError, ErrorCode, wire};

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParams, message)
}

fn not_found(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::NotFound, message)
}

/// Which number of the model to vary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// A task's units (a task with a resource of its own).
    Capacity,
    /// A shared pool's units (with shifts: the units of every shift that has any).
    Quantity,
    /// A task's finite buffer.
    QueueCapacity,
    /// The mean of an element's time distribution (a source's time between arrivals, a task's
    /// service time, a delay's duration), its shape kept: the distribution is rescaled.
    Mean,
}

impl Field {
    fn integral(self) -> bool {
        !matches!(self, Field::Mean)
    }

    fn name(self) -> &'static str {
        match self {
            Field::Capacity => "capacity",
            Field::Quantity => "quantity",
            Field::QueueCapacity => "queue_capacity",
            Field::Mean => "mean",
        }
    }
}

/// A parameter: an element's or a pool's number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Param {
    /// The element or pool.
    pub id: String,
    pub field: Field,
}

/// A result: an element's (or a pool's) metric, by the names the results use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub element: String,
    pub metric: String,
}

/// A target for a result, met by a replication whose value is at least `at_least` or at most
/// `at_most` (give one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub element: String,
    pub metric: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_least: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_most: Option<f64>,
}

impl Param {
    /// The parameter's value in `model`.
    pub fn get(&self, model: &Model) -> Result<f64, CommandError> {
        let id = ElementId::unchecked(&self.id);
        match self.field {
            Field::Quantity => pool(model, &id).map(|p| f64::from(p.most_units())),
            Field::Mean => model
                .estimated(&id)
                .and_then(Dist::mean)
                .ok_or_else(|| invalid(format!("`{id}` has no time distribution with a mean"))),
            Field::Capacity | Field::QueueCapacity => match model.element(&id).map(|e| &e.kind) {
                Some(ElementKind::Server {
                    capacity,
                    queue_capacity,
                    ..
                }) => match self.field {
                    Field::Capacity => Ok(f64::from(*capacity)),
                    _ => queue_capacity
                        .map(f64::from)
                        .ok_or_else(|| invalid(format!("`{id}` has no finite buffer"))),
                },
                Some(_) => Err(invalid(format!(
                    "`{id}` is not a task with a resource of its own, so it has no {}",
                    self.field.name()
                ))),
                None => Err(not_found(format!("no element with id `{id}`"))),
            },
        }
    }

    /// Sets the parameter to `value` in `model`.
    pub fn set(&self, model: &mut Model, value: f64) -> Result<(), CommandError> {
        let id = ElementId::unchecked(&self.id);
        let count = || -> Result<u32, CommandError> {
            if value.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&value) {
                return Err(invalid(format!(
                    "{} must be a whole number, not {value}",
                    self.field.name()
                )));
            }
            Ok(value as u32)
        };
        match self.field {
            Field::Quantity => {
                let n = count()?;
                let p = model
                    .resources
                    .iter_mut()
                    .find(|p| p.id == id)
                    .ok_or_else(|| not_found(format!("no pool with id `{id}`")))?;
                set_units(p, n);
                Ok(())
            }
            Field::Mean => {
                let old = self.get(model)?;
                if !(value.is_finite() && value > 0.0 && old > 0.0) {
                    return Err(invalid(format!(
                        "a mean must be above 0, and so must the one it rescales (`{id}` has {old})"
                    )));
                }
                let d = time_mut(model, &id)
                    .ok_or_else(|| invalid(format!("`{id}` has no time distribution")))?;
                *d = d
                    .scaled(value / old)
                    .ok_or_else(|| invalid(format!("`{id}`'s distribution cannot be rescaled")))?;
                Ok(())
            }
            Field::Capacity | Field::QueueCapacity => {
                let n = count()?;
                self.get(model)?;
                if let Some(ElementKind::Server {
                    capacity,
                    queue_capacity,
                    ..
                }) = model
                    .elements
                    .iter_mut()
                    .find(|e| e.id == id)
                    .map(|e| &mut e.kind)
                {
                    match self.field {
                        Field::Capacity => *capacity = n,
                        _ => *queue_capacity = Some(n),
                    }
                }
                Ok(())
            }
        }
    }
}

fn pool<'a>(model: &'a Model, id: &ElementId) -> Result<&'a ResourcePool, CommandError> {
    model
        .resources
        .iter()
        .find(|p| &p.id == id)
        .ok_or_else(|| not_found(format!("no pool with id `{id}`")))
}

/// Sets a pool's units to `n`: its quantity, or with shifts, every count that is not 0 (a pool
/// working days and closed at night keeps its nights).
fn set_units(p: &mut ResourcePool, n: u32) {
    if p.shifts.is_empty() || p.quantity > 0 {
        p.quantity = n;
    }
    for s in &mut p.shifts {
        if s.value > 0 {
            s.value = n;
        }
    }
}

/// The time distribution of element `id` that a mean refers to.
fn time_mut<'a>(model: &'a mut Model, id: &ElementId) -> Option<&'a mut Dist> {
    match &mut model.elements.iter_mut().find(|e| &e.id == id)?.kind {
        ElementKind::Source { interarrival, .. } => Some(interarrival),
        ElementKind::Server { service, .. } | ElementKind::SharedTask { service, .. } => {
            Some(service)
        }
        ElementKind::Delay { duration } => Some(duration),
        _ => None,
    }
}

/// Runs `model`'s experiment with `replications` and `seed` and summarises it.
pub fn run(model: &Model, replications: u64, seed: u64) -> Result<ExperimentSummary, CommandError> {
    let compiled = compile(model).map_err(|ds| CommandError {
        code: ErrorCode::InvalidModel,
        message: "the model has errors and cannot run".into(),
        details: Some(wire::diagnostics(&ds)),
    })?;
    let reports = Experiment::new(replications, seed)
        .run(|r| compiled.run(r.rng))
        .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| CommandError::new(ErrorCode::TimeStopped, e.to_string()))?;
    Ok(summarise(&reports))
}

fn metric(name: &str) -> Result<Metric, CommandError> {
    Metric::from_name(name).ok_or_else(|| {
        let names: Vec<&str> = Metric::ALL.iter().map(|m| m.name()).collect();
        invalid(format!(
            "no metric `{name}`; the metrics are {}",
            names.join(", ")
        ))
    })
}

/// The mean and half-width of `o` in `s`, `None` if the element did not report it.
fn value(s: &ExperimentSummary, o: &Output) -> Result<Option<(f64, f64)>, CommandError> {
    let m = metric(&o.metric)?;
    Ok(s.get(&o.element, m).map(|x| (x.mean, x.half_width)))
}

/// `o`'s value in each replication of `s`, in replication order (empty if the element did not
/// report it).
fn samples<'a>(s: &'a ExperimentSummary, o: &Output) -> Result<&'a [Option<f64>], CommandError> {
    let m = metric(&o.metric)?;
    Ok(s.elements
        .iter()
        .find(|e| e.id.as_str() == o.element)
        .and_then(|e| e.samples(m))
        .unwrap_or(&[]))
}

/// The paired difference `b` − `a` of `o`, replication by replication, as fields of an effect:
/// `difference` (the mean of the differences), `half_width` (its 95% t half-width, n − 1
/// degrees of freedom), `n` (the replications where both have a value) and `significant` (the
/// interval excludes 0; `null` with fewer than two pairs).
fn paired(
    a: &ExperimentSummary,
    b: &ExperimentSummary,
    o: &Output,
) -> Result<serde_json::Map<String, Value>, CommandError> {
    let d = difference::paired(samples(a, o)?, samples(b, o)?);
    let mut m = serde_json::Map::new();
    m.insert("difference".into(), json!(d.map(|d| d.mean)));
    m.insert("half_width".into(), json!(d.map(|d| d.half_width)));
    m.insert("n".into(), json!(d.map_or(0, |d| d.n_a)));
    m.insert("significant".into(), json!(d.and_then(|d| d.significant())));
    Ok(m)
}

impl Target {
    fn meets(&self) -> Result<Box<dyn Fn(f64) -> bool>, CommandError> {
        match (self.at_least, self.at_most) {
            (Some(x), None) => Ok(Box::new(move |v| v >= x)),
            (None, Some(x)) => Ok(Box::new(move |v| v <= x)),
            _ => Err(invalid(
                "give the target as `at_least` or `at_most`, one of them",
            )),
        }
    }

    /// How many replications of `s` met the target, out of how many.
    fn met(&self, s: &ExperimentSummary) -> Result<Proportion, CommandError> {
        let meets = self.meets()?;
        let m = metric(&self.metric)?;
        let samples = s
            .elements
            .iter()
            .find(|e| e.id.as_str() == self.element)
            .and_then(|e| e.samples(m))
            .unwrap_or(&[]);
        // A replication with no value (nothing reached the element) does not meet it.
        let met = samples.iter().filter(|v| v.is_some_and(&meets)).count() as u64;
        Ok(Proportion::new(met, s.replications))
    }
}

// ---------------------------------------------------------------------------------------------
// Sweeps and sensitivity

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SweepParams {
    pub parameter: Param,
    /// The values to run, in order (whole numbers for counts).
    pub values: Vec<f64>,
    /// The results to report at each value.
    pub results: Vec<Output>,
    /// Replications at each value (default: the experiment's).
    #[serde(default)]
    pub replications: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SensitivityParams {
    pub parameters: Vec<Param>,
    pub results: Vec<Output>,
    /// How far each parameter is moved each way, as a fraction of its value (default 0.1);
    /// counts move by at least one unit.
    #[serde(default)]
    pub change: Option<f64>,
    #[serde(default)]
    pub replications: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
}

fn reps_seed(model: &Model, replications: Option<u32>, seed: Option<u64>) -> (u64, u64) {
    let x = &model.experiment;
    (
        u64::from(replications.unwrap_or(x.replications).max(1)),
        seed.unwrap_or(x.seed),
    )
}

/// Each result at each value of a parameter.
pub fn sweep(model: &Model, p: &SweepParams) -> Result<Value, CommandError> {
    if p.values.is_empty() || p.results.is_empty() {
        return Err(invalid("give at least one value and one result"));
    }
    let (reps, seed) = reps_seed(model, p.replications, p.seed);
    let base = p.parameter.get(model)?;
    let mut points = Vec::new();
    let mut summaries = Vec::new();
    for &v in &p.values {
        let mut m = model.clone();
        p.parameter.set(&mut m, v)?;
        let s = run(&m, reps, seed)?;
        let mut results = Vec::new();
        for o in &p.results {
            let x = value(&s, o)?;
            results.push(json!({
                "element": o.element,
                "metric": o.metric,
                "mean": x.map(|x| x.0),
                "half_width": x.map(|x| x.1),
            }));
        }
        points.push(json!({ "value": v, "results": results }));
        summaries.push(s);
    }
    // Across the range: how much each result moved, and whether more than the noise, from the
    // differences between the first and last values' replications.
    let (Some(first), Some(last)) = (summaries.first(), summaries.last()) else {
        return Err(invalid("give at least one value and one result"));
    };
    let (lo, hi) = (p.values.first(), p.values.last());
    let effects: Vec<Value> = p
        .results
        .iter()
        .map(|o| {
            let mut e = paired(first, last, o)?;
            // How much the result moved: the paired difference.
            let change = e.remove("difference").unwrap_or(Value::Null);
            let per_unit = change
                .as_f64()
                .zip(lo.zip(hi))
                .and_then(|(c, (a, b))| (b != a).then(|| c / (b - a)));
            e.insert("element".into(), json!(o.element));
            e.insert("metric".into(), json!(o.metric));
            e.insert("change".into(), change);
            e.insert("per_unit".into(), json!(per_unit));
            Ok(Value::Object(e))
        })
        .collect::<Result<_, CommandError>>()?;
    Ok(json!({
        "parameter": p.parameter,
        "current": base,
        "replications": reps,
        "seed": seed,
        "points": points,
        "effects": effects,
        "method": "one run per value with the same seed (common random numbers), so replication i of every value meets the same random numbers; change: the mean over the replications of the result at the last value minus at the first, with half_width its 95% paired-t half-width (n − 1 degrees of freedom, n the replications where both have a value); significant: that interval excludes 0 (null with fewer than two pairs)",
    }))
}

/// One parameter at a time, moved down and up: the elasticity of each result, ranked.
pub fn sensitivity(model: &Model, p: &SensitivityParams) -> Result<Value, CommandError> {
    if p.parameters.is_empty() || p.results.is_empty() {
        return Err(invalid("give at least one parameter and one result"));
    }
    let change = p.change.unwrap_or(0.1);
    if !(change > 0.0 && change < 1.0) {
        return Err(invalid("`change` must be between 0 and 1"));
    }
    let (reps, seed) = reps_seed(model, p.replications, p.seed);
    let base = run(model, reps, seed)?;
    let mut rows = Vec::new();
    for param in &p.parameters {
        let x0 = param.get(model)?;
        let step = if param.field.integral() {
            (x0 * change).round().max(1.0)
        } else {
            x0 * change
        };
        // Units cannot go below one, nor anything below zero: a parameter at its floor is
        // compared one-sided, from its own value up.
        let floor: f64 = match param.field {
            Field::Capacity | Field::Quantity => 1.0,
            Field::QueueCapacity | Field::Mean => 0.0,
        };
        let (down, up) = ((x0 - step).max(floor.min(x0)), x0 + step);
        let at = |v: f64| -> Result<ExperimentSummary, CommandError> {
            let mut m = model.clone();
            param.set(&mut m, v)?;
            run(&m, reps, seed)
        };
        let s_down = if down == x0 { base.clone() } else { at(down)? };
        let s_up = at(up)?;
        for o in &p.results {
            let (y0, y_down, y_up) = (value(&base, o)?, value(&s_down, o)?, value(&s_up, o)?);
            let elasticity = match (y0, y_down, y_up) {
                (Some((y0, _)), Some((a, _)), Some((b, _))) if y0 != 0.0 && x0 != 0.0 => {
                    Some(((b - a) / y0) / ((up - down) / x0))
                }
                _ => None,
            };
            let mut row = json!({
                "parameter": param,
                "value": x0,
                "down": down,
                "up": up,
                "element": o.element,
                "metric": o.metric,
                "result": y0.map(|y| y.0),
                "at_down": y_down.map(|y| y.0),
                "at_up": y_up.map(|y| y.0),
                "elasticity": elasticity,
            });
            // The effect, up − down replication by replication, and whether it is more than
            // the noise.
            if let Some(r) = row.as_object_mut() {
                r.extend(paired(&s_down, &s_up, o)?);
            }
            rows.push(row);
        }
    }
    let tests = rows.len() as u64;
    // Largest effect first.
    let size = |r: &Value| r["elasticity"].as_f64().map_or(-1.0, f64::abs);
    rows.sort_by(|a, b| size(b).total_cmp(&size(a)));
    Ok(json!({
        "change": change,
        "replications": reps,
        "seed": seed,
        "effects": rows,
        "tests": tests,
        "method": "one parameter at a time, down and up by `change`, the same seed throughout (common random numbers), so replication i of every run meets the same random numbers; elasticity: the relative change in the result over the relative change in the parameter; difference: the mean over the replications of the result at up minus at down, with half_width its 95% paired-t half-width (n − 1 degrees of freedom, n the replications where both have a value); significant: that interval excludes 0 (null with fewer than two pairs). Each of the `tests` effects is tested on its own at 95%, so about one in twenty of those with no real effect still shows as significant by chance; for 95% confidence in all of them at once (Bonferroni), each would need its own interval at 1 − 0.05/tests, which is wider",
    }))
}

// ---------------------------------------------------------------------------------------------
// Capacity sizing

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SizeParams {
    pub target: Target,
    /// The share of replications that must meet the target, such as 0.7: a share of the
    /// replications run, not a statistical confidence (the answer's Wilson interval says how
    /// sure that share is). `confidence`, its old name, is still read.
    #[serde(alias = "confidence")]
    pub target_share: f64,
    /// The tasks (with a resource of their own) and pools that may grow; by default every one.
    #[serde(default)]
    pub resources: Option<Vec<String>>,
    /// The most units to add in all (default 50).
    #[serde(default)]
    pub max_added: Option<u32>,
    #[serde(default)]
    pub replications: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Make the stochastic answer the model's (one undoable edit).
    #[serde(default)]
    pub apply: bool,
}

/// What can grow: a task's or a pool's units.
fn candidates(model: &Model, only: Option<&[String]>) -> Result<Vec<Param>, CommandError> {
    let all: Vec<Param> = model
        .elements
        .iter()
        .filter(|e| matches!(e.kind, ElementKind::Server { .. }))
        .map(|e| Param {
            id: e.id.to_string(),
            field: Field::Capacity,
        })
        .chain(model.resources.iter().map(|p| Param {
            id: p.id.to_string(),
            field: Field::Quantity,
        }))
        .collect();
    let Some(only) = only else {
        return Ok(all);
    };
    only.iter()
        .map(|id| {
            all.iter().find(|c| &c.id == id).cloned().ok_or_else(|| {
                invalid(format!(
                    "`{id}` is neither a task with a resource of its own nor a pool"
                ))
            })
        })
        .collect()
}

/// The model with every time distribution replaced by its mean: what a spreadsheet of average
/// times would size.
pub fn deterministic(model: &Model) -> Model {
    let mut m = model.clone();
    let mean = |d: &mut Dist| {
        if let Some(x) = d.mean() {
            *d = Dist::Constant(x);
        }
    };
    for e in &mut m.elements {
        match &mut e.kind {
            ElementKind::Source {
                interarrival,
                timed,
                ..
            } => {
                mean(interarrival);
                timed.iter_mut().for_each(|t| mean(&mut t.value));
            }
            ElementKind::Server {
                service, breakdown, ..
            } => {
                mean(service);
                if let Some(b) = breakdown {
                    mean(&mut b.time_to_failure);
                    mean(&mut b.repair);
                }
            }
            ElementKind::SharedTask { service, .. } => mean(service),
            ElementKind::Delay { duration } => mean(duration),
            _ => {}
        }
    }
    m
}

/// The outcome of sizing one model.
struct Sized {
    model: Model,
    met: Proportion,
    steps: Vec<Value>,
    reached: bool,
}

fn grow(
    model: &Model,
    p: &SizeParams,
    resources: &[Param],
    reps: u64,
    seed: u64,
) -> Result<Sized, CommandError> {
    let mut m = model.clone();
    let mut steps = Vec::new();
    let limit = p.max_added.unwrap_or(50);
    for added in 0..=limit {
        let s = run(&m, reps, seed)?;
        let met = p.target.met(&s)?;
        let probability = met.estimate().unwrap_or(0.0);
        // The busiest of the resources that may grow.
        let busiest = resources
            .iter()
            .filter_map(|r| {
                let u = s.get(&r.id, Metric::Utilisation)?.mean;
                Some((r, u))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));
        let units: serde_json::Map<String, Value> = resources
            .iter()
            .filter_map(|r| Some((r.id.clone(), json!(r.get(&m).ok()?))))
            .collect();
        let mut step = json!({
            "units": units,
            "probability": probability,
            "interval": met.wilson(0.95),
        });
        if probability >= p.target_share {
            steps.push(step);
            return Ok(Sized {
                model: m,
                met,
                steps,
                reached: true,
            });
        }
        let Some((r, u)) = busiest else {
            return Err(invalid(
                "none of the resources that may grow reports a utilisation (nothing used them)",
            ));
        };
        step["bottleneck"] = json!({ "id": r.id, "utilisation": u });
        steps.push(step);
        if added == limit {
            break;
        }
        let now = r.get(&m)?;
        r.set(&mut m, now + 1.0)?;
    }
    let s = run(&m, reps, seed)?;
    Ok(Sized {
        met: p.target.met(&s)?,
        model: m,
        steps,
        reached: false,
    })
}

/// Adds a unit at the bottleneck until the share of replications meeting the target reaches the
/// target share; the same with every time at its mean beside it. Returns the answer and the
/// model sized stochastically.
///
/// The share is the one observed in the replications run. Its Wilson interval at 95% is in the
/// answer: when the interval's lower end is below the target share, the share is not shown to
/// be met with 95% confidence (more replications would narrow the interval).
pub fn size(model: &Model, p: &SizeParams) -> Result<(Value, Model), CommandError> {
    if !(p.target_share > 0.0 && p.target_share <= 1.0) {
        return Err(invalid("`target_share` must be above 0 and at most 1"));
    }
    drop(p.target.meets()?);
    metric(&p.target.metric)?;
    let resources = candidates(model, p.resources.as_deref())?;
    if resources.is_empty() {
        return Err(invalid(
            "the model has no task with a resource of its own and no pool",
        ));
    }
    let (reps, seed) = reps_seed(model, p.replications, p.seed);
    let stochastic = grow(model, p, &resources, reps, seed)?;
    // One replication is the whole story when nothing is random.
    let fixed = grow(&deterministic(model), p, &resources, 1, seed)?;
    let units = |m: &Model| -> serde_json::Map<String, Value> {
        resources
            .iter()
            .filter_map(|r| Some((r.id.clone(), json!(r.get(m).ok()?))))
            .collect()
    };
    let answer = json!({
        "target": p.target,
        "target_share": p.target_share,
        // The old name of `target_share`, kept for readers of earlier answers.
        "confidence": p.target_share,
        "replications": reps,
        "seed": seed,
        "stochastic": {
            "units": units(&stochastic.model),
            "reached": stochastic.reached,
            "probability": stochastic.met.estimate(),
            "interval": stochastic.met.wilson(0.95),
            "steps": stochastic.steps,
        },
        "deterministic": {
            "units": units(&fixed.model),
            "reached": fixed.reached,
            "steps": fixed.steps,
        },
        "method": "run, add a unit to the busiest resource that may grow (highest utilisation), run again, until the share of replications meeting the target reaches the target share (`confidence` in earlier answers: a share of the replications run, not a statistical confidence); deterministic: the same with every time distribution replaced by its mean and one replication; the interval is Wilson's at 95% for the share meeting the target, and when its lower end is below the target share, the share is not shown to be met with 95% confidence",
    });
    Ok((answer, stochastic.model))
}
