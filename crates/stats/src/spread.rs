//! The distribution of a result across replications, not only its mean: percentiles, the
//! empirical cumulative distribution with a confidence band, and the probability of meeting a
//! target with its interval.
//!
//! - [`quantile`]: Hyndman and Fan's (1996) definition 7, linear interpolation between order
//!   statistics (the default of most statistics packages).
//! - [`Ecdf`]: the empirical cumulative distribution and its Dvoretzky–Kiefer–Wolfowitz band,
//!   with Massart's (1990) tight constant: with probability at least the confidence asked for,
//!   the true distribution lies within ε = √(ln(2/α) / 2n) of the empirical one everywhere.
//! - [`order_statistic_interval`] and [`Ecdf::quantile_interval`]: a confidence interval for a
//!   quantile between two order statistics, [X(r), X(s)], chosen from the binomial distribution
//!   so that it covers the quantile with at least the confidence asked for whatever the
//!   distribution (W. J. Conover, *Practical Nonparametric Statistics*, 3rd ed., Wiley, 1999,
//!   section 3.2; A. M. Law, *Simulation Modeling and Analysis*, 5th ed., section 9.4.2, which
//!   gives the normal approximation to the same rule).
//! - [`Proportion`]: a probability estimated from successes in independent trials, with
//!   Wilson's (1927) score interval, which keeps close to its nominal coverage for small
//!   samples and probabilities near 0 or 1, where the textbook normal interval does not.

/// The natural logarithm, identical on every target.
fn ln(x: f64) -> f64 {
    libm::log(x)
}

/// The value below which a fraction `p` of `sorted` (ascending, finite) lies: Hyndman and
/// Fan's definition 7, x[h] interpolated linearly with h = (n − 1)p. `None` for no values or a
/// `p` outside [0, 1].
pub fn quantile(sorted: &[f64], p: f64) -> Option<f64> {
    if sorted.is_empty() || !(0.0..=1.0).contains(&p) {
        return None;
    }
    let h = (sorted.len() - 1) as f64 * p;
    let lo = h.floor();
    let i = lo as usize;
    let x0 = *sorted.get(i)?;
    let x1 = sorted.get(i + 1).copied().unwrap_or(x0);
    Some(x0 + (h - lo) * (x1 - x0))
}

/// The standard normal cumulative distribution Φ(x).
pub fn normal_cdf(x: f64) -> f64 {
    0.5 * libm::erfc(-x / std::f64::consts::SQRT_2)
}

/// The standard normal quantile Φ⁻¹(p) for 0 < p < 1, to within a few units in the last place:
/// Newton's method on [`normal_cdf`], kept inside a bracketing interval by bisection.
pub fn normal_quantile(p: f64) -> Option<f64> {
    if !(p > 0.0 && p < 1.0) {
        return None;
    }
    let (mut lo, mut hi) = (-40.0_f64, 40.0_f64);
    let mut x = 0.0_f64;
    for _ in 0..200 {
        let f = normal_cdf(x) - p;
        if f == 0.0 {
            return Some(x);
        }
        if f < 0.0 {
            lo = x;
        } else {
            hi = x;
        }
        let density = libm::exp(-0.5 * x * x) / (2.0 * std::f64::consts::PI).sqrt();
        let newton = x - f / density;
        let next = if density > 0.0 && newton > lo && newton < hi {
            newton
        } else {
            0.5 * (lo + hi)
        };
        if (next - x).abs() <= 1e-15 * x.abs().max(1.0) {
            return Some(next);
        }
        x = next;
    }
    Some(x)
}

/// z such that a standard normal lies within ±z with probability `confidence` (1.96 for 0.95).
pub fn z_two_sided(confidence: f64) -> Option<f64> {
    if !(confidence > 0.0 && confidence < 1.0) {
        return None;
    }
    normal_quantile(1.0 - (1.0 - confidence) / 2.0)
}

/// The empirical cumulative distribution of results across replications.
///
/// A value may be infinite: a time to an event that did not happen before the run's time
/// limit (right-censored there). It counts in `n` but is never at or below a finite x, which
/// is exact below the limit, the only place the distribution is known.
#[derive(Debug, Clone, PartialEq)]
pub struct Ecdf {
    /// Ascending; finite ones first, then the infinite (censored) ones.
    sorted: Vec<f64>,
}

