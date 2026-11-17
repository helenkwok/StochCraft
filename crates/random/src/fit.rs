//! Distributions fitted to measured data: a column of observed times fitted to each candidate
//! family by maximum likelihood, and ranked by how well each fits.
//!
//! The estimators are the textbook ones (A. M. Law, *Simulation Modeling and Analysis*, 5th ed.,
//! chapter 6; N. L. Johnson, S. Kotz and N. Balakrishnan, *Continuous Univariate Distributions*,
//! 2nd ed., 1994–1995):
//!
//! - **Exponential:** the mean is the sample mean.
//! - **Normal:** the sample mean, and the standard deviation with divisor n (the maximum-likelihood
//!   one, not n − 1).
//! - **Log-normal:** μ and σ² of the logarithms, σ² with divisor n; written as BPSim's log-normal,
//!   whose parameters are the mean e^(μ + σ²/2) and standard deviation of the values.
//! - **Gamma:** the shape k solves ln k − ψ(k) = ln x̄ − mean(ln x), by Newton's method, kept
//!   inside a bracket where the left side changes sign, from the positive root of
//!   (1/(2k))(1 + 1/(6k + 1)) = ln x̄ − mean(ln x), an approximation of ln k − ψ(k); the scale is
//!   x̄ / k.
//! - **Erlang:** the gamma with a whole-number shape: the whole numbers either side of the gamma's
//!   shape, whichever is more likely, with the sample mean as the mean.
//! - **Weibull:** the shape k solves Σ xᵏ ln x / Σ xᵏ − 1/k − mean(ln x) = 0, whose left side
//!   increases with k, by Newton's method from Menon's (1963) estimate π / (√6 · sd(ln x)), kept
//!   inside a bracket; the scale is (Σ xᵏ / n)^(1/k). The data are divided by their largest value
//!   first so that xᵏ cannot overflow.
//! - **Uniform and triangular:** their maximum-likelihood bounds are the smallest and largest
//!   values, where the distribution function is exactly 0 and 1 and a goodness-of-fit measure
//!   cannot be computed. The bounds are therefore widened by the range over n − 1 on each side
//!   (the minimum-variance unbiased estimates of a uniform's ends), but not below 0 when every
//!   value is positive. The triangular's mode is then the maximum-likelihood one for those bounds,
//!   which is one of the values (Johnson and Kotz, "Non-smooth sailing or triangular
//!   distributions revisited after some 50 years", *The Statistician* 48(2), 1999).
//!
//! Each fit is judged by the Anderson–Darling statistic A² = −n − (1/n) Σ (2i − 1)[ln zᵢ +
//! ln(1 − z₍ₙ₊₁₋ᵢ₎)] and the Kolmogorov–Smirnov distance D = maxᵢ max(i/n − zᵢ, zᵢ − (i − 1)/n),
//! where zᵢ is the fitted distribution function at the i-th smallest value (M. A. Stephens, "EDF
//! statistics for goodness of fit and some comparisons", *JASA* 69, 1974; R. B. D'Agostino and
//! M. A. Stephens, *Goodness-of-Fit Techniques*, 1986, chapter 4). 1 − z is computed directly
//! from each distribution's upper tail, not by subtraction, so the tails A² weighs keep their
//! digits. Fits are ranked by A² (smaller fits better), then D. Neither is turned into a p-value:
//! with parameters estimated from the same data their critical values depend on the family.
//!
//! Families that need values above 0 (exponential, gamma, Erlang, log-normal, Weibull) are left
//! out, with the reason, when a value is 0 or less.

use crate::Dist;
use crate::dist::ln_gamma;
use crate::math::{exp, ln};

/// Fewest values offered a fit, unless the caller asks for another number.
pub const MIN_COUNT: usize = 20;

/// Fewest values ever offered a fit, whatever the caller asks for.
pub const MIN_COUNT_FLOOR: usize = 5;

/// A family a column of data can be fitted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Exponential,
    Gamma,
    Erlang,
    LogNormal,
    Weibull,
    Normal,
    Uniform,
    Triangular,
}

impl Family {
    pub const ALL: [Family; 8] = [
        Family::Exponential,
        Family::Gamma,
        Family::Erlang,
        Family::LogNormal,
        Family::Weibull,
        Family::Normal,
        Family::Uniform,
        Family::Triangular,
    ];

    /// A stable name for files and agents: the `type` of the distribution it gives.
    pub fn name(self) -> &'static str {
        match self {
            Family::Exponential => "exponential",
            Family::Gamma => "gamma",
            Family::Erlang => "erlang",
            Family::LogNormal => "log_normal",
            Family::Weibull => "weibull",
            Family::Normal => "normal",
            Family::Uniform => "uniform",
            Family::Triangular => "triangular",
        }
    }

    /// The family by its name; `lognormal` is accepted too.
    pub fn from_name(s: &str) -> Option<Self> {
        if s == "lognormal" {
            return Some(Family::LogNormal);
        }
        Self::ALL.into_iter().find(|f| f.name() == s)
    }

    /// Whether every value must be above 0.
    pub fn needs_positive(self) -> bool {
        !matches!(self, Family::Normal | Family::Uniform | Family::Triangular)
    }
}

/// One family fitted to the data.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub family: Family,
    /// The fitted distribution, as BPSim writes it.
    pub dist: Dist,
    /// The log-likelihood of the data under it.
    pub log_likelihood: f64,
    /// Anderson–Darling A²; infinite when a value lies where the distribution has no
    /// probability.
    pub anderson_darling: f64,
    /// Kolmogorov–Smirnov D.
    pub kolmogorov_smirnov: f64,
}

