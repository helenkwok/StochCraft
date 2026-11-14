//! Whether the replications split into two outcomes: a result that is near one value in some
//! runs and near another, far away, in the rest (a server that collapses in some runs and
//! recovers in others), so that the mean across them describes no actual run.
//!
//! - [`best_split`]: the best division of the values into a low and a high group, the exact
//!   one-dimensional 2-means (Fisher's natural breaks for two classes): every cut between two
//!   neighbouring distinct values of the sorted sample is tried, and the one with the least
//!   within-group sum of squares is kept.
//! - [`two_outcomes`]: that split, when it is clear enough to report (see its rule).
//!
//! The separation of the two groups is measured with Ashman's D (K. M. Ashman, C. M. Bird and
//! S. E. Zepeda, "Detecting bimodality in astronomical datasets", *Astronomical Journal* 108
//! (1994) 2348–2361): D = √2 |m₁ − m₂| / √(s₁² + s₂²), where D > 2 is needed for a clean
//! separation of two normal components. Sarle's bimodality coefficient is not used: it reacts
//! to skewness as much as to two peaks, so a skewed single-peaked result can pass it (R.
//! Pfister, K. A. Schwarz, M. Janczyk, R. Dale and J. B. Freeman, "Good things peak in pairs: a
//! note on the bimodality coefficient", *Frontiers in Psychology* 4 (2013) 700).
//!
//! Ashman's D > 2 alone is not enough here. It is meant for two components fitted to the data,
//! but the groups here come from a hard cut, which trims each group's tails and so shrinks its
//! spread. Cutting one normal sample at its mean already gives D ≈ 2.65 (halves with means
//! ±0.80σ and standard deviations 0.60σ), an exponential sample D ≈ 2.6 and a uniform one
//! D ≈ 3.46. In simulation (2,000 samples each of 10, 30 and 128 values) D > 2 flagged 93% of
//! normal samples of 10 and all of 128. The rule therefore also asks that the cut explain
//! nearly all of the spread: the within-group sum of squares must be a small share of the
//! total (the statistic of L. Engelman and J. A. Hartigan's test for clusters, "Percentage
//! points of a test for clusters", *Journal of the American Statistical Association* 64 (1969)
//! 1647–1648). For two equal groups of equal spread, a share below 0.10 is the same as D > 6.

use crate::Tally;

/// One side of a [`Split`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Group {
    /// How many values are in it.
    pub n: u64,
    /// How many different values are in it.
    pub distinct: u64,
    pub mean: f64,
    /// The sample standard deviation (n − 1 denominator); 0 for a single value.
    pub std_dev: f64,
}

/// The best division of a sample into a low and a high group.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Split {
    /// The values below [`Split::cut`].
    pub low: Group,
    /// The values above it.
    pub high: Group,
    /// Halfway between the highest low value and the lowest high value.
    pub cut: f64,
    /// Ashman's D for the two groups; infinite when neither group has any spread.
    pub d: f64,
    /// The within-group sum of squares as a share of the total sum of squares about the mean
    /// (0 when each group is a single repeated value).
    pub within_share: f64,
}

/// The fewest values [`two_outcomes`] looks at.
pub const MIN_VALUES: u64 = 10;

/// The best 2-group split of `values` (non-finite values are ignored): the cut between two
/// neighbouring distinct sorted values with the least within-group sum of squares, the first
/// such cut on a tie. `None` with fewer than two distinct values.
pub fn best_split(values: &[f64]) -> Option<Split> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    // The sum of squares of v[..k] for every k, and of v[k..], by Welford's update in each
    // direction (stable whatever the values' offset).
    let running = |it: &mut dyn Iterator<Item = &f64>| -> Vec<f64> {
        let mut t = Tally::new();
        let mut out = vec![0.0];
        for x in it {
            t.add(*x);
            out.push(t.variance().map_or(0.0, |s| s * (t.count() - 1) as f64));
        }
        out
    };
    let below = running(&mut v.iter());
    let mut above = running(&mut v.iter().rev());
    above.reverse();
    let mut best: Option<(usize, f64)> = None;
    for k in 1..n {
        if v[k - 1] == v[k] {
            continue;
        }
        let w = below[k] + above[k];
        if best.is_none_or(|(_, b)| w < b) {
            best = Some((k, w));
        }
    }
    let (k, within) = best?;
    let group = |g: &[f64]| {
        let mut t = Tally::new();
        g.iter().for_each(|x| t.add(*x));
        Group {
            n: t.count(),
            // `g` is sorted, so each new value starts where it differs from the one before.
            distinct: 1 + g.windows(2).filter(|w| w[0] != w[1]).count() as u64,
            mean: t.mean().unwrap_or(0.0),
            std_dev: t.std_dev().unwrap_or(0.0),
        }
    };
    let (low, high) = (group(&v[..k]), group(&v[k..]));
    let total = below[n];
    let spread = (low.std_dev * low.std_dev + high.std_dev * high.std_dev).sqrt();
    let gap = (high.mean - low.mean).abs();
    Some(Split {
        low,
        high,
        cut: (v[k - 1] + v[k]) / 2.0,
        d: if spread > 0.0 {
            std::f64::consts::SQRT_2 * gap / spread
        } else {
            f64::INFINITY
        },
        within_share: if total > 0.0 { within / total } else { 0.0 },
    })
}

