//! Goodness-of-fit verdicts: whether the Kolmogorov–Smirnov distance D or the Anderson–Darling
//! A² of data against a fitted distribution is too large for the fit to stand, at a level.
//!
//! - **All parameters known** (and, conservatively, any family without a case of its own): the
//!   critical points are computed from the limiting distributions. Kolmogorov's, P(√n D ≤ x) →
//!   1 − 2 Σ (−1)^(k−1) e^(−2k²x²) (Kolmogorov, 1933), with Stephens's (1970) modification
//!   (√n + 0.12 + 0.11/√n) D, which makes the limiting points hold for n ≥ 5; and Anderson and
//!   Darling's (1954) series for the limit of A² ("Asymptotic theory of certain goodness of fit
//!   criteria based on stochastic processes", *Annals of Mathematical Statistics* 23, 193–212,
//!   1952, and "A test of goodness of fit", *JASA* 49, 765–769, 1954), whose points hold for
//!   n ≥ 5 unmodified.
//! - **Parameters estimated from the same data**, for the families with a case of their own
//!   (normal with mean X̄ and standard deviation S, divisor n − 1, and log-normal through the
//!   logarithms; exponential with mean X̄; Weibull by maximum likelihood): the statistics are
//!   modified as Stephens gave them and compared with his asymptotic points at 95% (M. A.
//!   Stephens, "EDF statistics for goodness of fit and some comparisons", *JASA* 69, 730–737,
//!   1974; for the Weibull, Chandra, Singpurwalla and Stephens, *JASA* 76, 729–731, 1981, and
//!   R. B. D'Agostino and M. A. Stephens, *Goodness-of-Fit Techniques*, Dekker, 1986, ch. 4).
//!   These points are smaller than the all-known ones: estimating the parameters from the data
//!   brings the fit closer to them. For the Weibull's D the point for an infinite sample is used,
//!   the largest of those for any n, so the test is conservative for small samples.
//! - A family with no case of its own (gamma, Erlang, uniform, triangular) is judged against the
//!   all-known points. With its parameters estimated, that test is **conservative**: it rejects
//!   less often than the level says, so a rejection stands but "not rejected" is weaker evidence
//!   (A. M. Law, *Simulation Modeling and Analysis*, 5th ed., McGraw-Hill, 2015, section 6.6.2,
//!   p. 352).
//!
//! With many data, these tests reject almost any fitted distribution, since no real data come
//! exactly from one: a rejection then says the fit is not exact, not that it is not useful
//! (Law, section 6.6.2).

/// What the critical points of a test are for: which parameters were estimated, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    /// Every parameter known in advance, or a family with no case of its own (conservative).
    AllKnown,
    /// A normal with mean X̄ and standard deviation S (divisor n − 1), or a log-normal by the
    /// same on the logarithms.
    Normal,
    /// An exponential with mean X̄.
    Exponential,
    /// A Weibull with both parameters by maximum likelihood.
    Weibull,
}

impl Case {
    /// Its name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Case::AllKnown => "all_parameters_known",
            Case::Normal => "normal",
            Case::Exponential => "exponential",
            Case::Weibull => "weibull",
        }
    }
}

/// The level the verdicts are given at.
pub const LEVEL: f64 = 0.95;

/// Kolmogorov's limiting distribution, P(K ≤ x) = 1 − 2 Σ_{k≥1} (−1)^(k−1) e^(−2k²x²).
pub fn kolmogorov_cdf(x: f64) -> f64 {
    if x.is_nan() || x <= 0.0 {
        return 0.0;
    }
    if x.is_infinite() {
        return 1.0;
    }
    let mut sum = 0.0;
    for k in 1..=200u32 {
        let k = f64::from(k);
        let term = libm::exp(-2.0 * k * k * x * x);
        sum += if k % 2.0 == 1.0 { term } else { -term };
        if term < 1e-18 {
            break;
        }
    }
    (1.0 - 2.0 * sum).clamp(0.0, 1.0)
}

