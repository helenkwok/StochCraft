//! Monte Carlo models (ROADMAP M4.1): a calculation, not a process.
//!
//! A [`MonteCarlo`] model (held in `stochcraft_model`) has inputs, each a distribution, and
//! outputs, each a BPSim expression over the inputs and the outputs before it. Each trial draws
//! every input once and evaluates every output in order. [`check`] says what is wrong with a
//! model, [`compile`] gets one ready to run, and [`Compiled::run`] runs it.
//!
//! **Trials and random streams.** Trials are made in batches of [`BATCH`]. Batch b's draws for
//! input `x` come from the stream keyed `mc/<b>/input/<x>` under the seed
//! (`stochcraft_random::Rng::keyed`), and the order Iman–Conover pairs them in from the stream
//! `mc/<b>/correlation`, so a batch depends only on the seed, its index and the model: results
//! are the same however many threads make the batches ([`Compiled::run_with`] lets a caller make
//! them in parallel), and adding an input leaves the others' draws as they were.
//!
//! **Sampling.** Plain random sampling draws each value independently. Latin hypercube sampling
//! (McKay, Beckman and Conover, "A comparison of three methods for selecting values of input
//! variables in the analysis of output from a computer code", *Technometrics* 21(2), 1979) cuts
//! each input's range, within each batch of n trials, into n equally likely strata and draws
//! one value from each, at a random place in it, through the inverse distribution function
//! ([`quantile`]); the strata are paired across inputs at random.
//!
//! **Correlation.** Rank correlations between inputs are induced by the method of Iman and
//! Conover (1982) within each batch ([`correlate`]).
//!
//! **Results** ([`McResults`]): for each output its mean with a 95% interval, standard
//! deviation, percentiles (Hyndman and Fan's definition 7), minimum and maximum, the probability
//! of meeting its target with its 95% interval, and a histogram and cumulative curve for charts.
//! A trial whose output is not a finite number (a division by zero, a missing value) is counted
//! apart and left out of them.
//!
//! **Intervals** ([`IntervalBasis`]). With plain random sampling and no correlations the trials
//! are independent, so the mean's interval is Student-t across trials and the target's is
//! Wilson's score interval. Latin hypercube sampling and Iman–Conover's reordering make the
//! trials within a batch dependent (a batch of 1,000 Latin hypercube draws of a uniform holds
//! exactly 500 below its median), so a per-trial interval would measure the wrong design. Each
//! batch, though, is an independently randomised design with streams of its own, so the batches
//! are independent and identically distributed: replicated Latin hypercube sampling (Owen, "A
//! central limit theorem for Latin hypercube sampling", *JRSS B* 54(2), 1992; Glasserman,
//! *Monte Carlo Methods in Financial Engineering*, Springer, 2003, §4.4). In those modes the
//! mean's interval is Student-t on the batch means (b − 1 degrees of freedom) and the target's
//! is Student-t on the batch proportions, clamped to [0, 1]; runs are whole batches (the trials
//! asked for are rounded up to a multiple of [`BATCH`]); with fewer than two batches there is
//! no interval. Point estimates are the same either way.
//!
//! **Sequential runs.** With a stop ([`McStop`]), batches are added until every output's mean
//! has a 95% half-width within the share of itself asked for, by the rule of
//! `stochcraft_stats::precision` (Hoad, Robinson and Davies, 2010), judged on the same interval
//! the results report (across batches in the modes above, which needs two batches at least).

use std::collections::BTreeMap;

use stochcraft_model::expr::{self, Expr, Kind, Value};
pub use stochcraft_model::{
    Dist, McCorrelation, McInput, McOutput, McStop, McTarget, MonteCarlo, Sampling,
};
use stochcraft_random::Rng;
use stochcraft_stats::precision::{self, Reason, Scale, Step};
use stochcraft_stats::spread::quantile as sorted_quantile;
use stochcraft_stats::{Proportion, Summary, t_critical_95};

pub mod correlate;
pub mod quantile;
pub mod table;

pub use correlate::rank_correlation;

/// Values sorted per step when the results are worked out a piece at a time.
const SORT_CHUNK: usize = 1 << 18;

/// Elements moved per merge step of that sort.
const MERGE_BUDGET: usize = 1 << 19;

/// Trials per batch: the unit of Latin hypercube strata, of correlation and of parallel work.
pub const BATCH: u64 = 1_000;

/// The most trials a run may have (each keeps every input's and output's value).
pub const MAX_TRIALS: u64 = 10_000_000;

/// The percentiles every output reports, from 0 to 100.
pub const PERCENTILES: [f64; 5] = [5.0, 10.0, 50.0, 90.0, 95.0];

/// Something wrong with a Monte Carlo model.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// An error stops the model running; a warning does not.
    pub error: bool,
    /// The input, output or correlation (`a~b`) it is about.
    pub at: Option<String>,
    /// A stable code: `no_outputs`, `bad_name`, `duplicate_name`, `bad_distribution`,
    /// `bad_expression`, `unknown_name`, `not_yet_computed`, `text_output`, `bad_target`,
    /// `bad_correlation`, `inconsistent_correlations`, `bad_stop` or `unused_input`.
    pub code: &'static str,
    /// What is wrong and what to do, in English.
    pub message: String,
}

impl Problem {
    fn error(at: Option<&str>, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            error: true,
            at: at.map(str::to_owned),
            code,
            message: message.into(),
        }
    }

    fn warning(at: Option<&str>, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            error: false,
            ..Self::error(at, code, message)
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = if self.error { "error" } else { "warning" };
        match &self.at {
            Some(at) => write!(f, "{level} at `{at}`: {}", self.message),
            None => write!(f, "{level}: {}", self.message),
        }
    }
}

/// Whether `name` can name an input or an output: letters, digits and `_`, starting with a
/// letter or `_`, so that `$name` reads it in an expression.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// What a model's expressions mean once parsed, and its correlations as a factored matrix.
struct Checked {
    exprs: Vec<Option<Expr>>,
    /// Indices (into the inputs) of the correlated inputs, and the Cholesky factor of their
    /// matrix of score correlations.
    correlated: Vec<usize>,
    factor: Vec<Vec<f64>>,
}

