//! Sobol sensitivity indices (ROADMAP M4.1 part 3): how much of an output's variance each input
//! accounts for, alone (the first-order index S_i) and with all its interactions (the total
//! index S_Ti).
//!
//! **Design.** Saltelli's (Saltelli, "Making best use of model evaluations to compute
//! sensitivity indices", *Computer Physics Communications* 145, 2002; Saltelli et al., *Global
//! Sensitivity Analysis: The Primer*, Wiley, 2008, §4.6): two independent base samples A and B,
//! each N rows of the k varied inputs, and for each input i the matrix A_B^(i): A with column i
//! taken from B. That is N(k + 2) evaluations of the model. A and B are drawn from the inputs'
//! own distributions by plain random sampling. Latin hypercube sampling is not used: the
//! estimators below assume rows drawn independently, and the model's own sampling setting does
//! not apply.
//!
//! **Random streams.** Batch b of A's draws for input `x` come from the stream keyed
//! `sobol/A/<b>/input/<x>` under the seed given to [`Compiled::sobol`], B's from
//! `sobol/B/<b>/input/<x>` (batches of [`BATCH`] rows, as a run's), and the bootstrap's from
//! `sobol/bootstrap/<output>`. None is a stream of a plain run, and a row's draws depend only on
//! the seed, its batch and the input, so adding an input leaves the others' draws as they were.
//!
//! **Estimators.** With f the output, f(A), f(B) and f(A_B^i) its values on the rows:
//!
//! - first order, Saltelli, Annoni, Azzini, Campolongo, Ratto and Tarantola, "Variance based
//!   sensitivity analysis of model output. Design and estimator for the total sensitivity
//!   index", *Computer Physics Communications* 181(2), 259–270 (2010):
//!   S_i = mean(f(B) · (f(A_B^i) − f(A))) / V;
//! - total effect, Jansen, "Analysis of model uncertainty and sensitivity: the estimators of
//!   Sobol's indices", *Journal of Statistical Computation and Simulation* 64(4), 1999:
//!   S_Ti = mean((f(A) − f(A_B^i))²) / 2 / V;
//! - V is the sample variance (n − 1) of f over the union of f(A) and f(B).
//!
//! The formulas are those SALib's documentation gives (SALib, MIT licence). f(B) is centred on
//! the mean of the union before the first-order product, which changes nothing in expectation
//! and keeps an output with a large mean from swamping it with noise.
//!
//! **Intervals.** A bootstrap over the N rows (all of a row's values resampled together, so its
//! pairing is kept; Efron and Tibshirani, *An Introduction to the Bootstrap*, 1993), by default
//! 1,000 resamples, with a fixed stream: the 2.5th and 97.5th percentiles of the resampled
//! index. Each resample recomputes V as well.
//!
//! **Limits.** The indices assume independent inputs (Saltelli et al., 2008), so a model with
//! correlated inputs is refused. An estimate can fall a little outside [0, 1], or a first-order
//! index above its total, when N is small: they are estimates, and are not clamped. Indices of a
//! model whose output depends on a draw other than its inputs are not defined.

use stochcraft_model::Dist;
use stochcraft_model::expr::Value;
use stochcraft_random::Rng;
use stochcraft_stats::spread::quantile as sorted_quantile;

use crate::{BATCH, Compiled, MAX_TRIALS, draw};

/// The rows of each base sample a run uses unless asked otherwise.
pub const DEFAULT_ROWS: u64 = 1_000;

/// The bootstrap resamples a run makes unless asked otherwise.
pub const DEFAULT_BOOTSTRAP: u32 = 1_000;

/// Row-input operations of bootstrap work one [`SobolRun::step`] does (about 2^18), so a step is
/// short however many rows and inputs there are.
pub const STEP_OPERATIONS: usize = 1 << 18;

/// The fewest rows (of each base sample, or kept for an output) the estimators are made from.
pub const MIN_ROWS: u64 = 10;

/// What a Sobol run makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SobolOptions {
    /// Bootstrap resamples for the intervals; 0 for no intervals.
    pub bootstrap: u32,
}

