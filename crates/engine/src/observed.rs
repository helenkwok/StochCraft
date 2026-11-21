//! Observed data: a run's results checked against real measurements (black-box validation).
//!
//! An experiment carries [`Observation`]s ([`stochcraft_model::Experiment::observed`]), each
//! naming a result the run reports (a block's or pool's metric, by the names the results use)
//! and either an observed value or a bound. The rules are
//! [`stochcraft_stats::observed::judge`]'s: an observed value agrees when the 95% interval of
//! (simulated mean − observed) holds 0; a bound passes or fails only when the whole simulated 95%
//! interval does, and is otherwise not yet decided.
//!
//! The results carry the checks as `observed` (left out when there are none): an array with one
//! entry for each observation, those that differ or fail first, the rest in the order they are
//! kept. An entry has the `element` (id) and its `name`, the `metric`, the `check` (`value`,
//! `at_most` or `at_least`), the `observed` value and its own `half_width` (`null` when not
//! given) or the `bound`, the `simulated` result (`mean`, 95% `half_width`, `n`; `null` when the
//! run has no such result), for an observed value the `difference` (simulated minus observed)
//! with the 95% `difference_half_width`, and the `verdict`: `agrees`, `differs`, `passes`,
//! `fails`, `cant_tell` (no interval yet, or a bound the interval straddles) or `no_result`
//! (the run reports no such result: the block is gone, or does not report that metric).

use serde_json::{Value, json};
use stochcraft_blocks::{ExperimentSummary, Metric};
use stochcraft_model::{ElementId, Model, Observation, Observed};
use stochcraft_stats::observed::{Check, Outcome, judge};

use crate::wire::ObservationSpec;
use crate::{CommandError, ErrorCode};

/// What `o` is checked against, for [`judge`].
fn check_of(o: &Observation) -> Check {
    match o.target {
        Observed::Value { value, half_width } => Check::Value { value, half_width },
        Observed::AtMost(bound) => Check::AtMost(bound),
        Observed::AtLeast(bound) => Check::AtLeast(bound),
    }
}

/// How an observation is named in the results' `check`.
fn kind_name(o: &Observation) -> &'static str {
    match o.target {
        Observed::Value { .. } => "value",
        Observed::AtMost(_) => "at_most",
        Observed::AtLeast(_) => "at_least",
    }
}

/// The check of one observation against the run's results.
fn one(model: &Model, summary: &ExperimentSummary, o: &Observation) -> Value {
    let found = summary.elements.iter().find(|e| e.id == o.element);
    let simulated = Metric::from_name(&o.metric)
        .and_then(|m| found.and_then(|e| e.metric(m)))
        .copied();
    let name = found.map_or_else(|| model.display_name(&o.element), |e| e.name.clone());
    let mut v = json!({
        "element": o.element.as_str(),
        "name": name,
        "metric": o.metric,
        "check": kind_name(o),
        "simulated": simulated.map(|s| json!({
            "mean": s.mean,
            "half_width": s.half_width,
            "n": s.n,
        })),
    });
    match o.target {
        Observed::Value { value, half_width } => {
            v["observed"] = json!({ "value": value, "half_width": half_width });
        }
        Observed::AtMost(bound) | Observed::AtLeast(bound) => v["bound"] = json!(bound),
    }
    let Some(s) = simulated else {
        v["verdict"] = json!("no_result");
        return v;
    };
    let verdict = judge(check_of(o), s.mean, s.half_width, s.n);
    v["verdict"] = json!(verdict.outcome.name());
    if matches!(o.target, Observed::Value { .. }) {
        v["difference"] = json!(verdict.difference);
        v["difference_half_width"] = json!(verdict.half_width);
    }
    v
}

/// The checks of every observation of `model`'s experiment against `summary`: see the module's
/// description. `None` when the experiment has no observed data.
pub fn check(model: &Model, summary: &ExperimentSummary) -> Option<Value> {
    let list = &model.experiment.observed;
    if list.is_empty() {
        return None;
    }
    let checks: Vec<Value> = list.iter().map(|o| one(model, summary, o)).collect();
    let bad = |c: &Value| {
        matches!(
            c["verdict"].as_str(),
            Some(v) if v == Outcome::Differs.name() || v == Outcome::Fails.name()
        )
    };
    // Those that differ or fail first; the rest as kept.
    let (first, rest): (Vec<Value>, Vec<Value>) = checks.into_iter().partition(bad);
    Some(Value::Array(first.into_iter().chain(rest).collect()))
}

/// The observation a command's `spec` gives, checked against `model`: the block or pool exists,
/// the metric has a name the results use, and the numbers are finite (a half-width not below 0).
pub fn from_spec(model: &Model, spec: ObservationSpec) -> Result<Observation, CommandError> {
    let bad = |what: String| CommandError::new(ErrorCode::InvalidParams, what);
    let id = ElementId::unchecked(spec.element());
    if model.element(&id).is_none() && !model.resources.iter().any(|r| r.id == id) {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            format!("no element or pool with id `{id}`"),
        ));
    }
    if Metric::from_name(spec.metric()).is_none() {
        return Err(bad(format!(
            "no metric named `{}`: use a name the results use, such as `mean_wait`",
            spec.metric()
        )));
    }
    let number = |name: &str, v: f64| {
        if v.is_finite() {
            Ok(())
        } else {
            Err(bad(format!("`{name}` must be a number, not {v}")))
        }
    };
    match &spec {
        ObservationSpec::Value {
            value, half_width, ..
        } => {
            number("value", *value)?;
            if let Some(h) = half_width
                && !(h.is_finite() && *h >= 0.0)
            {
                return Err(bad(format!("`half_width` must be 0 or more, not {h}")));
            }
        }
        ObservationSpec::AtMost { bound, .. } | ObservationSpec::AtLeast { bound, .. } => {
            number("bound", *bound)?;
        }
    }
    Ok(spec.into())
}