/// The split of `values` (non-finite ones ignored) into two outcomes, when it is clear enough
/// to report:
///
/// - at least [`MIN_VALUES`] values;
/// - each group holds at least max(2, ⌈n/10⌉) of them, and at least 3 different values;
/// - Ashman's D > 2;
/// - the within-group sum of squares is less than 5% of the total with fewer than 30 values,
///   less than 10% with 30 or more.
///
/// The 3 different values keep out results that take only a few values, such as a count of
/// entities left inside at the end that is 0 in most runs and 1 or 2 in the rest: such a count
/// splits trivially at a step, with no spread inside its groups, but the runs did not have two
/// outcomes.
///
/// The two share limits were chosen by simulating this rule (2,000 samples each of 10, 15, 20,
/// 29, 30, 50 and 128 values, 500 of 512): single-peaked samples (normal, gamma(2),
/// exponential, uniform, lognormal with σ = 1, Poisson(0.3) and Poisson(3), 0 or 1 with
/// probability 0.3) are reported in at most 0.6% of samples of any size, the very skewed
/// lognormal with σ = 1.5 in at most 1.9% (at 30 values, where the limit rises). Two clearly
/// separate outcomes (77% of runs near 1.7 with σ = 0.4, the rest near 9.9 with σ = 1.5) are
/// reported in 29% of samples of 10, 60% of 20, 97% of 30 and all of 128; two humps 5σ apart
/// are rarely reported (at most 17%), so only clearly separate outcomes are.
pub fn two_outcomes(values: &[f64]) -> Option<Split> {
    let n = values.iter().filter(|x| x.is_finite()).count() as u64;
    if n < MIN_VALUES {
        return None;
    }
    let s = best_split(values)?;
    let fewest = n.div_ceil(10).max(2);
    let limit = if n < 30 { 0.05 } else { 0.10 };
    let big_enough = |g: &Group| g.n >= fewest && g.distinct >= 3;
    (big_enough(&s.low) && big_enough(&s.high) && s.d > 2.0 && s.within_share < limit).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spread::normal_quantile;

    /// A uniform stream in (0, 1) from SplitMix64, so the tests need no other crate.
    struct Stream(u64);

    impl Stream {
        fn uniform(&mut self) -> f64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            ((z >> 11) as f64 + 0.5) / (1u64 << 53) as f64
        }
        fn normal(&mut self, mean: f64, sd: f64) -> f64 {
            mean + sd * normal_quantile(self.uniform()).unwrap_or(0.0)
        }
        fn exponential(&mut self, mean: f64) -> f64 {
            -mean * libm::log(self.uniform())
        }
    }

    /// How many of 20 seeds' samples of `n` values from `draw` are reported as split.
    fn flagged(n: usize, draw: impl Fn(&mut Stream) -> f64) -> usize {
        (1..=20u64)
            .filter(|seed| {
                let mut s = Stream(seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
                let v: Vec<f64> = (0..n).map(|_| draw(&mut s)).collect();
                two_outcomes(&v).is_some()
            })
            .count()
    }

    #[test]
    fn a_clear_mixture_is_reported_with_its_groups() {
        // 99 runs near 1.7 and 29 near 9.9, shuffled together: the design's example.
        let mut s = Stream(7);
        let mut v: Vec<f64> = (0..99).map(|_| s.normal(1.7, 0.3)).collect();
        v.extend((0..29).map(|_| s.normal(9.9, 1.0)));
        v.rotate_left(40);
        let split = two_outcomes(&v);
        assert!(split.is_some(), "{:?}", best_split(&v));
        let split = split.unwrap_or_else(|| unreachable!());
        assert_eq!((split.low.n, split.high.n), (99, 29));
        assert!((split.low.mean - 1.7).abs() < 0.1, "{split:?}");
        assert!((split.high.mean - 9.9).abs() < 0.5, "{split:?}");
        assert!(split.cut > 3.0 && split.cut < 7.0, "{split:?}");
        assert!(split.d > 6.0 && split.within_share < 0.05, "{split:?}");
    }

    #[test]
    fn a_normal_sample_is_not_reported() {
        for n in [10, 30, 128] {
            assert_eq!(flagged(n, |s| s.normal(5.0, 1.0)), 0, "n = {n}");
        }
    }

    #[test]
    fn a_skewed_sample_is_not_reported() {
        for n in [10, 30, 128] {
            assert_eq!(flagged(n, |s| s.exponential(2.0)), 0, "n = {n}");
        }
    }

    #[test]
    fn a_hard_cut_of_one_normal_already_passes_ashmans_d() {
        // Why D > 2 alone is not the rule: the best split of a normal sample of 128 has D near
        // 2.65 but leaves about a third of the spread within the groups.
        let mut s = Stream(3);
        let v: Vec<f64> = (0..128).map(|_| s.normal(0.0, 1.0)).collect();
        let split = best_split(&v);
        assert!(
            split.is_some_and(|s| s.d > 2.0 && s.within_share > 0.25),
            "{split:?}"
        );
        assert_eq!(two_outcomes(&v), None);
    }

    #[test]
    fn groups_smaller_than_a_tenth_are_not_reported() {
        // 2 runs far off among 28 (fewer than ⌈30/10⌉ = 3), and 1 among 10.
        let mut v: Vec<f64> = (0..28).map(|i| 1.0 + 0.01 * f64::from(i)).collect();
        v.extend([50.0, 50.5]);
        assert!(best_split(&v).is_some_and(|s| s.high.n == 2));
        assert_eq!(two_outcomes(&v), None);
        let mut v: Vec<f64> = (0..9).map(|i| 1.0 + 0.01 * f64::from(i)).collect();
        v.push(50.0);
        assert_eq!(two_outcomes(&v), None);
        // 3 among 30 is enough.
        let mut v: Vec<f64> = (0..27).map(|i| 1.0 + 0.01 * f64::from(i)).collect();
        v.extend([50.0, 50.5, 51.0]);
        assert!(two_outcomes(&v).is_some_and(|s| s.high.n == 3));
    }

    #[test]
    fn equal_values_and_too_few_values_are_not_reported() {
        assert_eq!(best_split(&[4.0; 50]), None);
        assert_eq!(two_outcomes(&[4.0; 50]), None);
        assert_eq!(best_split(&[]), None);
        // A clear split, but only 9 values.
        let v = [1.0, 1.0, 1.0, 1.0, 1.0, 9.0, 9.0, 9.0, 9.0];
        assert!(best_split(&v).is_some());
        assert_eq!(two_outcomes(&v), None);
    }

    #[test]
    fn groups_without_spread_have_infinite_d() {
        // A count that is 0 in 7 runs and 1 in 5: the best split has no spread inside its
        // groups, so D is infinite.
        let mut v = vec![0.0; 7];
        v.extend([1.0; 5]);
        let split = best_split(&v);
        assert!(
            split.is_some_and(|s| s.d.is_infinite()
                && s.within_share == 0.0
                && (s.low.n, s.high.n) == (7, 5)
                && (s.low.distinct, s.high.distinct) == (1, 1)
                && s.cut == 0.5),
            "{split:?}"
        );
        // One group without spread and one with: D is finite.
        let mut v = vec![0.0; 7];
        v.extend([10.0, 10.2, 10.4, 10.6, 10.8]);
        assert!(best_split(&v).is_some_and(|s| s.d.is_finite() && s.d > 2.0));
    }

    #[test]
    fn results_of_only_a_few_values_are_not_reported() {
        // Entities left inside at the end: 0 in most runs, 1 or 2 in the rest. It splits at
        // the step from 0, but the runs did not have two outcomes.
        // Without the 3 different values, every other condition would report it.
        let mut v = vec![0.0; 90];
        v.extend([1.0; 36]);
        v.extend([2.0; 2]);
        assert!(
            best_split(&v).is_some_and(|s| s.low.distinct == 1
                && s.high.distinct == 2
                && s.d > 2.0
                && s.within_share < 0.10),
            "{:?}",
            best_split(&v)
        );
        assert_eq!(two_outcomes(&v), None);
        let mut v = vec![0.0; 7];
        v.extend([1.0; 5]);
        assert_eq!(two_outcomes(&v), None);
        // Counts with enough different values in each group can split.
        let mut v: Vec<f64> = (0..20).map(|i| f64::from(i % 3)).collect();
        v.extend((0..10).map(|i| 40.0 + f64::from(i % 4)));
        assert!(two_outcomes(&v).is_some_and(|s| (s.low.n, s.high.n) == (20, 10)));
    }

    #[test]
    fn the_split_ignores_non_finite_values_and_order() {
        let mut v: Vec<f64> = (0..12)
            .map(|i| f64::from(i % 2) * 10.0 + 0.1 * f64::from(i))
            .collect();
        let a = two_outcomes(&v);
        v.push(f64::NAN);
        v.reverse();
        assert_eq!(two_outcomes(&v), a);
        assert!(a.is_some());
    }

    #[test]
    fn the_cut_is_the_least_within_group_sum_of_squares() {
        // Brute force over every cut for a small awkward sample.
        let v = [0.0, 0.5, 1.0, 4.0, 4.2, 4.4, 9.0, 9.5, 10.0, 10.5];
        let ss = |g: &[f64]| {
            let m = g.iter().sum::<f64>() / g.len() as f64;
            g.iter().map(|x| (x - m) * (x - m)).sum::<f64>()
        };
        let best_k = (1..v.len())
            .min_by(|a, b| {
                let wa = ss(&v[..*a]) + ss(&v[*a..]);
                let wb = ss(&v[..*b]) + ss(&v[*b..]);
                wa.total_cmp(&wb)
            })
            .unwrap_or(0);
        let split = best_split(&v);
        assert!(
            split.is_some_and(|s| s.low.n == best_k as u64),
            "{split:?} {best_k}"
        );
    }
}
