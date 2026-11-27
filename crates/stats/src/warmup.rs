//! Choosing a warm-up: where the start-up of a series ends, by Welch's graphical procedure and
//! by the MSER-5 rule.
//!
//! **Welch's procedure** (P. D. Welch, 1981 and 1983, as given in A. M. Law, *Simulation Modeling
//! and Analysis*, 5th ed., McGraw-Hill, 2015, section 9.5.1, pp. 513–515) makes n replications of
//! length m, averages them at each observation, Ȳᵢ = (Y₁ᵢ + … + Yₙᵢ)/n ([`welch_average`]), which
//! keeps the transient mean curve E(Yᵢ) but divides its variance by n, and smooths that with a
//! moving average of window w ([`welch_moving_average`]):
//!
//! ```text
//! Ȳᵢ(w) = (Ȳᵢ₋w + … + Ȳᵢ₊w) / (2w + 1)              for i = w + 1, …, m − w,
//! Ȳᵢ(w) = (Ȳ₁ + … + Ȳ₂ᵢ₋₁) / (2i − 1)               for i = 1, …, w,
//! ```
//!
//! centred on observation i, with the window shrunk near the start so that it never reaches
//! before the first observation. The warm-up is read off the plot of Ȳ₁(w), …, Ȳₘ₋w(w), at the
//! point beyond which it appears to have converged; w is the smallest that makes the plot
//! reasonably smooth, at most ⌊m/4⌋ ([`welch_max_window`]). Law, Example 9.24 (p. 515): w = 2
//! and Ȳᵢ = 1, 2, 3, 4, 5, 6, 6, 6, 6, 6 give 1, 2, 3, 4, 4.8, 5.4, 5.8, 6.
//!
//! The plot is the main tool, and MSER-5 below a starting point for it: Law (Examples 9.28 and
//! 9.29, pp. 521–522) shows MSER-5 deleting 15 of 65,000 delays of an M/M/1 queue at ρ = 0.9
//! started empty, and 10 observations of a network model where Welch's plot calls for 6,200,
//! both leaving much of the start-up's bias in.
//!
//! **MSER.** White's marginal standard error rule (MSER; K. P. White Jr., "An effective truncation
//! heuristic for bias reduction in simulation output", *Simulation* 69(6):323–334, 1997, doi:10.1177/003754979706900601) deletes
//! the first d observations of an output series Y₁ … Yₙ, choosing the d that minimises
//!
//! ```text
//! MSER(d) = 1 / (n − d)² · Σ_{i = d+1}^{n} (Yᵢ − Ȳ(d))²,   Ȳ(d) the mean of Y_{d+1} … Yₙ,
//! ```
//!
//! the squared standard error of the truncated mean, ignoring autocorrelation: deleting more
//! removes the start-up's bias, but leaves fewer observations, and the rule balances the two.
//! MSER-5 applies it to the means of consecutive batches of 5 observations, which smooths the
//! series and makes the choice more stable (K. P. White Jr., M. J. Cobb and S. C. Spratt, "A
//! comparison of five steady-state truncation heuristics for simulation", *Proceedings of the
//! 2000 Winter Simulation Conference*, 755–760, doi:10.1109/WSC.2000.899843). As is usually
//! recommended, a truncation in the second half of the series is not considered: there the
//! statistic rests on too few batches, and a minimum there says the series is too short to show
//! its steady state. (The statistic is as secondary descriptions of these papers give it, Law,
//! pp. 520–521, among them; the papers themselves were not at hand.)
//!
//! In a simulation the series is usually the mean, across replications, of a quantity recorded
//! at fixed times (Welch's plot); the rule then says how many of those times to leave out.

/// Observations per batch in MSER-5.
pub const MSER_BATCH: usize = 5;

/// The fewest batches MSER-5 needs to say anything: with fewer, the first half leaves too few
/// candidates and too few batches behind them.
pub const MSER_MIN_BATCHES: usize = 4;

/// What MSER-5 found in a series.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Truncation {
    /// Delete the first `observations` observations (a multiple of [`MSER_BATCH`]); 0 means
    /// the series shows no start-up worth removing.
    At { observations: usize },
    /// The statistic was still falling at the middle of the series: the start-up may last
    /// longer than half of it, so the series is too short to say where it ends.
    TooShort,
}