fn check_all(mc: &MonteCarlo) -> (Vec<Problem>, Checked) {
    let mut out = Vec::new();
    if mc.outputs.is_empty() {
        out.push(Problem::error(
            None,
            "no_outputs",
            "The model has no outputs. Add one: an expression over the inputs, such as $labour * $rate.",
        ));
    }
    // Names: inputs, then outputs, one namespace.
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    let names = mc
        .inputs
        .iter()
        .map(|i| i.name.as_str())
        .chain(mc.outputs.iter().map(|o| o.name.as_str()));
    for (k, name) in names.enumerate() {
        if !valid_name(name) {
            out.push(Problem::error(
                Some(name),
                "bad_name",
                format!(
                    "`{name}` cannot be a name: use letters, digits and _, starting with a letter or _."
                ),
            ));
        }
        if seen.insert(name, k).is_some() {
            out.push(Problem::error(
                Some(name),
                "duplicate_name",
                format!("Two inputs or outputs are called `{name}`. Give each its own name."),
            ));
        }
    }
    for i in &mc.inputs {
        if let Err(e) = i.dist.check() {
            out.push(Problem::error(
                Some(i.name.as_str()),
                "bad_distribution",
                format!(
                    "The distribution of `{}` cannot be drawn from: {e}.",
                    i.name
                ),
            ));
        }
    }
    // Expressions: each may read the inputs and the outputs before it.
    let inputs = mc.inputs.len();
    let mut used = vec![false; inputs];
    let mut exprs = Vec::new();
    for (k, o) in mc.outputs.iter().enumerate() {
        let parsed = match expr::parse_with_variables(&o.expression) {
            Ok(e) => e,
            Err(e) => {
                out.push(Problem::error(
                    Some(o.name.as_str()),
                    "bad_expression",
                    format!("The expression of `{}` cannot be read: {e}.", o.name),
                ));
                exprs.push(None);
                continue;
            }
        };
        let mut ok = true;
        for r in parsed.references() {
            match seen.get(r) {
                Some(&at) if at < inputs => {
                    if let Some(u) = used.get_mut(at) {
                        *u = true;
                    }
                }
                Some(&at) if at < inputs + k => {}
                Some(_) => {
                    ok = false;
                    out.push(Problem::error(
                        Some(o.name.as_str()),
                        "not_yet_computed",
                        format!(
                            "`{}` uses `{r}`, which is computed after it (or is itself). Move `{r}` above it, or use the inputs instead.",
                            o.name
                        ),
                    ));
                }
                None => {
                    ok = false;
                    out.push(Problem::error(
                        Some(o.name.as_str()),
                        "unknown_name",
                        format!(
                            "`{}` uses `{r}`, which is no input or output. Add the input, or correct the name.",
                            o.name
                        ),
                    ));
                }
            }
        }
        if parsed.kind() == Kind::Text {
            ok = false;
            out.push(Problem::error(
                Some(o.name.as_str()),
                "text_output",
                format!(
                    "`{}` gives text, not a number. Make it an arithmetic expression or a condition.",
                    o.name
                ),
            ));
        }
        if let Some(t) = o.target
            && !t.value().is_finite()
        {
            out.push(Problem::error(
                Some(o.name.as_str()),
                "bad_target",
                format!("The target of `{}` is not a finite number.", o.name),
            ));
        }
        exprs.push(ok.then_some(parsed));
    }
    for (k, i) in mc.inputs.iter().enumerate() {
        if !mc.outputs.is_empty() && !used.get(k).copied().unwrap_or(true) {
            out.push(Problem::warning(
                Some(i.name.as_str()),
                "unused_input",
                format!(
                    "No output uses `{}`, so it changes no result. Use it in an output, or remove it.",
                    i.name
                ),
            ));
        }
    }
    // Correlations: between two different inputs, strictly between -1 and 1, once per pair.
    let index = |n: &str| mc.inputs.iter().position(|i| i.name == n);
    let mut pairs: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for c in &mc.correlations {
        let at = format!("{}~{}", c.a, c.b);
        let bad = |message: String| Problem::error(Some(at.as_str()), "bad_correlation", message);
        match (index(c.a.as_str()), index(c.b.as_str())) {
            (Some(a), Some(b)) if a == b => {
                out.push(bad(format!("`{}` cannot be correlated with itself.", c.a)));
            }
            (Some(_), Some(_)) if !(c.rank.is_finite() && c.rank.abs() < 1.0) => {
                out.push(bad(format!(
                    "The rank correlation of `{}` and `{}` must be strictly between -1 and 1, not {}.",
                    c.a, c.b, c.rank
                )));
            }
            (Some(a), Some(b)) => {
                if pairs.insert((a.min(b), a.max(b)), c.rank).is_some() {
                    out.push(bad(format!(
                        "`{}` and `{}` are correlated twice. Keep one.",
                        c.a, c.b
                    )));
                }
            }
            (a, _) => {
                let missing = if a.is_none() { &c.a } else { &c.b };
                out.push(bad(format!(
                    "`{missing}` is no input, so it cannot be correlated."
                )));
            }
        }
    }
    let correlated: Vec<usize> = (0..inputs)
        .filter(|i| pairs.keys().any(|(a, b)| a == i || b == i))
        .collect();
    let k = correlated.len();
    let mut matrix = vec![vec![0.0; k]; k];
    for (r, row) in matrix.iter_mut().enumerate() {
        row[r] = 1.0;
    }
    for (&(a, b), &rank) in &pairs {
        let (Some(i), Some(j)) = (
            correlated.iter().position(|x| *x == a),
            correlated.iter().position(|x| *x == b),
        ) else {
            continue;
        };
        let c = correlate::score_correlation(rank);
        matrix[i][j] = c;
        matrix[j][i] = c;
    }
    let factor = if k >= 2 {
        match correlate::cholesky(&matrix) {
            Some(l) => l,
            None => {
                let names: Vec<String> = correlated
                    .iter()
                    .filter_map(|i| mc.inputs.get(*i))
                    .map(|i| format!("`{}`", i.name))
                    .collect();
                out.push(Problem::error(
                    None,
                    "inconsistent_correlations",
                    format!(
                        "The rank correlations between {} cannot all hold at once (their matrix is not positive definite): if a goes with b and b with c, a cannot go strongly against c. Make some of them weaker.",
                        names.join(", ")
                    ),
                ));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    if let Some(s) = mc.stop
        && !(s.relative.is_finite() && s.relative > 0.0 && s.max >= 1)
    {
        out.push(Problem::error(
            None,
            "bad_stop",
            "A sequential stop needs a share of the mean above 0 and at least one trial.",
        ));
    } else if let Some(s) = mc.stop
        && (mc.sampling == Sampling::LatinHypercube || k >= 2)
        && u64::from(s.max) < BATCH
    {
        out.push(Problem::error(
            None,
            "bad_stop",
            format!(
                "Latin hypercube sampling and correlated inputs sample in whole batches, so the most trials of a sequential stop must be at least one batch (1,000 trials); {} is too few. Raise the most, or use random sampling with no correlations.",
                s.max
            ),
        ));
    }
    out.sort_by_key(|p| !p.error);
    (
        out,
        Checked {
            exprs,
            correlated,
            factor,
        },
    )
}

/// Every problem with the model, errors first.
pub fn check(mc: &MonteCarlo) -> Vec<Problem> {
    check_all(mc).0
}

/// A model ready to run with a seed.
#[derive(Debug, Clone)]
pub struct Compiled {
    seed: u64,
    sampling: Sampling,
    inputs: Vec<McInput>,
    outputs: Vec<(McOutput, Expr)>,
    /// Where each name's value is in a trial's values: inputs, then outputs.
    names: BTreeMap<String, usize>,
    correlated: Vec<usize>,
    factor: Vec<Vec<f64>>,
    correlations: Vec<McCorrelation>,
    /// The model's warnings.
    pub warnings: Vec<Problem>,
}

/// What makes a set of batches for [`Compiled::run_with`]: in the order asked, or why not.
pub type MakeBlocks<'a, E> = dyn FnMut(&[BlockJob]) -> Result<Vec<Block>, E> + 'a;

/// One batch to make: its index and its number of trials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockJob {
    pub index: u64,
    pub trials: u64,
}

/// A batch's values: each input's and each output's, one per trial.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub index: u64,
    pub inputs: Vec<Vec<f64>>,
    pub outputs: Vec<Vec<f64>>,
}

/// Gets `mc` ready to run with `seed`, or says what stops it.
pub fn compile(mc: &MonteCarlo, seed: u64) -> Result<Compiled, Vec<Problem>> {
    let (problems, checked) = check_all(mc);
    if problems.iter().any(|p| p.error) {
        return Err(problems);
    }
    let names = mc
        .inputs
        .iter()
        .map(|i| i.name.clone())
        .chain(mc.outputs.iter().map(|o| o.name.clone()))
        .enumerate()
        .map(|(k, n)| (n, k))
        .collect();
    let outputs = mc
        .outputs
        .iter()
        .zip(checked.exprs)
        .filter_map(|(o, e)| Some((o.clone(), e?)))
        .collect();
    Ok(Compiled {
        seed,
        sampling: mc.sampling,
        inputs: mc.inputs.clone(),
        outputs,
        names,
        correlated: checked.correlated,
        factor: checked.factor,
        correlations: mc.correlations.clone(),
        warnings: problems,
    })
}

/// The batches that make trials `from` up to (not including) `to`; `from` is a multiple of
/// [`BATCH`].
pub fn jobs(from: u64, to: u64) -> Vec<BlockJob> {
    let mut out = Vec::new();
    let mut start = from;
    while start < to {
        let trials = BATCH.min(to - start);
        out.push(BlockJob {
            index: start / BATCH,
            trials,
        });
        start += trials;
    }
    out
}

/// A draw from `d`: a constant is its value as given (negative too).
fn draw(d: &Dist, rng: &mut Rng) -> f64 {
    match d {
        Dist::Constant(v) => *v,
        other => other.sample(rng),
    }
}

/// What a run makes.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOptions {
    /// The trials of a fixed run; a sequential run's first trials (rounded up to whole batches).
    pub trials: u64,
    /// More trials until the outputs' means are precise enough.
    pub stop: Option<McStop>,
    /// Percentiles (0 to 100) to report besides [`PERCENTILES`].
    pub percentiles: Vec<f64>,
}

