//! The inverse of each distribution's cumulative distribution function, which Latin hypercube
//! sampling draws through: x = F⁻¹(u) for a u in each stratum.
//!
//! Closed forms where there are any (exponential, uniform, triangular, Weibull, the normal
//! family through Φ⁻¹, a user distribution's piecewise-linear curve). The gamma, Erlang and beta
//! are inverted numerically: Newton's method on their distribution functions
//! (`stochcraft_random::fit::cdf`, `stochcraft_random::estimate::beta_cdf`), kept inside a
//! bracketing interval by bisection. Counts (Poisson, binomial, a discrete user distribution)
//! give the smallest value whose cumulative probability exceeds u, found for a whole batch of u
//! in one walk up their probabilities (starting a few standard deviations below the mean, and
//! from a standard deviation of 10^4 up taken from a Cornish–Fisher expansion, within 1 of the
//! exact value, so the cost does not grow with the mean).
//!
//! Unlike `Dist::sample`, a constant is its value as given, negative or not: a Monte Carlo input
//! is a quantity, not a duration.

use stochcraft_model::Dist;
use stochcraft_random::math::{exp, ln, powf};
use stochcraft_random::{estimate, fit, ln_binomial_pmf, ln_gamma, ln_poisson_pmf};
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
            let sd = m.sqrt();
            if sd > WALK_UP_TO_SD {
                for &i in &order {
                    out[i] = by_expansion(ps[i], m, sd, 1.0 / sd, f64::INFINITY);
                }
            } else {
                walk(
                    &order,
                    ps,
                    &mut out,
                    window_start(m, sd),
                    m + 50.0 * sd + 100.0,
                    |k| {
                        if m <= 0.0 {
                            f64::from(u8::from(k == 0.0))
                        } else {
                            exp(ln_poisson_pmf(k, m))
                        }
                    },
                );
            }
        }
        Dist::Binomial {
            probability,
            trials,
        } => {
            let (q, n) = (*probability, *trials as f64);
            let mean = n * q;
            let sd = (n * q * (1.0 - q)).sqrt();
            if q <= 0.0 || q >= 1.0 {
                // All the probability is at one end.
                let end = if q >= 1.0 { n } else { 0.0 };
                for &i in &order {
                    out[i] = end;
                }
            } else if sd > WALK_UP_TO_SD || mean + 50.0 * sd + 100.0 >= WHOLE_STEPS {
                for &i in &order {
                    out[i] = by_expansion(ps[i], mean, sd, (1.0 - 2.0 * q) / sd, n);
                }
            } else {
                walk(
                    &order,
                    ps,
                    &mut out,
                    window_start(mean, sd),
                    n.min(mean + 50.0 * sd + 100.0),
                    |k| exp(ln_binomial_pmf(k, n, q)),
                );
            }
        }
        _ => {
            for &i in &order {
                out[i] = continuous(d, ps[i]);
            }
        }
    }
    out
}

/// A count whose standard deviation is above this is not walked value by value (a walk costs
/// about 20 standard deviations of steps) but taken from [`by_expansion`].
const WALK_UP_TO_SD: f64 = 1e4;

/// 2^52: from here a `f64` can no longer step through the whole numbers one at a time.
const WHOLE_STEPS: f64 = 4_503_599_627_370_496.0;

/// Where a walk over a count with this mean and standard deviation starts: 12 standard
/// deviations and 40 below the mean, under which the probability left out (a Chernoff bound of
/// at most about 10^-17 for these counts) is below what a `f64` cumulative sum can hold. Small
/// counts start at 0.
fn window_start(mean: f64, sd: f64) -> f64 {
    (mean - 12.0 * sd - 40.0).floor().max(0.0)
}

/// The value at probability `p` of a count with a large standard deviation, from the
/// Cornish–Fisher expansion of the normal quantile to its first (skewness) term, with the
/// continuity correction of a count: the smallest whole number above x − ½ for the continuous
/// x = mean + sd (z + skew (z² − 1) / 6). The next terms are of order 1/sd² in z, so the result
/// is within 1 of the exact quantile once sd is in the thousands (tested against the walk at
/// 10^4), and the error as a share of the value falls as 1/sd².
fn by_expansion(p: f64, mean: f64, sd: f64, skew: f64, upper: f64) -> f64 {
    let z = z(p);
    let x = mean + sd * (z + skew * (z * z - 1.0) / 6.0);
    (x - 0.5).ceil().clamp(0.0, upper)
}

