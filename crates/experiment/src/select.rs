//! Selecting the best of a few systems, and deciding which of them meet constraints, with a
//! stated probability of being right.
//!
//! Four fully sequential procedures from the ranking-and-selection literature. Each takes a
//! first stage of n₀ replications from every system, then one more replication from every
//! system still undecided, until the evidence settles the question:
//!
//! - [`select_best`]: Kim and Nelson's procedure KN (2001), in the form Tsai and Nelson restate
//!   it (2008, their Generic Procedure with no controls). With probability at least 1 − α it
//!   chooses the best system, or one within the indifference amount δ of it.
//! - [`check_feasibility`]: Batur and Kim's procedure F_B (2010) for several constraints on
//!   means, as Zhou, Andradóttir, Kim and Park describe it (2025, Algorithm 1). With probability
//!   at least 1 − α every system at least ε inside all its limits is declared feasible and every
//!   system at least ε beyond one of them infeasible.
//! - [`select_feasible`]: the check, then KN among the systems it declared feasible, with α
//!   split between the two by Bonferroni's inequality, so the stated confidence covers both.
//! - [`compare_with_standard`]: Kim's comparison with a standard (2003, the generic procedure
//!   with an unknown standard): keeps the standard unless an alternative is better, and then
//!   chooses the best alternative. It is the local-optimality test of Industrial Strength
//!   COMPASS.
//!
//! The procedures are written from the published papers; no code of any implementation was
//! used. All of them use Kim and Nelson's triangular continuation region with c = 1, the region
//! Tsai and Nelson present and Batur and Kim recommend.
//!
//! **Samplers.** Observations come from the caller: `sample(system, replications)` returns one
//! value per replication in the range (one row of values per replication for the constraint
//! procedures). Every system is asked for replication j under the same index j, so a sampler
//! that seeds replication j alike for every system (as [`crate::Experiment::stream`] does) gives
//! common random numbers. KN, the check and the comparison allow them: KN and the comparison
//! work on each pair's variance of differences, and F_B and the comparison take their error per
//! system by Bonferroni's inequality ([`Sampling::Common`]). A sampler is called once with the
//! first stage's range for every system and then with one replication at a time, so the same
//! procedure runs on simulations (each call a range of replications) and on synthetic data.
//!
//! **Assumptions.** Observations are normal and independent across replications, as the papers
//! assume. The guarantees are for that case; a replication's mean over a long run is close to
//! normal, a count in a short run may not be.

use std::ops::Range;

use stochcraft_random::math::powf;

/// Whether larger or smaller values are better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sense {
    #[default]
    Minimise,
    Maximise,
}

impl Sense {
    /// The factor that turns an observation into one where larger is better, as the papers
    /// state their procedures ("multiplying each observation by −1 will solve a minimization
    /// problem", Kim 2003).
    fn sign(self) -> f64 {
        match self {
            Sense::Maximise => 1.0,
            Sense::Minimise => -1.0,
        }
    }
}

/// How the systems' replications relate to each other, which sets how the error is shared out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sampling {
    /// Common random numbers (or any dependence between systems): the error is split by
    /// Bonferroni's inequality, which holds whatever the dependence. Always valid.
    #[default]
    Common,
    /// Systems simulated independently: the papers' slightly smaller constants for that case
    /// (Tsai and Nelson's Remark 4.1, Kim's Case B, Batur and Kim's equation for independent
    /// systems). Wrong if the systems share random numbers.
    Independent,
}

/// The settings of [`select_best`] and [`compare_with_standard`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Kn {
    /// The indifference amount δ: the smallest difference in means worth telling apart, in the
    /// observations' unit. Required; there is no default across units.
    pub delta: f64,
    /// 1 − α, the probability of a correct selection (default 0.95).
    pub confidence: f64,
    /// n₀, the first stage's replications from every system (default 10, at least 2).
    pub first_stage: u64,
    pub sense: Sense,
    pub sampling: Sampling,
    /// The most replications to take, summed over systems. When the next step would pass it, the
    /// procedure stops and says the result is not proven ([`Outcome::NotProven`]).
    pub max_runs: Option<u64>,
}

impl Kn {
    pub fn new(delta: f64, sense: Sense) -> Self {
        Self {
            delta,
            confidence: 0.95,
            first_stage: 10,
            sense,
            sampling: Sampling::Common,
            max_runs: None,
        }
    }

    /// KN's η for `systems` systems: [`eta`] at β = α / (k − 1), or at
    /// β = 1 − (1 − α)^(1/(k − 1)) for independent systems (Tsai and Nelson, Setup and
    /// Remark 4.1). `None` for fewer than two systems or settings out of range.
    pub fn eta(&self, systems: usize) -> Option<f64> {
        if systems < 2 {
            return None;
        }
        let k = systems as f64;
        let alpha = 1.0 - self.confidence;
        let beta = match self.sampling {
            Sampling::Common => alpha / (k - 1.0),
            Sampling::Independent => 1.0 - powf(self.confidence, 1.0 / (k - 1.0)),
        };
        eta(beta, self.first_stage)
    }

    /// KN's h² = 2η(n₀ − 1) for `systems` systems.
    pub fn h2(&self, systems: usize) -> Option<f64> {
        Some(2.0 * self.eta(systems)? * (self.first_stage as f64 - 1.0))
    }

    /// Checks δ, the confidence against the number of systems and n₀.
    fn validate(&self, systems: usize) -> Result<(), SelectError> {
        if systems == 0 {
            return Err(SelectError::NoSystems);
        }
        if !(self.delta.is_finite() && self.delta > 0.0) {
            return Err(SelectError::Indifference(self.delta));
        }
        // Tsai and Nelson's Setup: 1/k < 1 − α < 1 (any confidence below 1 for one system).
        let floor = if systems > 1 {
            1.0 / systems as f64
        } else {
            0.0
        };
        if !(self.confidence > floor && self.confidence < 1.0) {
            return Err(SelectError::Confidence(self.confidence));
        }
        if self.first_stage < 2 {
            return Err(SelectError::FirstStage(self.first_stage));
        }
        Ok(())
    }
}

/// The constant η of the triangular region with c = 1: the solution of
/// ½ (1 + 2η)^(−(n₀ − 1)/2) = β, which is η = ½ [(2β)^(−2/(n₀ − 1)) − 1] (Kim and Nelson 2001;
/// Kim 2003's equation for η with c = 1; Batur and Kim's Remark 1 as Zhou et al. give it).
/// `None` unless 0 < 2β < 1 and n₀ ≥ 2.
pub fn eta(beta: f64, first_stage: u64) -> Option<f64> {
    if first_stage < 2 || !(beta > 0.0 && 2.0 * beta < 1.0) {
        return None;
    }
    let value = 0.5 * (powf(2.0 * beta, -2.0 / (first_stage as f64 - 1.0)) - 1.0);
    (value.is_finite() && value > 0.0).then_some(value)
}