impl Ecdf {
    /// The distribution of `values`; NaNs are left out, +∞ counts as censored.
    pub fn new(values: &[f64]) -> Self {
        let mut sorted: Vec<f64> = values
            .iter()
            .copied()
            .filter(|v| !v.is_nan() && *v != f64::NEG_INFINITY)
            .collect();
        sorted.sort_by(f64::total_cmp);
        Self { sorted }
    }

    /// Values, censored ones included.
    pub fn n(&self) -> usize {
        self.sorted.len()
    }

    /// The finite values, ascending.
    pub fn finite(&self) -> &[f64] {
        let k = self.sorted.partition_point(|v| v.is_finite());
        self.sorted.get(..k).unwrap_or(&[])
    }

    /// The fraction of values at or below `x`.
    pub fn at(&self, x: f64) -> Option<f64> {
        if self.sorted.is_empty() {
            return None;
        }
        let k = self.sorted.partition_point(|v| *v <= x);
        Some(k as f64 / self.sorted.len() as f64)
    }

    /// The value below which a fraction `p` lies (as [`quantile`]), or `None` when it lies
    /// among the censored values (beyond the time limit) or there are none.
    pub fn quantile(&self, p: f64) -> Option<f64> {
        let q = quantile(&self.sorted, p)?;
        q.is_finite().then_some(q)
    }

    /// A `confidence` interval for the value below which a fraction `p` lies, between two order
    /// statistics ([`order_statistic_interval`]): its ends, the upper infinite when it falls
    /// among the censored values. `None` when there are too few values.
    pub fn quantile_interval(&self, p: f64, confidence: f64) -> Option<(f64, f64)> {
        let (r, s, _) = order_statistic_interval(self.sorted.len() as u64, p, confidence)?;
        let at = |k: u64| self.sorted.get(usize::try_from(k - 1).ok()?).copied();
        Some((at(r)?, at(s)?))
    }

    /// The half-width ε of the Dvoretzky–Kiefer–Wolfowitz band at `confidence`: the true
    /// distribution lies within ±ε of this one at every x, with probability at least
    /// `confidence` (Massart, 1990).
    pub fn dkw_half_width(&self, confidence: f64) -> Option<f64> {
        dkw_half_width(self.sorted.len(), confidence)
    }

    /// The steps of the distribution: each distinct finite value and the fraction of values at
    /// or below it, ascending.
    pub fn steps(&self) -> Vec<(f64, f64)> {
        let n = self.sorted.len() as f64;
        let mut out: Vec<(f64, f64)> = Vec::new();
        for (i, v) in self.finite().iter().enumerate() {
            let f = (i + 1) as f64 / n;
            match out.last_mut() {
                Some(last) if last.0 == *v => last.1 = f,
                _ => out.push((*v, f)),
            }
        }
        out
    }
}