/// MSER-5 on `series` (in time order): how many observations to delete from its start.
/// Observations after the last whole batch are left out, as the rule does. `None` with fewer
/// than [`MSER_MIN_BATCHES`] batches, or a value that is not finite.
pub fn mser5(series: &[f64]) -> Option<Truncation> {
    if series.iter().any(|y| !y.is_finite()) {
        return None;
    }
    let (whole, _) = series.as_chunks::<MSER_BATCH>();
    let batches: Vec<f64> = whole
        .iter()
        .map(|b| b.iter().sum::<f64>() / MSER_BATCH as f64)
        .collect();
    let m = batches.len();
    if m < MSER_MIN_BATCHES {
        return None;
    }
    // Candidates d = 0 … m/2: the first half of the batches.
    let last = m / 2;
    let mut best: Option<(usize, f64)> = None;
    for d in 0..=last {
        let kept = &batches[d..];
        let k = kept.len() as f64;
        let mean = kept.iter().sum::<f64>() / k;
        let ss: f64 = kept.iter().map(|z| (z - mean) * (z - mean)).sum();
        let stat = ss / (k * k);
        // Strictly smaller: the earliest of equal minima, deleting no more than needed.
        if best.is_none_or(|(_, s)| stat < s) {
            best = Some((d, stat));
        }
    }
    let (d, _) = best?;
    Some(if d == last && last > 0 {
        Truncation::TooShort
    } else {
        Truncation::At {
            observations: d * MSER_BATCH,
        }
    })
}

/// Welch's averaged process: the mean of the replications at each observation, Ȳᵢ = (Y₁ᵢ + … +
/// Yₙᵢ)/n. `None` with no replications, replications of different lengths, or a value that is
/// not finite.
pub fn welch_average(replications: &[&[f64]]) -> Option<Vec<f64>> {
    let first = replications.first()?;
    let m = first.len();
    if replications
        .iter()
        .any(|r| r.len() != m || r.iter().any(|y| !y.is_finite()))
    {
        return None;
    }
    let n = replications.len() as f64;
    Some(
        (0..m)
            .map(|i| replications.iter().map(|r| r[i]).sum::<f64>() / n)
            .collect(),
    )
}

/// The largest window Welch's moving average takes over `m` observations: ⌊m/4⌋.
pub fn welch_max_window(m: usize) -> usize {
    m / 4
}

