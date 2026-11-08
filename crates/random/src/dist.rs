//! The distributions of BPSim 2.0 (section 6.3.5 of the specification), plus a constant.
//!
//! Every sampler is exact and never panics: parameters that [`Dist::check`] rejects give a safe
//! value (the lower bound, or 0) instead. Where the specification leaves a parameter's meaning
//! open, the choice made here is stated on the variant.

use crate::Rng;
use crate::math::{cos, exp, ln, powf, sin};

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
    /// mean duration is greater than `mean` (use a truncated normal to say what is meant).
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

/// Why a distribution's parameters cannot be sampled.
#[derive(Debug, Clone, PartialEq)]
pub enum DistError {
    /// A parameter is NaN or infinite.
    NotFinite(&'static str),
    /// A parameter is outside its range.
    OutOfRange {
        parameter: &'static str,
        requirement: &'static str,
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
            requirement: "greater than 0",
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
            requirement: "0 or more",
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
            if rng.uniform() <= exp(0.5 * (a * a - z * z)) {
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
                        requirement: "1 or more",
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
                        requirement: "between 0 and 1",
                    })
                }
            }
            Dist::User { points, .. } => {
                for (p, v) in points {
                    non_negative("probability", *p)?;
                    finite("value", *v)?;
                }
                let total: f64 = points.iter().map(|(p, _)| *p).sum();
                if total > 0.0 {
                    Ok(())
                } else {
                    Err(DistError::EmptyUserDistribution)
                }
            }
        }
    }

    /// Draws one value. Parameters [`Dist::check`] rejects give the lower bound or 0, never a panic.
    pub fn sample(&self, rng: &mut Rng) -> f64 {
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
            Dist::User { points, discrete } => {
                let Some(pts) = user_points(points, *discrete) else {
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
    /// invalid). For the truncated normal it is computed by numerical integration.
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
            } => {
                let (m0, m1, _) = truncated_moments(*mean, *std_dev, *min, *max);
                m1 / m0
            }
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
            } => {
                let (m0, m1, m2) = truncated_moments(*mean, *std_dev, *min, *max);
                let m = m1 / m0;
                m2 / m0 - m * m
            }
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

/// ∫ x^i φ((x − μ)/σ) dx over [min, max] for i = 0, 1, 2 (Simpson's rule, 4,000 intervals).
fn truncated_moments(mean: f64, std_dev: f64, min: f64, max: f64) -> (f64, f64, f64) {
    const N: usize = 4_000;
    let h = (max - min) / N as f64;
    let (mut m0, mut m1, mut m2) = (0.0, 0.0, 0.0);
    for i in 0..=N {
        let x = min + h * i as f64;
        let w = if i == 0 || i == N {
            1.0
        } else if i % 2 == 1 {
            4.0
        } else {
            2.0
        };
        let z = (x - mean) / std_dev;
        let f = w * exp(-0.5 * z * z);
        m0 += f;
        m1 += f * x;
        m2 += f * x * x;
    }
    (m0, m1, m2)
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

    fn moments(d: &Dist, n: usize, seed: u64) -> (f64, f64) {
        let mut rng = Rng::new(seed);
        let xs: Vec<f64> = (0..n).map(|_| d.sample(&mut rng)).collect();
        let mean = xs.iter().sum::<f64>() / n as f64;
        let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1) as f64;
        (mean, var)
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
