//! Finding the best whole-number decisions (M8, stage O2): `experiment.optimise`.
//!
//! The decisions are counts (a task's units, a pool's units, a buffer), each a whole number in a
//! range with a step. The objective is a weighted sum of results (each an element's or a pool's
//! metric) plus a cost per unit of each decision, to minimise or maximise; the decisions' part is
//! known exactly, so only the results are noisy. A cap on the decisions' costs is checked before
//! a design is run, so designs over it are never simulated. Constraints keep other results'
//! means at most or at least a limit, each with a tolerance ε.
//!
//! The search is COMPASS with the pruning, the local-optimality test and the clean-up of
//! Industrial Strength COMPASS, from the starting design and a few Latin hypercube points
//! (`stochcraft_experiment::compass`); a space of at most `all_designs_up_to` designs (50 by
//! default) is not searched, and every design goes to the clean-up. The clean-up compares the
//! finalists again on a fresh block of replications, seeded by
//! `Rng::keyed(seed, "optimize/clean-up")`, by Kim and Nelson's KN, after Batur and Kim's
//! feasibility check when there are constraints (α split between the two). Every design runs
//! replication i on the same seed (common random numbers), and every element's streams are keyed
//! by its id and purpose, so a change to one decision leaves the other elements' random numbers
//! as they were.
//!
//! The answer's claim covers the designs compared at the end, not the whole space, and says so;
//! when every design of the space was compared, it says that instead. A model with no randomness
//! is compared exactly, one replication per design, with no probability stated.

use std::collections::BTreeMap;
use std::ops::Range;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_blocks::{Compiled, compile, summarise};
use stochcraft_experiment::compass::{self, Answer, Compass, Ended, Linear, Space, Stream};
use stochcraft_experiment::select::{Bound, Constraint, Decision, Outcome, Sense};
use stochcraft_experiment::{Control, Experiment};
use stochcraft_model::{ElementId, Model};
use stochcraft_random::Rng;
use stochcraft_stats::{Summary, difference};

use crate::decide::{Field, Output, Param, deterministic, invalid, metric, samples};
use crate::{CommandError, ErrorCode};

/// The budget when none is given, in runs (replications summed over designs).
pub const DEFAULT_MAX_RUNS: u64 = 10_000;
/// The largest budget a search may be given.
pub const MOST_RUNS: u64 = 1_000_000;
/// Latin hypercube starts beside the starting design, when none is given.
pub const DEFAULT_STARTS: u32 = 3;
const MOST_STARTS: u32 = 20;

/// A decision: a count of the model, a whole number from `min` to `max` in steps of `step`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionRange {
    /// The element (a task with units of its own, or a buffer) or pool.
    pub id: String,
    /// `capacity`, `quantity` or `queue_capacity` (a mean is a continuous decision, not taken
    /// here).
    pub field: Field,
    pub min: u32,
    pub max: u32,
    /// The step between values (default 1).
    #[serde(default)]
    pub step: Option<u32>,
    /// The value the search starts from (default: the model's).
    #[serde(default)]
    pub start: Option<u32>,
}

/// A result in the objective, times its weight.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Term {
    pub element: String,
    pub metric: String,
    /// The weight (default 1): a cost per unit of the result, such as 50 per customer waiting.
    #[serde(default)]
    pub weight: Option<f64>,
}

/// A cost per unit of a decision's value, such as 20 per operator.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Cost {
    pub id: String,
    pub field: Field,
    pub cost: f64,
}

/// Whether the objective is to be made small or large.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Goal {
    #[default]
    #[serde(alias = "minimize")]
    Minimise,
    #[serde(alias = "maximize")]
    Maximise,
}

/// The objective: Σ weight × result + Σ cost × decision.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Objective {
    /// The results, each times its weight; none for an objective of decision costs alone, when
    /// constraints carry the statistics.
    #[serde(default)]
    pub results: Vec<Term>,
    #[serde(default)]
    pub sense: Goal,
    /// The decisions' costs per unit.
    #[serde(default)]
    pub costs: Vec<Cost>,
}

