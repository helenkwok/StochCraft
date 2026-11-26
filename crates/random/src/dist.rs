//! The distributions of BPSim 2.0 (section 6.3.5 of the specification), plus a constant.
//!
//! Every sampler is exact and never panics: parameters that [`Dist::check`] rejects give a safe
//! value (the lower bound, or 0) instead. Where the specification leaves a parameter's meaning
//! open, the choice made here is stated on the variant.

use crate::Rng;
use crate::math::{cos, erf, erfc, exp, ln, powf, sin};

/// A probability distribution, as a model describes a duration, a count or a property.
#[derive(Debug, Clone, PartialEq)]
pub enum Dist {
    /// Always the same value. Negative or non-finite values sample as 0.
    Constant(f64),
    /// Exponential with this mean (BPSim `NegativeExponentialDistribution`). Inversion.
    Exponential { mean: f64 },
    /// Uniform on [min, max).
    Uniform { min: f64, max: f64 },
    /// Triangular on [min, max] peaking at `mode`. Inversion.
    Triangular { min: f64, mode: f64, max: f64 },
    /// Normal. Box–Muller (Box and Muller, 1958). Can be negative; a duration drawn from it is
    /// treated as 0 by whoever waits for it, so when negative draws are likely the realised
    /// mean duration is greater than `mean` (use a truncated normal to say what is meant). A
    /// model's validation warns about a time drawn from one that is below 0 more than 1% of the
    /// time (`normal_often_negative`).
    Normal { mean: f64, std_dev: f64 },
    /// Normal restricted to [min, max]. Exact rejection after Robert (1995), choosing a normal,
    /// uniform or exponential proposal by where the interval lies.
    TruncatedNormal {
        mean: f64,
        std_dev: f64,
        min: f64,
        max: f64,
    },
    /// Log-normal whose own mean and standard deviation are given (not those of its logarithm,
    /// which BPSim leaves unstated; this is the reading practitioners expect).
    LogNormal { mean: f64, std_dev: f64 },
    /// Gamma with shape k and scale θ (mean kθ). Marsaglia and Tsang (2000), with the
    /// shape-below-one boost.
    Gamma { shape: f64, scale: f64 },
    /// Beta on [`min`, `max`] (BPSim's on [0, 1]: `min` 0 and `max` 1, see [`Dist::beta`]).
    /// BPSim names its parameters `shape` and `scale`; they are taken as the two shape
    /// parameters α = `shape`, β = `scale`, so the mean is min + (max − min) α / (α + β). A
    /// draw on [0, 1] is a ratio of two gammas, and for a shape below 0.1 the same ratio formed
    /// from their logs; it is then stretched to the range (min + (max − min) x, which leaves a
    /// draw on [0, 1] exactly as it was).
    Beta {
        shape: f64,
        scale: f64,
        min: f64,
        max: f64,
    },
    /// Erlang: the sum of `k` exponentials, with overall mean `mean`.
    Erlang { mean: f64, k: u64 },
    /// Weibull with shape k and scale λ. Inversion.
    Weibull { shape: f64, scale: f64 },
    /// Poisson count with this mean. Below a mean of 10, Knuth's multiplication method. From 10,
    /// the transformed rejection method PTRS (Hörmann, "The transformed rejection method for
    /// generating Poisson random variables", Insurance: Mathematics and Economics 12, 1993,
    /// 39-45), whose cost does not grow with the mean. From a mean of 10^15, where adding 1 to
    /// a count in a `f64` starts to fail, a rounded normal draw (the relative error of its
    /// shape is of order 1/sqrt(mean)).
    Poisson { mean: f64 },
    /// Binomial count of successes in `trials` with success `probability`. Geometric waiting
    /// times while trials × min(probability, 1 − probability) is below 10 (cost grows with that
    /// product), using symmetry above probability 0.5. From 10, the transformed rejection
    /// method BTRS (Hörmann, "The generation of binomial random variates", Journal of
    /// Statistical Computation and Simulation 46, 1993, 101-110), whose cost does not grow with
    /// the number of trials.
    Binomial { probability: f64, trials: u64 },
    /// Data points `(probability, value)`. `discrete`: one of the values, with its probability.
    /// Continuous: the points, sorted by value, are breakpoints of a cumulative distribution
    /// interpolated linearly, so the first point's probability is an atom at the lowest value
    /// (0 for a curve that starts there). Probabilities are normalised to sum to 1.
    User {
        points: Vec<(f64, f64)>,
        discrete: bool,
    },
}

/// The range a distribution's parameter must lie in ([`DistError::OutOfRange`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// Greater than 0.
    AboveZero,
    /// 0 or more.
    ZeroOrMore,
    /// 1 or more.
    OneOrMore,
    /// Between 0 and 1.
    UnitInterval,
}

impl std::fmt::Display for Requirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Requirement::AboveZero => "greater than 0",
            Requirement::ZeroOrMore => "0 or more",
            Requirement::OneOrMore => "1 or more",
            Requirement::UnitInterval => "between 0 and 1",
        })
    }
}

/// Why a distribution's parameters cannot be sampled.
#[derive(Debug, Clone, PartialEq)]
pub enum DistError {
    /// A parameter is NaN or infinite.
    NotFinite(&'static str),
    /// A parameter is outside its range.
    OutOfRange {
        parameter: &'static str,
        requirement: Requirement,
    },
    /// `min` is not below `max` (or the mode lies outside them).
    BadBounds,
    /// A user distribution has no points or its probabilities do not sum to a positive number.
    EmptyUserDistribution,
}

impl std::fmt::Display for DistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DistError::NotFinite(p) => write!(f, "{p} is not a finite number"),
            DistError::OutOfRange {
                parameter,
                requirement,
            } => write!(f, "{parameter} must be {requirement}"),
            DistError::BadBounds => write!(f, "min must be below max, with any mode between them"),
            DistError::EmptyUserDistribution => {
                write!(
                    f,
                    "a user distribution needs points whose probabilities sum to more than 0"
                )
            }
        }
    }
}

impl std::error::Error for DistError {}

fn finite(name: &'static str, v: f64) -> Result<(), DistError> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(DistError::NotFinite(name))
    }
}

fn positive(name: &'static str, v: f64) -> Result<(), DistError> {
    finite(name, v)?;
    if v > 0.0 {
        Ok(())
    } else {
        Err(DistError::OutOfRange {
            parameter: name,
            requirement: Requirement::AboveZero,
        })
    }
}

fn non_negative(name: &'static str, v: f64) -> Result<(), DistError> {
    finite(name, v)?;
    if v >= 0.0 {
        Ok(())
    } else {
        Err(DistError::OutOfRange {
            parameter: name,
            requirement: Requirement::ZeroOrMore,
        })
    }
}

/// ln Γ(x) for x > 0 (Lanczos approximation, g = 7, about 15 significant digits), with the
/// reflection formula below 0.5.
pub fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        let pi = std::f64::consts::PI;
        return ln(pi / sin(pi * x)) - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * ln(2.0 * std::f64::consts::PI) + (x + 0.5) * ln(t) - t + ln(a)
}

fn standard_normal(rng: &mut Rng) -> f64 {
    let u1 = rng.uniform_open0();
    let u2 = rng.uniform();
    (-2.0 * ln(u1)).sqrt() * cos(2.0 * std::f64::consts::PI * u2)
}

/// The largest Erlang `k` drawn as a sum of exponentials; above it, by [`gamma_sample`].
const ERLANG_BY_SUM: u64 = 64;

fn gamma_sample(rng: &mut Rng, shape: f64, scale: f64) -> f64 {
    if shape < 1.0 {
        // Γ(k) = Γ(k + 1) · U^(1/k).
        let u = rng.uniform_open0();
        return gamma_sample(rng, shape + 1.0, scale) * powf(u, 1.0 / shape);
    }
    let d = shape - 1.0 / 3.0;
    let c = 1.0 / (9.0 * d).sqrt();
    loop {
        let x = standard_normal(rng);
        let v = 1.0 + c * x;
        if v <= 0.0 {
            continue;
        }
        let v = v * v * v;
        let u = rng.uniform_open0();
        if ln(u) < 0.5 * x * x + d - d * v + d * ln(v) {
            return d * v * scale;
        }
    }
}

/// The natural log of a Gamma(`shape`, 1) draw, for a shape so small that the draw itself
/// underflows to 0: below 1, ln Γ(k) = ln Γ(k + 1) + ln(U) / k, kept as a log (the draws and
/// their order are those of [`gamma_sample`]).
fn ln_gamma_sample(rng: &mut Rng, shape: f64) -> f64 {
    if shape < 1.0 {
        let u = rng.uniform_open0();
        return ln_gamma_sample(rng, shape + 1.0) + ln(u) / shape;
    }
    ln(gamma_sample(rng, shape, 1.0))
}

/// Both shapes of a beta at or above this are drawn as X / (X + Y) from two gammas. Below it, a
/// gamma draw can lose its digits or underflow to 0 (which made the beta 0 too often and its
/// mean wrong), so it is drawn from the logs of the gammas instead.
const BETA_DIRECT_FROM: f64 = 0.1;

fn beta_sample(rng: &mut Rng, a: f64, b: f64) -> f64 {
    if a >= BETA_DIRECT_FROM && b >= BETA_DIRECT_FROM {
        let x = gamma_sample(rng, a, 1.0);
        let y = gamma_sample(rng, b, 1.0);
        return if x + y > 0.0 { x / (x + y) } else { 0.0 };
    }
    // X / (X + Y) = 1 / (1 + Y / X), with Y / X = exp(ln Y − ln X) formed without either.
    let lx = ln_gamma_sample(rng, a);
    let ly = ln_gamma_sample(rng, b);
    let d = ly - lx;
    if d.is_nan() {
        // Both logs overflowed the same way (shapes below 1e-300): a tie.
        return 0.5;
    }
    1.0 / (1.0 + exp(d))
}

