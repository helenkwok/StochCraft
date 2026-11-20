//! Which results are a run's key measures and how precisely its replications pin them down, on
//! the results as `run` gives them: the one rule the window's "How sure are these numbers?" and
//! sequential runs both use, so the two cannot disagree. The numbers behind it (the targets,
//! the score, the stopping rule) are [`stochcraft_stats::precision`]'s.

use serde_json::{Value, json};
use stochcraft_model::{Sequential, SequentialMeasure};
use stochcraft_stats::precision::{self as rule, Reason, Scale, Verdict};

pub use stochcraft_stats::precision::{SETTLED, SETTLED_POINTS, runs_needed, unseen_rate};

/// How a metric reads: `Time` takes the model's time unit, `Share` is a fraction shown as a
/// percentage (and judged in percentage points).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Count,
    Time,
    Share,
    Items,
    /// Kilowatt-hours.
    Energy,
}

/// The unit of metric `name` (its name in the results, such as `mean_wait`), whatever the kind of
/// element (a conveyor's `blocking_probability` is a share of time, another's a share of units).
pub fn unit(name: &str) -> Unit {
    match name {
        "mean_queue_length"
        | "mean_contents"
        | "work_in_progress"
        | "mean_work_in_progress"
        | "mean_in_service" => Unit::Items,
        "mean_wait"
        | "mean_time_in_system"
        | "mean_blocked_time"
        | "mean_time_on"
        | "abandoned_wait"
        | "wasted"
        | "takt"
        | "cycle_time" => Unit::Time,
        "utilisation"
        | "blocking_probability"
        | "loss_probability"
        | "abandoned_share"
        | "balked_share"
        | "wasted_share"
        | "availability"
        | "time_off_shift"
        | "time_processing"
        | "time_starved"
        | "time_blocked"
        | "time_failed"
        | "time_restarting"
        | "time_setup"
        | "oee_availability"
        | "oee_effectiveness"
        | "setup_ratio"
        | "quality_ratio"
        | "scrap_ratio"
        | "rework_ratio"
        | "oee"
        | "demand_met" => Unit::Share,
        "energy" | "specific_energy" => Unit::Energy,
        _ => Unit::Count,
    }
}

/// The target a key measure of unit `unit` is judged against by default: points for a share,
/// a share of the mean otherwise.
fn scale(unit: Unit, relative: f64, points: f64) -> Scale {
    match unit {
        Unit::Share => Scale::Points(points),
        _ => Scale::Relative(relative),
    }
}

/// How sure the runs are of result `m` (`{mean, half_width, …}` in the results) of unit `unit`:
/// its half-width over what counts as settled ([`SETTLED`], [`SETTLED_POINTS`]), so 1 or less is
/// settled. `None` without an interval, or for a mean of zero that is not a share.
pub fn score(m: &Value, unit: Unit) -> Option<f64> {
    let (mean, hw) = (m["mean"].as_f64()?, m["half_width"].as_f64()?);
    rule::score(mean, hw, scale(unit, SETTLED, SETTLED_POINTS))
}

/// Whether a result of element `e` is left out of the tables: OEE and its availability where no
/// planned run time is set (without a plan to compare against, they only restate availability).
pub fn hidden(e: &Value, key: &str) -> bool {
    key.starts_with("oee") && e["metrics"]["oee_effectiveness"].is_null()
}

/// The results a block's key measures are chosen from, in order, by its kind: the first four
/// with a value are shown. Those marked `false` are shown only when not zero (a block that
/// never failed has no "Time failed" row among its key measures).
fn key_candidates(kind: &str) -> &'static [(&'static str, bool)] {
    match kind {
        "server" | "shared_task" => &[
            ("utilisation", true),
            ("mean_wait", true),
            ("mean_queue_length", true),
            ("time_failed", false),
            ("abandoned_share", false),
            ("balked_share", false),
            ("loss_probability", false),
            ("left", true),
        ],
        "delay" | "batch" | "unbatch" => {
            &[("mean_contents", true), ("left", true), ("lost", false)]
        }
        "conveyor" => &[
            ("mean_time_on", true),
            ("mean_contents", true),
            ("blocking_probability", true),
            ("left", true),
        ],
        "sink" => &[
            ("entered", true),
            ("mean_time_in_system", true),
            ("cycle_time", true),
            ("demand_met", true),
        ],
        "source" => &[
            ("left", true),
            ("balked", false),
            ("lost", false),
            ("loss_probability", false),
        ],
        "resource" => &[
            ("utilisation", true),
            ("mean_in_service", true),
            ("time_failed", false),
        ],
        "exclusive_gateway" => &[("left", true), ("lost", false)],
        _ => &[("left", true), ("entered", true)],
    }
}

