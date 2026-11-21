//! Control variates for a Monte Carlo output (Law, *Simulation Modeling and Analysis*, 5th ed.,
//! §11.3; Glasserman, *Monte Carlo Methods in Financial Engineering*, Springer, 2003, §4.1;
//! Nelson, "Control variate remedies", *Operations Research* 38(6), 1990).
//!
//! A control C is a quantity computed in every trial whose mean μ is known exactly. The output Y
//! is replaced by Y_c = Y − bᵀ(C − μ), which has the same mean as Y and, where C goes with Y,
//! a smaller variance. With several controls b is the least-squares (regression) coefficient
//! vector, which minimises that variance.
//!
//! **Unbiased.** Estimating b from the trials that are then averaged makes the estimate slightly
//! biased (Law, §11.3; Nelson, 1990). So b is fitted on a **pilot**, the first batch (1,000
//! trials; for a run of fewer than 2,000 trials the first half), and the pilot's trials are not
//! used in the controlled estimate: the remaining trials are independent of b, so E[Y_c] = E[Y]
//! exactly. The controlled mean and its interval come from those remaining trials on the run's
//! interval basis ([`crate::IntervalBasis`]): Student-t across trials, across batch means, or
//! across pair means of Y_c.
//!
//! **What is controlled.** The mean and its interval, and a target's probability (the indicator
//! of meeting the target is controlled with coefficients of its own, fitted on the same pilot).
//! Percentiles, the histogram and the extremes are not.
//!
//! **Known means.** The mean of an input is its distribution's own mean, which holds only where
//! the sampler draws exactly that distribution ([`exact_mean`]). An output control carries the
//! mean the modeller types, which cannot be checked, so the sample mean of the control over the
//! trials used is compared with it and a gross disagreement is reported ([`ControlUsed::mismatch`]).

use stochcraft_model::{Control, Dist, McOutput, MonteCarlo};
use stochcraft_stats::{Tally, t_critical_95};

use crate::{BATCH, Block, Compiled};

/// The trials of the pilot that fits the coefficients: the first batch.
pub const PILOT: u64 = BATCH;

/// A control that narrows the interval by less than this share did not help.
pub const MIN_NARROWING: f64 = 0.1;

/// How many standard errors the sample mean of a control may be from its stated mean before the
/// disagreement is reported.
const MISMATCH_Z: f64 = 5.0;

/// A count whose standard deviation is above this is drawn by Latin hypercube, antithetic and
/// Sobol sampling from an expansion of its quantile function, within 1 of the exact value, so
/// its sampled mean is not exactly its mean.
const MAX_COUNT_SD: f64 = 1e4;

/// 2^52: from here the quantile functions of counts no longer step through whole numbers.
const WHOLE_STEPS: f64 = 4_503_599_627_370_496.0;

/// Whether a draw from `d` is always the same value.
pub fn is_constant(d: &Dist) -> bool {
    matches!(d, Dist::Constant(_)) || d.variance().is_some_and(|v| v <= 0.0)
}

/// The mean of what the Monte Carlo sampler draws from `d`, when that is exactly [`Dist::mean`],
/// or why it is not or is not known. Every distribution but counts with a standard deviation
/// above 10,000 is drawn exactly, by every sampling: no draw is clamped, resampled or truncated
/// (a normal input can go negative; a truncated normal is drawn by exact rejection; a
/// constant is its value as given). Those counts are drawn by a quantile expansion under Latin
/// hypercube, antithetic and Sobol sampling, which is within 1 of the exact value but not
/// exactly the distribution, so they are refused under every sampling.
pub fn exact_mean(d: &Dist) -> Result<f64, String> {
    let m = d
        .mean()
        .ok_or_else(|| "its distribution has no closed-form mean".to_owned())?;
    let refuse = |sd: f64| {
        format!(
            "its standard deviation ({sd:.4e}) is above {MAX_COUNT_SD:.0e}, where the draws of a count are not exactly its distribution under every sampling"
        )
    };
    match d {
        Dist::Poisson { mean } if mean.sqrt() > MAX_COUNT_SD => Err(refuse(mean.sqrt())),
        Dist::Binomial {
            probability,
            trials,
        } => {
            let n = *trials as f64;
            let sd = (n * probability * (1.0 - probability)).sqrt();
            if sd > MAX_COUNT_SD || n * probability + 50.0 * sd + 100.0 >= WHOLE_STEPS {
                Err(refuse(sd))
            } else {
                Ok(m)
            }
        }
        _ => Ok(m),
    }
}