impl Default for SobolOptions {
    fn default() -> Self {
        SobolOptions {
            bootstrap: DEFAULT_BOOTSTRAP,
        }
    }
}

/// Why a Sobol run cannot be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SobolError {
    /// Inputs are correlated (these names): the indices assume independent ones.
    Correlated(Vec<String>),
    /// Every input is a constant, so there is nothing to attribute variance to.
    NoVariedInputs,
    /// Fewer than [`MIN_ROWS`] rows asked for.
    TooFewRows,
    /// More evaluations than [`MAX_TRIALS`], N(k + 2).
    TooMany { evaluations: u64 },
}

impl std::fmt::Display for SobolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SobolError::Correlated(names) => write!(
                f,
                "Sobol indices need independent inputs, and {} {} correlated. Remove the \
                 correlations to get them (or read the rank correlations of the outputs instead).",
                names.join(", "),
                if names.len() == 1 { "is" } else { "are" }
            ),
            SobolError::NoVariedInputs => write!(
                f,
                "Every input is a constant, so no input can account for any variation."
            ),
            SobolError::TooFewRows => {
                write!(
                    f,
                    "Ask for at least {MIN_ROWS} rows to estimate Sobol indices."
                )
            }
            SobolError::TooMany { evaluations } => write!(
                f,
                "That is {evaluations} evaluations of the model, N × (inputs + 2); at most \
                 {MAX_TRIALS} are allowed. Ask for fewer rows."
            ),
        }
    }
}

impl std::error::Error for SobolError {}

/// An estimate with its 95% bootstrap interval (percentile method), if asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    pub value: f64,
    pub interval: Option<(f64, f64)>,
}

/// One input's indices for one output.
#[derive(Debug, Clone, PartialEq)]
pub struct SobolIndex {
    pub input: String,
    /// S_i: the share of the variance the input accounts for alone.
    pub first: Estimate,
    /// S_Ti: the share it accounts for with all its interactions.
    pub total: Estimate,
}

/// One output's indices, or why it has none.
#[derive(Debug, Clone, PartialEq)]
pub struct SobolOutput {
    pub name: String,
    /// V: the output's variance over the two base samples (when it has one).
    pub variance: Option<f64>,
    /// Rows used (those whose values are finite in every matrix).
    pub rows_used: u64,
    /// Rows left out for a value that is not a finite number.
    pub rows_dropped: u64,
    /// One per varied input, in the model's order; empty when `unavailable` says why.
    pub indices: Vec<SobolIndex>,
    pub unavailable: Option<String>,
}

/// A Sobol run's results.
#[derive(Debug, Clone, PartialEq)]
pub struct SobolResults {
    /// The rows of each base sample, N.
    pub rows: u64,
    /// The model evaluations made, N(k + 2).
    pub evaluations: u64,
    pub seed: u64,
    /// Bootstrap resamples (0: no intervals).
    pub bootstrap: u32,
    /// The inputs the indices are for (those that vary), in the model's order.
    pub inputs: Vec<String>,
    /// Constant inputs, left out.
    pub constants: Vec<String>,
    pub outputs: Vec<SobolOutput>,
}

/// A Sobol run, made a piece at a time: [`SobolRun::step`] evaluates one batch of rows, or
/// works out one output's indices, until [`SobolRun::is_done`].
#[derive(Debug, Clone)]
pub struct SobolRun {
    rows: u64,
    seed: u64,
    bootstrap: u32,
    /// Indices (into the inputs) of the inputs that vary.
    varied: Vec<usize>,
    /// Their names.
    names: Vec<String>,
    constants: Vec<String>,
    batches: u64,
    next_batch: u64,
    /// Per output: f(A), f(B), and per varied input f(A_B^i), one per row.
    fa: Vec<Vec<f64>>,
    fb: Vec<Vec<f64>>,
    fab: Vec<Vec<Vec<f64>>>,
    outputs: Vec<SobolOutput>,
    /// The output whose bootstrap is under way.
    boot: Option<Box<Boot>>,
    /// Bootstrap work (rows × inputs) a step does at most.
    step_operations: usize,
    results: Option<SobolResults>,
}