/// A constraint on a result's mean: at most or at least a limit (give one), with the tolerance
/// ε in the result's unit: a design within ε of the limit may count either way.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    pub element: String,
    pub metric: String,
    #[serde(default)]
    pub at_most: Option<f64>,
    #[serde(default)]
    pub at_least: Option<f64>,
    pub tolerance: f64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OptimiseParams {
    /// The decisions, each at most once.
    pub decisions: Vec<DecisionRange>,
    pub objective: Objective,
    /// The most the decisions' costs may add up to; designs over it are never run.
    #[serde(default)]
    pub cost_cap: Option<f64>,
    #[serde(default)]
    pub constraints: Vec<Limit>,
    /// δ, the smallest difference in the objective worth telling apart, in its unit (required).
    pub indifference: f64,
    /// 1 − α (default 0.95), covering the feasibility check and the selection together.
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Replications a design gets at its first visit (default 10).
    #[serde(default)]
    pub replications_per_visit: Option<u32>,
    /// The budget in runs, replications summed over designs (default 10,000, at most
    /// 1,000,000).
    #[serde(default)]
    pub max_runs: Option<u64>,
    /// The share of the budget kept for the clean-up (default 0.3).
    #[serde(default)]
    pub reserve: Option<f64>,
    /// The runs kept for the clean-up, in place of `reserve` (to repeat a search a time limit
    /// stopped, as its answer's `repeat` gives).
    #[serde(default)]
    pub reserve_runs: Option<u64>,
    /// Latin hypercube starts beside the starting design (default 3, at most 20).
    #[serde(default)]
    pub starts: Option<u32>,
    /// With at most this many designs (default 50), every design goes to the clean-up.
    #[serde(default)]
    pub all_designs_up_to: Option<u32>,
    /// Stop the search after this many seconds (the clean-up still runs). Not available in the
    /// browser.
    #[serde(default)]
    pub time_limit: Option<f64>,
    /// The seed (default: the experiment's).
    #[serde(default)]
    pub seed: Option<u64>,
    /// Make the chosen design the model's, as one undo step (only when the answer is proven or
    /// compared exactly).
    #[serde(default)]
    pub apply: bool,
    /// Run although the file had errors StochCraft could not read (as for `run`).
    #[serde(default)]
    pub allow_approximations: bool,
}

/// A decision checked against the model.
struct Checked {
    param: Param,
    label: String,
    min: u32,
    step: u32,
    /// The last index: values are min + step·k for k from 0 to `last`.
    last: u32,
    cost: f64,
    /// A cost entry was given for this decision (a cost of 0 included).
    costed: bool,
    start: u32,
}

impl Checked {
    fn value(&self, k: i64) -> f64 {
        f64::from(self.min) + f64::from(self.step) * k as f64
    }
}

fn label(id: &str, field: Field) -> String {
    format!("{id}.{}", field.name())
}

/// Checks that `element` names an element or pool and `metric` a metric.
fn check_result(model: &Model, what: &str, element: &str, name: &str) -> Result<(), CommandError> {
    metric(name)?;
    let id = ElementId::unchecked(element);
    if model.element(&id).is_none() && !model.resources.iter().any(|r| r.id == id) {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            format!("{what} `{element}.{name}`: no element or pool with id `{element}`"),
        ));
    }
    Ok(())
}