impl Compiled {
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Whether trials within a batch are dependent (Latin hypercube sampling, or correlations
    /// that reorder them), so that intervals come from batch means and runs are whole batches.
    pub fn batched(&self) -> bool {
        self.sampling == Sampling::LatinHypercube || self.correlated.len() >= 2
    }

    /// The trials a run of `trials` makes: whole batches where [`Compiled::batched`], at most
    /// [`MAX_TRIALS`] (itself whole batches).
    pub fn trials_for(&self, trials: u64) -> u64 {
        let t = trials.clamp(1, MAX_TRIALS);
        if self.batched() {
            t.div_ceil(BATCH).saturating_mul(BATCH).min(MAX_TRIALS)
        } else {
            t
        }
    }

    pub fn sampling(&self) -> Sampling {
        self.sampling
    }

    /// Makes one batch: draws each input, induces the correlations, evaluates the outputs.
    pub fn block(&self, job: BlockJob) -> Block {
        let n = usize::try_from(job.trials).unwrap_or(0);
        let mut inputs: Vec<Vec<f64>> = self
            .inputs
            .iter()
            .map(|input| {
                let mut rng =
                    Rng::keyed(self.seed, &format!("mc/{}/input/{}", job.index, input.name));
                match self.sampling {
                    Sampling::Random => (0..n).map(|_| draw(&input.dist, &mut rng)).collect(),
                    Sampling::LatinHypercube => {
                        let mut strata: Vec<u64> = (0..job.trials).collect();
                        correlate::shuffle(&mut strata, &mut rng);
                        let ps: Vec<f64> = strata
                            .iter()
                            .map(|s| (*s as f64 + rng.uniform()) / job.trials as f64)
                            .collect();
                        quantile::quantiles(&input.dist, &ps)
                    }
                }
            })
            .collect();
        if self.correlated.len() >= 2 && !self.factor.is_empty() {
            let mut rng = Rng::keyed(self.seed, &format!("mc/{}/correlation", job.index));
            let mut columns: Vec<Vec<f64>> = self
                .correlated
                .iter()
                .map(|&i| inputs.get_mut(i).map(std::mem::take).unwrap_or_default())
                .collect();
            correlate::induce(&mut columns, &self.factor, &mut rng);
            for (&i, column) in self.correlated.iter().zip(columns) {
                if let Some(slot) = inputs.get_mut(i) {
                    *slot = column;
                }
            }
        }
        let k = self.inputs.len();
        let mut outputs = vec![Vec::with_capacity(n); self.outputs.len()];
        let mut values = vec![0.0; k + self.outputs.len()];
        for t in 0..n {
            for (slot, column) in values.iter_mut().zip(&inputs) {
                *slot = column.get(t).copied().unwrap_or(f64::NAN);
            }
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
                if let Some(column) = outputs.get_mut(o) {
                    column.push(v);
                }
            }
        }
        Block {
            index: job.index,
            inputs,
            outputs,
        }
    }

    /// Runs on this thread.
    pub fn run(&self, options: &RunOptions) -> McResults {
        let mut serial = |jobs: &[BlockJob]| -> Result<Vec<Block>, std::convert::Infallible> {
            Ok(jobs.iter().map(|j| self.block(*j)).collect())
        };
        match self.run_with(options, &mut serial) {
            Ok(r) => r,
            Err(never) => match never {},
        }
    }

    /// Runs, with `make` making each set of batches asked for (in parallel, say); it must
    /// return them in the order asked. This is [`Compiled::stepper`] with every batch it wants
    /// made at once.
    pub fn run_with<E>(
        &self,
        options: &RunOptions,
        make: &mut MakeBlocks<'_, E>,
    ) -> Result<McResults, E> {
        let mut stepper = self.stepper(options);
        loop {
            if let Some(results) = stepper.take_results() {
                return Ok(results);
            }
            if stepper.is_summarising() {
                stepper.summarise_step(self);
                continue;
            }
            let wanted = stepper.wanted().to_vec();
            let blocks = make(&wanted)?;
            stepper.deliver(self, blocks);
        }
    }

    /// A run made a few batches at a time: see [`Stepper`].
    pub fn stepper(&self, options: &RunOptions) -> Stepper {
        let (asked, max) = match options.stop {
            None => (self.trials_for(options.trials), None),
            Some(stop) => {
                let mut max = u64::from(stop.max).clamp(1, MAX_TRIALS);
                if self.batched() {
                    // Every batch full, so the batches are alike.
                    max = (max / BATCH).max(1) * BATCH;
                }
                let n = options
                    .trials
                    .max(1)
                    .div_ceil(BATCH)
                    .saturating_mul(BATCH)
                    .min(max);
                (n, Some(max))
            }
        };
        Stepper {
            options: options.clone(),
            max,
            asked,
            made: 0,
            blocks: Vec::new(),
            wanted: jobs(0, asked),
            next: 0,
            looks: vec![asked],
            summing: None,
            results: None,
            done: false,
        }
    }

    /// Whether a sequential run stops after `n` trials, or how many it goes on to (whole
    /// batches, at most `max`), judged on `blocks`.
    fn next_look(&self, blocks: &[Block], stop: McStop, n: u64, max: u64) -> Step {
        let verdicts: Vec<precision::Verdict> = (0..self.outputs.len())
            .map(|o| {
                // Judged on the interval the results report: across trials, or across batches
                // (whose count, not the trials', says whether there is one).
                let s = if self.batched() {
                    Summary::of(&batch_means(blocks, o))
                } else {
                    Summary::of(&finite(blocks, o))
                };
                precision::verdict(
                    s.map_or(0, |s| s.n),
                    s.map(|s| s.mean),
                    s.map(|s| s.half_width),
                    s.map(|s| s.std_dev),
                    Scale::Relative(stop.relative),
                )
            })
            .collect();
        match precision::next_count(n, max, &verdicts) {
            Step::Continue(next) => {
                Step::Continue(next.div_ceil(BATCH).saturating_mul(BATCH).min(max))
            }
            stop => stop,
        }
    }

