//! Racing the candidates a search proposes, then the clean-up of the finalists by KN.
//!
//! A search (M9's own, or an outside one such as an ask-and-tell optimiser) proposes candidates
//! over time. Each candidate gets a first stage of replications, then more in stages while it
//! stays in contention. This is racing (Birattari, Stützle, Paquete and Varrentrapp 2002), with a
//! paired t interval in place of their Friedman test, because every candidate is run on the same
//! replications (common random numbers) and so two candidates pair replication by replication.
//! When the race is over, the candidates still in contention are run again on fresh replications
//! that no step of the race used, and KN ([`crate::select::select_best`]) selects among them.
//!
//! **Elimination.** After a candidate's stage, the candidate with the best mean in contention is
//! the leader b. Candidate i is dropped only when the one-sided paired t interval for
//! μ_b − μ_i (in the better direction), over the n replications they share (the first
//! n = min(nᵢ, n_b), since every candidate's replications are numbered from 0), lies wholly above
//! δ: when the mean difference d̄ exceeds δ and P(T_{n−1} ≥ (d̄ − δ)/(s_d/√n)) ≤ β. A candidate
//! within δ of the leader is never dropped, however many replications it has.
//!
//! **The error of the race (Bonferroni over the planned looks and pairs).** The race is planned
//! for at most M candidates ([`Racing::max_candidates`]) and L stages per candidate
//! ([`Racing::stages`]). Every shared count n is a stage boundary, so there are at most L values
//! of n. The best candidate tried can only be dropped by an event "the interval of some other
//! candidate b against it, over their first n replications, lies above δ", for one of the M − 1
//! other candidates and one of the L counts. Each such event is a fixed test on fixed data, of
//! probability at most β under normality, because the true μ_b − μ_best ≤ 0 < δ. Whichever
//! candidate leads at each look (it is chosen from the data), the union of these (M − 1)·L events
//! contains every way of dropping the best, so with β = α_race / ((M − 1)·L) the race drops the
//! best candidate tried with probability at most α_race. Testing the same pair again at the same
//! count repeats the same event and adds no error.
//!
//! **Which candidates the bound covers.** "A fixed test on fixed data" needs each candidate to be
//! chosen without looking at the values told on the replication indices it will share with the
//! others. That holds when the candidates are fixed in advance, or when the search chooses each
//! one from information independent of those values (a fixed design, a random or Sobol sequence,
//! a list). It does not hold for an adaptive search that reads the race's results and then
//! proposes a candidate to be run on the same indices: its paired differences on those indices
//! then depend on the choice, a single test can exceed β, and the bound on dropping the best is
//! not claimed. KN's clean-up on fresh replications is unaffected: its statement about the
//! finalists holds whatever chose them.
//!
//! **The order of tells.** Requests may complete in any order. The leader at each look, and so
//! which pairs are tested and at which counts, depends on that order: telling the same values in
//! another order can drop other candidates and change who reaches the clean-up. The bound holds
//! for every order, since it covers every pair and every count.
//!
//! **The clean-up.** By default every candidate still in contention is a finalist. KN runs on
//! replications numbered from the first index after every one the race asked for, so its data are
//! independent of the race and its guarantee holds for the set it is given, at confidence
//! 1 − α_clean. With α_race = share·α and α_clean = (1 − share)·α (half each by default),
//! Bonferroni's inequality bounds the chance that the race drops the best or that KN fails to
//! choose it or one within δ of it by α. The answer states that the chosen one is the best of the
//! k finalists, or within δ of it, with 1 − α confidence (KN's own level, 1 − α_clean, is
//! higher), and that the search tried more candidates than it proves anything about.
//!
//! **Candidates never raced.** A candidate whose first stage was never run (the race's budget ran
//! out first, or the clean-up was called while it still waited) was neither raced nor cleaned up.
//! The answer lists such candidates ([`RaceAnswer::untested`]) and says that its confidence
//! covers only the candidates that raced.
//!
//! **A cap on the finalists.** [`Racing::finalists`] may cap the clean-up at the k candidates in
//! contention with the best means. That cut is not covered by any probability: a candidate left
//! out may be the best. When the cap cuts, the answer lists the candidates left out
//! ([`RaceAnswer::left_out`]) and says in words that its confidence covers only the finalists.
//!
//! **A flat difference is not proof of no randomness.** Equal differences in every shared
//! replication can come from candidates that do vary (a rare event not yet seen, or common random
//! numbers that move both alike). Unless the caller states that the candidates have no randomness
//! ([`Racing::deterministic`]), such a pair is not tested at that look: nobody is dropped on it,
//! which only removes ways of dropping the best, so the bound above still holds. A later look
//! whose differences vary tests the pair as usual. With the statement, a constant difference is
//! known exactly and the candidate is dropped when it exceeds δ. The clean-up passes the same
//! statement to KN, whose answer is [`Outcome::ComparedExactly`] only with it; without it, a flat
//! pair of finalists leaves KN's answer not proven ([`crate::select`]'s notes).
//!
//! **Assumptions.** The vectors of all candidates' values in a replication must be independent
//! across replications and identically distributed, jointly (multivariate) normal, as KN and the
//! paired t interval assume; each candidate's values being normal on its own is not enough, since
//! the differences of two normal values need not be normal. [`Sampling::Common`] only chooses
//! KN's constants for dependent candidates; it does not remove this requirement. A replication's
//! mean over a long run is close to normal; a count in a short run may not be.
//!
//! The procedure is written from the published papers; no code of any implementation was used.

use std::ops::Range;

use stochcraft_random::estimate::beta_cdf;

use crate::select::{Kn, Outcome, Sampling, SelectError, Selection, Sense, select_best};

/// +1 to maximise, −1 to minimise.
fn sign(sense: Sense) -> f64 {
    match sense {
        Sense::Maximise => 1.0,
        Sense::Minimise => -1.0,
    }
}

/// The settings of a [`Race`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Racing {
    /// The indifference amount δ, in the observations' unit: the race drops a candidate only when
    /// it is worse than the leader by more than δ, and KN chooses the best or one within δ of it.
    pub delta: f64,
    /// 1 − α, covering the race and the clean-up together (default 0.95).
    pub confidence: f64,
    /// The share of α given to the race, strictly between 0 and 1 (default 0.5); the clean-up
    /// gets the rest.
    pub racing_share: f64,
    /// Each candidate's first stage, in replications (default 10, at least 2).
    pub first_stage: u64,
    /// Replications in each later stage (default 5, at least 1).
    pub stage: u64,
    /// The most stages a candidate takes, the first included (default 5, at least 1): a
    /// candidate that stays in contention ends the race with
    /// [`Racing::max_replications`] replications. These are the planned looks L.
    pub stages: u64,
    /// The most candidates the race takes, M, which sets its per-look level (at least 1).
    pub max_candidates: usize,
    /// A cap k on the candidates the clean-up takes, the best race means first (default `None`:
    /// every candidate still in contention). A cap that cuts is not covered by the confidence;
    /// the answer then lists the candidates left out.
    pub finalists: Option<usize>,
    /// KN's first stage n₀ in the clean-up (default 10, at least 2).
    pub cleanup_first_stage: u64,
    pub sense: Sense,
    /// How the candidates' replications relate, for KN's constants (default common random
    /// numbers, valid whatever the dependence). The race's paired interval is valid either way.
    pub sampling: Sampling,
    /// The most replications the race may take, summed over candidates. When the next stage would
    /// pass it, the race stops and the clean-up runs on what is left in contention.
    pub max_race_runs: Option<u64>,
    /// The most replications the clean-up may take. If it cannot pay for KN's first stage, or KN
    /// runs out, the answer is not proven.
    pub max_cleanup_runs: Option<u64>,
    /// The caller states that the candidates have no randomness (default `false`). Only then is
    /// a constant paired difference taken as exact in the race, and is the clean-up compared
    /// exactly; equal values alone are never proof of it.
    pub deterministic: bool,
}