fn decisions(model: &Model, p: &OptimiseParams) -> Result<Vec<Checked>, CommandError> {
    if p.decisions.is_empty() {
        return Err(invalid("give at least one decision"));
    }
    let mut out: Vec<Checked> = Vec::new();
    for d in &p.decisions {
        let name = label(&d.id, d.field);
        if out.iter().any(|c| c.label == name) {
            return Err(invalid(format!(
                "decision `{name}` is given more than once"
            )));
        }
        if d.field == Field::Mean {
            return Err(invalid(format!(
                "decision `{name}`: a mean is a number in a range, not a count; this search takes `capacity`, `quantity` and `queue_capacity`"
            )));
        }
        let floor = if d.field == Field::QueueCapacity {
            0
        } else {
            1
        };
        if d.min < floor || d.min > d.max {
            return Err(invalid(format!(
                "decision `{name}`: its range {}..{} must run upwards from at least {floor}",
                d.min, d.max
            )));
        }
        let step = d.step.unwrap_or(1);
        if step == 0 {
            return Err(invalid(format!(
                "decision `{name}`: its step must be at least 1"
            )));
        }
        let id = ElementId::unchecked(&d.id);
        let exists = match d.field {
            Field::Quantity => model.resources.iter().any(|r| r.id == id),
            _ => model.element(&id).is_some(),
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
        let param = Param {
            id: d.id.clone(),
            field: d.field,
        };
        let named = |e: CommandError| CommandError {
            message: format!("decision `{name}`: {}", e.message),
            ..e
        };
        let current = param.get(model).map_err(named)?;
        let start = match d.start {
            Some(s) => s,
            None => {
                if current.fract() != 0.0 || current < 0.0 || current > f64::from(u32::MAX) {
                    return Err(invalid(format!(
                        "decision `{name}`: the model's value {current} is not a whole number; give `start`"
                    )));
                }
                current as u32
            }
        };
        if start < d.min || start > d.max || (start - d.min) % step != 0 {
            return Err(invalid(format!(
                "decision `{name}`: the starting value {start} is not one of its values ({}..{} in steps of {step}); give `start`",
                d.min, d.max
            )));
        }
        out.push(Checked {
            param,
            label: name,
            min: d.min,
            step,
            last: (d.max - d.min) / step,
            cost: 0.0,
            costed: false,
            start,
        });
    }
    for c in &p.objective.costs {
        let name = label(&c.id, c.field);
        if !c.cost.is_finite() {
            return Err(invalid(format!("the cost of `{name}` must be a number")));
        }
        let Some(d) = out.iter_mut().find(|d| d.label == name) else {
            return Err(invalid(format!(
                "the cost of `{name}`: costs are for decisions, and `{name}` is not one"
            )));
        };
        if d.costed {
            return Err(invalid(format!(
                "the cost of `{name}` is given more than once"
            )));
        }
        d.cost = c.cost;
        d.costed = true;
    }
    Ok(out)
}

/// The model with each decision at its value for design `x` (indices into each range).
fn decided(model: &Model, ds: &[Checked], x: &[i64]) -> Result<Model, CommandError> {
    let mut m = model.clone();
    for (d, &k) in ds.iter().zip(x) {
        d.param.set(&mut m, d.value(k))?;
    }
    Ok(m)
}

/// The design as `{id, field, value}` for each decision.
fn design_json(ds: &[Checked], x: &[i64]) -> Value {
    Value::Array(
        ds.iter()
            .zip(x)
            .map(|(d, &k)| json!({ "id": d.param.id, "field": d.param.field, "value": d.value(k) }))
            .collect(),
    )
}

/// The design as text, `(14, 8)`.
fn design_text(ds: &[Checked], x: &[i64]) -> String {
    let values: Vec<String> = ds
        .iter()
        .zip(x)
        .map(|(d, &k)| format!("{}", d.value(k)))
        .collect();
    format!("({})", values.join(", "))
}

/// The decisions' cost of design `x`.
fn fixed_cost(ds: &[Checked], x: &[i64]) -> f64 {
    ds.iter().zip(x).map(|(d, &k)| d.cost * d.value(k)).sum()
}

/// A stop for the time limit: true once `seconds` have passed since it was made.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn deadline(seconds: Option<f64>) -> Result<Box<dyn FnMut() -> bool>, CommandError> {
    let start = std::time::Instant::now();
    Ok(match seconds {
        Some(s) => Box::new(move || start.elapsed().as_secs_f64() >= s),
        None => Box::new(|| false),
    })
}

/// In the browser there is no clock to read here: a time limit is refused.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn deadline(seconds: Option<f64>) -> Result<Box<dyn FnMut() -> bool>, CommandError> {
    match seconds {
        Some(_) => Err(invalid(
            "a time limit is not available in the browser: give `max_runs` instead",
        )),
        None => Ok(Box::new(|| false)),
    }
}

/// What, in a model's description, means something is drawn at random: a distribution other
/// than a constant (in a time, a property, a breakdown, a pool or anywhere else), a probability
/// on a connection, an exclusive gateway (its ways are drawn when no condition decides), balking,
/// scrap, and a property drawn from a mix of values.
const DRAWS: [&str; 17] = [
    "Exponential {",
    "Uniform {",
    "Triangular {",
    "Normal {",
    "Gamma {",
    "Beta {",
    "Erlang {",
    "Weibull {",
    "Poisson {",
    "Binomial {",
    "User {",
    "Random(",
    "Choice(",
    "probability: Some(",
    "ExclusiveGateway",
    "balking: Some(",
    "scrap: Some(",
];

/// Whether nothing in `model` is drawn at random: every time is constant, and nothing in its
/// whole description (`DRAWS`) draws. Conservative: a name that happens to contain one of those
/// words also counts as a draw, which only means the designs are compared statistically.
fn nothing_drawn(model: &Model) -> bool {
    if deterministic(model) != *model {
        return false;
    }
    let text = format!("{model:?}");
    !DRAWS.iter().any(|d| text.contains(d))
}

/// The sample skewness of `v` (n ≥ 3), `None` when it cannot be worked out.
fn skewness(v: &[f64]) -> Option<f64> {
    let n = v.len() as f64;
    if v.len() < 3 {
        return None;
    }
    let mean = v.iter().sum::<f64>() / n;
    let m2 = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let m3 = v
        .iter()
        .map(|x| (x - mean) * (x - mean) * (x - mean))
        .sum::<f64>()
        / n;
    let g = m3 / (m2 * m2.sqrt());
    g.is_finite().then_some(g)
}

fn finite(x: f64) -> Value {
    if x.is_finite() { json!(x) } else { Value::Null }
}

fn outcome_name(o: Outcome) -> &'static str {
    match o {
        Outcome::Selected => "selected",
        Outcome::ComparedExactly => "compared_exactly",
        Outcome::NotProven => "not_proven",
    }
}

