//! The difference between two systems' results, b − a, with a 95% confidence interval, or one
//! at another level ([`paired_at`], [`welch_at`]) (A. M. Law, *Simulation Modeling and
//! Analysis*, 5th ed., McGraw-Hill, 2015, section 10.2). Several differences that must hold
//! together take Bonferroni's level for each ([`crate::critical::bonferroni`]; Law, section
//! 10.3).
//!
//! - [`paired`]: the paired-t interval on the per-replication differences, for two runs whose
//!   replication i shares its random numbers (common random numbers: the same seed and
//!   replication count). n pairs give n − 1 degrees of freedom.
//! - [`welch`]: Welch's two-sample t interval, for independent runs, with the
//!   Welch–Satterthwaite degrees of freedom rounded down (the wider, safer interval).

use crate::critical::{DEFAULT_LEVEL, t_two_sided};
use crate::{Summary, Tally};

/// A difference b − a with its 95% confidence interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Difference {
    /// The values from a, and from b, that went into it (for a paired difference, both are the
    /// number of pairs).
    pub n_a: u64,
    pub n_b: u64,
    /// The estimated difference, b − a.
    pub mean: f64,
    /// Half-width of the 95% confidence interval; infinite when there are too few values to
    /// estimate it.
    pub half_width: f64,
    /// The degrees of freedom of the t value used; `None` with too few values.
    pub df: Option<u64>,
}

impl Difference {
    /// Whether the 95% interval excludes 0; `None` when it cannot be estimated.
    pub fn significant(&self) -> Option<bool> {
        self.half_width
            .is_finite()
            .then(|| self.mean.abs() > self.half_width)
    }
}

/// The paired-t difference b − a over the replications where both have a finite value, in
/// replication order. `None` when no replication has both.
pub fn paired(a: &[Option<f64>], b: &[Option<f64>]) -> Option<Difference> {
    paired_at(a, b, DEFAULT_LEVEL)
}

/// [`paired`] with the interval at confidence `level` (0.90 for 90%); at 0.95 the same.
pub fn paired_at(a: &[Option<f64>], b: &[Option<f64>], level: f64) -> Option<Difference> {
    let d: Vec<f64> = a
        .iter()
        .zip(b)
        .filter_map(|(x, y)| match (x, y) {
            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => Some(y - x),
            _ => None,
        })
        .collect();
    let s = Summary::at(&d, level)?;
    Some(Difference {
        n_a: s.n,
        n_b: s.n,
        mean: s.mean,
        half_width: s.half_width,
        df: (s.n > 1).then(|| s.n - 1),
    })
}

/// Welch's two-sample t difference mean(b) − mean(a), ignoring non-finite values. `None` when
/// either side has no value; an infinite half-width when either has only one.
pub fn welch(a: &[f64], b: &[f64]) -> Option<Difference> {
    welch_at(a, b, DEFAULT_LEVEL)
}