/// Why a family was not fitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// The family needs every value above 0.
    NotPositive,
    /// The Erlang's best whole-number shape is 1, which is the exponential, fitted beside it.
    SameAsExponential,
    /// The likelihood equations had no usable solution.
    NoSolution,
}

impl std::fmt::Display for Skip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Skip::NotPositive => "needs every value above 0",
            Skip::SameAsExponential => "its best shape is 1, which is the exponential",
            Skip::NoSolution => "the likelihood equations have no usable solution for these data",
        })
    }
}

/// A family left out, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skipped {
    pub family: Family,
    pub reason: Skip,
}

/// The fits to a column of data, best first.
#[derive(Debug, Clone, PartialEq)]
pub struct Fits {
    pub count: usize,
    pub mean: f64,
    /// The sample standard deviation (divisor n − 1).
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
    /// Ranked by Anderson–Darling A², then Kolmogorov–Smirnov D.
    pub candidates: Vec<Candidate>,
    pub skipped: Vec<Skipped>,
}

/// Why data cannot be fitted at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataError {
    /// Fewer values than a fit needs.
    TooFew { count: usize, needed: usize },
    /// The value at this index (from 0) is NaN or infinite.
    NotFinite { index: usize },
    /// Every value is the same: a constant, not a distribution.
    AllEqual,
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::TooFew { count, needed } => write!(
                f,
                "{count} values are too few to fit a distribution: at least {needed} are needed"
            ),
            DataError::NotFinite { index } => {
                write!(f, "value {} is not a finite number", index + 1)
            }
            DataError::AllEqual => {
                f.write_str("every value is the same: use a constant, not a distribution")
            }
        }
    }
}

impl std::error::Error for DataError {}

/// Fits `data` to each of `families` (each once, in the order given) and ranks the fits. At
/// least `min_count` values are needed, and never fewer than [`MIN_COUNT_FLOOR`].
pub fn fit(data: &[f64], families: &[Family], min_count: usize) -> Result<Fits, DataError> {
    if let Some(index) = data.iter().position(|v| !v.is_finite()) {
        return Err(DataError::NotFinite { index });
    }
    let needed = min_count.max(MIN_COUNT_FLOOR);
    if data.len() < needed {
        return Err(DataError::TooFew {
            count: data.len(),
            needed,
        });
    }
    let mut x = data.to_vec();
    x.sort_by(f64::total_cmp);
    let (min, max) = (x[0], x[x.len() - 1]);
    if min == max {
        return Err(DataError::AllEqual);
    }
    let n = x.len() as f64;
    let mean = x.iter().sum::<f64>() / n;
    let ss = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>();

    let mut seen = Vec::new();
    let mut candidates = Vec::new();
    let mut skipped = Vec::new();
    for &family in families {
        if seen.contains(&family) {
            continue;
        }
        seen.push(family);
        if family.needs_positive() && min <= 0.0 {
            skipped.push(Skipped {
                family,
                reason: Skip::NotPositive,
            });
            continue;
        }
        let dist = match family {
            Family::Exponential => Some(Dist::Exponential { mean }),
            Family::Normal => Some(Dist::Normal {
                mean,
                std_dev: (ss / n).sqrt(),
            }),
            Family::LogNormal => Some(lognormal(&x)),
            Family::Gamma => gamma_shape(&x, mean).map(|k| Dist::Gamma {
                shape: k,
                scale: mean / k,
            }),
            Family::Erlang => match erlang(&x, mean) {
                Some(Dist::Erlang { k: 1, .. }) if families.contains(&Family::Exponential) => {
                    skipped.push(Skipped {
                        family,
                        reason: Skip::SameAsExponential,
                    });
                    continue;
                }
                d => d,
            },
            Family::Weibull => weibull(&x),
            Family::Uniform => {
                let (a, b) = widened_bounds(&x);
                Some(Dist::Uniform { min: a, max: b })
            }
            Family::Triangular => Some(triangular(&x)),
        };
        let candidate = dist
            .filter(|d| d.check().is_ok())
            .and_then(|d| candidate(family, d, &x));
        match candidate {
            Some(c) => candidates.push(c),
            None => skipped.push(Skipped {
                family,
                reason: Skip::NoSolution,
            }),
        }
    }
    candidates.sort_by(|a, b| {
        a.anderson_darling
            .total_cmp(&b.anderson_darling)
            .then(a.kolmogorov_smirnov.total_cmp(&b.kolmogorov_smirnov))
    });
    Ok(Fits {
        count: x.len(),
        mean,
        std_dev: (ss / (n - 1.0)).sqrt(),
        min,
        max,
        candidates,
        skipped,
    })
}

fn candidate(family: Family, dist: Dist, sorted: &[f64]) -> Option<Candidate> {
    let log_likelihood = sorted
        .iter()
        .map(|&v| ln_density(&dist, v))
        .sum::<Option<f64>>()?;
    let (anderson_darling, kolmogorov_smirnov) = statistics(sorted, &dist)?;
    if log_likelihood.is_nan() || anderson_darling.is_nan() || kolmogorov_smirnov.is_nan() {
        return None;
    }
    Some(Candidate {
        family,
        dist,
        log_likelihood,
        anderson_darling,
        kolmogorov_smirnov,
    })
}

/// How well `dist` fits `data`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoodnessOfFit {
    /// Anderson–Darling A² (infinite when a value lies where `dist` has no probability).
    pub anderson_darling: f64,
    /// Kolmogorov–Smirnov D.
    pub kolmogorov_smirnov: f64,
}