impl Racing {
    pub fn new(delta: f64, sense: Sense, max_candidates: usize) -> Self {
        Self {
            delta,
            confidence: 0.95,
            racing_share: 0.5,
            first_stage: 10,
            stage: 5,
            stages: 5,
            max_candidates,
            finalists: None,
            cleanup_first_stage: 10,
            sense,
            sampling: Sampling::Common,
            max_race_runs: None,
            max_cleanup_runs: None,
            deterministic: false,
        }
    }

    /// The replications of a candidate that takes every stage: n₁ + (L − 1)·stage (saturating).
    pub fn max_replications(&self) -> u64 {
        self.first_stage
            .saturating_add(self.stages.saturating_sub(1).saturating_mul(self.stage))
    }

    /// The one-sided level β of each paired test: α_race / ((M − 1)·L), α_race = share·α.
    pub fn level(&self) -> f64 {
        let alpha_race = self.racing_share * (1.0 - self.confidence);
        let pairs = self.max_candidates.saturating_sub(1).max(1) as f64;
        alpha_race / (pairs * self.stages.max(1) as f64)
    }

    /// KN's confidence in the clean-up: 1 − (1 − share)·α.
    pub fn cleanup_confidence(&self) -> f64 {
        1.0 - (1.0 - self.racing_share) * (1.0 - self.confidence)
    }

    fn validate(&self) -> Result<(), RaceError> {
        let bad = |name: &'static str| -> Result<(), RaceError> { Err(RaceError::Setting(name)) };
        if !(self.delta.is_finite() && self.delta > 0.0) {
            return bad("delta");
        }
        if !(self.confidence > 0.0 && self.confidence < 1.0) {
            return bad("confidence");
        }
        if !(self.racing_share > 0.0 && self.racing_share < 1.0) {
            return bad("racing_share");
        }
        if self.first_stage < 2 {
            return bad("first_stage");
        }
        if self.stage == 0 {
            return bad("stage");
        }
        if self.stages == 0 {
            return bad("stages");
        }
        if self.max_candidates == 0 {
            return bad("max_candidates");
        }
        if self.finalists == Some(0) {
            return bad("finalists");
        }
        if self.cleanup_first_stage < 2 {
            return bad("cleanup_first_stage");
        }
        // KN's range for every count of finalists it can be given, 1 to k: 0 < 1 − α_clean < 1,
        // and 1/m < 1 − α_clean for m from 2 to k, which is 1/2 < 1 − α_clean when k ≥ 2.
        let k = self
            .finalists
            .unwrap_or(self.max_candidates)
            .min(self.max_candidates);
        let clean = self.cleanup_confidence();
        let floor = if k > 1 { 0.5 } else { 0.0 };
        if !(clean > floor && clean < 1.0) {
            return bad("confidence");
        }
        if self.max_replications() == u64::MAX {
            return bad("stages");
        }
        Ok(())
    }
}

/// Where a candidate stands in the race.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting for its first stage.
    Waiting,
    /// In contention, with more stages to take.
    Racing,
    /// In contention, with every stage taken.
    Finished,
    /// Dropped: the paired interval against `against` (the leader then), over the first
    /// `replications` replications they share, lay wholly beyond δ.
    Dropped { against: usize, replications: u64 },
    /// Never run: the race's budget ran out before its first stage. Not covered by the answer's
    /// confidence ([`RaceAnswer::untested`]).
    NotRun,
}

impl Status {
    /// Whether the candidate is in contention (its first stage taken and not dropped).
    pub fn in_contention(self) -> bool {
        matches!(self, Status::Racing | Status::Finished)
    }
}

/// Replications the race asks for: run `candidate` on `replications` and [`Race::tell`] the
/// values, one per replication, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub candidate: usize,
    pub replications: Range<u64>,
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    values: Vec<f64>,
    /// The mean of `values`, kept as a running mean (each step adds x/n − mean/n), so that large
    /// finite values do not overflow a sum.
    mean: f64,
    status: Status,
    pending: Option<Range<u64>>,
}

/// A race in progress: candidates are added as a search proposes them ([`Race::add`]), the race
/// says which replications to run next ([`Race::ask`]) and takes their values ([`Race::tell`]);
/// [`Race::clean_up`] ends it. Candidates are numbered 0, 1, 2… in the order they were added.
///
/// [`Race::ask`] gives every waiting candidate its first stage first, in the order they were
/// added, then the next stage to the candidate in contention with the fewest replications (the
/// lowest number on a tie), so the candidates advance nearly together. Several requests may be
/// outstanding at once (one per candidate), so they can run in parallel. The order in which they
/// are told can change who leads at each look, and so which pairs are compared, which candidates
/// are dropped and who reaches the clean-up; the race's error bound holds for every order.
#[derive(Debug, Clone, PartialEq)]
pub struct Race {
    settings: Racing,
    level: f64,
    candidates: Vec<Entry>,
    /// Replications asked for so far, summed over candidates.
    runs: u64,
    /// One past the highest replication index asked for.
    fresh_from: u64,
    budget_spent: bool,
    leader: Option<usize>,
}

impl Race {
    pub fn new(settings: Racing) -> Result<Self, RaceError> {
        settings.validate()?;
        Ok(Self {
            level: settings.level(),
            settings,
            candidates: Vec::new(),
            runs: 0,
            fresh_from: 0,
            budget_spent: false,
            leader: None,
        })
    }

    pub fn settings(&self) -> &Racing {
        &self.settings
    }

    /// Adds a candidate the search proposes and returns its number. After the race's budget is
    /// spent, the candidate is recorded as tried but is never run ([`Status::NotRun`]).
    pub fn add(&mut self) -> Result<usize, RaceError> {
        if self.candidates.len() >= self.settings.max_candidates {
            return Err(RaceError::TooManyCandidates {
                max: self.settings.max_candidates,
            });
        }
        self.candidates.push(Entry {
            values: Vec::new(),
            mean: 0.0,
            status: if self.budget_spent {
                Status::NotRun
            } else {
                Status::Waiting
            },
            pending: None,
        });
        Ok(self.candidates.len() - 1)
    }