fn decision_name(d: Decision) -> &'static str {
    match d {
        Decision::Feasible => "feasible",
        Decision::Infeasible => "infeasible",
        Decision::Undecided => "undecided",
    }
}

/// A percentage for words: 95, 97.5.
fn percent(c: f64) -> String {
    let p = (c * 100.0 * 1e6).round() / 1e6;
    format!("{p}")
}

/// The search and the clean-up for `p` on `model`: the answer, and the model with the chosen
/// design (for `apply`).
pub fn optimise(model: &Model, p: &OptimiseParams) -> Result<(Value, Option<Model>), CommandError> {
    let ds = decisions(model, p)?;
    let delta = p.indifference;
    if !(delta.is_finite() && delta > 0.0) {
        return Err(invalid(
            "`indifference` (δ, the smallest difference in the objective worth telling apart) must be a positive number",
        ));
    }
    let confidence = p.confidence.unwrap_or(0.95);
    if !(confidence > 0.5 && confidence < 1.0) {
        return Err(invalid("`confidence` must lie between 0.5 and 1"));
    }
    for t in &p.objective.results {
        check_result(model, "result", &t.element, &t.metric)?;
        if !t.weight.unwrap_or(1.0).is_finite() {
            return Err(invalid(format!(
                "the weight of `{}.{}` must be a number",
                t.element, t.metric
            )));
        }
    }
    let mut constraints = Vec::new();
    for c in &p.constraints {
        let name = format!("{}.{}", c.element, c.metric);
        check_result(model, "constraint", &c.element, &c.metric)?;
        let (bound, limit) = match (c.at_most, c.at_least) {
            (Some(x), None) => (Bound::AtMost, x),
            (None, Some(x)) => (Bound::AtLeast, x),
            _ => {
                return Err(invalid(format!(
                    "constraint `{name}`: give `at_most` or `at_least`, one of them"
                )));
            }
        };
        if !limit.is_finite() || !(c.tolerance.is_finite() && c.tolerance > 0.0) {
            return Err(invalid(format!(
                "constraint `{name}`: the limit must be a number and the tolerance a positive number"
            )));
        }
        constraints.push(Constraint {
            bound,
            limit,
            tolerance: c.tolerance,
        });
    }
    if p.objective.results.is_empty() && constraints.is_empty() {
        return Err(invalid(
            "the objective has no result and there are no constraints, so there is nothing random to compare: give a result to minimise or maximise, or a constraint",
        ));
    }
    if p.objective.results.is_empty() && ds.iter().all(|d| d.cost == 0.0) {
        return Err(invalid(
            "the objective is empty: give a result, or a cost for at least one decision",
        ));
    }
    let max_runs = p.max_runs.unwrap_or(DEFAULT_MAX_RUNS);
    if max_runs == 0 || max_runs > MOST_RUNS {
        return Err(invalid(format!(
            "`max_runs` must lie between 1 and {MOST_RUNS}"
        )));
    }
    let reserve = p.reserve.unwrap_or(0.3);
    if !(0.0..1.0).contains(&reserve) {
        return Err(invalid("`reserve` must lie from 0 up to (not including) 1"));
    }
    if p.reserve_runs.is_some_and(|r| r > max_runs) {
        return Err(invalid("`reserve_runs` must be at most `max_runs`"));
    }
    let extra_starts = p.starts.unwrap_or(DEFAULT_STARTS);
    if extra_starts > MOST_STARTS {
        return Err(invalid(format!("`starts` must be at most {MOST_STARTS}")));
    }
    let per_visit = u64::from(p.replications_per_visit.unwrap_or(10));
    if per_visit == 0 {
        return Err(invalid("`replications_per_visit` must be at least 1"));
    }
    if p.time_limit.is_some_and(|t| !(t.is_finite() && t >= 0.0)) {
        return Err(invalid(
            "`time_limit` must be a number of seconds, at least 0",
        ));
    }
    let mut stop = deadline(p.time_limit)?;

    // The space, in steps: index k of decision i is the value min + step·k.
    let mut space = Space {
        bounds: ds.iter().map(|d| (0, i64::from(d.last))).collect(),
        limits: Vec::new(),
    };
    let start: Vec<i64> = ds
        .iter()
        .map(|d| i64::from((d.start - d.min) / d.step))
        .collect();
    if let Some(cap) = p.cost_cap {
        if !cap.is_finite() {
            return Err(invalid("`cost_cap` must be a number"));
        }
        if ds.iter().all(|d| d.cost == 0.0) {
            return Err(invalid(
                "`cost_cap` caps the decisions' costs: give a cost for at least one decision",
            ));
        }
        let at_min: f64 = ds.iter().map(|d| d.cost * f64::from(d.min)).sum();
        // The costs at the ranges' least values are the limit's offset, not subtracted from the
        // cap, so the one rule of `Space::fits` judges the whole sum against the cap itself.
        space.limits.push(Linear {
            coefficients: ds.iter().map(|d| d.cost * f64::from(d.step)).collect(),
            offset: at_min,
            limit: cap,
        });
        if !space.contains(&start) {
            return Err(invalid(format!(
                "the starting design {} costs {}, over the cap of {cap}: give each decision a `start` within it",
                design_text(&ds, &start),
                fixed_cost(&ds, &start)
            )));
        }
    }

    let x = &model.experiment;
    let seed = p.seed.unwrap_or(x.seed);
    let cleanup_seed = Rng::keyed(seed, "optimize/clean-up").next_u64();
    let weights: Vec<(Output, f64)> = p
        .objective
        .results
        .iter()
        .map(|t| {
            (
                Output {
                    element: t.element.clone(),
                    metric: t.metric.clone(),
                },
                t.weight.unwrap_or(1.0),
            )
        })
        .collect();
    let limits: Vec<Output> = p
        .constraints
        .iter()
        .map(|c| Output {
            element: c.element.clone(),
            metric: c.metric.clone(),
        })
        .collect();

    // The sampler: each design compiled once, its replications run on the search's or the
    // clean-up's seed. A failure is kept here with its code and reported as it is.
    let mut compiled: BTreeMap<Vec<i64>, Compiled> = BTreeMap::new();
    let mut failure: Option<CommandError> = None;
    let mut runs_of = |stream: Stream, design: &[i64], reps: Range<u64>| {
        let result = rows(
            model,
            &ds,
            &mut compiled,
            &weights,
            &limits,
            if stream == Stream::Search {
                seed
            } else {
                cleanup_seed
            },
            design,
            reps,
        );
        result.map_err(|e| {
            let message = e.message.clone();
            failure.get_or_insert(e);
            message
        })
    };

    // No randomness: nothing in the model is drawn, and the starting design's first two
    // replications give the same values. The probe's two runs come out of the budget, so a
    // budget of one run never compares exactly.
    let mut probe = 0;
    let exact = if max_runs >= 2 && nothing_drawn(model) {
        let r = runs_of(Stream::Search, &start, 0..2);
        probe = 2;
        match r {
            Ok(rows) => rows.first() == rows.get(1),
            Err(_) => return Err(failure.unwrap_or_else(|| invalid("the model cannot run"))),
        }
    } else {
        false
    };

    let budget = max_runs.saturating_sub(probe);
    let mut setup = Compass::new(delta, sense(p.objective.sense), budget);
    setup.confidence = confidence;
    setup.constraints = constraints.clone();
    setup.per_visit = per_visit;
    setup.all_designs_up_to = u64::from(p.all_designs_up_to.unwrap_or(50));
    setup.exact = exact;
    setup.seed = seed;
    match p.reserve_runs {
        Some(r) => setup.search_runs = budget.saturating_sub(r),
        None => setup.reserve(reserve),
    }

    let mut starts = vec![start.clone()];
    starts.extend(hypercube_starts(&ds, &space, extra_starts, seed));
    starts.dedup();

    let found = compass::optimise(&setup, &space, &starts, &mut runs_of, &mut stop);
    let answer = match found {
        Ok(a) => a,
        Err(e) => {
            return Err(failure.unwrap_or_else(|| match e {
                compass::CompassError::Setting(_) | compass::CompassError::Space(_) => {
                    invalid(e.to_string())
                }
                _ => CommandError::new(ErrorCode::Internal, e.to_string()),
            }));
        }
    };
    let words = Words {
        ds: &ds,
        p,
        answer: &answer,
        confidence,
        exact,
    };
    let v = words.answer_json(&start, seed, probe, max_runs, &setup);
    let chosen_model = match answer
        .chosen
        .and_then(|c| answer.finalists.get(c))
        .filter(|_| answer.outcome != Outcome::NotProven)
    {
        Some(f) => Some(decided(model, &ds, &f.design)?),
        None => None,
    };
    Ok((v, chosen_model))
}

