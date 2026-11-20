//! The inverse of each distribution's cumulative distribution function, which Latin hypercube
//! sampling draws through: x = F⁻¹(u) for a u in each stratum.
//!
//! Closed forms where there are any (exponential, uniform, triangular, Weibull, the normal
//! family through Φ⁻¹, a user distribution's piecewise-linear curve). The gamma, Erlang and beta
//! are inverted numerically: Newton's method on their distribution functions
//! (`stochcraft_random::fit::cdf`, `stochcraft_random::estimate::beta_cdf`), kept inside a
//! bracketing interval by bisection. Counts (Poisson, binomial, a discrete user distribution)
//! give the smallest value whose cumulative probability exceeds u, found for a whole batch of u
//! in one walk up their probabilities.
//!
//! Unlike `Dist::sample`, a constant is its value as given, negative or not: a Monte Carlo input
//! is a quantity, not a duration.

use stochcraft_model::Dist;
use stochcraft_random::math::{exp, ln, powf};
use stochcraft_random::{estimate, fit, ln_gamma};
use stochcraft_stats::spread::{normal_cdf, normal_quantile};

/// The smallest probability drawn through an inverse: 2⁻⁵³, so 1 − it is still below 1.
pub(crate) const TINY: f64 = 1.0 / 9_007_199_254_740_992.0;

/// Whether a distribution takes only separate values (a count or a discrete table).
pub fn is_discrete(d: &Dist) -> bool {
    matches!(
        d,
        Dist::Poisson { .. } | Dist::Binomial { .. } | Dist::User { discrete: true, .. }
    )
}

/// The values at probabilities `ps` (each in (0, 1)), in the same order. `d` must pass
/// `Dist::check`.
pub fn quantiles(d: &Dist, ps: &[f64]) -> Vec<f64> {
    if is_discrete(d) {
        return discrete(d, ps);
    }
    ps.iter().map(|&p| continuous(d, p)).collect()
}

/// The value at probability `p` in (0, 1).
pub fn quantile(d: &Dist, p: f64) -> f64 {
    quantiles(d, &[p]).first().copied().unwrap_or(f64::NAN)
}

fn z(p: f64) -> f64 {
    normal_quantile(p.clamp(f64::MIN_POSITIVE, 1.0 - TINY)).unwrap_or(0.0)
}

fn continuous(d: &Dist, p: f64) -> f64 {
    let p = p.clamp(TINY, 1.0 - TINY);
    match d {
        Dist::Constant(v) => *v,
        Dist::Exponential { mean } => -mean * ln(1.0 - p),
        Dist::Uniform { min, max } => min + p * (max - min),
        Dist::Triangular { min, mode, max } => {
            let split = (mode - min) / (max - min);
            if p < split {
                min + (p * (max - min) * (mode - min)).sqrt()
            } else {
                max - ((1.0 - p) * (max - min) * (max - mode)).sqrt()
            }
        }
        Dist::Normal { mean, std_dev } => mean + std_dev * z(p),
        Dist::TruncatedNormal {
            mean,
            std_dev,
            min,
            max,
        } => {
            let (a, b) = ((min - mean) / std_dev, (max - mean) / std_dev);
            // In whichever tail keeps the probabilities away from 1, where they lose digits.
            let x = if a > 0.0 {
                let (sa, sb) = (normal_cdf(-a), normal_cdf(-b));
                mean - std_dev * z(sa - p * (sa - sb))
            } else {
                let (fa, fb) = (normal_cdf(a), normal_cdf(b));
                mean + std_dev * z(fa + p * (fb - fa))
            };
            x.clamp(*min, *max)
        }
        Dist::LogNormal { mean, std_dev } => {
            let cv = std_dev / mean;
            let s2 = log1p(cv * cv);
            let mu = ln(*mean) - 0.5 * s2;
            exp(mu + s2.sqrt() * z(p))
        }
        Dist::Weibull { shape, scale } => scale * powf(-ln(1.0 - p), 1.0 / shape),
        Dist::Gamma { shape, scale } => {
            let (mean, sd) = (shape * scale, shape.sqrt() * scale);
            invert_positive(d, p, mean, sd)
        }
        Dist::Erlang { mean, k } => invert_positive(d, p, *mean, mean / (*k as f64).sqrt()),
        Dist::Beta { shape: a, scale: b } => beta(p, *a, *b),
        Dist::User { points, .. } => user_continuous(points, p),
        // Counts are handled by `discrete`.
        Dist::Poisson { .. } | Dist::Binomial { .. } => {
            discrete(d, &[p]).first().copied().unwrap_or(f64::NAN)
        }
    }
}

/// ln(1 + x), accurate for small x.
fn log1p(x: f64) -> f64 {
    // `stochcraft_random::math` has no log1p; ln(1 + x) loses digits only for x far below the
    // coefficients of variation inputs have, and is identical on every target.
    if x < 1e-8 {
        x - 0.5 * x * x
    } else {
        ln(1.0 + x)
    }
}

