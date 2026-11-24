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
//!
//! **A flat first stage is not proof of no randomness.** n₀ equal values can come from a system
//! that does vary: one that returns 1 with probability 1% and 0 otherwise gives ten zeros with
//! probability 0.99¹⁰ ≈ 90%. Under common random numbers two varying systems can also give the
//! same difference in every first-stage replication. So a zero first-stage variance (of a pair's
//! differences for KN and the comparison, of a constrained value for the check) never by itself
//! gives an exact or confident answer. For a pair, a spread within rounding of the values (at
//! most 64 units in the last place of the largest, as when B = A + 0.1 is computed in floating
//! point) counts as zero:
//! - Unless the caller states that the systems have no randomness ([`Kn::deterministic`],
//!   [`Feasibility::deterministic`]), such a pair is never eliminated by the test and such a
//!   constraint is never settled by it. The procedures fix S² from the first stage, and their
//!   guarantee is for that fixed S², so it is not estimated again from later replications: a
//!   zero S² stays zero, the pair's or the constraint's region never opens, and more replications
//!   cannot settle it. Declining to eliminate or to decide only removes ways of being wrong, so
//!   the guarantee for every other pair and constraint is kept. A system still in contention
//!   with such a pair cannot be declared best: once every other pair's region has closed (or the
//!   budget runs out) the answer is [`Outcome::NotProven`], with [`Selection::flat`] set. A
//!   system left with only such constraints takes no more replications and stays
//!   [`Decision::Undecided`], with [`FeasibilityCheck::flat`] set.
//! - With the flag, a zero variance is taken as no randomness: the pair is compared, or the
//!   constraint decided, exactly on the first stage, and when every first-stage value of every
//!   system is the same the answer is [`Outcome::ComparedExactly`], with no probability stated.
//!   Values that vary are handled as usual.
//!
//! **Values near the ends of the `f64` range.** When the first stage's values (with δ, or with a
//! constraint's limit and ε) are larger than 1e150 or smaller than 1e-150 in size, everything is
//! computed on them times a power of two that brings them near 1, which changes no decision and
//! multiplies exactly; the means are given back in the values' unit. A sum, a difference, a
//! variance or a bound of the region that is still not finite (later values far larger than the
//! first stage's) is refused with [`SelectError::Overflow`] rather than compared.

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
    /// The caller states that the systems have no randomness (default `false`). Only then is a
    /// zero first-stage variance taken as exact, and [`Outcome::ComparedExactly`] possible;
    /// otherwise a pair with a zero variance of differences is left undecided (see the module's
    /// notes). Equal values in a first stage are never proof of it on their own.
    pub deterministic: bool,
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
            deterministic: false,
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
    /// The caller stated that the systems have no randomness ([`Kn::deterministic`]) and every
    /// system gave the same value in every first-stage replication: the systems were compared
    /// exactly on n₀ replications, and no probability is stated. Never given without the
    /// statement.
    ComparedExactly,
    /// Not settled: the budget ran out first, or a pair still in contention had a zero
    /// first-stage variance of differences without [`Kn::deterministic`] ([`Selection::flat`]).
    /// The chosen system is the best-looking of those still in contention, not proven best.
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
    /// Two systems still in contention gave the same difference in every first-stage
    /// replication, and the systems were not stated to be deterministic: the test cannot decide
    /// that pair, so the answer is [`Outcome::NotProven`] whatever the budget.
    pub flat: bool,
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
    /// The caller states that the systems have no randomness (default `false`): only then is a
    /// constrained value with a zero first-stage variance decided exactly. Otherwise that
    /// constraint is left undecided (see the module's notes).
    pub deterministic: bool,
}

