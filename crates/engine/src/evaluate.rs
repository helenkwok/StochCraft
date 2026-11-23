//! Evaluating given values of the decisions from outside (M9, stage P1): the model with each
//! decision set to its value, run for a range of replications, and each result's value in every
//! one of them, with their mean and 95% interval.
//!
//! This is what an optimiser of the user's own (Optuna, pymoo, or code in any language) needs
//! to drive the runs. Every evaluation uses the same seed, so replication i of every candidate
//! meets the same random numbers (common random numbers; A. M. Law, *Simulation Modeling and
//! Analysis*, 5th ed., McGraw-Hill, 2015, section 11.2), and two candidates can be compared
//! replication by replication. Replication i's random stream depends only on the seed and i
//! (`stochcraft_experiment::Experiment::stream`), so replications 0 to 9 and then 10 to 19 give
//! exactly what 0 to 19 give: an optimiser can add replications in stages and stop early, as a
//! pruner does.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_model::{ElementId, Model};
use stochcraft_stats::Summary;

use crate::decide::{Field, Output, Param, invalid, metric, run_range, samples};
use crate::design::MAX_RUNS;
use crate::{CommandError, ErrorCode};

/// A decision and its value: a parameter as a sweep takes it (`{id, field}`), the value to
/// evaluate, and optionally the range the value must lie in.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    /// The element or pool.
    pub id: String,
    pub field: Field,
    /// The value to evaluate (a whole number for counts).
    #[schemars(with = "f64")]
    pub value: Value,
    /// The least value allowed, if the caller's search has one.
    #[serde(default)]
    pub min: Option<f64>,
    /// The greatest value allowed, if the caller's search has one.
    #[serde(default)]
    pub max: Option<f64>,
}

impl Decision {
    fn param(&self) -> Param {
        Param {
            id: self.id.clone(),
            field: self.field,
        }
    }

    /// `id.field`, as the command line names it.
    fn label(&self) -> String {
        format!("{}.{}", self.id, self.field.name())
    }