/// Newton's method on `cdf`, kept inside [lo, hi] (which brackets p) by bisection.
fn invert(cdf: &dyn Fn(f64) -> f64, pdf: &dyn Fn(f64) -> f64, p: f64, lo: f64, hi: f64) -> f64 {
    let (mut lo, mut hi) = (lo, hi);
    let mut x = 0.5 * (lo + hi);
    for _ in 0..200 {
        let f = cdf(x) - p;
        if f == 0.0 {
            return x;
        }
        if f < 0.0 {
            lo = x;
        } else {
            hi = x;
        }
        let d = pdf(x);
        let newton = if d > 0.0 && d.is_finite() {
            x - f / d
        } else {
            f64::NAN
        };
        let next = if newton > lo && newton < hi {
            newton
        } else {
            0.5 * (lo + hi)
        };
        if (next - x).abs() <= 1e-14 * x.abs().max(f64::MIN_POSITIVE) || hi - lo <= 1e-15 * hi {
            return next;
        }
        x = next;
    }
    x
}

/// A gamma or Erlang quantile: bracketed from 0 up to where the distribution reaches `p`.
fn invert_positive(d: &Dist, p: f64, mean: f64, sd: f64) -> f64 {
    let cdf = |x: f64| fit::cdf(d, x).unwrap_or(0.0);
    let pdf = |x: f64| fit::density(d, x).unwrap_or(0.0);
    let mut hi = (mean + 10.0 * sd).max(f64::MIN_POSITIVE);
    let mut steps = 0;
    while cdf(hi) < p && hi.is_finite() && steps < 2_000 {
        hi *= 2.0;
        steps += 1;
    }
    invert(&cdf, &pdf, p, 0.0, hi)
}

/// The beta quantile on [0, 1] with shapes `a` and `b`.
fn beta(p: f64, a: f64, b: f64) -> f64 {
    let ln_b = ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b);
    let cdf = |x: f64| estimate::beta_cdf(x, a, b);
    let pdf = |x: f64| {
        if x <= 0.0 || x >= 1.0 {
            0.0
        } else {
            exp((a - 1.0) * ln(x) + (b - 1.0) * ln(1.0 - x) - ln_b)
        }
    };
    invert(&cdf, &pdf, p, 0.0, 1.0)
}

/// The normalised points of a user distribution, sorted by value.
fn sorted_points(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let total: f64 = points.iter().map(|(p, _)| *p).sum();
    let mut pts: Vec<(f64, f64)> = points.iter().map(|(p, v)| (p / total, *v)).collect();
    pts.sort_by(|a, b| a.1.total_cmp(&b.1));
    pts
}

/// A continuous user distribution's value at `u`: the first point's probability is an atom at
/// the lowest value, and each later point's spreads evenly back to the value before it (as
/// `Dist::User` draws).
fn user_continuous(points: &[(f64, f64)], u: f64) -> f64 {
    let mut cumulative = 0.0;
    let mut previous: Option<f64> = None;
    for (p, v) in sorted_points(points) {
        let below = cumulative;
        cumulative += p;
        if u < cumulative {
            return match previous {
                Some(v0) if p > 0.0 => v0 + (v - v0) * (u - below) / p,
                _ => v,
            };
        }
        previous = Some(v);
    }
    previous.unwrap_or(0.0)
}

/// The values of a discrete distribution at each of `ps`: the smallest value whose cumulative
/// probability exceeds p, found in one walk up the values for the probabilities in order.
fn discrete(d: &Dist, ps: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..ps.len()).collect();
    order.sort_by(|&i, &j| ps[i].total_cmp(&ps[j]));
    let mut out = vec![0.0; ps.len()];
    match d {
        Dist::User { points, .. } => {
            let pts = sorted_points(points);
            let last = pts.len().saturating_sub(1);
            let (mut k, mut cumulative) = (0, pts.first().map_or(1.0, |p| p.0));
            for &i in &order {
                while k < last && ps[i] >= cumulative {
                    k += 1;
                    cumulative += pts[k].0;
                }
                out[i] = pts.get(k).map_or(0.0, |p| p.1);
            }
        }
        Dist::Poisson { mean } => {
            let m = *mean;
            let ln_pmf = |k: f64| -m + k * ln(m) - ln_gamma(k + 1.0);
            walk(&order, ps, &mut out, m + 50.0 * m.sqrt() + 100.0, |k| {
                if m <= 0.0 {
                    f64::from(u8::from(k == 0.0))
                } else {
                    exp(ln_pmf(k))
                }
            });
        }
        Dist::Binomial {
            probability,
            trials,
        } => {
            let (q, n) = (*probability, *trials as f64);
            let ln_choose = |k: f64| ln_gamma(n + 1.0) - ln_gamma(k + 1.0) - ln_gamma(n - k + 1.0);
            walk(&order, ps, &mut out, n, |k| {
                if q <= 0.0 {
                    f64::from(u8::from(k == 0.0))
                } else if q >= 1.0 {
                    f64::from(u8::from(k == n))
                } else {
                    exp(ln_choose(k) + k * ln(q) + (n - k) * ln(1.0 - q))
                }
            });
        }
        _ => {
            for &i in &order {
                out[i] = continuous(d, ps[i]);
            }
        }
    }
    out
}