/// A standard normal restricted to [a, b], a < b (Robert, 1995).
fn truncated_standard_normal(rng: &mut Rng, a: f64, b: f64) -> f64 {
    if b <= 0.0 {
        return -truncated_standard_normal(rng, -b, -a);
    }
    if a <= 0.0 {
        // The interval contains 0.
        if b - a >= 1.0 {
            loop {
                let z = standard_normal(rng);
                if z >= a && z <= b {
                    return z;
                }
            }
        }
        loop {
            let z = rng.uniform_range(a, b);
            if rng.uniform() <= exp(-0.5 * z * z) {
                return z;
            }
        }
    }
    // 0 < a < b: a right-hand tail.
    let w = b - a;
    if a * w + 0.5 * w * w <= 1.5 {
        loop {
            let z = rng.uniform_range(a, b);
            if rng.uniform() <= exp(0.5 * (a - z) * (a + z)) {
                return z;
            }
        }
    }
    let alpha = 0.5 * (a + (a * a + 4.0).sqrt());
    loop {
        let z = a + rng.exponential(1.0 / alpha);
        if z <= b && rng.uniform() <= exp(-0.5 * (z - alpha) * (z - alpha)) {
            return z;
        }
    }
}

fn poisson_knuth(rng: &mut Rng, mean: f64) -> u64 {
    let limit = exp(-mean);
    let mut k = 0u64;
    let mut p = rng.uniform_open0();
    while p > limit {
        k += 1;
        p *= rng.uniform_open0();
    }
    k
}

fn binomial(rng: &mut Rng, p: f64, n: u64) -> u64 {
    if p <= 0.0 || n == 0 {
        return 0;
    }
    if p >= 1.0 {
        return n;
    }
    if p > 0.5 {
        return n - binomial(rng, 1.0 - p, n);
    }
    if n as f64 * p >= TRANSFORMED_FROM {
        return binomial_btrs(rng, p, n);
    }
    // Successes are where geometric waiting times land inside the n trials. (Below a success
    // probability of 2^-52, 1 − p rounds to 1, so ln(1 − p) is taken as ln_1p(−p).)
    let log_q = if p < f64::EPSILON {
        libm::log1p(-p)
    } else {
        ln(1.0 - p)
    };
    let mut successes = 0u64;
    // The trials used so far, so never above n.
    let mut position = 0u64;
    loop {
        let skip = (ln(rng.uniform_open0()) / log_q).floor();
        let step = if skip.is_finite() && skip < (n as f64) {
            skip as u64
        } else {
            n
        };
        // A success needs `step` failures and then itself; compared with what is left rather
        // than added to the position, which would never pass n = u64::MAX.
        if step >= n - position {
            return successes;
        }
        position += step + 1;
        successes += 1;
    }
}

/// Binomial by transformed rejection with squeeze, BTRS (Hörmann, "The generation of binomial
/// random variates", Journal of Statistical Computation and Simulation 46, 1993, 101-110), for
/// p ≤ 0.5 and n · p ≥ 10. The acceptance test uses [`ln_binomial_pmf`], which stays accurate
/// at any `n`.
fn binomial_btrs(rng: &mut Rng, p: f64, n: u64) -> u64 {
    let nf = n as f64;
    let spq = (nf * p * (1.0 - p)).sqrt();
    let b = 1.15 + 2.53 * spq;
    let a = -0.0873 + 0.0248 * b + 0.01 * p;
    let c = nf * p + 0.5;
    let vr = 0.92 - 4.2 / b;
    let alpha = (2.83 + 5.1 / b) * spq;
    // The mode.
    let m = ((nf + 1.0) * p).floor();
    let ln_mode = ln_binomial_pmf(m, nf, p);
    loop {
        let u = rng.uniform() - 0.5;
        let v = rng.uniform_open0();
        let us = 0.5 - u.abs();
        let k = ((2.0 * a / us + b) * u + c).floor();
        // Outside 0..=n (or NaN, from us = 0) is rejected.
        if !(0.0..=nf).contains(&k) {
            continue;
        }
        if us >= 0.07 && v <= vr {
            return (k as u64).min(n);
        }
        if ln(v * alpha / (a / (us * us) + b)) <= ln_binomial_pmf(k, nf, p) - ln_mode {
            return (k as u64).min(n);
        }
    }
}

/// The mean from which a Poisson count is drawn by [`poisson_ptrs`], and the trials × probability
/// (of the rarer side) from which a binomial one is drawn by [`binomial_btrs`]: the least values
/// at which the published methods are valid. Below them the older samplers run unchanged, so
/// their fixed-seed draws stay as they were.
const TRANSFORMED_FROM: f64 = 10.0;

/// The Poisson mean from which a rounded normal is drawn: far above 2^53, where adding 1 to a
/// count stalls.
const POISSON_NORMAL_FROM: f64 = 1e15;

/// A Poisson count with a valid `mean`.
fn poisson(rng: &mut Rng, mean: f64) -> f64 {
    if mean < TRANSFORMED_FROM {
        return poisson_knuth(rng, mean) as f64;
    }
    if mean >= POISSON_NORMAL_FROM {
        return libm::round(mean + mean.sqrt() * standard_normal(rng)).max(0.0);
    }
    poisson_ptrs(rng, mean)
}

/// Poisson by transformed rejection with squeeze, PTRS (Hörmann, 1993, Insurance: Mathematics
/// and Economics 12, 39-45), for a mean of at least 10. The acceptance test uses
/// [`ln_poisson_pmf`], which stays accurate at huge means.
fn poisson_ptrs(rng: &mut Rng, mean: f64) -> f64 {
    let b = 0.931 + 2.53 * mean.sqrt();
    let a = -0.059 + 0.02483 * b;
    let ln_inv_alpha = ln(1.1239 + 1.1328 / (b - 3.4));
    let vr = 0.9277 - 3.6224 / (b - 2.0);
    loop {
        let u = rng.uniform() - 0.5;
        let v = rng.uniform_open0();
        let us = 0.5 - u.abs();
        let k = ((2.0 * a / us + b) * u + mean + 0.43).floor();
        // Negative (or NaN, from us = 0) is rejected.
        if k.is_nan() || k < 0.0 {
            continue;
        }
        if us >= 0.07 && v <= vr {
            return k;
        }
        if us < 0.013 && v > us {
            continue;
        }
        if ln(v) + ln_inv_alpha - ln(a / (us * us) + b) <= ln_poisson_pmf(k, mean) {
            return k;
        }
    }
}

/// Below this, ln k! is taken from [`ln_gamma`]; from it, from Stirling's series.
const STIRLING_FROM: f64 = 32.0;

/// The natural logarithm of the Poisson probability of `k` (a whole number) at a `mean` above 0.
///
/// From k = 32 by Stirling's series for ln k!, arranged so that the large terms cancel
/// analytically (the main term is −mean · ((1 + x) ln(1 + x) − x) for x = (k − mean) / mean).
/// That keeps it accurate to many digits at means where the direct form loses all of them.
pub fn ln_poisson_pmf(k: f64, mean: f64) -> f64 {
    if k < STIRLING_FROM {
        return -mean + k * ln(mean) - ln_gamma(k + 1.0);
    }
    let x = (k - mean) / mean;
    -mean * entropy_excess(x) - 0.5 * ln(2.0 * std::f64::consts::PI * k) - stirling_tail(k)
}

/// The natural logarithm of the binomial probability of `k` successes (a whole number from 0 to
/// `n`) in `n` trials with success probability `p`, for p in (0, 1). Accurate to many digits for
/// any `n`, by the same arrangement as [`ln_poisson_pmf`].
pub fn ln_binomial_pmf(k: f64, n: f64, p: f64) -> f64 {
    let failures = n - k;
    if k < STIRLING_FROM || failures < STIRLING_FROM {
        // ln C(n, j) for the smaller of the two, as a sum of the ratios (n − i) / (i + 1).
        let j = k.min(failures);
        let mut ln_choose = 0.0;
        let mut i = 0.0;
        while i < j {
            ln_choose += ln((n - i) / (i + 1.0));
            i += 1.0;
        }
        return ln_choose + k * ln(p) + failures * libm::log1p(-p);
    }
    let (np, nq) = (n * p, n * (1.0 - p));
    let x = (k - np) / np;
    let y = (np - k) / nq;
    -(np * entropy_excess(x) + nq * entropy_excess(y))
        + 0.5 * ln(n / (2.0 * std::f64::consts::PI * k * failures))
        + stirling_tail(n)
        - stirling_tail(k)
        - stirling_tail(failures)
}

/// (1 + x) ln(1 + x) − x for x > −1: about x²/2 near 0, where the difference of the two terms
/// would lose its digits, so a series stands in there.
fn entropy_excess(x: f64) -> f64 {
    if x.abs() < 0.01 {
        // Σ (−1)^n xⁿ / (n (n − 1)) for n ≥ 2.
        let c = [
            1.0 / 2.0,
            -1.0 / 6.0,
            1.0 / 12.0,
            -1.0 / 20.0,
            1.0 / 30.0,
            -1.0 / 42.0,
            1.0 / 56.0,
        ];
        return x * x * c.iter().rev().fold(0.0, |acc, ci| ci + x * acc);
    }
    (1.0 + x) * libm::log1p(x) - x
}

/// ln k! − (k ln k − k + ½ ln(2πk)), by its asymptotic series, for k ≥ 32 (the next term is
/// below 10^-17 there).
fn stirling_tail(k: f64) -> f64 {
    let i = 1.0 / k;
    let i2 = i * i;
    i * (1.0 / 12.0 - i2 * (1.0 / 360.0 - i2 * (1.0 / 1260.0 - i2 / 1680.0)))
}

/// A draw from a user distribution (0 for points [`Dist::check`] rejects). Discrete points, and
/// continuous ones already in order of value, are read where they are, without the copy
/// [`user_points`] makes; the draw is the same either way.
fn sample_user(rng: &mut Rng, points: &[(f64, f64)], discrete: bool) -> f64 {
    // What `check` accepts, with the total summed as it sums it.
    let total: f64 = points.iter().map(|(p, _)| *p).sum();
    let valid = points
        .iter()
        .all(|(p, v)| p.is_finite() && *p >= 0.0 && v.is_finite())
        && total > 0.0
        && total.is_finite();
    if !valid {
        return 0.0;
    }
    let in_order = discrete
        || points
            .windows(2)
            .all(|w| w[0].1.total_cmp(&w[1].1) != std::cmp::Ordering::Greater);
    let u = rng.uniform();
    if in_order {
        draw_user(u, points.iter().map(|(p, v)| (p / total, *v)), discrete)
    } else {
        let pts = user_points(points, discrete).unwrap_or_default();
        draw_user(u, pts.into_iter(), discrete)
    }
}

