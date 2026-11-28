//! Comparing the results of two runs (`results.compare`): per element and metric, both means and
//! the difference b − a with a 95% confidence interval (A. M. Law, *Simulation Modeling and
//! Analysis*, 5th ed., section 10.2).
//!
//! How the two runs are compared follows where their random numbers came from (`class`):
//!
//! - **paired**: replication i of each met the same random numbers (common random numbers), so
//!   the difference is judged by the paired-t interval (section 10.2.1). Two runs with the same
//!   seed, or that read the same file from the same offset in segments of the same length, pair
//!   over their first min(n_a, n_b) replications: replication i's numbers depend only on the
//!   seed (or the file's place) and i, not on how many replications a run has.
//! - **independent**: Welch's two-sample t interval (section 10.2.2). Two seeds, a seed against a
//!   file, two files, or parts of one file that do not overlap. Different seeds' streams, and
//!   different files, are taken to be independent; that is assumed, not checked.
//! - **dependent**: the two runs read overlapping parts of one file, but not replication for
//!   replication (another offset or segment length), so their replications are neither
//!   independent nor paired, and neither interval holds. The means are given with no interval,
//!   and `reason` says why.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_blocks::Metric;
use stochcraft_stats::{Difference, difference};

use crate::decide::Output;
use crate::random::Identity;
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
    /// Where its random numbers came from ([`crate::random::identity`]).
    random: Identity,
    /// `a` or `b`, for messages.
    which: &'static str,
    values: &'a serde_json::Map<String, Value>,
    /// The elements in the order the results list them.
    order: Vec<&'a str>,
}

impl<'a> RunResults<'a> {
    fn read(v: &'a Value, which: &'static str) -> Result<Self, CommandError> {
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
            random: crate::random::identity(v, which)?,
            replications: field("replications")?,
            which,
            values,
            order,
        })
    }

    /// Whether the run has values for the metric at all.
    fn has(&self, element: &str, metric: &str) -> bool {
        self.values
            .get(element)
            .and_then(|e| e.get(metric))
            .is_some_and(Value::is_array)
    }

    /// The metric's value in each replication (`None` where it had none); empty if the run has
    /// no values for it at all. A list that is not one value per replication is refused: the
    /// pairs would not be replication for replication.
    fn samples(&self, element: &str, metric: &str) -> Result<Vec<Option<f64>>, CommandError> {
        let Some(list) = self.values.get(element).and_then(|e| e.get(metric)) else {
            return Ok(Vec::new());
        };
        let list = list.as_array().ok_or_else(|| {
            invalid(format!(
                "`{}`'s `values.{element}.{metric}` is not a list",
                self.which
            ))
        })?;
        if u64::try_from(list.len()).ok() != Some(self.replications) {
            return Err(invalid(format!(
                "`{}` has {} values of `{element}`'s `{metric}` for {} replications: give one per replication",
                self.which,
                list.len(),
                self.replications
            )));
        }
        list.iter()
            .map(|x| match x {
                Value::Null => Ok(None),
                x => x.as_f64().map(Some).ok_or_else(|| {
                    invalid(format!(
                        "`{}`'s `values.{element}.{metric}` holds {x}, not a number or null",
                        self.which
                    ))
                }),
            })
            .collect()
    }
}

/// How two runs' replications relate, from where their random numbers came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Basis {
    /// The same seed: paired.
    SameSeed,
    /// The same file, offset and segment length: paired.
    SameFile,
    /// Two seeds: independent.
    DifferentSeeds,
    /// A seed against a file: taken as independent.
    SeedAndFile,
    /// Two files: taken as independent.
    DifferentFiles,
    /// Parts of one file that do not overlap: independent.
    DisjointParts,
    /// Overlapping parts of one file, not replication for replication: dependent.
    OverlappingParts,
}