/// Walks a count's values 0, 1, 2, … up to `limit` with probabilities `pmf`, giving each
/// probability (in the ascending `order`) the first value whose cumulative probability exceeds it.
fn walk(order: &[usize], ps: &[f64], out: &mut [f64], limit: f64, pmf: impl Fn(f64) -> f64) {
    let mut k = 0.0;
    let mut cumulative = pmf(0.0);
    for &i in order {
        while ps[i] >= cumulative && k < limit {
            k += 1.0;
            cumulative += pmf(k);
        }
        out[i] = k;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * b.abs().max(1.0)
    }

    /// Each continuous family's quantile is the inverse of its distribution function.
    #[test]
    fn quantiles_invert_the_distribution_functions() {
        let families = [
            Dist::Exponential { mean: 2.5 },
            Dist::Uniform {
                min: -1.0,
                max: 3.0,
            },
            Dist::Triangular {
                min: 1.0,
                mode: 2.0,
                max: 6.0,
            },
            Dist::Normal {
                mean: 10.0,
                std_dev: 2.0,
            },
            Dist::LogNormal {
                mean: 5.0,
                std_dev: 2.0,
            },
            Dist::Gamma {
                shape: 0.7,
                scale: 3.0,
            },
            Dist::Gamma {
                shape: 40.0,
                scale: 0.5,
            },
            Dist::Erlang { mean: 6.0, k: 3 },
            Dist::Weibull {
                shape: 1.5,
                scale: 4.0,
            },
        ];
        for d in &families {
            for p in [0.001, 0.05, 0.3, 0.5, 0.77, 0.95, 0.999] {
                let x = quantile(d, p);
                let back = fit::cdf(d, x).unwrap_or(f64::NAN);
                assert!(close(back, p, 1e-9), "{d:?} at {p}: x {x}, F(x) {back}");
            }
        }
    }

    #[test]
    fn beta_and_truncated_normal_and_constants() {
        for p in [0.01, 0.4, 0.9] {
            let x = quantile(
                &Dist::Beta {
                    shape: 2.0,
                    scale: 5.0,
                },
                p,
            );
            assert!(close(estimate::beta_cdf(x, 2.0, 5.0), p, 1e-9), "{p}: {x}");
        }
        // A normal truncated far in its upper tail stays inside its bounds and is monotone.
        let t = Dist::TruncatedNormal {
            mean: 0.0,
            std_dev: 1.0,
            min: 8.0,
            max: 9.0,
        };
        let xs = quantiles(&t, &[0.1, 0.5, 0.9]);
        assert!(xs.iter().all(|x| (8.0..=9.0).contains(x)), "{xs:?}");
        assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
        // The middle of N(0, 1) truncated to [-1, 1] is 0.
        let mid = quantile(
            &Dist::TruncatedNormal {
                mean: 0.0,
                std_dev: 1.0,
                min: -1.0,
                max: 1.0,
            },
            0.5,
        );
        assert!(mid.abs() < 1e-12, "{mid}");
        assert_eq!(quantile(&Dist::Constant(-4.0), 0.3), -4.0);
    }

    #[test]
    fn counts_and_tables() {
        // Poisson(2): F(0) = 0.1353, F(1) = 0.4060, F(2) = 0.6767, F(3) = 0.8571.
        let got = quantiles(&Dist::Poisson { mean: 2.0 }, &[0.9, 0.1, 0.5, 0.2, 0.7]);
        assert_eq!(got, vec![4.0, 0.0, 2.0, 1.0, 3.0]);
        // Binomial(4, 0.5): F = 1/16, 5/16, 11/16, 15/16, 1.
        let b = Dist::Binomial {
            probability: 0.5,
            trials: 4,
        };
        assert_eq!(
            quantiles(&b, &[0.05, 0.07, 0.5, 0.95]),
            vec![0.0, 1.0, 2.0, 4.0]
        );
        let user = Dist::User {
            points: vec![(0.75, 4.0), (0.25, 1.0)],
            discrete: true,
        };
        assert_eq!(quantiles(&user, &[0.1, 0.3, 0.99]), vec![1.0, 4.0, 4.0]);
        // A continuous table: an atom of 0.25 at 1, then 0.75 spread evenly to 4.
        let curve = Dist::User {
            points: vec![(0.25, 1.0), (0.75, 4.0)],
            discrete: false,
        };
        assert_eq!(quantile(&curve, 0.1), 1.0);
        assert!(close(quantile(&curve, 0.625), 2.5, 1e-12));
    }
}
