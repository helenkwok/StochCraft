//! Searching numbers in ranges by Nelder–Mead on noisy estimates, then the clean-up by KN.
//!
//! A design here is a [`Point`]: a group (the values of the other decisions, whole numbers or
//! rule levels, which this search does not move) and a point of the unit cube, one coordinate per
//! continuous decision, which the caller maps to its range. The objective is noisy and observed
//! one replication at a time through the caller's sampler, as in [`crate::compass`].
//!
//! **The method.** The simplex method of J. A. Nelder and R. Mead ("A simplex method for function
//! minimization", *The Computer Journal* 7(4), 1965): reflection (coefficient 1), expansion (2),
//! contraction (½, outside or inside) and shrink, with the two changes R. R. Barton and
//! J. S. Ivey found best for noisy responses ("Nelder-Mead simplex modifications for simulation
//! optimization", *Management Science* 42(7), 1996), their RS + S9: the best vertex is evaluated
//! again at each shrink, and a shrink reduces the simplex by 10% (coefficient 0.9) rather than by
//! half. Plain Nelder–Mead on a noisy response shrinks too early around a lucky vertex; these
//! two changes let it recover. Written from the papers' published descriptions; no code of any
//! implementation was used.
//!
//! **Common random numbers.** Every point runs replication i on the same seed, so the vertices
//! are compared on the same random numbers. Each simplex has a level L (the replications per
//! vertex, starting at the replications per visit n), and every vertex is compared on its first
//! L replications only, so the comparisons stay paired. The re-evaluation at a shrink adds n
//! replications to the best vertex and raises L to match, so the shrunk vertices are run on the
//! same L replications as the best (Barton and Ivey's re-evaluation, kept paired). Points outside
//! the unit cube are moved onto its faces.
//!
//! **Stopping.** The spread of the vertices' values is a poor stopping rule under noise (it never
//! falls below the noise), so a simplex stops when its size, the largest distance of a vertex from
//! the best along any coordinate, is below [`NelderMead::tolerance`] (a share of each range), or
//! after [`NelderMead::most_steps`] steps. It then restarts from its best vertex with a fresh
//! simplex of the first size, [`NelderMead::restarts`] times (once by default), as Nelder and Mead
//! suggest to guard against a collapsed simplex; the last simplex's vertices go to the clean-up.
//!
//! **Groups.** With several starts (one per value of the other decisions), the searches take one
//! step each in turn, so the path of each does not depend on the budget, and a search stopped
//! after S runs is repeated exactly with a budget of S (as in [`crate::compass`]).
//!
//! **What it claims.** Nelder–Mead is a local method with no convergence guarantee under noise;
//! the clean-up's statement covers the points compared at the end, not the whole range.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use crate::compass::{
    self, Answer, Compass, CompassError, Design, Ended, Finalist, StartReport, Stream, TracePoint,
};

/// A design: the values of the decisions this search holds (`group`), and a point of the unit
/// cube for the continuous decisions (`x`, each coordinate from 0 to 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub group: Vec<i64>,
    pub x: Vec<f64>,
}