impl Basis {
    fn of(a: &RunResults, b: &RunResults) -> Self {
        match (&a.random, &b.random) {
            (Identity::Seed(x), Identity::Seed(y)) => {
                if x == y {
                    Basis::SameSeed
                } else {
                    Basis::DifferentSeeds
                }
            }
            (Identity::Seed(_), Identity::File { .. })
            | (Identity::File { .. }, Identity::Seed(_)) => Basis::SeedAndFile,
            (
                Identity::File {
                    sha256: fa,
                    offset: oa,
                    segment: la,
                },
                Identity::File {
                    sha256: fb,
                    offset: ob,
                    segment: lb,
                },
            ) => {
                if fa != fb {
                    Basis::DifferentFiles
                } else if oa == ob && la == lb {
                    Basis::SameFile
                } else {
                    match (a.random.span(a.replications), b.random.span(b.replications)) {
                        (Some((from_a, to_a)), Some((from_b, to_b)))
                            if to_a <= from_b || to_b <= from_a =>
                        {
                            Basis::DisjointParts
                        }
                        _ => Basis::OverlappingParts,
                    }
                }
            }
        }
    }

    fn name(self) -> &'static str {
        match self {
            Basis::SameSeed => "same_seed",
            Basis::SameFile => "same_file",
            Basis::DifferentSeeds => "different_seeds",
            Basis::SeedAndFile => "seed_and_file",
            Basis::DifferentFiles => "different_files",
            Basis::DisjointParts => "disjoint_parts",
            Basis::OverlappingParts => "overlapping_parts",
        }
    }

    fn class(self) -> Class {
        match self {
            Basis::SameSeed | Basis::SameFile => Class::Paired,
            Basis::OverlappingParts => Class::Dependent,
            _ => Class::Independent,
        }
    }

    /// Why the runs are compared as they are, before how.
    fn why(self) -> &'static str {
        match self {
            Basis::SameSeed => {
                "the runs have the same seed, and replication i's random numbers depend only on the seed and i, so replication i of each meets the same random numbers (common random numbers)"
            }
            Basis::SameFile => {
                "the runs read the same random-data file (the same SHA-256) from the same offset in segments of the same length, so replication i of each starts from the same random numbers (common random numbers); every draw takes the next numbers of its segment, so a change to the model can shift which event reads which number, and less of the chance difference may cancel than with a seed"
            }
            Basis::DifferentSeeds => {
                "the runs have different seeds, whose streams are taken to be independent"
            }
            Basis::SeedAndFile => {
                "one run drew from a seed and the other read a random-data file, which are taken to be independent (assumed, not checked)"
            }
            Basis::DifferentFiles => {
                "the runs read different random-data files (different SHA-256), which are taken to be independent (assumed, not checked)"
            }
            Basis::DisjointParts => {
                "the runs read parts of the same random-data file that do not overlap, so no random number is shared"
            }
            Basis::OverlappingParts => {
                "the runs read overlapping parts of the same random-data file, but not replication for replication (another offset or segment length), so some replications of one share random numbers with replications of the other: they are neither independent nor paired, and neither Welch's interval nor the paired-t interval holds. Only the means are given; read both from the same offset with the same segment length to pair them, or from parts that do not overlap to make them independent"
            }
        }
    }
}

/// How two runs are compared: `results.compare`'s `class`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Paired,
    Independent,
    Dependent,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Paired => "paired",
            Class::Independent => "independent",
            Class::Dependent => "dependent",
        }
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

/// The difference b − a of one result with its 95% interval, from each run's value in each
/// replication (`None` where it had none): the paired-t interval over the replications where
/// both have a value when the runs are `paired` (common random numbers), Welch's two-sample t
/// interval otherwise. `results.compare` and `data.analyse` both judge a difference by it.
pub(crate) fn difference_of(
    a: &[Option<f64>],
    b: &[Option<f64>],
    paired: bool,
) -> Option<Difference> {
    if paired {
        difference::paired(a, b)
    } else {
        let flat = |xs: &[Option<f64>]| xs.iter().flatten().copied().collect::<Vec<_>>();
        difference::welch(&flat(a), &flat(b))
    }
}

/// The name of the interval [`difference_of`] uses, as the answers give it.
pub(crate) fn interval_name(paired: bool) -> &'static str {
    if paired { "paired_t" } else { "welch_t" }
}