fn sense(g: Goal) -> Sense {
    match g {
        Goal::Minimise => Sense::Minimise,
        Goal::Maximise => Sense::Maximise,
    }
}

/// Each replication's row for design `x`: the objective (the weighted results plus the
/// decisions' costs), then each constrained result.
#[allow(clippy::too_many_arguments)]
fn rows(
    model: &Model,
    ds: &[Checked],
    compiled: &mut BTreeMap<Vec<i64>, Compiled>,
    weights: &[(Output, f64)],
    limits: &[Output],
    seed: u64,
    x: &[i64],
    reps: Range<u64>,
) -> Result<Vec<Vec<f64>>, CommandError> {
    if !compiled.contains_key(x) {
        let m = decided(model, ds, x)?;
        let c = compile(&m).map_err(|diagnostics| CommandError {
            code: ErrorCode::InvalidModel,
            message: format!(
                "the model with the design {} has errors and cannot run",
                design_text(ds, x)
            ),
            details: Some(crate::wire::diagnostics(&m, &diagnostics)),
        })?;
        compiled.insert(x.to_vec(), c);
    }
    let Some(c) = compiled.get(x) else {
        return Err(CommandError::new(ErrorCode::Internal, "no compiled model"));
    };
    let first = reps.start;
    let count = reps.end.saturating_sub(reps.start);
    let reports = Experiment::new(count, seed)
        .run_range(reps, |r| c.run(r.rng), &Control::new())
        .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
        .into_iter()
        .zip(first..)
        .map(|(r, index)| r.map_err(|e| crate::run::run_failed(index, &e)))
        .collect::<Result<Vec<_>, _>>()?;
    let s = summarise(&reports);
    let len = usize::try_from(count).unwrap_or(usize::MAX);
    let column = |o: &Output| -> Result<Vec<f64>, CommandError> {
        let each = samples(&s, o)?;
        (0..len)
            .map(|j| {
                each.get(j).copied().flatten().ok_or_else(|| {
                    invalid(format!(
                        "`{}.{}` has no value in replication {} of the design {}: that metric does not apply to this kind of element, or nothing reached it",
                        o.element,
                        o.metric,
                        first + j as u64,
                        design_text(ds, x)
                    ))
                })
            })
            .collect()
    };
    let fixed = fixed_cost(ds, x);
    let mut objective = vec![fixed; len];
    for (o, w) in weights {
        for (total, v) in objective.iter_mut().zip(column(o)?) {
            *total += w * v;
        }
    }
    let constrained: Vec<Vec<f64>> = limits.iter().map(column).collect::<Result<_, _>>()?;
    Ok((0..len)
        .map(|j| {
            let mut row = vec![objective.get(j).copied().unwrap_or(f64::NAN)];
            row.extend(
                constrained
                    .iter()
                    .map(|c| c.get(j).copied().unwrap_or(f64::NAN)),
            );
            row
        })
        .collect())
}