/// An output's bootstrap, made a few resamples a step: everything its intervals need, with the
/// keyed stream and the resamples collected so far, so that stopping between steps changes
/// nothing.
#[derive(Debug, Clone)]
struct Boot {
    /// The output as it is without intervals.
    out: SobolOutput,
    point: Point,
    data: Data,
    rng: Rng,
    done: u32,
    first: Vec<Vec<f64>>,
    total: Vec<Vec<f64>>,
    idx: Vec<usize>,
}

impl Compiled {
    /// Sobol indices of every output, from `rows` rows of each base sample (see the module's
    /// documentation), with `seed` for the draws.
    pub fn sobol(
        &self,
        rows: u64,
        seed: u64,
        options: &SobolOptions,
    ) -> Result<SobolResults, SobolError> {
        let mut run = self.sobol_run(rows, seed, options)?;
        loop {
            if let Some(results) = run.take_results() {
                return Ok(results);
            }
            run.step(self);
        }
    }

    /// A Sobol run to make a piece at a time with [`SobolRun::step`], which needs this same
    /// model.
    pub fn sobol_run(
        &self,
        rows: u64,
        seed: u64,
        options: &SobolOptions,
    ) -> Result<SobolRun, SobolError> {
        if self.correlated.len() >= 2 {
            let names = self
                .correlated
                .iter()
                .filter_map(|&i| self.inputs.get(i).map(|i| i.name.clone()))
                .collect();
            return Err(SobolError::Correlated(names));
        }
        let (varied, constants): (Vec<usize>, Vec<usize>) =
            (0..self.inputs.len()).partition(|&i| {
                !matches!(self.inputs.get(i).map(|i| &i.dist), Some(Dist::Constant(_)))
            });
        if varied.is_empty() {
            return Err(SobolError::NoVariedInputs);
        }
        if rows < MIN_ROWS {
            return Err(SobolError::TooFewRows);
        }
        let evaluations = rows.saturating_mul(varied.len() as u64 + 2);
        if evaluations > MAX_TRIALS {
            return Err(SobolError::TooMany { evaluations });
        }
        let outs = self.outputs.len();
        let cap = usize::try_from(rows).unwrap_or(0);
        Ok(SobolRun {
            rows,
            seed,
            bootstrap: options.bootstrap,
            names: varied
                .iter()
                .filter_map(|&i| self.inputs.get(i).map(|i| i.name.clone()))
                .collect(),
            constants: constants
                .iter()
                .filter_map(|&i| self.inputs.get(i).map(|i| i.name.clone()))
                .collect(),
            batches: rows.div_ceil(BATCH),
            next_batch: 0,
            fa: vec![Vec::with_capacity(cap); outs],
            fb: vec![Vec::with_capacity(cap); outs],
            fab: vec![vec![Vec::with_capacity(cap); varied.len()]; outs],
            varied,
            outputs: Vec::new(),
            boot: None,
            step_operations: STEP_OPERATIONS,
            results: None,
        })
    }

    /// Evaluates every output for the inputs in the first slots of `values` (the inputs, then
    /// room for the outputs), as a trial does.
    fn fill_outputs(&self, values: &mut [f64]) {
        let k = self.inputs.len();
        for (o, (_, e)) in self.outputs.iter().enumerate() {
            let v = {
                let read = |name: &str| {
                    self.names
                        .get(name)
                        .and_then(|&at| values.get(at))
                        .map(|v| Value::Number(*v))
                };
                e.eval(&read).number()
            };
            if let Some(slot) = values.get_mut(k + o) {
                *slot = v;
            }
        }
    }
}

impl SobolRun {
    /// Limits the bootstrap work of a step to about `operations` (rows × inputs, at least one
    /// resample). The results do not depend on it.
    #[must_use]
    pub fn with_step_operations(mut self, operations: usize) -> Self {
        self.step_operations = operations;
        self
    }

    pub fn is_done(&self) -> bool {
        self.results.is_some()
    }