impl Default for Feasibility {
    fn default() -> Self {
        Self {
            confidence: 0.95,
            first_stage: 10,
            sampling: Sampling::Common,
            max_runs: None,
            deterministic: false,
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
    /// The caller stated that the systems have no randomness ([`Feasibility::deterministic`])
    /// and every constrained value was the same in every first-stage replication of every
    /// system: decided exactly on n₀ replications, with no probability stated.
    pub exact: bool,
    /// The probability that every system was decided correctly: `Some(1 − α)` when every system
    /// was decided and the data were random.
    pub confidence: Option<f64>,
    /// A system left undecided has a constraint whose value was the same in every first-stage
    /// replication, and the systems were not stated to be deterministic: the check cannot
    /// settle that constraint, so the system stays [`Decision::Undecided`] whatever the budget.
    pub flat: bool,
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
    /// declared feasible, the check did not finish within the budget, or what the check left of
    /// the budget cannot pay for KN's first stage (then `confidence` is `None`: not proven).
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
    /// A quantity the procedure computes from finite values is not finite (it overflowed): what
    /// it was (a sum, a difference, a variance or a bound of the region, in words).
    Overflow(&'static str),
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
            SelectError::Overflow(what) => write!(
                f,
                "{what} is too large to compute (it overflowed); rescale the values"
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
/// screening then keeps only the largest totals, and the procedure stops there, where the
/// region closes (every W = 0). The papers' rule, one system left, then holds unless several
/// totals are exactly equal, and with W = 0 for good, more replications could not separate them
/// by the region. Exactly equal totals at that point, which happen
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
        deterministic: setup.deterministic,
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
        deterministic: setup.deterministic,
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
            deterministic: setup.deterministic,
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
///
/// [`Kn::deterministic`] here states that the objective has no randomness (as when it is made of
/// known costs alone): KN then compares it exactly. The constrained values are always treated as
/// random, so a constraint whose value is the same in every first-stage replication is left
/// undecided; for constraints with no randomness, run [`check_feasibility`] with
/// [`Feasibility::deterministic`] and then [`select_best`].
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
        // `setup.deterministic` speaks for the objective only: the constrained values are
        // always treated as random here.
        deterministic: false,
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
            deterministic: false,
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
    if kn
        .max_runs
        .is_some_and(|left| left < setup.first_stage.saturating_mul(m as u64))
    {
        // The check finished, but KN's first stage does not fit in what is left: not proven,
        // with the feasibility decisions kept.
        return Ok(ConstrainedSelection {
            feasibility: found,
            selection: None,
            confidence: None,
        });
    }
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
        deterministic: setup.deterministic,
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
    /// The caller states that there is no randomness: a zero S² is exact.
    deterministic: bool,
}

/// What [`sequential`] found, by position.
struct Found {
    chosen: usize,
    survivors: Vec<usize>,
    runs: Vec<u64>,
    means: Vec<f64>,
    exact: bool,
    proven: bool,
    flat: bool,
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
            flat: self.flat,
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

/// `value`, or [`SelectError::Overflow`] naming `what` when it is not finite.
fn finite(value: f64, what: &'static str) -> Result<f64, SelectError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(SelectError::Overflow(what))
    }
}

/// A spread of differences at most this many units in the last place of the values involved
/// (64 ε times their largest magnitude) is indistinguishable from rounding: two systems that
/// differ by a constant, each computed in floating point, give differences that vary in their
/// last bits. Such a spread is treated as zero.
pub(crate) const ROUNDING: f64 = 64.0 * f64::EPSILON;

/// Whether a spread `sd` of differences of values no larger than `magnitude` is zero or
/// indistinguishable from rounding.
pub(crate) fn negligible(sd: f64, magnitude: f64) -> bool {
    sd == 0.0 || sd <= ROUNDING * magnitude
}

/// A power of two that brings `big`, the largest magnitude in play, into (0.5, 1] when it lies
/// outside [1e-150, 1e150], and 1 otherwise. The procedures' decisions are unchanged when the
/// values and δ (or a constraint's values, limit and ε) are all multiplied by the same factor,
/// and a power of two multiplies exactly (away from subnormal results), so values near the ends
/// of the `f64` range are compared without their sums or squares overflowing or underflowing,
/// and values in an ordinary range give exactly what they gave without it.
fn scale_for(big: f64) -> f64 {
    if !(big.is_finite() && big > 0.0) || (1e-150..=1e150).contains(&big) {
        return 1.0;
    }
    let mut s = 1.0_f64;
    while big * s > 1.0 {
        s *= 0.5;
    }
    while big * s <= 0.5 && (s * 2.0).is_finite() {
        s *= 2.0;
    }
    s
}

const SUM: &str = "a sum of the values";
const DIFFERENCE: &str = "a difference of two systems' values";
const VARIANCE: &str = "a variance";
const REGION: &str = "a bound of the continuation region";

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
    // First stage.
    let mut stage: Vec<Vec<f64>> = Vec::with_capacity(m);
    for &system in &plan.systems {
        stage.push(sample(system, plan.start..plan.start + n0)?);
    }
    // Everything is computed on the values and δ times `scale` (see [`scale_for`]); the means
    // are divided back.
    let scale = scale_for(
        stage
            .iter()
            .flatten()
            .fold(plan.delta, |big, x| big.max(x.abs())),
    );
    let delta = plan.delta * scale;
    let bonus = |p: usize| {
        if plan.standard && p == 0 {
            delta / 2.0
        } else {
            0.0
        }
    };
    let pair_delta = |p: usize, q: usize| {
        if plan.standard && (p == 0 || q == 0) {
            delta / 2.0
        } else {
            delta
        }
    };
    let mut first: Vec<Vec<f64>> = Vec::with_capacity(m);
    let mut raw = vec![0.0; m];
    let mut sums = vec![0.0; m];
    for (p, values) in stage.iter().enumerate() {
        let scaled: Vec<f64> = values.iter().map(|x| x * scale).collect();
        let adjusted: Vec<f64> = scaled.iter().map(|x| plan.sign * x + bonus(p)).collect();
        raw[p] = finite(scaled.iter().sum(), SUM)?;
        sums[p] = finite(adjusted.iter().sum(), SUM)?;
        first.push(adjusted);
    }
    let mean = |s: f64, n: u64| s / n as f64 / scale;
    // Exact only on the caller's statement that there is no randomness: equal first-stage
    // values alone are not proof of it.
    let exact = plan.deterministic && first.iter().all(|v| constant(v));
    let mut runs = vec![n0; m];
    if m == 1 {
        return Ok(Found {
            chosen: 0,
            survivors: vec![0],
            means: raw.iter().map(|&s| mean(s, n0)).collect(),
            runs,
            exact,
            proven: true,
            flat: false,
        });
    }
    // aₚq = η(n₀ − 1)S²ₚq/δₚq, the region's half-width at r = 0, and whether the pair is flat:
    // a zero S² (or one indistinguishable from rounding, see [`ROUNDING`]) without the caller's
    // statement that there is no randomness. KN's guarantee is for S² fixed from the first
    // stage, so a zero S² is not estimated again later; the test never eliminates either system
    // of a flat pair, and a flat pair still in contention leaves the answer not proven (see the
    // module's notes).
    let mut a = vec![0.0; m * m];
    let mut flat = vec![false; m * m];
    for (p, x) in first.iter().enumerate() {
        for (q, y) in first.iter().enumerate().skip(p + 1) {
            let differences: Vec<f64> = x.iter().zip(y).map(|(u, v)| u - v).collect();
            if differences.iter().any(|d| !d.is_finite()) {
                return Err(SelectError::Overflow(DIFFERENCE));
            }
            let v = if constant(&differences) {
                0.0
            } else {
                finite(variance(differences.iter().copied()), VARIANCE)?
            };
            let d = pair_delta(p, q);
            let apq = finite(plan.eta * (n0 as f64 - 1.0) * v / d, REGION)?;
            let magnitude = x.iter().chain(y).fold(0.0_f64, |b, u| b.max(u.abs()));
            let f = !plan.deterministic && negligible(v.sqrt(), magnitude);
            a[p * m + q] = apq;
            a[q * m + p] = apq;
            flat[p * m + q] = f;
            flat[q * m + p] = f;
        }
    }
    // r·Wₚq(r) = max{0, aₚq − λₚq r}: the half-width of the region for the pair's sum of
    // differences at stage r.
    let width =
        |p: usize, q: usize, r: u64| (a[p * m + q] - pair_delta(p, q) / 2.0 * r as f64).max(0.0);
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
    let pairs = |active: &[usize]| -> Vec<(usize, usize)> {
        let mut all = Vec::new();
        for (i, &p) in active.iter().enumerate() {
            for &q in active.iter().skip(i + 1) {
                all.push((p, q));
            }
        }
        all
    };

    let mut active: Vec<usize> = (0..m).collect();
    let mut r = n0;
    let mut total = needed;
    loop {
        for (p, q) in pairs(&active) {
            finite(sums[p] - sums[q], DIFFERENCE)?;
        }
        let old = active.clone();
        active.retain(|&p| {
            old.iter()
                .all(|&q| q == p || flat[p * m + q] || sums[p] - sums[q] >= -width(p, q, r))
        });
        if active.is_empty() {
            // Cannot happen (the largest total always stays), kept for safety.
            active = old;
        }
        let left = pairs(&active);
        let stuck = left.iter().any(|&(p, q)| flat[p * m + q]);
        // Every pair left whose test can still decide it has closed. A flat pair cannot be
        // decided by more replications (its S² stays zero), so sampling stops there too.
        let closed = left
            .iter()
            .all(|&(p, q)| flat[p * m + q] || width(p, q, r) == 0.0);
        let out_of_budget = plan
            .budget
            .is_some_and(|max_runs| total + active.len() as u64 > max_runs);
        let settled = active.len() == 1 || closed;
        if settled || out_of_budget {
            return Ok(Found {
                chosen: best_of(&active, &sums),
                survivors: active,
                means: raw.iter().zip(&runs).map(|(&s, &n)| mean(s, n)).collect(),
                runs,
                exact,
                proven: settled && !stuck,
                flat: stuck,
            });
        }
        for &p in &active {
            let rep = plan.start + r;
            let values = sample(plan.systems[p], rep..rep + 1)?;
            let x = values.first().copied().unwrap_or(f64::NAN) * scale;
            raw[p] = finite(raw[p] + x, SUM)?;
            sums[p] = finite(sums[p] + plan.sign * x + bonus(p), SUM)?;
            runs[p] += 1;
        }
        total += active.len() as u64;
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
    /// The caller states that there is no randomness: a zero S² is exact.
    deterministic: bool,
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
        /// The factor of [`scale_for`] for each constraint: the values, the limit and ε are all
        /// multiplied by it.
        scale: Vec<f64>,
        /// Σⱼ Yᵢₗⱼ (scaled), for the means.
        raw: Vec<f64>,
        /// Σⱼ (Yᵢₗⱼ − qₗ) on the "at most" scale (scaled).
        sums: Vec<f64>,
        /// h²S²ᵢₗ/(2εₗ), with S²ᵢₗ from the first stage: the region's half-width at r = 0.
        a: Vec<f64>,
        /// A zero S²ᵢₗ without the caller's statement that there is no randomness: the test
        /// never settles that constraint (see the module's notes).
        flat: Vec<bool>,
    }

    impl State {
        /// Undecided, with every constraint still undecided flat: more replications cannot
        /// settle it, since S² is fixed from the first stage.
        fn stuck(&self) -> bool {
            self.decision == Decision::Undecided
                && self
                    .by_constraint
                    .iter()
                    .zip(&self.flat)
                    .all(|(d, f)| *d != Decision::Undecided || *f)
        }
    }

    let mut states = Vec::with_capacity(systems);
    let mut constant_all = true;
    for i in 0..systems {
        let rows = sample(i, 0..n0)?;
        let mut state = State {
            decision: Decision::Undecided,
            by_constraint: vec![Decision::Undecided; s],
            runs: n0,
            scale: Vec::with_capacity(s),
            raw: Vec::with_capacity(s),
            sums: Vec::with_capacity(s),
            a: Vec::with_capacity(s),
            flat: Vec::with_capacity(s),
        };
        for (l, c) in constraints.iter().enumerate() {
            let column: Vec<f64> = rows.iter().map(|row| value(row, l)).collect();
            let k = scale_for(
                column
                    .iter()
                    .fold(c.limit.abs().max(c.tolerance), |big, y| big.max(y.abs())),
            );
            let column: Vec<f64> = column.iter().map(|y| y * k).collect();
            let limit = c.limit * k;
            state.scale.push(k);
            state.raw.push(finite(column.iter().sum(), SUM)?);
            state.sums.push(finite(
                column.iter().map(|y| c.bound.sign() * (y - limit)).sum(),
                SUM,
            )?);
            let s2 = if constant(&column) {
                0.0
            } else {
                constant_all = false;
                finite(variance(column.iter().copied()), VARIANCE)?
            };
            state
                .a
                .push(finite(h2 * s2 / (2.0 * c.tolerance * k), REGION)?);
            state.flat.push(s2 == 0.0 && !plan.deterministic);
        }
        states.push(state);
    }
    // Exact only on the caller's statement that there is no randomness.
    let exact = plan.deterministic && constant_all;
    let mut r = n0;
    let mut total = needed;
    loop {
        for state in states
            .iter_mut()
            .filter(|st| st.decision == Decision::Undecided)
        {
            for (l, c) in constraints.iter().enumerate() {
                if state.by_constraint[l] != Decision::Undecided || state.flat[l] {
                    continue;
                }
                let bound = (state.a[l] - c.tolerance * state.scale[l] * r as f64 / 2.0).max(0.0);
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
            .filter_map(|(i, st)| (st.decision == Decision::Undecided && !st.stuck()).then_some(i))
            .collect();
        let step = open.len() as u64;
        if open.is_empty() || plan.budget.is_some_and(|max_runs| total + step > max_runs) {
            let complete = states.iter().all(|st| st.decision != Decision::Undecided);
            let flat = states.iter().any(|st| {
                st.decision == Decision::Undecided
                    && st
                        .by_constraint
                        .iter()
                        .zip(&st.flat)
                        .any(|(d, f)| *d == Decision::Undecided && *f)
            });
            return Ok(FeasibilityCheck {
                decisions: states.iter().map(|st| st.decision).collect(),
                means: states
                    .iter()
                    .map(|st| {
                        st.raw
                            .iter()
                            .zip(&st.scale)
                            .map(|(x, k)| x / st.runs as f64 / k)
                            .collect()
                    })
                    .collect(),
                runs: states.iter().map(|st| st.runs).collect(),
                by_constraint: states.into_iter().map(|st| st.by_constraint).collect(),
                exact,
                confidence: (complete && !exact).then_some(plan.confidence),
                flat,
            });
        }
        for i in open {
            let rows = sample(i, r..r + 1)?;
            let row = rows.first().map(Vec::as_slice).unwrap_or(&[]);
            let Some(state) = states.get_mut(i) else {
                continue;
            };
            for (l, c) in constraints.iter().enumerate() {
                let k = state.scale[l];
                let y = value(row, l) * k;
                state.raw[l] = finite(state.raw[l] + y, SUM)?;
                state.sums[l] = finite(state.sums[l] + c.bound.sign() * (y - c.limit * k), SUM)?;
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
        // Stated deterministic: compared exactly on the first stage.
        let setup = Kn {
            deterministic: true,
            ..Kn::new(0.5, Sense::Minimise)
        };
        let means = [3.0, 1.0, 2.0, 1.5];
        let s = select_best(&setup, 4, |s, reps| reps.map(|_| means[s]).collect());
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(s.outcome, Outcome::ComparedExactly);
        assert_eq!(s.chosen, 1);
        assert_eq!(s.survivors, vec![1]);
        assert_eq!(s.runs, vec![10; 4]);
        assert_eq!(s.confidence, None);
        assert!(!s.flat);
        assert_eq!(s.means[2], Some(2.0));
        // Exact ties: all of them survive and the lowest index is chosen.
        let means = [1.0, 2.0, 2.0];
        let s = select_best(
            &Kn {
                deterministic: true,
                ..Kn::new(0.5, Sense::Maximise)
            },
            3,
            |s, reps| reps.map(|_| means[s]).collect(),
        );
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!((s.chosen, s.survivors.clone()), (1, vec![1, 2]));
        assert_eq!(s.total_runs(), 30);
    }

    #[test]
    fn constant_samples_without_the_statement_are_not_proven() {
        // The same constant samplers, not stated deterministic: no pair can be decided, so
        // nothing is compared exactly or claimed with confidence, and sampling stops at once
        // (more replications cannot change a zero S²).
        let means = [3.0, 1.0, 2.0, 1.5];
        let s = select_best(&Kn::new(0.5, Sense::Minimise), 4, |s, reps| {
            reps.map(|_| means[s]).collect()
        });
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(s.outcome, Outcome::NotProven);
        assert_eq!(s.confidence, None);
        assert!(s.flat);
        assert_eq!(s.chosen, 1);
        assert_eq!(s.survivors, vec![0, 1, 2, 3]);
        assert_eq!(s.total_runs(), 40);
    }

    /// A rare event the first stage misses: system A gives 0.001 every time; system B gives 1 in
    /// replications 10, 110, 210…, and 0 otherwise, so its first ten values are all 0 (as they
    /// are with probability 0.99¹⁰ ≈ 90% for a 1% event) and its true mean (about 0.01) is the
    /// higher. Maximising, A must not be called best, exactly or with confidence.
    #[test]
    fn a_rare_event_missed_by_the_first_stage_proves_nothing() {
        let rare = |s: usize, reps: Range<u64>| -> Vec<f64> {
            reps.map(|j| match s {
                0 => 0.001,
                _ if j % 100 == 10 => 1.0,
                _ => 0.0,
            })
            .collect()
        };
        let setup = Kn {
            max_runs: Some(2000),
            ..Kn::new(0.005, Sense::Maximise)
        };
        let s = select_best(&setup, 2, rare);
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(s.outcome, Outcome::NotProven, "{s:?}");
        assert_eq!(s.confidence, None);
        assert!(s.flat);
        assert_eq!(s.survivors, vec![0, 1]);
        // The comparison with a standard (A the standard) cannot keep it either.
        let s = compare_with_standard(&setup, 1, rare);
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(s.outcome, Outcome::NotProven, "{s:?}");
        assert!(s.flat);
    }

    /// Common random numbers: both systems vary, but their difference is the same in every
    /// first-stage replication (B = A + 0.1 there), so S² of the difference is 0; later B is
    /// A − 0.5, so the first stage points the wrong way in either sense. The flat pair must not
    /// give a winner with confidence.
    #[test]
    fn a_flat_difference_under_common_random_numbers_proves_nothing() {
        let crn = |s: usize, reps: Range<u64>| -> Vec<f64> {
            reps.map(|j| {
                let a = normal(21, 0, j);
                match s {
                    0 => a,
                    _ if j < 10 => a + 0.1,
                    _ => a - 0.5,
                }
            })
            .collect()
        };
        for sense in [Sense::Minimise, Sense::Maximise] {
            let setup = Kn {
                max_runs: Some(500),
                ..Kn::new(0.05, sense)
            };
            let s = select_best(&setup, 2, crn);
            let Ok(s) = s else { panic!("{s:?}") };
            assert_eq!(s.outcome, Outcome::NotProven, "{sense:?}: {s:?}");
            assert_eq!(s.confidence, None);
            assert!(s.flat);
            // A third system clearly worse still is eliminated by its own pairs, which vary.
            let three = |s: usize, reps: Range<u64>| -> Vec<f64> {
                if s == 2 {
                    reps.map(|j| {
                        normal(22, 2, j)
                            + if sense == Sense::Minimise {
                                50.0
                            } else {
                                -50.0
                            }
                    })
                    .collect()
                } else {
                    crn(s, reps)
                }
            };
            let s = select_best(&setup, 3, three);
            let Ok(s) = s else { panic!("{s:?}") };
            assert_eq!(s.outcome, Outcome::NotProven, "{sense:?}: {s:?}");
            assert_eq!(s.survivors, vec![0, 1]);
            assert_eq!(s.runs[2], 10);
        }
    }

    #[test]
    fn values_near_the_largest_f64_are_compared_without_overflow() {
        // Truly constant, and ten of them sum past the largest f64: compared on a scale where
        // they do not, so the right one wins (or nothing is claimed), never a tie of infinities.
        let huge = [1e308, 1.1e308];
        for (sense, best) in [(Sense::Minimise, 0), (Sense::Maximise, 1)] {
            let exact = Kn {
                deterministic: true,
                ..Kn::new(1.0, sense)
            };
            let s = select_best(&exact, 2, |s, reps| reps.map(|_| huge[s]).collect());
            let Ok(s) = s else { panic!("{s:?}") };
            assert_eq!((s.outcome, s.chosen), (Outcome::ComparedExactly, best));
            assert!(
                s.means[0].is_some_and(|m| close(m, 1e308))
                    && s.means[1].is_some_and(|m| close(m, 1.1e308)),
                "{s:?}"
            );
            let s = compare_with_standard(&exact, 1, |s, reps| reps.map(|_| huge[s]).collect());
            let Ok(s) = s else { panic!("{s:?}") };
            assert_eq!((s.outcome, s.chosen), (Outcome::ComparedExactly, best));
            // Not stated deterministic: flat, so not proven.
            let s = select_best(&Kn::new(1.0, sense), 2, |s, reps| {
                reps.map(|_| huge[s]).collect()
            });
            let Ok(s) = s else { panic!("{s:?}") };
            assert_eq!(
                (s.outcome, s.chosen, s.flat),
                (Outcome::NotProven, best, true)
            );
        }
        // ±9.45e307 totals whose difference would overflow unscaled.
        let s = select_best(
            &Kn {
                deterministic: true,
                ..Kn::new(1.0, Sense::Minimise)
            },
            2,
            |s, reps| {
                reps.map(|_| if s == 0 { 9.45e306 } else { -9.45e306 })
                    .collect()
            },
        );
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!((s.outcome, s.chosen), (Outcome::ComparedExactly, 1));
        // The check, on the same scale.
        let constraint = [Constraint {
            bound: Bound::AtMost,
            limit: 0.0,
            tolerance: 1.0,
        }];
        let check = check_feasibility(
            &Feasibility {
                deterministic: true,
                ..Feasibility::default()
            },
            2,
            &constraint,
            |s, reps| reps.map(|_| vec![huge[s]]).collect(),
        );
        let Ok(check) = check else {
            panic!("{check:?}")
        };
        assert_eq!(check.decisions, vec![Decision::Infeasible; 2]);
        assert!(
            close(check.means[0][0], 1e308) && close(check.means[1][0], 1.1e308),
            "{check:?}"
        );
    }

    #[test]
    fn values_that_overflow_are_refused() {
        // A first stage near 0 sets an ordinary scale; then both systems give 1.7e308, and the
        // second such replication takes the totals past the largest f64: refused, not compared.
        let late = |s: usize, reps: Range<u64>| -> Vec<f64> {
            reps.map(|j| {
                if j < 10 {
                    normal(51, s as u64, j)
                } else {
                    1.7e308
                }
            })
            .collect()
        };
        let setup = Kn::new(0.01, Sense::Maximise);
        assert_eq!(
            select_best(&setup, 2, late),
            Err(SelectError::Overflow(SUM))
        );
        assert_eq!(
            compare_with_standard(&setup, 1, late),
            Err(SelectError::Overflow(SUM))
        );
        // The check: the first constraint's value is 0 in the first stage (flat, so never
        // settled), then 1.7e308; the second stays near its limit, so the system stays undecided
        // while the first constraint's total overflows.
        let constraints = [
            Constraint {
                bound: Bound::AtMost,
                limit: 0.0,
                tolerance: 0.01,
            },
            Constraint {
                bound: Bound::AtMost,
                limit: 0.0,
                tolerance: 0.01,
            },
        ];
        let check = check_feasibility(&Feasibility::default(), 2, &constraints, |s, reps| {
            reps.map(|j| vec![if j < 10 { 0.0 } else { 1.7e308 }, normal(52, s as u64, j)])
                .collect()
        });
        assert_eq!(check, Err(SelectError::Overflow(SUM)));
        assert!(
            SelectError::Overflow("a sum of the values")
                .to_string()
                .contains("overflowed")
        );
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
        let setup = Feasibility {
            deterministic: true,
            ..Feasibility::default()
        };
        let check = check_feasibility(&setup, 4, &constraints, |s, reps| {
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
        // Not stated deterministic: no constraint is settled by its flat first stage, and the
        // systems take no more replications (more cannot change a zero S²).
        let check = check_feasibility(&Feasibility::default(), 4, &constraints, |s, reps| {
            reps.map(|_| values[s].to_vec()).collect()
        });
        let Ok(check) = check else {
            panic!("{check:?}")
        };
        assert_eq!(check.decisions, vec![Undecided; 4]);
        assert!(!check.exact);
        assert!(check.flat);
        assert_eq!(check.confidence, None);
        assert_eq!(check.runs, vec![10; 4]);
    }

    /// A rare event the first stage misses, against a limit: system 1 gives 1 in replications
    /// 10, 110, 210… and 0 otherwise (mean about 0.01), above the limit 0.005; system 0 gives
    /// 0.001, below it. The first ten values of both are constant, which must not declare
    /// system 1 feasible, exactly or with confidence.
    #[test]
    fn a_rare_event_missed_by_the_first_stage_decides_no_constraint() {
        let constraints = [Constraint {
            bound: Bound::AtMost,
            limit: 0.005,
            tolerance: 0.001,
        }];
        let rare = |s: usize, reps: Range<u64>| -> Vec<Vec<f64>> {
            reps.map(|j| {
                vec![match s {
                    0 => 0.001,
                    _ if j % 100 == 10 => 1.0,
                    _ => 0.0,
                }]
            })
            .collect()
        };
        let setup = Feasibility {
            max_runs: Some(1000),
            ..Feasibility::default()
        };
        let check = check_feasibility(&setup, 2, &constraints, rare);
        let Ok(check) = check else {
            panic!("{check:?}")
        };
        assert_eq!(check.decisions, vec![Decision::Undecided; 2]);
        assert!(check.feasible().is_empty());
        assert!(!check.exact);
        assert!(check.flat);
        assert_eq!(check.confidence, None);
        // KN after the check: nothing is chosen, nothing is claimed.
        let s = select_feasible(
            &Kn {
                max_runs: Some(1000),
                ..Kn::new(0.01, Sense::Minimise)
            },
            &constraints,
            0.5,
            2,
            |s, reps: Range<u64>| {
                rare(s, reps)
                    .into_iter()
                    .map(|row| vec![s as f64, row[0]])
                    .collect()
            },
        );
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!((s.chosen(), s.confidence), (None, None));
        assert!(s.selection.is_none());
        // A varying constraint beside the flat one still settles an infeasible system.
        let two = [
            constraints[0],
            Constraint {
                bound: Bound::AtMost,
                limit: 0.0,
                tolerance: 0.1,
            },
        ];
        let check = check_feasibility(&setup, 2, &two, |s, reps| {
            rare(s, reps.clone())
                .into_iter()
                .zip(reps)
                .map(|(row, j)| vec![row[0], 5.0 + normal(31, s as u64, j)])
                .collect()
        });
        let Ok(check) = check else {
            panic!("{check:?}")
        };
        assert_eq!(check.decisions, vec![Decision::Infeasible; 2]);
        assert_eq!(
            check.by_constraint[1],
            vec![Decision::Undecided, Decision::Infeasible]
        );
        assert!(!check.flat);
        assert_eq!(check.confidence, Some(0.95));
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
    fn a_budget_short_of_kn_keeps_the_feasibility_decisions() {
        // Constraint values far from the limit with a tiny spread: the check is decided on the
        // first stage, 4 × 10 runs.
        let constrained = [-1.0, -1.0, 1.0, -1.0];
        let constraints = [Constraint {
            bound: Bound::AtMost,
            limit: 0.0,
            tolerance: 0.5,
        }];
        let rows = |s: usize, reps: Range<u64>| -> Vec<Vec<f64>> {
            reps.map(|j| {
                vec![
                    normal(9, s as u64, j),
                    constrained[s] + 0.001 * normal(10, s as u64, j),
                ]
            })
            .collect()
        };
        // KN among the three feasible needs 30 more; one short.
        let setup = Kn {
            max_runs: Some(40 + 30 - 1),
            ..Kn::new(0.5, Sense::Maximise)
        };
        let s = select_feasible(&setup, &constraints, 0.5, 4, rows);
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(s.feasibility.feasible(), vec![0, 1, 3]);
        assert_eq!(s.feasibility.total_runs(), 40);
        assert_eq!(
            (s.selection.as_ref(), s.confidence, s.chosen()),
            (None, None, None)
        );
        // With the 30, KN runs.
        let setup = Kn {
            max_runs: Some(70),
            ..setup
        };
        let s = select_feasible(&setup, &constraints, 0.5, 4, rows);
        let Ok(s) = s else { panic!("{s:?}") };
        assert!(s.selection.is_some());
        assert!(s.total_runs() <= 70, "{}", s.total_runs());
    }

    #[test]
    fn feasibility_out_of_budget_leaves_systems_undecided() {
        // Means 0.05 from the limit with ε = 0.01: far from settled after a few steps.
        let constraints = [Constraint {
            bound: Bound::AtMost,
            limit: 0.0,
            tolerance: 0.01,
        }];
        let setup = Feasibility {
            max_runs: Some(4 * 10 + 4 * 5),
            ..Feasibility::default()
        };
        let check = check_feasibility(&setup, 4, &constraints, |s, reps| {
            reps.map(|j| vec![0.05 * (s as f64 - 1.5) + normal(13, s as u64, j)])
                .collect()
        });
        let Ok(check) = check else {
            panic!("{check:?}")
        };
        assert!(!check.is_complete());
        assert!(check.decisions.contains(&Decision::Undecided));
        assert!(
            check
                .feasible()
                .iter()
                .all(|&i| check.decisions[i] == Decision::Feasible)
        );
        assert_eq!(check.confidence, None);
        assert!(check.total_runs() <= 60, "{}", check.total_runs());
    }

    #[test]
    fn the_standard_is_kept_unless_an_alternative_is_better() {
        let setup = Kn::new(0.5, Sense::Minimise);
        // Stated without randomness: the standard (0) at 2, alternatives at 3, 2.2 and 2.5.
        let means = [2.0, 3.0, 2.2, 2.5];
        let exact = Kn {
            deterministic: true,
            ..setup
        };
        let s = compare_with_standard(&exact, 3, |s, reps| reps.map(|_| means[s]).collect());
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!((s.outcome, s.chosen), (Outcome::ComparedExactly, 0));
        // The same values not stated deterministic: not proven.
        let s = compare_with_standard(&setup, 3, |s, reps| reps.map(|_| means[s]).collect());
        let Ok(s) = s else { panic!("{s:?}") };
        assert_eq!(
            (s.outcome, s.confidence, s.flat),
            (Outcome::NotProven, None, true)
        );
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