/// `n` Latin hypercube points of the space (as `design.rs` samples one): each decision's column
/// from the stream `Rng::keyed(seed, "optimize/start/<id>.<field>")`, a shuffle of 0..n by
/// Fisher–Yates and an offset in each stratum, placed at ⌊u (L + 1)⌋ of its L + 1 values. Points
/// over the cap are left out.
fn hypercube_starts(ds: &[Checked], space: &Space, n: u32, seed: u64) -> Vec<Vec<i64>> {
    let n = n as usize;
    if n == 0 {
        return Vec::new();
    }
    let columns: Vec<Vec<i64>> = ds
        .iter()
        .map(|d| {
            let mut rng = Rng::keyed(seed, &format!("optimize/start/{}", d.label));
            let mut perm: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                let j = ((rng.uniform() * (i + 1) as f64) as usize).min(i);
                perm.swap(i, j);
            }
            let levels = f64::from(d.last) + 1.0;
            perm.iter()
                .map(|&q| {
                    let u = (q as f64 + rng.uniform()) / n as f64;
                    ((u * levels) as i64).clamp(0, i64::from(d.last))
                })
                .collect()
        })
        .collect();
    (0..n)
        .map(|i| {
            columns
                .iter()
                .map(|c| c.get(i).copied().unwrap_or(0))
                .collect::<Vec<i64>>()
        })
        .filter(|x| space.contains(x))
        .collect()
}

/// What goes into the answer.
struct Words<'a> {
    ds: &'a [Checked],
    p: &'a OptimiseParams,
    answer: &'a Answer,
    confidence: f64,
    exact: bool,
}

