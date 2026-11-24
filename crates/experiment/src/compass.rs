//! Searching whole-number decisions by COMPASS, then the clean-up of the finalists by KN.
//!
//! A design is a vector of whole numbers, one per decision, each in a range, all of them under
//! linear limits (a cap on the decisions' costs, say). Its objective is noisy and is observed
//! one replication at a time through the caller's sampler, as in [`crate::select`]. The search
//! is COMPASS, Convergent Optimization via Most-Promising-Area Stochastic Search (L. J. Hong and
//! B. L. Nelson, "Discrete optimization via simulation using COMPASS", *Operations Research*
//! 54(1), 2006), with the pruning and the local-optimality test of Industrial Strength COMPASS
//! (J. Xu, B. L. Nelson and L. J. Hong, *ACM TOMACS* 20(1), 2010), written from the papers'
//! published descriptions; no code of any implementation was used.
//!
//! **The most promising area.** At each step, the visited design with the best mean is the
//! current best x̂*. The most promising area is every design in the space at least as close to
//! x̂* as to any other visited design: for each visited y, the halfspace
//! 2 (y − x̂*)·x ≤ ‖y‖² − ‖x̂*‖², kept in whole numbers so that a design is in or out exactly.
//! New designs are sampled from it by a walk over the coordinates, as Hong and Nelson's
//! coordinate sampling does: from x̂*, a coordinate at random is set to a value drawn uniformly
//! from the whole numbers it may take with the others fixed, ten moves per decision.
//!
//! **Pruning.** A visited design whose halfspace does not change the area (it is implied by the
//! ranges, the limits and the other halfspaces) is not active. Each halfspace is tested by a
//! small linear programme, maximising its left side over the others (the area relaxed to real
//! numbers, which can only keep a design that whole numbers would let go). Only the current best,
//! the active designs and the new samples receive replications, as in Industrial Strength
//! COMPASS. On numerical doubt a design is kept.
//!
//! **Replications (the simulation-allocation rule).** At step k of a start, every active design
//! and every new sample is brought to at least n(1 + ⌊ln k⌋) replications, n the replications
//! per visit. Every design active infinitely often then gets infinitely many, the condition Hong
//! and Nelson's convergence proof asks of the rule; the slow growth keeps the cost down.
//!
//! **The local-optimality test.** When every neighbour of x̂* (one step along one decision,
//! within the space) has been visited, the area is x̂* alone. x̂* is then compared, as the
//! standard, with its neighbours by Kim's comparison with a standard
//! ([`crate::select::compare_with_standard`]), on the search's own replications (more are run as
//! the procedure asks). If it keeps the standard, x̂* is this start's local optimum. If not, the
//! search goes on, and the next look at the same design adds n replications to it and its
//! neighbours first. The test reuses the search's replications, so it is a stopping rule, not a
//! guarantee: the guarantee comes from the clean-up on fresh replications.
//!
//! **Starts.** In place of Industrial Strength COMPASS's niching genetic algorithm, the search
//! runs COMPASS from each design in `starts` (the starting design first), each with its own
//! visited set and random stream; they share the replications of designs they both visit
//! (replication i of a design is the same whichever start asks for it). The starts take one step
//! each in turn, so the path of each does not depend on the budget.
//!
//! **Held decisions.** A decision in [`Compass::held`] (a rule, whose levels have no order) is
//! never moved by the search: each start searches the other decisions with it held at the start's
//! value, so its walks, neighbours and most promising area stay inside that level, and the levels
//! meet only in the clean-up. The caller gives a start for each level it wants searched.
//!
//! **The clean-up.** The finalists are the search's best design, the designs active around it,
//! each start's final design and the starting design. They are compared again on a fresh block
//! of replications ([`Stream::CleanUp`]) by KN ([`crate::select::select_best`]), or with
//! constraints on other results by Batur and Kim's check followed by KN
//! ([`crate::select::select_feasible`]). KN's statement holds for the finalists whatever chose
//! them; it does not cover designs that were not finalists. When the space has at most
//! [`Compass::all_designs_up_to`] designs and the budget can pay for KN's first stage on all of
//! them, there is no search: every design goes to the clean-up.
//!
//! **Constraints during the search.** Designs whose constraint means meet every limit come first,
//! the best objective among them is x̂*; when none does, the one with the least violation (summed
//! in units of each constraint's tolerance). This is a heuristic for where to search; the
//! clean-up's feasibility check decides.
//!
//! **The budget.** [`Compass::search_runs`] caps the search; what it leaves of
//! [`Compass::max_runs`] goes to the clean-up. A step whose replications do not fit ends the
//! search; nothing is run past the cap. A caller's `stop` (a time limit) is asked after each step
//! that ran replications; a search it stops after S runs is repeated exactly by setting
//! `search_runs` to S with the same `max_runs` (see [`Answer::search_runs`]).
//!
//! **No randomness** ([`Compass::exact`]): every design is run once, the search compares values,
//! the local test is that x̂* is no worse than any neighbour, and the clean-up picks the best
//! finalist by value: "compared exactly", with no probability stated. This is the caller's
//! statement (the engine sets it only when nothing in the model is drawn); it is never inferred
//! from equal values. Without it, the local test and the clean-up run KN and the comparison with
//! a standard, where a pair of designs with the same difference in every first-stage replication
//! is left undecided and the answer is not proven ([`Answer::flat`]; [`crate::select`]'s notes).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use stochcraft_random::Rng;
use stochcraft_random::math::ln;

use crate::select::{
    Constraint, Decision, Kn, Outcome, Sampling, SelectError, Sense, compare_with_standard,
    select_best, select_feasible,
};

/// A linear limit on the decisions: `offset` + Σᵢ `coefficients[i]`·xᵢ ≤ `limit` (a cap on
/// costs: the costs at the ranges' least values, plus a cost per step of each decision).
#[derive(Debug, Clone, PartialEq)]
pub struct Linear {
    pub coefficients: Vec<f64>,
    pub offset: f64,
    pub limit: f64,
}

impl Linear {
    /// The left side at `x`, and the size of the numbers summed (for the rounding allowance).
    fn sum(&self, x: &[i64]) -> (f64, f64) {
        let mut sum = self.offset;
        let mut size = self.limit.abs() + self.offset.abs();
        for (c, &xi) in self.coefficients.iter().zip(x) {
            let t = c * xi as f64;
            sum += t;
            size += t.abs();
        }
        (sum, size)
    }
}

/// The rounding allowed when a sum of `size` is compared with a limit: 16 units in the last place
/// of the size, so a design whose decimal costs add up to the cap is inside it, and one above it
/// by more than rounding is not.
fn allowance(size: f64) -> f64 {
    16.0 * f64::EPSILON * size
}

/// The designs: whole numbers in each decision's range (`bounds`, least and greatest), under
/// every limit in `limits`.
#[derive(Debug, Clone, PartialEq)]
pub struct Space {
    pub bounds: Vec<(i64, i64)>,
    pub limits: Vec<Linear>,
}

/// The most box points [`Space::enumerate`] walks through to count the designs.
const MAX_ENUMERATED: u128 = 1_000_000;

impl Space {
    fn validate(&self) -> Result<(), CompassError> {
        if self.bounds.is_empty() {
            return Err(CompassError::Space("there are no decisions".into()));
        }
        for (i, (lo, hi)) in self.bounds.iter().enumerate() {
            if lo > hi {
                return Err(CompassError::Space(format!(
                    "decision {i}'s range is empty ({lo} to {hi})"
                )));
            }
        }
        for (i, l) in self.limits.iter().enumerate() {
            if l.coefficients.len() != self.bounds.len()
                || !l.limit.is_finite()
                || !l.offset.is_finite()
                || l.coefficients.iter().any(|c| !c.is_finite())
            {
                return Err(CompassError::Space(format!(
                    "limit {i} needs a finite limit and one finite coefficient per decision"
                )));
            }
        }
        Ok(())
    }

    /// Whether `x` meets the limit `l`, with the rounding [`allowance`] for the size of the
    /// numbers summed: the one rule for whether a design is in the space, used by the
    /// enumeration, the neighbours, the walks and the starts alike.
    fn fits(l: &Linear, x: &[i64]) -> bool {
        let (sum, size) = l.sum(x);
        sum <= l.limit + allowance(size)
    }

    /// The largest rounding allowance [`Space::fits`] gives `l` anywhere in the ranges, so that a
    /// relaxation with it covers every design the space admits.
    fn slack(&self, l: &Linear) -> f64 {
        let mut size = l.limit.abs() + l.offset.abs();
        for (c, (lo, hi)) in l.coefficients.iter().zip(&self.bounds) {
            size += c.abs() * (*lo as f64).abs().max((*hi as f64).abs());
        }
        allowance(size)
    }

    /// Whether `x` is a design of the space.
    pub fn contains(&self, x: &[i64]) -> bool {
        x.len() == self.bounds.len()
            && x.iter()
                .zip(&self.bounds)
                .all(|(v, (lo, hi))| lo <= v && v <= hi)
            && self.limits.iter().all(|l| Self::fits(l, x))
    }

    /// The designs one step from `x` along one decision, in the space: decision 0 down, then
    /// up, then decision 1, and so on.
    pub fn neighbours(&self, x: &[i64]) -> Vec<Vec<i64>> {
        let mut out = Vec::new();
        for i in 0..x.len() {
            for d in [-1i64, 1] {
                let mut y = x.to_vec();
                if let Some(v) = y.get_mut(i) {
                    match v.checked_add(d) {
                        Some(n) => *v = n,
                        None => continue,
                    }
                }
                if self.contains(&y) {
                    out.push(y);
                }
            }
        }
        out
    }

    /// Every design, in lexicographic order, if there are at most `up_to` of them and the ranges
    /// hold at most a million points; `None` otherwise.
    pub fn enumerate(&self, up_to: u64) -> Option<Vec<Vec<i64>>> {
        let mut points: u128 = 1;
        for (lo, hi) in &self.bounds {
            let n = u128::try_from(i128::from(*hi) - i128::from(*lo) + 1).ok()?;
            points = points.checked_mul(n)?;
            if points > MAX_ENUMERATED {
                return None;
            }
        }
        let mut out = Vec::new();
        let mut x: Vec<i64> = self.bounds.iter().map(|b| b.0).collect();
        loop {
            if self.contains(&x) {
                if out.len() as u64 >= up_to {
                    return None;
                }
                out.push(x.clone());
            }
            // The next point, the last decision changing fastest.
            let mut i = x.len();
            loop {
                if i == 0 {
                    return Some(out);
                }
                i -= 1;
                let (lo, hi) = self.bounds.get(i).copied().unwrap_or((0, 0));
                if let Some(v) = x.get_mut(i) {
                    if *v < hi {
                        *v += 1;
                        break;
                    }
                    *v = lo;
                }
            }
        }
    }
}

