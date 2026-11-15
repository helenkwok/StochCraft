//! Choosing a warm-up: where the start-up of a series ends, by the MSER-5 rule.
//!
//! White's marginal standard error rule (MSER; K. P. White Jr., "An effective truncation
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
//! its steady state. (The statistic is as secondary descriptions of these papers give it; the
//! papers themselves were not at hand.)
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

#[cfg(test)]
mod tests {
    use super::*;

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