    /// The value as a number, checked against its kind and range: refused with the decision
    /// named, never adjusted.
    fn checked_value(&self) -> Result<f64, CommandError> {
        let name = self.label();
        let v = self
            .value
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| {
                invalid(format!(
                    "decision `{name}`: the value must be a number, not {}",
                    self.value
                ))
            })?;
        if self.field.integral() && v.fract() != 0.0 {
            return Err(invalid(format!(
                "decision `{name}`: the value must be a whole number, not {v}"
            )));
        }
        // The field's own range: at least one unit, a buffer of at least none, a mean above 0.
        let (floor, open) = match self.field {
            Field::Capacity | Field::Quantity => (1.0, false),
            Field::QueueCapacity => (0.0, false),
            Field::Mean => (0.0, true),
        };
        let too_big = self.field.integral() && v > f64::from(u32::MAX);
        if v < floor || (open && v == floor) || too_big {
            let least = if open { "above" } else { "at least" };
            return Err(invalid(format!(
                "decision `{name}`: the value {v} is out of its range (a {} must be {least} {floor})",
                self.field.name()
            )));
        }
        if let (Some(lo), Some(hi)) = (self.min, self.max)
            && lo > hi
        {
            return Err(invalid(format!(
                "decision `{name}`: its range is empty (min {lo} is above max {hi})"
            )));
        }
        if self.min.is_some_and(|lo| v < lo) || self.max.is_some_and(|hi| v > hi) {
            let bound = |b: Option<f64>| b.map_or_else(|| "none".to_string(), |b| b.to_string());
            return Err(invalid(format!(
                "decision `{name}`: the value {v} is out of its range (min {}, max {})",
                bound(self.min),
                bound(self.max)
            )));
        }
        Ok(v)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluateParams {
    /// The decisions and their values; the model's other numbers are kept. Each decision at
    /// most once.
    pub decisions: Vec<Decision>,
    /// The results to report.
    pub results: Vec<Output>,
    /// The first replication, counting from 0 (default 0).
    #[serde(default)]
    pub first: Option<u64>,
    /// How many replications from `first` (default: the experiment's replications).
    #[serde(default)]
    pub count: Option<u64>,
    /// The seed (default: the experiment's).
    #[serde(default)]
    pub seed: Option<u64>,
    /// Run although the file had errors StochCraft could not read, with what it substituted
    /// for each (as for `run`); the answer then lists them (`approximations`).
    #[serde(default)]
    pub allow_approximations: bool,
}

/// The model with each decision set to its value: unknown decisions, values of the wrong kind
/// or out of their range, and a decision given twice are refused with the decision named.
fn decided(model: &Model, decisions: &[Decision]) -> Result<(Model, Vec<Value>), CommandError> {
    let mut m = model.clone();
    let mut echo = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for d in decisions {
        let name = d.label();
        if seen.contains(&name) {
            return Err(invalid(format!(
                "decision `{name}` is given more than once"
            )));
        }
        seen.push(name.clone());
        let v = d.checked_value()?;
        let named = |e: CommandError| CommandError {
            message: format!("decision `{name}`: {}", e.message),
            ..e
        };
        let p = d.param();
        // Whether the model has this decision at all, before changing anything.
        let id = ElementId::unchecked(&d.id);
        let exists = match d.field {
            Field::Quantity => m.resources.iter().any(|r| r.id == id),
            _ => m.element(&id).is_some(),
        };
        if !exists {
            let what = if d.field == Field::Quantity {
                "pool"
            } else {
                "element"
            };
            return Err(CommandError::new(
                ErrorCode::NotFound,
                format!("decision `{name}`: no {what} with id `{}`", d.id),
            ));
        }
        p.get(&m).map_err(named)?;
        p.set(&mut m, v).map_err(named)?;
        echo.push(json!({ "id": d.id, "field": d.field, "value": v }));
    }
    Ok((m, echo))
}

/// A number for JSON, `null` when it is not finite (the interval of a single replication).
fn finite(x: f64) -> Value {
    if x.is_finite() { json!(x) } else { Value::Null }
}

/// The results of `model` with the decisions' values, for a range of replications.
pub fn evaluate(model: &Model, p: &EvaluateParams) -> Result<Value, CommandError> {
    if p.results.is_empty() {
        return Err(invalid("give at least one result"));
    }
    for o in &p.results {
        metric(&o.metric)?;
        let id = ElementId::unchecked(&o.element);
        if model.element(&id).is_none() && !model.resources.iter().any(|r| r.id == id) {
            return Err(CommandError::new(
                ErrorCode::NotFound,
                format!(
                    "result `{}.{}`: no element or pool with id `{}`",
                    o.element, o.metric, o.element
                ),
            ));
        }
    }
    let x = &model.experiment;
    let first = p.first.unwrap_or(0);
    let count = p.count.unwrap_or(u64::from(x.replications));
    let seed = p.seed.unwrap_or(x.seed);
    if count == 0 {
        return Err(invalid("`count` must be at least 1"));
    }
    if count > MAX_RUNS {
        return Err(invalid(format!(
            "`count` is {count}, more than the {MAX_RUNS} replications one evaluation may take: evaluate them in stages"
        )));
    }
    let end = first
        .checked_add(count)
        .ok_or_else(|| invalid("`first` + `count` is too large"))?;
    let (m, decisions) = decided(model, &p.decisions)?;
    let (s, warnings) = run_range(&m, first..end, seed)?;
    let len = usize::try_from(count).unwrap_or(usize::MAX);
    let mut results = Vec::new();
    for o in &p.results {
        let each = samples(&s, o)?;
        let values: Vec<Option<f64>> = if each.is_empty() {
            vec![None; len]
        } else {
            each.to_vec()
        };
        let present: Vec<f64> = values.iter().flatten().copied().collect();
        let summary = Summary::of(&present);
        results.push(json!({
            "element": o.element,
            "metric": o.metric,
            "values": values,
            "mean": summary.map(|x| finite(x.mean)),
            "half_width": summary.map(|x| finite(x.half_width)),
            "n": summary.map_or(0, |x| x.n),
        }));
    }
    Ok(json!({
        "decisions": decisions,
        "seed": seed,
        "first": first,
        "count": count,
        "results": results,
        "warnings": warnings,
        "method": "the model with each decision set to its value, run for replications `first` to `first` + `count` − 1 with the seed given (common random numbers: replication i of every evaluation meets the same random numbers, and depends only on the seed and i, so replications evaluated in stages give exactly what they give evaluated at once); values: each result in each of those replications, in order (null where a replication has none); mean and half_width: their mean and its 95% t half-width (n − 1 degrees of freedom, n the replications with a value; null with one)",
    }))
}