/// What a selection's answer claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The chosen system is the best, or within δ of the best, with the stated confidence.
    Selected,
    /// Every system gave the same value in every first-stage replication: there is no
    /// randomness, the systems were compared exactly on n₀ replications, and no probability is
    /// stated.
    ComparedExactly,
    /// The budget ran out first: the chosen system is the best-looking of those still in
    /// contention, not proven best.
    NotProven,
}

/// The answer of [`select_best`] or [`compare_with_standard`]. Systems are the sampler's
/// indices.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub outcome: Outcome,
    /// The chosen system ([`Outcome::NotProven`]: the best-looking one still in contention).
    pub chosen: usize,
    /// The systems still in contention when the procedure stopped: the chosen one alone, unless
    /// the budget ran out or several gave exactly the same total at the end of the region.
    pub survivors: Vec<usize>,
    /// Replications taken from each system.
    pub runs: Vec<u64>,
    /// Each system's mean over its replications, in the observations' unit and sense; `None`
    /// for a system that was not sampled.
    pub means: Vec<Option<f64>>,
    /// The probability of a correct selection that the answer claims: `Some(1 − α)` only for
    /// [`Outcome::Selected`].
    pub confidence: Option<f64>,
}

impl Selection {
    /// Replications summed over systems.
    pub fn total_runs(&self) -> u64 {
        self.runs.iter().sum()
    }
}

/// A constraint on a system's mean: at most or at least `limit`, with the tolerance ε of
/// Andradóttir and Kim. A system whose mean is within ε of the limit may be declared either
/// way; one at least ε inside it must be declared feasible, one at least ε beyond it infeasible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constraint {
    pub bound: Bound,
    pub limit: f64,
    pub tolerance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    AtMost,
    AtLeast,
}

impl Bound {
    /// The factor that turns the constraint into "at most", as the papers state it.
    fn sign(self) -> f64 {
        match self {
            Bound::AtMost => 1.0,
            Bound::AtLeast => -1.0,
        }
    }
}

/// The settings of [`check_feasibility`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Feasibility {
    /// 1 − α, the probability that every system is decided correctly (default 0.95).
    pub confidence: f64,
    /// n₀ (default 10, at least 2).
    pub first_stage: u64,
    pub sampling: Sampling,
    /// The most replications to take, summed over systems; systems not decided by then are
    /// [`Decision::Undecided`].
    pub max_runs: Option<u64>,
}

impl Default for Feasibility {
    fn default() -> Self {
        Self {
            confidence: 0.95,
            first_stage: 10,
            sampling: Sampling::Common,
            max_runs: None,
        }
    }
}

impl Feasibility {
    /// F_B's η for `systems` systems and `constraints` constraints: [`eta`] at
    /// β_B = α / (k s), or at [1 − (1 − α)^(1/k)] / s for independent systems (Batur and Kim's
    /// equation (2) as Zhou et al. give it).
    pub fn eta(&self, systems: usize, constraints: usize) -> Option<f64> {
        if systems == 0 || constraints == 0 {
            return None;
        }
        let (k, s) = (systems as f64, constraints as f64);
        let beta = match self.sampling {
            Sampling::Common => (1.0 - self.confidence) / (k * s),
            Sampling::Independent => (1.0 - powf(self.confidence, 1.0 / k)) / s,
        };
        eta(beta, self.first_stage)
    }

    fn validate(&self, systems: usize, constraints: &[Constraint]) -> Result<(), SelectError> {
        if systems == 0 {
            return Err(SelectError::NoSystems);
        }
        if constraints.is_empty() {
            return Err(SelectError::NoConstraints);
        }
        if !(self.confidence > 0.0 && self.confidence < 1.0) {
            return Err(SelectError::Confidence(self.confidence));
        }
        if self.first_stage < 2 {
            return Err(SelectError::FirstStage(self.first_stage));
        }
        for (index, c) in constraints.iter().enumerate() {
            if !(c.tolerance.is_finite() && c.tolerance > 0.0) || !c.limit.is_finite() {
                return Err(SelectError::Constraint { index });
            }
        }
        Ok(())
    }
}

/// A feasibility decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Feasible,
    Infeasible,
    /// Not settled when the budget ran out (never counted as feasible); for one constraint of a
    /// system, also a constraint not settled before another one made the system infeasible.
    Undecided,
}

/// The answer of [`check_feasibility`].
#[derive(Debug, Clone, PartialEq)]
pub struct FeasibilityCheck {
    /// Each system's decision.
    pub decisions: Vec<Decision>,
    /// Each system's decision on each constraint.
    pub by_constraint: Vec<Vec<Decision>>,
    /// Replications taken from each system.
    pub runs: Vec<u64>,
    /// Each system's mean of each constrained value over its replications.
    pub means: Vec<Vec<f64>>,
    /// Every constrained value was the same in every first-stage replication of every system:
    /// decided exactly on n₀ replications, with no probability stated.
    pub exact: bool,
    /// The probability that every system was decided correctly: `Some(1 − α)` when every system
    /// was decided and the data were random.
    pub confidence: Option<f64>,
}

impl FeasibilityCheck {
    /// The systems declared feasible.
    pub fn feasible(&self) -> Vec<usize> {
        self.decisions
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (*d == Decision::Feasible).then_some(i))
            .collect()
    }

    /// Whether every system was decided before the budget ran out.
    pub fn is_complete(&self) -> bool {
        !self.decisions.contains(&Decision::Undecided)
    }

    pub fn total_runs(&self) -> u64 {
        self.runs.iter().sum()
    }
}

/// The answer of [`select_feasible`].
#[derive(Debug, Clone, PartialEq)]
pub struct ConstrainedSelection {
    pub feasibility: FeasibilityCheck,
    /// KN among the systems declared feasible, on fresh replications; `None` when none was
    /// declared feasible or the check did not finish within the budget.
    pub selection: Option<Selection>,
    /// The probability, covering the feasibility decisions and the selection together, that
    /// the answer claims: `Some(1 − α)` when both were completed on random data.
    pub confidence: Option<f64>,
}

impl ConstrainedSelection {
    /// The chosen system, when the answer is proven (with the stated confidence, or exactly).
    pub fn chosen(&self) -> Option<usize> {
        let s = self.selection.as_ref()?;
        (self.feasibility.is_complete() && s.outcome != Outcome::NotProven).then_some(s.chosen)
    }