/// Which replications the sampler is asked for: the search's, or the clean-up's fresh block.
/// Replication j of every design in one stream meets the same random numbers (common random
/// numbers); the two streams must be independent of each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Search,
    CleanUp,
}

/// The settings of [`optimise`].
#[derive(Debug, Clone, PartialEq)]
pub struct Compass {
    /// The indifference amount δ, in the objective's unit (required).
    pub delta: f64,
    /// 1 − α, the clean-up's probability of a correct selection, covering the feasibility check
    /// too when there are constraints (default 0.95).
    pub confidence: f64,
    pub sense: Sense,
    /// Constraints on other results' means: the sampler's rows carry one value for each after
    /// the objective.
    pub constraints: Vec<Constraint>,
    /// With constraints, the share of α given to the feasibility check (default 0.5).
    pub feasibility_share: f64,
    /// n: replications a design gets at its first visit (default 10).
    pub per_visit: u64,
    /// New designs sampled from the most promising area at each step (default 5).
    pub samples_per_step: usize,
    /// n₀ of the local-optimality test and of the clean-up (default 10).
    pub first_stage: u64,
    /// The most replications in all, the search's and the clean-up's.
    pub max_runs: u64,
    /// The most replications the search may take; the rest of `max_runs` is the clean-up's.
    pub search_runs: u64,
    /// With at most this many designs (default 50), every design goes to the clean-up.
    pub all_designs_up_to: u64,
    /// The caller states that there is no randomness: each design is run once and compared by
    /// value. Never inferred from equal values.
    pub exact: bool,
    /// The caller states that the objective has no randomness (it is made of known costs alone),
    /// while the constrained results may vary (default `false`): the local-optimality test and
    /// the clean-up compare the objective exactly ([`Kn::deterministic`]) and check the
    /// constraints as random.
    pub exact_objective: bool,
    /// The seed of the walks that sample the most promising area.
    pub seed: u64,
    /// Decisions the search never moves (rules): each start keeps them at its own values.
    pub held: Vec<usize>,
}

impl Compass {
    /// The defaults, with `reserve` (default 30%) of `max_runs` kept for the clean-up.
    pub fn new(delta: f64, sense: Sense, max_runs: u64) -> Self {
        let mut c = Self {
            delta,
            confidence: 0.95,
            sense,
            constraints: Vec::new(),
            feasibility_share: 0.5,
            per_visit: 10,
            samples_per_step: 5,
            first_stage: 10,
            max_runs,
            search_runs: max_runs,
            all_designs_up_to: 50,
            exact: false,
            exact_objective: false,
            seed: 0,
            held: Vec::new(),
        };
        c.reserve(0.3);
        c
    }

    /// Keeps the share `reserve` of `max_runs` (rounded up) for the clean-up.
    pub fn reserve(&mut self, reserve: f64) {
        let kept = (reserve.clamp(0.0, 1.0) * self.max_runs as f64).ceil();
        let kept = if kept >= self.max_runs as f64 {
            self.max_runs
        } else {
            kept as u64
        };
        self.search_runs = self.max_runs - kept;
    }

    pub(crate) fn validate(&self) -> Result<(), CompassError> {
        let bad = |name: &'static str| Err(CompassError::Setting(name));
        if !(self.delta.is_finite() && self.delta > 0.0) {
            return bad("delta");
        }
        if !(self.confidence > 0.5 && self.confidence < 1.0) {
            return bad("confidence");
        }
        if !(self.feasibility_share > 0.0 && self.feasibility_share < 1.0) {
            return bad("feasibility_share");
        }
        if self.per_visit == 0 {
            return bad("per_visit");
        }
        if self.samples_per_step == 0 {
            return bad("samples_per_step");
        }
        if self.first_stage < 2 {
            return bad("first_stage");
        }
        if self.search_runs > self.max_runs {
            return bad("search_runs");
        }
        for c in &self.constraints {
            if !(c.tolerance.is_finite() && c.tolerance > 0.0 && c.limit.is_finite()) {
                return bad("constraints");
            }
        }
        Ok(())
    }

    /// The replications every active design has at step k: n(1 + ⌊ln k⌋), or 1 without
    /// randomness.
    pub(crate) fn level(&self, k: u64) -> u64 {
        if self.exact {
            return 1;
        }
        let growth = ln(k.max(1) as f64).floor();
        let factor = if growth.is_finite() && growth >= 0.0 {
            1 + growth as u64
        } else {
            1
        };
        self.per_visit.saturating_mul(factor)
    }
}

/// Why the search stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// The space was small: no search, every design went to the clean-up.
    AllDesigns,
    /// Every start reached a design its local test kept.
    LocalOptima,
    /// The search's budget could not pay for the next step.
    Budget,
    /// The caller's `stop` asked it to.
    Stopped,
}

/// A design as the procedures here handle it: whole numbers for COMPASS, a point of a
/// continuous range for Nelder–Mead ([`crate::simplex`]). `show` names it in messages.
pub trait Design: Clone {
    fn show(&self) -> String;
}

