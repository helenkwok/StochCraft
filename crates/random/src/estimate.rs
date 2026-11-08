//! Distributions fitted from three-point estimates: the minimum, the most likely value and the
//! maximum (or the 95th percentile) that an expert gives when there are no data.
//!
//! Each fit is a standard BPSim distribution, so a file using it reads in any BPSim tool:
//!
//! - **Triangular** on [min, max] peaking at the mode: BPSim's `TriangularDistribution`. Given a
//!   95th percentile instead of a maximum, the maximum is solved for.
//! - **PERT**: the beta distribution scaled to [min, max] with shapes α = 1 + 4(mode − min) /
//!   (max − min) and β = 1 + 4(max − mode) / (max − min), whose mean is (min + 4·mode + max) / 6
//!   (Malcolm, Roseboom, Clark and Fazar, 1959; the shapes as in Vose, *Risk Analysis*, 2008).
//!   BPSim's beta has no bounds, so it is written as a continuous `UserDistribution`: its
//!   quantiles at every 1% (a piecewise-linear cumulative distribution through them).
//! - **Log-normal** with its lower end at the minimum, its mode at the most likely value and its
//!   95th percentile as given: with Y = X − min ~ LogNormal(μ, σ²), the mode e^(μ − σ²) and the
//!   95th percentile e^(μ + 1.645σ) fix σ from ln(p95 / mode) = σ² + 1.645σ.
//! - **Weibull** likewise: shape k > 1 and scale λ from the mode λ((k − 1)/k)^(1/k) and the 95th
//!   percentile λ(−ln 0.05)^(1/k).
//!
//! The log-normal and Weibull have no maximum, so they take a 95th percentile. With a minimum of
//! 0 they are BPSim's own `LogNormalDistribution` and `WeibullDistribution`; otherwise, since
//! BPSim's have no shift, they too are written as a `UserDistribution` of quantiles (every 1% to
//! the 99th, then the 99.5th, 99.9th and 99.99th, where it ends).

use crate::Dist;
use crate::dist::ln_gamma;
use crate::math::{exp, ln, powf};

/// Φ⁻¹(0.95), the standard normal's 95th percentile.
const Z95: f64 = 1.644_853_626_951_472_2;

/// Which distribution a three-point estimate is fitted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Triangular,
    Pert,
    LogNormal,
    Weibull,
}

impl Family {
    pub const ALL: [Family; 4] = [
        Family::Triangular,
        Family::Pert,
        Family::LogNormal,
        Family::Weibull,
    ];

    /// A stable name for files and agents.
    pub fn name(self) -> &'static str {
        match self {
            Family::Triangular => "triangular",
            Family::Pert => "pert",
            Family::LogNormal => "lognormal",
            Family::Weibull => "weibull",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == s)
    }
}

/// What the third point of an estimate is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upper {
    /// The largest value possible.
    Max,
    /// The value exceeded one time in twenty.
    P95,
}

impl Upper {
    pub fn name(self) -> &'static str {
        match self {
            Upper::Max => "max",
            Upper::P95 => "p95",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        [Upper::Max, Upper::P95].into_iter().find(|u| u.name() == s)
    }
}

/// A three-point estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreePoint {
    pub family: Family,
    pub min: f64,
    /// The most likely value.
    pub mode: f64,
    /// The maximum or the 95th percentile, as `upper_is` says.
    pub upper: f64,
    pub upper_is: Upper,
}

/// Why an estimate cannot be fitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitError {
    /// A point is NaN or infinite.
    NotFinite,
    /// The points are not in order: min ≤ mode ≤ max with min < max for a triangular or PERT
    /// estimate (and mode < p95 when the upper point is the 95th percentile); min < mode < p95 for
    /// a log-normal or Weibull one.
    NotInOrder,
    /// A log-normal or Weibull estimate given a maximum, which they do not have.
    NoMaximum,
}