    pub fn total_runs(&self) -> u64 {
        self.feasibility.total_runs() + self.selection.as_ref().map_or(0, Selection::total_runs)
    }
}

/// Why a procedure could not run.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectError {
    NoSystems,
    NoConstraints,
    /// δ must be finite and positive.
    Indifference(f64),
    /// The confidence must lie strictly between 1/k (for KN) or 0 and 1.
    Confidence(f64),
    /// n₀ must be at least 2.
    FirstStage(u64),
    /// A constraint's limit must be finite and its tolerance finite and positive.
    Constraint {
        index: usize,
    },
    /// The share of α given to the feasibility check must lie strictly between 0 and 1.
    Share(f64),
    /// The budget cannot pay for the first stage.
    Budget {
        needed: u64,
        max_runs: u64,
    },
    /// The sampler returned a value that is not finite.
    NotFinite {
        system: usize,
        replication: u64,
    },
    /// The sampler returned the wrong number of replications, or a row of the wrong length.
    WrongCount {
        system: usize,
        expected: u64,
        got: u64,
    },
}

impl std::fmt::Display for SelectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectError::NoSystems => write!(f, "there are no systems to compare"),
            SelectError::NoConstraints => write!(f, "there are no constraints to check"),
            SelectError::Indifference(d) => {
                write!(f, "the indifference amount must be positive, not {d}")
            }
            SelectError::Confidence(c) => write!(f, "the confidence {c} is out of range"),
            SelectError::FirstStage(n) => {
                write!(f, "the first stage needs at least 2 replications, not {n}")
            }
            SelectError::Constraint { index } => write!(
                f,
                "constraint {index} needs a finite limit and a positive tolerance"
            ),
            SelectError::Share(s) => write!(
                f,
                "the feasibility check's share of the error must lie between 0 and 1, not {s}"
            ),
            SelectError::Budget { needed, max_runs } => write!(
                f,
                "the first stage needs {needed} replications, more than the {max_runs} allowed"
            ),
            SelectError::NotFinite {
                system,
                replication,
            } => write!(
                f,
                "system {system} gave a value that is not a number in replication {replication}"
            ),
            SelectError::WrongCount {
                system,
                expected,
                got,
            } => write!(
                f,
                "system {system} gave {got} values where {expected} were asked for"
            ),
        }
    }
}

impl std::error::Error for SelectError {}

/// Kim and Nelson's KN: the best of `systems` systems (or one within δ of it), with probability
/// at least 1 − α when the observations are normal, with unknown and unequal variances, and
/// with or without common random numbers.
///
/// The procedure (Tsai and Nelson's Generic Procedure with no controls, which is KN):
/// - *Setup:* λ = δ/2, η = ½ [(2α/(k − 1))^(−2/(n₀ − 1)) − 1] and h² = 2η(n₀ − 1).
/// - *First stage:* n₀ replications from every system; S²ᵢₗ, the sample variance of the
///   differences Xᵢⱼ − Xₗⱼ, for every pair. r = n₀.
/// - *Screening:* system i stays if X̄ᵢ(r) ≥ X̄ₗ(r) − Wᵢₗ(r) for every other system l still in
///   contention, where Wᵢₗ(r) = max{0, h²S²ᵢₗ/(2δr) − λ}.
/// - *Stopping:* one system left: it is chosen. Otherwise one more replication from each, and
///   screen again.
///
/// The region closes when r reaches h²S²ᵢₗ/δ² for every pair left, where every W is 0: the
/// screening then keeps only the largest totals, and the procedure stops there (Kim and
/// Nelson's stopping rule, N = ⌊h²S²/δ²⌋). Exactly equal totals at that point, which happen
/// only without randomness, are all kept as survivors and the lowest index is chosen; their
/// means are equal, so any of them is correct.
pub fn select_best<F>(setup: &Kn, systems: usize, mut sample: F) -> Result<Selection, SelectError>
where
    F: FnMut(usize, Range<u64>) -> Vec<f64>,
{
    setup.validate(systems)?;
    let eta = if systems > 1 {
        setup
            .eta(systems)
            .ok_or(SelectError::Confidence(setup.confidence))?
    } else {
        // One system: chosen after its first stage; η is never used.
        1.0
    };
    let plan = Plan {
        systems: (0..systems).collect(),
        eta,
        first_stage: setup.first_stage,
        delta: setup.delta,
        sign: setup.sense.sign(),
        standard: false,
        start: 0,
        budget: setup.max_runs,
    };
    let run = sequential(&plan, &mut |s, reps| checked(&mut sample, s, reps))?;
    Ok(run.into_selection(&plan, systems, setup.confidence))
}

/// Kim's comparison with a standard, for a standard whose mean is unknown and is simulated:
/// system 0 of the sampler is the standard and systems 1 to `alternatives` the alternatives.
///
/// Kim (2003) guarantees, for the better sense:
/// - the standard is kept with probability at least 1 − α when no alternative is better than it;
/// - the best alternative is chosen with probability at least 1 − α when it is at least δ better
///   than the standard and than every other alternative.
///
/// The procedure is KN's with three changes (Kim's generic procedure, c = 1): the pairs with the
/// standard use δ/2 in place of δ; δ/2 is added to each of the standard's observations; and η
/// comes from β = α/k with common random numbers (Kim's Case C) or 1 − (1 − α)^(1/k) for
/// independent systems (Case B), k the number of alternatives. With aᵢₗ = η(n₀ − 1)S²ᵢₗ/δᵢₗ and
/// λᵢₗ = δᵢₗ/2, system i is eliminated when Σⱼ (X̃ᵢⱼ − X̃ₗⱼ) < min{0, −aᵢₗ + λᵢₗ r}.
///
/// The paper prints that bound as max{0, −aᵢₗ + λᵢₗ r}. Read literally, both systems of a pair
/// would be eliminated once λᵢₗ r > aᵢₗ and their difference was small. The min form is Kim and
/// Nelson's triangular region (r·Wᵢₗ(r) = max{0, aᵢₗ − λᵢₗ r}), which the procedure reduces to
/// when there is no standard, so it is the one used.
pub fn compare_with_standard<F>(
    setup: &Kn,
    alternatives: usize,
    mut sample: F,
) -> Result<Selection, SelectError>
where
    F: FnMut(usize, Range<u64>) -> Vec<f64>,
{
    if alternatives == 0 {
        return Err(SelectError::NoSystems);
    }
    // The confidence only needs to lie in (0, 1) here; η's own range is checked below.
    setup.validate(1)?;
    let k = alternatives as f64;
    let beta = match setup.sampling {
        Sampling::Common => (1.0 - setup.confidence) / k,
        Sampling::Independent => 1.0 - powf(setup.confidence, 1.0 / k),
    };
    let eta = eta(beta, setup.first_stage).ok_or(SelectError::Confidence(setup.confidence))?;
    let systems = alternatives + 1;
    let plan = Plan {
        systems: (0..systems).collect(),
        eta,
        first_stage: setup.first_stage,
        delta: setup.delta,
        sign: setup.sense.sign(),
        standard: true,
        start: 0,
        budget: setup.max_runs,
    };
    let run = sequential(&plan, &mut |s, reps| checked(&mut sample, s, reps))?;
    Ok(run.into_selection(&plan, systems, setup.confidence))
}