    /// The results of `blocks` (in index order), every piece worked out without a pause, as
    /// [`Stepper::summarise_step`] does one at a time.
    pub fn summarise(&self, blocks: &[Block], percentiles: &[f64]) -> McResults {
        let mut summing = Summing::new(percentiles, None);
        while !summing.step(self, blocks) {}
        let mut results = summing.finish(self, blocks);
        results.requested_trials = results.trials;
        results
    }
}

/// The results of a run, worked out a piece at a time: each [`Summing::step`] summarises one
/// output (a copy of its values, sorted), or one input, or one correlation, so that a window can
/// draw between the pieces when a run has millions of trials. [`Compiled::summarise`] does the
/// same pieces without stopping, so the results are the same.
#[derive(Debug, Clone)]
struct Summing {
    /// The percentiles to report, ascending.
    ps: Vec<f64>,
    outputs: Vec<OutputResult>,
    /// Each input's values, in trial order, for the correlations.
    columns: Vec<Vec<f64>>,
    inputs: Vec<InputResult>,
    correlations: Vec<CorrelationResult>,
    /// How a sequential run went, for the results.
    stop: Option<StopReport>,
    /// The output being worked out: its finite values being sorted, and how many trials gave a
    /// value of any kind.
    sorting: Option<(Sorter, usize)>,
    /// Values sorted per chunk, and elements moved per merge step ([`Sorter`]).
    chunk: usize,
    budget: usize,
}

/// Values sorted in fixed-size chunks, one chunk per step, and then merged in bottom-up passes
/// of at most `budget` elements per step into a second buffer, which becomes the first at the
/// end of each pass. The order is `f64::total_cmp`'s, so the result is that of
/// `sort_by(f64::total_cmp)` (values that compare equal are the same bits), however the work is
/// cut into steps.
#[derive(Debug, Clone)]
struct Sorter {
    /// The values: sorted runs of `width` (the last may be shorter) once the chunks are done.
    a: Vec<f64>,
    /// The merge's output buffer.
    b: Vec<f64>,
    chunk: usize,
    budget: usize,
    /// Chunks sorted so far.
    chunks: usize,
    /// The length of the runs being merged; 0 while chunks are still being sorted.
    width: usize,
    /// The merge of the pair of runs from `lo`: next of the left run (`i`), of the right (`j`),
    /// and the next place in `b` (`k`).
    lo: usize,
    i: usize,
    j: usize,
    k: usize,
}

impl Sorter {
    fn new(values: Vec<f64>, chunk: usize, budget: usize) -> Self {
        Self {
            a: values,
            b: Vec::new(),
            chunk: chunk.max(1),
            budget: budget.max(1),
            chunks: 0,
            width: 0,
            lo: 0,
            i: 0,
            j: 0,
            k: 0,
        }
    }

    fn is_done(&self) -> bool {
        self.width != 0 && self.width >= self.a.len()
    }

    /// The sorted values; only once [`Sorter::is_done`].
    fn into_sorted(self) -> Vec<f64> {
        self.a
    }

    /// Does the next piece of work: sorts a chunk, or moves up to `budget` elements in a merge.
    fn step(&mut self) {
        let n = self.a.len();
        if self.width == 0 {
            let start = self.chunks * self.chunk;
            if start < n {
                let end = (start + self.chunk).min(n);
                if let Some(part) = self.a.get_mut(start..end) {
                    part.sort_by(f64::total_cmp);
                }
                self.chunks += 1;
            }
            if self.chunks * self.chunk >= n {
                self.width = self.chunk;
                self.b = vec![0.0; n];
                self.start_pair(0);
            }
            return;
        }
        let mut moved = 0;
        while moved < self.budget && self.width < n {
            let mid = (self.lo + self.width).min(n);
            let hi = (self.lo + 2 * self.width).min(n);
            if self.k < hi {
                let left = match (self.a.get(self.i), self.a.get(self.j)) {
                    (Some(l), Some(r)) if self.i < mid && self.j < hi => {
                        l.total_cmp(r) != std::cmp::Ordering::Greater
                    }
                    _ => self.i < mid,
                };
                let at = if left { self.i } else { self.j };
                if let (Some(v), Some(slot)) = (self.a.get(at).copied(), self.b.get_mut(self.k)) {
                    *slot = v;
                }
                if left {
                    self.i += 1;
                } else {
                    self.j += 1;
                }
                self.k += 1;
                moved += 1;
            } else if hi >= n {
                // A pass is done: its output is the input of the next, with runs twice as long.
                std::mem::swap(&mut self.a, &mut self.b);
                self.width = self.width.saturating_mul(2);
                self.start_pair(0);
            } else {
                self.start_pair(hi);
            }
        }
    }

    fn start_pair(&mut self, lo: usize) {
        self.lo = lo;
        self.i = lo;
        self.j = (lo + self.width).min(self.a.len());
        self.k = lo;
    }
}

impl Summing {
    fn new(percentiles: &[f64], stop: Option<StopReport>) -> Self {
        let mut ps: Vec<f64> = PERCENTILES
            .iter()
            .chain(percentiles)
            .copied()
            .filter(|p| (0.0..=100.0).contains(p))
            .collect();
        ps.sort_by(f64::total_cmp);
        ps.dedup();
        Self {
            ps,
            outputs: Vec::new(),
            columns: Vec::new(),
            inputs: Vec::new(),
            correlations: Vec::new(),
            stop,
            sorting: None,
            chunk: SORT_CHUNK,
            budget: MERGE_BUDGET,
        }
    }

    /// Works out the next output, input or correlation. True once there is none left.
    fn step(&mut self, c: &Compiled, blocks: &[Block]) -> bool {
        if let Some((out, _)) = c.outputs.get(self.outputs.len()) {
            let o = self.outputs.len();
            match self.sorting.take() {
                // First the values are copied; then they are sorted a piece at a time; then the
                // output is described from them.
                None => {
                    let total = blocks
                        .iter()
                        .filter_map(|b| b.outputs.get(o))
                        .map(Vec::len)
                        .sum();
                    let sorter = Sorter::new(finite(blocks, o), self.chunk, self.budget);
                    self.sorting = Some((sorter, total));
                }
                Some((mut sorter, total)) if !sorter.is_done() => {
                    sorter.step();
                    self.sorting = Some((sorter, total));
                }
                Some((sorter, total)) => {
                    let mut r = describe_output(out, &sorter.into_sorted(), total, &self.ps);
                    if c.batched() {
                        by_batches(&mut r, out, blocks, o);
                    }
                    self.outputs.push(r);
                }
            }
        } else if let Some(input) = c.inputs.get(self.inputs.len()) {
            let values = column(blocks, self.inputs.len(), true);
            let s = Summary::of(&values);
            self.inputs.push(InputResult {
                name: input.name.clone(),
                unit: input.unit.clone(),
                mean: s.map(|s| s.mean),
                std_dev: s.map(|s| s.std_dev),
                min: values.iter().copied().reduce(f64::min),
                max: values.iter().copied().reduce(f64::max),
                expected_mean: match &input.dist {
                    Dist::Constant(v) => Some(*v),
                    d => d.mean(),
                },
            });
            self.columns.push(values);
        } else if let Some(want) = c.correlations.get(self.correlations.len()) {
            let index = |n: &str| c.inputs.iter().position(|i| i.name == n);
            self.correlations.push(CorrelationResult {
                a: want.a.clone(),
                b: want.b.clone(),
                target: want.rank,
                achieved: index(want.a.as_str())
                    .zip(index(want.b.as_str()))
                    .and_then(|(a, b)| {
                        rank_correlation(self.columns.get(a)?, self.columns.get(b)?)
                    }),
            });
        }
        self.outputs.len() >= c.outputs.len()
            && self.inputs.len() >= c.inputs.len()
            && self.correlations.len() >= c.correlations.len()
    }