/// The key measures of block `e` (an element of the results): up to four of its results by
/// [`key_candidates`].
pub fn key_of(e: &Value) -> Vec<&'static str> {
    let kind = e["kind"].as_str().unwrap_or("");
    key_candidates(kind)
        .iter()
        .filter(|(k, always)| {
            let m = &e["metrics"][*k];
            !m.is_null()
                && !hidden(e, k)
                && (*always || m["mean"].as_f64().is_some_and(|v| v != 0.0))
        })
        .map(|(k, _)| *k)
        .take(4)
        .collect()
}

/// One result a sequential run watches, with its verdict after the replications so far.
#[derive(Debug, Clone, PartialEq)]
pub struct Watched {
    pub element: String,
    pub metric: String,
    pub verdict: Verdict,
}

impl Watched {
    fn to_json(&self) -> Value {
        json!({
            "element": self.element,
            "metric": self.metric,
            "status": self.verdict.name(),
            "score": self.verdict.score(),
        })
    }
}

/// The verdict on result `m` of the results (`{mean, half_width, std_dev, n}`, or null) against
/// `scale`.
fn verdict(m: &Value, scale: Scale) -> Verdict {
    rule::verdict(
        m["n"].as_u64().unwrap_or(0),
        m["mean"].as_f64(),
        m["half_width"].as_f64(),
        m["std_dev"].as_f64(),
        scale,
    )
}

