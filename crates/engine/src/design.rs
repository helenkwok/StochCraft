//! Designed experiments with several factors at once (M2.8): a full factorial, every
//! combination of the factors' levels, or a Latin hypercube sample of their ranges.
//!
//! As in a sweep, every design point runs the model's replications on the same seed, so
//! replication i of every point meets the same random numbers (common random numbers; A. M.
//! Law, *Simulation Modeling and Analysis*, 5th ed., McGraw-Hill, 2015, section 11.2). An effect
//! is a contrast between points, worked out replication by replication; its 95% interval is the
//! t interval on those per-replication contrasts, n − 1 degrees of freedom for n replications
//! (Law, section 12.2, for 2^k designs).
//!
//! - **Full factorial**: the points are every combination of the levels, in the order of
//!   Python's `itertools.product` (the last factor changes fastest). The main effect of a
//!   factor is the mean of the result over the points at its highest level minus the mean over
//!   the points at its lowest (by value), so averaged over the other factors' levels. For two
//!   factors with two levels each, the two-way interaction is half the change in one factor's
//!   effect between the other's high and low level, Law's e12 (section 12.2): the mean over
//!   the points where both are high or both low minus the mean over the rest.
//! - **Latin hypercube** (M. D. McKay, R. J. Beckman and W. J. Conover, "A comparison of three
//!   methods for selecting values of input variables in the analysis of output from a computer
//!   code", *Technometrics* 21(2), 1979, 239–245): each factor's range is cut into as many
//!   equal strata as there are points, and each stratum is used exactly once. For n points,
//!   factor k's value at point i is at (π_k(i) + U_ik) / n of its range, π_k a random
//!   permutation of 0..n and U_ik uniform on [0, 1). The draws come from the stream
//!   `Rng::keyed(seed, "design/<id>.<field>")`, one per factor, so the same seed always gives the
//!   same design, and adding a factor leaves the others' columns as they were. Each factor
//!   first shuffles 0..n by Fisher–Yates (i from n − 1 down to 1, swapping i with
//!   ⌊U (i + 1)⌋), then draws its n offsets U in point order. A factor given as levels, or a
//!   count (units, a buffer) given as a range, which means its whole numbers, takes the level
//!   at ⌊u L⌋ of its L sorted levels. Each result is judged by its rank (Spearman) correlation
//!   with each factor across the points.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_model::Model;
use stochcraft_random::Rng;
use stochcraft_stats::difference;

use crate::CommandError;
use crate::decide::{Field, Output, Param, invalid, metric, reps_seed, run, samples};

/// The most runs (design points × replications) a design may take. A 3 × 3 × 3 factorial with
/// 20 replications is 540 runs; a 9 × 5 grid with 100 is 4,500.
pub const MAX_RUNS: u64 = 10_000;

/// How the design points are chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Every combination of the factors' levels.
    #[default]
    FullFactorial,
    /// `points` points, each factor's range cut into `points` strata, each stratum used once.
    LatinHypercube,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::FullFactorial => "full_factorial",
            Kind::LatinHypercube => "latin_hypercube",
        }
    }
}