impl std::fmt::Display for FitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FitError::NotFinite => "every point must be a finite number",
            FitError::NotInOrder => {
                "the points must be in order: minimum, most likely, then maximum or 95th percentile (the most likely value above the minimum for a log-normal or Weibull estimate)"
            }
            FitError::NoMaximum => {
                "log-normal and Weibull distributions have no maximum: give the 95th percentile"
            }
        })
    }
}

impl std::error::Error for FitError {}

impl ThreePoint {
    /// The fitted distribution, as BPSim writes it.
    pub fn fit(&self) -> Result<Dist, FitError> {
        let Self {
            family,
            min,
            mode,
            upper,
            upper_is,
        } = *self;
        if ![min, mode, upper].iter().all(|v| v.is_finite()) {
            return Err(FitError::NotFinite);
        }
        match family {
            Family::Triangular | Family::Pert => {
                let ordered = match upper_is {
                    Upper::Max => min <= mode && mode <= upper && min < upper,
                    Upper::P95 => min <= mode && mode < upper,
                };
                if !ordered {
                    return Err(FitError::NotInOrder);
                }
                let max = match upper_is {
                    Upper::Max => upper,
                    Upper::P95 => solve_max(family, min, mode, upper),
                };
                Ok(match family {
                    Family::Triangular => Dist::Triangular { min, mode, max },
                    _ => {
                        let (a, b) = pert_shapes(min, mode, max);
                        table(|p| min + (max - min) * beta_quantile(p, a, b), true)
                    }
                })
            }
            Family::LogNormal | Family::Weibull => {
                if upper_is == Upper::Max {
                    return Err(FitError::NoMaximum);
                }
                if !(min < mode && mode < upper) {
                    return Err(FitError::NotInOrder);
                }
                let (m, q) = (mode - min, upper - min);
                if family == Family::LogNormal {
                    let sigma = lognormal_sigma(m, q);
                    let mu = ln(m) + sigma * sigma;
                    if min == 0.0 {
                        let mean = exp(mu + sigma * sigma / 2.0);
                        let var = (exp(sigma * sigma) - 1.0) * exp(2.0 * mu + sigma * sigma);
                        return Ok(Dist::LogNormal {
                            mean,
                            std_dev: var.sqrt(),
                        });
                    }
                    Ok(table(
                        |p| {
                            if p <= 0.0 {
                                min
                            } else {
                                min + exp(mu + sigma * normal_quantile(p))
                            }
                        },
                        false,
                    ))
                } else {
                    let (k, lambda) = weibull_params(m, q);
                    if min == 0.0 {
                        return Ok(Dist::Weibull {
                            shape: k,
                            scale: lambda,
                        });
                    }
                    Ok(table(|p| min + lambda * powf(-ln(1.0 - p), 1.0 / k), false))
                }
            }
        }
    }
}

/// PERT's beta shapes for [min, max] and the mode.
fn pert_shapes(min: f64, mode: f64, max: f64) -> (f64, f64) {
    let r = max - min;
    (1.0 + 4.0 * (mode - min) / r, 1.0 + 4.0 * (max - mode) / r)
}