/// The order statistics, 1-based, whose interval [X(r), X(s)] covers the `p` quantile with
/// probability at least `confidence`, and that probability: with B the number of n values
/// below the quantile (binomial with n and p), r is the largest with P(B < r) ≤ α/2 and s the
/// smallest with P(B ≥ s) ≤ α/2, α = 1 − `confidence`, so P(X(r) ≤ x_p ≤ X(s)) ≥ P(r ≤ B < s)
/// ≥ 1 − α. `None` when there are too few values for either end (r below 1 or s above n), a `p`
/// not strictly between 0 and 1, or a `confidence` not strictly between 0 and 1.
pub fn order_statistic_interval(n: u64, p: f64, confidence: f64) -> Option<(u64, u64, f64)> {
    if n == 0 || !(p > 0.0 && p < 1.0) || !(confidence > 0.0 && confidence < 1.0) {
        return None;
    }
    let tail = 0.5 * (1.0 - confidence);
    let nf = n as f64;
    let (lp, lq) = (ln(p), libm::log1p(-p));
    // P(B = k), from logarithms so that large n neither overflows nor underflows to nothing.
    let pmf = |k: u64| {
        let k = k as f64;
        libm::exp(
            libm::lgamma(nf + 1.0) - libm::lgamma(k + 1.0) - libm::lgamma(nf - k + 1.0)
                + k * lp
                + (nf - k) * lq,
        )
    };
    let mut cdf = Vec::with_capacity(usize::try_from(n).ok()? + 1);
    let mut sum = 0.0;
    for k in 0..=n {
        sum += pmf(k);
        // cdf[k] = P(B ≤ k).
        cdf.push(sum);
    }
    let below = |k: u64| -> f64 {
        // P(B < k) = P(B ≤ k − 1).
        k.checked_sub(1)
            .and_then(|j| cdf.get(usize::try_from(j).ok()?).copied())
            .unwrap_or(0.0)
    };
    // The largest r in 1..=n with P(B < r) ≤ tail.
    let r = (1..=n).rev().find(|&r| below(r) <= tail)?;
    // The smallest s in 1..=n with P(B ≥ s) = 1 − P(B < s) ≤ tail.
    let s = (1..=n).find(|&s| 1.0 - below(s) <= tail)?;
    (r < s).then(|| (r, s, below(s) - below(r)))
}

/// ε = √(ln(2/α) / 2n) with α = 1 − `confidence`, for `n` values.
pub fn dkw_half_width(n: usize, confidence: f64) -> Option<f64> {
    if n == 0 || !(confidence > 0.0 && confidence < 1.0) {
        return None;
    }
    Some((ln(2.0 / (1.0 - confidence)) / (2.0 * n as f64)).sqrt())
}

/// A probability estimated from `successes` out of `n` independent trials (replications that
/// met a target, say).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Proportion {
    pub successes: u64,
    pub n: u64,
}

impl Proportion {
    pub fn new(successes: u64, n: u64) -> Self {
        Self {
            successes: successes.min(n),
            n,
        }
    }

    /// The fraction of successes, or `None` with no trials.
    pub fn estimate(&self) -> Option<f64> {
        (self.n > 0).then(|| self.successes as f64 / self.n as f64)
    }