/// A factor: a parameter (as a sweep's) with its levels, or its range as `min` and `max`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Factor {
    /// The element or pool.
    pub id: String,
    pub field: Field,
    /// The values to try (whole numbers for counts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub levels: Option<Vec<f64>>,
    /// The range, instead of levels: in a full factorial its two ends are the two levels; in a
    /// Latin hypercube it is sampled (a count takes its whole numbers from `min` to `max`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl Factor {
    fn param(&self) -> Param {
        Param {
            id: self.id.clone(),
            field: self.field,
        }
    }

    fn label(&self) -> String {
        format!("{}.{}", self.id, self.field.name())
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DesignParams {
    pub factors: Vec<Factor>,
    /// The results to report at each point.
    pub results: Vec<Output>,
    /// `full_factorial` (the default) or `latin_hypercube`.
    #[serde(default)]
    pub design: Kind,
    /// A Latin hypercube's number of points.
    #[serde(default)]
    pub points: Option<u32>,
    /// Replications at each point (default: the experiment's).
    #[serde(default)]
    pub replications: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
}

/// What a factor takes: levels, or a continuous range.
enum Values {
    Levels(Vec<f64>),
    Range(f64, f64),
}

fn values(f: &Factor, kind: Kind) -> Result<Values, CommandError> {
    let label = f.label();
    match (&f.levels, f.min, f.max) {
        (Some(levels), None, None) => {
            if levels.is_empty() {
                return Err(invalid(format!("`{label}` has no levels")));
            }
            if let Some(x) = levels.iter().find(|x| !x.is_finite()) {
                return Err(invalid(format!(
                    "`{label}` has a level that is not a number: {x}"
                )));
            }
            for (i, x) in levels.iter().enumerate() {
                if levels[..i].contains(x) {
                    return Err(invalid(format!("`{label}` has the level {x} twice")));
                }
            }
            Ok(Values::Levels(levels.clone()))
        }
        (None, Some(lo), Some(hi)) => {
            if !(lo.is_finite() && hi.is_finite() && lo < hi) {
                return Err(invalid(format!(
                    "`{label}`'s range must have `min` below `max`, both numbers"
                )));
            }
            match kind {
                Kind::FullFactorial => Ok(Values::Levels(vec![lo, hi])),
                Kind::LatinHypercube if f.field.integral() => {
                    if lo.fract() != 0.0 || hi.fract() != 0.0 {
                        return Err(invalid(format!(
                            "`{label}` is a count: give its range as whole numbers"
                        )));
                    }
                    if hi - lo >= MAX_RUNS as f64 {
                        return Err(invalid(format!(
                            "`{label}`'s range has more than {MAX_RUNS} whole numbers"
                        )));
                    }
                    let n = (hi - lo) as u64;
                    Ok(Values::Levels((0..=n).map(|i| lo + i as f64).collect()))
                }
                Kind::LatinHypercube => Ok(Values::Range(lo, hi)),
            }
        }
        _ => Err(invalid(format!(
            "give `{label}` either `levels` or `min` and `max`"
        ))),
    }
}

/// The refusal for a design of `runs` runs.
fn too_many(runs: u128, points: u128, reps: u64) -> CommandError {
    invalid(format!(
        "the design would take {runs} runs ({points} points × {reps} replications); the limit is \
         {MAX_RUNS}: use fewer levels, points or replications"
    ))
}

/// Every combination of the levels, the last factor changing fastest.
fn factorial(levels: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let mut points = vec![Vec::new()];
    for ls in levels {
        points = points
            .into_iter()
            .flat_map(|p: Vec<f64>| {
                ls.iter().map(move |&x| {
                    let mut q = p.clone();
                    q.push(x);
                    q
                })
            })
            .collect();
    }
    points
}

/// Factor `f`'s column of an `n`-point Latin hypercube with `seed` (see the module's comment).
fn hypercube_column(f: &Factor, v: &Values, n: usize, seed: u64) -> Vec<f64> {
    let mut rng = Rng::keyed(seed, &format!("design/{}", f.label()));
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = ((rng.uniform() * (i + 1) as f64) as usize).min(i);
        perm.swap(i, j);
    }
    let sorted = match v {
        Values::Levels(ls) => {
            let mut s = ls.clone();
            s.sort_by(f64::total_cmp);
            s
        }
        Values::Range(..) => Vec::new(),
    };
    perm.iter()
        .map(|&p| {
            let u = (p as f64 + rng.uniform()) / n as f64;
            match v {
                Values::Range(lo, hi) => lo + u * (hi - lo),
                Values::Levels(_) => {
                    let k = ((u * sorted.len() as f64) as usize).min(sorted.len() - 1);
                    sorted[k]
                }
            }
        })
        .collect()
}

/// One result at one point: its summary and its value in each replication.
struct Cell {
    mean: Option<f64>,
    half_width: Option<f64>,
    n: u64,
    samples: Vec<Option<f64>>,
}

