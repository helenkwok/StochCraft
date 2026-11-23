//! Racing the candidates a search proposes, then the clean-up of the finalists by KN.
//!
//! A search (M9's own, or an outside one such as an ask-and-tell optimiser) proposes candidates
//! over time. Each candidate gets a first stage of replications, then more in stages while it
//! stays in contention. This is racing (Birattari, Stützle, Paquete and Varrentrapp 2002), with a
//! paired t interval in place of their Friedman test, because every candidate is run on the same
//! replications (common random numbers) and so two candidates pair replication by replication.
//! When the race is over, the k best candidates still in contention are run again on fresh
//! replications that no step of the race used, and KN ([`crate::select::select_best`]) selects
//! among them.
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
//! **The clean-up.** The finalists are the k candidates in contention with the best means (fewer
//! when fewer are left). KN runs on replications numbered from the first index after every one
//! the race asked for, so its data are independent of the race and its guarantee holds for the
//! set it is given, at confidence 1 − α_clean. With α_race = share·α and
//! α_clean = (1 − share)·α (half each by default), Bonferroni's inequality bounds the chance that
//! the race drops the best or that KN, given the best among the finalists, fails to choose it or
//! one within δ of it, by α. The answer claims only what KN proves: of these k, the chosen one is
//! the best or within δ of it, with 1 − α confidence (KN's own level, 1 − α_clean, is higher).
//! The cut to k finalists by their means is not covered: a candidate still in contention but left
//! out may be better, and the answer says so, with the number of candidates the search tried.
//!
//! **Assumptions.** Observations are normal and independent across replications, as KN and the t
//! interval assume.
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
    /// k, the most candidates the clean-up takes (default 5, at least 1).
    pub finalists: usize,
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
            finalists: 5,
            cleanup_first_stage: 10,
            sense,
            sampling: Sampling::Common,
            max_race_runs: None,
            max_cleanup_runs: None,
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
        if self.finalists == 0 {
            return bad("finalists");
        }
        if self.cleanup_first_stage < 2 {
            return bad("cleanup_first_stage");
        }
        // KN's range for up to k systems: 1/k < 1 − α_clean.
        if self.finalists > 1 && self.cleanup_confidence() <= 1.0 / self.finalists as f64 {
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
    /// Never run: the race's budget ran out before its first stage.
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
    /// The sum of `values`, for the mean.
    sum: f64,
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
/// outstanding at once (one per candidate), so they can run in parallel; the order of the tells
/// does not change what a candidate is compared on, only when.
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
            sum: 0.0,
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
            .is_some_and(|max| self.runs.saturating_add(size) > max)
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
        entry.values.extend_from_slice(values);
        entry.sum += values.iter().sum::<f64>();
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
    /// replications)` giving one value per replication.
    pub fn run<F>(&mut self, sample: &mut F) -> Result<(), RaceError>
    where
        F: FnMut(usize, Range<u64>) -> Vec<f64>,
    {
        while let Some(request) = self.ask() {
            let values = sample(request.candidate, request.replications.clone());
            self.tell(request.candidate, &values)?;
        }
        Ok(())
    }

    /// The candidate's mean over its replications, as observed, or `None` before its
    /// first stage.
    pub fn mean(&self, candidate: usize) -> Option<f64> {
        let e = self.candidates.get(candidate)?;
        (!e.values.is_empty()).then(|| e.sum / e.values.len() as f64)
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
        let nf = n as f64;
        let mean = d.iter().sum::<f64>() / nf;
        if mean.is_nan() || mean <= self.settings.delta {
            return None;
        }
        let constant = d.iter().all(|v| Some(v) == d.first());
        if constant {
            // No variation in the differences: the difference is known exactly.
            return Some(n as u64);
        }
        let var = d.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / (nf - 1.0);
        if var.is_nan() || var <= 0.0 {
            return Some(n as u64);
        }
        let t = (mean - self.settings.delta) / (var / nf).sqrt();
        (t_upper_tail(t, nf - 1.0) <= self.level).then_some(n as u64)
    }

    /// Ends the race: KN among the k best candidates in contention, on replications numbered from
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
        let k = self.settings.finalists.min(ranked.len());
        let left_out = ranked.split_off(k);
        let finalists = ranked;
        let count =
            |f: fn(&Status) -> bool| self.candidates.iter().filter(|e| f(&e.status)).count();
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
            not_run: count(|s| matches!(s, Status::NotRun | Status::Waiting)),
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
        };
        let start = self.fresh_from;
        let selection = select_best(&kn, finalists.len(), |p, reps| {
            finalists.get(p).map_or_else(Vec::new, |&c| {
                sample(c, reps.start + start..reps.end + start)
            })
        })
        .map_err(|e| match e {
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
                replication: replication + start,
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
    /// [`Outcome::ComparedExactly`] without randomness, or [`Outcome::NotProven`] when the
    /// clean-up's budget could not pay for KN or ran out, or nothing was left to clean up.
    pub outcome: Outcome,
    /// The chosen candidate; when not proven, the best-looking finalist (by KN's means, or by the
    /// race's when KN did not run). `None` when no candidate finished its first stage.
    pub chosen: Option<usize>,
    /// The candidates the clean-up took, best race mean first.
    pub finalists: Vec<usize>,
    /// Candidates still in contention but beyond the k finalists.
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
    /// Candidates never run (the race's budget ran out first).
    pub not_run: usize,
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
                "Of these {k} finalists ({list}), candidate {chosen} is the best: every replication gave the same values, so they were compared exactly and no probability is stated."
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
        if !self.left_out.is_empty() {
            write!(
                f,
                ", and {} still in contention were left out of the clean-up",
                self.left_out.len()
            )?;
        }
        if self.not_run > 0 {
            write!(f, ", and {} were never run", self.not_run)?;
        }
        write!(
            f,
            ". This does not prove that none of the others is better."
        )
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
        // Minimise; candidate 2 is best; 1 is within δ of it and stays; 0 and 3 are worse by more
        // than δ and are dropped on their first stage.
        let means = [3.0, 1.3, 1.0, 2.0];
        let settings = Racing::new(0.5, Sense::Minimise, 4);
        let a = race(&settings, 4, |s, reps| reps.map(|_| means[s]).collect());
        let Ok(a) = a else { panic!("{a:?}") };
        assert_eq!(a.outcome, Outcome::ComparedExactly);
        assert_eq!(a.chosen, Some(2));
        assert_eq!(a.finalists, vec![2, 1]);
        assert_eq!(a.dropped, 2);
        assert_eq!(a.confidence, None);
        assert!(a.to_string().contains("compared exactly"), "{a}");
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
        assert_eq!((a.tried, a.not_run), (7, 1));
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
}