/// A² and D of `data` against `dist`, any distribution [`cdf`] supports (`None` otherwise, or
/// with no data).
pub fn goodness_of_fit(data: &[f64], dist: &Dist) -> Option<GoodnessOfFit> {
    let mut x = data.to_vec();
    x.sort_by(f64::total_cmp);
    let (anderson_darling, kolmogorov_smirnov) = statistics(&x, dist)?;
    Some(GoodnessOfFit {
        anderson_darling,
        kolmogorov_smirnov,
    })
}

/// A² and D of sorted data.
fn statistics(sorted: &[f64], dist: &Dist) -> Option<(f64, f64)> {
    if sorted.is_empty() {
        return None;
    }
    let tails = sorted
        .iter()
        .map(|&v| ln_cdf_sf(dist, v))
        .collect::<Option<Vec<_>>>()?;
    let n = sorted.len();
    let nf = n as f64;
    let mut sum = 0.0;
    let mut d: f64 = 0.0;
    for (i, &(ln_f, _)) in tails.iter().enumerate() {
        let ln_s_mirror = tails[n - 1 - i].1;
        sum += (2.0 * i as f64 + 1.0) * (ln_f + ln_s_mirror);
        let z = exp(ln_f);
        d = d.max((i as f64 + 1.0) / nf - z).max(z - i as f64 / nf);
    }
    Some((-nf - sum / nf, d))
}

/// The distribution function of `dist` at `x`, for the families data are fitted to (exponential,
/// gamma, Erlang, log-normal, Weibull, normal, uniform, triangular); `None` for the others or
/// invalid parameters.
pub fn cdf(dist: &Dist, x: f64) -> Option<f64> {
    ln_cdf_sf(dist, x).map(|(f, _)| exp(f))
}

/// The probability density of `dist` at `x`, for the same families as [`cdf`]: what a histogram
/// of the data is drawn against.
pub fn density(dist: &Dist, x: f64) -> Option<f64> {
    ln_density(dist, x).map(exp)
}

const LN_SQRT_2PI: f64 = 0.918_938_533_204_672_8;

/// The log-normal's μ and σ from BPSim's mean and standard deviation of the values.
fn lognormal_mu_sigma(mean: f64, std_dev: f64) -> (f64, f64) {
    let s2 = libm::log1p((std_dev / mean) * (std_dev / mean));
    (ln(mean) - s2 / 2.0, s2.sqrt())
}

/// ln F(x) and ln(1 − F(x)), each computed directly where it is small.
fn ln_cdf_sf(dist: &Dist, x: f64) -> Option<(f64, f64)> {
    dist.check().ok()?;
    let below = (f64::NEG_INFINITY, 0.0);
    let above = (0.0, f64::NEG_INFINITY);
    // From ln of the upper tail t of an exponential-type survival e^(−t).
    let from_t = |t: f64| (ln(-libm::expm1(-t)), -t);
    Some(match *dist {
        Dist::Exponential { mean } => {
            if x <= 0.0 {
                below
            } else {
                from_t(x / mean)
            }
        }
        Dist::Weibull { shape, scale } => {
            if x <= 0.0 {
                below
            } else {
                from_t(exp(shape * ln(x / scale)))
            }
        }
        Dist::Gamma { shape, scale } => {
            if x <= 0.0 {
                below
            } else {
                ln_incomplete_gamma(shape, x / scale)
            }
        }
        Dist::Erlang { mean, k } => {
            if x <= 0.0 {
                below
            } else {
                ln_incomplete_gamma(k as f64, x * k as f64 / mean)
            }
        }
        Dist::Normal { mean, std_dev } => ln_normal_tails((x - mean) / std_dev),
        Dist::LogNormal { mean, std_dev } => {
            if x <= 0.0 {
                below
            } else {
                let (mu, sigma) = lognormal_mu_sigma(mean, std_dev);
                ln_normal_tails((ln(x) - mu) / sigma)
            }
        }
        Dist::Uniform { min, max } => {
            if x <= min {
                below
            } else if x >= max {
                above
            } else {
                (ln((x - min) / (max - min)), ln((max - x) / (max - min)))
            }
        }
        Dist::Triangular { min, mode, max } => {
            if x <= min {
                below
            } else if x >= max {
                above
            } else if x <= mode {
                let f = ln((x - min) * (x - min) / ((max - min) * (mode - min)));
                (f, libm::log1p(-exp(f)))
            } else {
                let s = ln((max - x) * (max - x) / ((max - min) * (max - mode)));
                (libm::log1p(-exp(s)), s)
            }
        }
        _ => return None,
    })
}

/// ln Φ(z) and ln(1 − Φ(z)).
fn ln_normal_tails(z: f64) -> (f64, f64) {
    let r = std::f64::consts::FRAC_1_SQRT_2;
    (ln(0.5 * libm::erfc(-z * r)), ln(0.5 * libm::erfc(z * r)))
}