/// The per-replication contrast mean(`plus` points) − mean(`minus` points) of result `r`, with
/// its 95% t interval, as fields of an effect. A replication where any of those points has no
/// value is left out.
fn contrast(
    cells: &[Vec<Cell>],
    r: usize,
    reps: usize,
    minus: &[usize],
    plus: &[usize],
) -> serde_json::Map<String, Value> {
    let side = |ps: &[usize]| -> Vec<Option<f64>> {
        (0..reps)
            .map(|i| {
                let mut sum = 0.0;
                for &p in ps {
                    sum += cells[p][r].samples.get(i).copied().flatten()?;
                }
                (!ps.is_empty()).then(|| sum / ps.len() as f64)
            })
            .collect()
    };
    let d = difference::paired(&side(minus), &side(plus));
    let mut m = serde_json::Map::new();
    m.insert("effect".into(), json!(d.map(|d| d.mean)));
    m.insert("half_width".into(), json!(d.map(|d| d.half_width)));
    m.insert("n".into(), json!(d.map_or(0, |d| d.n_a)));
    m.insert("significant".into(), json!(d.and_then(|d| d.significant())));
    m
}

/// Average ranks (1-based) of `xs`, ties sharing the mean of their ranks.
fn ranks(xs: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..xs.len()).collect();
    order.sort_by(|&a, &b| xs[a].total_cmp(&xs[b]));
    let mut r = vec![0.0; xs.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && xs[order[j + 1]] == xs[order[i]] {
            j += 1;
        }
        let rank = (i + j) as f64 / 2.0 + 1.0;
        for &k in &order[i..=j] {
            r[k] = rank;
        }
        i = j + 1;
    }
    r
}