    /// How far along, from 0 to 1: batches evaluated, then outputs worked out.
    pub fn progress(&self, compiled: &Compiled) -> f64 {
        let total = self.batches + compiled.outputs.len() as u64;
        if total == 0 {
            return 1.0;
        }
        (self.next_batch + self.outputs.len() as u64) as f64 / total as f64
    }

    pub fn take_results(&mut self) -> Option<SobolResults> {
        self.results.take()
    }

    /// Does one piece of the work: a batch of up to [`BATCH`] rows evaluated (N(k + 2) model
    /// evaluations in all), then one output's indices and bootstrap. `compiled` is the model
    /// the run was made from.
    pub fn step(&mut self, compiled: &Compiled) {
        if self.results.is_some() {
            return;
        }
        if self.next_batch < self.batches {
            self.evaluate_batch(compiled, self.next_batch);
            self.next_batch += 1;
        } else if let Some(mut boot) = self.boot.take() {
            if boot.advance(self.bootstrap, self.step_operations) {
                let out = self.finish(*boot);
                self.outputs.push(out);
            } else {
                self.boot = Some(boot);
            }
        } else if self.outputs.len() < compiled.outputs.len() {
            let o = self.outputs.len();
            let name = compiled
                .outputs
                .get(o)
                .map(|(out, _)| out.name.clone())
                .unwrap_or_default();
            match self.summarise(o, name) {
                Summary::Done(out) => self.outputs.push(out),
                Summary::Boot(boot) => self.boot = Some(boot),
            }
        } else {
            self.results = Some(SobolResults {
                rows: self.rows,
                evaluations: self.rows * (self.varied.len() as u64 + 2),
                seed: self.seed,
                bootstrap: self.bootstrap,
                inputs: self.names.clone(),
                constants: self.constants.clone(),
                outputs: std::mem::take(&mut self.outputs),
            });
        }
    }

    fn evaluate_batch(&mut self, c: &Compiled, batch: u64) {
        let m = BATCH.min(self.rows - batch * BATCH);
        let m = usize::try_from(m).unwrap_or(0);
        let columns = |which: &str| -> Vec<Vec<f64>> {
            c.inputs
                .iter()
                .map(|input| {
                    let mut rng = Rng::keyed(
                        self.seed,
                        &format!("sobol/{which}/{batch}/input/{}", input.name),
                    );
                    (0..m).map(|_| draw(&input.dist, &mut rng)).collect()
                })
                .collect()
        };
        let a = columns("A");
        let b = columns("B");
        let k = c.inputs.len();
        let mut values = vec![0.0; k + c.outputs.len()];
        let outs = c.outputs.len();
        for t in 0..m {
            let at = |cols: &[Vec<f64>], i: usize| {
                cols.get(i)
                    .and_then(|col| col.get(t))
                    .copied()
                    .unwrap_or(f64::NAN)
            };
            for (i, slot) in values.iter_mut().take(k).enumerate() {
                *slot = at(&a, i);
            }
            c.fill_outputs(&mut values);
            for (o, col) in self.fa.iter_mut().enumerate().take(outs) {
                col.push(values.get(k + o).copied().unwrap_or(f64::NAN));
            }
            for (i, slot) in values.iter_mut().take(k).enumerate() {
                *slot = at(&b, i);
            }
            c.fill_outputs(&mut values);
            for (o, col) in self.fb.iter_mut().enumerate().take(outs) {
                col.push(values.get(k + o).copied().unwrap_or(f64::NAN));
            }
            for (v, &i) in self.varied.iter().enumerate() {
                for (j, slot) in values.iter_mut().take(k).enumerate() {
                    *slot = if j == i { at(&b, j) } else { at(&a, j) };
                }
                c.fill_outputs(&mut values);
                for (o, per_input) in self.fab.iter_mut().enumerate().take(outs) {
                    if let Some(col) = per_input.get_mut(v) {
                        col.push(values.get(k + o).copied().unwrap_or(f64::NAN));
                    }
                }
            }
        }
    }