    /// Wilson's score interval at `confidence` (0.95 for 95%), within [0, 1].
    pub fn wilson(&self, confidence: f64) -> Option<(f64, f64)> {
        let p = self.estimate()?;
        let z = z_two_sided(confidence)?;
        let n = self.n as f64;
        let z2 = z * z;
        let centre = (p + z2 / (2.0 * n)) / (1.0 + z2 / n);
        let half = z / (1.0 + z2 / n) * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
        // At 0 or n successes the matching end is exactly 0 or 1 (rounding aside).
        let lo = if self.successes == 0 {
            0.0
        } else {
            (centre - half).max(0.0)
        };
        let hi = if self.successes == self.n {
            1.0
        } else {
            (centre + half).min(1.0)
        };
        Some((lo, hi))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn quantile_definition_7() {
        let xs = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(quantile(&xs, 0.0), Some(1.0));
        assert_eq!(quantile(&xs, 1.0), Some(4.0));
        // h = 3 × 0.5 = 1.5: halfway between 2 and 3.
        assert_eq!(quantile(&xs, 0.5), Some(2.5));
        // h = 3 × 0.9 = 2.7.
        assert!(quantile(&xs, 0.9).is_some_and(|q| close(q, 3.7, 1e-12)));
        assert_eq!(quantile(&[], 0.5), None);
        assert_eq!(quantile(&xs, 1.5), None);
        assert_eq!(quantile(&[7.0], 0.3), Some(7.0));
    }

    #[test]
    fn order_statistic_intervals() {
        // Law, 5th ed., Example 9.22 (p. 510): 100 replications, the 0.95 quantile at 90%:
        // [X(91), X(99)]; it covers with probability 0.9347 here.
        let got = order_statistic_interval(100, 0.95, 0.90);
        assert!(got.is_some_and(|(r, s, c)| r == 91 && s == 99 && (c - 0.934_730_5).abs() < 1e-6));
        // The median of 100 at 95%: [X(40), X(61)], coverage 0.9648.
        let got = order_statistic_interval(100, 0.5, 0.95);
        assert!(got.is_some_and(|(r, s, c)| r == 40 && s == 61 && (c - 0.964_8).abs() < 1e-4));
        // Coverage is never below the level asked for.
        for n in [10u64, 20, 37, 100, 1000] {
            for p in [0.1, 0.5, 0.9] {
                if let Some((r, s, c)) = order_statistic_interval(n, p, 0.95) {
                    assert!(c >= 0.95 - 1e-12 && r >= 1 && s <= n && r < s, "{n} {p}");
                }
            }
        }
        // Too few values for the 0.95 quantile at 95%: no interval.
        assert_eq!(order_statistic_interval(10, 0.95, 0.95), None);
        let e = Ecdf::new(&(1..=100).map(f64::from).collect::<Vec<_>>());
        assert_eq!(e.quantile_interval(0.95, 0.90), Some((91.0, 99.0)));
    }

    #[test]
    fn normal_quantiles_match_tables() {
        // Published values (Abramowitz and Stegun, table 26.5).
        for (p, z) in [
            (0.5, 0.0),
            (0.975, 1.959_963_984_540_054),
            (0.95, 1.644_853_626_951_472),
            (0.85, 1.036_433_389_493_789_7),
            (0.001, -3.090_232_306_167_813_5),
            (1e-10, -6.361_340_902_404_056),
        ] {
            let got = normal_quantile(p).unwrap_or(f64::NAN);
            assert!(close(got, z, 1e-12), "Φ⁻¹({p}) = {got}, want {z}");
        }
        assert_eq!(normal_quantile(0.0), None);
        assert_eq!(normal_quantile(1.0), None);
        assert!(z_two_sided(0.95).is_some_and(|z| close(z, 1.959_964, 1e-6)));
    }

    #[test]
    fn ecdf_steps_quantiles_and_censoring() {
        let e = Ecdf::new(&[3.0, 1.0, 2.0, 2.0, f64::INFINITY, f64::NAN]);
        assert_eq!(e.n(), 5);
        assert_eq!(e.finite(), &[1.0, 2.0, 2.0, 3.0]);
        assert_eq!(e.at(0.5), Some(0.0));
        assert_eq!(e.at(2.0), Some(0.6));
        assert_eq!(e.at(100.0), Some(0.8));
        assert_eq!(e.steps(), vec![(1.0, 0.2), (2.0, 0.6), (3.0, 0.8)]);
        assert_eq!(e.quantile(0.5), Some(2.0));
        // The 90th percentile lies among the censored values: unknown.
        assert_eq!(e.quantile(0.9), None);
        assert_eq!(Ecdf::new(&[]).at(1.0), None);
    }

    #[test]
    fn dkw_band() {
        // n = 100 at 95%: √(ln 40 / 200) = 0.1358.
        assert!(dkw_half_width(100, 0.95).is_some_and(|e| close(e, 0.135_810, 1e-6)));
        assert_eq!(dkw_half_width(0, 0.95), None);
        assert_eq!(dkw_half_width(10, 1.0), None);
    }

    #[test]
    fn wilson_interval() {
        // 8 of 10 at 95%, worked by hand from Wilson's formula: centre 0.7167, half-width
        // 0.2266.
        let (lo, hi) = Proportion::new(8, 10).wilson(0.95).unwrap_or_default();
        assert!(
            close(lo, 0.4902, 5e-5) && close(hi, 0.9433, 5e-5),
            "{lo} {hi}"
        );
        // 0 of 10: [0, 0.2775].
        let (lo, hi) = Proportion::new(0, 10).wilson(0.95).unwrap_or_default();
        assert!(lo == 0.0 && close(hi, 0.2775, 5e-5), "{hi}");
        let (lo, hi) = Proportion::new(10, 10).wilson(0.95).unwrap_or_default();
        assert!(close(lo, 0.7225, 5e-5) && hi == 1.0);
        assert_eq!(Proportion::new(0, 0).wilson(0.95), None);
        assert_eq!(Proportion::new(5, 3), Proportion { successes: 3, n: 3 });
    }
}