/// Spearman's rank correlation: the Pearson correlation of the average ranks. `None` with fewer
/// than three pairs or when either side is constant.
pub(crate) fn spearman(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() != y.len() || x.len() < 3 {
        return None;
    }
    let (rx, ry) = (ranks(x), ranks(y));
    let n = rx.len() as f64;
    let (mx, my) = (rx.iter().sum::<f64>() / n, ry.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in rx.iter().zip(&ry) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    (sxx > 0.0 && syy > 0.0).then(|| sxy / (sxx * syy).sqrt())
}

/// Runs a designed experiment: each result at each point, and the factors' effects on them.
pub fn design(model: &Model, p: &DesignParams) -> Result<Value, CommandError> {
    if p.factors.is_empty() || p.results.is_empty() {
        return Err(invalid("give at least one factor and one result"));
    }
    for o in &p.results {
        metric(&o.metric)?;
    }
    for (i, f) in p.factors.iter().enumerate() {
        if p.factors[..i]
            .iter()
            .any(|g| g.id == f.id && g.field == f.field)
        {
            return Err(invalid(format!("`{}` is a factor twice", f.label())));
        }
        // The parameter exists and has this field.
        f.param().get(model)?;
    }
    let (reps, seed) = reps_seed(model, p.replications, p.seed);
    let vals: Vec<Values> = p
        .factors
        .iter()
        .map(|f| values(f, p.design))
        .collect::<Result<_, _>>()?;
    let points: Vec<Vec<f64>> = match p.design {
        Kind::FullFactorial => {
            if p.points.is_some() {
                return Err(invalid(
                    "`points` is for a Latin hypercube; a full factorial runs every combination",
                ));
            }
            let levels: Vec<Vec<f64>> = vals
                .iter()
                .map(|v| match v {
                    Values::Levels(ls) => ls.clone(),
                    Values::Range(lo, hi) => vec![*lo, *hi],
                })
                .collect();
            let count = levels
                .iter()
                .fold(1u128, |acc, ls| acc.saturating_mul(ls.len() as u128));
            let runs = count.saturating_mul(u128::from(reps));
            if runs > u128::from(MAX_RUNS) {
                return Err(too_many(runs, count, reps));
            }
            factorial(&levels)
        }
        Kind::LatinHypercube => {
            let n = p
                .points
                .ok_or_else(|| invalid("a Latin hypercube needs its number of `points`"))?;
            if n < 2 {
                return Err(invalid("a Latin hypercube needs at least 2 points"));
            }
            let runs = u128::from(n) * u128::from(reps);
            if runs > u128::from(MAX_RUNS) {
                return Err(too_many(runs, u128::from(n), reps));
            }
            let n = n as usize;
            let columns: Vec<Vec<f64>> = p
                .factors
                .iter()
                .zip(&vals)
                .map(|(f, v)| hypercube_column(f, v, n, seed))
                .collect();
            (0..n)
                .map(|i| columns.iter().map(|c| c[i]).collect())
                .collect()
        }
    };
    // Every point's model first, so a level that cannot be set fails before anything runs.
    let models: Vec<Model> = points
        .iter()
        .map(|xs| {
            let mut m = model.clone();
            for (f, &x) in p.factors.iter().zip(xs) {
                f.param().set(&mut m, x)?;
            }
            Ok(m)
        })
        .collect::<Result<_, CommandError>>()?;
    // The points one after another, as in a sweep; each runs its replications in parallel.
    let mut cells: Vec<Vec<Cell>> = Vec::with_capacity(models.len());
    for m in &models {
        let s = run(m, reps, seed)?;
        let row = p
            .results
            .iter()
            .map(|o| {
                let x = s.get(&o.element, metric(&o.metric)?);
                Ok(Cell {
                    mean: x.map(|x| x.mean),
                    half_width: x.map(|x| x.half_width),
                    n: x.map_or(0, |x| x.n),
                    samples: samples(&s, o)?.to_vec(),
                })
            })
            .collect::<Result<Vec<_>, CommandError>>()?;
        cells.push(row);
    }
    let point_json: Vec<Value> = points
        .iter()
        .zip(&cells)
        .map(|(xs, row)| {
            let results: Vec<Value> = p
                .results
                .iter()
                .zip(row)
                .map(|(o, c)| {
                    json!({
                        "element": o.element,
                        "metric": o.metric,
                        "mean": c.mean,
                        "half_width": c.half_width,
                        "n": c.n,
                    })
                })
                .collect();
            json!({ "values": xs, "results": results })
        })
        .collect();
    let factors_json: Vec<Value> = p
        .factors
        .iter()
        .zip(&vals)
        .map(|(f, v)| match v {
            Values::Levels(ls) => json!({ "id": f.id, "field": f.field, "levels": ls }),
            Values::Range(lo, hi) => json!({ "id": f.id, "field": f.field, "min": lo, "max": hi }),
        })
        .collect();
    let reps_n = reps as usize;
    let mut answer = json!({
        "design": p.design.name(),
        "factors": factors_json,
        "replications": reps,
        "seed": seed,
        "runs": points.len() as u64 * reps,
        "points": point_json,
    });
    match p.design {
        Kind::FullFactorial => {
            let mut main = Vec::new();
            for (k, f) in p.factors.iter().enumerate() {
                let (lo, hi) = points
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |a, x| {
                        (a.0.min(x[k]), a.1.max(x[k]))
                    });
                if lo == hi {
                    continue;
                }
                let at = |v: f64| -> Vec<usize> {
                    (0..points.len()).filter(|&i| points[i][k] == v).collect()
                };
                let (low, high) = (at(lo), at(hi));
                for (r, o) in p.results.iter().enumerate() {
                    let mut e = contrast(&cells, r, reps_n, &low, &high);
                    e.insert("id".into(), json!(f.id));
                    e.insert("field".into(), json!(f.field));
                    e.insert("low".into(), json!(lo));
                    e.insert("high".into(), json!(hi));
                    e.insert("element".into(), json!(o.element));
                    e.insert("metric".into(), json!(o.metric));
                    main.push(Value::Object(e));
                }
            }
            // Two-way interactions of the factors with two levels, as in a 2^k design.
            let two: Vec<usize> = vals
                .iter()
                .enumerate()
                .filter(|(_, v)| matches!(v, Values::Levels(ls) if ls.len() == 2))
                .map(|(k, _)| k)
                .collect();
            let mut inter = Vec::new();
            for (a, &i) in two.iter().enumerate() {
                for &j in &two[a + 1..] {
                    let high = |k: usize, x: f64| matches!(&vals[k], Values::Levels(ls) if x == ls[0].max(ls[1]));
                    let (mut plus, mut minus) = (Vec::new(), Vec::new());
                    for (q, x) in points.iter().enumerate() {
                        if high(i, x[i]) == high(j, x[j]) {
                            plus.push(q);
                        } else {
                            minus.push(q);
                        }
                    }
                    for (r, o) in p.results.iter().enumerate() {
                        let mut e = contrast(&cells, r, reps_n, &minus, &plus);
                        e.insert(
                            "factors".into(),
                            json!([p.factors[i].param(), p.factors[j].param()]),
                        );
                        e.insert("element".into(), json!(o.element));
                        e.insert("metric".into(), json!(o.metric));
                        inter.push(Value::Object(e));
                    }
                }
            }
            let tests = (main.len() + inter.len()) as u64;
            answer["main_effects"] = json!(main);
            answer["interactions"] = json!(inter);
            answer["tests"] = json!(tests);
            answer["method"] = json!(
                "full factorial: every combination of the levels, the last factor changing fastest, each point run with the same seed (common random numbers), so replication i of every point meets the same random numbers; main effect: the mean of the result over the points at the factor's highest level minus the mean over the points at its lowest, so averaged over the other factors' levels; interaction (two factors with two levels each, as in a 2^k design, Law section 12.2): half the change in one factor's effect between the other's high and low level, the mean over the points where both are high or both low minus the mean over the rest; each effect is worked out replication by replication, and half_width is the 95% t half-width of those values (n − 1 degrees of freedom, n the replications where every point involved has a value); significant: that interval excludes 0 (null with fewer than two replications). Each of the `tests` effects is tested on its own at 95%, so about one in twenty of those with no real effect still shows as significant by chance; for 95% confidence in all of them at once (Bonferroni), each would need its own interval at 1 − 0.05/tests, which is wider"
            );
        }
        Kind::LatinHypercube => {
            let mut corr = Vec::new();
            for (k, f) in p.factors.iter().enumerate() {
                for (r, o) in p.results.iter().enumerate() {
                    let (xs, ys): (Vec<f64>, Vec<f64>) = points
                        .iter()
                        .zip(&cells)
                        .filter_map(|(x, row)| Some((x[k], row[r].mean?)))
                        .unzip();
                    corr.push(json!({
                        "id": f.id,
                        "field": f.field,
                        "element": o.element,
                        "metric": o.metric,
                        "spearman": spearman(&xs, &ys),
                        "n": xs.len(),
                    }));
                }
            }
            answer["correlations"] = json!(corr);
            answer["method"] = json!(
                "Latin hypercube (McKay, Beckman and Conover, Technometrics 21, 1979): each factor's range cut into as many equal strata as there are points, each stratum used once, the strata matched at random between factors; the draws come from the seed, so the same seed gives the same design; each point run with the same seed (common random numbers); spearman: the rank correlation across the points between the factor's value and the result's mean (ties share their mean rank; null with fewer than three points with a value or when either is constant), from −1 to 1, near 0 when the factor does not drive the result in one direction"
            );
        }
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factorial_points_follow_itertools_product() {
        let pts = factorial(&[vec![1.0, 2.0], vec![10.0, 20.0, 30.0]]);
        assert_eq!(
            pts,
            vec![
                vec![1.0, 10.0],
                vec![1.0, 20.0],
                vec![1.0, 30.0],
                vec![2.0, 10.0],
                vec![2.0, 20.0],
                vec![2.0, 30.0],
            ]
        );
    }

    #[test]
    fn spearman_uses_average_ranks() {
        assert_eq!(spearman(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0]), Some(1.0));
        assert_eq!(spearman(&[1.0, 2.0, 3.0], &[3.0, 2.0, 1.0]), Some(-1.0));
        assert_eq!(spearman(&[1.0, 1.0, 1.0], &[3.0, 2.0, 1.0]), None);
        assert_eq!(spearman(&[1.0, 2.0], &[1.0, 2.0]), None);
        assert_eq!(ranks(&[5.0, 1.0, 5.0, 2.0]), vec![3.5, 1.0, 3.5, 2.0]);
        // A monotone but not linear relation is a rank correlation of 1.
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        let y: Vec<f64> = x.iter().map(|v| v * v * v).collect();
        assert_eq!(spearman(&x, &y), Some(1.0));
    }
}