impl Design for Point {
    fn show(&self) -> String {
        self.group
            .iter()
            .map(i64::to_string)
            .chain(self.x.iter().map(f64::to_string))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// A point as a key (its coordinates' bits).
type Key = (Vec<i64>, Vec<u64>);

fn key(p: &Point) -> Key {
    (p.group.clone(), p.x.iter().map(|v| v.to_bits()).collect())
}

/// The settings of the simplex, beside the [`Compass`] settings the clean-up and the budget use.
#[derive(Debug, Clone, PartialEq)]
pub struct NelderMead {
    /// The first simplex's edge, as a share of each range (default 0.25, at most 0.5).
    pub initial: f64,
    /// A simplex smaller than this share of each range stops (default 0.01).
    pub tolerance: f64,
    /// Fresh simplexes from the best vertex after the first stops (default 1).
    pub restarts: u32,
    /// The shrink coefficient (default 0.9, Barton and Ivey's S9).
    pub shrink: f64,
    /// The most steps of one simplex (default 200).
    pub most_steps: u64,
}

impl Default for NelderMead {
    fn default() -> Self {
        Self {
            initial: 0.25,
            tolerance: 0.01,
            restarts: 1,
            shrink: 0.9,
            most_steps: 200,
        }
    }
}

impl NelderMead {
    fn validate(&self) -> Result<(), CompassError> {
        let bad = |name: &'static str| Err(CompassError::Setting(name));
        if !(self.initial > 0.0 && self.initial <= 0.5) {
            return bad("initial");
        }
        if !(self.tolerance > 0.0 && self.tolerance < self.initial) {
            return bad("tolerance");
        }
        if !(self.shrink > 0.0 && self.shrink < 1.0) {
            return bad("shrink");
        }
        if self.most_steps == 0 {
            return bad("most_steps");
        }
        Ok(())
    }
}

/// The search's replications, shared by every start: each point's rows for replications 0 to
/// n − 1 of the search stream.
struct Evals<'a, F> {
    setup: &'a Compass,
    sample: F,
    width: usize,
    cache: BTreeMap<Key, Vec<Vec<f64>>>,
    runs: u64,
    budget: u64,
}

impl<F> Evals<'_, F>
where
    F: FnMut(Stream, &Point, Range<u64>) -> Result<Vec<Vec<f64>>, String>,
{
    fn have(&self, p: &Point) -> u64 {
        self.cache.get(&key(p)).map_or(0, |r| r.len() as u64)
    }

    /// Brings `p` to `n` replications: `Ok(false)`, with nothing run, when the budget cannot pay.
    fn top_up(&mut self, p: &Point, n: u64) -> Result<bool, CompassError> {
        let have = self.have(p);
        if have >= n {
            return Ok(true);
        }
        let need = n - have;
        let total = match self.runs.checked_add(need) {
            Some(t) if t <= self.budget => t,
            _ => return Ok(false),
        };
        let rows = (self.sample)(Stream::Search, p, have..n).map_err(CompassError::Sampler)?;
        if rows.len() as u64 != need || rows.iter().any(|r| r.len() != self.width) {
            return Err(CompassError::WrongCount {
                design: p.show(),
                expected: need,
                got: rows.len() as u64,
            });
        }
        if let Some(j) = rows.iter().position(|r| r.iter().any(|v| !v.is_finite())) {
            return Err(CompassError::NotFinite {
                design: p.show(),
                replication: have + j as u64,
            });
        }
        self.runs = total;
        self.cache.entry(key(p)).or_default().extend(rows);
        Ok(true)
    }

    /// Each value's mean over the first `level` replications of `p` (all of them for `None`).
    fn means(&self, p: &Point, level: Option<u64>) -> Option<Vec<f64>> {
        let rows = self.cache.get(&key(p))?;
        let n = level.map_or(rows.len(), |l| (l as usize).min(rows.len()));
        if n == 0 {
            return None;
        }
        let mut m = vec![0.0; self.width];
        for (k, row) in rows.iter().take(n).enumerate() {
            let kf = (k + 1) as f64;
            for (mi, &v) in m.iter_mut().zip(row) {
                *mi += v / kf - *mi / kf;
            }
        }
        Some(m)
    }

    fn standing(&self, p: &Point, level: u64) -> Option<(bool, f64)> {
        compass::standing(self.setup, &self.means(p, Some(level))?)
    }

    /// Whether `a` stands strictly better than `b` on their first `level` replications.
    fn better(&self, a: &Point, b: &Point, level: u64) -> bool {
        match (self.standing(a, level), self.standing(b, level)) {
            (Some(_), None) => true,
            (None, _) => false,
            (Some(sa), Some(sb)) => sa.partial_cmp(&sb) == Some(std::cmp::Ordering::Greater),
        }
    }
}

/// One start's search.
struct Run {
    start: Point,
    /// The vertices, best first after each step.
    vertices: Vec<Vec<f64>>,
    /// Replications per vertex.
    level: u64,
    restarts_left: u32,
    /// Steps of the current simplex, and of all.
    in_simplex: u64,
    steps: u64,
    started: bool,
    done: bool,
    /// The last simplex stopped by its size (not by its step limit or the budget).
    converged: bool,
    visited: BTreeSet<Key>,
}

/// What one step of a start did.
enum StepEnd {
    /// It ran this many replications.
    Ran(u64),
    Exhausted,
}

fn clamp(x: Vec<f64>) -> Vec<f64> {
    x.into_iter().map(|v| v.clamp(0.0, 1.0)).collect()
}

/// The first simplex around `x`: `x` and one vertex `initial` along each coordinate (towards
/// the larger side of the range when the other has no room).
fn simplex_around(x: &[f64], initial: f64) -> Vec<Vec<f64>> {
    let mut out = vec![x.to_vec()];
    for j in 0..x.len() {
        let mut v = x.to_vec();
        if let Some(c) = v.get_mut(j) {
            *c = if *c + initial <= 1.0 {
                *c + initial
            } else {
                (*c - initial).max(0.0)
            };
        }
        out.push(v);
    }
    out
}

/// The largest distance of a vertex from the first (the best) along any coordinate.
fn size(vertices: &[Vec<f64>]) -> f64 {
    let Some(best) = vertices.first() else {
        return 0.0;
    };
    vertices
        .iter()
        .skip(1)
        .flat_map(|v| v.iter().zip(best).map(|(a, b)| (a - b).abs()))
        .fold(0.0, f64::max)
}

/// `a + t (b − a)`, coordinate by coordinate, moved into the unit cube.
fn along(a: &[f64], b: &[f64], t: f64) -> Vec<f64> {
    clamp(a.iter().zip(b).map(|(p, q)| p + t * (q - p)).collect())
}

impl Run {
    fn point(&self, x: &[f64]) -> Point {
        Point {
            group: self.start.group.clone(),
            x: x.to_vec(),
        }
    }