    /// The results, once every piece is in.
    fn finish(self, c: &Compiled, blocks: &[Block]) -> McResults {
        let trials: u64 = blocks
            .iter()
            .map(|b| {
                b.outputs
                    .first()
                    .or(b.inputs.first())
                    .map_or(0, |col| col.len() as u64)
            })
            .sum();
        let basis = if c.batched() {
            IntervalBasis::Batches {
                batches: blocks.len() as u64,
                size: BATCH,
            }
        } else {
            IntervalBasis::Trials
        };
        McResults {
            trials,
            requested_trials: trials,
            interval: basis,
            seed: c.seed,
            sampling: c.sampling,
            inputs: self.inputs,
            outputs: self.outputs,
            correlations: self.correlations,
            stop: self.stop,
        }
    }
}

/// Input (or output) `k`'s values across `blocks`, in trial order.
fn column(blocks: &[Block], k: usize, input: bool) -> Vec<f64> {
    blocks
        .iter()
        .filter_map(|b| {
            if input {
                b.inputs.get(k)
            } else {
                b.outputs.get(k)
            }
        })
        .flatten()
        .copied()
        .collect()
}

/// Output `o`'s finite values in each block.
fn per_block(blocks: &[Block], o: usize) -> Vec<Vec<f64>> {
    blocks
        .iter()
        .map(|b| {
            b.outputs
                .get(o)
                .map(|c| c.iter().copied().filter(|v| v.is_finite()).collect())
                .unwrap_or_default()
        })
        .collect()
}

/// Output `o`'s mean in each block (over its valid trials; a block with none is left out).
fn batch_means(blocks: &[Block], o: usize) -> Vec<f64> {
    per_block(blocks, o)
        .iter()
        .filter(|v| !v.is_empty())
        .map(|v| v.iter().sum::<f64>() / v.len() as f64)
        .collect()
}

/// The half-width of the 95% Student-t interval of the mean of `xs` (independent values), or
/// `None` with fewer than two.
fn t_half_width(xs: &[f64]) -> Option<f64> {
    let s = Summary::of(xs)?;
    let t = t_critical_95(s.n.checked_sub(1)?)?;
    Some(t * s.std_dev / (s.n as f64).sqrt()).filter(|h| h.is_finite())
}

/// Replaces an output's intervals by those across batches: the mean's from the batch means, the
/// target's from the batch proportions (clamped to [0, 1]); none with fewer than two batches,
/// and none for the target when every batch gave the same proportion (their spread says
/// nothing about how far the chance may be from it, and a width of 0 would claim it is exact).
fn by_batches(r: &mut OutputResult, out: &McOutput, blocks: &[Block], o: usize) {
    let each = per_block(blocks, o);
    let means: Vec<f64> = each
        .iter()
        .filter(|v| !v.is_empty())
        .map(|v| v.iter().sum::<f64>() / v.len() as f64)
        .collect();
    r.half_width = t_half_width(&means);
    r.interval_count = means.len() as u64;
    if let (Some(t), Some(result)) = (out.target, r.target.as_mut()) {
        let shares: Vec<f64> = each
            .iter()
            .filter(|v| !v.is_empty())
            .map(|v| v.iter().filter(|x| t.met(**x)).count() as f64 / v.len() as f64)
            .collect();
        result.interval = result
            .probability
            .zip(t_half_width(&shares).filter(|h| *h > 0.0))
            .map(|(p, h)| ((p - h).max(0.0), (p + h).min(1.0)));
    }
}

/// Output `o`'s finite values across `blocks`.
fn finite(blocks: &[Block], o: usize) -> Vec<f64> {
    blocks
        .iter()
        .filter_map(|b| b.outputs.get(o))
        .flatten()
        .copied()
        .filter(|v| v.is_finite())
        .collect()
}

/// A run's results.
#[derive(Debug, Clone, PartialEq)]
pub struct McResults {
    /// The trials run: whole batches where the intervals are across batches.
    pub trials: u64,
    /// The trials asked for (fewer than `trials` when rounded up to whole batches, or for a
    /// sequential run its first look).
    pub requested_trials: u64,
    /// What the intervals are across.
    pub interval: IntervalBasis,
    pub seed: u64,
    pub sampling: Sampling,
    pub inputs: Vec<InputResult>,
    pub outputs: Vec<OutputResult>,
    /// Each correlation asked for, with the rank correlation the trials have.
    pub correlations: Vec<CorrelationResult>,
    /// How a sequential run went; `None` for a fixed one.
    pub stop: Option<StopReport>,
}

/// What a run's intervals are across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalBasis {
    /// Independent trials (plain random sampling, no correlations): Student-t for the mean,
    /// Wilson's score interval for a target's probability.
    Trials,
    /// Independent batches of `size` trials (Latin hypercube sampling or correlations, which
    /// make the trials within a batch dependent): Student-t on the batch means and on the batch
    /// proportions, with `batches` − 1 degrees of freedom.
    Batches { batches: u64, size: u64 },
}

/// What an input's draws were, to check them against its distribution.
#[derive(Debug, Clone, PartialEq)]
pub struct InputResult {
    pub name: String,
    pub unit: String,
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// The distribution's own mean, where it has a closed form.
    pub expected_mean: Option<f64>,
}

/// An output's results.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputResult {
    pub name: String,
    pub unit: String,
    /// Trials whose value is a finite number.
    pub valid: u64,
    /// Trials whose value is not (left out of everything below).
    pub invalid: u64,
    pub mean: Option<f64>,
    /// Half-width of the 95% Student-t interval of the mean, across trials or across batches
    /// ([`IntervalBasis`]); `None` without two of them.
    pub half_width: Option<f64>,
    /// How many values the intervals are across: this output's valid trials, or the batches
    /// that have a valid trial of it (so the run's count less those left out).
    pub interval_count: u64,
    pub std_dev: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// (percentile from 0 to 100, value), ascending.
    pub percentiles: Vec<(f64, f64)>,
    pub target: Option<TargetResult>,
    pub histogram: Histogram,
    /// Points (x, share of trials at or below x) of the empirical cumulative distribution, at
    /// every 1% (its quantiles), for a chart.
    pub cdf: Vec<(f64, f64)>,
}

/// The chance of meeting an output's target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TargetResult {
    pub target: McTarget,
    pub met: u64,
    pub of: u64,
    pub probability: Option<f64>,
    /// The 95% interval: Wilson's score interval across trials, or Student-t on the batch
    /// proportions ([`IntervalBasis`]).
    pub interval: Option<(f64, f64)>,
}

/// Equal-width bins from the least value to the greatest.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Histogram {
    /// Bin edges, one more than the counts.
    pub edges: Vec<f64>,
    pub counts: Vec<u64>,
}

/// A correlation asked for and the one the trials have.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrelationResult {
    pub a: String,
    pub b: String,
    pub target: f64,
    pub achieved: Option<f64>,
}

/// How a sequential run went.
#[derive(Debug, Clone, PartialEq)]
pub struct StopReport {
    pub relative: f64,
    pub max: u64,
    pub reason: Reason,
    /// The trials after each look.
    pub looks: Vec<u64>,
}

