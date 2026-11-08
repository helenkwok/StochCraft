//! The distribution of one result across the replications of the last run: percentiles, the
//! empirical cumulative distribution with its Dvoretzky–Kiefer–Wolfowitz band, the probability
//! of meeting a target with Wilson's interval, and the values as an empirical BPSim user
//! distribution for another model (`results.distribution`).

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_model::Dist;
use stochcraft_stats::{Ecdf, Proportion, Summary};

use crate::wire::DistSpec;
use crate::{CommandError, ErrorCode};

/// Which result, and what to ask of it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Params {
    /// The element whose metric it is; left out for `stop_time`.
    #[serde(default)]
    pub element: Option<String>,
    /// A metric's name, as the results list them, or `stop_time`: when each replication met the
    /// experiment's stop condition (one that did not counts as later than its time limit).
    pub metric: String,
    /// A target the result should reach: the probability that a replication's value is at least
    /// this, with its interval.
    #[serde(default)]
    pub at_least: Option<f64>,
    /// A target the result should not exceed: the probability that a replication's value is at
    /// most this, with its interval.
    #[serde(default)]
    pub at_most: Option<f64>,
    /// For the band and the interval; 0.95 by default.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// The percentiles to report, from 0 to 100; 5, 10, 25, 50, 75, 90 and 95 by default.
    #[serde(default)]
    pub percentiles: Option<Vec<f64>>,
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParams, message)
}

/// The values the run's `results` hold for `p`'s metric, one per replication (`None` where a
/// replication had none; +∞ for a stop not met).
fn values(results: &Value, p: &Params) -> Result<Vec<Option<f64>>, CommandError> {
    if p.metric == "stop_time" {
        let stop = results
            .get("stop")
            .ok_or_else(|| invalid("the run had no stop condition, so there is no stop_time"))?;
        let times = stop["times"].as_array().cloned().unwrap_or_default();
        return Ok(times
            .iter()
            .map(|t| Some(t.as_f64().unwrap_or(f64::INFINITY)))
            .collect());
    }
    let element = p
        .element
        .as_deref()
        .ok_or_else(|| invalid("give the `element` whose metric it is"))?;
    let per = results["values"].get(element).ok_or_else(|| {
        CommandError::new(
            ErrorCode::NotFound,
            format!("the last run has no results for `{element}`"),
        )
    })?;
    let list = per.get(&p.metric).and_then(Value::as_array).ok_or_else(|| {
        CommandError::new(
            ErrorCode::NotFound,
            format!(
                "`{element}` reported no `{}` in the last run (the metrics are listed with the results)",
                p.metric
            ),
        )
    })?;
    Ok(list.iter().map(Value::as_f64).collect())
}

/// The values as a continuous empirical distribution: linear between the sorted values, each
/// step between neighbours equally likely (Law, *Simulation Modeling and Analysis*, 5th ed.,
/// §6.2.4), written as a BPSim user distribution; a constant for a single distinct value.
pub fn empirical(sorted: &[f64]) -> Option<Dist> {
    let (first, last) = (sorted.first()?, sorted.last()?);
    if first == last {
        return Some(Dist::Constant(*first));
    }
    let step = 1.0 / (sorted.len() - 1) as f64;
    let points = sorted
        .iter()
        .enumerate()
        .map(|(i, v)| (if i == 0 { 0.0 } else { step }, *v))
        .collect();
    Some(Dist::User {
        points,
        discrete: false,
    })
}

/// The answer to `results.distribution` from a run's `results`.
pub fn describe(results: &Value, p: &Params) -> Result<Value, CommandError> {
    let confidence = p.confidence.unwrap_or(0.95);
    if !(confidence > 0.0 && confidence < 1.0) {
        return Err(invalid("`confidence` must be between 0 and 1"));
    }
    let percentiles = p
        .percentiles
        .clone()
        .unwrap_or_else(|| vec![5.0, 10.0, 25.0, 50.0, 75.0, 90.0, 95.0]);
    if let Some(bad) = percentiles.iter().find(|q| !(0.0..=100.0).contains(*q)) {
        return Err(invalid(format!(
            "percentile {bad} is not between 0 and 100"
        )));
    }
    let each = values(results, p)?;
    let present: Vec<f64> = each.iter().flatten().copied().collect();
    if present.is_empty() {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "no replication of the last run has a value for it",
        ));
    }
    let ecdf = Ecdf::new(&present);
    let finite = ecdf.finite();
    let band = ecdf.dkw_half_width(confidence);
    let pct: serde_json::Map<String, Value> = percentiles
        .iter()
        .map(|q| (format!("{q}"), json!(ecdf.quantile(q / 100.0))))
        .collect();
    let summary = Summary::of(finite);
    let mut v = json!({
        "metric": p.metric,
        "element": p.element,
        "replications": each.len(),
        "n": ecdf.n(),
        "censored": ecdf.n() - finite.len(),
        "mean": summary.map(|s| s.mean),
        "half_width": summary.map(|s| s.half_width),
        "min": finite.first(),
        "max": finite.last(),
        "percentiles": pct,
        "confidence": confidence,
        "ecdf": ecdf.steps(),
        "band": band,
        "method": "percentiles: Hyndman and Fan's definition 7; band: Dvoretzky–Kiefer–Wolfowitz with Massart's constant, ε = √(ln(2/α) / 2n); target probability: Wilson's score interval",
    });
    let target = |name: &str, x: f64, meets: &dyn Fn(f64) -> bool| {
        let met = present.iter().filter(|v| meets(**v)).count() as u64;
        let prop = Proportion::new(met, present.len() as u64);
        json!({
            name: x,
            "met": met,
            "of": present.len(),
            "probability": prop.estimate(),
            "interval": prop.wilson(confidence),
        })
    };
    match (p.at_least, p.at_most) {
        (Some(_), Some(_)) => return Err(invalid("give `at_least` or `at_most`, not both")),
        (Some(x), None) => v["target"] = target("at_least", x, &|v| v >= x),
        (None, Some(x)) => v["target"] = target("at_most", x, &|v| v <= x),
        (None, None) => {}
    }
    // As an input for another model: the finite values only.
    if let Some(d) = empirical(finite) {
        v["as_distribution"] = serde_json::to_value(DistSpec::from(&d)).unwrap_or(Value::Null);
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empirical_distribution_through_the_values() {
        let d = empirical(&[1.0, 2.0, 4.0]);
        assert_eq!(
            d,
            Some(Dist::User {
                points: vec![(0.0, 1.0), (0.5, 2.0), (0.5, 4.0)],
                discrete: false
            })
        );
        // Its mean is that of the piecewise-linear distribution: (1.5 + 3) / 2.
        assert_eq!(d.and_then(|d| d.mean()), Some(2.25));
        assert_eq!(empirical(&[3.0, 3.0]), Some(Dist::Constant(3.0)));
        assert_eq!(empirical(&[]), None);
    }
}