/// Batur and Kim's feasibility check F_B for several constraints on the means of each system.
/// `sample` returns, for each replication, one value per constraint (in the constraints'
/// order).
///
/// The procedure (Zhou et al., Algorithm 1, with c = 1): η from β_B = α/(ks) (common random
/// numbers or any dependence) or [1 − (1 − α)^(1/k)]/s (independent systems), h² = 2η(n₀ − 1).
/// After n₀ replications, S²ᵢₗ is the sample variance of system i's value for constraint l. At
/// each step, for each constraint l not yet settled, with
/// R(r) = max{0, h²S²ᵢₗ/(2εₗ) − εₗ r/2}: if Σⱼ (Yᵢₗⱼ − qₗ) ≤ −R(r) the constraint is met; else if
/// it is ≥ R(r) the system is infeasible. A system all of whose constraints are met is feasible.
/// Undecided systems take one more replication each. An "at least" constraint is checked as
/// "at most" on the negated values.
///
/// When the region has closed (R = 0) and the sum is exactly 0, the first test settles it: the
/// mean equals the limit, which is feasible by definition (y ≤ q).
pub fn check_feasibility<F>(
    setup: &Feasibility,
    systems: usize,
    constraints: &[Constraint],
    mut sample: F,
) -> Result<FeasibilityCheck, SelectError>
where
    F: FnMut(usize, Range<u64>) -> Vec<Vec<f64>>,
{
    setup.validate(systems, constraints)?;
    let eta = setup
        .eta(systems, constraints.len())
        .ok_or(SelectError::Confidence(setup.confidence))?;
    let width = constraints.len();
    feasibility(
        &FeasibilityPlan {
            eta,
            first_stage: setup.first_stage,
            budget: setup.max_runs,
            confidence: setup.confidence,
            offset: 0,
        },
        systems,
        constraints,
        &mut |s, reps| checked_rows(&mut sample, s, reps, width),
    )
}

/// The best system among those that meet every constraint: Batur and Kim's check, then KN among
/// the systems it declared feasible. `sample` returns, for each replication, the objective
/// followed by one value per constraint.
///
/// The error α is split by Bonferroni's inequality: the check is run at confidence
/// 1 − `feasibility_share`·α and KN at 1 − (1 − `feasibility_share`)·α (ROADMAP M8: half each by
/// default), so with probability at least 1 − α every system at least ε inside its limits is
/// declared feasible, every one at least ε beyond one of them infeasible, and the chosen system
/// is the best, or within δ of the best, of those declared feasible.
///
/// KN runs on fresh replications, numbered after the last one the check took, so its data are
/// independent of which systems passed and its guarantee holds for the set it is given (its η
/// comes from the number of systems declared feasible). The check's replications of the
/// objective are not reused. The budget `setup.max_runs` covers both stages.
pub fn select_feasible<F>(
    setup: &Kn,
    constraints: &[Constraint],
    feasibility_share: f64,
    systems: usize,
    mut sample: F,
) -> Result<ConstrainedSelection, SelectError>
where
    F: FnMut(usize, Range<u64>) -> Vec<Vec<f64>>,
{
    if !(feasibility_share > 0.0 && feasibility_share < 1.0) {
        return Err(SelectError::Share(feasibility_share));
    }
    if systems == 0 {
        return Err(SelectError::NoSystems);
    }
    // δ, n₀ and a confidence in (0, 1); KN's own range is checked for the systems found feasible.
    setup.validate(1)?;
    let alpha = 1.0 - setup.confidence;
    let check = Feasibility {
        confidence: 1.0 - feasibility_share * alpha,
        first_stage: setup.first_stage,
        sampling: setup.sampling,
        max_runs: setup.max_runs,
    };
    check.validate(systems, constraints)?;
    let eta = check
        .eta(systems, constraints.len())
        .ok_or(SelectError::Confidence(setup.confidence))?;
    let width = constraints.len() + 1;
    let found = feasibility(
        &FeasibilityPlan {
            eta,
            first_stage: setup.first_stage,
            budget: setup.max_runs,
            confidence: check.confidence,
            offset: 1,
        },
        systems,
        constraints,
        &mut |s, reps| checked_rows(&mut sample, s, reps, width),
    )?;
    let feasible = found.feasible();
    if !found.is_complete() || feasible.is_empty() {
        let confidence = found.confidence.map(|_| setup.confidence);
        return Ok(ConstrainedSelection {
            feasibility: found,
            selection: None,
            confidence,
        });
    }
    let kn = Kn {
        confidence: 1.0 - (1.0 - feasibility_share) * alpha,
        max_runs: setup.max_runs.map(|m| m.saturating_sub(found.total_runs())),
        ..*setup
    };
    let m = feasible.len();
    kn.validate(m)?;
    let eta = if m > 1 {
        kn.eta(m).ok_or(SelectError::Confidence(kn.confidence))?
    } else {
        1.0
    };
    let plan = Plan {
        systems: feasible,
        eta,
        first_stage: setup.first_stage,
        delta: setup.delta,
        sign: setup.sense.sign(),
        standard: false,
        start: found.runs.iter().copied().max().unwrap_or(0),
        budget: kn.max_runs,
    };
    let run = sequential(&plan, &mut |s, reps| {
        Ok(checked_rows(&mut sample, s, reps, width)?
            .into_iter()
            .map(|row| row.first().copied().unwrap_or(f64::NAN))
            .collect())
    })?;
    let selection = run.into_selection(&plan, systems, kn.confidence);
    let confidence = match (found.confidence, selection.outcome) {
        (Some(_), Outcome::Selected) => Some(setup.confidence),
        // A feasible set found exactly and a selection with confidence: the selection's.
        (None, Outcome::Selected) if found.exact => Some(kn.confidence),
        (Some(_), Outcome::ComparedExactly) => Some(check.confidence),
        _ => None,
    };
    Ok(ConstrainedSelection {
        feasibility: found,
        selection: Some(selection),
        confidence,
    })
}