/// A control, resolved for a run: where its values are in a trial's values (inputs, then
/// outputs) and its known mean.
#[derive(Debug, Clone)]
pub(crate) struct ControlSpec {
    pub name: String,
    pub at: usize,
    pub input: bool,
    pub mean: f64,
}

/// Each output's controls, resolved. Only for a model that passed [`crate::check`].
pub(crate) fn resolve(
    mc: &MonteCarlo,
    names: &std::collections::BTreeMap<String, usize>,
) -> Vec<Vec<ControlSpec>> {
    mc.outputs
        .iter()
        .map(|o| {
            o.controls
                .iter()
                .filter_map(|c| {
                    let at = *names.get(c.name())?;
                    let (mean, input) = match c {
                        Control::Input { name } => (exact_mean(&mc.input(name)?.dist).ok()?, true),
                        Control::Output { expected, .. } => (*expected, false),
                    };
                    Some(ControlSpec {
                        name: c.name().to_owned(),
                        at,
                        input,
                        mean,
                    })
                })
                .collect()
        })
        .collect()
}

/// The problems with the controls of every output of `mc`, added to `out`; `constant_output(i)`
/// says whether output `i` always has the same value.
pub(crate) fn check(
    mc: &MonteCarlo,
    constant_output: &dyn Fn(usize) -> bool,
    out: &mut Vec<crate::Problem>,
) {
    for (k, o) in mc.outputs.iter().enumerate() {
        check_output(mc, k, o, constant_output, out);
    }
}