    /// Output `o`'s indices from the evaluations.
    fn summarise(&self, o: usize, name: String) -> Summary {
        let empty = Vec::new();
        let fa = self.fa.get(o).unwrap_or(&empty);
        let fb = self.fb.get(o).unwrap_or(&empty);
        let fab: Vec<&Vec<f64>> = (0..self.varied.len())
            .map(|v| self.fab.get(o).and_then(|p| p.get(v)).unwrap_or(&empty))
            .collect();
        // Rows whose value is finite in every matrix.
        let keep: Vec<usize> = (0..fa.len())
            .filter(|&t| {
                let ok = |col: &[f64]| col.get(t).is_some_and(|x| x.is_finite());
                ok(fa.as_slice()) && ok(fb.as_slice()) && fab.iter().all(|col| ok(col.as_slice()))
            })
            .collect();
        let dropped = (fa.len() - keep.len()) as u64;
        let pick = |col: &[f64]| -> Vec<f64> {
            keep.iter()
                .map(|&t| col.get(t).copied().unwrap_or(f64::NAN))
                .collect()
        };
        let data = Data {
            a: pick(fa.as_slice()),
            b: pick(fb.as_slice()),
            ab: fab.iter().map(|col| pick(col.as_slice())).collect(),
        };
        let used = keep.len() as u64;
        let none = |why: String, variance: Option<f64>| {
            Summary::Done(SobolOutput {
                name: name.clone(),
                variance,
                rows_used: used,
                rows_dropped: dropped,
                indices: Vec::new(),
                unavailable: Some(why),
            })
        };
        if used < MIN_ROWS {
            return none(
                format!(
                    "Only {used} of {} rows have a finite value in every matrix; at least \
                     {MIN_ROWS} are needed.",
                    fa.len()
                ),
                None,
            );
        }
        let Some(point) = data.indices(None) else {
            return none("The output's variance is not a finite number.".into(), None);
        };
        if !point.variance.is_finite() {
            return none("The output's variance is not a finite number.".into(), None);
        }
        let (lo, hi) = data.spread();
        if point.variance <= 0.0 || hi <= lo {
            return none(
                "The output does not vary (its variance is 0), so no input accounts for any of it."
                    .into(),
                Some(0.0),
            );
        }
        let m = data.a.len();
        let k = data.ab.len();
        let out = SobolOutput {
            name: name.clone(),
            variance: Some(point.variance),
            rows_used: used,
            rows_dropped: dropped,
            indices: Vec::new(),
            unavailable: None,
        };
        let rng = Rng::keyed(self.seed, &format!("sobol/bootstrap/{name}"));
        Summary::Boot(Box::new(Boot {
            out,
            point,
            data,
            rng,
            done: 0,
            first: vec![Vec::new(); k],
            total: vec![Vec::new(); k],
            idx: vec![0usize; m],
        }))
    }

    /// An output whose bootstrap is made: its indices with their 95% percentile intervals (none
    /// without a bootstrap).
    fn finish(&self, boot: Boot) -> SobolOutput {
        let Boot {
            mut out,
            point,
            first,
            total,
            ..
        } = boot;
        let intervals = (self.bootstrap > 0).then(|| {
            let band = |mut xs: Vec<f64>| -> Option<(f64, f64)> {
                xs.retain(|x| x.is_finite());
                xs.sort_by(f64::total_cmp);
                Some((sorted_quantile(&xs, 0.025)?, sorted_quantile(&xs, 0.975)?))
            };
            let bands = |all: Vec<Vec<f64>>| -> Vec<(f64, f64)> {
                all.into_iter()
                    .map(|xs| band(xs).unwrap_or((f64::NAN, f64::NAN)))
                    .collect()
            };
            Intervals {
                first: bands(first),
                total: bands(total),
            }
        });
        out.indices = self
            .names
            .iter()
            .enumerate()
            .map(|(v, input)| SobolIndex {
                input: input.clone(),
                first: Estimate {
                    value: point.first.get(v).copied().unwrap_or(f64::NAN),
                    interval: intervals.as_ref().and_then(|i| i.first.get(v).copied()),
                },
                total: Estimate {
                    value: point.total.get(v).copied().unwrap_or(f64::NAN),
                    interval: intervals.as_ref().and_then(|i| i.total.get(v).copied()),
                },
            })
            .collect();
        out
    }
}