/// The value at `u` of the normalised points, in order of value for the continuous form.
fn draw_user(u: f64, pts: impl Iterator<Item = (f64, f64)>, discrete: bool) -> f64 {
    let mut cumulative = 0.0;
    let mut previous: Option<(f64, f64)> = None;
    for (p, v) in pts {
        let below = cumulative;
        cumulative += p;
        if u < cumulative {
            return match (discrete, previous) {
                (false, Some((_, v0))) if p > 0.0 => v0 + (v - v0) * (u - below) / p,
                _ => v,
            };
        }
        previous = Some((p, v));
    }
    previous.map_or(0.0, |(_, v)| v)
}

/// The normalised points of a user distribution, sorted by value for the continuous form.
fn user_points(points: &[(f64, f64)], discrete: bool) -> Option<Vec<(f64, f64)>> {
    let total: f64 = points.iter().map(|(p, _)| *p).sum();
    if points.is_empty() || !(total.is_finite() && total > 0.0) {
        return None;
    }
    let mut pts: Vec<(f64, f64)> = points.iter().map(|(p, v)| (p / total, *v)).collect();
    if !discrete {
        pts.sort_by(|a, b| a.1.total_cmp(&b.1));
    }
    Some(pts)
}

impl Dist {
    /// Exponential with the given rate (events per time unit). A non-positive rate gives mean 0.
    pub fn exponential_rate(rate: f64) -> Self {
        let mean = if rate.is_finite() && rate > 0.0 {
            1.0 / rate
        } else {
            0.0
        };
        Dist::Exponential { mean }
    }

    /// The beta on [0, 1] with shape parameters α = `shape`, β = `scale` (BPSim's).
    pub fn beta(shape: f64, scale: f64) -> Self {
        Dist::Beta {
            shape,
            scale,
            min: 0.0,
            max: 1.0,
        }
    }

    /// The distribution of `factor` times a draw from this one (a time measured on another
    /// scale, such as a service taking 10% longer), or `None` for a count (Poisson, binomial),
    /// which cannot be scaled, or a factor that is not a finite number above 0.
    pub fn scaled(&self, factor: f64) -> Option<Dist> {
        if !(factor.is_finite() && factor > 0.0) {
            return None;
        }
        let f = factor;
        Some(match self {
            Dist::Constant(v) => Dist::Constant(v * f),
            Dist::Exponential { mean } => Dist::Exponential { mean: mean * f },
            Dist::Uniform { min, max } => Dist::Uniform {
                min: min * f,
                max: max * f,
            },
            Dist::Triangular { min, mode, max } => Dist::Triangular {
                min: min * f,
                mode: mode * f,
                max: max * f,
            },
            Dist::Normal { mean, std_dev } => Dist::Normal {
                mean: mean * f,
                std_dev: std_dev * f,
            },
            Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            } => Dist::TruncatedNormal {
                mean: mean * f,
                std_dev: std_dev * f,
                min: min * f,
                max: max * f,
            },
            Dist::LogNormal { mean, std_dev } => Dist::LogNormal {
                mean: mean * f,
                std_dev: std_dev * f,
            },
            Dist::Gamma { shape, scale } => Dist::Gamma {
                shape: *shape,
                scale: scale * f,
            },
            Dist::Erlang { mean, k } => Dist::Erlang {
                mean: mean * f,
                k: *k,
            },
            Dist::Weibull { shape, scale } => Dist::Weibull {
                shape: *shape,
                scale: scale * f,
            },
            Dist::User { points, discrete } => Dist::User {
                points: points.iter().map(|(p, v)| (*p, v * f)).collect(),
                discrete: *discrete,
            },
            // A beta keeps its shape; its range stretches.
            Dist::Beta {
                shape,
                scale,
                min,
                max,
            } => Dist::Beta {
                shape: *shape,
                scale: *scale,
                min: min * f,
                max: max * f,
            },
            Dist::Poisson { .. } | Dist::Binomial { .. } => return None,
        })
    }

    /// Whether the parameters can be sampled.
    pub fn check(&self) -> Result<(), DistError> {
        match self {
            Dist::Constant(v) => finite("value", *v),
            Dist::Exponential { mean } => non_negative("mean", *mean),
            Dist::Uniform { min, max } => {
                finite("min", *min)?;
                finite("max", *max)?;
                if min < max {
                    Ok(())
                } else {
                    Err(DistError::BadBounds)
                }
            }
            Dist::Triangular { min, mode, max } => {
                finite("min", *min)?;
                finite("mode", *mode)?;
                finite("max", *max)?;
                if min < max && min <= mode && mode <= max {
                    Ok(())
                } else {
                    Err(DistError::BadBounds)
                }
            }
            Dist::Normal { mean, std_dev } | Dist::LogNormal { mean, std_dev } => {
                finite("mean", *mean)?;
                non_negative("standardDeviation", *std_dev)?;
                if matches!(self, Dist::LogNormal { .. }) {
                    positive("mean", *mean)
                } else {
                    Ok(())
                }
            }
            Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            } => {
                finite("mean", *mean)?;
                positive("standardDeviation", *std_dev)?;
                finite("min", *min)?;
                finite("max", *max)?;
                if min < max {
                    Ok(())
                } else {
                    Err(DistError::BadBounds)
                }
            }
            Dist::Gamma { shape, scale } | Dist::Weibull { shape, scale } => {
                positive("shape", *shape)?;
                positive("scale", *scale)
            }
            Dist::Beta {
                shape,
                scale,
                min,
                max,
            } => {
                positive("shape", *shape)?;
                positive("scale", *scale)?;
                finite("min", *min)?;
                finite("max", *max)?;
                if min < max {
                    Ok(())
                } else {
                    Err(DistError::BadBounds)
                }
            }
            Dist::Erlang { mean, k } => {
                non_negative("mean", *mean)?;
                if *k >= 1 {
                    Ok(())
                } else {
                    Err(DistError::OutOfRange {
                        parameter: "k",
                        requirement: Requirement::OneOrMore,
                    })
                }
            }
            Dist::Poisson { mean } => non_negative("mean", *mean),
            Dist::Binomial { probability, .. } => {
                finite("probability", *probability)?;
                if (0.0..=1.0).contains(probability) {
                    Ok(())
                } else {
                    Err(DistError::OutOfRange {
                        parameter: "probability",
                        requirement: Requirement::UnitInterval,
                    })
                }
            }
            Dist::User { points, .. } => {
                for (p, v) in points {
                    non_negative("probability", *p)?;
                    finite("value", *v)?;
                }
                let total: f64 = points.iter().map(|(p, _)| *p).sum();
                // Finite too: probabilities that overflow cannot be shared out.
                if total > 0.0 && total.is_finite() {
                    Ok(())
                } else {
                    Err(DistError::EmptyUserDistribution)
                }
            }
        }
    }

    /// Draws one value. Parameters [`Dist::check`] rejects give the lower bound or 0, never a panic.
    pub fn sample(&self, rng: &mut Rng) -> f64 {
        if let Dist::User { points, discrete } = self {
            return sample_user(rng, points, *discrete);
        }
        if self.check().is_err() {
            return self.fallback();
        }
        match self {
            Dist::Constant(v) => v.max(0.0),
            Dist::Exponential { mean } => rng.exponential(*mean),
            Dist::Uniform { min, max } => rng.uniform_range(*min, *max),
            Dist::Triangular { min, mode, max } => rng.triangular(*min, *mode, *max),
            Dist::Normal { mean, std_dev } => mean + std_dev * standard_normal(rng),
            Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            } => {
                let (a, b) = ((min - mean) / std_dev, (max - mean) / std_dev);
                (mean + std_dev * truncated_standard_normal(rng, a, b)).clamp(*min, *max)
            }
            Dist::LogNormal { mean, std_dev } => {
                let cv = std_dev / mean;
                let s2 = ln(1.0 + cv * cv);
                let mu = ln(*mean) - 0.5 * s2;
                exp(mu + s2.sqrt() * standard_normal(rng))
            }
            Dist::Gamma { shape, scale } => gamma_sample(rng, *shape, *scale),
            // On [0, 1], 0 + 1 · x is x exactly, so BPSim's beta draws as it always has.
            Dist::Beta {
                shape,
                scale,
                min,
                max,
            } => min + (max - min) * beta_sample(rng, *shape, *scale),
            // The sum of k exponentials costs k draws, so a large k (a fit to tightly clustered
            // data can give millions) is drawn as the gamma it is. Small k keep the sum, so
            // their fixed-seed draws stay as they were.
            Dist::Erlang { mean, k } if *k > ERLANG_BY_SUM => {
                gamma_sample(rng, *k as f64, mean / *k as f64)
            }
            Dist::Erlang { mean, k } => {
                let each = mean / *k as f64;
                (0..*k).map(|_| rng.exponential(each)).sum()
            }
            Dist::Weibull { shape, scale } => scale * powf(-ln(rng.uniform_open0()), 1.0 / shape),
            Dist::Poisson { mean } => poisson(rng, *mean),
            Dist::Binomial {
                probability,
                trials,
            } => binomial(rng, *probability, *trials) as f64,
            Dist::User { points, discrete } => sample_user(rng, points, *discrete),
        }
    }

    fn fallback(&self) -> f64 {
        match self {
            Dist::Uniform { min, .. }
            | Dist::Triangular { min, .. }
            | Dist::TruncatedNormal { min, .. }
            | Dist::Beta { min, .. }
                if min.is_finite() =>
            {
                *min
            }
            _ => 0.0,
        }
    }

    /// Whether a draw can be greater than 0. Durations clamp negative draws to 0, so a delay
    /// whose distribution cannot give a positive value always takes zero time (`false` too for
    /// parameters [`Dist::check`] rejects).
    pub fn can_be_positive(&self) -> bool {
        if self.check().is_err() {
            return false;
        }
        match self {
            Dist::Constant(v) => *v > 0.0,
            Dist::Exponential { mean } | Dist::Poisson { mean } | Dist::Erlang { mean, .. } => {
                *mean > 0.0
            }
            Dist::Uniform { max, .. }
            | Dist::Triangular { max, .. }
            | Dist::TruncatedNormal { max, .. }
            | Dist::Beta { max, .. } => *max > 0.0,
            Dist::Normal { mean, std_dev } => *mean > 0.0 || *std_dev > 0.0,
            Dist::LogNormal { .. } | Dist::Gamma { .. } | Dist::Weibull { .. } => true,
            Dist::Binomial {
                probability,
                trials,
            } => *probability > 0.0 && *trials > 0,
            // Discrete: a positive value with weight. Continuous: point i's probability spreads
            // over the values up to v_i (the first is an atom at v_0), so the same test holds.
            Dist::User { points, .. } => points.iter().any(|(p, v)| *p > 0.0 && *v > 0.0),
        }
    }

    /// The theoretical mean (`None` where it has no simple closed form or the parameters are
    /// invalid). A constant is clamped at 0, but a normal is not: its mean is its `mean`, whatever
    /// the chance of a negative draw, so it is not the mean of a duration drawn from it (negative
    /// draws wait 0, see [`Dist::Normal`]). Code that needs the mean of a time must not use it for
    /// a normal; the capacity table refuses one, and a truncated normal says what is meant.
    pub fn mean(&self) -> Option<f64> {
        self.check().ok()?;
        Some(match self {
            Dist::Constant(v) => v.max(0.0),
            Dist::Exponential { mean } | Dist::Poisson { mean } | Dist::Erlang { mean, .. } => {
                *mean
            }
            Dist::Uniform { min, max } => (min + max) / 2.0,
            Dist::Triangular { min, mode, max } => (min + mode + max) / 3.0,
            Dist::Normal { mean, .. } | Dist::LogNormal { mean, .. } => *mean,
            Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            } => truncated_moments(*mean, *std_dev, *min, *max).0,
            Dist::Gamma { shape, scale } => shape * scale,
            // Written so that two shapes near the largest number do not overflow their sum.
            Dist::Beta {
                shape,
                scale,
                min,
                max,
            } => min + (max - min) * (1.0 / (1.0 + scale / shape)),
            Dist::Weibull { shape, scale } => scale * exp(ln_gamma(1.0 + 1.0 / shape)),
            Dist::Binomial {
                probability,
                trials,
            } => probability * *trials as f64,
            Dist::User { points, discrete } => {
                let pts = user_points(points, *discrete)?;
                if *discrete {
                    pts.iter().map(|(p, v)| p * v).sum()
                } else {
                    let mut m = pts.first().map_or(0.0, |(p, v)| p * v);
                    for w in pts.windows(2) {
                        m += w[1].0 * 0.5 * (w[0].1 + w[1].1);
                    }
                    m
                }
            }
        })
    }

    /// The theoretical variance, where it has a closed form (`None` otherwise or if invalid).
    pub fn variance(&self) -> Option<f64> {
        self.check().ok()?;
        Some(match self {
            Dist::Constant(_) => 0.0,
            Dist::Exponential { mean } => mean * mean,
            Dist::Uniform { min, max } => (max - min) * (max - min) / 12.0,
            Dist::Triangular {
                min: a,
                mode: c,
                max: b,
            } => (a * a + b * b + c * c - a * b - a * c - b * c) / 18.0,
            Dist::Normal { std_dev, .. } | Dist::LogNormal { std_dev, .. } => std_dev * std_dev,
            Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            } => truncated_moments(*mean, *std_dev, *min, *max).1,
            Dist::Gamma { shape, scale } => shape * scale * scale,
            Dist::Beta {
                shape: a,
                scale: b,
                min,
                max,
            } => {
                let w = max - min;
                w * w * (a * b / ((a + b) * (a + b) * (a + b + 1.0)))
            }
            Dist::Erlang { mean, k } => mean * mean / *k as f64,
            Dist::Weibull { shape, scale } => {
                let g1 = exp(ln_gamma(1.0 + 1.0 / shape));
                let g2 = exp(ln_gamma(1.0 + 2.0 / shape));
                scale * scale * (g2 - g1 * g1)
            }
            Dist::Poisson { mean } => *mean,
            Dist::Binomial {
                probability: p,
                trials,
            } => *trials as f64 * p * (1.0 - p),
            // Exact: the second moment about the mean. A continuous table is an atom at the
            // lowest value and then uniform pieces between breakpoints, each with the second
            // moment (a² + ab + b²) / 3 of a uniform from a to b (taken about the mean).
            Dist::User { points, discrete } => {
                let pts = user_points(points, *discrete)?;
                let mean = self.mean()?;
                let about = |v: f64| v - mean;
                if *discrete {
                    pts.iter().map(|(p, v)| p * about(*v) * about(*v)).sum()
                } else {
                    let atom = pts.first().map_or(0.0, |(p, v)| p * about(*v) * about(*v));
                    let pieces: f64 = pts
                        .windows(2)
                        .map(|w| {
                            let (a, b) = (about(w[0].1), about(w[1].1));
                            w[1].0 * (a * a + a * b + b * b) / 3.0
                        })
                        .sum();
                    atom + pieces
                }
            }
        })
    }
}