impl Design for Vec<i64> {
    fn show(&self) -> String {
        self.iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// What became of one start.
#[derive(Debug, Clone, PartialEq)]
pub struct StartReport<D = Vec<i64>> {
    pub start: D,
    /// The best design of this start when the search ended (`None` if it never ran).
    pub end: Option<D>,
    /// Its local test kept it.
    pub local_optimum: bool,
    /// Steps taken.
    pub steps: u64,
    /// Designs this start visited.
    pub visited: usize,
}

/// The search's state after a step that ran replications.
#[derive(Debug, Clone, PartialEq)]
pub struct TracePoint {
    /// The search's replications so far.
    pub runs: u64,
    /// The best mean of the objective among the designs visited so far (that look feasible,
    /// when any does).
    pub best: Option<f64>,
}

/// A design compared at the end.
#[derive(Debug, Clone, PartialEq)]
pub struct Finalist<D = Vec<i64>> {
    pub design: D,
    /// The objective's mean over the search's replications (`None` if the search did not run it).
    pub search_mean: Option<f64>,
    pub search_replications: u64,
    /// The clean-up's rows (objective, then each constraint), by replication index.
    pub cleanup: BTreeMap<u64, Vec<f64>>,
}

impl<D> Finalist<D> {
    /// The clean-up's objective values, in replication order.
    pub fn objective(&self) -> Vec<f64> {
        self.cleanup
            .values()
            .filter_map(|r| r.first().copied())
            .collect()
    }
}

/// The answer of [`optimise`].
#[derive(Debug, Clone, PartialEq)]
pub struct Answer<D = Vec<i64>> {
    /// [`Outcome::Selected`] with the confidence; [`Outcome::ComparedExactly`] in either exact
    /// mode: [`Compass::exact`] (the caller states there is no randomness at all) or
    /// [`Compass::exact_objective`] (the objective alone is known exactly, as with costs only,
    /// while the constraints are still checked as random, with the check's confidence). An exact
    /// comparison therefore does not mean the whole model is deterministic. Otherwise
    /// [`Outcome::NotProven`]: the clean-up's budget could not settle it, a flat first stage left
    /// it undecided ([`Answer::flat`], [`Answer::flat_constraint`]), or no finalist was found
    /// feasible.
    pub outcome: Outcome,
    /// The chosen finalist (when not proven, the best-looking one), by position in `finalists`;
    /// `None` when there is none, or none was declared feasible.
    pub chosen: Option<usize>,
    /// The confidence the answer claims, covering the feasibility check with constraints.
    pub confidence: Option<f64>,
    /// Every design of the space was a finalist.
    pub all_designs: bool,
    pub finalists: Vec<Finalist<D>>,
    /// When not proven: the finalists still in contention.
    pub in_contention: Vec<usize>,
    /// With constraints: each finalist's feasibility decision and its decision on each
    /// constraint (`None` when the check did not run).
    pub feasibility: Option<Vec<Decision>>,
    pub by_constraint: Option<Vec<Vec<Decision>>>,
    /// Designs the search ran.
    pub tried: usize,
    /// The search's replications. A search stopped by the caller is repeated exactly by
    /// setting [`Compass::search_runs`] to this, with the same `max_runs`.
    pub search_runs: u64,
    pub cleanup_runs: u64,
    pub ended: Ended,
    pub starts: Vec<StartReport<D>>,
    pub trace: Vec<TracePoint>,
    /// The number of designs in the space, when it was counted (at most
    /// [`Compass::all_designs_up_to`]).
    pub space: Option<u64>,
    /// The clean-up was not proven because two finalists gave the same objective difference in
    /// every replication of KN's first stage, without a statement that the objective is exact:
    /// that is not proof of no randomness, so KN could not tell them apart
    /// ([`crate::select::Selection::flat`]).
    pub flat: bool,
    /// The clean-up was not proven because the feasibility check left a finalist undecided whose
    /// constrained value was the same in every replication of the check's first stage, which is
    /// not proof of no randomness ([`crate::select::FeasibilityCheck::flat`]). Such a finalist
    /// is in [`Answer::in_contention`].
    pub flat_constraint: bool,
    /// With [`Compass::exact_objective`]: finalists the feasibility check left undecided that do
    /// not affect the answer, since their exact objective is no better than the chosen one's
    /// ([`crate::select::ConstrainedSelection::set_aside`]).
    pub set_aside: Vec<usize>,
}

impl<D> Answer<D> {
    pub fn total_runs(&self) -> u64 {
        self.search_runs.saturating_add(self.cleanup_runs)
    }

    /// The same answer with each design mapped by `f` (whole numbers to the caller's designs).
    pub fn map<E>(self, f: impl Fn(D) -> E) -> Answer<E> {
        Answer {
            outcome: self.outcome,
            chosen: self.chosen,
            confidence: self.confidence,
            all_designs: self.all_designs,
            finalists: self
                .finalists
                .into_iter()
                .map(|x| Finalist {
                    design: f(x.design),
                    search_mean: x.search_mean,
                    search_replications: x.search_replications,
                    cleanup: x.cleanup,
                })
                .collect(),
            in_contention: self.in_contention,
            feasibility: self.feasibility,
            by_constraint: self.by_constraint,
            tried: self.tried,
            search_runs: self.search_runs,
            cleanup_runs: self.cleanup_runs,
            ended: self.ended,
            starts: self
                .starts
                .into_iter()
                .map(|r| StartReport {
                    start: f(r.start),
                    end: r.end.map(&f),
                    local_optimum: r.local_optimum,
                    steps: r.steps,
                    visited: r.visited,
                })
                .collect(),
            trace: self.trace,
            space: self.space,
            flat: self.flat,
            flat_constraint: self.flat_constraint,
            set_aside: self.set_aside,
        }
    }
}

/// Why the search could not go on.
#[derive(Debug, Clone, PartialEq)]
pub enum CompassError {
    /// A setting is out of range (its field's name).
    Setting(&'static str),
    Space(String),
    /// A start is not a design of the space (its position in `starts`).
    Start(usize),
    NoStart,
    /// The sampler failed, with its message.
    Sampler(String),
    /// The design is named as [`Design::show`] gives it.
    WrongCount {
        design: String,
        expected: u64,
        got: u64,
    },
    NotFinite {
        design: String,
        replication: u64,
    },
    Select(SelectError),
}

impl std::fmt::Display for CompassError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompassError::Setting(name) => write!(f, "the search's setting {name} is out of range"),
            CompassError::Space(m) => write!(f, "{m}"),
            CompassError::Start(i) => write!(f, "start {i} is not a design of the space"),
            CompassError::NoStart => write!(f, "the search needs a starting design"),
            CompassError::Sampler(m) => write!(f, "{m}"),
            CompassError::WrongCount {
                design,
                expected,
                got,
            } => write!(
                f,
                "design ({design}) gave {got} rows where {expected} were asked for, or a row of the wrong length"
            ),
            CompassError::NotFinite {
                design,
                replication,
            } => write!(
                f,
                "design ({design}) gave a value that is not a number in replication {replication}"
            ),
            CompassError::Select(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CompassError {}

/// The halfspace of visited design y around the current best b: 2 (y − b)·x ≤ ‖y‖² − ‖b‖².
#[derive(Debug, Clone, PartialEq)]
struct Half {
    a: Vec<i128>,
    rhs: i128,
}

impl Half {
    fn new(y: &[i64], b: &[i64]) -> Self {
        let a: Vec<i128> = y
            .iter()
            .zip(b)
            .map(|(&p, &q)| i128::from(p) - i128::from(q))
            .collect();
        let norm = |x: &[i64]| -> i128 { x.iter().map(|&v| i128::from(v) * i128::from(v)).sum() };
        Self {
            a,
            rhs: norm(y) - norm(b),
        }
    }

    #[cfg(test)]
    fn holds(&self, x: &[i64]) -> bool {
        let lhs: i128 = self
            .a
            .iter()
            .zip(x)
            .map(|(a, &v)| 2 * a * i128::from(v))
            .sum();
        lhs <= self.rhs
    }
}

/// A row g·w ≤ β of a linear programme.
type Row = (Vec<f64>, f64);

/// ⌊p / q⌋ for q > 0.
fn floor_div(p: i128, q: i128) -> i128 {
    p.div_euclid(q)
}

/// ⌈p / q⌉ for q > 0.
fn ceil_div(p: i128, q: i128) -> i128 {
    -(-p).div_euclid(q)
}

fn clamp_i64(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// The whole numbers coordinate `i` of `x` may take, the others fixed, in the space and every
/// halfspace. Always holds `x[i]` when `x` is in the area.
fn coordinate_range(space: &Space, halves: &[Half], x: &[i64], i: usize) -> (i64, i64) {
    let (mut lo, mut hi) = space.bounds.get(i).copied().unwrap_or((0, 0));
    let xi = x.get(i).copied().unwrap_or(0);
    for h in halves {
        let ai = h.a.get(i).copied().unwrap_or(0);
        if ai == 0 {
            continue;
        }
        let rest: i128 =
            h.a.iter()
                .zip(x)
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, (a, &v))| 2 * a * i128::from(v))
                .sum();
        let r = h.rhs - rest;
        if ai > 0 {
            hi = hi.min(clamp_i64(floor_div(r, 2 * ai)));
        } else {
            lo = lo.max(clamp_i64(ceil_div(-r, -2 * ai)));
        }
    }
    for l in &space.limits {
        let ci = l.coefficients.get(i).copied().unwrap_or(0.0);
        if ci == 0.0 {
            continue;
        }
        let mut y = x.to_vec();
        let mut fits = |t: i64| {
            if let Some(v) = y.get_mut(i) {
                *v = t;
            }
            Space::fits(l, &y)
        };
        // The left side moves one way with t, so whether t fits changes once: a bisection over
        // the range finds the edge in about 64 steps, however wide the range.
        if ci > 0.0 {
            // The largest t in [x[i], hi] that fits; x[i] does.
            let (mut a, mut b) = (i128::from(xi), i128::from(hi.max(xi)));
            while a < b {
                let m = a + (b - a + 1) / 2;
                if fits(clamp_i64(m)) {
                    a = m;
                } else {
                    b = m - 1;
                }
            }
            hi = hi.min(clamp_i64(a));
        } else {
            // The smallest t in [lo, x[i]] that fits.
            let (mut a, mut b) = (i128::from(lo.min(xi)), i128::from(xi));
            while a < b {
                let m = a + (b - a) / 2;
                if fits(clamp_i64(m)) {
                    b = m;
                } else {
                    a = m + 1;
                }
            }
            lo = lo.max(clamp_i64(b));
        }
    }
    (lo.min(xi), hi.max(xi))
}

/// The largest c·w over {w : gᵣ·w ≤ βᵣ for every row}, w real and free, every βᵣ ≥ 0 (so w = 0
/// is feasible): the simplex method on w = p − q, p, q ≥ 0, from the slack basis, with Bland's
/// rule (the lowest index enters, ties in the ratio test go to the lowest basic index), which
/// cannot cycle. `None` if the programme is unbounded or does not finish in the pivots allowed.
fn lp_max(c: &[f64], rows: &[Row]) -> Option<f64> {
    let d = c.len();
    let m = rows.len();
    let n = 2 * d;
    let cols = n + m;
    let mut t = vec![vec![0.0; cols]; m];
    let mut rhs = vec![0.0; m];
    let mut basis: Vec<usize> = (n..n + m).collect();
    for (r, (g, beta)) in rows.iter().enumerate() {
        for (j, &gj) in g.iter().enumerate().take(d) {
            t[r][j] = gj;
            t[r][d + j] = -gj;
        }
        t[r][n + r] = 1.0;
        rhs[r] = beta.max(0.0);
    }
    let mut rc = vec![0.0; cols];
    for (j, &cj) in c.iter().enumerate() {
        rc[j] = cj;
        rc[d + j] = -cj;
    }
    let mut value = 0.0;
    let eps = 1e-12;
    for _ in 0..(50 * (cols + 1)) {
        let Some(e) = (0..cols).find(|&j| rc[j] > eps) else {
            return Some(value);
        };
        let mut leave: Option<usize> = None;
        let mut best = f64::INFINITY;
        for (r, row) in t.iter().enumerate() {
            if row[e] > eps {
                let ratio = rhs[r] / row[e];
                let better = match leave {
                    None => true,
                    Some(l) => ratio < best || (ratio == best && basis[r] < basis[l]),
                };
                if better {
                    best = ratio;
                    leave = Some(r);
                }
            }
        }
        let r = leave?;
        let p = t[r][e];
        for v in &mut t[r] {
            *v /= p;
        }
        rhs[r] /= p;
        let pivot_row = t[r].clone();
        let pivot_rhs = rhs[r];
        for (q, row) in t.iter_mut().enumerate() {
            if q != r {
                let f = row[e];
                if f != 0.0 {
                    for (v, pv) in row.iter_mut().zip(&pivot_row) {
                        *v -= f * pv;
                    }
                    rhs[q] -= f * pivot_rhs;
                    if rhs[q] < 0.0 {
                        rhs[q] = 0.0;
                    }
                }
            }
        }
        let f = rc[e];
        for (v, pv) in rc.iter_mut().zip(&pivot_row) {
            *v -= f * pv;
        }
        value += f * pivot_rhs;
        basis[r] = e;
    }
    None
}

/// A row g·w ≤ β of the area around `b` (w = x − b), scaled by its largest coefficient.
fn scaled(g: Vec<f64>, beta: f64) -> Row {
    let s = g.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    if s > 0.0 && s.is_finite() {
        (g.iter().map(|v| v / s).collect(), beta / s)
    } else {
        (g, beta)
    }
}

/// The rows of the ranges and limits around `b`.
fn base_rows(space: &Space, b: &[i64]) -> Vec<Row> {
    let d = b.len();
    let mut rows = Vec::new();
    for (i, ((lo, hi), &bi)) in space.bounds.iter().zip(b).enumerate() {
        let mut up = vec![0.0; d];
        up[i] = 1.0;
        rows.push((up, (hi - bi) as f64));
        let mut down = vec![0.0; d];
        down[i] = -1.0;
        rows.push((down, (bi - lo) as f64));
    }
    for l in &space.limits {
        let at: f64 = l
            .coefficients
            .iter()
            .zip(b)
            .map(|(c, &v)| c * v as f64)
            .sum();
        // The allowance the space gives the limit is added, so the relaxation covers every
        // design `Space::fits` admits: a halfspace is dropped only when it is implied there.
        rows.push(scaled(
            l.coefficients.clone(),
            l.limit - l.offset - at + space.slack(l),
        ));
    }
    rows
}

/// The row of halfspace `h` around `b`: a·w ≤ rhs/2 − a·b.
fn half_row(h: &Half, b: &[i64]) -> Row {
    let at: i128 = h.a.iter().zip(b).map(|(a, &v)| a * i128::from(v)).sum();
    let beta = h.rhs as f64 / 2.0 - at as f64;
    scaled(h.a.iter().map(|&a| a as f64).collect(), beta)
}

/// Whether row `test` is implied by `rows`: the largest value of its left side over them is at
/// most its bound. Kept (not redundant) when the programme does not answer.
fn redundant(test: &Row, rows: &[Row]) -> bool {
    lp_max(&test.0, rows).is_some_and(|max| max <= test.1 + 1e-9 * (1.0 + test.1.abs()))
}

/// The visited designs whose halfspaces around `b` define the most promising area, with their
/// halfspaces: every other one is implied by the ranges, the limits and these.
fn active(space: &Space, b: &[i64], visited: &BTreeSet<Vec<i64>>) -> Vec<(Vec<i64>, Half)> {
    let dist = |y: &[i64]| -> i128 {
        y.iter()
            .zip(b)
            .map(|(&p, &q)| {
                let d = i128::from(p) - i128::from(q);
                d * d
            })
            .sum()
    };
    let mut others: Vec<&Vec<i64>> = visited.iter().filter(|y| y.as_slice() != b).collect();
    others.sort_by(|p, q| dist(p).cmp(&dist(q)).then(p.cmp(q)));
    let base = base_rows(space, b);
    // The box alone: the largest a·w over the ranges, in closed form.
    let over_box = |row: &Row| -> bool {
        let max: f64 = row
            .0
            .iter()
            .zip(space.bounds.iter().zip(b))
            .map(|(a, ((lo, hi), &bi))| (a * (hi - bi) as f64).max(a * (lo - bi) as f64))
            .sum();
        max <= row.1 + 1e-9 * (1.0 + row.1.abs())
    };
    let mut kept: Vec<(Vec<i64>, Half, Row)> = Vec::new();
    for y in others {
        let h = Half::new(y, b);
        let row = half_row(&h, b);
        if over_box(&row) {
            continue;
        }
        let mut rows = base.clone();
        rows.extend(kept.iter().map(|k| k.2.clone()));
        if !redundant(&row, &rows) {
            kept.push((y.clone(), h, row));
        }
    }
    // A halfspace kept early may be implied by later ones.
    let mut i = 0;
    while i < kept.len() {
        let mut rows = base.clone();
        rows.extend(
            kept.iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, k)| k.2.clone()),
        );
        if kept.get(i).is_some_and(|k| redundant(&k.2, &rows)) {
            kept.remove(i);
        } else {
            i += 1;
        }
    }
    kept.into_iter().map(|(y, h, _)| (y, h)).collect()
}

/// One design of the most promising area: a walk of ten coordinate moves per decision from `b`.
fn walk(space: &Space, halves: &[Half], b: &[i64], rng: &mut Rng) -> Vec<i64> {
    let d = b.len();
    let mut x = b.to_vec();
    for _ in 0..10 * d {
        let i = ((rng.uniform() * d as f64) as usize).min(d - 1);
        let (lo, hi) = coordinate_range(space, halves, &x, i);
        let width = (i128::from(hi) - i128::from(lo) + 1) as f64;
        let k = ((rng.uniform() * width) as i128).min(i128::from(hi) - i128::from(lo));
        if let Some(v) = x.get_mut(i) {
            *v = clamp_i64(i128::from(lo) + k);
        }
    }
    x
}

/// The search's replications, shared by every start: each design's rows for replications 0 to
/// n − 1 of the search stream.
struct Runner<'a, F> {
    setup: &'a Compass,
    sample: F,
    width: usize,
    cache: BTreeMap<Vec<i64>, Vec<Vec<f64>>>,
    runs: u64,
    budget: u64,
    /// A request the budget could not pay for, inside a procedure that cannot be told so.
    exhausted: bool,
    error: Option<CompassError>,
}

