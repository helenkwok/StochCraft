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
    /// Beta on [0, 1]. BPSim names its parameters `shape` and `scale`; they are taken as the two
    /// shape parameters α = `shape`, β = `scale` (mean α / (α + β)). Ratio of two gammas.
    Beta { shape: f64, scale: f64 },
    /// Erlang: the sum of `k` exponentials, with overall mean `mean`.
    Erlang { mean: f64, k: u64 },
    /// Weibull with shape k and scale λ. Inversion.
    Weibull { shape: f64, scale: f64 },
    /// Poisson count with this mean. Knuth's multiplication method, applied to pieces of mean at
    /// most 30 whose sum is exact (cost grows with the mean).
    Poisson { mean: f64 },
    /// Binomial count of successes in `trials` with success `probability`. Geometric waiting
    /// times (cost grows with trials × probability), using symmetry above probability 0.5.
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
    // Successes are where geometric waiting times land inside the n trials.
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
            // A beta lives on [0, 1]; BPSim's has no scale to change.
            Dist::Beta { .. } | Dist::Poisson { .. } | Dist::Binomial { .. } => return None,
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
            Dist::Gamma { shape, scale }
            | Dist::Beta { shape, scale }
            | Dist::Weibull { shape, scale } => {
                positive("shape", *shape)?;
                positive("scale", *scale)
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
            Dist::Beta { shape, scale } => {
                let x = gamma_sample(rng, *shape, 1.0);
                let y = gamma_sample(rng, *scale, 1.0);
                if x + y > 0.0 { x / (x + y) } else { 0.0 }
            }
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
            Dist::Poisson { mean } => {
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
            | Dist::TruncatedNormal { max, .. } => *max > 0.0,
            Dist::Normal { mean, std_dev } => *mean > 0.0 || *std_dev > 0.0,
            Dist::LogNormal { .. }
            | Dist::Gamma { .. }
            | Dist::Beta { .. }
            | Dist::Weibull { .. } => true,
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
    /// invalid).
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
            Dist::Beta { shape, scale } => shape / (shape + scale),
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
            Dist::Beta { shape: a, scale: b } => a * b / ((a + b) * (a + b) * (a + b + 1.0)),
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
            Dist::User { .. } => return None,
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
            Dist::Beta {
                shape: 2.0,
                scale: 5.0,
            },
            Dist::Beta {
                shape: 0.5,
                scale: 0.5,
            },
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
            let b = Dist::Beta {
                shape: 0.5,
                scale: 0.5,
            }
            .sample(&mut r);
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
            Dist::Beta {
                shape: 1.0,
                scale: f64::NAN,
            },
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