    /// The next replications to run, or `None` when every candidate in contention has taken all
    /// its stages or has a request outstanding, or the budget is spent.
    pub fn ask(&mut self) -> Option<Request> {
        if self.budget_spent {
            return None;
        }
        let first = self
            .candidates
            .iter()
            .position(|e| e.status == Status::Waiting && e.pending.is_none())
            .map(|c| (c, 0..self.settings.first_stage));
        let next = first.or_else(|| {
            self.candidates
                .iter()
                .enumerate()
                .filter(|(_, e)| e.status == Status::Racing && e.pending.is_none())
                .min_by_key(|(c, e)| (e.values.len(), *c))
                .map(|(c, e)| {
                    let n = e.values.len() as u64;
                    (c, n..n + self.settings.stage)
                })
        });
        let (candidate, replications) = next?;
        let size = replications.end - replications.start;
        if self
            .settings
            .max_race_runs
            .is_some_and(|max| self.runs.checked_add(size).is_none_or(|total| total > max))
        {
            self.budget_spent = true;
            for e in &mut self.candidates {
                if e.status == Status::Waiting && e.pending.is_none() {
                    e.status = Status::NotRun;
                }
            }
            return None;
        }
        self.runs += size;
        self.fresh_from = self.fresh_from.max(replications.end);
        if let Some(e) = self.candidates.get_mut(candidate) {
            e.pending = Some(replications.clone());
        }
        Some(Request {
            candidate,
            replications,
        })
    }

    /// The values of the replications last asked for `candidate`, one per replication, in order.
    /// The candidate is then compared with the leader, and the leader with the others when the
    /// leader changes or gains replications.
    pub fn tell(&mut self, candidate: usize, values: &[f64]) -> Result<(), RaceError> {
        let n_max = self.settings.max_replications();
        let entry = self
            .candidates
            .get_mut(candidate)
            .ok_or(RaceError::UnknownCandidate(candidate))?;
        let reps = entry
            .pending
            .clone()
            .ok_or(RaceError::NotAsked(candidate))?;
        let expected = reps.end - reps.start;
        if values.len() as u64 != expected {
            return Err(RaceError::WrongCount {
                candidate,
                expected,
                got: values.len() as u64,
            });
        }
        if let Some(j) = values.iter().position(|v| !v.is_finite()) {
            return Err(RaceError::NotFinite {
                candidate,
                replication: reps.start + j as u64,
            });
        }
        entry.pending = None;
        for &x in values {
            entry.values.push(x);
            let n = entry.values.len() as f64;
            entry.mean = if entry.values.len() == 1 {
                x
            } else {
                entry.mean + (x / n - entry.mean / n)
            };
        }
        if entry.status == Status::Waiting {
            entry.status = Status::Racing;
        }
        if entry.status == Status::Racing && entry.values.len() as u64 >= n_max {
            entry.status = Status::Finished;
        }
        if entry.status.in_contention() {
            self.screen(candidate);
        }
        Ok(())
    }

    /// Asks and tells until the race has nothing more to run, with `sample(candidate,
    /// replications)` giving one value per replication. Requests already outstanding are run
    /// first, without being charged again: when a tell fails (wrong count, a value not finite),
    /// its request stays outstanding, so calling `run` again samples that same range again.
    pub fn run<F>(&mut self, sample: &mut F) -> Result<(), RaceError>
    where
        F: FnMut(usize, Range<u64>) -> Vec<f64>,
    {
        loop {
            let outstanding = self.candidates.iter().enumerate().find_map(|(c, e)| {
                e.pending.clone().map(|replications| Request {
                    candidate: c,
                    replications,
                })
            });
            let Some(request) = outstanding.or_else(|| self.ask()) else {
                break;
            };
            let values = sample(request.candidate, request.replications.clone());
            self.tell(request.candidate, &values)?;
        }
        Ok(())
    }

    /// The candidate's mean over its replications, as observed, or `None` before its
    /// first stage.
    pub fn mean(&self, candidate: usize) -> Option<f64> {
        let e = self.candidates.get(candidate)?;
        (!e.values.is_empty()).then_some(e.mean)
    }

    /// Replications told for the candidate.
    pub fn replications(&self, candidate: usize) -> Option<u64> {
        self.candidates
            .get(candidate)
            .map(|e| e.values.len() as u64)
    }

    pub fn status(&self, candidate: usize) -> Option<Status> {
        self.candidates.get(candidate).map(|e| e.status)
    }

    /// The candidate in contention with the best mean (the lowest number on a tie).
    pub fn leader(&self) -> Option<usize> {
        self.leader
    }

    /// The candidates in contention, in number order.
    pub fn contenders(&self) -> Vec<usize> {
        self.candidates
            .iter()
            .enumerate()
            .filter_map(|(c, e)| e.status.in_contention().then_some(c))
            .collect()
    }

    /// The candidates added so far.
    pub fn tried(&self) -> usize {
        self.candidates.len()
    }

    /// Replications asked for so far, summed over candidates.
    pub fn runs(&self) -> u64 {
        self.runs
    }

    /// Whether the race stopped because its budget was spent.
    pub fn budget_spent(&self) -> bool {
        self.budget_spent
    }

    /// The first replication index the race never asked for: the clean-up starts there.
    pub fn fresh_from(&self) -> u64 {
        self.fresh_from
    }

    /// The candidate's mean in the "larger is better" direction.
    fn score(&self, c: usize) -> f64 {
        self.mean(c)
            .map_or(f64::NEG_INFINITY, |m| sign(self.settings.sense) * m)
    }

    fn find_leader(&self) -> Option<usize> {
        self.contenders()
            .into_iter()
            .fold(None, |best, c| match best {
                Some(b) if self.score(b) >= self.score(c) => Some(b),
                _ => Some(c),
            })
    }

    /// After `changed` took a stage: the new leader, then the pairs whose shared replications or
    /// leader changed. Pairs not tested again would repeat the same test on the same data.
    fn screen(&mut self, changed: usize) {
        let old = self.leader;
        self.leader = self.find_leader();
        let Some(b) = self.leader else {
            return;
        };
        let others: Vec<usize> = if old != Some(b) || changed == b {
            self.contenders().into_iter().filter(|&c| c != b).collect()
        } else {
            vec![changed]
        };
        for i in others {
            if let Some(n) = self.worse(i, b)
                && let Some(e) = self.candidates.get_mut(i)
            {
                e.status = Status::Dropped {
                    against: b,
                    replications: n,
                };
            }
        }
    }