/// What one step of a start did.
enum StepEnd {
    /// It ran this many replications.
    Ran(u64),
    /// The start's local test kept its best design, after running this many replications.
    LocalOptimum(u64),
    Exhausted,
}

impl<F> Runner<'_, F>
where
    F: FnMut(Stream, &[i64], Range<u64>) -> Result<Vec<Vec<f64>>, String>,
{
    fn have(&self, x: &[i64]) -> u64 {
        self.cache.get(x).map_or(0, |r| r.len() as u64)
    }

    fn left(&self) -> u64 {
        self.budget.saturating_sub(self.runs)
    }

    /// Checked rows from the sampler.
    fn fetch(
        &mut self,
        stream: Stream,
        x: &[i64],
        reps: Range<u64>,
    ) -> Result<Vec<Vec<f64>>, CompassError> {
        let expected = reps.end - reps.start;
        let rows = (self.sample)(stream, x, reps.clone()).map_err(CompassError::Sampler)?;
        if rows.len() as u64 != expected || rows.iter().any(|r| r.len() != self.width) {
            return Err(CompassError::WrongCount {
                design: x.to_vec().show(),
                expected,
                got: rows.len() as u64,
            });
        }
        if let Some(j) = rows.iter().position(|r| r.iter().any(|v| !v.is_finite())) {
            return Err(CompassError::NotFinite {
                design: x.to_vec().show(),
                replication: reps.start + j as u64,
            });
        }
        Ok(rows)
    }

    /// Brings `x` to `n` search replications.
    fn top_up(&mut self, x: &[i64], n: u64) -> Result<(), CompassError> {
        let have = self.have(x);
        if have >= n {
            return Ok(());
        }
        let rows = self.fetch(Stream::Search, x, have..n)?;
        self.runs += n - have;
        self.cache.entry(x.to_vec()).or_default().extend(rows);
        Ok(())
    }

    /// The new replications a plan of (design, level) needs.
    fn need(&self, plan: &[(Vec<i64>, u64)]) -> u64 {
        let mut seen = BTreeMap::new();
        for (x, n) in plan {
            let e = seen.entry(x.clone()).or_insert(0);
            *e = (*e).max(*n);
        }
        seen.iter()
            .map(|(x, n)| n.saturating_sub(self.have(x)))
            .fold(0u64, u64::saturating_add)
    }

    /// Carries out a plan if it fits in what is left of the budget: `Ok(None)` if it does not.
    fn run_plan(&mut self, plan: &[(Vec<i64>, u64)]) -> Result<Option<u64>, CompassError> {
        let need = self.need(plan);
        if need > self.left() {
            return Ok(None);
        }
        for (x, n) in plan {
            self.top_up(x, *n)?;
        }
        Ok(Some(need))
    }

    /// The objective for replications `reps` of `x` from the search stream, run as needed. Inside
    /// a procedure: a request the budget cannot pay for, or a failure, is recorded and answered
    /// with no values, which the procedure refuses.
    fn objective(&mut self, x: &[i64], reps: Range<u64>) -> Vec<f64> {
        if reps.end > self.have(x) {
            let more = reps.end - self.have(x);
            if more > self.left() {
                self.exhausted = true;
                return Vec::new();
            }
            if let Err(e) = self.top_up(x, reps.end) {
                self.error.get_or_insert(e);
                return Vec::new();
            }
        }
        self.cache.get(x).map_or_else(Vec::new, |rows| {
            rows.iter()
                .skip(reps.start as usize)
                .take((reps.end - reps.start) as usize)
                .filter_map(|r| r.first().copied())
                .collect()
        })
    }

    /// Each value's mean over the search's replications of `x`.
    fn means(&self, x: &[i64]) -> Option<Vec<f64>> {
        let rows = self.cache.get(x)?;
        if rows.is_empty() {
            return None;
        }
        let mut m = vec![0.0; self.width];
        for (k, row) in rows.iter().enumerate() {
            let kf = (k + 1) as f64;
            for (mi, &v) in m.iter_mut().zip(row) {
                // A running mean, so that large values do not overflow a sum.
                *mi += v / kf - *mi / kf;
            }
        }
        Some(m)
    }

    /// How `x` stands: whether its constraint means meet every limit, and a score where larger
    /// is better (the objective's mean in the better direction, or minus the violation).
    fn standing(&self, x: &[i64]) -> Option<(bool, f64)> {
        standing(self.setup, &self.means(x)?)
    }

    /// Whether `a` stands better than `b` (looks feasible first, then score, then the
    /// lexicographically smaller design).
    fn better(&self, a: &[i64], b: &[i64]) -> bool {
        match (self.standing(a), self.standing(b)) {
            (Some(_), None) => true,
            (None, _) => false,
            (Some((fa, sa)), Some((fb, sb))) => {
                (fa, sa).partial_cmp(&(fb, sb)) == Some(std::cmp::Ordering::Greater)
                    || ((fa, sa) == (fb, sb) && a < b)
            }
        }
    }

    fn best_of<'b>(&self, designs: impl Iterator<Item = &'b Vec<i64>>) -> Option<Vec<i64>> {
        let mut best: Option<&Vec<i64>> = None;
        for x in designs {
            if best.is_none_or(|b| self.better(x, b)) {
                best = Some(x);
            }
        }
        best.cloned()
    }

    fn best_mean(&self) -> Option<f64> {
        let b = self.best_of(self.cache.keys())?;
        self.means(&b).and_then(|m| m.first().copied())
    }
}

/// How a design with the means `m` (the objective, then each constraint) stands: whether its
/// constraint means meet every limit, and a score where larger is better (the objective's mean in
/// the better direction, or minus the violation in units of each tolerance).
pub(crate) fn standing(setup: &Compass, m: &[f64]) -> Option<(bool, f64)> {
    let objective = m.first().copied()?;
    let mut violation = 0.0;
    for (c, &v) in setup.constraints.iter().zip(m.iter().skip(1)) {
        let over = match c.bound {
            crate::select::Bound::AtMost => v - c.limit,
            crate::select::Bound::AtLeast => c.limit - v,
        };
        if over > 0.0 {
            violation += over / c.tolerance;
        }
    }
    let sign = match setup.sense {
        Sense::Maximise => 1.0,
        Sense::Minimise => -1.0,
    };
    Some(if violation > 0.0 {
        (false, -violation)
    } else {
        (true, sign * objective)
    })
}

/// One start's search.
struct Local {
    start: Vec<i64>,
    visited: BTreeSet<Vec<i64>>,
    best: Vec<i64>,
    rng: Rng,
    step: u64,
    done: bool,
    local_optimum: bool,
    /// The design whose last local test did not keep it.
    failed_at: Option<Vec<i64>>,
    /// The designs active around the best at the last step.
    active: Vec<Vec<i64>>,
    /// The space this start searches: the whole space with the held decisions fixed at the
    /// start's values.
    space: Space,
}

impl Local {
    fn new(start: Vec<i64>, index: usize, seed: u64, space: &Space, held: &[usize]) -> Self {
        let mut own = space.clone();
        for &i in held {
            if let (Some(b), Some(&v)) = (own.bounds.get_mut(i), start.get(i)) {
                *b = (v, v);
            }
        }
        Self {
            space: own,
            visited: BTreeSet::new(),
            best: start.clone(),
            start,
            rng: Rng::keyed(seed, &format!("optimize/compass/{index}")),
            step: 0,
            done: false,
            local_optimum: false,
            failed_at: None,
            active: Vec::new(),
        }
    }
}