fn ln_density(dist: &Dist, x: f64) -> Option<f64> {
    dist.check().ok()?;
    let ninf = f64::NEG_INFINITY;
    let gamma = |shape: f64, scale: f64| {
        if x <= 0.0 {
            ninf
        } else {
            let y = x / scale;
            (shape - 1.0) * ln(y) - y - ln_gamma(shape) - ln(scale)
        }
    };
    Some(match *dist {
        Dist::Exponential { mean } => {
            if x < 0.0 {
                ninf
            } else {
                -ln(mean) - x / mean
            }
        }
        Dist::Weibull { shape, scale } => {
            if x <= 0.0 {
                ninf
            } else {
                let l = ln(x / scale);
                ln(shape / scale) + (shape - 1.0) * l - exp(shape * l)
            }
        }
        Dist::Gamma { shape, scale } => gamma(shape, scale),
        Dist::Erlang { mean, k } => gamma(k as f64, mean / k as f64),
        Dist::Normal { mean, std_dev } => {
            let z = (x - mean) / std_dev;
            -LN_SQRT_2PI - ln(std_dev) - z * z / 2.0
        }
        Dist::LogNormal { mean, std_dev } => {
            if x <= 0.0 {
                ninf
            } else {
                let (mu, sigma) = lognormal_mu_sigma(mean, std_dev);
                let z = (ln(x) - mu) / sigma;
                -LN_SQRT_2PI - ln(sigma) - ln(x) - z * z / 2.0
            }
        }
        Dist::Uniform { min, max } => {
            if x < min || x > max {
                ninf
            } else {
                -ln(max - min)
            }
        }
        Dist::Triangular { min, mode, max } => {
            if x < min || x > max {
                ninf
            } else if x == mode {
                ln(2.0 / (max - min))
            } else if x < mode {
                ln(2.0 * (x - min) / ((max - min) * (mode - min)))
            } else {
                ln(2.0 * (max - x) / ((max - min) * (max - mode)))
            }
        }
        _ => return None,
    })
}

/// ln P(a, y) and ln Q(a, y), the regularised lower and upper incomplete gamma functions, for
/// a, y > 0. P by its power series when y < a + 1, otherwise Q by Legendre's continued fraction
/// in its even (contracted) form, evaluated by the modified Lentz method; the other as the
/// complement (Abramowitz and Stegun, *Handbook of Mathematical Functions*, chapter 6).
fn ln_incomplete_gamma(a: f64, y: f64) -> (f64, f64) {
    let ln_front = a * ln(y) - y - ln_gamma(a);
    if y < a + 1.0 {
        let mut term = 1.0 / a;
        let mut sum = term;
        for n in 1..100_000 {
            term *= y / (a + f64::from(n));
            sum += term;
            if term < sum * 1e-17 {
                break;
            }
        }
        let ln_p = ln_front + ln(sum);
        (ln_p, libm::log1p(-exp(ln_p)))
    } else {
        const TINY: f64 = 1e-300;
        let mut f = y + 1.0 - a;
        if f.abs() < TINY {
            f = TINY;
        }
        let (mut c, mut d) = (f, 0.0);
        for n in 1..100_000 {
            let nf = f64::from(n);
            let an = -nf * (nf - a);
            let bn = y + 2.0 * nf + 1.0 - a;
            d = bn + an * d;
            if d.abs() < TINY {
                d = TINY;
            }
            c = bn + an / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            let delta = c * d;
            f *= delta;
            if (delta - 1.0).abs() < 1e-16 {
                break;
            }
        }
        let ln_q = ln_front - ln(f);
        (libm::log1p(-exp(ln_q)), ln_q)
    }
}

/// The digamma function ψ(x) for x > 0: the recurrence ψ(x) = ψ(x + 1) − 1/x up to x ≥ 10, then
/// the asymptotic series (Abramowitz and Stegun, chapter 6).
fn digamma(mut x: f64) -> f64 {
    let mut shift = 0.0;
    while x < 10.0 {
        shift -= 1.0 / x;
        x += 1.0;
    }
    let r = 1.0 / (x * x);
    let series = r
        * (1.0 / 12.0
            - r * (1.0 / 120.0 - r * (1.0 / 252.0 - r * (1.0 / 240.0 - r * (1.0 / 132.0)))));
    shift + ln(x) - 0.5 / x - series
}

/// The trigamma function ψ′(x) for x > 0: ψ′(x) = ψ′(x + 1) + 1/x² up to x ≥ 10, then the
/// asymptotic series (Abramowitz and Stegun, chapter 6).
fn trigamma(mut x: f64) -> f64 {
    let mut shift = 0.0;
    while x < 10.0 {
        shift += 1.0 / (x * x);
        x += 1.0;
    }
    let r = 1.0 / (x * x);
    let series = 1.0 / x
        + r / 2.0
        + r / x
            * (1.0 / 6.0 - r * (1.0 / 30.0 - r * (1.0 / 42.0 - r * (1.0 / 30.0 - r * 5.0 / 66.0))));
    shift + series
}

/// The root of `g`, which changes sign once between `lo` and `hi` (`g_lo_positive` says which
/// way), by Newton's method from `x0` with `dg` its derivative, falling back to bisection (on a
/// log scale) whenever a step would leave the bracket.
fn solve(
    g: impl Fn(f64) -> (f64, f64),
    mut lo: f64,
    mut hi: f64,
    g_lo_positive: bool,
    x0: f64,
) -> Option<f64> {
    let mut x = x0.clamp(lo, hi);
    for _ in 0..500 {
        let (gx, dgx) = g(x);
        if !gx.is_finite() {
            return None;
        }
        if gx == 0.0 {
            return Some(x);
        }
        if (gx > 0.0) == g_lo_positive {
            lo = x;
        } else {
            hi = x;
        }
        let newton = x - gx / dgx;
        let next = if newton.is_finite() && newton > lo && newton < hi {
            newton
        } else {
            (lo * hi).sqrt()
        };
        if (next - x).abs() <= 1e-14 * x || hi - lo <= 1e-14 * hi {
            return Some(next);
        }
        x = next;
    }
    Some(x)
}

/// Widens `lo` (to smaller) and `hi` (to larger) until `g` has the sign wanted at each.
fn bracket(g: impl Fn(f64) -> f64, x0: f64, g_lo_positive: bool) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (x0, x0);
    let wanted_lo = |v: f64| (v > 0.0) == g_lo_positive;
    for _ in 0..2_000 {
        if wanted_lo(g(lo)) {
            break;
        }
        lo /= 2.0;
        if lo < 1e-300 {
            return None;
        }
    }
    for _ in 0..2_000 {
        if !wanted_lo(g(hi)) {
            break;
        }
        hi *= 2.0;
        if hi > 1e300 {
            return None;
        }
    }
    Some((lo, hi))
}

