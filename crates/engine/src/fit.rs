//! Fitting a distribution to measured data (`estimate.fit`): each candidate family fitted by
//! maximum likelihood and ranked by Anderson–Darling A², with the Kolmogorov–Smirnov distance
//! beside it (`stochcraft_random::fit` has the methods and their sources).
//!
//! A query: it changes nothing. The chosen fit's `distribution` is a standard BPSim one in the
//! form `block.set` takes, so applying it is `block.set` with it as the element's
//! `interarrival`, `service` or `duration`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_random::fit::{self, Family};

use crate::wire::DistSpec;
use crate::{CommandError, ErrorCode};

/// A family measured data can be fitted to; the name is the `type` of the distribution it gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FitFamily {
    Exponential,
    Gamma,
    /// A gamma with a whole-number shape.
    Erlang,
    #[serde(alias = "lognormal")]
    LogNormal,
    Weibull,
    Normal,
    Uniform,
    Triangular,
}

impl From<FitFamily> for Family {
    fn from(f: FitFamily) -> Self {
        match f {
            FitFamily::Exponential => Family::Exponential,
            FitFamily::Gamma => Family::Gamma,
            FitFamily::Erlang => Family::Erlang,
            FitFamily::LogNormal => Family::LogNormal,
            FitFamily::Weibull => Family::Weibull,
            FitFamily::Normal => Family::Normal,
            FitFamily::Uniform => Family::Uniform,
            FitFamily::Triangular => Family::Triangular,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Params {
    /// The measured values, such as observed service times, in the model's time unit.
    pub data: Vec<f64>,
    /// The families to try; by default all of them.
    #[serde(default)]
    pub families: Option<Vec<FitFamily>>,
    /// The fewest values a fit is offered for (default 20; never fewer than 5).
    #[serde(default)]
    pub min_count: Option<u32>,
}

/// A number, or `null` where it is not finite (an infinite A²).
fn num(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

pub fn run(p: &Params) -> Result<Value, CommandError> {
    let families: Vec<Family> = match &p.families {
        Some(fs) if fs.is_empty() => {
            return Err(CommandError::new(
                ErrorCode::InvalidParams,
                "`families` is empty: name at least one, or leave it out for all",
            ));
        }
        Some(fs) => fs.iter().map(|&f| f.into()).collect(),
        None => Family::ALL.to_vec(),
    };
    let min_count = p
        .min_count
        .map_or(fit::MIN_COUNT, |m| usize::try_from(m).unwrap_or(usize::MAX));
    let fits = fit::fit(&p.data, &families, min_count)
        .map_err(|e| CommandError::new(ErrorCode::InvalidParams, e.to_string()))?;
    let skipped: Vec<Value> = fits
        .skipped
        .iter()
        .map(|s| json!({ "family": s.family.name(), "reason": s.reason.to_string() }))
        .collect();
    if fits.candidates.is_empty() {
        let reasons: Vec<String> = fits
            .skipped
            .iter()
            .map(|s| format!("{}: {}", s.family.name(), s.reason))
            .collect();
        let mut err = CommandError::new(
            ErrorCode::InvalidParams,
            format!(
                "no family can be fitted to these data ({})",
                reasons.join("; ")
            ),
        );
        err.details = Some(Value::Array(skipped));
        return Err(err);
    }
    let candidates: Vec<Value> = fits
        .candidates
        .iter()
        .map(|c| {
            json!({
                "family": c.family.name(),
                "distribution": DistSpec::from(&c.dist),
                "mean": c.dist.mean().map_or(Value::Null, num),
                "std_dev": c.dist.variance().map_or(Value::Null, |v| num(v.sqrt())),
                "log_likelihood": num(c.log_likelihood),
                "anderson_darling": num(c.anderson_darling),
                "kolmogorov_smirnov": num(c.kolmogorov_smirnov),
            })
        })
        .collect();
    Ok(json!({
        "count": fits.count as u64,
        "data": {
            "mean": num(fits.mean),
            "std_dev": num(fits.std_dev),
            "min": num(fits.min),
            "max": num(fits.max),
        },
        "ranked_by": "anderson_darling",
        "best": candidates.first().map_or(Value::Null, |c| c["distribution"].clone()),
        "candidates": candidates,
        "skipped": skipped,
    }))
}