/// One step of start `s`.
fn step<F>(runner: &mut Runner<'_, F>, s: &mut Local) -> Result<StepEnd, CompassError>
where
    F: FnMut(Stream, &[i64], Range<u64>) -> Result<Vec<Vec<f64>>, String>,
{
    let space = &s.space.clone();
    let setup = runner.setup;
    if runner.runs >= runner.budget {
        return Ok(StepEnd::Exhausted);
    }
    s.step += 1;
    let k = s.step;
    let level = setup.level(k);
    if s.visited.is_empty() {
        let Some(ran) = runner.run_plan(&[(s.start.clone(), level)])? else {
            return Ok(StepEnd::Exhausted);
        };
        s.visited.insert(s.start.clone());
        s.best = s.start.clone();
        s.active = vec![s.start.clone()];
        return Ok(StepEnd::Ran(ran));
    }
    let best = runner
        .best_of(s.visited.iter())
        .unwrap_or_else(|| s.best.clone());
    s.best = best.clone();
    let around = active(space, &best, &s.visited);
    let halves: Vec<Half> = around.iter().map(|(_, h)| h.clone()).collect();
    let mut act: Vec<Vec<i64>> = vec![best.clone()];
    act.extend(around.iter().map(|(y, _)| y.clone()));
    s.active = act.clone();
    let neighbours = space.neighbours(&best);
    if neighbours.iter().all(|y| s.visited.contains(y)) {
        // The area is the best design alone: the local-optimality test.
        let mut plan: Vec<(Vec<i64>, u64)> = act.iter().map(|x| (x.clone(), level)).collect();
        if s.failed_at.as_ref() == Some(&best) && !setup.exact {
            for x in std::iter::once(&best).chain(&neighbours) {
                plan.push((x.clone(), runner.have(x).saturating_add(setup.per_visit)));
            }
        }
        let Some(mut ran) = runner.run_plan(&plan)? else {
            return Ok(StepEnd::Exhausted);
        };
        let now = runner
            .best_of(s.visited.iter())
            .unwrap_or_else(|| best.clone());
        if now != best {
            s.best = now;
            return Ok(StepEnd::Ran(ran));
        }
        if setup.exact || runner.standing(&best).is_some_and(|(f, _)| !f) {
            // Without randomness the best is no worse than any neighbour; a best that does not
            // look feasible has no neighbour that does, so there is nothing to test against.
            s.local_optimum = true;
            s.done = true;
            return Ok(StepEnd::LocalOptimum(ran));
        }
        let alternatives: Vec<Vec<i64>> = neighbours
            .into_iter()
            .filter(|y| runner.standing(y).is_some_and(|(f, _)| f))
            .collect();
        if alternatives.is_empty() {
            s.local_optimum = true;
            s.done = true;
            return Ok(StepEnd::LocalOptimum(ran));
        }
        let kn = Kn {
            delta: setup.delta,
            confidence: setup.confidence,
            first_stage: setup.first_stage,
            sense: setup.sense,
            sampling: Sampling::Common,
            max_runs: None,
            // Exact mode never reaches the test; here only the objective may be stated exact.
            deterministic: setup.exact_objective,
        };
        let before = runner.runs;
        let test = compare_with_standard(&kn, alternatives.len(), |p, reps| {
            let x = if p == 0 {
                &best
            } else {
                match alternatives.get(p - 1) {
                    Some(x) => x,
                    None => return Vec::new(),
                }
            };
            runner.objective(x, reps)
        });
        ran += runner.runs - before;
        if let Some(e) = runner.error.take() {
            return Err(e);
        }
        if runner.exhausted {
            runner.exhausted = false;
            return Ok(StepEnd::Exhausted);
        }
        let test = test.map_err(CompassError::Select)?;
        if test.chosen == 0 && test.outcome != Outcome::NotProven {
            s.local_optimum = true;
            s.done = true;
            return Ok(StepEnd::LocalOptimum(ran));
        }
        s.failed_at = Some(best);
        return Ok(StepEnd::Ran(ran));
    }
    // Sample new designs from the most promising area.
    let mut new: Vec<Vec<i64>> = Vec::new();
    for _ in 0..setup.samples_per_step {
        let x = walk(space, &halves, &best, &mut s.rng);
        if !s.visited.contains(&x) && !new.contains(&x) {
            new.push(x);
        }
    }
    if new.is_empty()
        && let Some(y) = neighbours.into_iter().find(|y| !s.visited.contains(y))
    {
        // The walks met only visited designs; an unvisited neighbour is always in the area.
        new.push(y);
    }
    let plan: Vec<(Vec<i64>, u64)> = act.iter().chain(&new).map(|x| (x.clone(), level)).collect();
    let Some(ran) = runner.run_plan(&plan)? else {
        return Ok(StepEnd::Exhausted);
    };
    s.visited.extend(new);
    Ok(StepEnd::Ran(ran))
}

/// The clean-up's replications of the finalists, by replication index, run as asked.
struct CleanUp<'a, D, F> {
    sample: &'a mut F,
    width: usize,
    finalists: Vec<Finalist<D>>,
    runs: u64,
    error: Option<CompassError>,
}

impl<D, F> CleanUp<'_, D, F>
where
    D: Design,
    F: FnMut(Stream, &D, Range<u64>) -> Result<Vec<Vec<f64>>, String>,
{
    fn rows(&mut self, p: usize, reps: Range<u64>) -> Vec<Vec<f64>> {
        let Some(f) = self.finalists.get(p) else {
            return Vec::new();
        };
        let design = f.design.clone();
        let missing: Vec<u64> = reps
            .clone()
            .filter(|j| !f.cleanup.contains_key(j))
            .collect();
        // Run each stretch of missing replications at once.
        let mut i = 0;
        while i < missing.len() {
            let a = missing[i];
            let mut b = a + 1;
            while i + 1 < missing.len() && missing[i + 1] == b {
                i += 1;
                b += 1;
            }
            i += 1;
            let expected = b - a;
            let rows = match (self.sample)(Stream::CleanUp, &design, a..b) {
                Ok(r) => r,
                Err(m) => {
                    self.error.get_or_insert(CompassError::Sampler(m));
                    return Vec::new();
                }
            };
            if rows.len() as u64 != expected || rows.iter().any(|r| r.len() != self.width) {
                self.error.get_or_insert(CompassError::WrongCount {
                    design: design.show(),
                    expected,
                    got: rows.len() as u64,
                });
                return Vec::new();
            }
            if let Some(j) = rows.iter().position(|r| r.iter().any(|v| !v.is_finite())) {
                self.error.get_or_insert(CompassError::NotFinite {
                    design: design.show(),
                    replication: a + j as u64,
                });
                return Vec::new();
            }
            self.runs = self.runs.saturating_add(expected);
            if let Some(f) = self.finalists.get_mut(p) {
                for (j, row) in (a..b).zip(rows) {
                    f.cleanup.insert(j, row);
                }
            }
        }
        self.finalists.get(p).map_or_else(Vec::new, |f| {
            reps.filter_map(|j| f.cleanup.get(&j).cloned()).collect()
        })
    }
}

/// Searches `space` from `starts` (the starting design first) with `sample(stream, design,
/// replications)` giving, for each replication, the objective followed by one value per
/// constraint, then cleans up. `stop` is asked after each step that ran replications; when it
/// says so, the search ends and the clean-up runs.
pub fn optimise<F, S>(
    setup: &Compass,
    space: &Space,
    starts: &[Vec<i64>],
    mut sample: F,
    mut stop: S,
) -> Result<Answer, CompassError>
where
    F: FnMut(Stream, &[i64], Range<u64>) -> Result<Vec<Vec<f64>>, String>,
    S: FnMut() -> bool,
{
    setup.validate()?;
    space.validate()?;
    let first = starts.first().ok_or(CompassError::NoStart)?.clone();
    for (i, s) in starts.iter().enumerate() {
        if !space.contains(s) {
            return Err(CompassError::Start(i));
        }
    }
    if setup.held.iter().any(|&i| i >= space.bounds.len()) {
        return Err(CompassError::Setting("held"));
    }
    let width = 1 + setup.constraints.len();
    let per_design = if setup.exact { 1 } else { setup.first_stage };

    // A small space: every design to the clean-up.
    let all = space
        .enumerate(setup.all_designs_up_to)
        .filter(|d| (d.len() as u64).saturating_mul(per_design) <= setup.max_runs);
    if let Some(designs) = all {
        let count = designs.len() as u64;
        let finalists = designs
            .into_iter()
            .map(|design| Finalist {
                design,
                search_mean: None,
                search_replications: 0,
                cleanup: BTreeMap::new(),
            })
            .collect();
        let mut whole = |st: Stream, x: &Vec<i64>, r: Range<u64>| sample(st, x, r);
        let mut answer = clean_up(setup, finalists, setup.max_runs, &mut whole, width)?;
        answer.all_designs = true;
        answer.ended = Ended::AllDesigns;
        answer.space = Some(count);
        answer.starts = Vec::new();
        return Ok(answer);
    }

    let mut runner = Runner {
        setup,
        sample,
        width,
        cache: BTreeMap::new(),
        runs: 0,
        budget: setup.search_runs,
        exhausted: false,
        error: None,
    };
    let mut locals: Vec<Local> = Vec::new();
    for s in starts {
        if !locals.iter().any(|l| &l.start == s) {
            locals.push(Local::new(
                s.clone(),
                locals.len(),
                setup.seed,
                space,
                &setup.held,
            ));
        }
    }
    let mut trace = Vec::new();
    let ended = 'search: loop {
        if locals.iter().all(|l| l.done) {
            break Ended::LocalOptima;
        }
        for s in locals.iter_mut().filter(|l| !l.done) {
            match step(&mut runner, s)? {
                StepEnd::Exhausted => break 'search Ended::Budget,
                // A step that ran replications is traced and followed by a look at `stop`,
                // whether or not it ended its start.
                StepEnd::LocalOptimum(ran) | StepEnd::Ran(ran) => {
                    if ran > 0 {
                        trace.push(TracePoint {
                            runs: runner.runs,
                            best: runner.best_mean(),
                        });
                        if stop() {
                            break 'search Ended::Stopped;
                        }
                    }
                }
            }
        }
    };

    // Replications a step ran before the budget stopped it are traced too.
    if runner.runs > trace.last().map_or(0, |t: &TracePoint| t.runs) {
        trace.push(TracePoint {
            runs: runner.runs,
            best: runner.best_mean(),
        });
    }
    // Each start's best and the designs active around it, ranked on every replication run, so
    // that what the last steps (of this start or another) found counts.
    for l in locals.iter_mut().filter(|l| !l.visited.is_empty()) {
        if let Some(b) = runner.best_of(l.visited.iter()) {
            let mut act = vec![b.clone()];
            act.extend(active(&l.space, &b, &l.visited).into_iter().map(|(y, _)| y));
            l.best = b;
            l.active = act;
        }
    }
    // The finalists: the search's best and the designs active around it, each start's final
    // design, and the starting design.
    let ends: Vec<Vec<i64>> = locals
        .iter()
        .filter(|l| !l.visited.is_empty())
        .map(|l| l.best.clone())
        .collect();
    let overall = runner.best_of(ends.iter());
    let mut chosen_designs: Vec<Vec<i64>> = Vec::new();
    let push = |x: &Vec<i64>, list: &mut Vec<Vec<i64>>| {
        if !list.contains(x) {
            list.push(x.clone());
        }
    };
    if let Some(b) = &overall {
        push(b, &mut chosen_designs);
        if let Some(l) = locals.iter().find(|l| &l.best == b) {
            let mut around = l.active.clone();
            around.sort_by(|p, q| {
                if runner.better(p, q) {
                    std::cmp::Ordering::Less
                } else if runner.better(q, p) {
                    std::cmp::Ordering::Greater
                } else {
                    p.cmp(q)
                }
            });
            for x in &around {
                push(x, &mut chosen_designs);
            }
        }
    }
    for e in &ends {
        push(e, &mut chosen_designs);
    }
    push(&first, &mut chosen_designs);
    let finalists: Vec<Finalist> = chosen_designs
        .into_iter()
        .map(|design| Finalist {
            search_mean: runner.means(&design).and_then(|m| m.first().copied()),
            search_replications: runner.have(&design),
            design,
            cleanup: BTreeMap::new(),
        })
        .collect();
    let starts_report = locals
        .iter()
        .map(|l| StartReport {
            start: l.start.clone(),
            end: (!l.visited.is_empty()).then(|| l.best.clone()),
            local_optimum: l.local_optimum,
            steps: l.step,
            visited: l.visited.len(),
        })
        .collect();
    let search_runs = runner.runs;
    let tried = runner.cache.len();
    // In exact mode a finalist's single search replication is its value.
    let exact_values: Vec<Option<Vec<f64>>> = finalists
        .iter()
        .map(|f| runner.cache.get(&f.design).and_then(|r| r.first().cloned()))
        .collect();
    let mut sample = runner.sample;
    let mut finalists = finalists;
    if setup.exact {
        for (f, v) in finalists.iter_mut().zip(exact_values) {
            if let Some(row) = v {
                f.cleanup.insert(0, row);
            }
        }
    }
    let budget = setup.max_runs.saturating_sub(search_runs);
    let mut whole = |st: Stream, x: &Vec<i64>, r: Range<u64>| sample(st, x, r);
    let mut answer = clean_up(setup, finalists, budget, &mut whole, width)?;
    answer.tried = tried;
    answer.search_runs = search_runs;
    answer.ended = ended;
    answer.starts = starts_report;
    answer.trace = trace;
    Ok(answer)
}