/// 1/√(2π).
const FRAC_1_SQRT_2PI: f64 = 0.398_942_280_401_432_7;

/// The standard normal density; 0 at ±∞.
fn normal_pdf(z: f64) -> f64 {
    if z.is_finite() {
        FRAC_1_SQRT_2PI * exp(-0.5 * z * z)
    } else {
        0.0
    }
}

/// The mean and variance of the normal (`mean`, `std_dev`) truncated to [min, max], for valid
/// parameters.
///
/// In closed form (Johnson, Kotz and Balakrishnan, *Continuous Univariate Distributions*, vol. 1,
/// 2nd ed., 1994, chapter 13), worked so that it stays finite and accurate however far the
/// bounds are from the mean and however narrow the normal is beside them. An interval on one side
/// of the mean is the upper tail beyond its near bound less the tail beyond its far bound (by
/// symmetry for the left side); one around the mean takes its probability from `erf`. Where the
/// density is nearly flat over the interval, which the closed forms lose to cancellation,
/// Simpson's rule gives the moments instead.
fn truncated_moments(mean: f64, std_dev: f64, min: f64, max: f64) -> (f64, f64) {
    let w = (max - min) / std_dev;
    let (m, v) = if min >= mean {
        let (u, v) = tail_moments((min - mean) / std_dev, w);
        (min + std_dev * u, v)
    } else if max <= mean {
        let (u, v) = tail_moments((mean - max) / std_dev, w);
        (max - std_dev * u, v)
    } else {
        let (a, b) = ((min - mean) / std_dev, (max - mean) / std_dev);
        if 0.5 * (a * a).max(b * b) <= FLAT {
            let (u, v) = flat_moments(a, w);
            (min + std_dev * u, v)
        } else {
            let z = 0.5 * (erf(b / std::f64::consts::SQRT_2) + erf(-a / std::f64::consts::SQRT_2));
            let z_pdf = |x: f64| {
                if x.is_finite() {
                    x * normal_pdf(x)
                } else {
                    0.0
                }
            };
            let m = (normal_pdf(a) - normal_pdf(b)) / z;
            (mean + std_dev * m, 1.0 + (z_pdf(a) - z_pdf(b)) / z - m * m)
        }
    };
    let v = std_dev * std_dev * v;
    (
        m.clamp(min, max),
        v.clamp(0.0, 0.25 * (max - min) * (max - min)),
    )
}

/// The most the log of the standard normal density may fall across an interval for it to count
/// as nearly flat there (a fall by a factor of e²).
const FLAT: f64 = 2.0;

/// The standard normal truncated to [a, a + w] with a ≥ 0: its mean less a, and its variance.
fn tail_moments(a: f64, w: f64) -> (f64, f64) {
    if !a.is_finite() {
        // The bound overflowed: everything is at it.
        return (0.0, 0.0);
    }
    // The log density falls by this much across the interval.
    let fall = 0.5 * w * (2.0 * a + w);
    if fall <= FLAT {
        return flat_moments(a, w);
    }
    let (mills_a, excess_a, var_a) = upper_tail(a);
    let (mills_b, excess_b, var_b) = upper_tail(a + w);
    // The tail beyond a + w as a share of the tail beyond a: below e^-FLAT.
    let p = if mills_a > 0.0 {
        exp(-fall) * mills_b / mills_a
    } else {
        0.0
    };
    if p == 0.0 {
        return (excess_a, var_a);
    }
    // Moments about a, of the tail beyond a less the tail beyond a + w.
    let (b1, b2) = (excess_b + w, var_b + (excess_b + w) * (excess_b + w));
    let m1 = (excess_a - p * b1) / (1.0 - p);
    let m2 = (var_a + excess_a * excess_a - p * b2) / (1.0 - p);
    (m1, m2 - m1 * m1)
}

/// The standard normal's upper tail beyond x ≥ 0: Mills' ratio Q(x)/φ(x), the tail's mean less
/// x, and its variance. From `erfc` near the mean; further out, where that loses digits to
/// cancellation (and underflows past x ≈ 38), from Laplace's continued fraction
/// Q(x)/φ(x) = 1/(x + 1/(x + 2/(x + 3/(x + …)))), whose last terms give the excess and variance
/// without cancellation (40 terms are exact to rounding from x = 4 on).
fn upper_tail(x: f64) -> (f64, f64, f64) {
    if !x.is_finite() {
        return (0.0, 0.0, 0.0);
    }
    if x < 4.0 {
        let mills = 0.5 * erfc(x / std::f64::consts::SQRT_2) / normal_pdf(x);
        let mean = 1.0 / mills;
        let excess = mean - x;
        return (mills, excess, 1.0 - mean * excess);
    }
    // t_k = x + (k + 1)/t_(k+1), from t_40 ≈ x down to t_2; then 1/t_0 is the ratio, the excess
    // is 1/t_1 and the variance 1 − t_0/t_1 = (2 t_1 − t_2)/(t_1² t_2).
    let mut t2 = x;
    for k in (3..=40).rev() {
        t2 = x + f64::from(k) / t2;
    }
    let t1 = x + 2.0 / t2;
    let t0 = x + 1.0 / t1;
    (1.0 / t0, 1.0 / t1, (2.0 * t1 - t2) / (t1 * t1 * t2))
}