/// A run made a few batches at a time ([`Compiled::stepper`]), so a window can draw between
/// them: it says which batches it wants ([`Stepper::wanted`]), takes them as they are made
/// ([`Stepper::deliver`]), and once every batch it asked for is in, it looks at the results:
/// a fixed run, or a sequential one that has stopped, has its results
/// ([`Stepper::take_results`]); a sequential one that goes on wants more. The looks fall at the
/// same numbers of trials however the batches are handed over, so the results are those of
/// [`Compiled::run_with`].
#[derive(Debug, Clone)]
pub struct Stepper {
    options: RunOptions,
    /// A sequential run's most trials; `None` for a fixed run.
    max: Option<u64>,
    /// The trials asked for so far.
    asked: u64,
    /// The trials made so far.
    made: u64,
    blocks: Vec<Block>,
    /// The batches asked for, the first `next` of them made.
    wanted: Vec<BlockJob>,
    next: usize,
    /// The trials at each look so far.
    looks: Vec<u64>,
    /// The results being worked out, a piece at a time, once the run has all its batches.
    summing: Option<Summing>,
    results: Option<McResults>,
    /// The run has its results (taken or not).
    done: bool,
}

impl Stepper {
    /// The batches wanted next, in order: those asked for and not yet made.
    pub fn wanted(&self) -> &[BlockJob] {
        self.wanted.get(self.next..).unwrap_or(&[])
    }

    /// The trials made so far.
    pub fn made(&self) -> u64 {
        self.made
    }

    /// The trials asked for so far: a sequential run asks for more after a look.
    pub fn asked(&self) -> u64 {
        self.asked
    }

    /// The run has its results (they may have been taken).
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Every batch is in and the results are being worked out, a piece at a time
    /// ([`Stepper::summarise_step`]), so that a window can draw between the pieces.
    pub fn is_summarising(&self) -> bool {
        self.summing.is_some()
    }

    /// Works out the next piece of the results (an output's, an input's or a correlation's):
    /// the same pieces, in the same order, as [`Compiled::summarise`]. Once the last is in,
    /// [`Stepper::is_done`] is true. Nothing to do unless [`Stepper::is_summarising`].
    pub fn summarise_step(&mut self, compiled: &Compiled) {
        let Some(summing) = self.summing.as_mut() else {
            return;
        };
        if !summing.step(compiled, &self.blocks) {
            return;
        }
        if let Some(summing) = self.summing.take() {
            let mut results = summing.finish(compiled, &self.blocks);
            results.requested_trials = self.options.trials;
            self.results = Some(results);
            self.done = true;
        }
    }

    /// The results, once the run has them (the first call takes them).
    pub fn take_results(&mut self) -> Option<McResults> {
        self.results.take()
    }

    /// Takes batches made by `compiled` from the front of [`Stepper::wanted`], in its order (one
    /// that is not the next wanted is left out). Once every batch asked for is in, looks at the
    /// results.
    pub fn deliver(&mut self, compiled: &Compiled, blocks: Vec<Block>) {
        for b in blocks {
            if let Some(job) = self.wanted.get(self.next)
                && job.index == b.index
            {
                self.made += job.trials;
                self.next += 1;
                self.blocks.push(b);
            }
        }
        if self.next >= self.wanted.len() && !self.done && self.summing.is_none() {
            self.look(compiled);
        }
    }

    /// Every batch asked for is in: more batches for a sequential run, or else the results begin
    /// to be worked out.
    fn look(&mut self, compiled: &Compiled) {
        let step = match (self.options.stop, self.max) {
            (Some(stop), Some(max)) => Some((
                stop,
                max,
                compiled.next_look(&self.blocks, stop, self.asked, max),
            )),
            _ => None,
        };
        if let Some((_, _, Step::Continue(next))) = step
            && next > self.asked
        {
            self.wanted = jobs(self.asked, next);
            self.next = 0;
            self.asked = next;
            self.looks.push(next);
            return;
        }
        let stop = step.map(|(stop, max, step)| StopReport {
            relative: stop.relative,
            max,
            reason: match step {
                Step::Stop(reason) => reason,
                // Not expected: no more trials to go on to.
                Step::Continue(_) => Reason::MaxReached,
            },
            looks: self.looks.clone(),
        });
        self.summing = Some(Summing::new(&self.options.percentiles, stop));
    }
}

/// An output's results from its finite values `sorted` (ascending) among `total` trials.
fn describe_output(out: &McOutput, sorted: &[f64], total: usize, ps: &[f64]) -> OutputResult {
    let valid = sorted.len() as u64;
    let s = Summary::of(sorted);
    let target = out.target.map(|t| {
        let met = sorted.iter().filter(|v| t.met(**v)).count() as u64;
        let prop = Proportion::new(met, valid);
        TargetResult {
            target: t,
            met,
            of: valid,
            probability: prop.estimate(),
            interval: prop.wilson(0.95),
        }
    });
    OutputResult {
        name: out.name.clone(),
        unit: out.unit.clone(),
        valid,
        invalid: (total as u64).saturating_sub(valid),
        mean: s.map(|s| s.mean),
        half_width: s.map(|s| s.half_width).filter(|h| h.is_finite()),
        interval_count: valid,
        std_dev: s.map(|s| s.std_dev),
        min: sorted.first().copied(),
        max: sorted.last().copied(),
        percentiles: ps
            .iter()
            .filter_map(|p| Some((*p, sorted_quantile(sorted, p / 100.0)?)))
            .collect(),
        target,
        histogram: histogram(sorted),
        cdf: (0..=100)
            .filter_map(|i| {
                let p = f64::from(i) / 100.0;
                Some((sorted_quantile(sorted, p)?, p))
            })
            .collect(),
    }
}