fn check_output(
    mc: &MonteCarlo,
    k: usize,
    o: &McOutput,
    constant_output: &dyn Fn(usize) -> bool,
    out: &mut Vec<crate::Problem>,
) {
    let at = Some(o.name.as_str());
    for (n, c) in o.controls.iter().enumerate() {
        let name = c.name();
        if o.controls.iter().take(n).any(|x| x.name() == name) {
            out.push(crate::Problem::error(
                at,
                "duplicate_control",
                format!("`{}` names the control `{name}` twice. Keep one.", o.name),
            ));
            continue;
        }
        match c {
            Control::Input { .. } => match mc.input(name) {
                None => {
                    let hint = if mc.output(name).is_some() {
                        format!(
                            "`{name}` is an output; to use it as a control, give its expected mean"
                        )
                    } else {
                        format!("`{name}` is no input")
                    };
                    out.push(crate::Problem::error(
                        at,
                        "unknown_control",
                        format!("`{}` has the control `{name}`: {hint}.", o.name),
                    ));
                }
                Some(i) if i.dist.check().is_err() => {}
                Some(i) if is_constant(&i.dist) => out.push(crate::Problem::error(
                    at,
                    "constant_control",
                    format!(
                        "The control `{name}` of `{}` is always the same value, so it cannot help. Remove it.",
                        o.name
                    ),
                )),
                Some(i) => {
                    if let Err(why) = exact_mean(&i.dist) {
                        out.push(crate::Problem::error(
                            at,
                            "inexact_control",
                            format!(
                                "The input `{name}` cannot be a control of `{}`: {why}.",
                                o.name
                            ),
                        ));
                    }
                }
            },
            Control::Output { expected, .. } => {
                let position = mc.outputs.iter().position(|x| x.name == name);
                match position {
                    None => {
                        let hint = if mc.input(name).is_some() {
                            format!("`{name}` is an input; use it as an input control")
                        } else {
                            format!("`{name}` is no output")
                        };
                        out.push(crate::Problem::error(
                            at,
                            "unknown_control",
                            format!("`{}` has the control `{name}`: {hint}.", o.name),
                        ));
                    }
                    Some(p) if p >= k => out.push(crate::Problem::error(
                        at,
                        "control_not_earlier",
                        format!(
                            "The control `{name}` of `{}` is the output itself or comes after it. An output control must be an earlier output.",
                            o.name
                        ),
                    )),
                    Some(p) => {
                        if !expected.is_finite() {
                            out.push(crate::Problem::error(
                                at,
                                "bad_control",
                                format!(
                                    "The expected mean of the control `{name}` of `{}` is not a finite number.",
                                    o.name
                                ),
                            ));
                        }
                        if constant_output(p) {
                            out.push(crate::Problem::error(
                                at,
                                "constant_control",
                                format!(
                                    "The control `{name}` of `{}` is always the same value, so it cannot help. Remove it.",
                                    o.name
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }
}

/// A control as used for an output.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlUsed {
    pub name: String,
    /// An input (its mean is exact) or an output (its mean is the one typed).
    pub input: bool,
    /// The known mean.
    pub mean: f64,
    /// The coefficient b of the output's mean, fitted on the pilot (0 until fitted, or for a
    /// control that adds nothing to the others).
    pub coefficient: f64,
    /// The control's average over the trials of the estimate.
    pub sample_mean: Option<f64>,
    /// The sample mean is more than five standard errors from the known mean: the mean is
    /// wrong (for an output, a mistyped expected mean), and the controlled estimate is biased.
    pub mismatch: bool,
}

/// An output's controlled results.
#[derive(Debug, Clone, PartialEq)]
pub struct Controlled {
    pub controls: Vec<ControlUsed>,
    /// The trials of the pilot that fitted the coefficients (left out of the estimate).
    pub pilot_trials: u64,
    /// The multiple correlation of the output with the controls on the pilot, 0 to 1: how much
    /// of the output's spread the controls can account for.
    pub correlation: Option<f64>,
    /// Why there is no estimate (English), if there is none.
    pub unavailable: Option<String>,
    pub estimate: Option<ControlledEstimate>,
}

/// The controlled mean and target probability, from the trials after the pilot.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlledEstimate {
    /// The trials after the pilot with a number in the output and every control.
    pub trials: u64,
    /// The independent units the interval is across: those trials, or their batches, or pairs.
    pub units: u64,
    pub mean: f64,
    /// The 95% half-width of the controlled mean; `None` with fewer than two units.
    pub half_width: Option<f64>,
    /// The same trials, without the controls: their mean and half-width on the same basis.
    pub uncontrolled_mean: f64,
    pub uncontrolled_half_width: Option<f64>,
    /// The variance of the uncontrolled estimate over that of the controlled, on the same
    /// trials (above 1 when the controls helped); `None` if either has no spread.
    pub variance_ratio: Option<f64>,
    /// How much narrower the interval is: 1 − controlled width / uncontrolled width (negative
    /// where the controls made it wider); `None` without both.
    pub narrowing: Option<f64>,
    pub target: Option<ControlledTarget>,
}

impl ControlledEstimate {
    /// The controls narrowed the interval by at least [`MIN_NARROWING`].
    pub fn helped(&self) -> bool {
        self.narrowing.is_some_and(|n| n >= MIN_NARROWING)
    }

    /// The controls made the interval wider: keep the uncontrolled result.
    pub fn worse(&self) -> bool {
        self.narrowing.is_some_and(|n| n < 0.0)
    }
}

/// The controlled probability of meeting a target.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlledTarget {
    /// The coefficients of the indicator of meeting the target, one per control.
    pub coefficients: Vec<f64>,
    /// The controlled probability, kept within 0 and 1.
    pub probability: f64,
    /// Its 95% interval, within 0 and 1; `None` with fewer than two units or no spread.
    pub interval: Option<(f64, f64)>,
    /// The same trials without the controls.
    pub uncontrolled_probability: f64,
    pub uncontrolled_interval: Option<(f64, f64)>,
    pub variance_ratio: Option<f64>,
    pub narrowing: Option<f64>,
}

impl ControlledTarget {
    pub fn helped(&self) -> bool {
        self.narrowing.is_some_and(|n| n >= MIN_NARROWING)
    }

    pub fn worse(&self) -> bool {
        self.narrowing.is_some_and(|n| n < 0.0)
    }
}

/// Solves the least-squares normal equations S b = s for the coefficients. A control that adds
/// nothing to the ones before it (no spread, or a copy of another) gets coefficient 0.
#[allow(clippy::needless_range_loop)]
fn solve(s: &[Vec<f64>], rhs: &[f64]) -> Vec<f64> {
    let k = rhs.len();
    let scale = (0..k)
        .filter_map(|i| s.get(i).and_then(|r| r.get(i)).copied())
        .fold(0.0_f64, f64::max);
    let tol = 1e-10 * scale;
    let mut a: Vec<Vec<f64>> = s.to_vec();
    let mut r: Vec<f64> = rhs.to_vec();
    let mut skipped = vec![false; k];
    for col in 0..k {
        let pivot = (col..k)
            .max_by(|&x, &y| {
                let v = |i: usize| {
                    a.get(i)
                        .and_then(|row| row.get(col))
                        .map_or(0.0, |v| v.abs())
                };
                v(x).total_cmp(&v(y))
            })
            .unwrap_or(col);
        let p = a
            .get(pivot)
            .and_then(|row| row.get(col))
            .copied()
            .unwrap_or(0.0);
        if p.is_nan() || p.abs() <= tol {
            if let Some(f) = skipped.get_mut(col) {
                *f = true;
            }
            continue;
        }
        a.swap(col, pivot);
        r.swap(col, pivot);
        for row in col + 1..k {
            let f = a[row][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            for j in col..k {
                let v = a[col][j];
                a[row][j] -= f * v;
            }
            let v = r[col];
            r[row] -= f * v;
        }
    }
    let mut b = vec![0.0; k];
    for i in (0..k).rev() {
        if skipped[i] {
            continue;
        }
        let tail: f64 = (i + 1..k).map(|j| a[i][j] * b[j]).sum();
        b[i] = (r[i] - tail) / a[i][i];
    }
    b
}

/// The coefficients fitted on the pilot, for the output's mean and for its target's indicator.
struct Fit {
    mean: Vec<f64>,
    target: Vec<f64>,
    correlation: f64,
}

/// Fits the pilot's rows (each `[y, z, c₁ … c_k]`, the output, the indicator of meeting the
/// target, the controls), or says why not.
fn fit(rows: &[Vec<f64>], k: usize) -> Result<Fit, String> {
    let m = rows.len();
    if m < k + 2 {
        return Err(format!(
            "the pilot gave only {m} trials with a number in the output and every control, and fitting {k} coefficient{} needs at least {}",
            if k == 1 { "" } else { "s" },
            k + 2
        ));
    }
    let n = m as f64;
    let width = k + 2;
    let mut mean = vec![0.0; width];
    for row in rows {
        for (s, v) in mean.iter_mut().zip(row) {
            *s += v / n;
        }
    }
    let cov = |a: usize, b: usize| -> f64 {
        rows.iter()
            .map(|r| (r[a] - mean[a]) * (r[b] - mean[b]))
            .sum::<f64>()
            / (n - 1.0)
    };
    let syy = cov(0, 0);
    if syy.is_nan() || syy <= 0.0 {
        return Err("the output did not vary in the pilot".to_owned());
    }
    let s: Vec<Vec<f64>> = (0..k)
        .map(|i| (0..k).map(|j| cov(2 + i, 2 + j)).collect())
        .collect();
    let scy: Vec<f64> = (0..k).map(|i| cov(2 + i, 0)).collect();
    let scz: Vec<f64> = (0..k).map(|i| cov(2 + i, 1)).collect();
    let b = solve(&s, &scy);
    let bz = solve(&s, &scz);
    let explained: f64 = b.iter().zip(&scy).map(|(x, y)| x * y).sum();
    Ok(Fit {
        mean: b,
        target: bz,
        correlation: (explained / syy).clamp(0.0, 1.0).sqrt(),
    })
}

/// The 95% half-width of the mean of the values `t` holds, or `None` with fewer than two.
fn half_width(t: &Tally) -> Option<f64> {
    let n = t.count();
    let tc = t_critical_95(n.checked_sub(1)?)?;
    Some(tc * (t.variance()? / n as f64).sqrt()).filter(|h| h.is_finite())
}

fn interval(p: f64, h: Option<f64>) -> Option<(f64, f64)> {
    h.filter(|h| *h > 0.0)
        .map(|h| ((p - h).max(0.0), (p + h).min(1.0)))
}

fn ratio(uncontrolled: &Tally, controlled: &Tally) -> Option<f64> {
    let (u, c) = (uncontrolled.variance()?, controlled.variance()?);
    (u > 0.0 && c > 0.0).then_some(u / c)
}

fn narrowing(uncontrolled: Option<f64>, controlled: Option<f64>) -> Option<f64> {
    let (u, c) = (uncontrolled?, controlled?);
    (u > 0.0).then(|| 1.0 - c / u)
}

/// What a unit (a trial, a batch or a pair) has gathered of its usable trials.
#[derive(Default)]
struct Unit {
    key: Option<u64>,
    n: u64,
    y: f64,
    yc: f64,
    z: f64,
    zc: f64,
}

/// The values across units.
#[derive(Default)]
struct Across {
    y: Tally,
    yc: Tally,
    z: Tally,
    zc: Tally,
    trials: u64,
}

impl Across {
    fn flush(&mut self, u: &Unit) {
        if u.n > 0 {
            let n = u.n as f64;
            self.y.add(u.y / n);
            self.yc.add(u.yc / n);
            self.z.add(u.z / n);
            self.zc.add(u.zc / n);
            self.trials += u.n;
        }
    }
}

/// What [`each_trial`] calls for each trial: its index, its block, the output and the controls.
type TrialFn<'a> = dyn FnMut(u64, usize, f64, &[f64]) + 'a;

/// Calls `f(index, block, y, controls)` for each trial of output `o` in order.
fn each_trial(
    blocks: &[Block],
    inputs: usize,
    specs: &[ControlSpec],
    o: usize,
    f: &mut TrialFn<'_>,
) {
    let mut index = 0_u64;
    let mut values = vec![0.0; specs.len()];
    for (bi, b) in blocks.iter().enumerate() {
        let Some(y) = b.outputs.get(o) else {
            continue;
        };
        let columns: Vec<&Vec<f64>> = specs
            .iter()
            .filter_map(|s| {
                if s.at < inputs {
                    b.inputs.get(s.at)
                } else {
                    b.outputs.get(s.at - inputs)
                }
            })
            .collect();
        if columns.len() != specs.len() {
            index += y.len() as u64;
            continue;
        }
        for (t, yv) in y.iter().enumerate() {
            for (slot, col) in values.iter_mut().zip(&columns) {
                *slot = col.get(t).copied().unwrap_or(f64::NAN);
            }
            f(index, bi, *yv, &values);
            index += 1;
        }
    }
}

/// How a run's trials are grouped into independent units.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Grouping {
    Trials,
    Batches,
    Pairs,
}

/// Output `o`'s controlled results from the run's `blocks`, in one linear pass over its trials
/// (after fitting the pilot's few rows); `None` for an output with no controls.
pub(crate) fn controlled(c: &Compiled, o: usize, blocks: &[Block]) -> Option<Controlled> {
    let specs = c.controls.get(o)?;
    if specs.is_empty() {
        return None;
    }
    let k = specs.len();
    let target = c.outputs.get(o).and_then(|(out, _)| out.target);
    let grouping = if c.batched() {
        Grouping::Batches
    } else if c.paired() {
        Grouping::Pairs
    } else {
        Grouping::Trials
    };
    let total: u64 = blocks
        .iter()
        .filter_map(|b| b.outputs.get(o))
        .map(|col| col.len() as u64)
        .sum();
    let mut pilot = if total >= 2 * PILOT { PILOT } else { total / 2 };
    match grouping {
        Grouping::Batches => {
            pilot = blocks
                .first()
                .and_then(|b| b.outputs.get(o))
                .map_or(0, |col| col.len() as u64);
        }
        Grouping::Pairs => pilot -= pilot % 2,
        Grouping::Trials => {}
    }
    let mut used: Vec<ControlUsed> = specs
        .iter()
        .map(|s| ControlUsed {
            name: s.name.clone(),
            input: s.input,
            mean: s.mean,
            coefficient: 0.0,
            sample_mean: None,
            mismatch: false,
        })
        .collect();
    let none = |used: Vec<ControlUsed>, why: String| {
        Some(Controlled {
            controls: used,
            pilot_trials: pilot,
            correlation: None,
            unavailable: Some(why),
            estimate: None,
        })
    };
    if grouping == Grouping::Batches && blocks.len() < 2 {
        return none(
            used,
            "controls need at least two batches: the first fits the coefficients and the estimate uses the rest".to_owned(),
        );
    }
    if pilot == 0 {
        return none(
            used,
            "the run is too short: the first half of its trials fits the coefficients and the second half is the estimate".to_owned(),
        );
    }

    let inputs = c.inputs.len();
    let mut rows: Vec<Vec<f64>> = Vec::new();
    let mut fitted: Option<Result<Fit, String>> = None;
    let mut unit = Unit::default();
    let mut across = Across::default();
    let mut samples: Vec<Tally> = vec![Tally::new(); k];
    each_trial(
        blocks,
        inputs,
        specs,
        o,
        &mut |index: u64, bi: usize, y: f64, cv: &[f64]| {
            let usable = y.is_finite() && cv.iter().all(|v| v.is_finite());
            let z = match target {
                Some(t) if t.met(y) => 1.0,
                _ => 0.0,
            };
            if index < pilot {
                if usable {
                    let mut row = vec![y, z];
                    row.extend_from_slice(cv);
                    rows.push(row);
                }
                return;
            }
            if fitted.is_none() {
                fitted = Some(fit(&rows, k));
            }
            let Some(Ok(f)) = fitted.as_ref() else {
                return;
            };
            if !usable {
                return;
            }
            let key = match grouping {
                Grouping::Trials => index,
                Grouping::Batches => bi as u64,
                Grouping::Pairs => index / 2,
            };
            if unit.key != Some(key) {
                across.flush(&unit);
                unit = Unit {
                    key: Some(key),
                    ..Unit::default()
                };
            }
            let shift = |b: &[f64]| -> f64 {
                b.iter()
                    .zip(cv)
                    .zip(specs)
                    .map(|((b, v), s)| b * (v - s.mean))
                    .sum()
            };
            unit.n += 1;
            unit.y += y;
            unit.yc += y - shift(&f.mean);
            unit.z += z;
            unit.zc += z - shift(&f.target);
            for (t, v) in samples.iter_mut().zip(cv) {
                t.add(*v);
            }
        },
    );
    across.flush(&unit);

    let fit = match fitted {
        Some(Ok(f)) => f,
        Some(Err(why)) => return none(used, why),
        None => return none(used, "no trials are left after the pilot".to_owned()),
    };
    for (u, (b, t)) in used.iter_mut().zip(fit.mean.iter().zip(&samples)) {
        u.coefficient = *b;
        u.sample_mean = t.mean();
        u.mismatch = match (t.mean(), t.variance()) {
            (Some(m), Some(v)) if v > 0.0 => {
                (m - u.mean).abs() > MISMATCH_Z * (v / t.count() as f64).sqrt()
            }
            _ => false,
        };
    }
    let correlation = Some(fit.correlation);
    let (Some(mean), Some(raw)) = (across.yc.mean(), across.y.mean()) else {
        return Some(Controlled {
            controls: used,
            pilot_trials: pilot,
            correlation,
            unavailable: Some(
                "no trials after the pilot have a number in the output and every control"
                    .to_owned(),
            ),
            estimate: None,
        });
    };
    let hw = half_width(&across.yc);
    let raw_hw = half_width(&across.y);
    let target_result = target.map(|_| {
        let p = across.zc.mean().unwrap_or(0.0).clamp(0.0, 1.0);
        let raw_p = across.z.mean().unwrap_or(0.0).clamp(0.0, 1.0);
        let (h, raw_h) = (half_width(&across.zc), half_width(&across.z));
        ControlledTarget {
            coefficients: fit.target.clone(),
            probability: p,
            interval: interval(p, h),
            uncontrolled_probability: raw_p,
            uncontrolled_interval: interval(raw_p, raw_h),
            variance_ratio: ratio(&across.z, &across.zc),
            narrowing: narrowing(raw_h, h),
        }
    });
    Some(Controlled {
        controls: used,
        pilot_trials: pilot,
        correlation,
        unavailable: None,
        estimate: Some(ControlledEstimate {
            trials: across.trials,
            units: across.yc.count(),
            mean,
            half_width: hw,
            uncontrolled_mean: raw,
            uncontrolled_half_width: raw_hw,
            variance_ratio: ratio(&across.y, &across.yc),
            narrowing: narrowing(raw_hw, hw),
            target: target_result,
        }),
    })
}