/// The standard normal truncated to [a, a + w] where its density is nearly flat: its mean less
/// a, and its variance, by Simpson's rule (1,000 intervals) about the interval's middle.
fn flat_moments(a: f64, w: f64) -> (f64, f64) {
    const N: u32 = 1_000;
    let h = w / f64::from(N);
    let (mut m0, mut m1, mut m2) = (0.0, 0.0, 0.0);
    for i in 0..=N {
        let x = h * f64::from(i);
        let y = x - 0.5 * w;
        // Twice the fall in log density from the point nearest the mean (a, or the mean itself
        // when the interval holds it), kept small so nothing underflows.
        let fall = if a >= 0.0 {
            x * (2.0 * a + x)
        } else {
            (a + x) * (a + x)
        };
        let weight = if i == 0 || i == N {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        let f = weight * exp(-0.5 * fall);
        m0 += f;
        m1 += f * y;
        m2 += f * y * y;
    }
    let shift = m1 / m0;
    (0.5 * w + shift, m2 / m0 - shift * shift)
}

#[cfg(test)]
mod tests {
    #[test]
    fn scaled_distributions_scale_their_mean() {
        for d in [
            Dist::Constant(2.0),
            Dist::Exponential { mean: 1.5 },
            Dist::Uniform { min: 1.0, max: 3.0 },
            Dist::Triangular {
                min: 1.0,
                mode: 2.0,
                max: 6.0,
            },
            Dist::LogNormal {
                mean: 3.0,
                std_dev: 1.0,
            },
            Dist::Gamma {
                shape: 2.0,
                scale: 1.5,
            },
            Dist::Erlang { mean: 4.0, k: 3 },
            Dist::Weibull {
                shape: 1.5,
                scale: 2.0,
            },
            Dist::User {
                points: vec![(0.0, 1.0), (0.5, 2.0), (0.5, 4.0)],
                discrete: false,
            },
        ] {
            let (m, s) = (d.mean(), d.scaled(1.1).and_then(|s| s.mean()));
            assert!(
                matches!((m, s), (Some(m), Some(s)) if (s - 1.1 * m).abs() < 1e-9 * m.max(1.0)),
                "{d:?}: {m:?} {s:?}"
            );
        }
        assert_eq!(Dist::Poisson { mean: 2.0 }.scaled(2.0), None);
        assert_eq!(Dist::Constant(1.0).scaled(0.0), None);
    }

    use super::*;

    /// The truncated normal's mean and variance, against known values: half and symmetric
    /// truncations, tails far from the mean, a narrow interval, and a narrow normal inside wide
    /// bounds (where integrating over the bounds missed the peak).
    #[test]
    fn truncated_normal_moments_are_exact_in_the_tails() {
        let cases = [
            // (mean, std_dev, min, max), mean, variance
            (
                (0.0, 1.0, 0.0, 60.0),
                0.797_884_560_802_865_4,
                0.363_380_227_632_418_7,
            ),
            ((0.0, 1.0, -1.0, 1.0), 0.0, 0.291_125_094_772_793),
            (
                (0.0, 1.0, 1.0, 60.0),
                1.525_135_276_160_98,
                0.199_097_665_570_346,
            ),
            (
                (2.0, 1.0, 1.0, 1000.0),
                2.287_599_970_939_18,
                0.629_686_285_776_529,
            ),
            (
                (0.0, 1.0, 3.0, 3.4),
                3.158_665_253_073_33,
                0.012_252_218_751_405_3,
            ),
            ((30.0, 0.5, 0.0, 1e5), 30.0, 0.25),
            ((5.1, 0.001, 0.0, 1000.0), 5.1, 1e-6),
            (
                (0.0, 1.0, 40.0, 50.0),
                40.024_968_847_207_3,
                6.226_683_785_913_73e-4,
            ),
            (
                (0.0, 1.0, -50.0, -40.0),
                -40.024_968_847_207_3,
                6.226_683_785_913_73e-4,
            ),
            (
                (0.0, 1.0, 1.0, 1.000_001),
                1.000_000_499_999_92,
                8.333_333_331_961_53e-14,
            ),
            ((1e3, 1.0, 1e9, 1e9 + 1.0), 1e9, 1.000_002_000_003e-18),
        ];
        for ((mean, std_dev, min, max), m, v) in cases {
            let d = Dist::TruncatedNormal {
                mean,
                std_dev,
                min,
                max,
            };
            let (dm, dv) = (d.mean(), d.variance());
            assert!(
                dm.is_some_and(|x| (x - m).abs() <= 1e-9 * m.abs().max(1.0)),
                "{d:?}: mean {dm:?}, not {m}"
            );
            assert!(
                dv.is_some_and(|x| (x - v).abs() <= 1e-6 * v.max(1e-12)),
                "{d:?}: variance {dv:?}, not {v}"
            );
        }
    }

    /// User distributions sampled as before they were sampled in place: the old sampler, copied
    /// here, gives the same bits and leaves the generator in the same state.
    #[test]
    fn user_distributions_sample_as_they_did() {
        fn old(rng: &mut Rng, points: &[(f64, f64)], discrete: bool) -> f64 {
            let d = Dist::User {
                points: points.to_vec(),
                discrete,
            };
            if d.check().is_err() {
                return 0.0;
            }
            let Some(pts) = user_points(points, discrete) else {
                return 0.0;
            };
            let u = rng.uniform();
            let mut cumulative = 0.0;
            let mut previous: Option<(f64, f64)> = None;
            for (p, v) in &pts {
                let below = cumulative;
                cumulative += p;
                if u < cumulative {
                    return match (discrete, previous) {
                        (false, Some((_, v0))) if *p > 0.0 => v0 + (v - v0) * (u - below) / p,
                        _ => *v,
                    };
                }
                previous = Some((*p, *v));
            }
            pts.last().map_or(0.0, |(_, v)| *v)
        }
        let sets: Vec<Vec<(f64, f64)>> = vec![
            vec![(0.0, 1.0), (0.5, 2.0), (0.5, 4.0)],
            vec![(3.0, 1.0), (7.0, 2.5), (1.0, 9.0)],
            vec![(1.0, 5.0), (2.0, 1.0), (3.0, 3.0)],
            vec![(0.1, 2.0), (0.2, 2.0), (0.0, 1.0), (0.7, 3.0)],
            vec![(0.3, 1.0), (0.0, 2.0), (0.0, 2.0), (0.7, 4.0)],
            vec![(1.0, 7.0)],
            vec![(0.1, 0.1), (0.2, 0.2), (0.3, 0.3), (0.4, 0.4), (1e-9, 0.5)],
            vec![],
            vec![(0.0, 1.0), (0.0, 2.0)],
            vec![(-1.0, 1.0), (2.0, 2.0)],
            vec![(1.0, f64::NAN), (1.0, 2.0)],
            vec![(f64::MAX, 1.0), (f64::MAX, 2.0)],
        ];
        for points in &sets {
            for discrete in [false, true] {
                let d = Dist::User {
                    points: points.clone(),
                    discrete,
                };
                let (mut a, mut b) = (Rng::new(11), Rng::new(11));
                for _ in 0..2_000 {
                    let (x, y) = (d.sample(&mut a), old(&mut b, points, discrete));
                    assert_eq!(x.to_bits(), y.to_bits(), "{d:?}: {x} vs {y}");
                }
                assert_eq!(a.next_u64(), b.next_u64(), "{d:?}");
            }
        }
    }

    fn moments(d: &Dist, n: usize, seed: u64) -> (f64, f64) {
        let mut rng = Rng::new(seed);
        let xs: Vec<f64> = (0..n).map(|_| d.sample(&mut rng)).collect();
        let mean = xs.iter().sum::<f64>() / n as f64;
        let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1) as f64;
        (mean, var)
    }

    #[test]
    fn a_large_erlang_is_drawn_quickly_with_its_moments() {
        // k = 10¹⁰ by a sum would take 10¹⁰ draws each; mean 1, variance 10⁻¹⁰.
        let d = Dist::Erlang {
            mean: 1.0,
            k: 10_000_000_000,
        };
        let (m, v) = moments(&d, 20_000, 9);
        assert!((m - 1.0).abs() < 4.0 * 1e-5 / 20_000f64.sqrt(), "{m}");
        assert!((v / 1e-10 - 1.0).abs() < 0.05, "{v}");
        // Just above the switch, the same mean and variance as the sum gives.
        let d = Dist::Erlang { mean: 6.0, k: 65 };
        let (m, v) = moments(&d, 40_000, 10);
        assert!(
            (m - 6.0).abs() < 4.0 * (36.0 / 65.0 / 40_000.0f64).sqrt(),
            "{m}"
        );
        assert!((v / (36.0 / 65.0) - 1.0).abs() < 0.05, "{v}");
    }

    /// A beta with a tiny shape is nearly all at 0 or 1, but its mean is still α / (α + β). Drawn
    /// as a ratio of two gammas it lost its mean once a gamma underflowed to 0.
    #[test]
    fn a_beta_with_tiny_shapes_keeps_its_mean() {
        let n = 100_000;
        for (a, b, seed) in [
            (0.01, 0.01, 21),
            (0.001, 0.001, 22),
            (0.000_1, 0.000_1, 23),
            (0.002, 0.004, 24),
            (0.000_5, 3.0, 25),
        ] {
            let d = Dist::beta(a, b);
            let mut rng = Rng::new(seed);
            let xs: Vec<f64> = (0..n).map(|_| d.sample(&mut rng)).collect();
            assert!(xs.iter().all(|x| (0.0..=1.0).contains(x)), "({a}, {b})");
            let m = xs.iter().sum::<f64>() / n as f64;
            let want = a / (a + b);
            let var = d.variance().unwrap_or(0.25);
            // Seven standard errors.
            let tol = 7.0 * (var / n as f64).sqrt();
            assert!((m - want).abs() < tol, "Beta({a}, {b}): {m} against {want}");
        }
    }

    /// The beta mean does not overflow when both shapes are near the largest number.
    #[test]
    fn the_beta_mean_does_not_overflow() {
        let big = Dist::beta(1e308, 1e308);
        assert_eq!(big.mean(), Some(0.5));
        let d = Dist::beta(2.0, 3.0);
        assert!((d.mean().unwrap_or(f64::NAN) - 0.4).abs() < 1e-15);
    }

    /// A beta on [0, 1] draws bit for bit what the [0, 1] sampler draws, and has the [0, 1]
    /// formulas' mean and variance exactly, so models and files with BPSim's beta keep their
    /// results.
    #[test]
    fn a_beta_on_its_default_range_draws_exactly_as_before() {
        for (a, b, seed) in [
            (2.0, 5.0, 31),
            (0.5, 0.5, 32),
            (0.01, 3.0, 33),
            (40.0, 7.0, 34),
        ] {
            let d = Dist::beta(a, b);
            let (mut r1, mut r2) = (Rng::new(seed), Rng::new(seed));
            for _ in 0..2_000 {
                assert_eq!(
                    d.sample(&mut r1).to_bits(),
                    beta_sample(&mut r2, a, b).to_bits(),
                    "Beta({a}, {b})"
                );
            }
            assert_eq!(
                d.mean().map(f64::to_bits),
                Some((1.0 / (1.0 + b / a)).to_bits())
            );
            assert_eq!(
                d.variance().map(f64::to_bits),
                Some((a * b / ((a + b) * (a + b) * (a + b + 1.0))).to_bits())
            );
        }
    }

    /// Beta(3, 5) stretched to [1, 4]: mean 1 + 3 · 3/8 = 2.125 and variance
    /// 3² · 15 / (64 · 9) = 0.234375, every draw inside the range, and checks on the range.
    #[test]
    fn a_beta_on_a_range_has_its_mean_variance_and_bounds() {
        let d = Dist::Beta {
            shape: 3.0,
            scale: 5.0,
            min: 1.0,
            max: 4.0,
        };
        assert!(d.mean().is_some_and(|m| (m - 2.125).abs() < 1e-15));
        assert!(d.variance().is_some_and(|v| (v - 0.234_375).abs() < 1e-15));
        let n = 100_000;
        let mut rng = Rng::new(41);
        let xs: Vec<f64> = (0..n).map(|_| d.sample(&mut rng)).collect();
        assert!(xs.iter().all(|x| (1.0..=4.0).contains(x)));
        let m = xs.iter().sum::<f64>() / n as f64;
        assert!(
            (m - 2.125).abs() < 7.0 * (0.234_375 / n as f64).sqrt(),
            "{m}"
        );
        // A change of time unit stretches the range and keeps the shapes.
        assert_eq!(
            d.scaled(60.0),
            Some(Dist::Beta {
                shape: 3.0,
                scale: 5.0,
                min: 60.0,
                max: 240.0,
            })
        );
        assert!(d.can_be_positive());
        let range = |min: f64, max: f64| Dist::Beta {
            shape: 3.0,
            scale: 5.0,
            min,
            max,
        };
        assert_eq!(range(4.0, 1.0).check(), Err(DistError::BadBounds));
        assert_eq!(range(2.0, 2.0).check(), Err(DistError::BadBounds));
        assert!(range(f64::NAN, 1.0).check().is_err());
        assert!(range(0.0, f64::INFINITY).check().is_err());
        assert!(!range(-2.0, 0.0).can_be_positive());
        // Parameters it refuses draw its lower bound, never a panic.
        assert_eq!(range(4.0, 1.0).sample(&mut rng), 4.0);
    }

    fn all() -> Vec<Dist> {
        vec![
            Dist::Constant(2.5),
            Dist::Exponential { mean: 2.0 },
            Dist::Uniform { min: 2.0, max: 6.0 },
            Dist::Triangular {
                min: 1.0,
                mode: 2.0,
                max: 6.0,
            },
            Dist::Normal {
                mean: 10.0,
                std_dev: 2.0,
            },
            Dist::TruncatedNormal {
                mean: 2.0,
                std_dev: 1.0,
                min: 1.0,
                max: 1000.0,
            },
            Dist::TruncatedNormal {
                mean: 0.0,
                std_dev: 1.0,
                min: -0.3,
                max: 0.2,
            },
            Dist::TruncatedNormal {
                mean: 0.0,
                std_dev: 1.0,
                min: 3.0,
                max: 3.4,
            },
            Dist::TruncatedNormal {
                mean: 0.0,
                std_dev: 1.0,
                min: 4.0,
                max: 9.0,
            },
            Dist::TruncatedNormal {
                mean: 0.0,
                std_dev: 1.0,
                min: -9.0,
                max: -2.5,
            },
            Dist::LogNormal {
                mean: 5.0,
                std_dev: 2.0,
            },
            Dist::Gamma {
                shape: 2.5,
                scale: 1.5,
            },
            Dist::Gamma {
                shape: 0.4,
                scale: 2.0,
            },
            Dist::beta(2.0, 5.0),
            Dist::beta(0.5, 0.5),
            Dist::Erlang { mean: 6.0, k: 3 },
            Dist::Weibull {
                shape: 1.5,
                scale: 2.0,
            },
            Dist::Poisson { mean: 3.5 },
            Dist::Poisson { mean: 250.0 },
            Dist::Binomial {
                probability: 0.3,
                trials: 40,
            },
            Dist::Binomial {
                probability: 0.9,
                trials: 25,
            },
            Dist::User {
                points: vec![(0.2, 1.0), (0.5, 4.0), (0.3, 10.0)],
                discrete: true,
            },
            Dist::User {
                points: vec![(0.0, 0.0), (0.5, 2.0), (0.5, 10.0)],
                discrete: false,
            },
            // Last, so the seeds of those before it stay as they were.
            Dist::Beta {
                shape: 3.0,
                scale: 5.0,
                min: 1.0,
                max: 4.0,
            },
        ]
    }

    #[test]
    fn sample_moments_match_theory() {
        let n = 200_000;
        for (i, d) in all().iter().enumerate() {
            let (m, v) = moments(d, n, 1_000 + i as u64);
            let tm = d.mean().unwrap_or(f64::NAN);
            // Five standard errors of the mean, with a small absolute floor.
            let sd = d.variance().unwrap_or(v).sqrt();
            let tol = 5.0 * sd / (n as f64).sqrt() + 1e-9;
            assert!((m - tm).abs() <= tol, "{d:?}: mean {m} vs {tm} (tol {tol})");
            if let Some(tv) = d.variance() {
                assert!(
                    (v - tv).abs() <= 0.03 * tv + 1e-9,
                    "{d:?}: variance {v} vs {tv}"
                );
            }
        }
    }

    #[test]
    fn continuous_user_distribution_variance() {
        // Uniform on [0, 2] with probability 0.5 and on [2, 10] with 0.5.
        let d = Dist::User {
            points: vec![(0.0, 0.0), (0.5, 2.0), (0.5, 10.0)],
            discrete: false,
        };
        let (_, v) = moments(&d, 200_000, 7);
        let e2 = 0.5 * (4.0 / 3.0) + 0.5 * (1000.0 - 8.0) / 24.0;
        let tv = e2 - 3.5 * 3.5;
        assert!((v - tv).abs() < 0.03 * tv, "{v} vs {tv}");
    }

    #[test]
    fn supports_and_integers() {
        let mut r = Rng::new(3);
        for _ in 0..20_000 {
            let t = Dist::TruncatedNormal {
                mean: 0.0,
                std_dev: 1.0,
                min: 3.0,
                max: 3.4,
            }
            .sample(&mut r);
            assert!((3.0..=3.4).contains(&t));
            let b = Dist::beta(0.5, 0.5).sample(&mut r);
            assert!((0.0..=1.0).contains(&b));
            let p = Dist::Poisson { mean: 3.5 }.sample(&mut r);
            assert_eq!(p, p.trunc());
            let k = Dist::Binomial {
                probability: 0.9,
                trials: 25,
            }
            .sample(&mut r);
            assert!(k == k.trunc() && (0.0..=25.0).contains(&k));
            let u = Dist::User {
                points: vec![(0.2, 1.0), (0.5, 4.0), (0.3, 10.0)],
                discrete: true,
            }
            .sample(&mut r);
            assert!([1.0, 4.0, 10.0].contains(&u));
            assert!(
                Dist::LogNormal {
                    mean: 5.0,
                    std_dev: 2.0
                }
                .sample(&mut r)
                    > 0.0
            );
        }
    }

    #[test]
    fn ln_gamma_known_values() {
        assert!((ln_gamma(5.0) - ln(24.0)).abs() < 1e-12);
        assert!((ln_gamma(0.5) - ln(std::f64::consts::PI.sqrt())).abs() < 1e-12);
        assert!((ln_gamma(1.0)).abs() < 1e-12);
        assert!((ln_gamma(10.5) - ln(1_133_278.388_7)).abs() < 1e-9);
    }

    #[test]
    fn invalid_parameters_are_reported_and_never_panic() {
        let bad = [
            Dist::Uniform { min: 3.0, max: 3.0 },
            Dist::Triangular {
                min: 1.0,
                mode: 7.0,
                max: 6.0,
            },
            Dist::Normal {
                mean: 1.0,
                std_dev: -1.0,
            },
            Dist::LogNormal {
                mean: -1.0,
                std_dev: 1.0,
            },
            Dist::TruncatedNormal {
                mean: 0.0,
                std_dev: 0.0,
                min: 0.0,
                max: 1.0,
            },
            Dist::Gamma {
                shape: 0.0,
                scale: 1.0,
            },
            Dist::beta(1.0, f64::NAN),
            Dist::Erlang { mean: 1.0, k: 0 },
            Dist::Poisson { mean: -2.0 },
            Dist::Binomial {
                probability: 1.5,
                trials: 3,
            },
            Dist::User {
                points: vec![],
                discrete: true,
            },
            Dist::User {
                points: vec![(-0.5, 1.0)],
                discrete: true,
            },
            Dist::Exponential {
                mean: f64::INFINITY,
            },
        ];
        let mut r = Rng::new(1);
        for d in bad {
            assert!(d.check().is_err(), "{d:?} should be invalid");
            assert!(d.sample(&mut r).is_finite(), "{d:?}");
            assert_eq!(d.mean(), None);
        }
        assert_eq!(Dist::Uniform { min: 3.0, max: 3.0 }.sample(&mut r), 3.0);
        assert_eq!(Dist::Constant(-2.0).sample(&mut r), 0.0);
        assert_eq!(Dist::exponential_rate(0.0), Dist::Exponential { mean: 0.0 });
        for (d, positive) in [
            (Dist::Constant(0.0), false),
            (Dist::Constant(-1.0), false),
            (Dist::exponential_rate(0.0), false),
            (
                Dist::Uniform {
                    min: -2.0,
                    max: 0.0,
                },
                false,
            ),
            (
                Dist::Normal {
                    mean: 0.0,
                    std_dev: 0.0,
                },
                false,
            ),
            (
                Dist::User {
                    points: vec![(1.0, 0.0), (0.0, 5.0)],
                    discrete: false,
                },
                false,
            ),
            (Dist::Poisson { mean: -2.0 }, false),
            (Dist::Constant(0.5), true),
            (Dist::Exponential { mean: 2.0 }, true),
            (
                Dist::Normal {
                    mean: 0.0,
                    std_dev: 1.0,
                },
                true,
            ),
            (
                Dist::Uniform {
                    min: -1.0,
                    max: 1.0,
                },
                true,
            ),
            (
                Dist::User {
                    points: vec![(0.5, 0.0), (0.5, 5.0)],
                    discrete: true,
                },
                true,
            ),
        ] {
            assert_eq!(d.can_be_positive(), positive, "{d:?}");
        }
    }

    /// The Poisson sampler as it was before the transformed method was added: pieces of mean at
    /// most 30, each by Knuth's method.
    fn old_poisson(rng: &mut Rng, mean: f64) -> f64 {
        let pieces = (mean / 30.0).ceil().max(1.0);
        let each = mean / pieces;
        let mut total = 0u64;
        let mut left = pieces;
        while left > 0.0 {
            total += poisson_knuth(rng, each);
            left -= 1.0;
        }
        total as f64
    }

    /// The binomial sampler as it was before the transformed method and the remaining-trials
    /// check were added.
    fn old_binomial(rng: &mut Rng, p: f64, n: u64) -> u64 {
        if p <= 0.0 || n == 0 {
            return 0;
        }
        if p >= 1.0 {
            return n;
        }
        if p > 0.5 {
            return n - old_binomial(rng, 1.0 - p, n);
        }
        let log_q = ln(1.0 - p);
        let mut successes = 0u64;
        let mut position = 0u64;
        loop {
            let skip = (ln(rng.uniform_open0()) / log_q).floor();
            let step = if skip.is_finite() && skip < (n as f64) {
                skip as u64
            } else {
                n
            };
            position = position.saturating_add(step).saturating_add(1);
            if position > n {
                return successes;
            }
            successes += 1;
        }
    }

    #[test]
    fn small_counts_keep_their_fixed_seed_draws() {
        for mean in [0.0, 0.3, 2.0, 3.5, 9.0, 9.999_999] {
            for seed in 0..40 {
                let (mut a, mut b) = (Rng::new(seed), Rng::new(seed));
                let d = Dist::Poisson { mean };
                for _ in 0..50 {
                    assert_eq!(d.sample(&mut a), old_poisson(&mut b, mean), "mean {mean}");
                }
            }
        }
        // Each pair has trials × min(p, 1 − p) below 10.
        for (probability, trials) in [
            (0.3, 20),
            (0.9, 25),
            (0.05, 150),
            (0.5, 19),
            (0.97, 300),
            (0.2, 49),
            (0.001, 5_000),
            (0.4, 1),
        ] {
            for seed in 0..40 {
                let (mut a, mut b) = (Rng::new(seed), Rng::new(seed));
                let d = Dist::Binomial {
                    probability,
                    trials,
                };
                for _ in 0..50 {
                    let new = d.sample(&mut a);
                    let old = old_binomial(&mut b, probability, trials) as f64;
                    assert_eq!(new, old, "p {probability}, n {trials}");
                }
            }
        }
    }

    /// The sample mean and variance of `n` draws against theory, each within `sigmas` standard
    /// errors (a Poisson's variance estimate has variance (λ + 2λ²) / n).
    fn check_count_moments(d: &Dist, mean: f64, var: f64, fourth: f64, n: usize, seed: u64) {
        let (m, v) = moments(d, n, seed);
        let se_mean = (var / n as f64).sqrt();
        assert!(
            (m - mean).abs() <= 5.0 * se_mean + 1e-6,
            "{d:?}: mean {m} vs {mean} (se {se_mean})"
        );
        // The standard error of the sample variance: sqrt((μ4 − σ⁴) / n).
        let se_var = ((fourth - var * var) / n as f64).sqrt();
        assert!(
            (v - var).abs() <= 5.0 * se_var,
            "{d:?}: variance {v} vs {var} (se {se_var})"
        );
    }

    #[test]
    fn poisson_means_from_50_to_10_to_the_12_have_their_moments() {
        for (mean, seed) in [(10.0, 1u64), (50.0, 2), (1e6, 3), (1e12, 4)] {
            let d = Dist::Poisson { mean };
            // μ4 = λ + 3λ².
            check_count_moments(&d, mean, mean, mean + 3.0 * mean * mean, 200_000, seed);
        }
    }

    #[test]
    fn poisson_draws_are_whole_and_never_negative_at_the_edges() {
        let mut rng = Rng::new(5);
        for mean in [9.999_999, 10.0, 10.000_001, 31.9, 32.0, 1e14] {
            let d = Dist::Poisson { mean };
            for _ in 0..5_000 {
                let x = d.sample(&mut rng);
                assert!(x >= 0.0 && x == x.trunc(), "{mean}: {x}");
            }
        }
        // Above 10^15 a rounded normal, with the mean and variance of the Poisson.
        let mean = 1e16;
        let d = Dist::Poisson { mean };
        let (m, v) = moments(&d, 20_000, 6);
        assert!(
            (m - mean).abs() <= 5.0 * mean.sqrt() / 20_000f64.sqrt(),
            "{m}"
        );
        assert!((v / mean - 1.0).abs() < 0.1, "{v}");
        // A mean so large that adding 1 to a count no longer changes it ends at once.
        assert!(Dist::Poisson { mean: 1e300 }.sample(&mut rng) > 0.0);
    }

    #[test]
    fn binomials_with_many_trials_have_their_moments() {
        let n = 1e9;
        for (p, seed) in [
            (0.3, 11u64),
            (0.5, 12),
            (0.7, 13),
            (0.999_999, 14),
            (1e-6, 15),
        ] {
            let d = Dist::Binomial {
                probability: p,
                trials: 1_000_000_000,
            };
            let q = 1.0 - p;
            let var = n * p * q;
            // μ4 = σ⁴ (3 + (1 − 6pq) / (npq)) + ...: the fourth central moment of a binomial.
            let fourth = var * var * 3.0 + var * (1.0 - 6.0 * p * q);
            check_count_moments(&d, n * p, var, fourth, 200_000, seed);
        }
    }

    #[test]
    fn a_binomial_at_the_threshold_is_drawn_either_side_of_it() {
        // 20 × 0.5 = 10 uses the transformed method; 19 × 0.5 = 9.5 the waiting times.
        for (trials, seed) in [(19u64, 21u64), (20, 22), (21, 23)] {
            let p = 0.5;
            let d = Dist::Binomial {
                probability: p,
                trials,
            };
            let n = trials as f64;
            let var = n * p * (1.0 - p);
            let fourth = var * var * 3.0 + var * (1.0 - 6.0 * p * (1.0 - p));
            check_count_moments(&d, n * p, var, fourth, 200_000, seed);
        }
    }

    /// A chi-squared statistic and its number of cells for `draws` draws against the exact
    /// probabilities of 0 to `top`, running neighbouring values together until a cell expects at
    /// least 5 (what is left at the top, under 5 expected, is dropped).
    fn chi_squared(
        d: &Dist,
        pmf: impl Fn(u64) -> f64,
        top: u64,
        draws: usize,
        seed: u64,
    ) -> (f64, usize) {
        let mut rng = Rng::new(seed);
        let mut counts = vec![0usize; top as usize + 1];
        for _ in 0..draws {
            if let Some(c) = counts.get_mut(d.sample(&mut rng) as usize) {
                *c += 1;
            }
        }
        let n = draws as f64;
        let (mut chi, mut cells) = (0.0, 0usize);
        let (mut seen, mut expected) = (0.0, 0.0);
        for (k, count) in counts.iter().enumerate() {
            seen += *count as f64;
            expected += pmf(k as u64) * n;
            if expected >= 5.0 {
                chi += (seen - expected) * (seen - expected) / expected;
                cells += 1;
                (seen, expected) = (0.0, 0.0);
            }
        }
        (chi, cells)
    }

    #[test]
    fn the_transformed_methods_fit_their_distributions() {
        let draws = 400_000;
        let check = |name: String, (chi, cells): (f64, usize)| {
            // Chi-squared with cells − 1 degrees of freedom: its mean plus 8 standard
            // deviations is far beyond the 10^-9 point.
            let df = cells as f64 - 1.0;
            assert!(
                chi < df + 8.0 * (2.0 * df).sqrt() + 10.0,
                "{name}: {chi} on {df}"
            );
        };
        for (mean, seed) in [(10.0, 31u64), (12.5, 32), (31.0, 33), (45.0, 34)] {
            let d = Dist::Poisson { mean };
            let top = (mean + 12.0 * mean.sqrt()) as u64;
            let fit = chi_squared(
                &d,
                |k| exp(ln_poisson_pmf(k as f64, mean)),
                top,
                draws,
                seed,
            );
            check(format!("Poisson({mean})"), fit);
        }
        for (p, n, seed) in [
            (0.3, 40u64, 41u64),
            (0.5, 20, 42),
            (0.45, 100, 43),
            (0.8, 60, 44),
        ] {
            let d = Dist::Binomial {
                probability: p,
                trials: n,
            };
            let pmf = |k: u64| exp(ln_binomial_pmf(k as f64, n as f64, p));
            check(
                format!("Binomial({n}, {p})"),
                chi_squared(&d, pmf, n, draws, seed),
            );
        }
    }

    #[test]
    fn a_binomial_of_u64_max_trials_returns() {
        let mut rng = Rng::new(51);
        let max = u64::MAX;
        // Waiting times: a mean of 1.8 successes.
        let d = Dist::Binomial {
            probability: 1e-19,
            trials: max,
        };
        let total: f64 = (0..2_000).map(|_| d.sample(&mut rng)).sum();
        assert!(
            (1.5..2.1).contains(&(total / 2_000.0)),
            "{}",
            total / 2_000.0
        );
        // Probability below 2^-52, where 1 − p rounds to 1, still gives its successes.
        let tiny = Dist::Binomial {
            probability: 1e-17,
            trials: 100_000_000_000_000_000,
        };
        let total: f64 = (0..2_000).map(|_| tiny.sample(&mut rng)).sum();
        assert!(
            (0.8..1.2).contains(&(total / 2_000.0)),
            "{}",
            total / 2_000.0
        );
        // The transformed method, near the mean n p.
        for p in [0.3, 0.5, 0.9] {
            let k = Dist::Binomial {
                probability: p,
                trials: max,
            }
            .sample(&mut rng);
            let mean = max as f64 * p;
            assert!((k / mean - 1.0).abs() < 1e-8, "p {p}: {k}");
        }
        assert_eq!(
            Dist::Binomial {
                probability: 1.0,
                trials: max
            }
            .sample(&mut rng),
            max as f64
        );
        assert_eq!(binomial(&mut rng, 0.5, 0), 0);
    }

    #[test]
    fn the_log_probabilities_agree_with_the_direct_forms_and_sum_to_one() {
        // Against ln Γ where that is accurate.
        for (k, mean) in [(31.0, 40.0), (32.0, 40.0), (40.0, 50.0), (500.0, 480.0)] {
            let direct = -mean + k * ln(mean) - ln_gamma(k + 1.0);
            assert!(
                (ln_poisson_pmf(k, mean) - direct).abs() < 1e-10,
                "{k} {mean}"
            );
        }
        let (n, p) = (1000.0, 0.3);
        for k in [5.0, 31.0, 32.0, 250.0, 300.0, 340.0, 968.0, 969.0, 990.0] {
            let direct = ln_gamma(n + 1.0) - ln_gamma(k + 1.0) - ln_gamma(n - k + 1.0)
                + k * ln(p)
                + (n - k) * ln(1.0 - p);
            assert!(
                (ln_binomial_pmf(k, n, p) - direct).abs() < 1e-9 * direct.abs().max(1.0),
                "{k}: {} vs {direct}",
                ln_binomial_pmf(k, n, p)
            );
        }
        // At scales where ln Γ fails: the probabilities over ten standard deviations add to 1.
        let mean: f64 = 1e8;
        let sd = mean.sqrt();
        let lo = (mean - 10.0 * sd).floor();
        let total: f64 = (0..(20.0 * sd) as u64)
            .map(|i| exp(ln_poisson_pmf(lo + i as f64, mean)))
            .sum();
        assert!((total - 1.0).abs() < 1e-9, "{total}");
        let (n, p): (f64, f64) = (1e9, 0.3);
        let sd = (n * p * (1.0 - p)).sqrt();
        let lo = (n * p - 10.0 * sd).floor();
        let total: f64 = (0..(20.0 * sd) as u64)
            .map(|i| exp(ln_binomial_pmf(lo + i as f64, n, p)))
            .sum();
        assert!((total - 1.0).abs() < 1e-9, "{total}");
    }

    #[test]
    fn user_distribution_variances_are_exact() {
        let close =
            |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() <= 1e-12 * b.abs().max(1.0));
        // Discrete: E[X²] − mean² = 38.2 − 5.2².
        let discrete = Dist::User {
            points: vec![(0.2, 1.0), (0.5, 4.0), (0.3, 10.0)],
            discrete: true,
        };
        assert!(
            close(discrete.variance(), 38.2 - 5.2 * 5.2),
            "{:?}",
            discrete.variance()
        );
        // Weights that do not add to 1 are normalised: the same distribution.
        let scaled = Dist::User {
            points: vec![(2.0, 1.0), (5.0, 4.0), (3.0, 10.0)],
            discrete: true,
        };
        assert!(close(scaled.variance(), 38.2 - 5.2 * 5.2));
        // Continuous: an atom of 0.5 below, uniform on [0, 2] and on [2, 10] by halves.
        let e2 = 0.5 * (4.0 / 3.0) + 0.5 * (1000.0 - 8.0) / 24.0;
        let curve = Dist::User {
            points: vec![(0.0, 0.0), (0.5, 2.0), (0.5, 10.0)],
            discrete: false,
        };
        assert!(
            close(curve.variance(), e2 - 3.5 * 3.5),
            "{:?}",
            curve.variance()
        );
        // The same points out of order of value are sorted first.
        let shuffled = Dist::User {
            points: vec![(0.5, 10.0), (0.0, 0.0), (0.5, 2.0)],
            discrete: false,
        };
        assert!(close(shuffled.variance(), e2 - 3.5 * 3.5));
        // An atom of 1/4 at 1 and 3/4 even over [1, 4]: E[X²] = 1/4 + 3/4 · 7, mean 2.125.
        let atom = Dist::User {
            points: vec![(0.25, 1.0), (0.75, 4.0)],
            discrete: false,
        };
        assert!(close(atom.variance(), 0.25 + 0.75 * 7.0 - 2.125 * 2.125));
        // One point is a constant; a table the sampler rejects has none.
        let one = Dist::User {
            points: vec![(1.0, 5.0)],
            discrete: false,
        };
        assert!(close(one.variance(), 0.0));
        let bad = Dist::User {
            points: vec![(-1.0, 5.0)],
            discrete: false,
        };
        assert_eq!(bad.variance(), None);
    }

    #[test]
    fn same_seed_same_draws() {
        for d in all() {
            let (mut a, mut b) = (Rng::new(9), Rng::new(9));
            for _ in 0..100 {
                assert_eq!(
                    d.sample(&mut a).to_bits(),
                    d.sample(&mut b).to_bits(),
                    "{d:?}"
                );
            }
        }
    }
}