impl Words<'_> {
    fn answer_json(
        &self,
        start: &[i64],
        seed: u64,
        probe: u64,
        max_runs: u64,
        setup: &Compass,
    ) -> Value {
        let a = self.answer;
        let ds = self.ds;
        let k = a.finalists.len();
        let tried = if a.all_designs { k } else { a.tried };
        let summary = |f: &compass::Finalist| {
            let v = f.objective();
            let s = Summary::of(&v);
            json!({
                "mean": s.map(|s| finite(s.mean)),
                "half_width": s.map(|s| finite(s.half_width)),
                "n": s.map_or(0, |s| s.n),
            })
        };
        let start_at = a.finalists.iter().position(|f| f.design == start);
        let chosen = a.chosen.and_then(|c| a.finalists.get(c));
        let mut warnings = Vec::new();
        let against = match (chosen, start_at.and_then(|s| a.finalists.get(s))) {
            (Some(c), Some(s)) => {
                let common: Vec<u64> = c
                    .cleanup
                    .keys()
                    .filter(|j| s.cleanup.contains_key(j))
                    .copied()
                    .collect();
                let first = |f: &compass::Finalist| -> Vec<Option<f64>> {
                    common
                        .iter()
                        .map(|j| f.cleanup.get(j).and_then(|r| r.first().copied()))
                        .collect()
                };
                let (xs, ys) = (first(s), first(c));
                let d = difference::paired(&xs, &ys);
                let exact_difference = match (xs.first(), ys.first()) {
                    (Some(Some(x)), Some(Some(y))) => Some(y - x),
                    _ => None,
                };
                Some(json!({
                    "start": design_json(ds, start),
                    "difference": d.map(|d| finite(d.mean)).or(exact_difference.map(finite)),
                    "half_width": d.map(|d| finite(d.half_width)),
                    "n": d.map_or(common.len(), |d| d.n_a as usize),
                    "significant": d.and_then(|d| d.significant()),
                }))
            }
            _ => None,
        };
        if let Some(c) = chosen {
            let v = c.objective();
            if v.len() < 30
                && let Some(g) = skewness(&v)
                && g.abs() > 1.0
            {
                warnings.push(json!({
                    "code": "skewed",
                    "message": format!(
                        "the chosen design's {} clean-up replications are strongly skewed (skewness {:.2}); the selection assumes values close to normal, so longer runs or more replications per value make it safer",
                        v.len(),
                        g
                    ),
                }));
            }
        }
        let finalists: Vec<Value> = a
            .finalists
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let mut row = json!({
                    "decisions": design_json(ds, &f.design),
                    "cost": fixed_cost(ds, &f.design),
                    "objective": summary(f),
                    "search_mean": f.search_mean.map(finite),
                    "search_replications": f.search_replications,
                    "chosen": a.chosen == Some(i),
                    "starting_design": start_at == Some(i),
                });
                if a.outcome == Outcome::NotProven {
                    row["in_contention"] = json!(a.in_contention.contains(&i));
                }
                if let Some(fs) = &a.feasibility {
                    row["feasibility"] = json!(fs.get(i).map(|d| decision_name(*d)));
                }
                row
            })
            .collect();
        let constraints: Vec<Value> = self
            .p
            .constraints
            .iter()
            .enumerate()
            .map(|(l, c)| {
                let decided = a.chosen.and_then(|i| {
                    a.by_constraint
                        .as_ref()
                        .and_then(|b| b.get(i))
                        .and_then(|row| row.get(l))
                        .map(|d| decision_name(*d))
                        .or_else(|| {
                            a.feasibility
                                .as_ref()
                                .and_then(|f| f.get(i))
                                .map(|d| decision_name(*d))
                        })
                });
                let mean = chosen.and_then(|f| {
                    let v: Vec<f64> = f
                        .cleanup
                        .values()
                        .filter_map(|r| r.get(l + 1).copied())
                        .collect();
                    Summary::of(&v)
                });
                json!({
                    "element": c.element,
                    "metric": c.metric,
                    "at_most": c.at_most,
                    "at_least": c.at_least,
                    "tolerance": c.tolerance,
                    "chosen": decided,
                    "mean": mean.map(|s| finite(s.mean)),
                    "half_width": mean.map(|s| finite(s.half_width)),
                })
            })
            .collect();
        let ended = match a.ended {
            Ended::AllDesigns => "all_designs",
            Ended::LocalOptima => "local_optima",
            Ended::Budget => "budget",
            Ended::Stopped => "time_limit",
        };
        let mut v = json!({
            "outcome": outcome_name(a.outcome),
            "claim": self.claim(),
            "confidence": a.confidence,
            "indifference": self.p.indifference,
            "chosen": chosen.map(|f| json!({
                "decisions": design_json(ds, &f.design),
                "cost": fixed_cost(ds, &f.design),
                "objective": summary(f),
            })),
            "against_start": against,
            "constraints": constraints,
            "finalists": finalists,
            "designs_compared": k,
            "designs_tried": tried,
            "all_designs": a.all_designs,
            "space": a.space,
            "search": {
                "ended": ended,
                "starts": a.starts.iter().map(|s| json!({
                    "start": design_json(ds, &s.start),
                    "end": s.end.as_ref().map(|e| design_json(ds, e)),
                    "local_optimum": s.local_optimum,
                    "steps": s.steps,
                    "visited": s.visited,
                })).collect::<Vec<_>>(),
                "trace": a.trace.iter().map(|t| json!({
                    "runs": t.runs,
                    "best": t.best.map(finite),
                })).collect::<Vec<_>>(),
            },
            "runs": {
                "search": a.search_runs,
                "clean_up": a.cleanup_runs,
                "probe": probe,
                "total": a.total_runs().saturating_add(probe),
                "max_runs": max_runs,
                "search_budget": setup.search_runs,
            },
            "seed": seed,
            "no_randomness": self.exact,
            "warnings": warnings,
            "method": METHOD,
        });
        if a.ended == Ended::Stopped {
            v["repeat"] = json!({
                "max_runs": max_runs,
                "reserve_runs": setup.max_runs.saturating_sub(a.search_runs),
                "note": "the search stopped at its time limit after this many runs; the same parameters with these `max_runs` and `reserve_runs` (and no time limit) repeat it exactly",
            });
        }
        v
    }

    /// The claim in words.
    fn claim(&self) -> String {
        let a = self.answer;
        let ds = self.ds;
        let k = a.finalists.len();
        let scope = if a.all_designs {
            format!("of all {k} designs")
        } else {
            format!("of the {k} designs compared at the end ({} tried)", a.tried)
        };
        let chosen = a.chosen.and_then(|c| a.finalists.get(c));
        let what = chosen.map_or_else(String::new, |f| design_text(ds, &f.design));
        let delta = self.p.indifference;
        let constrained = !self.p.constraints.is_empty();
        let mut s = match (a.outcome, chosen) {
            (Outcome::Selected, Some(_)) => {
                let c = a.confidence.unwrap_or(self.confidence);
                if constrained {
                    format!(
                        "With {}% confidence, {what} is the best, or within δ = {delta} of the best, {scope} that meet the constraints, and every one of them was rightly declared feasible or infeasible (within each constraint's tolerance).",
                        percent(c)
                    )
                } else {
                    format!(
                        "With {}% confidence, {what} is the best, or within δ = {delta} of the best, {scope}.",
                        percent(c)
                    )
                }
            }
            (Outcome::ComparedExactly, Some(_)) => {
                if self.exact {
                    format!(
                        "The model has no randomness, so the designs were compared exactly, one replication each: {what} is the best {scope}. No probability is stated."
                    )
                } else if let Some(c) = a.confidence {
                    format!(
                        "With {}% confidence every design was rightly declared feasible or infeasible (within each constraint's tolerance); the objective has no randomness, so {what} is exactly the best {scope} that meet the constraints.",
                        percent(c)
                    )
                } else {
                    format!(
                        "Every replication gave the same values, so the designs were compared exactly: {what} is the best {scope}. No probability is stated."
                    )
                }
            }
            (_, Some(_)) => {
                let still: Vec<String> = a
                    .in_contention
                    .iter()
                    .filter_map(|&i| a.finalists.get(i))
                    .map(|f| design_text(ds, &f.design))
                    .collect();
                format!(
                    "Best found, not proven best: {what} looks best {scope}, but the budget ran out before the clean-up could settle it. Still in contention: {}.",
                    still.join(", ")
                )
            }
            (_, None) if constrained => format!(
                "No design was proven to meet the constraints {scope}: none was declared feasible, or the budget ran out before every one was decided."
            ),
            (_, None) => format!("Nothing was selected {scope}: the budget ran out."),
        };
        if !a.all_designs {
            s.push_str(
                " The claim covers the designs compared at the end, not every design of the space.",
            );
        }
        match a.ended {
            Ended::Budget => s.push_str(&format!(
                " The search stopped at its budget after {} runs.",
                a.search_runs
            )),
            Ended::Stopped => s.push_str(&format!(
                " The search stopped at its time limit after {} runs.",
                a.search_runs
            )),
            _ => {}
        }
        s
    }
}