/// The sampler's values for `reps` of `system`, checked: one per replication, all finite.
fn checked<F>(sample: &mut F, system: usize, reps: Range<u64>) -> Result<Vec<f64>, SelectError>
where
    F: FnMut(usize, Range<u64>) -> Vec<f64>,
{
    let expected = reps.end - reps.start;
    let values = sample(system, reps.clone());
    if values.len() as u64 != expected {
        return Err(SelectError::WrongCount {
            system,
            expected,
            got: values.len() as u64,
        });
    }
    if let Some(j) = values.iter().position(|v| !v.is_finite()) {
        return Err(SelectError::NotFinite {
            system,
            replication: reps.start + j as u64,
        });
    }
    Ok(values)
}

/// The sampler's rows for `reps` of `system`, checked: one row of `width` finite values per
/// replication.
fn checked_rows<F>(
    sample: &mut F,
    system: usize,
    reps: Range<u64>,
    width: usize,
) -> Result<Vec<Vec<f64>>, SelectError>
where
    F: FnMut(usize, Range<u64>) -> Vec<Vec<f64>>,
{
    let expected = reps.end - reps.start;
    let rows = sample(system, reps.clone());
    if rows.len() as u64 != expected {
        return Err(SelectError::WrongCount {
            system,
            expected,
            got: rows.len() as u64,
        });
    }
    for (j, row) in rows.iter().enumerate() {
        if row.len() != width {
            return Err(SelectError::WrongCount {
                system,
                expected: width as u64,
                got: row.len() as u64,
            });
        }
        if !row.iter().all(|v| v.is_finite()) {
            return Err(SelectError::NotFinite {
                system,
                replication: reps.start + j as u64,
            });
        }
    }
    Ok(rows)
}

/// One run of the fully sequential selection shared by KN and the comparison with a standard.
struct Plan {
    /// The sampler's index of each system taking part, by position.
    systems: Vec<usize>,
    eta: f64,
    first_stage: u64,
    delta: f64,
    /// +1 to maximise, −1 to minimise.
    sign: f64,
    /// Position 0 is a standard (Kim 2003): δ/2 for its pairs, and δ/2 added to its
    /// observations.
    standard: bool,
    /// The first replication's index.
    start: u64,
    budget: Option<u64>,
}

/// What [`sequential`] found, by position.
struct Found {
    chosen: usize,
    survivors: Vec<usize>,
    runs: Vec<u64>,
    means: Vec<f64>,
    exact: bool,
    proven: bool,
}

impl Found {
    fn into_selection(self, plan: &Plan, systems: usize, confidence: f64) -> Selection {
        let mut runs = vec![0; systems];
        let mut means = vec![None; systems];
        for (p, &system) in plan.systems.iter().enumerate() {
            if let (Some(r), Some(m)) = (runs.get_mut(system), means.get_mut(system)) {
                *r = self.runs.get(p).copied().unwrap_or(0);
                *m = self.means.get(p).copied();
            }
        }
        let index = |p: usize| plan.systems.get(p).copied().unwrap_or(p);
        let outcome = if self.exact {
            Outcome::ComparedExactly
        } else if self.proven {
            Outcome::Selected
        } else {
            Outcome::NotProven
        };
        Selection {
            outcome,
            chosen: index(self.chosen),
            survivors: self.survivors.iter().map(|&p| index(p)).collect(),
            runs,
            means,
            confidence: (outcome == Outcome::Selected).then_some(confidence),
        }
    }
}

/// The sample variance of `values` (n ≥ 2).
fn variance(values: impl Iterator<Item = f64> + Clone) -> f64 {
    let (n, sum) = values
        .clone()
        .fold((0.0, 0.0), |(n, s), x| (n + 1.0, s + x));
    let mean = sum / n;
    values
        .map(|x| {
            let d = x - mean;
            d * d
        })
        .sum::<f64>()
        / (n - 1.0)
}

/// Whether every value equals the first.
fn constant(values: &[f64]) -> bool {
    values
        .first()
        .is_none_or(|v0| values.iter().all(|v| v == v0))
}

fn sequential<F>(plan: &Plan, sample: &mut F) -> Result<Found, SelectError>
where
    F: FnMut(usize, Range<u64>) -> Result<Vec<f64>, SelectError>,
{
    let m = plan.systems.len();
    let n0 = plan.first_stage;
    let needed = n0.saturating_mul(m as u64);
    if let Some(max_runs) = plan.budget
        && needed > max_runs
    {
        return Err(SelectError::Budget { needed, max_runs });
    }
    let bonus = |p: usize| {
        if plan.standard && p == 0 {
            plan.delta / 2.0
        } else {
            0.0
        }
    };
    let pair_delta = |p: usize, q: usize| {
        if plan.standard && (p == 0 || q == 0) {
            plan.delta / 2.0
        } else {
            plan.delta
        }
    };

    // First stage.
    let mut first: Vec<Vec<f64>> = Vec::with_capacity(m);
    let mut raw = vec![0.0; m];
    let mut sums = vec![0.0; m];
    for (p, &system) in plan.systems.iter().enumerate() {
        let values = sample(system, plan.start..plan.start + n0)?;
        let adjusted: Vec<f64> = values.iter().map(|x| plan.sign * x + bonus(p)).collect();
        raw[p] = values.iter().sum();
        sums[p] = adjusted.iter().sum();
        first.push(adjusted);
    }
    let exact = first.iter().all(|v| constant(v));
    let mut runs = vec![n0; m];
    if m == 1 {
        return Ok(Found {
            chosen: 0,
            survivors: vec![0],
            means: raw.iter().map(|s| s / n0 as f64).collect(),
            runs,
            exact,
            proven: true,
        });
    }
    let mut s2 = vec![0.0; m * m];
    for (p, a) in first.iter().enumerate() {
        for (q, b) in first.iter().enumerate().skip(p + 1) {
            let differences: Vec<f64> = a.iter().zip(b).map(|(x, y)| x - y).collect();
            let v = if constant(&differences) {
                0.0
            } else {
                variance(differences.iter().copied())
            };
            s2[p * m + q] = v;
            s2[q * m + p] = v;
        }
    }
    // r·Wₚq(r) = max{0, aₚq − λₚq r}: the half-width of the region for the pair's sum of
    // differences at stage r.
    let width = |p: usize, q: usize, r: u64| {
        let d = pair_delta(p, q);
        let a = plan.eta * (n0 as f64 - 1.0) * s2[p * m + q] / d;
        (a - d / 2.0 * r as f64).max(0.0)
    };
    let best_of = |active: &[usize], sums: &[f64]| {
        active
            .iter()
            .copied()
            .fold(None, |best: Option<usize>, p| match best {
                Some(b) if sums[b] >= sums[p] => Some(b),
                _ => Some(p),
            })
            .unwrap_or(0)
    };

    let mut active: Vec<usize> = (0..m).collect();
    let mut r = n0;
    let mut total = needed;
    loop {
        let old = active.clone();
        active.retain(|&p| {
            old.iter()
                .all(|&q| q == p || sums[p] - sums[q] >= -width(p, q, r))
        });
        if active.is_empty() {
            // Cannot happen (the largest total always stays), kept for safety.
            active = old;
        }
        let closed = active
            .iter()
            .all(|&p| active.iter().all(|&q| q == p || width(p, q, r) == 0.0));
        if active.len() == 1 || closed {
            return Ok(Found {
                chosen: best_of(&active, &sums),
                survivors: active,
                means: raw.iter().zip(&runs).map(|(s, n)| s / *n as f64).collect(),
                runs,
                exact,
                proven: true,
            });
        }
        let step = active.len() as u64;
        if plan.budget.is_some_and(|max_runs| total + step > max_runs) {
            return Ok(Found {
                chosen: best_of(&active, &sums),
                survivors: active,
                means: raw.iter().zip(&runs).map(|(s, n)| s / *n as f64).collect(),
                runs,
                exact,
                proven: false,
            });
        }
        for &p in &active {
            let rep = plan.start + r;
            let values = sample(plan.systems[p], rep..rep + 1)?;
            let x = values.first().copied().unwrap_or(f64::NAN);
            raw[p] += x;
            sums[p] += plan.sign * x + bonus(p);
            runs[p] += 1;
        }
        total += step;
        r += 1;
    }
}