/// The gamma's maximum-likelihood shape, from s = ln x̄ − mean(ln x).
fn gamma_shape(x: &[f64], mean: f64) -> Option<f64> {
    let n = x.len() as f64;
    let s = ln(mean) - x.iter().map(|&v| ln(v)).sum::<f64>() / n;
    if !(s.is_finite() && s > 0.0) {
        return None;
    }
    // The positive root of 6s k² + (s − 3) k − 1 = 0, from ln k − ψ(k) ≈ (1/(2k))(1 + 1/(6k + 1)).
    let k0 = (3.0 - s + ((s - 3.0) * (s - 3.0) + 24.0 * s).sqrt()) / (12.0 * s);
    let g = |k: f64| ln(k) - digamma(k) - s;
    let (lo, hi) = bracket(g, k0, true)?;
    solve(|k| (g(k), 1.0 / k - trigamma(k)), lo, hi, true, k0)
}

/// The Erlang: the gamma's shape rounded down and up (at least 1), whichever is more likely,
/// with the sample mean.
fn erlang(x: &[f64], mean: f64) -> Option<Dist> {
    let k = gamma_shape(x, mean)?;
    let ll = |d: &Dist| x.iter().map(|&v| ln_density(d, v)).sum::<Option<f64>>();
    let mut best: Option<(f64, Dist)> = None;
    for whole in [libm::floor(k), libm::ceil(k)] {
        let whole = whole.max(1.0);
        if whole > 1e15 {
            return None;
        }
        let d = Dist::Erlang {
            mean,
            k: whole as u64,
        };
        let l = ll(&d)?;
        if best.as_ref().is_none_or(|(b, _)| l > *b) {
            best = Some((l, d));
        }
    }
    best.map(|(_, d)| d)
}

fn lognormal(x: &[f64]) -> Dist {
    let n = x.len() as f64;
    let mu = x.iter().map(|&v| ln(v)).sum::<f64>() / n;
    let s2 = x.iter().map(|&v| (ln(v) - mu) * (ln(v) - mu)).sum::<f64>() / n;
    let mean = exp(mu + s2 / 2.0);
    Dist::LogNormal {
        mean,
        std_dev: mean * libm::expm1(s2).sqrt(),
    }
}

fn weibull(x: &[f64]) -> Option<Dist> {
    let n = x.len() as f64;
    let top = x.iter().copied().fold(f64::MIN_POSITIVE, f64::max);
    // ln of the data divided by their largest value: all ≤ 0, so yᵏ ≤ 1.
    let ly: Vec<f64> = x.iter().map(|&v| ln(v / top)).collect();
    let mean_ly = ly.iter().sum::<f64>() / n;
    let sd_ly = (ly
        .iter()
        .map(|l| (l - mean_ly) * (l - mean_ly))
        .sum::<f64>()
        / n)
        .sqrt();
    if !(sd_ly.is_finite() && sd_ly > 0.0) {
        return None;
    }
    // Σ yᵏ, Σ yᵏ ln y and Σ yᵏ (ln y)².
    let sums = |k: f64| {
        ly.iter().fold((0.0, 0.0, 0.0), |(s0, s1, s2), &l| {
            let w = exp(k * l);
            (s0 + w, s1 + w * l, s2 + w * l * l)
        })
    };
    let h = |k: f64| {
        let (s0, s1, s2) = sums(k);
        let m1 = s1 / s0;
        (m1 - 1.0 / k - mean_ly, s2 / s0 - m1 * m1 + 1.0 / (k * k))
    };
    // Menon (1963): 1/k is about √6/π times the standard deviation of ln x.
    let k0 = std::f64::consts::PI / (6.0_f64.sqrt() * sd_ly);
    let (lo, hi) = bracket(|k| h(k).0, k0, false)?;
    let k = solve(h, lo, hi, false, k0)?;
    let (s0, _, _) = sums(k);
    Some(Dist::Weibull {
        shape: k,
        scale: top * exp(ln(s0 / n) / k),
    })
}

/// The smallest and largest values moved out by the range over n − 1, the lower not below 0
/// when every value is above 0.
fn widened_bounds(sorted: &[f64]) -> (f64, f64) {
    let (lo, hi) = (sorted[0], sorted[sorted.len() - 1]);
    let pad = (hi - lo) / (sorted.len() as f64 - 1.0);
    let a = lo - pad;
    (if lo > 0.0 { a.max(0.0) } else { a }, hi + pad)
}

/// The triangular on the widened bounds whose mode, one of the values, is the most likely.
fn triangular(sorted: &[f64]) -> Dist {
    let (a, b) = widened_bounds(sorted);
    let n = sorted.len();
    // prefix[r] = Σ_{i<r} ln(xᵢ − a); suffix[r] = Σ_{i≥r} ln(b − xᵢ).
    let mut prefix = vec![0.0; n + 1];
    for (i, &v) in sorted.iter().enumerate() {
        prefix[i + 1] = prefix[i] + ln(v - a);
    }
    let mut suffix = vec![0.0; n + 1];
    for (i, &v) in sorted.iter().enumerate().rev() {
        suffix[i] = suffix[i + 1] + ln(b - v);
    }
    // With the mode at x_r, values up to it have density 2(x − a)/((b − a)(c − a)), the rest
    // 2(b − x)/((b − a)(b − c)); the terms common to every r are left out.
    let mut best = (f64::NEG_INFINITY, sorted[0]);
    for (r, &c) in sorted.iter().enumerate() {
        let left = (r + 1) as f64;
        let right = (n - r - 1) as f64;
        let l = prefix[r + 1] - left * ln(c - a) + suffix[r + 1] - right * ln(b - c);
        if l > best.0 {
            best = (l, c);
        }
    }
    Dist::Triangular {
        min: a,
        mode: best.1,
        max: b,
    }
}