/// Welch's moving average with window `w` of the averaged process `averaged` (Ȳ₁ … Ȳₘ, in time
/// order): Ȳᵢ(w) for i = 1 … m − w, each the plain mean of the 2w + 1 observations centred on
/// observation i, or of the 2i − 1 from the first when i ≤ w (see the module's description).
/// `None` when `w` is 0 or above [`welch_max_window`] (so with fewer than 4 observations), or a
/// value is not finite.
pub fn welch_moving_average(averaged: &[f64], w: usize) -> Option<Vec<f64>> {
    let m = averaged.len();
    if w == 0 || w > welch_max_window(m) || averaged.iter().any(|y| !y.is_finite()) {
        return None;
    }
    // Sums of the first k observations, so each window's sum is one difference; a window's mean
    // is its sum divided by its count, as the definition writes it.
    let mut prefix = Vec::with_capacity(m + 1);
    prefix.push(0.0);
    let mut s = 0.0;
    for y in averaged {
        s += y;
        prefix.push(s);
    }
    Some(
        (0..m - w)
            .map(|i| {
                // Observation i + 1 in the book's numbering; s runs over −h … h.
                let h = i.min(w);
                (prefix[i + h + 1] - prefix[i - h]) / (2 * h + 1) as f64
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welchs_moving_average_is_laws_example_9_24() {
        // Law, Simulation Modeling and Analysis, 5th ed., Example 9.24 (p. 515): m = 10, w = 2,
        // Ȳᵢ = i for i ≤ 5 and 6 after.
        let y = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 6.0, 6.0, 6.0, 6.0];
        assert_eq!(
            welch_moving_average(&y, 2),
            Some(vec![1.0, 2.0, 3.0, 4.0, 4.8, 5.4, 5.8, 6.0])
        );
    }

    #[test]
    fn welchs_window_is_centred_and_shrinks_at_the_start() {
        let y: Vec<f64> = (0..12).map(|i| f64::from(i * i)).collect();
        let w = 3;
        let got = welch_moving_average(&y, w).unwrap_or_default();
        assert_eq!(got.len(), y.len() - w);
        for (i, g) in got.iter().enumerate() {
            let h = i.min(w);
            let window = &y[i - h..=i + h];
            let want = window.iter().sum::<f64>() / window.len() as f64;
            assert_eq!(*g, want, "observation {}", i + 1);
        }
        // w = 1 is a three-point average after the first observation.
        assert_eq!(
            welch_moving_average(&[0.0, 3.0, 6.0, 3.0, 0.0], 1),
            Some(vec![0.0, 3.0, 4.0, 3.0])
        );
    }

    #[test]
    fn welchs_window_is_at_least_1_and_at_most_a_quarter_of_the_series() {
        let y = [1.0; 10];
        assert_eq!(welch_max_window(10), 2);
        assert_eq!(welch_moving_average(&y, 0), None);
        assert_eq!(welch_moving_average(&y, 3), None);
        assert_eq!(welch_moving_average(&y[..3], 1), None);
        assert_eq!(welch_moving_average(&[], 1), None);
        assert_eq!(welch_moving_average(&[1.0, f64::NAN, 1.0, 1.0], 1), None);
    }

    #[test]
    fn welchs_averaged_process_is_the_mean_at_each_observation() {
        let a = [1.0, 2.0, 3.0];
        let b = [3.0, 4.0, 8.0];
        assert_eq!(welch_average(&[&a, &b]), Some(vec![2.0, 3.0, 5.5]));
        assert_eq!(welch_average(&[]), None);
        assert_eq!(welch_average(&[&a, &b[..2]]), None);
        assert_eq!(welch_average(&[&a, &[1.0, f64::INFINITY, 0.0]]), None);
    }

    /// The exact mean delay in queue E(Dᵢ) of customers 1 … `count` of an M/M/1 queue started
    /// empty and idle: the number an arrival finds is a Markov chain (between arrivals, each
    /// service that ends does so before the next arrival with chance μ/(λ + μ)), and the delay of
    /// an arrival that finds k is k exponential services, k/μ on average.
    fn mm1_mean_delays(lambda: f64, mu: f64, count: usize) -> Vec<f64> {
        let (stay, go) = (lambda / (lambda + mu), mu / (lambda + mu));
        let mut p = vec![1.0];
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(p.iter().enumerate().map(|(k, q)| k as f64 * q).sum::<f64>() / mu);
            let mut next = vec![0.0; p.len() + 1];
            for (k, q) in p.iter().enumerate() {
                // k found, so k + 1 inside just after the arrival; j of them served before the
                // next arrival, or all of them.
                let mut served = 1.0;
                for j in 0..=k {
                    next[k + 1 - j] += q * served * stay;
                    served *= go;
                }
                next[0] += q * served;
            }
            p = next;
        }
        out
    }

    #[test]
    fn mser5_under_deletes_on_laws_example_9_28() {
        // Law, Example 9.28 (p. 521): delays in queue of an M/M/1 queue with λ = 1 and ω = 10/9
        // (ρ = 0.9) started empty, 5 replications of 65,000 delays averaged; MSER-5 deletes 15
        // of them, while E(Dᵢ) is still far below its steady state, ρ/(ω − λ) = 8.1.
        let (lambda, mu) = (1.0, 10.0 / 9.0);
        let exact = mm1_mean_delays(lambda, mu, 101);
        // The chain checked against the book: the mean of the first 25 expected delays is
        // d(25 | s = 0) = 2.12 (Table 9.2, p. 500).
        let d25 = exact[..25].iter().sum::<f64>() / 25.0;
        assert!((d25 - 2.12).abs() < 0.005, "{d25}");

        // The replications, by Lindley's recursion with exponential times from the uniforms of
        // the generator above.
        let (n, m) = (5, 65_000);
        let reps: Vec<Vec<f64>> = (0..n)
            .map(|r| {
                let u = noise(2 * m, 9_280 + r);
                let mut d = 0.0_f64;
                (0..m)
                    .map(|i| {
                        let delay = d;
                        // noise is in [−0.5, 0.5), so 0.5 − e is in (0, 1].
                        let arrival = -libm::log(0.5 - u[2 * i]) / lambda;
                        let service = -libm::log(0.5 - u[2 * i + 1]) / mu;
                        d = (d + service - arrival).max(0.0);
                        delay
                    })
                    .collect()
            })
            .collect();
        let views: Vec<&[f64]> = reps.iter().map(Vec::as_slice).collect();
        let averaged = welch_average(&views).unwrap_or_default();
        assert_eq!(averaged.len(), m);
        let t = mser5(&averaged);
        let cut = match t {
            Some(Truncation::At { observations }) => observations,
            _ => usize::MAX,
        };
        // The book's 15, give or take its random numbers: a few batches out of 13,000.
        assert!(cut <= 100, "{t:?}");
        // The first delay kept is still expected to be under a third of the steady state, and
        // the hundred-and-first under three quarters: the kept series carries the start-up's
        // bias, which a plot of it shows.
        let steady = (lambda / mu) / (mu - lambda);
        assert!((steady - 8.1).abs() < 1e-9);
        assert!(exact[cut] < 0.75 * steady, "E(D) {} at {cut}", exact[cut]);
        assert!(exact[15] < steady / 3.0, "{}", exact[15]);
        assert!(exact[100] < 0.75 * steady, "{}", exact[100]);
    }

    /// A small deterministic generator of noise in [−0.5, 0.5) (a linear congruential one), so
    /// the tests need no random crate.
    fn noise(n: usize, mut state: u64) -> Vec<f64> {
        (0..n)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 11) as f64 / (1u64 << 53) as f64 - 0.5
            })
            .collect()
    }

    #[test]
    fn a_flat_series_needs_no_truncation() {
        assert_eq!(mser5(&[3.0; 100]), Some(Truncation::At { observations: 0 }));
    }

    #[test]
    fn a_stationary_series_needs_little_or_none() {
        // Pure noise around 10: whatever the rule deletes is chance. Mostly nothing; now and
        // then a run of low or high values near the start is cut (which is why the run warning
        // also asks whether the cut moves the mean by more than chance), but here never more
        // than a quarter of the series.
        let cuts: Vec<usize> = (1..=20)
            .map(|seed| {
                let y: Vec<f64> = noise(200, seed).iter().map(|e| 10.0 + e).collect();
                match mser5(&y) {
                    Some(Truncation::At { observations }) => observations,
                    _ => usize::MAX,
                }
            })
            .collect();
        assert!(cuts.iter().all(|&c| c <= 50), "{cuts:?}");
        assert!(cuts.iter().filter(|&&c| c == 0).count() >= 15, "{cuts:?}");
    }

    #[test]
    fn a_start_up_is_cut_where_it_ends() {
        // 30 observations climbing from 0 to 10, then 170 at 10 with noise.
        let e = noise(200, 7);
        let y: Vec<f64> = (0..200)
            .map(|i| {
                let level = if i < 30 { i as f64 / 3.0 } else { 10.0 };
                level + 0.5 * e[i]
            })
            .collect();
        let t = mser5(&y);
        assert!(
            matches!(t, Some(Truncation::At { observations }) if (25..=40).contains(&observations)),
            "{t:?}"
        );
    }

    #[test]
    fn a_start_up_longer_than_half_the_series_is_too_short() {
        // Still rising at the end.
        let y: Vec<f64> = (0..100).map(|i| i as f64).collect();
        assert_eq!(mser5(&y), Some(Truncation::TooShort));
    }

    #[test]
    fn too_few_or_bad_observations_give_nothing() {
        assert_eq!(mser5(&[1.0; 19]), None);
        assert_eq!(mser5(&[]), None);
        let mut y = vec![1.0; 50];
        y[3] = f64::NAN;
        assert_eq!(mser5(&y), None);
    }

    #[test]
    fn the_statistic_is_whites() {
        // Batches 0, 4, 4, 4, 4, 4 (from observations of five equal values): deleting the first
        // batch leaves no spread, so d = 1, the first 5 observations.
        let mut y = vec![0.0; 5];
        y.extend([4.0; 25]);
        assert_eq!(mser5(&y), Some(Truncation::At { observations: 5 }));
        // Worked by hand: batches 1, 3, 2, 2 → d = 0: mean 2, SS 2, 2/16 = 0.125; d = 1: mean
        // 7/3, SS 2/3, (2/3)/9 ≈ 0.074; d = 2 (the middle, the last candidate): mean 2, SS 0,
        // 0 → the minimum is at the middle, so too short.
        let y: Vec<f64> = [1.0, 3.0, 2.0, 2.0].iter().flat_map(|b| [*b; 5]).collect();
        assert_eq!(mser5(&y), Some(Truncation::TooShort));
    }
}