    /// `Some(n)` when the paired interval of the leader `b` against `i`, over their first n shared
    /// replications, lies wholly beyond δ.
    fn worse(&self, i: usize, b: usize) -> Option<u64> {
        let (Some(xi), Some(xb)) = (self.candidates.get(i), self.candidates.get(b)) else {
            return None;
        };
        let n = xi.values.len().min(xb.values.len());
        if n < 2 {
            return None;
        }
        let s = sign(self.settings.sense);
        let d: Vec<f64> = xb
            .values
            .iter()
            .zip(&xi.values)
            .take(n)
            .map(|(y, x)| s * (y - x))
            .collect();
        // A difference that overflowed: nothing reliable to test, so nobody is dropped.
        if d.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let delta = self.settings.delta;
        if let Some(&first) = d.first()
            && d.iter().all(|&v| v == first)
        {
            // Finite and exactly constant. Known exactly only when the caller states there is no
            // randomness; otherwise it may be a rare event not yet seen, or common random numbers
            // moving both alike, and with a zero spread there is nothing to test: nobody is
            // dropped at this look.
            return (self.settings.deterministic && first > delta).then_some(n as u64);
        }
        // The differences scaled by the largest in size, so that neither their sum nor their
        // squared deviations overflow or underflow.
        let scale = d.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        let nf = n as f64;
        let scaled: Vec<f64> = d.iter().map(|v| v / scale).collect();
        let mean_scaled = scaled.iter().sum::<f64>() / nf;
        let mean = mean_scaled * scale;
        if !mean.is_finite() || mean <= delta {
            return None;
        }
        let var_scaled = scaled
            .iter()
            .map(|v| (v - mean_scaled) * (v - mean_scaled))
            .sum::<f64>()
            / (nf - 1.0);
        let se = scale * (var_scaled / nf).sqrt();
        if !(se.is_finite() && se > 0.0) {
            // The spread could not be computed reliably: no elimination.
            return None;
        }
        let t = (mean - delta) / se;
        (t_upper_tail(t, nf - 1.0) <= self.level).then_some(n as u64)
    }

    /// Ends the race: KN among the candidates in contention (the k best by their race means when
    /// [`Racing::finalists`] caps them), on replications numbered from
    /// [`Race::fresh_from`], with `sample(candidate, replications)` as in the race. Refused while
    /// a request is outstanding.
    pub fn clean_up<F>(&self, mut sample: F) -> Result<RaceAnswer, RaceError>
    where
        F: FnMut(usize, Range<u64>) -> Vec<f64>,
    {
        if let Some(c) = self.candidates.iter().position(|e| e.pending.is_some()) {
            return Err(RaceError::Pending(c));
        }
        let mut ranked = self.contenders();
        ranked.sort_by(|&a, &b| self.score(b).total_cmp(&self.score(a)).then(a.cmp(&b)));
        let k = self
            .settings
            .finalists
            .map_or(ranked.len(), |cap| cap.min(ranked.len()));
        let left_out = ranked.split_off(k);
        let finalists = ranked;
        let count =
            |f: fn(&Status) -> bool| self.candidates.iter().filter(|e| f(&e.status)).count();
        let untested: Vec<usize> = self
            .candidates
            .iter()
            .enumerate()
            .filter_map(|(c, e)| matches!(e.status, Status::NotRun | Status::Waiting).then_some(c))
            .collect();
        let mut answer = RaceAnswer {
            outcome: Outcome::NotProven,
            chosen: finalists.first().copied(),
            finalists: finalists.clone(),
            left_out,
            selection: None,
            confidence: None,
            cleanup_confidence: self.settings.cleanup_confidence(),
            delta: self.settings.delta,
            tried: self.candidates.len(),
            dropped: count(|s| matches!(s, Status::Dropped { .. })),
            untested,
            race_runs: self.runs,
            cleanup_runs: 0,
            race_budget_spent: self.budget_spent,
            fresh_from: self.fresh_from,
        };
        if finalists.is_empty() {
            return Ok(answer);
        }
        let n0 = self.settings.cleanup_first_stage;
        if self
            .settings
            .max_cleanup_runs
            .is_some_and(|max| max < n0.saturating_mul(finalists.len() as u64))
        {
            return Ok(answer);
        }
        let kn = Kn {
            delta: self.settings.delta,
            confidence: self.settings.cleanup_confidence(),
            first_stage: n0,
            sense: self.settings.sense,
            sampling: self.settings.sampling,
            max_runs: self.settings.max_cleanup_runs,
            deterministic: self.settings.deterministic,
        };
        let start = self.fresh_from;
        // KN numbers its replications from 0; they are moved past the race's. A range that cannot
        // be represented after the move is refused before the sampler is called.
        let mut unrepresentable: Option<RaceError> = None;
        let selection = select_best(&kn, finalists.len(), |p, reps| {
            let Some(&c) = finalists.get(p) else {
                return Vec::new();
            };
            match (reps.start.checked_add(start), reps.end.checked_add(start)) {
                (Some(a), Some(b)) => sample(c, a..b),
                _ => {
                    unrepresentable.get_or_insert(RaceError::Replications {
                        candidate: c,
                        start: reps.start,
                        end: reps.end,
                        offset: start,
                    });
                    Vec::new()
                }
            }
        });
        if let Some(e) = unrepresentable {
            return Err(e);
        }
        let selection = selection.map_err(|e| match e {
            SelectError::WrongCount {
                system,
                expected,
                got,
            } => RaceError::WrongCount {
                candidate: finalists.get(system).copied().unwrap_or(system),
                expected,
                got,
            },
            SelectError::NotFinite {
                system,
                replication,
            } => RaceError::NotFinite {
                candidate: finalists.get(system).copied().unwrap_or(system),
                replication: replication.saturating_add(start),
            },
            other => RaceError::Select(other),
        })?;
        answer.outcome = selection.outcome;
        answer.chosen = finalists.get(selection.chosen).copied();
        answer.cleanup_runs = selection.total_runs();
        answer.confidence =
            (selection.outcome == Outcome::Selected).then_some(self.settings.confidence);
        answer.selection = Some(selection);
        Ok(answer)
    }
}

/// Races `candidates` candidates proposed all at once, then cleans up: [`Race::add`] for each,
/// [`Race::run`], [`Race::clean_up`].
pub fn race<F>(settings: &Racing, candidates: usize, mut sample: F) -> Result<RaceAnswer, RaceError>
where
    F: FnMut(usize, Range<u64>) -> Vec<f64>,
{
    let mut race = Race::new(*settings)?;
    for _ in 0..candidates {
        race.add()?;
    }
    race.run(&mut sample)?;
    race.clean_up(sample)
}