    /// Sorts the vertices best first (a stable sort: ties keep their order).
    fn sort<F>(&self, ev: &Evals<'_, F>, vertices: &mut [Vec<f64>])
    where
        F: FnMut(Stream, &Point, Range<u64>) -> Result<Vec<Vec<f64>>, String>,
    {
        vertices.sort_by(|a, b| {
            let (pa, pb) = (self.point(a), self.point(b));
            if ev.better(&pa, &pb, self.level) {
                std::cmp::Ordering::Less
            } else if ev.better(&pb, &pa, self.level) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
    }
}

/// Runs `x` of start `r` to its simplex's level: `Ok(false)` when the budget cannot pay.
fn eval<F>(ev: &mut Evals<'_, F>, r: &mut Run, x: &[f64]) -> Result<bool, CompassError>
where
    F: FnMut(Stream, &Point, Range<u64>) -> Result<Vec<Vec<f64>>, String>,
{
    let p = r.point(x);
    let fits = ev.top_up(&p, r.level)?;
    if fits {
        r.visited.insert(key(&p));
    }
    Ok(fits)
}

/// One step of start `r`. Nothing of the step is kept when the budget cannot pay for all of it.
fn step<F>(ev: &mut Evals<'_, F>, nm: &NelderMead, r: &mut Run) -> Result<StepEnd, CompassError>
where
    F: FnMut(Stream, &Point, Range<u64>) -> Result<Vec<Vec<f64>>, String>,
{
    let setup = ev.setup;
    if ev.runs >= ev.budget {
        return Ok(StepEnd::Exhausted);
    }
    let before = ev.runs;
    if !r.started {
        let mut v = simplex_around(&r.start.x, nm.initial);
        for x in &v {
            if !eval(ev, r, x)? {
                return Ok(StepEnd::Exhausted);
            }
        }
        r.sort(ev, &mut v);
        r.vertices = v;
        r.started = true;
        return Ok(StepEnd::Ran(ev.runs - before));
    }
    // A simplex that is small enough, or has taken its steps, restarts or ends.
    let small = size(&r.vertices) < nm.tolerance;
    if small || r.in_simplex >= nm.most_steps {
        if r.restarts_left == 0 {
            r.done = true;
            r.converged = small;
            return Ok(StepEnd::Ran(0));
        }
        let best = r
            .vertices
            .first()
            .cloned()
            .unwrap_or_else(|| r.start.x.clone());
        let mut v = simplex_around(&best, nm.initial);
        for x in &v {
            if !eval(ev, r, x)? {
                return Ok(StepEnd::Exhausted);
            }
        }
        r.sort(ev, &mut v);
        r.vertices = v;
        r.restarts_left -= 1;
        r.in_simplex = 0;
        r.steps += 1;
        return Ok(StepEnd::Ran(ev.runs - before));
    }
    let mut v = r.vertices.clone();
    let level = r.level;
    let n = v.len();
    let Some(worst) = v.last().cloned() else {
        r.done = true;
        return Ok(StepEnd::Ran(0));
    };
    let d = worst.len();
    // The centroid of every vertex but the worst.
    let mut centroid = vec![0.0; d];
    for x in v.iter().take(n - 1) {
        for (c, xi) in centroid.iter_mut().zip(x) {
            *c += xi / (n - 1) as f64;
        }
    }
    let group = r.start.group.clone();
    let at = |x: &[f64]| Point {
        group: group.clone(),
        x: x.to_vec(),
    };
    let better = |ev: &Evals<'_, F>, a: &[f64], b: &[f64]| ev.better(&at(a), &at(b), level);
    let reflected = along(&centroid, &worst, -1.0);
    if !eval(ev, r, &reflected)? {
        return Ok(StepEnd::Exhausted);
    }
    let best = v.first().cloned().unwrap_or_default();
    let second_worst = v.get(n.saturating_sub(2)).cloned().unwrap_or_default();
    let mut shrink = false;
    if better(&*ev, &reflected, &best) {
        let expanded = along(&centroid, &worst, -2.0);
        if !eval(ev, r, &expanded)? {
            return Ok(StepEnd::Exhausted);
        }
        let take = if better(&*ev, &expanded, &reflected) {
            expanded
        } else {
            reflected
        };
        if let Some(w) = v.last_mut() {
            *w = take;
        }
    } else if better(&*ev, &reflected, &second_worst) {
        if let Some(w) = v.last_mut() {
            *w = reflected;
        }
    } else {
        // Contract: outside when the reflection beats the worst, inside otherwise.
        let outside = better(&*ev, &reflected, &worst);
        let contracted = if outside {
            along(&centroid, &reflected, 0.5)
        } else {
            along(&centroid, &worst, 0.5)
        };
        if !eval(ev, r, &contracted)? {
            return Ok(StepEnd::Exhausted);
        }
        let against = if outside { &reflected } else { &worst };
        let accepted = if outside {
            !better(&*ev, against, &contracted)
        } else {
            better(&*ev, &contracted, against)
        };
        if accepted {
            if let Some(w) = v.last_mut() {
                *w = contracted;
            }
        } else {
            shrink = true;
        }
    }
    if shrink {
        // Barton and Ivey's RS + S9: the best vertex is evaluated again (n more replications,
        // the simplex's level raised to match, so its vertices stay paired), then every other
        // vertex moves 10% of the way towards it.
        let mut level_after = level;
        if !setup.exact {
            level_after = level.saturating_add(setup.per_visit);
            let p = r.point(&best);
            if !ev.top_up(&p, level_after)? {
                return Ok(StepEnd::Exhausted);
            }
        }
        let moved: Vec<Vec<f64>> = v
            .iter()
            .skip(1)
            .map(|x| along(&best, x, nm.shrink))
            .collect();
        let saved = r.level;
        r.level = level_after;
        for x in &moved {
            if !eval(ev, r, x)? {
                r.level = saved;
                return Ok(StepEnd::Exhausted);
            }
        }
        // The others in the simplex at the new level, the best included.
        v.truncate(1);
        v.extend(moved);
    }
    r.sort(ev, &mut v);
    r.vertices = v;
    r.in_simplex += 1;
    r.steps += 1;
    Ok(StepEnd::Ran(ev.runs - before))
}

/// Searches from each of `starts` (the starting design first; each start's `x` in the unit cube,
/// all of the same length, at least 1) with `sample(stream, point, replications)` giving, for each
/// replication, the objective followed by one value per constraint, then cleans up the last
/// simplexes' vertices and the starting design. `stop` is asked after each step that ran
/// replications; when it says so, the search ends and the clean-up runs. `setup`'s
/// `per_visit` is the replications per vertex; its COMPASS-only settings are not used.
pub fn optimise<F, S>(
    setup: &Compass,
    nm: &NelderMead,
    starts: &[Point],
    sample: F,
    mut stop: S,
) -> Result<Answer<Point>, CompassError>
where
    F: FnMut(Stream, &Point, Range<u64>) -> Result<Vec<Vec<f64>>, String>,
    S: FnMut() -> bool,
{
    setup.validate()?;
    nm.validate()?;
    let first = starts.first().ok_or(CompassError::NoStart)?.clone();
    let dims = first.x.len();
    for (i, s) in starts.iter().enumerate() {
        let inside = s.x.iter().all(|v| (0.0..=1.0).contains(v));
        if s.x.len() != dims || dims == 0 || !inside || s.group.len() != first.group.len() {
            return Err(CompassError::Start(i));
        }
    }
    let width = 1 + setup.constraints.len();
    let mut ev = Evals {
        setup,
        sample,
        width,
        cache: BTreeMap::new(),
        runs: 0,
        budget: setup.search_runs,
    };
    let level = setup.level(1);
    let mut runs: Vec<Run> = Vec::new();
    for s in starts {
        if !runs.iter().any(|r| r.start == *s) {
            runs.push(Run {
                start: s.clone(),
                vertices: Vec::new(),
                level,
                restarts_left: nm.restarts,
                in_simplex: 0,
                steps: 0,
                started: false,
                done: false,
                converged: false,
                visited: BTreeSet::new(),
            });
        }
    }
    let mut trace: Vec<TracePoint> = Vec::new();
    let best_mean = |ev: &Evals<'_, F>, runs: &[Run]| -> Option<f64> {
        let mut best: Option<(Point, u64)> = None;
        for r in runs.iter().filter(|r| r.started) {
            if let Some(x) = r.vertices.first() {
                let p = r.point(x);
                let replace = match &best {
                    None => true,
                    Some((b, _)) => ev.better(&p, b, r.level),
                };
                if replace {
                    best = Some((p, r.level));
                }
            }
        }
        best.and_then(|(p, l)| ev.means(&p, Some(l)).and_then(|m| m.first().copied()))
    };
    let ended = 'search: loop {
        if runs.iter().all(|r| r.done) {
            break Ended::LocalOptima;
        }
        for i in 0..runs.len() {
            let Some(r) = runs.get_mut(i) else { continue };
            if r.done {
                continue;
            }
            match step(&mut ev, nm, r)? {
                StepEnd::Exhausted => break 'search Ended::Budget,
                StepEnd::Ran(ran) => {
                    if ran > 0 {
                        trace.push(TracePoint {
                            runs: ev.runs,
                            best: best_mean(&ev, &runs),
                        });
                        if stop() {
                            break 'search Ended::Stopped;
                        }
                    }
                }
            }
        }
    };

    // The finalists: the vertices of each start's last simplex, best first, the start whose best
    // looks best first of all; then the starting design.
    let mut order: Vec<usize> = (0..runs.len())
        .filter(|&i| runs.get(i).is_some_and(|r| r.started))
        .collect();
    order.sort_by(|&a, &b| {
        let (Some(ra), Some(rb)) = (runs.get(a), runs.get(b)) else {
            return std::cmp::Ordering::Equal;
        };
        let (Some(xa), Some(xb)) = (ra.vertices.first(), rb.vertices.first()) else {
            return std::cmp::Ordering::Equal;
        };
        // Across starts the levels may differ: compare on the replications both have.
        let l = ra.level.min(rb.level);
        if ev.better(&ra.point(xa), &rb.point(xb), l) {
            std::cmp::Ordering::Less
        } else if ev.better(&rb.point(xb), &ra.point(xa), l) {
            std::cmp::Ordering::Greater
        } else {
            a.cmp(&b)
        }
    });
    let mut designs: Vec<Point> = Vec::new();
    let mut seen: BTreeSet<Key> = BTreeSet::new();
    for &i in &order {
        if let Some(r) = runs.get(i) {
            for x in &r.vertices {
                let p = r.point(x);
                if seen.insert(key(&p)) {
                    designs.push(p);
                }
            }
        }
    }
    if seen.insert(key(&first)) {
        designs.push(first);
    }
    let finalists: Vec<Finalist<Point>> = designs
        .into_iter()
        .map(|design| Finalist {
            search_mean: ev.means(&design, None).and_then(|m| m.first().copied()),
            search_replications: ev.have(&design),
            cleanup: BTreeMap::new(),
            design,
        })
        .collect();
    let reports: Vec<StartReport<Point>> = runs
        .iter()
        .map(|r| StartReport {
            start: r.start.clone(),
            end: r.vertices.first().map(|x| r.point(x)),
            local_optimum: r.converged,
            steps: r.steps,
            visited: r.visited.len(),
        })
        .collect();
    let search_runs = ev.runs;
    let tried = ev.cache.len();
    let mut finalists = finalists;
    if setup.exact {
        // Without randomness a finalist's single search replication is its value.
        for f in &mut finalists {
            if let Some(row) = ev.cache.get(&key(&f.design)).and_then(|r| r.first()) {
                f.cleanup.insert(0, row.clone());
            }
        }
    }
    let mut sample = ev.sample;
    let budget = setup.max_runs.saturating_sub(search_runs);
    let mut answer = compass::clean_up(setup, finalists, budget, &mut sample, width)?;
    answer.tried = tried;
    answer.search_runs = search_runs;
    answer.ended = ended;
    answer.starts = reports;
    answer.trace = trace;
    Ok(answer)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;
    use crate::select::{Outcome, Sense};
    use stochcraft_random::Rng;

    /// A noisy quadratic with its least value at `at` (in the unit cube): replication j adds the
    /// same uniform noise to every point (common random numbers), plus a little of its own.
    fn quadratic(
        at: Vec<f64>,
        noise: f64,
    ) -> impl FnMut(Stream, &Point, Range<u64>) -> Result<Vec<Vec<f64>>, String> {
        move |stream: Stream, p: &Point, reps: Range<u64>| {
            Ok(reps
                .map(|j| {
                    let salt = if stream == Stream::Search { 0 } else { 1 << 32 };
                    let mut common = Rng::keyed(j + salt, "common");
                    let mut own = Rng::keyed(j + salt, &format!("own {}", p.show()));
                    let f: f64 =
                        p.x.iter()
                            .zip(&at)
                            .map(|(x, a)| 10.0 * (x - a) * (x - a))
                            .sum();
                    let shared = common.uniform() - 0.5;
                    let alone = own.uniform() - 0.5;
                    vec![f + noise * shared + 0.05 * noise * alone]
                })
                .collect())
        }
    }

    fn setup(max_runs: u64) -> Compass {
        let mut c = Compass::new(0.05, Sense::Minimise, max_runs);
        c.per_visit = 5;
        c
    }

    fn start(x: Vec<f64>) -> Point {
        Point { group: vec![], x }
    }

    #[test]
    fn it_finds_the_least_value_of_a_noisy_bowl() {
        let a = optimise(
            &setup(4000),
            &NelderMead::default(),
            &[start(vec![0.9, 0.1])],
            quadratic(vec![0.3, 0.6], 1.0),
            || false,
        )
        .unwrap();
        let chosen = &a.finalists[a.chosen.unwrap()].design;
        let far = chosen
            .x
            .iter()
            .zip([0.3, 0.6])
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f64::max);
        assert!(far < 0.05, "{chosen:?} {a:?}");
        assert!(a.total_runs() <= 4000);
        // The starting design is a finalist.
        assert!(a.finalists.iter().any(|f| f.design.x == vec![0.9, 0.1]));
    }

    #[test]
    fn the_budget_is_never_exceeded_and_a_stopped_search_repeats() {
        for max in [0u64, 1, 7, 50, 333, 1000] {
            let a = optimise(
                &setup(max),
                &NelderMead::default(),
                &[start(vec![0.5])],
                quadratic(vec![0.2], 1.0),
                || false,
            )
            .unwrap();
            assert!(a.total_runs() <= max, "{max}: {}", a.total_runs());
        }
        // Stopped after a few steps, then repeated with the runs it used as its search budget.
        let mut steps = 0;
        let stopped = optimise(
            &setup(3000),
            &NelderMead::default(),
            &[start(vec![0.5]), start(vec![0.9])],
            quadratic(vec![0.2], 1.0),
            || {
                steps += 1;
                steps >= 6
            },
        )
        .unwrap();
        assert_eq!(stopped.ended, Ended::Stopped);
        let mut again = setup(3000);
        again.search_runs = stopped.search_runs;
        let repeated = optimise(
            &again,
            &NelderMead::default(),
            &[start(vec![0.5]), start(vec![0.9])],
            quadratic(vec![0.2], 1.0),
            || false,
        )
        .unwrap();
        assert_eq!(repeated.finalists, stopped.finalists);
        assert_eq!(repeated.chosen, stopped.chosen);
        assert_eq!(repeated.search_runs, stopped.search_runs);
    }

    #[test]
    fn groups_are_searched_each_and_meet_in_one_clean_up() {
        // Group 1 is 1 better everywhere: its best point is chosen.
        let sample = |_: Stream, p: &Point, reps: Range<u64>| -> Result<Vec<Vec<f64>>, String> {
            Ok(reps
                .map(|j| {
                    let mut rng = Rng::keyed(j, "noise");
                    let mut own = Rng::keyed(j, &format!("own {}", p.show()));
                    let x = p.x[0];
                    let shift = if p.group == [1] { -1.0 } else { 0.0 };
                    let noise = 0.2 * (rng.uniform() - 0.5) + 0.01 * (own.uniform() - 0.5);
                    vec![10.0 * (x - 0.7) * (x - 0.7) + shift + noise]
                })
                .collect())
        };
        let starts = [
            Point {
                group: vec![0],
                x: vec![0.1],
            },
            Point {
                group: vec![1],
                x: vec![0.1],
            },
        ];
        let a = optimise(
            &setup(5000),
            &NelderMead::default(),
            &starts,
            sample,
            || false,
        )
        .unwrap();
        let chosen = &a.finalists[a.chosen.unwrap()].design;
        assert_eq!(chosen.group, vec![1]);
        assert!((chosen.x[0] - 0.7).abs() < 0.05, "{chosen:?}");
        assert!(a.finalists.iter().any(|f| f.design.group == vec![0]));
        assert_eq!(a.starts.len(), 2);
        assert_eq!(a.outcome, Outcome::Selected);
    }

    #[test]
    fn without_randomness_it_compares_exactly() {
        let mut s = setup(500);
        s.exact = true;
        let a = optimise(
            &s,
            &NelderMead::default(),
            &[start(vec![0.0])],
            quadratic(vec![0.4], 0.0),
            || false,
        )
        .unwrap();
        assert_eq!(a.outcome, Outcome::ComparedExactly);
        let chosen = &a.finalists[a.chosen.unwrap()].design;
        assert!((chosen.x[0] - 0.4).abs() < 0.02, "{chosen:?}");
        // The simplex shrank below its tolerance, after its restart.
        assert!(a.starts[0].local_optimum, "{:?}", a.starts);
    }

    /// The same values without noise, but not stated exact: the clean-up (shared with COMPASS)
    /// does not take the finalists' constant differences as proof of no randomness.
    #[test]
    fn constant_values_without_the_statement_are_not_proven() {
        let a = optimise(
            &setup(500),
            &NelderMead::default(),
            &[start(vec![0.0])],
            quadratic(vec![0.4], 0.0),
            || false,
        )
        .unwrap();
        assert!(a.finalists.len() > 1, "{a:?}");
        assert_eq!(a.outcome, Outcome::NotProven, "{a:?}");
        assert_eq!(a.confidence, None);
        assert!(a.flat, "{a:?}");
    }

    #[test]
    fn a_value_that_is_not_a_number_is_refused() {
        let sample = |_: Stream, _: &Point, reps: Range<u64>| -> Result<Vec<Vec<f64>>, String> {
            Ok(reps.map(|_| vec![f64::NAN]).collect())
        };
        let e = optimise(
            &setup(500),
            &NelderMead::default(),
            &[start(vec![0.5])],
            sample,
            || false,
        );
        assert!(matches!(e, Err(CompassError::NotFinite { .. })), "{e:?}");
    }

    #[test]
    fn starts_outside_the_cube_are_refused() {
        let e = optimise(
            &setup(500),
            &NelderMead::default(),
            &[start(vec![1.5])],
            quadratic(vec![0.4], 0.0),
            || false,
        );
        assert_eq!(e.unwrap_err(), CompassError::Start(0));
    }
}