/// The maximum whose triangular or PERT distribution (with `min` and `mode`) has `p95` as its
/// 95th percentile. The percentile grows with the maximum, so bisection finds it.
fn solve_max(family: Family, min: f64, mode: f64, p95: f64) -> f64 {
    let q95 = |max: f64| match family {
        Family::Triangular => {
            // At 95% the quantile is on the upper side unless the mode is nearly the maximum.
            let (a, b, c) = (min, max, mode);
            let fc = (c - a) / (b - a);
            if 0.95 >= fc {
                b - ((1.0 - 0.95) * (b - a) * (b - c)).sqrt()
            } else {
                a + (0.95 * (b - a) * (c - a)).sqrt()
            }
        }
        _ => {
            let (a, b) = pert_shapes(min, mode, max);
            min + (max - min) * beta_quantile(0.95, a, b)
        }
    };
    let mut lo = p95;
    let mut hi = p95 + (p95 - min).max(f64::MIN_POSITIVE);
    while q95(hi) < p95 && hi.is_finite() {
        hi = p95 + 2.0 * (hi - p95);
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if q95(mid) < p95 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// σ of the log-normal with mode `m` and 95th percentile `q`: the positive root of
/// σ² + z₉₅σ − ln(q/m) = 0.
fn lognormal_sigma(m: f64, q: f64) -> f64 {
    let d = ln(q / m);
    (-Z95 + (Z95 * Z95 + 4.0 * d).sqrt()) / 2.0
}

/// Weibull shape and scale with mode `m` and 95th percentile `q` (q > m > 0): the ratio q/m is
/// (−ln 0.05)^(1/k) / ((k − 1)/k)^(1/k), which falls from infinity at k = 1 towards 1 as k
/// grows, so bisection on k finds it.
fn weibull_params(m: f64, q: f64) -> (f64, f64) {
    let target = ln(q / m);
    let c = ln(-ln(0.05));
    let ratio = |k: f64| (c - ln((k - 1.0) / k)) / k;
    let (mut lo, mut hi) = (1.0 + 1e-12, 1_000.0);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if ratio(mid) > target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let k = 0.5 * (lo + hi);
    (k, m / powf((k - 1.0) / k, 1.0 / k))
}

/// A continuous user distribution through the quantiles `q(p)`: at every 1% from 0 (bounded)
/// or, for an unbounded upper tail, every 1% to 99% then 99.5%, 99.9% and 99.99%.
fn table(q: impl Fn(f64) -> f64, bounded: bool) -> Dist {
    let mut ps: Vec<f64> = (0..=100).map(|i| f64::from(i) / 100.0).collect();
    if !bounded {
        ps.pop();
        ps.extend([0.995, 0.999, 0.9999]);
    }
    let mut points = Vec::with_capacity(ps.len());
    let mut previous = 0.0;
    for p in ps {
        // Quantiles at exactly 0 and 1 of a beta are its ends; rounding can step past them.
        let v = q(p);
        points.push((p - previous, round(v)));
        previous = p;
    }
    Dist::User {
        points,
        discrete: false,
    }
}

/// `v` to 12 significant digits, so files hold readable numbers.
fn round(v: f64) -> f64 {
    if v == 0.0 || !v.is_finite() {
        return v;
    }
    let digits = 12 - (libm::floor(libm::log10(v.abs())) as i32) - 1;
    let f = powf(10.0, f64::from(digits));
    libm::round(v * f) / f
}

/// The standard normal quantile, by bisection on Φ (enough for building a table).
fn normal_quantile(p: f64) -> f64 {
    let cdf = |x: f64| 0.5 * libm::erfc(-x / std::f64::consts::SQRT_2);
    let (mut lo, mut hi) = (-40.0, 40.0);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if cdf(mid) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// The regularised incomplete beta function I_x(a, b): the continued fraction of DLMF 8.17.22,
/// evaluated by the modified Lentz method, on whichever of x and 1 − x converges fast
/// (DLMF 8.17.4 for the symmetry).
pub fn beta_cdf(x: f64, a: f64, b: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_front = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * ln(x) + b * ln(1.0 - x);
    if x < (a + 1.0) / (a + b + 2.0) {
        exp(ln_front) * beta_fraction(x, a, b) / a
    } else {
        1.0 - exp(ln_front) * beta_fraction(1.0 - x, b, a) / b
    }
}

/// The continued fraction 1 / (1 + d₁ / (1 + d₂ / (1 + …))) of DLMF 8.17.22.
fn beta_fraction(x: f64, a: f64, b: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let mut c = 1.0;
    let mut d = 1.0 - (a + b) * x / (a + 1.0);
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=300 {
        let m = f64::from(m);
        // The even then the odd coefficient.
        for coefficient in [
            m * (b - m) * x / ((a + 2.0 * m - 1.0) * (a + 2.0 * m)),
            -(a + m) * (a + b + m) * x / ((a + 2.0 * m) * (a + 2.0 * m + 1.0)),
        ] {
            d = 1.0 + coefficient * d;
            if d.abs() < TINY {
                d = TINY;
            }
            c = 1.0 + coefficient / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            h *= d * c;
        }
        if (d * c - 1.0).abs() < 1e-15 {
            break;
        }
    }
    h
}

/// The quantile of the beta distribution with shapes `a` and `b`, by bisection on
/// [`beta_cdf`].
pub fn beta_quantile(p: f64, a: f64, b: f64) -> f64 {
    if p <= 0.0 {
        return 0.0;
    }
    if p >= 1.0 {
        return 1.0;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if beta_cdf(mid, a, b) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rng;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * b.abs().max(1.0)
    }

    fn est(family: Family, min: f64, mode: f64, upper: f64, upper_is: Upper) -> ThreePoint {
        ThreePoint {
            family,
            min,
            mode,
            upper,
            upper_is,
        }
    }

    /// The p-quantile of draws from `d`.
    fn sampled_quantile(d: &Dist, p: f64) -> f64 {
        let mut rng = Rng::new(7);
        let mut xs: Vec<f64> = (0..200_000).map(|_| d.sample(&mut rng)).collect();
        xs.sort_by(f64::total_cmp);
        xs[(p * xs.len() as f64) as usize]
    }

    #[test]
    fn incomplete_beta_matches_closed_forms() {
        // I_x(1, 1) = x; I_x(2, 1) = x²; I_x(1, 3) = 1 − (1 − x)³; I_x(2, 2) = 3x² − 2x³.
        for x in [0.05, 0.3, 0.5, 0.77, 0.99] {
            assert!(close(beta_cdf(x, 1.0, 1.0), x, 1e-13));
            assert!(close(beta_cdf(x, 2.0, 1.0), x * x, 1e-13));
            assert!(close(
                beta_cdf(x, 1.0, 3.0),
                1.0 - (1.0 - x) * (1.0 - x) * (1.0 - x),
                1e-13
            ));
            assert!(close(
                beta_cdf(x, 2.0, 2.0),
                3.0 * x * x - 2.0 * x * x * x,
                1e-13
            ));
        }
        assert!(close(beta_quantile(0.5, 2.0, 2.0), 0.5, 1e-12));
    }

    #[test]
    fn triangular_with_a_maximum_is_bpsims_own() {
        assert_eq!(
            est(Family::Triangular, 2.0, 4.0, 9.0, Upper::Max).fit(),
            Ok(Dist::Triangular {
                min: 2.0,
                mode: 4.0,
                max: 9.0
            })
        );
    }

    #[test]
    fn triangular_from_a_95th_percentile() {
        let d = est(Family::Triangular, 2.0, 4.0, 9.0, Upper::P95).fit();
        let Ok(Dist::Triangular { min, mode, max }) = d else {
            unreachable!("{d:?}")
        };
        assert_eq!((min, mode), (2.0, 4.0));
        // 95th percentile b − √(0.05 (b − 2)(b − 4)) = 9.
        let q = max - (0.05 * (max - 2.0) * (max - 4.0)).sqrt();
        assert!(close(q, 9.0, 1e-12), "{max}");
    }

    #[test]
    fn pert_has_its_mean_and_percentile() {
        let d = est(Family::Pert, 10.0, 20.0, 60.0, Upper::Max)
            .fit()
            .unwrap_or(Dist::Constant(0.0));
        // Mean (10 + 4·20 + 60) / 6 = 25; the 1% table is within 0.2% of it.
        assert!(
            d.mean().is_some_and(|m| close(m, 25.0, 2e-3)),
            "{:?}",
            d.mean()
        );
        // Shapes α = 1 + 4·10/50 = 1.8, β = 1 + 4·40/50 = 4.2; median from the beta.
        let median = 10.0 + 50.0 * beta_quantile(0.5, 1.8, 4.2);
        let got = sampled_quantile(&d, 0.5);
        assert!(close(got, median, 5e-3), "{got} {median}");
        assert!(matches!(d, Dist::User { ref points, discrete: false } if points.len() == 101));
        // A 95th percentile instead: the table's 95th percentile is the one given.
        let d = est(Family::Pert, 10.0, 20.0, 45.0, Upper::P95)
            .fit()
            .unwrap_or(Dist::Constant(0.0));
        let got = sampled_quantile(&d, 0.95);
        assert!(close(got, 45.0, 5e-3), "{got}");
    }

    #[test]
    fn lognormal_has_its_mode_and_percentile() {
        let d = est(Family::LogNormal, 0.0, 30.0, 90.0, Upper::P95).fit();
        let Ok(Dist::LogNormal { mean, std_dev }) = d else {
            unreachable!("{d:?}")
        };
        // Back to μ and σ: σ² = ln(1 + s²/m²), μ = ln m − σ²/2.
        let cv = std_dev / mean;
        let s2 = ln(1.0 + cv * cv);
        let mu = ln(mean) - s2 / 2.0;
        assert!(close(exp(mu - s2), 30.0, 1e-12));
        assert!(close(exp(mu + Z95 * s2.sqrt()), 90.0, 1e-12));
        // Shifted by a minimum: a table, with its 95th percentile where asked.
        let d = est(Family::LogNormal, 5.0, 35.0, 95.0, Upper::P95)
            .fit()
            .unwrap_or(Dist::Constant(0.0));
        assert!(matches!(d, Dist::User { .. }));
        assert!(close(sampled_quantile(&d, 0.95), 95.0, 5e-3));
        assert!(matches!(d, Dist::User { ref points, .. } if points[0] == (0.0, 5.0)));
    }

    #[test]
    fn weibull_has_its_mode_and_percentile() {
        let d = est(Family::Weibull, 0.0, 40.0, 100.0, Upper::P95).fit();
        let Ok(Dist::Weibull { shape: k, scale }) = d else {
            unreachable!("{d:?}")
        };
        assert!(close(scale * powf((k - 1.0) / k, 1.0 / k), 40.0, 1e-9));
        assert!(close(scale * powf(-ln(0.05), 1.0 / k), 100.0, 1e-9));
        let d = est(Family::Weibull, 10.0, 50.0, 110.0, Upper::P95)
            .fit()
            .unwrap_or(Dist::Constant(0.0));
        assert!(close(sampled_quantile(&d, 0.95), 110.0, 5e-3));
    }

    #[test]
    fn bad_estimates() {
        use FitError::*;
        let fit = |f, a, m, b, u| est(f, a, m, b, u).fit().err();
        assert_eq!(
            fit(Family::Pert, 5.0, 3.0, 9.0, Upper::Max),
            Some(NotInOrder)
        );
        assert_eq!(
            fit(Family::Pert, 5.0, 5.0, 5.0, Upper::Max),
            Some(NotInOrder)
        );
        assert_eq!(
            fit(Family::Triangular, 1.0, 9.0, 9.0, Upper::P95),
            Some(NotInOrder)
        );
        assert_eq!(
            fit(Family::LogNormal, 0.0, 3.0, 9.0, Upper::Max),
            Some(NoMaximum)
        );
        assert_eq!(
            fit(Family::Weibull, 3.0, 3.0, 9.0, Upper::P95),
            Some(NotInOrder)
        );
        assert_eq!(
            fit(Family::Weibull, f64::NAN, 3.0, 9.0, Upper::P95),
            Some(NotFinite)
        );
        // Mode at an end is fine for triangular and PERT.
        assert!(fit(Family::Pert, 0.0, 0.0, 9.0, Upper::Max).is_none());
        assert!(fit(Family::Triangular, 0.0, 9.0, 9.0, Upper::Max).is_none());
        assert_eq!(Family::from_name("pert"), Some(Family::Pert));
        assert_eq!(Upper::from_name("p95"), Some(Upper::P95));
    }
}