struct FeasibilityPlan {
    eta: f64,
    first_stage: u64,
    budget: Option<u64>,
    confidence: f64,
    /// Values in each row before the constraints' (1 when the objective comes first).
    offset: usize,
}

fn feasibility<F>(
    plan: &FeasibilityPlan,
    systems: usize,
    constraints: &[Constraint],
    sample: &mut F,
) -> Result<FeasibilityCheck, SelectError>
where
    F: FnMut(usize, Range<u64>) -> Result<Vec<Vec<f64>>, SelectError>,
{
    let s = constraints.len();
    let n0 = plan.first_stage;
    let needed = n0.saturating_mul(systems as u64);
    if let Some(max_runs) = plan.budget
        && needed > max_runs
    {
        return Err(SelectError::Budget { needed, max_runs });
    }
    let h2 = 2.0 * plan.eta * (n0 as f64 - 1.0);
    let value = |row: &[f64], l: usize| row.get(plan.offset + l).copied().unwrap_or(f64::NAN);

    /// One system's state, by constraint.
    struct State {
        decision: Decision,
        by_constraint: Vec<Decision>,
        runs: u64,
        /// Σⱼ Yᵢₗⱼ, for the means.
        raw: Vec<f64>,
        /// Σⱼ (Yᵢₗⱼ − qₗ) on the "at most" scale.
        sums: Vec<f64>,
        /// S²ᵢₗ from the first stage.
        s2: Vec<f64>,
    }

    let mut states = Vec::with_capacity(systems);
    let mut exact = true;
    for i in 0..systems {
        let rows = sample(i, 0..n0)?;
        let mut state = State {
            decision: Decision::Undecided,
            by_constraint: vec![Decision::Undecided; s],
            runs: n0,
            raw: Vec::with_capacity(s),
            sums: Vec::with_capacity(s),
            s2: Vec::with_capacity(s),
        };
        for (l, c) in constraints.iter().enumerate() {
            let column: Vec<f64> = rows.iter().map(|row| value(row, l)).collect();
            state.raw.push(column.iter().sum());
            state
                .sums
                .push(column.iter().map(|y| c.bound.sign() * (y - c.limit)).sum());
            if constant(&column) {
                state.s2.push(0.0);
            } else {
                exact = false;
                state.s2.push(variance(column.iter().copied()));
            }
        }
        states.push(state);
    }
    let mut r = n0;
    let mut total = needed;
    loop {
        for state in states
            .iter_mut()
            .filter(|st| st.decision == Decision::Undecided)
        {
            for (l, c) in constraints.iter().enumerate() {
                if state.by_constraint[l] != Decision::Undecided {
                    continue;
                }
                let eps = c.tolerance;
                let bound = (h2 * state.s2[l] / (2.0 * eps) - eps * r as f64 / 2.0).max(0.0);
                if state.sums[l] <= -bound {
                    state.by_constraint[l] = Decision::Feasible;
                } else if state.sums[l] >= bound {
                    state.by_constraint[l] = Decision::Infeasible;
                    state.decision = Decision::Infeasible;
                    break;
                }
            }
            if state.by_constraint.iter().all(|d| *d == Decision::Feasible) {
                state.decision = Decision::Feasible;
            }
        }
        let open: Vec<usize> = states
            .iter()
            .enumerate()
            .filter_map(|(i, st)| (st.decision == Decision::Undecided).then_some(i))
            .collect();
        let step = open.len() as u64;
        if open.is_empty() || plan.budget.is_some_and(|max_runs| total + step > max_runs) {
            let complete = open.is_empty();
            return Ok(FeasibilityCheck {
                decisions: states.iter().map(|st| st.decision).collect(),
                means: states
                    .iter()
                    .map(|st| st.raw.iter().map(|x| x / st.runs as f64).collect())
                    .collect(),
                runs: states.iter().map(|st| st.runs).collect(),
                by_constraint: states.into_iter().map(|st| st.by_constraint).collect(),
                exact,
                confidence: (complete && !exact).then_some(plan.confidence),
            });
        }
        for i in open {
            let rows = sample(i, r..r + 1)?;
            let row = rows.first().map(Vec::as_slice).unwrap_or(&[]);
            let Some(state) = states.get_mut(i) else {
                continue;
            };
            for (l, c) in constraints.iter().enumerate() {
                let y = value(row, l);
                state.raw[l] += y;
                state.sums[l] += c.bound.sign() * (y - c.limit);
            }
            state.runs += 1;
        }
        total += step;
        r += 1;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use stochcraft_random::math::{cos, ln};
    use stochcraft_random::{Rng, SplitMix64};

    /// A standard normal keyed by (seed, a, b), so it does not depend on the order of calls.
    fn normal(seed: u64, a: u64, b: u64) -> f64 {
        let mut sm = SplitMix64::new(
            seed ^ a.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ b.wrapping_mul(0xd1b5_4a32_d192_ed03),
        );
        let mut rng = Rng::new(sm.next_u64());
        let u1 = rng.uniform_open0();
        let u2 = rng.uniform();
        (-2.0 * ln(u1)).sqrt() * cos(2.0 * std::f64::consts::PI * u2)
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    /// Systems with the given means and standard deviation 1, independent.
    fn normals(seed: u64, means: Vec<f64>) -> impl FnMut(usize, Range<u64>) -> Vec<f64> {
        move |s, reps| reps.map(|j| means[s] + normal(seed, s as u64, j)).collect()
    }

    #[test]
    fn eta_and_h2_from_the_formula_by_hand() {
        // η = ½ [(2β)^(−2/(n₀ − 1)) − 1], h² = 2η(n₀ − 1).
        // k = 3, n₀ = 2, α = 0.05: β = 0.025, (0.05)^(−2) = 400, η = 199.5, h² = 399.
        let kn = Kn {
            first_stage: 2,
            ..Kn::new(1.0, Sense::Maximise)
        };
        assert!(close(kn.eta(3).unwrap_or(0.0), 199.5));
        assert!(close(kn.h2(3).unwrap_or(0.0), 399.0));
        // k = 2, n₀ = 3, α = 0.05: β = 0.05, (0.1)^(−1) = 10, η = 4.5, h² = 18.
        let kn = Kn {
            first_stage: 3,
            ..kn
        };
        assert!(close(kn.eta(2).unwrap_or(0.0), 4.5));
        assert!(close(kn.h2(2).unwrap_or(0.0), 18.0));
        // k = 10, n₀ = 10, α = 0.05: β = 0.05/9, (1/90)^(−2/9) = 2.7181668598…,
        // η = 0.85908342992…, h² = 18η = 15.4635017386….
        let kn = Kn {
            first_stage: 10,
            ..kn
        };
        assert!(close(kn.eta(10).unwrap_or(0.0), 0.859_083_429_924_899_1));
        assert!(close(kn.h2(10).unwrap_or(0.0), 15.463_501_738_648_183));
        // Independent systems (Tsai and Nelson's Remark 4.1): β = 1 − 0.95^(1/9).
        let independent = Kn {
            sampling: Sampling::Independent,
            ..kn
        };
        assert!(close(
            independent.eta(10).unwrap_or(0.0),
            0.852_248_252_415_315_9
        ));
        // Batur and Kim, k = 4 systems, s = 2 constraints, n₀ = 10, α = 0.05: β = 0.05/8.
        let f = Feasibility::default();
        assert!(close(f.eta(4, 2).unwrap_or(0.0), 0.823_972_273_770_045_9));
        let f = Feasibility {
            sampling: Sampling::Independent,
            ..f
        };
        assert!(close(f.eta(4, 2).unwrap_or(0.0), 0.818_355_218_258_613_6));
        // Out of range: 2β ≥ 1, or n₀ < 2.
        assert_eq!(eta(0.5, 10), None);
        assert_eq!(eta(0.01, 1), None);
        assert_eq!(kn.eta(1), None);
    }

    #[test]
    fn no_randomness_is_compared_exactly_on_the_first_stage() {
        let setup = Kn::new(0.5, Sense::Minimise);
        let means = [3.0, 1.0, 2.0, 1.5];
        let s = select_best(&setup, 4, |s, reps| reps.map(|_| means[s]).collect());
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(s.outcome, Outcome::ComparedExactly);
        assert_eq!(s.chosen, 1);
        assert_eq!(s.survivors, vec![1]);
        assert_eq!(s.runs, vec![10; 4]);
        assert_eq!(s.confidence, None);
        assert_eq!(s.means[2], Some(2.0));
        // Exact ties: all of them survive and the lowest index is chosen.
        let means = [1.0, 2.0, 2.0];
        let s = select_best(&Kn::new(0.5, Sense::Maximise), 3, |s, reps| {
            reps.map(|_| means[s]).collect()
        });
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!((s.chosen, s.survivors.clone()), (1, vec![1, 2]));
        assert_eq!(s.total_runs(), 30);
    }

    #[test]
    fn a_clear_winner_is_chosen_and_the_sense_is_respected() {
        let means = vec![0.0, 3.0, 0.5, 1.0];
        let s = select_best(&Kn::new(0.5, Sense::Maximise), 4, normals(7, means.clone()));
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!((s.outcome, s.chosen), (Outcome::Selected, 1));
        assert_eq!(s.confidence, Some(0.95));
        // The same data negated, minimised: the same choice and the same runs.
        let neg = select_best(&Kn::new(0.5, Sense::Minimise), 4, {
            let mut f = normals(7, means);
            move |s, reps| f(s, reps).into_iter().map(|x| -x).collect()
        });
        let Ok(neg) = neg else { panic!("{neg:?}") };
        assert_eq!((neg.chosen, &neg.runs), (1, &s.runs));
    }

    #[test]
    fn the_same_seed_gives_the_same_answer() {
        let setup = Kn::new(0.3, Sense::Maximise);
        let run = || select_best(&setup, 5, normals(11, vec![0.0, 0.3, 0.1, 0.2, 0.0]));
        let (a, b) = (run(), run());
        assert!(a.is_ok());
        assert_eq!(a, b);
        let constraints = [Constraint {
            bound: Bound::AtMost,
            limit: 0.0,
            tolerance: 0.2,
        }];
        let rows = |seed: u64| {
            move |s: usize, reps: Range<u64>| -> Vec<Vec<f64>> {
                reps.map(|j| {
                    vec![
                        s as f64 + normal(seed, s as u64, j),
                        s as f64 * 0.1 - 0.15 + normal(seed + 1, s as u64, j),
                    ]
                })
                .collect()
            }
        };
        let a = select_feasible(&setup, &constraints, 0.5, 4, rows(3));
        let b = select_feasible(&setup, &constraints, 0.5, 4, rows(3));
        assert!(a.is_ok());
        assert_eq!(a, b);
    }

    #[test]
    fn a_budget_that_runs_out_says_not_proven() {
        let setup = Kn {
            max_runs: Some(80),
            ..Kn::new(0.01, Sense::Maximise)
        };
        let s = select_best(&setup, 5, normals(5, vec![0.0, 0.01, 0.0, 0.0, 0.0]));
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(s.outcome, Outcome::NotProven);
        assert_eq!(s.confidence, None);
        assert!(s.survivors.len() > 1, "{:?}", s.survivors);
        assert!(s.survivors.contains(&s.chosen));
        assert!(s.total_runs() <= 80, "{}", s.total_runs());
        // Too small for the first stage.
        let setup = Kn {
            max_runs: Some(49),
            ..setup
        };
        let e = select_best(&setup, 5, normals(5, vec![0.0; 5]));
        assert_eq!(
            e,
            Err(SelectError::Budget {
                needed: 50,
                max_runs: 49
            })
        );
    }

    #[test]
    fn bad_settings_and_bad_samples_are_refused() {
        let ok = Kn::new(1.0, Sense::Maximise);
        let zeros = |_: usize, reps: Range<u64>| reps.map(|_| 0.0).collect::<Vec<f64>>();
        assert_eq!(select_best(&ok, 0, zeros), Err(SelectError::NoSystems));
        let bad = Kn::new(0.0, Sense::Maximise);
        assert_eq!(
            select_best(&bad, 2, zeros),
            Err(SelectError::Indifference(0.0))
        );
        // 1 − α must exceed 1/k.
        let bad = Kn {
            confidence: 0.2,
            ..ok
        };
        assert_eq!(
            select_best(&bad, 5, zeros),
            Err(SelectError::Confidence(0.2))
        );
        let bad = Kn {
            first_stage: 1,
            ..ok
        };
        assert_eq!(select_best(&bad, 2, zeros), Err(SelectError::FirstStage(1)));
        let short = |_: usize, _: Range<u64>| vec![0.0];
        assert_eq!(
            select_best(&ok, 2, short),
            Err(SelectError::WrongCount {
                system: 0,
                expected: 10,
                got: 1
            })
        );
        let nan = |s: usize, reps: Range<u64>| {
            reps.map(|j| if s == 1 && j == 3 { f64::NAN } else { 0.0 })
                .collect::<Vec<f64>>()
        };
        assert_eq!(
            select_best(&ok, 2, nan),
            Err(SelectError::NotFinite {
                system: 1,
                replication: 3
            })
        );
        let constraint = Constraint {
            bound: Bound::AtMost,
            limit: 0.0,
            tolerance: 0.0,
        };
        let rows = |_: usize, reps: Range<u64>| reps.map(|_| vec![0.0]).collect::<Vec<_>>();
        assert_eq!(
            check_feasibility(&Feasibility::default(), 2, &[constraint], rows),
            Err(SelectError::Constraint { index: 0 })
        );
        assert_eq!(
            check_feasibility(&Feasibility::default(), 2, &[], rows),
            Err(SelectError::NoConstraints)
        );
        assert_eq!(
            select_feasible(&ok, &[constraint], 1.0, 2, rows),
            Err(SelectError::Share(1.0))
        );
    }

    #[test]
    fn feasibility_without_randomness_is_decided_on_the_first_stage() {
        let constraints = [
            Constraint {
                bound: Bound::AtMost,
                limit: 5.0,
                tolerance: 0.1,
            },
            Constraint {
                bound: Bound::AtLeast,
                limit: 2.0,
                tolerance: 0.1,
            },
        ];
        // System 0 meets both, 1 is over the first limit, 2 under the second, 3 sits exactly on
        // both limits (feasible by definition).
        let values = [[4.0, 3.0], [6.0, 3.0], [4.0, 1.0], [5.0, 2.0]];
        let check = check_feasibility(&Feasibility::default(), 4, &constraints, |s, reps| {
            reps.map(|_| values[s].to_vec()).collect()
        });
        let Ok(check) = check else {
            panic!("{check:?}")
        };
        use Decision::*;
        assert_eq!(
            check.decisions,
            vec![Feasible, Infeasible, Infeasible, Feasible]
        );
        assert!(check.exact);
        assert_eq!(check.confidence, None);
        assert_eq!(check.runs, vec![10; 4]);
        assert_eq!(check.by_constraint[2], vec![Feasible, Infeasible]);
        assert_eq!(check.feasible(), vec![0, 3]);
    }

    #[test]
    fn the_best_feasible_system_is_chosen() {
        // System 3 has the best objective but breaks the constraint (mean 1 against at most 0);
        // of the feasible ones (0, 1, 2; mean −1), system 2 is best by 1.
        let objective = [0.0, 0.5, 1.5, 5.0];
        let constrained = [-1.0, -1.0, -1.0, 1.0];
        let constraints = [Constraint {
            bound: Bound::AtMost,
            limit: 0.0,
            tolerance: 0.5,
        }];
        let s = select_feasible(
            &Kn::new(0.5, Sense::Maximise),
            &constraints,
            0.5,
            4,
            |s, reps| {
                reps.map(|j| {
                    vec![
                        objective[s] + normal(1, s as u64, j),
                        constrained[s] + normal(2, s as u64, j),
                    ]
                })
                .collect()
            },
        );
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(s.feasibility.feasible(), vec![0, 1, 2]);
        assert_eq!(s.chosen(), Some(2));
        assert_eq!(s.confidence, Some(0.95));
        let Some(kn) = &s.selection else {
            panic!("no selection")
        };
        // KN took nothing from the infeasible system, and its replications came after the
        // check's.
        assert_eq!(kn.runs[3], 0);
        assert_eq!(kn.means[3], None);
        assert!(kn.confidence.is_some_and(|c| close(c, 0.975)));
        assert_eq!(s.total_runs(), s.feasibility.total_runs() + kn.total_runs());
    }

    #[test]
    fn the_standard_is_kept_unless_an_alternative_is_better() {
        let setup = Kn::new(0.5, Sense::Minimise);
        // Without randomness: the standard (0) at 2, alternatives at 3, 2.2 and 2.5.
        let means = [2.0, 3.0, 2.2, 2.5];
        let s = compare_with_standard(&setup, 3, |s, reps| reps.map(|_| means[s]).collect());
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!((s.outcome, s.chosen), (Outcome::ComparedExactly, 0));
        // An alternative clearly better (lower) than the standard is chosen.
        let means = vec![2.0, 3.0, 0.5, 2.5];
        let s = compare_with_standard(&setup, 3, normals(4, means));
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!((s.outcome, s.chosen), (Outcome::Selected, 2));
        // All alternatives clearly worse: the standard is kept.
        let means = vec![0.0, 3.0, 2.0, 2.5];
        let s = compare_with_standard(&setup, 3, normals(4, means));
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!((s.outcome, s.chosen), (Outcome::Selected, 0));
        assert_eq!(
            compare_with_standard(&setup, 0, normals(4, vec![0.0])),
            Err(SelectError::NoSystems)
        );
    }
}