/// Whether `row`'s constraint values meet every limit exactly.
fn meets(setup: &Compass, row: &[f64]) -> bool {
    setup
        .constraints
        .iter()
        .zip(row.iter().skip(1))
        .all(|(c, &v)| match c.bound {
            crate::select::Bound::AtMost => v <= c.limit,
            crate::select::Bound::AtLeast => v >= c.limit,
        })
}

/// The clean-up of `finalists` with `budget` replications, on the sampler's [`Stream::CleanUp`]
/// rows (each the objective, then one value per constraint: `width` values). A finalist that
/// already has rows (a search's single replication, without randomness) keeps them. The answer's
/// search fields (`tried`, `search_runs`, `ended`, `starts`, `trace`) are the caller's to fill.
pub fn clean_up<D, F>(
    setup: &Compass,
    finalists: Vec<Finalist<D>>,
    budget: u64,
    sample: &mut F,
    width: usize,
) -> Result<Answer<D>, CompassError>
where
    D: Design,
    F: FnMut(Stream, &D, Range<u64>) -> Result<Vec<Vec<f64>>, String>,
{
    let k = finalists.len();
    let sign = match setup.sense {
        Sense::Maximise => 1.0,
        Sense::Minimise => -1.0,
    };
    let mut answer = Answer {
        outcome: Outcome::NotProven,
        chosen: None,
        confidence: None,
        all_designs: false,
        finalists: Vec::new(),
        in_contention: Vec::new(),
        feasibility: None,
        by_constraint: None,
        tried: 0,
        search_runs: 0,
        cleanup_runs: 0,
        ended: Ended::Budget,
        starts: Vec::new(),
        trace: Vec::new(),
        space: None,
        flat: false,
        flat_constraint: false,
        set_aside: Vec::new(),
    };
    let mut c = CleanUp {
        sample,
        width,
        finalists,
        runs: 0,
        error: None,
    };
    if k == 0 {
        answer.finalists = c.finalists;
        return Ok(answer);
    }
    if setup.exact {
        // One replication each, compared by value.
        for p in 0..k {
            if c.finalists.get(p).is_some_and(|f| f.cleanup.is_empty()) {
                if c.runs >= budget {
                    break;
                }
                c.rows(p, 0..1);
                if let Some(e) = c.error.take() {
                    return Err(e);
                }
            }
        }
        let mut best: Option<(usize, f64)> = None;
        let mut every = true;
        for (p, f) in c.finalists.iter().enumerate() {
            let Some(row) = f.cleanup.values().next() else {
                every = false;
                continue;
            };
            if !meets(setup, row) {
                continue;
            }
            let score = sign * row.first().copied().unwrap_or(f64::NAN);
            if best.is_none_or(|(_, s)| score > s) {
                best = Some((p, score));
            }
        }
        answer.chosen = best.map(|b| b.0);
        answer.outcome = if every && best.is_some() {
            Outcome::ComparedExactly
        } else {
            Outcome::NotProven
        };
        if !setup.constraints.is_empty() {
            answer.feasibility = Some(
                c.finalists
                    .iter()
                    .map(|f| match f.cleanup.values().next() {
                        Some(r) if meets(setup, r) => Decision::Feasible,
                        Some(_) => Decision::Infeasible,
                        None => Decision::Undecided,
                    })
                    .collect(),
            );
        }
        answer.cleanup_runs = c.runs;
        answer.finalists = c.finalists;
        return Ok(answer);
    }
    let kn = Kn {
        delta: setup.delta,
        confidence: setup.confidence,
        first_stage: setup.first_stage,
        sense: setup.sense,
        sampling: Sampling::Common,
        max_runs: Some(budget),
        // Exact mode returned above; here only the objective may be stated exact.
        deterministic: setup.exact_objective,
    };
    let best_search = |fs: &[Finalist<D>]| -> Option<usize> {
        fs.iter()
            .enumerate()
            .filter_map(|(p, f)| f.search_mean.map(|m| (p, sign * m)))
            .fold(None, |b: Option<(usize, f64)>, (p, s)| match b {
                Some((_, bs)) if bs >= s => b,
                _ => Some((p, s)),
            })
            .map(|b| b.0)
    };
    if setup.first_stage.saturating_mul(k as u64) > budget {
        // The clean-up cannot pay for its first stage: not proven. With constraints no design
        // was declared feasible, so none is chosen; without, the best-looking one is named.
        answer.chosen = if setup.constraints.is_empty() {
            best_search(&c.finalists).or(Some(0))
        } else {
            None
        };
        answer.in_contention = (0..k).collect();
        answer.finalists = c.finalists;
        return Ok(answer);
    }
    if setup.constraints.is_empty() {
        let selection = select_best(&kn, k, |p, reps| {
            c.rows(p, reps)
                .into_iter()
                .filter_map(|r| r.first().copied())
                .collect()
        });
        if let Some(e) = c.error.take() {
            return Err(e);
        }
        let selection = selection.map_err(CompassError::Select)?;
        answer.outcome = selection.outcome;
        answer.chosen = Some(selection.chosen);
        answer.flat = selection.flat;
        answer.confidence = (selection.outcome == Outcome::Selected).then_some(setup.confidence);
        if selection.outcome == Outcome::NotProven {
            answer.in_contention = selection.survivors.clone();
        }
    } else {
        let found = select_feasible(
            &kn,
            &setup.constraints,
            setup.feasibility_share,
            k,
            |p, reps| c.rows(p, reps),
        );
        if let Some(e) = c.error.take() {
            return Err(e);
        }
        let found = found.map_err(CompassError::Select)?;
        answer.feasibility = Some(found.feasibility.decisions.clone());
        answer.by_constraint = Some(found.feasibility.by_constraint.clone());
        answer.confidence = found.confidence;
        answer.flat = !found.proven && found.selection.as_ref().is_some_and(|s| s.flat);
        answer.flat_constraint = !found.proven && found.feasibility.flat;
        answer.set_aside = found.set_aside.clone();
        match &found.selection {
            Some(s) => {
                answer.outcome = if found.proven {
                    s.outcome
                } else {
                    Outcome::NotProven
                };
                answer.chosen = Some(s.chosen);
                if answer.outcome == Outcome::NotProven {
                    // KN's survivors among those declared feasible, and every finalist the check
                    // left undecided: any of them may be the answer.
                    let mut still = s.survivors.clone();
                    still.extend(
                        found
                            .feasibility
                            .decisions
                            .iter()
                            .enumerate()
                            .filter_map(|(p, d)| (*d == Decision::Undecided).then_some(p)),
                    );
                    still.sort_unstable();
                    still.dedup();
                    answer.in_contention = still;
                }
            }
            None => {
                answer.outcome = Outcome::NotProven;
                answer.chosen = None;
                answer.in_contention = found
                    .feasibility
                    .decisions
                    .iter()
                    .enumerate()
                    .filter_map(|(p, d)| (*d != Decision::Infeasible).then_some(p))
                    .collect();
            }
        }
        if answer.outcome == Outcome::NotProven {
            answer.confidence = None;
        }
    }
    answer.cleanup_runs = c.runs;
    answer.finalists = c.finalists;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn square(n: i64) -> Space {
        Space {
            bounds: vec![(0, n), (0, n)],
            limits: Vec::new(),
        }
    }

    /// The area by its definition: every design at least as close to `b` as to any other visited.
    fn area_by_definition(space: &Space, b: &[i64], visited: &BTreeSet<Vec<i64>>) -> Vec<Vec<i64>> {
        let d2 =
            |x: &[i64], y: &[i64]| -> i64 { x.iter().zip(y).map(|(p, q)| (p - q) * (p - q)).sum() };
        space
            .enumerate(u64::MAX)
            .unwrap()
            .into_iter()
            .filter(|x| {
                visited
                    .iter()
                    .all(|y| y.as_slice() == b || d2(x, b) <= d2(x, y))
            })
            .collect()
    }

    #[test]
    fn the_linear_programme_finds_a_corner() {
        // max x + y with x ≤ 3, y ≤ 2, x + y ≤ 4, −x ≤ 1, −y ≤ 1: 4.
        let rows = vec![
            (vec![1.0, 0.0], 3.0),
            (vec![0.0, 1.0], 2.0),
            (vec![1.0, 1.0], 4.0),
            (vec![-1.0, 0.0], 1.0),
            (vec![0.0, -1.0], 1.0),
        ];
        let v = lp_max(&[1.0, 1.0], &rows).unwrap();
        assert!((v - 4.0).abs() < 1e-12, "{v}");
        let v = lp_max(&[-1.0, 2.0], &rows).unwrap();
        assert!((v - 5.0).abs() < 1e-12, "{v}");
        // Unbounded without the box.
        assert_eq!(lp_max(&[1.0, 0.0], &[(vec![0.0, 1.0], 1.0)]), None);
    }

    /// Pruning keeps the area exactly: on random visited sets in small lattices, with and
    /// without a cap, the designs in the area by the pruned halfspaces are those by the
    /// definition, and every design dropped is implied by the others.
    #[test]
    fn pruning_keeps_the_area() {
        let mut rng = Rng::new(42);
        for case in 0..200 {
            let n = 4 + (case % 5) as i64;
            let mut space = square(n);
            if case % 3 == 0 {
                space.limits.push(Linear {
                    offset: 0.0,
                    coefficients: vec![1.0, 2.0],
                    limit: 1.5 * n as f64,
                });
            }
            let all = space.enumerate(u64::MAX).unwrap();
            let mut visited = BTreeSet::new();
            let count = 2 + (rng.uniform() * 8.0) as usize;
            for _ in 0..count {
                let x = &all[(rng.uniform() * all.len() as f64) as usize % all.len()];
                visited.insert(x.clone());
            }
            let b = visited.iter().next().unwrap().clone();
            let kept = active(&space, &b, &visited);
            let halves: Vec<Half> = kept.iter().map(|(_, h)| h.clone()).collect();
            let pruned: Vec<Vec<i64>> = all
                .iter()
                .filter(|x| halves.iter().all(|h| h.holds(x)))
                .cloned()
                .collect();
            assert_eq!(
                pruned,
                area_by_definition(&space, &b, &visited),
                "case {case}"
            );
        }
    }

    #[test]
    fn walks_stay_in_the_area_and_reach_all_of_it() {
        let space = Space {
            bounds: vec![(0, 9), (0, 9)],
            limits: vec![Linear {
                offset: 0.0,
                coefficients: vec![1.0, 1.0],
                limit: 12.0,
            }],
        };
        let visited: BTreeSet<Vec<i64>> = [vec![4, 4], vec![8, 1], vec![1, 7], vec![6, 6]]
            .into_iter()
            .collect();
        let b = vec![4, 4];
        let halves: Vec<Half> = active(&space, &b, &visited)
            .into_iter()
            .map(|(_, h)| h)
            .collect();
        let area = area_by_definition(&space, &b, &visited);
        let mut rng = Rng::new(7);
        let mut seen = BTreeSet::new();
        for _ in 0..3000 {
            let x = walk(&space, &halves, &b, &mut rng);
            assert!(area.contains(&x), "{x:?} is outside the area");
            seen.insert(x);
        }
        assert_eq!(
            seen.len(),
            area.len(),
            "the walks reach every design of the area"
        );
    }

    #[test]
    fn neighbours_and_enumeration_respect_the_cap() {
        let space = Space {
            bounds: vec![(1, 3), (1, 3)],
            limits: vec![Linear {
                offset: 0.0,
                coefficients: vec![1.0, 1.0],
                limit: 4.0,
            }],
        };
        assert_eq!(
            space.enumerate(100).unwrap(),
            vec![
                vec![1, 1],
                vec![1, 2],
                vec![1, 3],
                vec![2, 1],
                vec![2, 2],
                vec![3, 1]
            ]
        );
        assert_eq!(space.enumerate(5), None);
        assert_eq!(space.neighbours(&[2, 2]), vec![vec![1, 2], vec![2, 1]]);
    }

    /// A quadratic bowl with normal noise and common random numbers: the minimum (7, 3) of
    /// (x − 7)² + (y − 3)² over 0..40 × 0..40 is found and selected, and the budget is kept.
    #[test]
    fn a_noisy_bowl_is_minimised() {
        let space = square(40);
        let f = |x: &[i64]| ((x[0] - 7) * (x[0] - 7) + (x[1] - 3) * (x[1] - 3)) as f64;
        let mut setup = Compass::new(0.5, Sense::Minimise, 6000);
        setup.seed = 11;
        let mut calls = 0u64;
        let answer = optimise(
            &setup,
            &space,
            &[vec![30, 30], vec![5, 35]],
            |stream, x, reps| {
                calls += reps.end - reps.start;
                let salt = if stream == Stream::Search { 1 } else { 2 };
                Ok(reps
                    .map(|j| {
                        let mut r = Rng::keyed(j, &format!("{salt}"));
                        let shared = r.uniform() - 0.5;
                        let mut own = Rng::keyed(j, &format!("{salt}/{x:?}"));
                        vec![f(x) + shared + 0.3 * (own.uniform() - 0.5)]
                    })
                    .collect())
            },
            || false,
        )
        .unwrap();
        let chosen = &answer.finalists[answer.chosen.unwrap()].design;
        assert_eq!(chosen, &vec![7, 3], "{answer:?}");
        assert_eq!(answer.outcome, Outcome::Selected);
        assert!(answer.total_runs() <= 6000);
        assert_eq!(calls, answer.total_runs());
        assert_eq!(answer.starts.len(), 2);
        // The starting design is a finalist.
        assert!(answer.finalists.iter().any(|f| f.design == vec![30, 30]));
    }

    /// A held decision (a rule's level) is never moved: each start searches the other decision
    /// at its own level, and the levels meet in the clean-up, where the better one is chosen.
    #[test]
    fn a_held_decision_stays_at_each_starts_level() {
        let space = Space {
            bounds: vec![(0, 1), (0, 60)],
            limits: Vec::new(),
        };
        let f = |x: &[i64]| {
            let shift = if x[0] == 1 { -5.0 } else { 0.0 };
            ((x[1] - 10) * (x[1] - 10)) as f64 + shift
        };
        let mut setup = Compass::new(0.5, Sense::Minimise, 6000);
        setup.held = vec![0];
        let answer = optimise(
            &setup,
            &space,
            &[vec![0, 50], vec![1, 50]],
            |stream, x, reps| {
                Ok(reps
                    .map(|j| {
                        let salt = if stream == Stream::Search { 1 } else { 2 };
                        let mut r = Rng::keyed(j, &format!("{salt}"));
                        let mut own = Rng::keyed(j, &format!("{salt}/{x:?}"));
                        vec![f(x) + (r.uniform() - 0.5) + 0.3 * (own.uniform() - 0.5)]
                    })
                    .collect())
            },
            || false,
        )
        .unwrap();
        for s in &answer.starts {
            let end = s.end.clone().unwrap();
            assert_eq!(end[0], s.start[0], "{:?}", answer.starts);
        }
        let chosen = &answer.finalists[answer.chosen.unwrap()].design;
        assert_eq!(chosen, &vec![1, 10], "{answer:?}");
        assert!(answer.finalists.iter().any(|f| f.design[0] == 0));
        // A held position outside the space is refused.
        setup.held = vec![2];
        let e = optimise(
            &setup,
            &space,
            &[vec![0, 50]],
            |_, _, _| Ok(vec![]),
            || false,
        );
        assert_eq!(e.unwrap_err(), CompassError::Setting("held"));
    }

    /// A search stopped by the caller after S runs is repeated exactly with `search_runs` = S.
    #[test]
    fn a_stopped_search_repeats_with_its_runs() {
        let space = square(30);
        let f = |x: &[i64]| ((x[0] - 20) * (x[0] - 20) + 2 * (x[1] - 9) * (x[1] - 9)) as f64;
        let sample =
            |stream: Stream, x: &[i64], reps: Range<u64>| -> Result<Vec<Vec<f64>>, String> {
                let salt = if stream == Stream::Search { "s" } else { "c" };
                Ok(reps
                    .map(|j| {
                        let mut r = Rng::keyed(j, &format!("{salt}/{x:?}"));
                        vec![f(x) + r.uniform()]
                    })
                    .collect())
            };
        let mut setup = Compass::new(0.5, Sense::Minimise, 4000);
        setup.seed = 3;
        let mut steps = 0;
        let stopped = optimise(&setup, &space, &[vec![0, 0]], sample, || {
            steps += 1;
            steps >= 6
        })
        .unwrap();
        assert_eq!(stopped.ended, Ended::Stopped);
        let again = Compass {
            search_runs: stopped.search_runs,
            ..setup.clone()
        };
        let repeated = optimise(&again, &space, &[vec![0, 0]], sample, || false).unwrap();
        assert_eq!(repeated.ended, Ended::Budget);
        assert_eq!(repeated.finalists, stopped.finalists);
        assert_eq!(repeated.chosen, stopped.chosen);
        assert_eq!(repeated.search_runs, stopped.search_runs);
        assert_eq!(repeated.cleanup_runs, stopped.cleanup_runs);
    }

    #[test]
    fn a_small_space_sends_every_design_to_the_clean_up() {
        let space = Space {
            bounds: vec![(1, 5)],
            limits: Vec::new(),
        };
        let setup = Compass::new(1.0, Sense::Maximise, 2000);
        let answer = optimise(
            &setup,
            &space,
            &[vec![1]],
            |_, x, reps| {
                Ok(reps
                    .map(|j| vec![x[0] as f64 * 3.0 + Rng::keyed(j, "n").uniform()])
                    .collect())
            },
            || false,
        )
        .unwrap();
        assert!(answer.all_designs);
        assert_eq!(answer.ended, Ended::AllDesigns);
        assert_eq!(answer.space, Some(5));
        assert_eq!(answer.finalists.len(), 5);
        assert_eq!(answer.finalists[answer.chosen.unwrap()].design, vec![5]);
        assert_eq!(answer.search_runs, 0);
    }

    /// A costs-only objective (stated exact) and a constraint whose value is the same in every
    /// first-stage replication for the cheapest design: the check leaves that design undecided,
    /// and since it costs less than the one KN chose it may be the answer. Not proven, with the
    /// undecided design in contention and the reason marked as a flat constraint.
    #[test]
    fn a_cheaper_design_with_a_flat_constraint_stays_in_contention() {
        let space = Space {
            bounds: vec![(1, 3)],
            limits: Vec::new(),
        };
        let mut setup = Compass::new(1.0, Sense::Minimise, 2000);
        setup.exact_objective = true;
        setup.constraints = vec![Constraint {
            bound: crate::select::Bound::AtMost,
            limit: 0.0,
            tolerance: 0.5,
        }];
        let answer = optimise(
            &setup,
            &space,
            &[vec![2]],
            |_, x, reps| {
                Ok(reps
                    .map(|j| {
                        let y = if x[0] == 1 {
                            -1.0
                        } else {
                            -1.0 + 0.001 * (Rng::keyed(j, &format!("{x:?}")).uniform() - 0.5)
                        };
                        vec![x[0] as f64, y]
                    })
                    .collect())
            },
            || false,
        )
        .unwrap();
        assert_eq!(answer.outcome, Outcome::NotProven, "{answer:?}");
        assert_eq!(answer.confidence, None);
        assert!(answer.flat_constraint, "{answer:?}");
        assert!(!answer.flat, "{answer:?}");
        let still: Vec<&Vec<i64>> = answer
            .in_contention
            .iter()
            .map(|&p| &answer.finalists[p].design)
            .collect();
        assert!(still.contains(&&vec![1]), "{answer:?}");
        assert!(still.contains(&&vec![2]), "{answer:?}");
    }

    /// Equal values in every replication, without the statement that there is no randomness
    /// (`exact` false): never compared exactly, and nothing claimed with confidence.
    #[test]
    fn constant_values_without_the_statement_are_not_proven() {
        let space = Space {
            bounds: vec![(1, 5)],
            limits: Vec::new(),
        };
        let setup = Compass::new(1.0, Sense::Maximise, 2000);
        let answer = optimise(
            &setup,
            &space,
            &[vec![1]],
            |_, x, reps| Ok(reps.map(|_| vec![x[0] as f64 * 3.0]).collect()),
            || false,
        )
        .unwrap();
        assert_eq!(answer.outcome, Outcome::NotProven, "{answer:?}");
        assert_eq!(answer.confidence, None);
        assert!(answer.flat);
        assert_eq!(answer.finalists[answer.chosen.unwrap()].design, vec![5]);
    }

    #[test]
    fn without_randomness_each_design_runs_once() {
        let space = square(60);
        let mut setup = Compass::new(1.0, Sense::Minimise, 5000);
        setup.exact = true;
        let mut asked = BTreeMap::new();
        let answer = optimise(
            &setup,
            &space,
            &[vec![50, 50]],
            |stream, x, reps| {
                *asked
                    .entry((stream == Stream::Search, x.to_vec()))
                    .or_insert(0) += reps.end - reps.start;
                Ok(reps
                    .map(|_| vec![((x[0] - 12).abs() + (x[1] - 40).abs()) as f64])
                    .collect())
            },
            || false,
        )
        .unwrap();
        assert_eq!(answer.outcome, Outcome::ComparedExactly);
        assert_eq!(answer.confidence, None);
        assert_eq!(
            answer.finalists[answer.chosen.unwrap()].design,
            vec![12, 40]
        );
        assert!(asked.values().all(|&n| n == 1), "{asked:?}");
        assert_eq!(answer.cleanup_runs, 0);
    }

    #[test]
    fn a_value_that_is_not_a_number_is_refused() {
        let space = square(60);
        let setup = Compass::new(1.0, Sense::Minimise, 5000);
        let e = optimise(
            &setup,
            &space,
            &[vec![3, 3]],
            |_, x, reps| {
                Ok(reps
                    .map(|_| vec![if x == [3, 3] { 1.0 } else { f64::NAN }])
                    .collect())
            },
            || false,
        );
        assert!(matches!(e, Err(CompassError::NotFinite { .. })), "{e:?}");
    }

    #[test]
    fn a_start_outside_the_space_is_refused() {
        let space = Space {
            bounds: vec![(0, 9), (0, 9)],
            limits: vec![Linear {
                offset: 0.0,
                coefficients: vec![1.0, 1.0],
                limit: 5.0,
            }],
        };
        let setup = Compass::new(1.0, Sense::Minimise, 5000);
        let e = optimise(
            &setup,
            &space,
            &[vec![4, 4]],
            |_, _, r| Ok(r.map(|_| vec![0.0]).collect()),
            || false,
        );
        assert_eq!(e, Err(CompassError::Start(0)));
    }

    /// A coordinate's edge under a limit is found by bisection: a range of a billion values with
    /// a tiny cost per step takes a few dozen checks, and the edge is where `fits` changes.
    #[test]
    fn a_wide_range_is_bounded_by_bisection() {
        let space = Space {
            bounds: vec![(0, 1_000_000_000)],
            limits: vec![Linear {
                coefficients: vec![1e-18],
                offset: 0.0,
                limit: 1e-12,
            }],
        };
        let (lo, hi) = coordinate_range(&space, &[], &[0], 0);
        assert_eq!(lo, 0);
        assert!(space.contains(&[hi]), "{hi}");
        assert!(hi == 1_000_000_000 || !space.contains(&[hi + 1]), "{hi}");
        // 10⁻¹⁸·x ≤ 10⁻¹² up to x = 10⁶, and the allowance is far too small to admit 10⁶ + 2.
        assert!((999_999..=1_000_001).contains(&hi), "{hi}");
    }

    /// A cap of decimal costs: a design exactly on it is inside, one above it by more than
    /// rounding is not, however small the numbers.
    #[test]
    fn the_cap_admits_rounding_and_nothing_more() {
        let tenth = Space {
            bounds: vec![(0, 5)],
            limits: vec![Linear {
                coefficients: vec![0.1],
                offset: 0.0,
                limit: 0.3,
            }],
        };
        assert!(
            tenth.contains(&[3]),
            "0.1 × 3 is 0.30000000000000004, on the cap"
        );
        assert!(!tenth.contains(&[4]));
        let tiny = Space {
            bounds: vec![(0, 2)],
            limits: vec![Linear {
                coefficients: vec![1e-10],
                offset: 0.0,
                limit: 1e-10,
            }],
        };
        assert!(tiny.contains(&[1]));
        assert!(!tiny.contains(&[2]), "2e-10 is over a cap of 1e-10");
    }

    /// Pruning works on a relaxation that covers every design the space admits: with a limit
    /// of 10⁻¹² on 10⁻¹²·x over 0..3, the area around 0 with 2 visited is as its definition says.
    #[test]
    fn pruning_covers_what_the_space_admits() {
        let space = Space {
            bounds: vec![(0, 3)],
            limits: vec![Linear {
                coefficients: vec![1e-12],
                offset: 0.0,
                limit: 1e-12,
            }],
        };
        let visited: BTreeSet<Vec<i64>> = [vec![0], vec![2]].into_iter().collect();
        let halves: Vec<Half> = active(&space, &[0], &visited)
            .into_iter()
            .map(|(_, h)| h)
            .collect();
        let all = space.enumerate(u64::MAX).unwrap();
        let pruned: Vec<Vec<i64>> = all
            .iter()
            .filter(|x| halves.iter().all(|h| h.holds(x)))
            .cloned()
            .collect();
        assert_eq!(pruned, area_by_definition(&space, &[0], &visited));
    }

    /// Every replication the search runs is in its trace, and `stop` is asked after every step
    /// that ran replications, a step that ended its start included: here every design breaks the
    /// constraint, so each start ends when its best's neighbours are all visited, after a top-up.
    #[test]
    fn the_trace_and_the_stop_see_every_step_that_ran() {
        let space = Space {
            bounds: vec![(0, 3)],
            limits: Vec::new(),
        };
        let mut setup = Compass::new(1.0, Sense::Minimise, 5000);
        setup.all_designs_up_to = 0;
        setup.constraints = vec![Constraint {
            bound: crate::select::Bound::AtMost,
            limit: -1.0,
            tolerance: 0.1,
        }];
        let mut asked = 0;
        let answer = optimise(
            &setup,
            &space,
            &[vec![0], vec![3]],
            |_, x, reps| {
                Ok(reps
                    .map(|j| vec![x[0] as f64, Rng::keyed(j, "c").uniform()])
                    .collect())
            },
            || {
                asked += 1;
                false
            },
        )
        .unwrap();
        assert_eq!(answer.ended, Ended::LocalOptima);
        assert_eq!(
            answer.trace.last().map(|t| t.runs),
            Some(answer.search_runs)
        );
        assert_eq!(asked, answer.trace.len());
        assert!(answer.starts.iter().all(|s| s.local_optimum));
    }

    /// The shortlist is ranked on the last replications run: a search whose budget ends right
    /// after it ran a better design still sends that design to the clean-up.
    #[test]
    fn the_shortlist_ranks_the_last_replications() {
        let space = Space {
            bounds: vec![(0, 1)],
            limits: Vec::new(),
        };
        let mut setup = Compass::new(1.0, Sense::Minimise, 10);
        setup.exact = true;
        setup.all_designs_up_to = 0;
        setup.search_runs = 2;
        let answer = optimise(
            &setup,
            &space,
            &[vec![0]],
            |_, x, reps| Ok(reps.map(|_| vec![1.0 - x[0] as f64]).collect()),
            || false,
        )
        .unwrap();
        assert_eq!(answer.search_runs, 2);
        assert_eq!(
            answer.finalists[answer.chosen.unwrap()].design,
            vec![1],
            "{answer:?}"
        );
    }

    /// With constraints and no budget left for the clean-up, nothing was declared feasible, so
    /// nothing is chosen, however good a design that breaks the constraint looks.
    #[test]
    fn a_constrained_search_without_a_clean_up_chooses_nothing() {
        let space = Space {
            bounds: vec![(0, 1)],
            limits: Vec::new(),
        };
        let mut setup = Compass::new(1.0, Sense::Maximise, 40);
        setup.all_designs_up_to = 0;
        setup.search_runs = 40;
        setup.constraints = vec![Constraint {
            bound: crate::select::Bound::AtMost,
            limit: 1.0,
            tolerance: 0.1,
        }];
        let answer = optimise(
            &setup,
            &space,
            &[vec![0]],
            |_, x, reps| {
                Ok(reps
                    .map(|j| {
                        let e = Rng::keyed(j, "n").uniform() * 0.01;
                        vec![10.0 * x[0] as f64 + e, 2.0 * x[0] as f64 + e]
                    })
                    .collect())
            },
            || false,
        )
        .unwrap();
        assert_eq!(answer.outcome, Outcome::NotProven);
        assert_eq!(answer.chosen, None, "{answer:?}");
        assert!(!answer.in_contention.is_empty());
    }
}