/// The limiting distribution of the Anderson–Darling A² (Anderson and Darling, 1952 and 1954):
/// P(A² < z) = (√(2π)/z) Σ_{j≥0} a_j (4j + 1) e^(−(4j+1)²π²/(8z))
/// ∫₀^∞ e^(z/(8(w² + 1)) − (4j+1)²π²w²/(8z)) dw, with a_j = (−1)^j Γ(j + ½)/(Γ(½) j!). The
/// integral by Simpson's rule over twelve widths of its Gaussian factor.
pub fn anderson_darling_cdf(z: f64) -> f64 {
    if z.is_nan() || z <= 0.0 {
        return 0.0;
    }
    if z.is_infinite() {
        return 1.0;
    }
    let pi = std::f64::consts::PI;
    let mut total = 0.0;
    for j in 0..60u32 {
        let jf = f64::from(j);
        let c = 4.0 * jf + 1.0;
        let outside = -c * c * pi * pi / (8.0 * z);
        if outside < -745.0 {
            break;
        }
        let a = libm::exp(libm::lgamma(jf + 0.5) - libm::lgamma(0.5) - libm::lgamma(jf + 1.0));
        let a = if j % 2 == 0 { a } else { -a };
        let width = (4.0 * z).sqrt() / (c * pi);
        let steps = 4000u32;
        let h = 12.0 * width / f64::from(steps);
        let mut sum = 0.0;
        for i in 0..=steps {
            let w = f64::from(i) * h;
            let v = libm::exp(z / (8.0 * (w * w + 1.0)) - c * c * pi * pi * w * w / (8.0 * z));
            let weight = if i == 0 || i == steps {
                1.0
            } else if i % 2 == 1 {
                4.0
            } else {
                2.0
            };
            sum += weight * v;
        }
        total += a * c * libm::exp(outside) * sum * h / 3.0;
    }
    ((2.0 * pi).sqrt() / z * total).clamp(0.0, 1.0)
}