#[cfg(test)]
#[allow(clippy::panic)] // A wrong answer fails the test.
mod tests {
    use super::*;
    use crate::Rng;

    fn sample(d: &Dist, n: usize, seed: u64) -> Vec<f64> {
        let mut rng = Rng::new(seed);
        (0..n).map(|_| d.sample(&mut rng)).collect()
    }

    fn only(data: &[f64], family: Family) -> Dist {
        let fits = fit(data, &[family], MIN_COUNT);
        match fits {
            Ok(f) if f.candidates.len() == 1 => f.candidates[0].dist.clone(),
            other => panic!("{family:?}: {other:?}"),
        }
    }

    fn within(name: &str, got: f64, want: f64, tol: f64) {
        assert!(
            (got - want).abs() <= tol,
            "{name}: got {got}, want {want} ± {tol}"
        );
    }

    /// The tiny data set of the hand check below.
    const TINY: [f64; 5] = [0.3, 0.9, 1.4, 2.2, 3.7];

    #[test]
    fn statistics_match_a_hand_computation() {
        // Computed independently in Python (standard library only) from Stephens's formulas,
        // with 1 − F rather than the survival function, for TINY against fixed distributions:
        //   exponential mean 1.5:      A² = 0.352173071045, D = 0.251188363906
        //   normal 2, 1:               A² = 0.709196358491, D = 0.325746882250
        //   Erlang k = 2, mean 2:      A² = 0.285703241666, D = 0.191832713460
        //   Weibull shape 1.5, scale 2: A² = 0.185976569121, D = 0.156737169438
        //   log-normal mean 2, sd 1.5: A² = 0.600932205466, D = 0.205453305976
        //   uniform 0..4:              A² = 0.365291950396, D = 0.25
        //   triangular 0, 1, 4:        A² = 0.695587905488, D = 0.1975
        let cases = [
            (
                Dist::Exponential { mean: 1.5 },
                0.352_173_071_045,
                0.251_188_363_906,
            ),
            (
                Dist::Normal {
                    mean: 2.0,
                    std_dev: 1.0,
                },
                0.709_196_358_491,
                0.325_746_882_250,
            ),
            (
                Dist::Erlang { mean: 2.0, k: 2 },
                0.285_703_241_666,
                0.191_832_713_460,
            ),
            (
                Dist::Gamma {
                    shape: 2.0,
                    scale: 1.0,
                },
                0.285_703_241_666,
                0.191_832_713_460,
            ),
            (
                Dist::Weibull {
                    shape: 1.5,
                    scale: 2.0,
                },
                0.185_976_569_121,
                0.156_737_169_438,
            ),
            (
                Dist::LogNormal {
                    mean: 2.0,
                    std_dev: 1.5,
                },
                0.600_932_205_466,
                0.205_453_305_976,
            ),
            (
                Dist::Uniform { min: 0.0, max: 4.0 },
                0.365_291_950_396,
                0.25,
            ),
            (
                Dist::Triangular {
                    min: 0.0,
                    mode: 1.0,
                    max: 4.0,
                },
                0.695_587_905_488,
                0.1975,
            ),
        ];
        for (d, a2, ks) in cases {
            let g = goodness_of_fit(&TINY, &d).unwrap_or_else(|| panic!("{d:?}"));
            within(&format!("{d:?} A²"), g.anderson_darling, a2, 1e-10);
            within(&format!("{d:?} D"), g.kolmogorov_smirnov, ks, 1e-10);
        }
        assert_eq!(goodness_of_fit(&TINY, &Dist::Poisson { mean: 1.0 }), None);
        assert_eq!(goodness_of_fit(&[], &Dist::Exponential { mean: 1.0 }), None);
    }