/// Walks a count's values from `start` (0 unless the probability below it is negligible) up to
/// `limit` with probabilities `pmf`, giving each probability (in the ascending `order`) the first
/// value whose cumulative probability exceeds it.
fn walk(
    order: &[usize],
    ps: &[f64],
    out: &mut [f64],
    start: f64,
    limit: f64,
    pmf: impl Fn(f64) -> f64,
) {
    let mut k = start;
    let mut cumulative = pmf(start);
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

    /// The least k with cumulative probability above `p`, by summing the probabilities from 0.
    fn quantile_by_summing(pmf: impl Fn(f64) -> f64, p: f64) -> f64 {
        let (mut k, mut cumulative) = (0.0, pmf(0.0));
        while cumulative <= p {
            k += 1.0;
            cumulative += pmf(k);
        }
        k
    }

    /// A walk that starts near the mean gives what a walk from 0 gives.
    #[test]
    fn counts_with_large_means_agree_with_summing_from_zero() {
        let ps = [1e-9, 0.001, 0.2, 0.5, 0.8, 0.999, 1.0 - 1e-9];
        let poisson = Dist::Poisson { mean: 5_000.0 };
        let got = quantiles(&poisson, &ps);
        for (p, k) in ps.iter().zip(&got) {
            let want = quantile_by_summing(|k| exp(ln_poisson_pmf(k, 5_000.0)), *p);
            assert!((k - want).abs() < 0.5, "Poisson at {p}: {k} vs {want}");
        }
        let binomial = Dist::Binomial {
            probability: 0.3,
            trials: 20_000,
        };
        let got = quantiles(&binomial, &ps);
        for (p, k) in ps.iter().zip(&got) {
            let want = quantile_by_summing(|k| exp(ln_binomial_pmf(k, 20_000.0, 0.3)), *p);
            assert!((k - want).abs() < 0.5, "Binomial at {p}: {k} vs {want}");
        }
    }

    /// The expansion used above 10^4 standard deviations is within 1 of the walk just below it.
    #[test]
    fn the_expansion_is_within_one_of_the_walk() {
        let ps = [1e-6, 0.001, 0.1, 0.5, 0.9, 0.999, 1.0 - 1e-6];
        let cases = [
            (
                Dist::Poisson { mean: 9e7 },
                9e7,
                9e7_f64.sqrt(),
                1.0 / 9e7_f64.sqrt(),
                f64::INFINITY,
            ),
            (
                Dist::Binomial {
                    probability: 0.3,
                    trials: 300_000_000,
                },
                9e7,
                (300_000_000.0_f64 * 0.21).sqrt(),
                0.4 / (300_000_000.0_f64 * 0.21).sqrt(),
                300_000_000.0,
            ),
            (
                Dist::Binomial {
                    probability: 0.005,
                    trials: 200_000_000,
                },
                1e6,
                (200_000_000.0_f64 * 0.005 * 0.995).sqrt(),
                0.99 / (200_000_000.0_f64 * 0.005 * 0.995).sqrt(),
                200_000_000.0,
            ),
            (
                Dist::Binomial {
                    probability: 0.97,
                    trials: 50_000_000,
                },
                48_500_000.0,
                (50_000_000.0_f64 * 0.97 * 0.03).sqrt(),
                -0.94 / (50_000_000.0_f64 * 0.97 * 0.03).sqrt(),
                50_000_000.0,
            ),
        ];
        for (d, mean, sd, skew, upper) in cases {
            let walked = quantiles(&d, &ps);
            for (p, k) in ps.iter().zip(&walked) {
                let approx = by_expansion(*p, mean, sd, skew, upper);
                assert!(
                    (k - approx).abs() <= 1.0,
                    "{d:?} at {p}: walked {k}, expansion {approx}"
                );
            }
        }
    }

    /// Counts of any size are inverted at once, not by walking up to them.
    #[test]
    fn huge_counts_are_inverted_without_a_walk() {
        let ps = [0.025, 0.5, 0.975];
        for mean in [1e9, 1e12, 1e18] {
            let sd = mean.sqrt();
            let got = quantiles(&Dist::Poisson { mean }, &ps);
            assert!(got.windows(2).all(|w| w[0] < w[1]), "{mean}: {got:?}");
            assert!(
                (got[1] - mean).abs() <= 1.0 + 0.0001 * sd,
                "{mean}: {got:?}"
            );
            assert!(
                (got[2] - (mean + 1.959_964 * sd)).abs() <= 0.001 * sd + 2.0,
                "{got:?}"
            );
        }
        let trials = u64::MAX;
        let n = trials as f64;
        let got = quantiles(
            &Dist::Binomial {
                probability: 0.3,
                trials,
            },
            &ps,
        );
        let sd = (n * 0.21).sqrt();
        assert!(got.windows(2).all(|w| w[0] < w[1]), "{got:?}");
        assert!(
            (got[1] - 0.3 * n).abs() <= 0.001 * sd + 2.0 * 4096.0,
            "{got:?}"
        );
        assert!(got.iter().all(|k| (0.0..=n).contains(k)), "{got:?}");
        // Ends of the range, and a success probability that leaves almost all at the top.
        let near_one = quantiles(
            &Dist::Binomial {
                probability: 1.0 - f64::EPSILON,
                trials,
            },
            &ps,
        );
        assert!(
            near_one.iter().all(|k| (0.0..=n).contains(k)),
            "{near_one:?}"
        );
        let all = quantiles(
            &Dist::Binomial {
                probability: 1.0,
                trials,
            },
            &ps,
        );
        assert_eq!(all, vec![n; 3]);
        let none = quantiles(
            &Dist::Binomial {
                probability: 0.0,
                trials,
            },
            &ps,
        );
        assert_eq!(none, vec![0.0; 3]);
    }
}