/// A histogram of `sorted` (ascending, finite) with as many bins as the larger of Sturges' rule
/// (log₂ n + 1) and the Freedman–Diaconis rule (bin width 2 IQR / n^(1/3)) give, as numpy's
/// `auto` estimator does (its documentation, `numpy.histogram_bin_edges`), at most 200.
pub fn histogram(sorted: &[f64]) -> Histogram {
    use stochcraft_random::math::{ln, powf};
    let (Some(&lo), Some(&hi)) = (sorted.first(), sorted.last()) else {
        return Histogram::default();
    };
    let n = sorted.len() as f64;
    if hi <= lo {
        return Histogram {
            edges: vec![lo, hi],
            counts: vec![sorted.len() as u64],
        };
    }
    let sturges = (ln(n) / std::f64::consts::LN_2).ceil() + 1.0;
    let iqr =
        sorted_quantile(sorted, 0.75).unwrap_or(hi) - sorted_quantile(sorted, 0.25).unwrap_or(lo);
    let fd = if iqr > 0.0 {
        ((hi - lo) / (2.0 * iqr / powf(n, 1.0 / 3.0))).ceil()
    } else {
        0.0
    };
    let bins = sturges.max(fd).clamp(1.0, 200.0) as usize;
    let width = (hi - lo) / bins as f64;
    let edges: Vec<f64> = (0..=bins)
        .map(|i| if i == bins { hi } else { lo + width * i as f64 })
        .collect();
    let mut counts = vec![0u64; bins];
    for v in sorted {
        let at = (((v - lo) / width) as usize).min(bins - 1);
        if let Some(c) = counts.get_mut(at) {
            *c += 1;
        }
    }
    Histogram { edges, counts }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, dist: Dist) -> McInput {
        McInput {
            name: name.into(),
            dist,
            unit: String::new(),
        }
    }

    fn output(name: &str, expression: &str) -> McOutput {
        McOutput {
            name: name.into(),
            expression: expression.into(),
            unit: String::new(),
            target: None,
        }
    }

    fn codes(mc: &MonteCarlo) -> Vec<&'static str> {
        check(mc).iter().map(|p| p.code).collect()
    }

    #[test]
    fn names_and_expressions_are_checked() {
        let mut mc = MonteCarlo {
            inputs: vec![
                input("a", Dist::Constant(1.0)),
                input("b", Dist::Uniform { min: 1.0, max: 0.0 }),
            ],
            outputs: vec![
                output("total", "$a + $later"),
                output("later", "$a * 2"),
                output("label", "concat('x', $a)"),
                output("x y", "1"),
                output("broken", "1 +"),
                output("ghost", "$nobody"),
            ],
            ..MonteCarlo::default()
        };
        let got = codes(&mc);
        for want in [
            "bad_distribution",
            "not_yet_computed",
            "text_output",
            "bad_name",
            "bad_expression",
            "unknown_name",
            "unused_input",
        ] {
            assert!(got.contains(&want), "{want} missing from {got:?}");
        }
        // Errors come first.
        let first_warning = check(&mc).iter().position(|p| !p.error).unwrap_or(0);
        assert!(check(&mc)[first_warning..].iter().all(|p| !p.error));
        mc.outputs = vec![output("a", "1")];
        assert!(codes(&mc).contains(&"duplicate_name"));
        mc.outputs.clear();
        assert!(codes(&mc).contains(&"no_outputs"));
        assert!(valid_name("cost_2") && valid_name("_x") && valid_name("été"));
        assert!(!valid_name("2x") && !valid_name("a-b") && !valid_name(""));
    }

    #[test]
    fn correlations_are_checked() {
        let three = |pairs: &[(&str, &str, f64)]| MonteCarlo {
            inputs: ["a", "b", "c"]
                .iter()
                .map(|n| input(n, Dist::Uniform { min: 0.0, max: 1.0 }))
                .collect(),
            outputs: vec![output("s", "$a + $b + $c")],
            correlations: pairs
                .iter()
                .map(|(a, b, r)| McCorrelation {
                    a: (*a).into(),
                    b: (*b).into(),
                    rank: *r,
                })
                .collect(),
            ..MonteCarlo::default()
        };
        assert!(check(&three(&[("a", "b", 0.5), ("b", "c", -0.3)])).is_empty());
        assert!(codes(&three(&[("a", "a", 0.5)])).contains(&"bad_correlation"));
        assert!(codes(&three(&[("a", "b", 1.0)])).contains(&"bad_correlation"));
        assert!(codes(&three(&[("a", "z", 0.5)])).contains(&"bad_correlation"));
        assert!(codes(&three(&[("a", "b", 0.5), ("b", "a", 0.4)])).contains(&"bad_correlation"));
        assert_eq!(
            codes(&three(&[
                ("a", "b", 0.9),
                ("b", "c", 0.9),
                ("a", "c", -0.9)
            ])),
            vec!["inconsistent_correlations"]
        );
    }

    #[test]
    fn a_batch_depends_only_on_the_seed_and_its_index() {
        let mc = MonteCarlo {
            inputs: vec![
                input(
                    "x",
                    Dist::Normal {
                        mean: 0.0,
                        std_dev: 1.0,
                    },
                ),
                input("y", Dist::Exponential { mean: 2.0 }),
            ],
            outputs: vec![output("s", "$x + $y")],
            correlations: vec![McCorrelation {
                a: "x".into(),
                b: "y".into(),
                rank: 0.6,
            }],
            sampling: Sampling::LatinHypercube,
            stop: None,
        };
        let c = compile(&mc, 7).unwrap_or_else(|p| unreachable!("{p:?}"));
        let job = BlockJob {
            index: 3,
            trials: 200,
        };
        assert_eq!(c.block(job), c.block(job));
        assert_ne!(c.block(job), c.block(BlockJob { index: 4, ..job }));
        // Adding an input leaves the others' draws as they were (without correlations, which
        // pair them anew).
        let plain = MonteCarlo {
            correlations: vec![],
            ..mc.clone()
        };
        let mut more = plain.clone();
        more.inputs.push(input("z", Dist::Constant(1.0)));
        let (a, b) = (
            compile(&plain, 7).map(|c| c.block(job)),
            compile(&more, 7).map(|c| c.block(job)),
        );
        assert_eq!(
            a.map(|b| b.inputs[0].clone()),
            b.map(|b| b.inputs[0].clone())
        );
    }

    #[test]
    fn negative_constants_and_invalid_trials() {
        let mc = MonteCarlo {
            inputs: vec![
                input("discount", Dist::Constant(-5.0)),
                input(
                    "u",
                    Dist::Uniform {
                        min: -1.0,
                        max: 1.0,
                    },
                ),
            ],
            outputs: vec![
                output("price", "100 + $discount"),
                output("ratio", "1 div ($u > 0)"),
            ],
            ..MonteCarlo::default()
        };
        let c = compile(&mc, 1).unwrap_or_else(|p| unreachable!("{p:?}"));
        let r = c.run(&RunOptions {
            trials: 2_000,
            stop: None,
            percentiles: vec![],
        });
        assert_eq!(r.outputs[0].mean, Some(95.0));
        assert_eq!(r.outputs[0].half_width, Some(0.0));
        // About half the trials divide by zero (infinite), and are counted apart.
        let ratio = &r.outputs[1];
        assert_eq!(ratio.valid + ratio.invalid, 2_000);
        assert!(ratio.invalid > 800 && ratio.invalid < 1_200, "{ratio:?}");
        assert_eq!(ratio.mean, Some(1.0));
    }

    #[test]
    fn histograms_cover_every_value() {
        let xs: Vec<f64> = (0..1000).map(|i| f64::from(i) / 10.0).collect();
        let h = histogram(&xs);
        assert_eq!(h.edges.len(), h.counts.len() + 1);
        assert_eq!(h.counts.iter().sum::<u64>(), 1000);
        assert_eq!(h.edges.first(), Some(&0.0));
        assert_eq!(h.edges.last(), Some(&99.9));
        assert_eq!(histogram(&[2.0, 2.0]).counts, vec![2]);
        assert_eq!(histogram(&[]), Histogram::default());
    }

    #[test]
    fn jobs_cut_trials_into_batches() {
        assert_eq!(
            jobs(0, 2_500),
            vec![
                BlockJob {
                    index: 0,
                    trials: 1_000
                },
                BlockJob {
                    index: 1,
                    trials: 1_000
                },
                BlockJob {
                    index: 2,
                    trials: 500
                },
            ]
        );
        assert_eq!(jobs(2_000, 3_000).first().map(|j| j.index), Some(2));
        assert!(jobs(5, 5).is_empty());
    }

    /// A run made one batch at a time, as the window makes it between frames, has the results
    /// of one made at once: fixed, and sequential with its looks at the same trials.
    #[test]
    fn a_run_made_a_batch_at_a_time_is_the_same_run() {
        let mc = MonteCarlo {
            inputs: vec![
                input(
                    "x",
                    Dist::Normal {
                        mean: 10.0,
                        std_dev: 3.0,
                    },
                ),
                input("y", Dist::Exponential { mean: 2.0 }),
            ],
            outputs: vec![output("s", "$x + $y")],
            correlations: vec![],
            sampling: Sampling::Random,
            stop: None,
        };
        let c = compile(&mc, 11).unwrap_or_else(|p| unreachable!("{p:?}"));
        for stop in [
            None,
            Some(McStop {
                relative: 0.002,
                max: 40_000,
            }),
        ] {
            let options = RunOptions {
                trials: 2_500,
                stop,
                percentiles: vec![],
            };
            let at_once = c.run(&options);
            let mut stepper = c.stepper(&options);
            let mut steps = 0;
            let mut pieces = 0;
            while !stepper.is_done() && steps < 1_000 {
                if stepper.is_summarising() {
                    // The results come one output, input or correlation at a time.
                    assert!(stepper.take_results().is_none());
                    stepper.summarise_step(&c);
                    pieces += 1;
                } else {
                    let next: Vec<BlockJob> = stepper.wanted().iter().take(1).copied().collect();
                    let blocks = next.iter().map(|j| c.block(*j)).collect();
                    stepper.deliver(&c, blocks);
                    assert!(stepper.made() <= stepper.asked());
                }
                steps += 1;
            }
            // Two inputs, and for the output a step to copy its values, one to sort them (all
            // in one chunk here) and one to describe them.
            assert_eq!(pieces, 5, "{stop:?}");
            assert_eq!(stepper.take_results(), Some(at_once), "{stop:?}");
            assert!(stepper.is_done());
        }
    }
    fn model(sampling: Sampling, stop: Option<McStop>) -> MonteCarlo {
        MonteCarlo {
            inputs: vec![input("x", Dist::Uniform { min: 0.0, max: 1.0 })],
            outputs: vec![output("y", "$x")],
            correlations: vec![],
            sampling,
            stop,
        }
    }

    /// Sequential runs of Latin hypercube sampling or correlated inputs are whole batches, so a
    /// most below one batch is refused, not raised; random sampling can stop after 500.
    #[test]
    fn a_most_below_one_batch_is_refused_where_runs_are_whole_batches() {
        let stop = Some(McStop {
            relative: 0.01,
            max: 500,
        });
        assert!(codes(&model(Sampling::LatinHypercube, stop)).contains(&"bad_stop"));
        assert!(compile(&model(Sampling::LatinHypercube, stop), 1).is_err());
        assert!(!codes(&model(Sampling::Random, stop)).contains(&"bad_stop"));
        let mut mc = model(Sampling::Random, stop);
        mc.inputs
            .push(input("z", Dist::Uniform { min: 0.0, max: 1.0 }));
        mc.correlations.push(McCorrelation {
            a: "x".into(),
            b: "z".into(),
            rank: 0.5,
        });
        assert!(codes(&mc).contains(&"bad_stop"));
        let stop = Some(McStop {
            relative: 0.01,
            max: 1_000,
        });
        assert!(!codes(&model(Sampling::LatinHypercube, stop)).contains(&"bad_stop"));
    }

    /// When every batch meets its target alike, their spread gives no interval (not a width of
    /// 0), and the interval of the mean still says how many batches it is across.
    #[test]
    fn identical_batch_shares_give_no_interval() {
        let mut mc = model(Sampling::LatinHypercube, None);
        if let Some(o) = mc.outputs.first_mut() {
            o.target = Some(McTarget::AtMost(2.0));
        }
        let c = compile(&mc, 3).unwrap_or_else(|p| unreachable!("{p:?}"));
        let r = c.run(&RunOptions {
            trials: 4_000,
            stop: None,
            percentiles: vec![],
        });
        let out = &r.outputs[0];
        let Some(t) = out.target else {
            unreachable!("the output has a target")
        };
        assert_eq!(t.probability, Some(1.0));
        assert_eq!(t.interval, None);
        assert_eq!(out.interval_count, 4);
        assert!(out.half_width.is_some());
    }

    /// An output's interval counts the values it is across: its valid trials, or the batches
    /// with one.
    #[test]
    fn an_interval_counts_what_it_is_across() {
        let mc = MonteCarlo {
            inputs: vec![input("x", Dist::Uniform { min: 0.0, max: 1.0 })],
            outputs: vec![output("y", "1 / ($x - $x)"), output("z", "$x")],
            ..MonteCarlo::default()
        };
        let c = compile(&mc, 5).unwrap_or_else(|p| unreachable!("{p:?}"));
        let r = c.run(&RunOptions {
            trials: 100,
            stop: None,
            percentiles: vec![],
        });
        assert_eq!(r.outputs[0].valid, 0);
        assert_eq!(r.outputs[0].interval_count, 0);
        assert_eq!(r.outputs[1].interval_count, 100);
    }

    /// Values in a spread of lengths, with the odd ones (infinities, both zeros, not a number),
    /// sorted a chunk and a merge budget at a time, come out as `sort_by(total_cmp)` has them.
    #[test]
    fn a_sort_made_in_pieces_is_the_whole_sort() {
        let mut seed = 12345_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 11) as f64 / (1_u64 << 53) as f64
        };
        for n in [0, 1, 2, 63, 64, 65, 127, 129, 1_000, 1_003] {
            let mut values: Vec<f64> = (0..n).map(|_| next() * 200.0 - 100.0).collect();
            if n > 8 {
                values[1] = f64::INFINITY;
                values[2] = f64::NEG_INFINITY;
                values[3] = 0.0;
                values[4] = -0.0;
                values[5] = f64::NAN;
                values[6] = 7.0;
                values[7] = 7.0;
            }
            let mut want = values.clone();
            want.sort_by(f64::total_cmp);
            for (chunk, budget) in [(64, 50), (1, 1), (7, 1_000), (100_000, 100_000)] {
                let mut sorter = Sorter::new(values.clone(), chunk, budget);
                let mut steps = 0;
                while !sorter.is_done() {
                    sorter.step();
                    steps += 1;
                    assert!(steps < 1_000_000, "n {n}, chunk {chunk}");
                }
                let got: Vec<u64> = sorter.into_sorted().iter().map(|v| v.to_bits()).collect();
                let want: Vec<u64> = want.iter().map(|v| v.to_bits()).collect();
                assert_eq!(got, want, "n {n}, chunk {chunk}, budget {budget}");
            }
        }
    }

    /// Results worked out with a small chunk and merge budget (so many pieces) are the results
    /// worked out with the usual sizes, whatever the columns hold.
    #[test]
    fn results_in_small_pieces_are_the_same_results() {
        let mc = MonteCarlo {
            inputs: vec![input("x", Dist::Uniform { min: 0.0, max: 1.0 })],
            outputs: vec![
                McOutput {
                    target: Some(McTarget::AtMost(2.0)),
                    ..output("y", "$x * 3")
                },
                output("gap", "1 / ($x - $x)"),
                output("some", "if($x > 0.5, $x, 1 / ($x - $x))"),
            ],
            correlations: vec![],
            sampling: Sampling::LatinHypercube,
            stop: None,
        };
        let c = compile(&mc, 9).unwrap_or_else(|p| unreachable!("{p:?}"));
        let blocks: Vec<Block> = jobs(0, 2_500).iter().map(|j| c.block(*j)).collect();
        let whole = c.summarise(&blocks, &[1.0, 99.0]);
        let mut small = Summing::new(&[1.0, 99.0], None);
        small.chunk = 100;
        small.budget = 37;
        let mut steps = 0;
        while !small.step(&c, &blocks) {
            steps += 1;
        }
        assert!(steps > 100, "{steps} pieces");
        let mut got = small.finish(&c, &blocks);
        got.requested_trials = got.trials;
        assert_eq!(got, whole);
        assert!(whole.outputs[2].invalid > 0 && whole.outputs[2].valid > 0);
        assert_eq!(whole.outputs[1].valid, 0);
    }
}