/// The x with `cdf(x) = level`, by bisection on [lo, hi] (cdf rising).
fn solve(cdf: impl Fn(f64) -> f64, level: f64, mut lo: f64, mut hi: f64) -> f64 {
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if mid <= lo || mid >= hi {
            break;
        }
        if cdf(mid) < level {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The all-parameters-known critical point of the modified D, (√n + 0.12 + 0.11/√n) D, at
/// `level` (1.358 at 95%), from Kolmogorov's limit. `None` for a level not strictly between 0
/// and 1.
pub fn kolmogorov_critical(level: f64) -> Option<f64> {
    (level > 0.0 && level < 1.0).then(|| solve(kolmogorov_cdf, level, 0.0, 10.0))
}

/// The all-parameters-known critical point of A² at `level` (2.492 at 95%), from its limit.
/// `None` for a level not strictly between 0 and 1.
pub fn anderson_darling_critical(level: f64) -> Option<f64> {
    (level > 0.0 && level < 1.0).then(|| solve(anderson_darling_cdf, level, 0.01, 50.0))
}

/// D modified for `case` with `n` values, to be compared with [`ks_point`].
pub fn ks_modified(case: Case, n: u64, d: f64) -> f64 {
    let n = n as f64;
    let r = n.sqrt();
    match case {
        Case::AllKnown => (r + 0.12 + 0.11 / r) * d,
        Case::Normal => (r - 0.01 + 0.85 / r) * d,
        Case::Exponential => (d - 0.2 / n) * (r + 0.26 + 0.5 / r),
        Case::Weibull => r * d,
    }
}

/// A² modified for `case` with `n` values, to be compared with [`ad_point`].
pub fn ad_modified(case: Case, n: u64, a2: f64) -> f64 {
    let n = n as f64;
    match case {
        Case::AllKnown => a2,
        Case::Normal => (1.0 + 4.0 / n - 25.0 / (n * n)) * a2,
        Case::Exponential => (1.0 + 0.6 / n) * a2,
        Case::Weibull => (1.0 + 0.2 / n.sqrt()) * a2,
    }
}

/// The 95% point of the modified D for `case`: computed for all parameters known, Stephens's
/// asymptotic points otherwise.
pub fn ks_point(case: Case) -> f64 {
    match case {
        Case::AllKnown => kolmogorov_critical(LEVEL).unwrap_or(f64::NAN),
        Case::Normal => 0.895,
        Case::Exponential => 1.094,
        Case::Weibull => 0.874,
    }
}

/// The 95% point of the modified A² for `case`: computed for all parameters known, Stephens's
/// asymptotic points otherwise.
pub fn ad_point(case: Case) -> f64 {
    match case {
        Case::AllKnown => {
            // Worked out once: the series is slow.
            static POINT: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
            *POINT.get_or_init(|| anderson_darling_critical(LEVEL).unwrap_or(f64::NAN))
        }
        Case::Normal => 0.751,
        Case::Exponential => 1.321,
        Case::Weibull => 0.757,
    }
}

/// A test's verdict at 95%.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    /// The statistic as modified for its case.
    pub modified: f64,
    /// The 95% point it is compared with.
    pub critical: f64,
    /// The modified statistic is above the point: the fit is rejected at 5%.
    pub rejected: bool,
}

/// The Kolmogorov–Smirnov verdict on distance `d` over `n` values in `case`.
pub fn ks_verdict(case: Case, n: u64, d: f64) -> Verdict {
    let modified = ks_modified(case, n, d);
    let critical = ks_point(case);
    Verdict {
        modified,
        critical,
        rejected: modified > critical,
    }
}

/// The Anderson–Darling verdict on `a2` over `n` values in `case`. An infinite A² (a value
/// where the fit has no probability) is rejected.
pub fn ad_verdict(case: Case, n: u64, a2: f64) -> Verdict {
    let modified = ad_modified(case, n, a2);
    let critical = ad_point(case);
    Verdict {
        modified,
        critical,
        rejected: modified.is_nan() || modified > critical,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Law, 5th ed., Table 6.15 (p. 354), all parameters known: 1.138, 1.224, 1.358, 1.480 and
    /// 1.628 at 0.85 to 0.99. Each computed point rounds to it.
    #[test]
    fn kolmogorov_s_points_match_the_published_row() {
        for (level, want) in [
            (0.85, 1.138),
            (0.90, 1.224),
            (0.95, 1.358),
            (0.975, 1.480),
            (0.99, 1.628),
        ] {
            let got = kolmogorov_critical(level).unwrap_or(f64::NAN);
            assert!((got - want).abs() <= 5e-4, "{level}: {got}");
        }
        // The series against a value known in closed form: P(K ≤ x) at x where only k = 1
        // matters, 1 − 2e^(−2x²).
        let x = 3.0;
        assert!((kolmogorov_cdf(x) - (1.0 - 2.0 * libm::exp(-18.0))).abs() < 1e-15);
    }

    /// Law, 5th ed., Table 6.18 (p. 357), all parameters known: 1.933 and 2.492 at 0.90 and
    /// 0.95, as computed. Its 0.975 and 0.99 entries, 3.070 and 3.857, are below the limit
    /// computed here, 3.0775 and 3.8781 (the series converges to these to 1e-12 with twice the
    /// terms and five times the steps); the verdicts use 0.95 only.
    #[test]
    fn anderson_darling_s_points_match_the_published_row_where_they_hold() {
        let at = |l: f64| anderson_darling_critical(l).unwrap_or(f64::NAN);
        assert!((at(0.90) - 1.933).abs() <= 5e-4, "{}", at(0.90));
        assert!((at(0.95) - 2.492).abs() <= 5e-4, "{}", at(0.95));
        assert!((at(0.975) - 3.0775).abs() <= 5e-4, "{}", at(0.975));
        assert!((at(0.99) - 3.8781).abs() <= 5e-4, "{}", at(0.99));
        assert_eq!(anderson_darling_cdf(0.0), 0.0);
        assert!(anderson_darling_cdf(40.0) > 0.999_999);
    }

    /// Law, 5th ed., Example 6.19 (p. 357): an exponential fitted to 219 interarrival times,
    /// A² = 0.558, modified (1 + 0.6/219) 0.558 = 0.560, not rejected.
    #[test]
    fn the_book_s_exponential_example() {
        let v = ad_verdict(Case::Exponential, 219, 0.558);
        assert!((v.modified - 0.560).abs() < 5e-4, "{v:?}");
        assert!(!v.rejected);
        // The same A² in the all-known case is far from 2.492.
        assert!(!ad_verdict(Case::AllKnown, 219, 0.558).rejected);
        // An infinite A² is rejected.
        assert!(ad_verdict(Case::Weibull, 50, f64::INFINITY).rejected);
    }

    #[test]
    fn modified_statistics() {
        // √100 = 10: (10 + 0.12 + 0.011) D.
        assert!((ks_modified(Case::AllKnown, 100, 0.1) - 1.0131).abs() < 1e-12);
        assert!((ks_modified(Case::Normal, 100, 0.1) - 1.0075).abs() < 1e-12);
        assert!((ks_modified(Case::Exponential, 100, 0.1) - 0.098 * 10.31).abs() < 1e-12);
        assert!((ks_modified(Case::Weibull, 100, 0.1) - 1.0).abs() < 1e-12);
        assert!((ad_modified(Case::Normal, 100, 1.0) - 1.0375).abs() < 1e-12);
        assert!((ad_modified(Case::Weibull, 100, 1.0) - 1.02).abs() < 1e-12);
        assert_eq!(Case::AllKnown.name(), "all_parameters_known");
    }
}