/// The answer of a race and its clean-up. Candidates are numbered in the order they were added.
#[derive(Debug, Clone, PartialEq)]
pub struct RaceAnswer {
    /// KN's outcome in the clean-up: [`Outcome::Selected`] with the confidence,
    /// [`Outcome::ComparedExactly`] when the candidates were stated to have no randomness
    /// ([`Racing::deterministic`]) and every value was the same, or [`Outcome::NotProven`] when
    /// the clean-up's budget could not pay for KN or ran out, two finalists gave the same
    /// difference in every first-stage replication without that statement ([`Selection::flat`]),
    /// or nothing was left to clean up.
    pub outcome: Outcome,
    /// The chosen candidate; when not proven, the best-looking finalist (by KN's means, or by the
    /// race's when KN did not run). `None` when no candidate finished its first stage.
    pub chosen: Option<usize>,
    /// The candidates the clean-up took, best race mean first.
    pub finalists: Vec<usize>,
    /// Candidates still in contention that a cap on the finalists ([`Racing::finalists`]) left
    /// out of the clean-up, best race mean first; empty without a cap. The confidence does not
    /// cover them.
    pub left_out: Vec<usize>,
    /// KN's answer, its systems numbered by position in `finalists`; `None` when it did not run.
    pub selection: Option<Selection>,
    /// The confidence claimed: `Some(1 − α)` only for [`Outcome::Selected`].
    pub confidence: Option<f64>,
    /// KN's own level in the clean-up, 1 − α_clean.
    pub cleanup_confidence: f64,
    pub delta: f64,
    /// Candidates the search tried (added to the race).
    pub tried: usize,
    /// Candidates the race dropped.
    pub dropped: usize,
    /// Candidates tried but never raced: their first stage never ran (the race's budget ran out
    /// first, or the clean-up was called while they waited). The confidence does not cover them.
    pub untested: Vec<usize>,
    pub race_runs: u64,
    pub cleanup_runs: u64,
    /// The race stopped because its budget was spent.
    pub race_budget_spent: bool,
    /// The clean-up's first replication index; the race used only indices below it.
    pub fresh_from: u64,
}

impl RaceAnswer {
    pub fn total_runs(&self) -> u64 {
        self.race_runs + self.cleanup_runs
    }
}

impl std::fmt::Display for RaceAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.race_budget_spent {
            write!(
                f,
                "The racing budget ran out after {} runs; the clean-up took the candidates still in contention. ",
                self.race_runs
            )?;
        }
        let Some(chosen) = self.chosen else {
            return write!(
                f,
                "Not proven: no candidate finished its first stage, so nothing was selected. The search tried {} candidates.",
                self.tried
            );
        };
        let list = self
            .finalists
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let k = self.finalists.len();
        match (self.outcome, self.confidence) {
            (Outcome::Selected, Some(c)) => write!(
                f,
                "Of these {k} finalists ({list}), candidate {chosen} is the best or within δ = {} of it, with {}% confidence.",
                self.delta,
                (c * 100.0 * 1e6).round() / 1e6
            )?,
            (Outcome::ComparedExactly, _) => write!(
                f,
                "Of these {k} finalists ({list}), candidate {chosen} is the best: they were stated to have no randomness and every replication gave the same values, so they were compared exactly and no probability is stated."
            )?,
            _ if self.selection.as_ref().is_some_and(|s| s.flat) => write!(
                f,
                "Not proven: some finalists gave the same difference in every replication of the clean-up's first stage, which does not show that they have no randomness, so the clean-up could not tell them apart; candidate {chosen} looks best of the finalists ({list})."
            )?,
            _ => write!(
                f,
                "Not proven: the clean-up's budget could not settle it; candidate {chosen} looks best of the finalists ({list})."
            )?,
        }
        write!(
            f,
            " The search tried {} candidates; racing dropped {}",
            self.tried, self.dropped
        )?;
        write!(f, ".")?;
        if !self.untested.is_empty() {
            let never = self
                .untested
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            write!(
                f,
                " Never raced: candidates {never}; the confidence covers only the candidates that raced.",
            )?;
        }
        if !self.left_out.is_empty() {
            let left = self
                .left_out
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            write!(
                f,
                " The cap on the finalists left out {} candidates still in contention ({left}), chosen by their means alone; the confidence covers only the finalists, and one left out may be the best.",
                self.left_out.len()
            )?;
        }
        write!(f, " This does not prove that none of the others is better.")
    }
}

/// Why a race could not go on.
#[derive(Debug, Clone, PartialEq)]
pub enum RaceError {
    /// A setting is out of range (its field's name).
    Setting(&'static str),
    /// More candidates than [`Racing::max_candidates`], for which the race's level was set.
    TooManyCandidates {
        max: usize,
    },
    UnknownCandidate(usize),
    /// Values told for a candidate with no request outstanding.
    NotAsked(usize),
    /// A request is outstanding for the candidate.
    Pending(usize),
    WrongCount {
        candidate: usize,
        expected: u64,
        got: u64,
    },
    NotFinite {
        candidate: usize,
        replication: u64,
    },
    /// KN's replications `start..end`, moved past the race's by `offset`, cannot be represented.
    Replications {
        candidate: usize,
        start: u64,
        end: u64,
        offset: u64,
    },
    /// KN refused in the clean-up.
    Select(SelectError),
}

impl std::fmt::Display for RaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RaceError::Setting(name) => write!(f, "the race's setting {name} is out of range"),
            RaceError::TooManyCandidates { max } => {
                write!(f, "the race is planned for at most {max} candidates")
            }
            RaceError::UnknownCandidate(c) => write!(f, "there is no candidate {c}"),
            RaceError::NotAsked(c) => {
                write!(f, "no replications were asked for candidate {c}")
            }
            RaceError::Pending(c) => {
                write!(f, "candidate {c} still has replications outstanding")
            }
            RaceError::WrongCount {
                candidate,
                expected,
                got,
            } => write!(
                f,
                "candidate {candidate} gave {got} values where {expected} were asked for"
            ),
            RaceError::NotFinite {
                candidate,
                replication,
            } => write!(
                f,
                "candidate {candidate} gave a value that is not a number in replication {replication}"
            ),
            RaceError::Replications {
                candidate,
                start,
                end,
                offset,
            } => write!(
                f,
                "the clean-up's replications {start}..{end} of candidate {candidate}, moved past the race's {offset}, cannot be numbered"
            ),
            RaceError::Select(e) => write!(f, "the clean-up: {e}"),
        }
    }
}

impl std::error::Error for RaceError {}