/// Compares `a` with `b` (results as `run` returns them).
pub fn compare(a: &Value, b: &Value, only: Option<&[Output]>) -> Result<Value, CommandError> {
    let (ra, rb) = (RunResults::read(a, "a")?, RunResults::read(b, "b")?);
    let basis = Basis::of(&ra, &rb);
    let class = basis.class();
    let paired = class == Class::Paired;
    // Paired runs pair their first min(n_a, n_b) replications: replication i of each met the
    // same random numbers however many replications either has.
    let over = ra.replications.min(rb.replications);
    let first = usize::try_from(over).unwrap_or(usize::MAX);
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
            .filter(|(id, m)| ra.has(id, m) && rb.has(id, m))
            .collect(),
    };
    let comparisons = wanted
        .iter()
        .map(|(element, metric)| {
            let (xa, xb) = (ra.samples(element, metric)?, rb.samples(element, metric)?);
            let ((mean_a, n_a), (mean_b, n_b)) = (mean(&xa), mean(&xb));
            let mut c = json!({
                "element": element,
                "metric": metric,
                "a": { "mean": mean_a, "n": n_a },
                "b": { "mean": mean_b, "n": n_b },
            });
            match class {
                Class::Dependent => {
                    // The means, and b's minus a's, with no interval: neither holds.
                    let d = mean_a.zip(mean_b).map(|(x, y)| y - x);
                    c["difference"] = json!(d);
                    c["half_width"] = Value::Null;
                    c["df"] = Value::Null;
                    c["significant"] = Value::Null;
                    c["interval"] = json!("none");
                }
                Class::Paired | Class::Independent => {
                    let cut =
                        |xs: &[Option<f64>]| xs.iter().take(first).copied().collect::<Vec<_>>();
                    let d = if paired {
                        difference_of(&cut(&xa), &cut(&xb), true)
                    } else {
                        difference_of(&xa, &xb, false)
                    };
                    c["difference"] = json!(d.map(|d| d.mean));
                    c["half_width"] = json!(d.map(|d| d.half_width));
                    c["df"] = json!(d.and_then(|d| d.df));
                    c["significant"] = json!(d.and_then(|d| d.significant()));
                    c["interval"] = json!(interval_name(paired));
                    if paired {
                        c["pairs"] = json!(d.map_or(0, |d| d.n_a));
                    }
                }
            }
            Ok(c)
        })
        .collect::<Result<Vec<Value>, CommandError>>()?;
    let mut notes = Vec::new();
    for k in ["time_unit", "warmup", "duration"] {
        if let (Some(x), Some(y)) = (a.get(k), b.get(k))
            && x != y
        {
            notes.push(format!("the runs differ in `{k}` ({x} and {y})"));
        }
    }
    let how = match class {
        Class::Paired => {
            "so they are compared in pairs over their first paired_replications replications (the fewer of the two runs'; a's and b's means are over all of each run's): difference is the mean over those replications where both have a value (pairs) of b minus a, with half_width its 95% paired-t half-width (pairs − 1 degrees of freedom); significant: that interval excludes 0 (null with fewer than two pairs)"
        }
        Class::Independent => {
            "so they are compared as independent runs: difference is b's mean minus a's, with half_width its 95% Welch two-sample t half-width (Welch–Satterthwaite degrees of freedom, rounded down); significant: that interval excludes 0 (null when either run has fewer than two values). Running both with the same seed gives a paired comparison, usually much tighter"
        }
        Class::Dependent => {
            "difference is b's mean minus a's, with no half_width, df or significant"
        }
    };
    let mut out = json!({
        "a": { "seed": ra.seed, "replications": ra.replications, "random": ra.random.json() },
        "b": { "seed": rb.seed, "replications": rb.replications, "random": rb.random.json() },
        "paired": paired,
        "class": class.name(),
        "basis": basis.name(),
        "comparisons": comparisons,
        "notes": notes,
        "method": format!("{}, {how}", basis.why()),
    });
    match class {
        Class::Paired => out["paired_replications"] = json!(over),
        Class::Dependent => out["reason"] = json!(basis.why()),
        Class::Independent => {}
    }
    Ok(out)
}