/// [`welch`] with the interval at confidence `level` (0.90 for 90%); at 0.95 the same.
pub fn welch_at(a: &[f64], b: &[f64], level: f64) -> Option<Difference> {
    let tally = |xs: &[f64]| {
        let mut t = Tally::new();
        xs.iter().for_each(|x| t.add(*x));
        t
    };
    let (ta, tb) = (tally(a), tally(b));
    let mean = tb.mean()? - ta.mean()?;
    let (na, nb) = (ta.count(), tb.count());
    let (n_a, n_b) = (na as f64, nb as f64);
    let mut d = Difference {
        n_a: na,
        n_b: nb,
        mean,
        half_width: f64::INFINITY,
        df: None,
    };
    let (Some(va), Some(vb)) = (ta.variance(), tb.variance()) else {
        return Some(d);
    };
    let (ua, ub) = (va / n_a, vb / n_b);
    let se2 = ua + ub;
    // Welch–Satterthwaite; with no spread on either side, the pooled count.
    let df = if se2 > 0.0 {
        let f = se2 * se2 / (ua * ua / (n_a - 1.0) + ub * ub / (n_b - 1.0));
        (f.floor() as u64).max(1)
    } else {
        na + nb - 2
    };
    if let Some(t) = t_two_sided(level, df) {
        d.half_width = t * se2.sqrt();
        d.df = Some(df);
    }
    Some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn paired_differences() {
        // Differences 1, 2, 3, 2: mean 2, sample sd sqrt(2/3), t(3) = 3.182.
        let a = [Some(10.0), Some(11.0), Some(9.0), Some(12.0)];
        let b = [Some(11.0), Some(13.0), Some(12.0), Some(14.0)];
        let d = paired(&a, &b);
        let want = 3.182 * (2.0_f64 / 3.0).sqrt() / 2.0;
        assert!(d.is_some_and(|d| d.n_a == 4
            && close(d.mean, 2.0)
            && close(d.half_width, want)
            && d.df == Some(3)
            && d.significant() == Some(true)));
    }

    #[test]
    fn paired_skips_replications_without_both_values() {
        let a = [Some(1.0), None, Some(3.0), Some(f64::NAN), Some(5.0)];
        let b = [Some(2.0), Some(9.0), None, Some(4.0), Some(7.0)];
        // Pairs: (1, 2) and (5, 7): differences 1 and 2.
        let d = paired(&a, &b);
        assert!(d.is_some_and(|d| d.n_a == 2 && close(d.mean, 1.5) && d.df == Some(1)));
        // One pair: a difference but no interval.
        let one = paired(&[Some(1.0), None], &[Some(3.0), Some(4.0)]);
        assert!(one.is_some_and(|d| d.n_a == 1
            && close(d.mean, 2.0)
            && d.half_width.is_infinite()
            && d.df.is_none()
            && d.significant().is_none()));
        assert_eq!(paired(&[None, Some(1.0)], &[Some(1.0), None]), None);
        assert_eq!(paired(&[], &[]), None);
    }

    #[test]
    fn identical_runs_differ_by_exactly_nothing() {
        let a = [Some(1.5), Some(2.5), Some(0.25)];
        let d = paired(&a, &a);
        assert!(d.is_some_and(|d| d.mean == 0.0
            && d.half_width == 0.0
            && d.significant() == Some(false)));
        // A constant shift with no spread is significant.
        let b = [Some(2.5), Some(3.5), Some(1.25)];
        assert!(paired(&a, &b).is_some_and(|d| d.significant() == Some(true)));
    }

    #[test]
    fn welch_interval() {
        // a: mean 5, variance 32/7 (n 8); b: mean 9, variance 2/3 (n 4).
        let a = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let b = [8.0, 9.0, 10.0, 9.0];
        let (ua, ub): (f64, f64) = (32.0 / 7.0 / 8.0, 2.0 / 3.0 / 4.0);
        let f = (ua + ub) * (ua + ub) / (ua * ua / 7.0 + ub * ub / 3.0);
        assert!((f - 9.75).abs() < 0.01, "{f}");
        let d = welch(&a, &b);
        // df 9.75 is rounded down to 9: t = 2.262.
        assert!(d.is_some_and(|d| d.n_a == 8
            && d.n_b == 4
            && close(d.mean, 4.0)
            && d.df == Some(9)
            && close(d.half_width, 2.262 * (ua + ub).sqrt())
            && d.significant() == Some(true)));
    }

    #[test]
    fn welch_edge_cases() {
        assert_eq!(welch(&[], &[1.0]), None);
        let one = welch(&[1.0], &[2.0, 3.0]);
        assert!(one.is_some_and(|d| close(d.mean, 1.5)
            && d.half_width.is_infinite()
            && d.significant().is_none()));
        // No spread on either side: the difference is exact.
        let flat = welch(&[2.0, 2.0], &[3.0, 3.0, 3.0]);
        assert!(flat.is_some_and(|d| d.half_width == 0.0
            && d.df == Some(3)
            && d.significant() == Some(true)));
        assert!(welch(&[2.0, 2.0], &[2.0, 2.0]).is_some_and(|d| d.significant() == Some(false)));
    }

    /// Law, 5th ed., Table 10.5 (p. 567): five inventory policies, five independent
    /// replications each (average monthly cost).
    const POLICIES: [[f64; 5]; 5] = [
        [126.97, 124.31, 126.68, 122.66, 127.23],
        [118.21, 120.22, 122.45, 122.68, 119.40],
        [120.77, 129.32, 120.61, 123.65, 127.34],
        [131.64, 137.07, 129.91, 129.97, 131.08],
        [141.09, 143.86, 144.30, 141.72, 142.61],
    ];

    /// Table 10.6 (p. 567): each of the four policies against the first, at 97.5% each so that
    /// all four hold together with at least 90% (Bonferroni). Every paired-t and Welch interval
    /// reproduces to the two decimals printed. (The book prints the last Welch half-length as
    /// 6.15, which is that interval's whole width; its half-length is 3.08.)
    #[test]
    fn comparisons_with_a_standard_at_bonferroni_s_level() {
        let level = crate::critical::bonferroni(0.90, 4).unwrap_or(f64::NAN);
        let paired_want = [(-10.44, 0.48), (-8.80, 6.34), (0.27, 12.46), (13.48, 20.81)];
        let welch_want = [(-8.52, -1.44), (-7.44, 4.97), (1.82, 10.91), (14.07, 20.22)];
        let r2 = |x: f64| (x * 100.0).round() / 100.0;
        let ends = |d: Option<Difference>| {
            d.map_or((f64::NAN, f64::NAN), |d| {
                (r2(d.mean - d.half_width), r2(d.mean + d.half_width))
            })
        };
        let some = |xs: &[f64; 5]| xs.map(Some);
        for i in 1..5 {
            let p = paired_at(&some(&POLICIES[0]), &some(&POLICIES[i]), level);
            assert_eq!(ends(p), paired_want[i - 1], "paired {i}");
            let w = welch_at(&POLICIES[0], &POLICIES[i], level);
            assert_eq!(ends(w), welch_want[i - 1], "Welch {i}");
            assert_eq!(w.and_then(|w| w.df), Some([7, 5, 7, 7][i - 1]));
        }
    }

    /// Table 10.7 (p. 568): all ten pairs at 99% each, for 90% together; the paired-t
    /// half-lengths computed from Table 10.5. Within 0.02 of every printed one: 2–3, 2–4 and 2–5
    /// reproduce (9.58, 8.38, 3.80); the book prints 7.18, 9.99, 8.01, 4.83, 5.66, 7.73 and 5.85
    /// for the others.
    #[test]
    fn all_pairwise_comparisons_at_bonferroni_s_level() {
        let level = crate::critical::bonferroni(0.90, 10).unwrap_or(f64::NAN);
        let want = [
            ((0, 1), 7.19),
            ((0, 2), 9.97),
            ((0, 3), 8.03),
            ((0, 4), 4.82),
            ((1, 2), 9.58),
            ((1, 3), 8.38),
            ((1, 4), 3.80),
            ((2, 3), 5.65),
            ((2, 4), 7.74),
            ((3, 4), 5.84),
        ];
        let some = |xs: &[f64; 5]| xs.map(Some);
        for ((a, b), hw) in want {
            let d = paired_at(&some(&POLICIES[a]), &some(&POLICIES[b]), level);
            let got = d.map_or(f64::NAN, |d| (d.half_width * 100.0).round() / 100.0);
            assert_eq!(got, hw, "{a}-{b}");
        }
    }

    #[test]
    fn the_default_level_is_the_difference_of_before() {
        let a = [Some(10.0), Some(11.0), Some(9.0), Some(12.0)];
        let b = [Some(11.0), Some(13.0), Some(12.0), Some(14.0)];
        assert_eq!(paired_at(&a, &b, 0.95), paired(&a, &b));
        let (x, y) = (
            [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0],
            [8.0, 9.0, 10.0, 9.0],
        );
        assert_eq!(welch_at(&x, &y, 0.95), welch(&x, &y));
    }
}