/// P(T ≥ t) for Student's t with `df` degrees of freedom: ½ I_x(df/2, ½) with x = df/(df + t²)
/// for t ≥ 0 (the t distribution's tail as a regularised incomplete beta function), and
/// 1 − P(T ≥ −t) below 0.
pub fn t_upper_tail(t: f64, df: f64) -> f64 {
    if t.is_nan() || df.is_nan() || df <= 0.0 {
        return f64::NAN;
    }
    let tail = |t: f64| 0.5 * beta_cdf(df / (df + t * t), 0.5 * df, 0.5);
    if t >= 0.0 { tail(t) } else { 1.0 - tail(-t) }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;
    use stochcraft_random::math::{cos, ln};
    use stochcraft_random::{Rng, SplitMix64};

    fn normal(seed: u64, a: u64, b: u64) -> f64 {
        let mut sm = SplitMix64::new(
            seed ^ a.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ b.wrapping_mul(0xd1b5_4a32_d192_ed03),
        );
        let mut rng = Rng::new(sm.next_u64());
        let u1 = rng.uniform_open0();
        let u2 = rng.uniform();
        (-2.0 * ln(u1)).sqrt() * cos(2.0 * std::f64::consts::PI * u2)
    }

    fn normals(seed: u64, means: Vec<f64>) -> impl FnMut(usize, Range<u64>) -> Vec<f64> {
        move |s, reps| reps.map(|j| means[s] + normal(seed, s as u64, j)).collect()
    }

    #[test]
    fn the_t_tail_matches_closed_forms_and_the_table() {
        // df = 1 (Cauchy): P(T ≥ 1) = 1/4. df = 2: P(T ≥ t) = ½ (1 − t/√(t² + 2)).
        assert!((t_upper_tail(1.0, 1.0) - 0.25).abs() < 1e-12);
        let two = 0.5 * (1.0 - 2.0 / 6f64.sqrt());
        assert!((t_upper_tail(2.0, 2.0) - two).abs() < 1e-12);
        assert!((t_upper_tail(0.0, 7.0) - 0.5).abs() < 1e-12);
        assert!((t_upper_tail(-1.0, 1.0) - 0.75).abs() < 1e-12);
        // The two-sided 95% table: each tail 0.025, to the table's three decimals.
        for df in [1, 4, 9, 20, 30, 60] {
            let Some(t) = stochcraft_stats::t_critical_95(df) else {
                panic!("no table value for {df}")
            };
            let p = t_upper_tail(t, df as f64);
            assert!((p - 0.025).abs() < 2e-4, "df {df}: {p}");
        }
        assert_eq!(t_upper_tail(f64::INFINITY, 5.0), 0.0);
    }

    #[test]
    fn the_level_is_bonferroni_over_the_planned_pairs_and_looks() {
        let r = Racing::new(0.5, Sense::Maximise, 10);
        // α = 0.05, half to the race, M − 1 = 9 others, L = 5 looks.
        assert!((r.level() - 0.025 / 45.0).abs() < 1e-15);
        assert!((r.cleanup_confidence() - 0.975).abs() < 1e-15);
        assert_eq!(r.max_replications(), 30);
    }

    #[test]
    fn no_randomness_is_compared_exactly() {
        // Stated without randomness. Minimise; candidate 2 is best; 1 is within δ of it and
        // stays; 0 and 3 are worse by more than δ and are dropped on their first stage.
        let means = [3.0, 1.3, 1.0, 2.0];
        let settings = Racing {
            deterministic: true,
            ..Racing::new(0.5, Sense::Minimise, 4)
        };
        let a = race(&settings, 4, |s, reps| reps.map(|_| means[s]).collect());
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.outcome, Outcome::ComparedExactly);
        assert_eq!(a.chosen, Some(2));
        assert_eq!(a.finalists, vec![2, 1]);
        assert_eq!(a.dropped, 2);
        assert_eq!(a.confidence, None);
        assert!(a.to_string().contains("compared exactly"), "{a}");
        // The same values without the statement: nobody is dropped on a flat difference, and
        // the clean-up proves nothing.
        let settings = Racing::new(0.5, Sense::Minimise, 4);
        let a = race(&settings, 4, |s, reps| reps.map(|_| means[s]).collect());
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!((a.outcome, a.confidence), (Outcome::NotProven, None));
        assert_eq!(a.dropped, 0);
        assert_eq!(a.finalists, vec![2, 1, 3, 0]);
        assert!(a.selection.as_ref().is_some_and(|s| s.flat));
        assert!(a.to_string().contains("could not tell them apart"), "{a}");
    }

    /// A rare event under common random numbers: both candidates vary alike, candidate 0 is 2
    /// above a shared normal, candidate 1 is the shared normal itself except in replications
    /// numbered 37 mod 50, where it is 200 above it, so its mean is the higher (by 2). Maximising
    /// with δ = 0.5, the race's first stage (replications 0 to 9) and the clean-up's
    /// (10 to 19) both see a constant difference of 2 > δ, which must neither drop candidate 1
    /// nor let the clean-up pick candidate 0.
    #[test]
    fn a_flat_difference_drops_nobody() {
        let sample = |s: usize, reps: Range<u64>| -> Vec<f64> {
            reps.map(|j| {
                let base = normal(41, 0, j);
                match s {
                    0 => base + 2.0,
                    _ if j % 50 == 37 => base + 200.0,
                    _ => base,
                }
            })
            .collect()
        };
        let settings = Racing {
            stages: 1,
            max_cleanup_runs: Some(400),
            ..Racing::new(0.5, Sense::Maximise, 2)
        };
        let a = race(&settings, 2, sample);
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.dropped, 0, "{a:?}");
        assert_eq!(a.finalists, vec![0, 1]);
        assert_eq!((a.outcome, a.confidence), (Outcome::NotProven, None));
        assert!(a.selection.as_ref().is_some_and(|s| s.flat), "{a:?}");
        // With the (here false) statement of no randomness, the constant difference is taken as
        // exact and candidate 1 is dropped: the flag is the caller's responsibility.
        let stated = Racing {
            deterministic: true,
            ..settings
        };
        let a = race(&stated, 2, sample);
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.dropped, 1, "{a:?}");
    }

    #[test]
    fn the_clean_up_uses_only_fresh_replications() {
        let settings = Racing::new(0.3, Sense::Maximise, 8);
        let means = vec![0.0, 0.3, 1.5, 0.2, -2.0, 0.1, 0.0, 3.0];
        let mut base = normals(3, means);
        let mut race_reps: Vec<Range<u64>> = Vec::new();
        let mut clean_reps: Vec<Range<u64>> = Vec::new();
        let mut r = match Race::new(settings) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        for _ in 0..8 {
            assert!(r.add().is_ok());
        }
        let ran = r.run(&mut |c, reps: Range<u64>| {
            race_reps.push(reps.clone());
            base(c, reps)
        });
        assert!(ran.is_ok());
        let a = r.clean_up(|c, reps: Range<u64>| {
            clean_reps.push(reps.clone());
            base(c, reps)
        });
        let Ok(a) = a else { panic!("{a:?}") };
        let race_max = race_reps.iter().map(|r| r.end).max().unwrap_or(0);
        let clean_min = clean_reps.iter().map(|r| r.start).min().unwrap_or(0);
        assert!(!clean_reps.is_empty());
        assert!(clean_min >= race_max, "{clean_min} < {race_max}");
        assert_eq!(a.fresh_from, race_max);
        assert_eq!(a.outcome, Outcome::Selected);
        assert_eq!(a.chosen, Some(7));
        assert!(a.dropped > 0, "{a:?}");
        assert!(a.to_string().contains("does not prove"), "{a}");
    }

    #[test]
    fn a_spent_budget_still_cleans_up_or_says_not_proven() {
        let means = vec![0.0, 0.5, 0.2, 0.1, 0.0, 0.3];
        let settings = Racing {
            max_race_runs: Some(6 * 10 + 7),
            ..Racing::new(0.5, Sense::Maximise, 8)
        };
        let mut r = match Race::new(settings) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        for _ in 0..6 {
            assert!(r.add().is_ok());
        }
        let mut sample = normals(9, means.clone());
        assert!(r.run(&mut sample).is_ok());
        assert!(r.budget_spent());
        assert_eq!(r.runs(), 65);
        // A candidate added after the budget is spent is tried but never run.
        assert_eq!(r.add(), Ok(6));
        assert_eq!(r.status(6), Some(Status::NotRun));
        let a = r.clean_up(&mut sample);
        let Ok(a) = a else { panic!("{a:?}") };
        assert!(a.race_budget_spent);
        assert_eq!((a.tried, a.untested.clone()), (7, vec![6]));
        assert!(a.selection.is_some());
        assert!(
            a.to_string().starts_with("The racing budget ran out"),
            "{a}"
        );
        // A clean-up budget short of KN's first stage: not proven, nothing claimed.
        let short = Racing {
            max_cleanup_runs: Some(9),
            ..settings
        };
        let a = race(&short, 6, normals(9, means));
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!((a.outcome, a.confidence), (Outcome::NotProven, None));
        assert!(a.selection.is_none());
        assert!(a.chosen.is_some());
        assert!(
            a.to_string().starts_with("The racing budget ran out"),
            "{a}"
        );
    }

    #[test]
    fn every_contender_is_a_finalist_unless_a_cap_cuts_and_says_so() {
        // Minimise, no randomness, all within δ = 0.5 of the best: none is dropped.
        let means = [1.2, 1.0, 1.3, 1.1];
        let exact = |s: usize, reps: Range<u64>| reps.map(|_| means[s]).collect::<Vec<f64>>();
        let settings = Racing {
            deterministic: true,
            ..Racing::new(0.5, Sense::Minimise, 4)
        };
        let a = race(&settings, 4, exact);
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.finalists, vec![1, 3, 0, 2]);
        assert!(a.left_out.is_empty());
        assert!(!a.to_string().contains("left out"), "{a}");
        // A cap of 2: the two best means go to the clean-up, the others are listed as left out.
        let capped = Racing {
            finalists: Some(2),
            ..settings
        };
        let a = race(&capped, 4, exact);
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(
            (a.finalists.clone(), a.left_out.clone()),
            (vec![1, 3], vec![0, 2])
        );
        assert_eq!(a.chosen, Some(1));
        let words = a.to_string();
        assert!(
            words.contains("left out 2 candidates still in contention (0, 2)"),
            "{words}"
        );
        assert!(words.contains("covers only the finalists"), "{words}");
        // With randomness, the stated confidence is for the finalists only.
        let a = race(&capped, 4, normals(5, vec![1.2, 0.0, 1.3, 1.1]));
        let Ok(a) = a else { panic!("{a:?}") };
        assert!(a.finalists.len() <= 2, "{a:?}");
        assert_eq!(a.confidence, Some(0.95));
        assert_eq!(a.finalists.len() + a.left_out.len(), 4 - a.dropped, "{a:?}");
        let bad = Racing {
            finalists: Some(0),
            ..settings
        };
        assert_eq!(Race::new(bad).err(), Some(RaceError::Setting("finalists")));
    }

    #[test]
    fn the_same_seed_gives_the_same_answer() {
        let settings = Racing::new(0.3, Sense::Minimise, 12);
        let means: Vec<f64> = (0..12).map(|i| f64::from(i) * 0.2).collect();
        let a = race(&settings, 12, normals(21, means.clone()));
        let b = race(&settings, 12, normals(21, means));
        assert!(a.is_ok());
        assert_eq!(a, b);
    }

    #[test]
    fn bad_settings_and_bad_values_are_refused() {
        let bad = Racing::new(0.0, Sense::Maximise, 3);
        assert_eq!(Race::new(bad).err(), Some(RaceError::Setting("delta")));
        let bad = Racing {
            racing_share: 1.0,
            ..Racing::new(1.0, Sense::Maximise, 3)
        };
        assert_eq!(
            Race::new(bad).err(),
            Some(RaceError::Setting("racing_share"))
        );
        let Ok(mut r) = Race::new(Racing::new(1.0, Sense::Maximise, 2)) else {
            panic!("settings")
        };
        assert_eq!(r.tell(0, &[]), Err(RaceError::UnknownCandidate(0)));
        assert_eq!(r.add(), Ok(0));
        assert_eq!(r.tell(0, &[]), Err(RaceError::NotAsked(0)));
        assert_eq!(r.add(), Ok(1));
        assert_eq!(r.add(), Err(RaceError::TooManyCandidates { max: 2 }));
        let Some(q) = r.ask() else {
            panic!("no request")
        };
        assert_eq!(
            q,
            Request {
                candidate: 0,
                replications: 0..10
            }
        );
        assert_eq!(
            r.tell(0, &[0.0]),
            Err(RaceError::WrongCount {
                candidate: 0,
                expected: 10,
                got: 1
            })
        );
        assert_eq!(
            r.clean_up(|_, _| Vec::new()).err(),
            Some(RaceError::Pending(0))
        );
        let mut v = vec![0.0; 10];
        v[4] = f64::NAN;
        assert_eq!(
            r.tell(0, &v),
            Err(RaceError::NotFinite {
                candidate: 0,
                replication: 4
            })
        );
    }

    fn new_race(settings: Racing) -> Race {
        match Race::new(settings) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        }
    }

    #[test]
    fn a_clean_up_confidence_kn_would_refuse_is_refused_up_front() {
        // 1 − α_clean = 1 − 0.8 × 0.8 = 0.36, below the 1/2 KN needs for two finalists.
        let low = Racing {
            confidence: 0.2,
            racing_share: 0.2,
            ..Racing::new(1.0, Sense::Maximise, 3)
        };
        assert_eq!(Race::new(low).err(), Some(RaceError::Setting("confidence")));
        // One finalist at most (one candidate, or a cap of one): KN gets one system, valid.
        assert!(
            Race::new(Racing {
                max_candidates: 1,
                ..low
            })
            .is_ok()
        );
        assert!(
            Race::new(Racing {
                finalists: Some(1),
                ..low
            })
            .is_ok()
        );
        // A split that rounds the clean-up's confidence to 1, which KN refuses.
        let near = Racing {
            confidence: 1.0 - f64::EPSILON / 2.0,
            ..Racing::new(1.0, Sense::Maximise, 3)
        };
        assert_eq!(near.cleanup_confidence(), 1.0);
        assert_eq!(
            Race::new(near).err(),
            Some(RaceError::Setting("confidence"))
        );
    }

    #[test]
    fn the_run_cap_refuses_a_total_that_overflows() {
        let big = Racing {
            first_stage: 1 << 63,
            stages: 1,
            max_race_runs: Some(u64::MAX),
            ..Racing::new(1.0, Sense::Maximise, 2)
        };
        let mut r = new_race(big);
        assert_eq!((r.add(), r.add()), (Ok(0), Ok(1)));
        assert!(r.ask().is_some());
        assert_eq!(r.ask(), None);
        assert!(r.budget_spent());
        assert_eq!(r.runs(), 1 << 63);
        assert_eq!(r.status(1), Some(Status::NotRun));
        // An exact fit still passes.
        let fit = Racing {
            first_stage: 10,
            stages: 1,
            max_race_runs: Some(20),
            ..Racing::new(1.0, Sense::Maximise, 2)
        };
        let mut r = new_race(fit);
        assert_eq!((r.add(), r.add()), (Ok(0), Ok(1)));
        assert!(r.ask().is_some() && r.ask().is_some());
        assert_eq!(r.ask(), None);
        assert!(!r.budget_spent());
        assert_eq!(r.runs(), 20);
    }

    #[test]
    fn large_finite_values_keep_finite_distinct_means() {
        let settings = Racing {
            first_stage: 2,
            stages: 1,
            finalists: Some(1),
            ..Racing::new(1e308, Sense::Maximise, 2)
        };
        let values = [1e308, 1.5e308];
        let a = race(&settings, 2, |c, reps| reps.map(|_| values[c]).collect());
        let Ok(a) = a else { panic!("{a:?}") };
        // A summed total would be infinite for both and tie; the means are 1e308 and 1.5e308.
        assert_eq!(
            (a.finalists.clone(), a.left_out.clone()),
            (vec![1], vec![0])
        );
        assert_eq!(a.chosen, Some(1));
        let mut r = new_race(settings);
        assert_eq!((r.add(), r.add()), (Ok(0), Ok(1)));
        let mut sample = |c: usize, reps: Range<u64>| reps.map(|_| values[c]).collect::<Vec<_>>();
        assert!(r.run(&mut sample).is_ok());
        assert_eq!((r.mean(0), r.mean(1)), (Some(1e308), Some(1.5e308)));
    }

    #[test]
    fn a_failed_tell_leaves_the_request_for_run_to_retry() {
        let settings = Racing {
            stages: 1,
            ..Racing::new(1.0, Sense::Maximise, 1)
        };
        let mut r = new_race(settings);
        assert_eq!(r.add(), Ok(0));
        let mut short = |_: usize, _: Range<u64>| vec![0.0];
        assert_eq!(
            r.run(&mut short),
            Err(RaceError::WrongCount {
                candidate: 0,
                expected: 10,
                got: 1
            })
        );
        assert_eq!(r.runs(), 10);
        let mut asked = Vec::new();
        let mut good = |_: usize, reps: Range<u64>| {
            asked.push(reps.clone());
            reps.map(|j| j as f64).collect::<Vec<_>>()
        };
        assert!(r.run(&mut good).is_ok());
        // The same range again, not charged twice.
        assert_eq!(asked, vec![0..10]);
        assert_eq!((r.runs(), r.replications(0)), (10, Some(10)));
        assert!(
            r.clean_up(|_, reps| reps.map(|j| j as f64).collect())
                .is_ok()
        );
        // Through ask and tell, corrected values are accepted for the same request.
        let mut r = new_race(settings);
        assert_eq!(r.add(), Ok(0));
        let Some(q) = r.ask() else {
            panic!("no request")
        };
        assert!(r.tell(0, &[1.0]).is_err());
        let values: Vec<f64> = q.replications.map(|j| j as f64).collect();
        assert_eq!(r.tell(0, &values), Ok(()));
        assert_eq!(r.status(0), Some(Status::Finished));
    }

    #[test]
    fn an_unreliable_spread_drops_nobody() {
        // Paired differences [0, 4e-200] with δ = 1e-200: squaring them unscaled underflows to
        // a variance of 0; scaled, t = 0.5, far from the level.
        let settings = Racing {
            first_stage: 2,
            stages: 1,
            ..Racing::new(1e-200, Sense::Maximise, 2)
        };
        let values = [[0.0, 0.0], [0.0, -4e-200]];
        let a = race(&settings, 2, |c, reps| {
            reps.map(|j| values[c][(j % 2) as usize]).collect()
        });
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.dropped, 0, "{a:?}");
        assert_eq!(a.finalists, vec![0, 1]);
        // Differences that overflow (1.7e308 − (−1.7e308)): no elimination either.
        let wide = Racing {
            first_stage: 2,
            stages: 1,
            ..Racing::new(1.0, Sense::Maximise, 2)
        };
        let values = [[1.7e308, 1.6e308], [-1.7e308, -1.6e308]];
        let a = race(&wide, 2, |c, reps| {
            reps.map(|j| values[c][(j % 2) as usize]).collect()
        });
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.dropped, 0, "{a:?}");
    }

    #[test]
    fn clean_up_ranges_that_cannot_be_numbered_are_refused() {
        let settings = Racing {
            first_stage: 2,
            stages: 1,
            cleanup_first_stage: u64::MAX,
            ..Racing::new(1.0, Sense::Maximise, 1)
        };
        let mut r = new_race(settings);
        assert_eq!(r.add(), Ok(0));
        assert!(r.run(&mut normals(1, vec![0.0])).is_ok());
        assert_eq!(r.fresh_from(), 2);
        let mut called = false;
        let e = r.clean_up(|_, _| {
            called = true;
            Vec::new()
        });
        assert_eq!(
            e,
            Err(RaceError::Replications {
                candidate: 0,
                start: 0,
                end: u64::MAX,
                offset: 2
            })
        );
        assert!(!called);
    }

    #[test]
    fn candidates_never_raced_are_listed_and_not_covered() {
        // The budget pays for the first candidate's first stage only.
        let settings = Racing {
            max_race_runs: Some(10),
            ..Racing::new(0.5, Sense::Maximise, 3)
        };
        let a = race(&settings, 3, normals(2, vec![0.0, 5.0, 5.0]));
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.untested, vec![1, 2]);
        assert_eq!(a.finalists, vec![0]);
        let words = a.to_string();
        assert!(
            words.contains(
                "Never raced: candidates 1, 2; the confidence covers only the candidates that raced."
            ),
            "{words}"
        );
        // A candidate still waiting, with nothing outstanding, when the clean-up is called.
        let mut r = new_race(Racing::new(0.5, Sense::Maximise, 2));
        assert_eq!((r.add(), r.add()), (Ok(0), Ok(1)));
        let Some(q) = r.ask() else {
            panic!("no request")
        };
        let values: Vec<f64> = q.replications.map(|j| j as f64).collect();
        assert_eq!(r.tell(q.candidate, &values), Ok(()));
        let a = r.clean_up(normals(3, vec![0.0, 0.0]));
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.untested, vec![1]);
        assert!(a.to_string().contains("Never raced: candidates 1;"), "{a}");
    }
}
