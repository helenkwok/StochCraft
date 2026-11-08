//! Comparing the results of two runs (`results.compare`): per element and metric, both means and
//! the difference b − a with a 95% confidence interval (A. M. Law, *Simulation Modeling and
//! Analysis*, 5th ed., section 10.2).
//!
//! Two runs with the same seed and the same number of replications share their random numbers
//! replication by replication (common random numbers), so their replications are paired and the
//! difference is judged by the paired-t interval (section 10.2.1). Otherwise the runs are
//! independent and Welch's two-sample t interval is used (section 10.2.2).

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_blocks::Metric;
use stochcraft_stats::{Difference, difference};

use crate::decide::Output;
use crate::{CommandError, ErrorCode};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Params {
    /// The first run's results, as `run` or `run.status` returned them (at least `seed`,
    /// `replications` and `values`); by default the run before the last.
    #[serde(default)]
    pub a: Option<Value>,
    /// The second run's results; by default the last run.
    #[serde(default)]
    pub b: Option<Value>,
    /// The results to compare; by default every element's metrics that both runs report.
    #[serde(default)]
    pub results: Option<Vec<Output>>,
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParams, message)
}

/// What a run's results say about how it was run, and its per-replication values.
struct RunResults<'a> {
    seed: u64,
    replications: u64,
    values: &'a serde_json::Map<String, Value>,
    /// The elements in the order the results list them.
    order: Vec<&'a str>,
}

impl<'a> RunResults<'a> {
    fn read(v: &'a Value, which: &str) -> Result<Self, CommandError> {
        let field = |k: &str| {
            v.get(k)
                .and_then(Value::as_u64)
                .ok_or_else(|| invalid(format!("`{which}` has no `{k}`: give a run's results")))
        };
        let values = v.get("values").and_then(Value::as_object).ok_or_else(|| {
            invalid(format!(
                "`{which}` has no per-replication `values`: give a run's results"
            ))
        })?;
        let mut order: Vec<&str> = v
            .get("elements")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| e["id"].as_str())
            .filter(|id| values.contains_key(*id))
            .collect();
        for id in values.keys() {
            if !order.contains(&id.as_str()) {
                order.push(id);
            }
        }
        Ok(Self {
            seed: field("seed")?,
            replications: field("replications")?,
            values,
            order,
        })
    }

    /// The metric's value in each replication (`None` where it had none); `None` if the run
    /// has no values for it at all.
    fn samples(&self, element: &str, metric: &str) -> Option<Vec<Option<f64>>> {
        let list = self.values.get(element)?.get(metric)?.as_array()?;
        Some(list.iter().map(Value::as_f64).collect())
    }
}

fn mean(xs: &[Option<f64>]) -> (Option<f64>, u64) {
    let v: Vec<f64> = xs
        .iter()
        .flatten()
        .copied()
        .filter(|x| x.is_finite())
        .collect();
    let n = v.len() as u64;
    ((n > 0).then(|| v.iter().sum::<f64>() / n as f64), n)
}

/// Compares `a` with `b` (results as `run` returns them).
pub fn compare(a: &Value, b: &Value, only: Option<&[Output]>) -> Result<Value, CommandError> {
    let (ra, rb) = (RunResults::read(a, "a")?, RunResults::read(b, "b")?);
    let paired = ra.seed == rb.seed && ra.replications == rb.replications;
    let wanted: Vec<(String, String)> = match only {
        Some(list) => {
            if list.is_empty() {
                return Err(invalid("give at least one result, or leave `results` out"));
            }
            for o in list {
                if Metric::from_name(&o.metric).is_none() {
                    let names: Vec<&str> = Metric::ALL.iter().map(|m| m.name()).collect();
                    return Err(invalid(format!(
                        "no metric `{}`; the metrics are {}",
                        o.metric,
                        names.join(", ")
                    )));
                }
            }
            list.iter()
                .map(|o| (o.element.clone(), o.metric.clone()))
                .collect()
        }
        // Every element and metric both runs report, in a's order.
        None => ra
            .order
            .iter()
            .flat_map(|id| {
                Metric::ALL
                    .iter()
                    .map(move |m| ((*id).to_owned(), m.name().to_owned()))
            })
            .filter(|(id, m)| ra.samples(id, m).is_some() && rb.samples(id, m).is_some())
            .collect(),
    };
    let comparisons: Vec<Value> = wanted
        .iter()
        .map(|(element, metric)| {
            let (xa, xb) = (
                ra.samples(element, metric).unwrap_or_default(),
                rb.samples(element, metric).unwrap_or_default(),
            );
            let ((mean_a, n_a), (mean_b, n_b)) = (mean(&xa), mean(&xb));
            let d: Option<Difference> = if paired {
                difference::paired(&xa, &xb)
            } else {
                let flat = |xs: &[Option<f64>]| xs.iter().flatten().copied().collect::<Vec<_>>();
                difference::welch(&flat(&xa), &flat(&xb))
            };
            let mut c = json!({
                "element": element,
                "metric": metric,
                "a": { "mean": mean_a, "n": n_a },
                "b": { "mean": mean_b, "n": n_b },
                "difference": d.map(|d| d.mean),
                "half_width": d.map(|d| d.half_width),
                "df": d.and_then(|d| d.df),
                "significant": d.and_then(|d| d.significant()),
                "interval": if paired { "paired_t" } else { "welch_t" },
            });
            if paired {
                c["pairs"] = json!(d.map_or(0, |d| d.n_a));
            }
            c
        })
        .collect();
    let mut notes = Vec::new();
    for k in ["time_unit", "warmup", "duration"] {
        if let (Some(x), Some(y)) = (a.get(k), b.get(k))
            && x != y
        {
            notes.push(format!("the runs differ in `{k}` ({x} and {y})"));
        }
    }
    let method = if paired {
        "the runs have the same seed and replications, so replication i of each meets the same random numbers (common random numbers) and they are compared in pairs: difference is the mean over the replications where both have a value (pairs) of b minus a, with half_width its 95% paired-t half-width (pairs − 1 degrees of freedom); significant: that interval excludes 0 (null with fewer than two pairs)"
    } else {
        "the runs differ in seed or replications, so they are independent: difference is b's mean minus a's, with half_width its 95% Welch two-sample t half-width (Welch–Satterthwaite degrees of freedom, rounded down); significant: that interval excludes 0 (null when either run has fewer than two values). Running both with the same seed and replications gives a paired comparison, usually much tighter"
    };
    Ok(json!({
        "a": { "seed": ra.seed, "replications": ra.replications },
        "b": { "seed": rb.seed, "replications": rb.replications },
        "paired": paired,
        "comparisons": comparisons,
        "notes": notes,
        "method": method,
    }))
}