/// What working out an output's indices comes to: done, or a bootstrap still to make.
enum Summary {
    Done(SobolOutput),
    Boot(Box<Boot>),
}

impl Boot {
    /// Makes the next resamples, as many as `operations` (rows × inputs) allows and at least
    /// one, from the output's keyed stream in order. True once all `total` are made.
    fn advance(&mut self, total: u32, operations: usize) -> bool {
        let m = self.data.a.len();
        let per = (operations / (m.max(1) * (self.first.len() + 1))).max(1);
        let mut made = 0;
        while self.done < total && made < per {
            for slot in self.idx.iter_mut() {
                *slot = usize::try_from(self.rng.next_u64() % m as u64).unwrap_or(0);
            }
            self.done += 1;
            made += 1;
            let Some(p) = self.data.indices_at(&self.idx, Some(self.point.centre)) else {
                continue;
            };
            if !(p.variance.is_finite() && p.variance > 0.0) {
                continue;
            }
            for (v, (f, t)) in self.first.iter_mut().zip(self.total.iter_mut()).enumerate() {
                f.push(p.first.get(v).copied().unwrap_or(f64::NAN));
                t.push(p.total.get(v).copied().unwrap_or(f64::NAN));
            }
        }
        self.done >= total
    }
}

/// Intervals per varied input.
struct Intervals {
    first: Vec<(f64, f64)>,
    total: Vec<(f64, f64)>,
}

/// An output's kept rows: f(A), f(B) and f(A_B^i) for each varied input.
#[derive(Debug, Clone)]
struct Data {
    a: Vec<f64>,
    b: Vec<f64>,
    ab: Vec<Vec<f64>>,
}

/// The estimators' values on a set of rows.
#[derive(Debug, Clone)]
struct Point {
    variance: f64,
    /// The mean of f over A and B, which f(B) is centred on.
    centre: f64,
    first: Vec<f64>,
    total: Vec<f64>,
}

impl Data {
    /// The least and greatest value of f(A) and f(B).
    fn spread(&self) -> (f64, f64) {
        let all = self.a.iter().chain(&self.b);
        let lo = all.clone().copied().fold(f64::INFINITY, f64::min);
        let hi = all.copied().fold(f64::NEG_INFINITY, f64::max);
        (lo, hi)
    }

    /// The estimators on every row, centring on the sample mean.
    fn indices(&self, centre: Option<f64>) -> Option<Point> {
        let all: Vec<usize> = (0..self.a.len()).collect();
        self.indices_at(&all, centre)
    }

    /// The estimators on the rows `idx` (a bootstrap resample, or every row), centring f(B) on
    /// `centre`, or on the sample mean of the rows if none is given.
    fn indices_at(&self, idx: &[usize], centre: Option<f64>) -> Option<Point> {
        let get = |col: &[f64], t: usize| col.get(t).copied();
        let m = idx.len();
        if m < 2 {
            return None;
        }
        let mut sum = 0.0;
        for &t in idx {
            sum += get(&self.a, t)? + get(&self.b, t)?;
        }
        let mean = sum / (2 * m) as f64;
        let c = centre.unwrap_or(mean);
        // Variance over the union, about the mean, in two passes.
        let mut ss = 0.0;
        for &t in idx {
            let (x, y) = (get(&self.a, t)? - mean, get(&self.b, t)? - mean);
            ss += x * x + y * y;
        }
        let variance = ss / (2 * m - 1) as f64;
        let mut first = Vec::with_capacity(self.ab.len());
        let mut total = Vec::with_capacity(self.ab.len());
        for col in &self.ab {
            let (mut s1, mut st) = (0.0, 0.0);
            for &t in idx {
                let (fa, fb, fab) = (get(&self.a, t)?, get(&self.b, t)?, get(col, t)?);
                s1 += (fb - c) * (fab - fa);
                let d = fa - fab;
                st += d * d;
            }
            first.push(s1 / m as f64 / variance);
            total.push(st / (2 * m) as f64 / variance);
        }
        Some(Point {
            variance,
            centre: mean,
            first,
            total,
        })
    }
}