/// The results `x` watches in `results` (as `run` gives them), each with its verdict: one named
/// result, or every block's key measures ([`key_of`]). A named result missing from the results
/// (its element removed, or a metric the element does not have) has [`Verdict::NoValue`].
pub fn watched(results: &Value, x: &Sequential) -> Vec<Watched> {
    let elements = results["elements"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    match &x.measure {
        Some(SequentialMeasure {
            element,
            metric,
            half_width,
        }) => {
            let e = elements
                .iter()
                .find(|e| e["id"].as_str() == Some(element.as_str()));
            let target = match half_width {
                Some(h) => Scale::Absolute(*h),
                None => scale(unit(metric), x.relative, x.points),
            };
            let found = e.map_or(Verdict::NoValue, |e| verdict(&e["metrics"][metric], target));
            vec![Watched {
                element: element.to_string(),
                metric: metric.clone(),
                verdict: found,
            }]
        }
        None => elements
            .iter()
            .flat_map(|e| {
                let id = e["id"].as_str().unwrap_or("");
                key_of(e).into_iter().map(move |key| Watched {
                    element: id.to_owned(),
                    metric: key.to_owned(),
                    verdict: verdict(&e["metrics"][key], scale(unit(key), x.relative, x.points)),
                })
            })
            .collect(),
    }
}

/// The run record's words for the uncertainty of a sequential run's results.
pub const SEQUENTIAL_UNCERTAINTY: &str = "estimated precision after adaptive stopping: the half-width of a 95% Student-t confidence interval of the mean across independent replications, whose number was chosen from the results themselves (sequential replications), so the interval's coverage is not guaranteed to be 95%; std_dev is the sample standard deviation across replications";

/// What the results say of a sequential run that stopped after `stages` (the replication count
/// at each look, the last the count run) for `reason`, with the results it watched at the last
/// look: the settings, the counts, why it stopped, each watched result's status, and the ones
/// whose replications all gave the same value (`no_spread`), with how often per run an event
/// none of them met may still happen (`unseen_rate`).
pub fn summary(x: &Sequential, stages: &[u64], reason: Reason, watched: &[Watched]) -> Value {
    let named = |status: &str| {
        watched
            .iter()
            .filter(|w| w.verdict.name() == status)
            .map(|w| json!({ "element": w.element, "metric": w.metric }))
            .collect::<Vec<_>>()
    };
    json!({
        "settings": crate::wire::SequentialSpec::from(x),
        "replications": stages.last().copied().unwrap_or(0),
        "stages": stages,
        "reason": reason.name(),
        "met": reason == Reason::TargetMet,
        "measures": watched.iter().map(Watched::to_json).collect::<Vec<_>>(),
        "no_spread": named("no_spread"),
        "mean_near_zero": named("mean_near_zero"),
        // The most often per run an event none of the replications met may still happen (95%).
        "unseen_rate": unseen_rate(stages.last().copied().unwrap_or(0)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_are_judged_in_points_and_the_rest_against_their_mean() {
        let m = |mean: f64, hw: f64| json!({ "mean": mean, "half_width": hw });
        assert!(score(&m(0.017, 0.006), Unit::Share).is_some_and(|s| s <= 1.0));
        assert!(score(&m(0.92, 0.03), Unit::Share).is_some_and(|s| s > 2.0));
        assert!(score(&m(38.0, 12.16), Unit::Time).is_some_and(|s| (s - 6.4).abs() < 1e-9));
        assert!(score(&m(0.0, 0.0), Unit::Time).is_none());
        // Infinite half-widths are written as null.
        assert!(score(&json!({ "mean": 3.0, "half_width": null }), Unit::Time).is_none());
        assert_eq!(unit("utilisation"), Unit::Share);
        assert_eq!(unit("blocking_probability"), Unit::Share);
        assert_eq!(unit("mean_wait"), Unit::Time);
        assert_eq!(unit("left"), Unit::Count);
    }

    fn results() -> Value {
        let m = |mean: f64, hw: f64, sd: f64| json!({ "mean": mean, "half_width": hw, "std_dev": sd, "n": 10 });
        json!({ "elements": [
            { "id": "desk", "kind": "server", "metrics": {
                "utilisation": m(0.8, 0.004, 0.006),
                "mean_wait": m(3.0, 0.6, 0.9),
                "mean_queue_length": m(2.4, 0.05, 0.07),
                "time_failed": m(0.0, 0.0, 0.0),
                "left": m(100.0, 1.0, 1.4),
            }},
            { "id": "exit", "kind": "sink", "metrics": {
                "entered": m(100.0, 0.0, 0.0),
                "mean_time_in_system": m(0.0, 0.3, 0.4),
            }},
        ]})
    }

    #[test]
    fn every_key_measure_is_watched_with_its_verdict() {
        let w = watched(&results(), &Sequential::default());
        let status: Vec<(&str, &str, &str)> = w
            .iter()
            .map(|w| (w.element.as_str(), w.metric.as_str(), w.verdict.name()))
            .collect();
        assert_eq!(
            status,
            vec![
                // ± 0.4 points: settled.
                ("desk", "utilisation", "settled"),
                // ± 20%: wide.
                ("desk", "mean_wait", "wide"),
                ("desk", "mean_queue_length", "settled"),
                // A time failed of zero is not a key measure; `left` is the fourth.
                ("desk", "left", "settled"),
                // The same count in every run: no spread, so not settled.
                ("exit", "entered", "no_spread"),
                // A mean of zero cannot meet a share of itself.
                ("exit", "mean_time_in_system", "mean_near_zero"),
            ]
        );
    }

    #[test]
    fn one_named_result_is_watched_against_its_own_target() {
        let one = |half_width: Option<f64>| Sequential {
            measure: Some(SequentialMeasure {
                element: stochcraft_model::ElementId::unchecked("exit"),
                metric: "mean_time_in_system".into(),
                half_width,
            }),
            ..Sequential::default()
        };
        // Against a share of a mean of zero: not met, and the results say why.
        let w = watched(&results(), &one(None));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].verdict.name(), "mean_near_zero");
        // Against an absolute half-width it is met.
        let w = watched(&results(), &one(Some(0.5)));
        assert!(w[0].verdict.is_settled(), "{w:?}");
        // A result the run does not have has no value.
        let mut gone = one(None);
        if let Some(m) = &mut gone.measure {
            m.element = stochcraft_model::ElementId::unchecked("removed");
        }
        assert_eq!(watched(&results(), &gone)[0].verdict, Verdict::NoValue);
    }
}