const METHOD: &str = "a space is compared whole, with no search, when it has at most all_designs_up_to designs, it can be counted (its ranges hold at most a million points) and the budget pays for the clean-up's first stage (10 replications) of every design; otherwise COMPASS (Hong and Nelson 2006) over the decisions' values, with the pruning and local-optimality test of Industrial Strength COMPASS (Xu, Nelson and Hong 2010), from the starting design and Latin hypercube starts; every design runs replication i on the same seed (common random numbers); the finalists (the search's best, the designs active around it, each start's final design and the starting design) are compared again on fresh replications seeded by Rng::keyed(seed, \"optimize/clean-up\") by Kim and Nelson's KN, after Batur and Kim's feasibility check when there are constraints (alpha split equally between the two); designs over the cost cap are never run; objective: the weighted results plus the decisions' costs, per replication; objective mean and half_width: over the clean-up's replications of the design (95% t interval); against_start: the paired difference chosen minus starting design over their common clean-up replications";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skewness_of_a_symmetric_sample_is_zero() {
        assert_eq!(skewness(&[1.0, 2.0, 3.0]), Some(0.0));
        assert!(skewness(&[0.0, 0.0, 0.0, 10.0]).is_some_and(|g| g > 1.0));
        assert_eq!(skewness(&[1.0, 2.0]), None);
    }
}