    #[test]
    fn shape_equations_match_a_hand_computation() {
        // The same Python check solved the likelihood equations for TINY by bisection, with ψ
        // from a central difference of ln Γ: gamma shape 1.7833691649, scale 0.9532518748;
        // Weibull shape 1.4452264959, scale 1.8758236531.
        let x = TINY;
        let mean = x.iter().sum::<f64>() / 5.0;
        let k = gamma_shape(&x, mean).unwrap_or(f64::NAN);
        within("gamma shape", k, 1.783_369_164_9, 1e-7);
        within("gamma scale", mean / k, 0.953_251_874_8, 1e-7);
        match weibull(&x) {
            Some(Dist::Weibull { shape, scale }) => {
                within("Weibull shape", shape, 1.445_226_495_9, 1e-8);
                within("Weibull scale", scale, 1.875_823_653_1, 1e-8);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn special_functions_match_known_values() {
        // ψ(1) = −γ, ψ(1/2) = −γ − 2 ln 2, ψ′(1) = π²/6, ψ′(1/2) = π²/2.
        let euler = 0.577_215_664_901_532_9;
        let pi2 = std::f64::consts::PI * std::f64::consts::PI;
        within("ψ(1)", digamma(1.0), -euler, 1e-13);
        within("ψ(1/2)", digamma(0.5), -euler - 2.0 * ln(2.0), 1e-13);
        within("ψ′(1)", trigamma(1.0), pi2 / 6.0, 1e-12);
        within("ψ′(1/2)", trigamma(0.5), pi2 / 2.0, 1e-12);
        // P(1, y) = 1 − e^−y; P(3, y) = 1 − e^−y (1 + y + y²/2), either side of y = a + 1.
        for y in [0.01, 0.5, 2.0, 3.9, 4.1, 10.0, 40.0] {
            let (lp, lq) = ln_incomplete_gamma(1.0, y);
            within("Q(1, y)", lq, -y, 1e-12);
            within("P(1, y)", exp(lp), 1.0 - exp(-y), 1e-14);
            let q3 = exp(-y) * (1.0 + y + y * y / 2.0);
            let (lp, lq) = ln_incomplete_gamma(3.0, y);
            within("Q(3, y)", exp(lq), q3, 1e-14);
            within("P(3, y)", exp(lp), 1.0 - q3, 1e-14);
        }
    }

    #[test]
    fn density_and_distribution_function_agree() {
        let dists = [
            Dist::Exponential { mean: 2.0 },
            Dist::Gamma {
                shape: 2.5,
                scale: 0.8,
            },
            Dist::Erlang { mean: 3.0, k: 3 },
            Dist::LogNormal {
                mean: 2.0,
                std_dev: 1.0,
            },
            Dist::Weibull {
                shape: 1.7,
                scale: 2.0,
            },
            Dist::Normal {
                mean: 2.0,
                std_dev: 0.7,
            },
            Dist::Uniform { min: 0.5, max: 3.0 },
            Dist::Triangular {
                min: 0.5,
                mode: 1.0,
                max: 4.0,
            },
        ];
        for d in dists {
            // The density is the slope of the distribution function, away from any corner.
            for x in [0.7, 1.3, 1.9, 2.6] {
                let h = 1e-5;
                let up = cdf(&d, x + h).unwrap_or(f64::NAN);
                let down = cdf(&d, x - h).unwrap_or(f64::NAN);
                let slope = (up - down) / (2.0 * h);
                let at = density(&d, x).unwrap_or(f64::NAN);
                within(&format!("{d:?} at {x}"), at, slope, 1e-6);
            }
        }
    }

    #[test]
    fn maximum_likelihood_recovers_the_parameters() {
        // 20,000 values from StochCraft's own sampler, fixed seeds. Each tolerance is four
        // asymptotic standard errors of the estimator (the inverse Fisher information), so a correct estimator fails it about once in 16,000 seeds.
        let n = 20_000;
        let nf = n as f64;
        let se = |var: f64| 4.0 * (var / nf).sqrt();

        let x = sample(&Dist::Exponential { mean: 2.5 }, n, 1);
        match only(&x, Family::Exponential) {
            Dist::Exponential { mean } => within("exponential mean", mean, 2.5, se(2.5 * 2.5)),
            d => panic!("{d:?}"),
        }

        // Gamma shape: Var(k̂) ≈ k / (n (k ψ′(k) − 1)).
        let (k, theta) = (2.5, 1.2);
        let x = sample(
            &Dist::Gamma {
                shape: k,
                scale: theta,
            },
            n,
            2,
        );
        match only(&x, Family::Gamma) {
            Dist::Gamma { shape, scale } => {
                within("gamma shape", shape, k, se(k / (k * trigamma(k) - 1.0)));
                within(
                    "gamma mean",
                    shape * scale,
                    k * theta,
                    se(k * theta * theta),
                );
            }
            d => panic!("{d:?}"),
        }

        let x = sample(&Dist::Erlang { mean: 6.0, k: 3 }, n, 3);
        match only(&x, Family::Erlang) {
            Dist::Erlang { mean, k } => {
                assert_eq!(k, 3);
                within("Erlang mean", mean, 6.0, se(36.0 / 3.0));
            }
            d => panic!("{d:?}"),
        }

        // Log-normal: μ̂ has variance σ²/n and σ̂ σ²/(2n); compared through μ and σ.
        let (mu, sigma) = (0.5, 0.8);
        let mean = exp(mu + sigma * sigma / 2.0);
        let sd = mean * libm::expm1(sigma * sigma).sqrt();
        let x = sample(&Dist::LogNormal { mean, std_dev: sd }, n, 4);
        match only(&x, Family::LogNormal) {
            Dist::LogNormal { mean, std_dev } => {
                let (m, s) = lognormal_mu_sigma(mean, std_dev);
                within("log-normal μ", m, mu, se(sigma * sigma) + 1e-12);
                within("log-normal σ", s, sigma, se(sigma * sigma / 2.0) + 1e-12);
            }
            d => panic!("{d:?}"),
        }

        // Weibull: Var(k̂) ≈ 0.608 k²/n, Var(λ̂) ≈ 1.109 λ²/(k² n).
        let (k, lambda) = (1.7, 3.0);
        let x = sample(
            &Dist::Weibull {
                shape: k,
                scale: lambda,
            },
            n,
            5,
        );
        match only(&x, Family::Weibull) {
            Dist::Weibull { shape, scale } => {
                within("Weibull shape", shape, k, se(0.608 * k * k));
                within(
                    "Weibull scale",
                    scale,
                    lambda,
                    se(1.109 * lambda * lambda / (k * k)),
                );
            }
            d => panic!("{d:?}"),
        }

        // Normal: σ²/n and σ²/(2n).
        let x = sample(
            &Dist::Normal {
                mean: 10.0,
                std_dev: 2.0,
            },
            n,
            6,
        );
        match only(&x, Family::Normal) {
            Dist::Normal { mean, std_dev } => {
                within("normal mean", mean, 10.0, se(4.0));
                within("normal sd", std_dev, 2.0, se(2.0));
            }
            d => panic!("{d:?}"),
        }

        // Uniform: each end is within a few range/n of the truth.
        let x = sample(&Dist::Uniform { min: 2.0, max: 5.0 }, n, 7);
        match only(&x, Family::Uniform) {
            Dist::Uniform { min, max } => {
                within("uniform min", min, 2.0, 4.0 * 3.0 / nf);
                within("uniform max", max, 5.0, 4.0 * 3.0 / nf);
            }
            d => panic!("{d:?}"),
        }

        // Triangular: the ends as for the uniform (the density near them is small, so they are
        // slower to pin down); the mode, an order statistic, to within 2% of the range.
        let x = sample(
            &Dist::Triangular {
                min: 1.0,
                mode: 2.0,
                max: 5.0,
            },
            n,
            8,
        );
        match only(&x, Family::Triangular) {
            Dist::Triangular { min, mode, max } => {
                within("triangular min", min, 1.0, 0.05);
                within("triangular max", max, 5.0, 0.05);
                within("triangular mode", mode, 2.0, 0.08);
            }
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn ranking_picks_the_generating_family_on_clear_cases() {
        // Not the exponential: the gamma and the Weibull contain it (shape 1), so on exponential
        // data their extra parameter usually fits the sample's noise a little better and A²
        // ranks them first, with nearly the same A² (checked below).
        let cases = [
            (
                Dist::LogNormal {
                    mean: 2.0,
                    std_dev: 2.5,
                },
                Family::LogNormal,
            ),
            (
                Dist::Weibull {
                    shape: 0.6,
                    scale: 2.0,
                },
                Family::Weibull,
            ),
            (Dist::Uniform { min: 1.0, max: 3.0 }, Family::Uniform),
            (
                Dist::Triangular {
                    min: 1.0,
                    mode: 1.5,
                    max: 6.0,
                },
                Family::Triangular,
            ),
        ];
        for (d, want) in cases {
            for seed in 1..=5 {
                let x = sample(&d, 2_000, seed);
                let fits = fit(&x, &Family::ALL, MIN_COUNT);
                let best = fits
                    .as_ref()
                    .ok()
                    .and_then(|f| f.candidates.first())
                    .map(|c| c.family);
                assert_eq!(best, Some(want), "{d:?}, seed {seed}: {fits:?}");
            }
        }
        for seed in 1..=5 {
            let x = sample(&Dist::Exponential { mean: 1.0 }, 2_000, seed);
            let fits = fit(&x, &Family::ALL, MIN_COUNT).unwrap_or_else(|e| panic!("{e}"));
            let a2 = |f: Family| {
                fits.candidates
                    .iter()
                    .find(|c| c.family == f)
                    .map_or(f64::NAN, |c| c.anderson_darling)
            };
            let top3: Vec<Family> = fits.candidates.iter().take(3).map(|c| c.family).collect();
            assert!(top3.contains(&Family::Exponential), "{fits:?}");
            assert!(a2(Family::Exponential) - a2(top3[0]) < 0.5, "{fits:?}");
        }
    }

    #[test]
    fn data_that_cannot_be_fitted_are_refused_or_families_skipped() {
        let few = [1.0; 19];
        assert_eq!(
            fit(&few, &Family::ALL, MIN_COUNT),
            Err(DataError::TooFew {
                count: 19,
                needed: 20
            })
        );
        // A smaller minimum is honoured down to the floor.
        let x = sample(&Dist::Exponential { mean: 1.0 }, 6, 9);
        assert!(fit(&x, &Family::ALL, 6).is_ok());
        assert_eq!(
            fit(&x[..4], &Family::ALL, 1),
            Err(DataError::TooFew {
                count: 4,
                needed: MIN_COUNT_FLOOR
            })
        );
        let mut bad = sample(&Dist::Exponential { mean: 1.0 }, 30, 10);
        bad[7] = f64::NAN;
        assert_eq!(
            fit(&bad, &Family::ALL, MIN_COUNT),
            Err(DataError::NotFinite { index: 7 })
        );
        assert_eq!(
            fit(&[3.0; 25], &Family::ALL, MIN_COUNT),
            Err(DataError::AllEqual)
        );

        // A zero rules out the families that need positive values, with the reason.
        let mut x = sample(&Dist::Uniform { min: 0.0, max: 2.0 }, 40, 11);
        x[0] = 0.0;
        let fits = fit(&x, &Family::ALL, MIN_COUNT).unwrap_or_else(|e| panic!("{e}"));
        let skipped: Vec<_> = fits.skipped.iter().map(|s| (s.family, s.reason)).collect();
        for f in [
            Family::Exponential,
            Family::Gamma,
            Family::Erlang,
            Family::LogNormal,
            Family::Weibull,
        ] {
            assert!(skipped.contains(&(f, Skip::NotPositive)), "{skipped:?}");
        }
        assert_eq!(fits.candidates.len(), 3);
        assert!(
            fits.candidates
                .iter()
                .all(|c| c.anderson_darling.is_finite())
        );

        // The Erlang whose shape rounds to 1 is the exponential, so it is not offered twice.
        let x = sample(&Dist::Exponential { mean: 1.0 }, 500, 12);
        let fits = fit(&x, &Family::ALL, MIN_COUNT).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            fits.skipped.contains(&Skipped {
                family: Family::Erlang,
                reason: Skip::SameAsExponential
            }),
            "{fits:?}"
        );
        // Asked for on its own, it is given.
        assert!(matches!(
            only(&x, Family::Erlang),
            Dist::Erlang { k: 1, .. }
        ));
    }

    #[test]
    fn names_round_trip() {
        for f in Family::ALL {
            assert_eq!(Family::from_name(f.name()), Some(f));
        }
        assert_eq!(Family::from_name("lognormal"), Some(Family::LogNormal));
        assert_eq!(Family::from_name("beta"), None);
    }
}