/// Machine-checked properties (Kani, `cargo kani -p stochcraft-random`; see docs/developers.md).
/// `Dist::check` only accepts parameters a draw can be made from: each harness takes arbitrary
/// `f64` parameters (NaN and infinities included) and shows what an `Ok` implies.
#[cfg(kani)]
mod proofs {
    use super::*;

    fn any_f64() -> f64 {
        kani::any()
    }

    #[kani::proof]
    fn bounded_kinds_are_accepted_only_with_finite_ordered_bounds() {
        let (min, mode, max) = (any_f64(), any_f64(), any_f64());
        if (Dist::Uniform { min, max }).check().is_ok() {
            assert!(min.is_finite() && max.is_finite() && min < max);
        }
        if (Dist::Triangular { min, mode, max }).check().is_ok() {
            assert!(min.is_finite() && mode.is_finite() && max.is_finite());
            assert!(min < max && min <= mode && mode <= max);
        }
        let (mean, std_dev) = (any_f64(), any_f64());
        if (Dist::TruncatedNormal {
            mean,
            std_dev,
            min,
            max,
        })
        .check()
        .is_ok()
        {
            assert!(mean.is_finite() && min.is_finite() && max.is_finite() && min < max);
            assert!(std_dev.is_finite() && std_dev > 0.0);
        }
    }

