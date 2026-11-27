//! Fitting a distribution to measured data (`estimate.fit`): each candidate family fitted by
//! maximum likelihood and ranked by Anderson–Darling A², with the Kolmogorov–Smirnov distance
//! beside it (`stochcraft_random::fit` has the methods and their sources).
//!
//! Each candidate also has a verdict at 5% from each test (`tests`): whether its modified
//! statistic is above the 95% point for its case ([`stochcraft_stats::gof`]): computed for all
//! parameters known; Stephens's points for a normal or log-normal (judged with the standard
//! deviation S, divisor n − 1, as his case assumes, though the fit itself is the
//! maximum-likelihood one, divisor n), an exponential and a Weibull; and the all-known points,
//! a conservative test, for the gamma, Erlang, uniform and triangular. With many data the tests
//! reject almost any fit, which the answer says.
//!
//! A query: it changes nothing. The chosen fit's `distribution` is a standard BPSim one in the
//! form `block.set` takes, so applying it is `block.set` with it as the element's
//! `interarrival`, `service` or `duration`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_random::Dist;
use stochcraft_random::fit::{self, Family};
use stochcraft_random::math::{exp, ln};
use stochcraft_stats::gof::{self, Case};

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

/// What the verdicts say of fits to many data: since no real data come exactly from a
/// distribution, a large enough sample rejects any fit.
pub const LARGE_SAMPLE: &str = "with many data these tests reject almost any fitted distribution, since no real data come exactly from one: a rejection then says the fit is not exact, not that it is not useful (Law, Simulation Modeling and Analysis, 5th ed., section 6.6.2)";

/// The case of `family`'s tests, and the distribution its statistics are taken against: the fit
/// itself, except a normal or log-normal, whose case takes the standard deviation with divisor
/// n − 1 (of the logarithms, for a log-normal).
fn case_of(family: Family, fitted: &Dist, data: &[f64]) -> (Case, Dist) {
    let n = data.len() as f64;
    let mean_sd = |xs: &[f64]| {
        let m = xs.iter().sum::<f64>() / n;
        let s2 = xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (n - 1.0);
        (m, s2)
    };
    match family {
        Family::Exponential => (Case::Exponential, fitted.clone()),
        Family::Weibull => (Case::Weibull, fitted.clone()),
        Family::Normal if n > 1.0 => {
            let (m, s2) = mean_sd(data);
            (
                Case::Normal,
                Dist::Normal {
                    mean: m,
                    std_dev: s2.sqrt(),
                },
            )
        }
        Family::LogNormal if n > 1.0 => {
            let logs: Vec<f64> = data.iter().map(|&x| ln(x)).collect();
            let (mu, s2) = mean_sd(&logs);
            let mean = exp(mu + s2 / 2.0);
            (
                Case::Normal,
                Dist::LogNormal {
                    mean,
                    std_dev: mean * (exp(s2) - 1.0).sqrt(),
                },
            )
        }
        _ => (Case::AllKnown, fitted.clone()),
    }
}

/// One test's verdict as the answer gives it.
fn verdict_json(statistic: f64, v: gof::Verdict, case: Case) -> Value {
    let verdict = if v.rejected {
        "rejected"
    } else if case == Case::AllKnown {
        "not_rejected_conservative"
    } else {
        "not_rejected"
    };
    json!({
        "statistic": num(statistic),
        "modified": num(v.modified),
        "critical": num(v.critical),
        "verdict": verdict,
    })
}

/// The 5% verdicts of both tests on `family`'s fit `fitted` to `data`.
fn tests(family: Family, fitted: &Dist, data: &[f64]) -> Value {
    let (case, against) = case_of(family, fitted, data);
    let n = data.len() as u64;
    let Some(g) = fit::goodness_of_fit(data, &against) else {
        return Value::Null;
    };
    json!({
        "case": case.name(),
        "level": gof::LEVEL,
        "anderson_darling": verdict_json(
            g.anderson_darling,
            gof::ad_verdict(case, n, g.anderson_darling),
            case,
        ),
        "kolmogorov_smirnov": verdict_json(
            g.kolmogorov_smirnov,
            gof::ks_verdict(case, n, g.kolmogorov_smirnov),
            case,
        ),
    })
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
    let finite: Vec<f64> = p.data.iter().copied().filter(|x| x.is_finite()).collect();
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
                "tests": tests(c.family, &c.dist, &finite),
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
        "tests_note": LARGE_SAMPLE,
    }))
}