    #[kani::proof]
    fn scale_kinds_are_accepted_only_with_finite_non_negative_parameters() {
        let (a, b) = (any_f64(), any_f64());
        if (Dist::Exponential { mean: a }).check().is_ok() {
            assert!(a.is_finite() && a >= 0.0);
        }
        if (Dist::Poisson { mean: a }).check().is_ok() {
            assert!(a.is_finite() && a >= 0.0);
        }
        if (Dist::Normal {
            mean: a,
            std_dev: b,
        })
        .check()
        .is_ok()
        {
            assert!(a.is_finite() && b.is_finite() && b >= 0.0);
        }
        if (Dist::LogNormal {
            mean: a,
            std_dev: b,
        })
        .check()
        .is_ok()
        {
            assert!(a.is_finite() && a > 0.0 && b.is_finite() && b >= 0.0);
        }
        let k: u64 = kani::any();
        if (Dist::Erlang { mean: a, k }).check().is_ok() {
            assert!(a.is_finite() && a >= 0.0 && k >= 1);
        }
    }

    #[kani::proof]
    fn shape_scale_kinds_and_binomial_are_accepted_only_in_range() {
        let (shape, scale) = (any_f64(), any_f64());
        let (min, max) = (any_f64(), any_f64());
        let kinds = [
            Dist::Gamma { shape, scale },
            Dist::Beta {
                shape,
                scale,
                min,
                max,
            },
            Dist::Weibull { shape, scale },
        ];
        for d in &kinds {
            if d.check().is_ok() {
                assert!(shape.is_finite() && shape > 0.0);
                assert!(scale.is_finite() && scale > 0.0);
            }
        }
        // A beta's range: finite, and min below max.
        if kinds[1].check().is_ok() {
            assert!(min.is_finite() && max.is_finite() && min < max);
        }
        let trials: u64 = kani::any();
        let probability = any_f64();
        if (Dist::Binomial {
            probability,
            trials,
        })
        .check()
        .is_ok()
        {
            assert!(probability >= 0.0 && probability <= 1.0);
        }
    }

    /// A constant never samples NaN, infinite or negative, whatever the value.
    #[kani::proof]
    fn a_constant_samples_finite_and_non_negative() {
        let mut rng = Rng { s: [1, 2, 3, 4] };
        let x = Dist::Constant(any_f64()).sample(&mut rng);
        assert!(x.is_finite() && x >= 0.0);
    }
}
