//! Critical values for confidence intervals at any level: Student's t distribution, computed
//! rather than looked up, and Bonferroni's rule for several intervals that must hold together.
//!
//! - [`t_quantile`] and [`t_upper_tail`]: Student's t distribution for any degrees of freedom,
//!   from the regularized incomplete beta function, P(T > t) = I_x(df/2, 1/2) / 2 with
//!   x = df/(df + t²) (Abramowitz and Stegun, *Handbook of Mathematical Functions*, 26.5.27 and
//!   26.7.1), evaluated by its continued fraction (26.5.8) and solved for t by bisection.
//! - [`t_two_sided`]: the t value of a two-sided interval at a confidence level. At 95% it is
//!   [`crate::t_critical_95`], the table every result has used so far, so a 95% interval is the
//!   same to the last bit as before; at any other level it is computed.
//! - [`bonferroni`]: the level each of k intervals needs for all k to hold together with at
//!   least the overall level, 1 − (1 − level)/k (the Bonferroni inequality; A. M. Law,
//!   *Simulation Modeling and Analysis*, 5th ed., McGraw-Hill, 2015, section 9.7, and for
//!   comparing systems section 10.3).

/// The confidence levels an experiment offers.
pub const LEVELS: [f64; 3] = [0.90, 0.95, 0.99];

/// The confidence level results use unless an experiment says otherwise.
pub const DEFAULT_LEVEL: f64 = 0.95;

/// Whether `level` is one of [`LEVELS`].
pub fn is_offered(level: f64) -> bool {
    LEVELS.contains(&level)
}

/// The natural logarithm of the beta function B(a, b).
fn ln_beta(a: f64, b: f64) -> f64 {
    libm::lgamma(a) + libm::lgamma(b) - libm::lgamma(a + b)
}

/// The continued fraction of the incomplete beta function (Abramowitz and Stegun 26.5.8),
/// evaluated by the modified Lentz method; it converges fast for x < (a + 1)/(a + b + 2).
fn beta_fraction(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    const EPS: f64 = 1e-16;
    let guard = |v: f64| if v.abs() < TINY { TINY } else { v };
    let mut c = 1.0;
    let mut d = 1.0 / guard(1.0 - (a + b) * x / (a + 1.0));
    let mut f = d;
    for m in 1..=1000u32 {
        let m = f64::from(m);
        // The even step, d(2m).
        let even = m * (b - m) * x / ((a + 2.0 * m - 1.0) * (a + 2.0 * m));
        d = 1.0 / guard(1.0 + even * d);
        c = guard(1.0 + even / c);
        f *= d * c;
        // The odd step, d(2m + 1).
        let odd = -(a + m) * (a + b + m) * x / ((a + 2.0 * m) * (a + 2.0 * m + 1.0));
        d = 1.0 / guard(1.0 + odd * d);
        c = guard(1.0 + odd / c);
        let step = d * c;
        f *= step;
        if (step - 1.0).abs() < EPS {
            break;
        }
    }
    f
}

/// The regularized incomplete beta function I_x(a, b) for a, b > 0 and 0 ≤ x ≤ 1. `None`
/// outside that domain.
pub fn incomplete_beta(a: f64, b: f64, x: f64) -> Option<f64> {
    if !(a > 0.0 && b > 0.0 && a.is_finite() && b.is_finite() && (0.0..=1.0).contains(&x)) {
        return None;
    }
    if x == 0.0 || x == 1.0 {
        return Some(x);
    }
    let front = libm::exp(a * libm::log(x) + b * libm::log(1.0 - x) - ln_beta(a, b));
    // The fraction converges fast on one side of the mean; on the other, I_x(a, b) =
    // 1 − I_{1−x}(b, a).
    Some(if x < (a + 1.0) / (a + b + 2.0) {
        front * beta_fraction(a, b, x) / a
    } else {
        1.0 - front * beta_fraction(b, a, 1.0 - x) / b
    })
}

/// P(T > t) for Student's t distribution with `df` degrees of freedom (any positive real), for
/// t ≥ 0; by symmetry 1 − P(T > −t) below 0. `None` for no degrees of freedom or a t that is
/// not a number.
pub fn t_upper_tail(t: f64, df: f64) -> Option<f64> {
    if df.is_nan() || df <= 0.0 || t.is_nan() {
        return None;
    }
    if t.is_infinite() {
        return Some(if t > 0.0 { 0.0 } else { 1.0 });
    }
    let x = df / (df + t * t);
    let half = 0.5 * incomplete_beta(0.5 * df, 0.5, x)?;
    Some(if t >= 0.0 { half } else { 1.0 - half })
}

/// The t with P(T > t) = `tail` for 0 < `tail` ≤ 1/2 (t ≥ 0), by bisection on
/// [`t_upper_tail`], which falls as t grows.
fn upper_quantile(tail: f64, df: f64) -> Option<f64> {
    if tail == 0.5 {
        return Some(0.0);
    }
    // A bracket: double the top until the tail beyond it is small enough.
    let mut hi = 1.0_f64;
    while t_upper_tail(hi, df)? > tail {
        hi *= 2.0;
        if !hi.is_finite() {
            return None;
        }
    }
    let mut lo = 0.0_f64;
    for _ in 0..2000 {
        let mid = 0.5 * (lo + hi);
        if mid <= lo || mid >= hi {
            break;
        }
        if t_upper_tail(mid, df)? > tail {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// The `p` quantile of Student's t distribution with `df` degrees of freedom: the t with
/// P(T ≤ t) = p, for 0 < p < 1. `None` outside that range or for no degrees of freedom.
pub fn t_quantile(p: f64, df: f64) -> Option<f64> {
    if !(p > 0.0 && p < 1.0 && df > 0.0) {
        return None;
    }
    if p >= 0.5 {
        upper_quantile(1.0 - p, df)
    } else {
        upper_quantile(p, df).map(|t| -t)
    }
}

/// The t value of a two-sided interval at confidence `level` (0.95 for 95%) with `df` degrees of
/// freedom: P(|T| ≤ t) = level. At exactly 95% it is [`crate::t_critical_95`], the table results
/// have always used, so the default leaves every interval unchanged; at any other level it is
/// computed from the upper tail (1 − level)/2, without subtracting from 1. `None` for no degrees
/// of freedom or a level not strictly between 0 and 1.
pub fn t_two_sided(level: f64, df: u64) -> Option<f64> {
    if !(level > 0.0 && level < 1.0) || df == 0 {
        return None;
    }
    if level == DEFAULT_LEVEL {
        return crate::t_critical_95(df);
    }
    upper_quantile(0.5 * (1.0 - level), df as f64)
}

/// The confidence level each of `k` intervals needs so that all `k` hold together with
/// probability at least `level` (Bonferroni): 1 − (1 − level)/k, so three intervals for 90%
/// together are each at 96.667%. One interval is at `level` itself. `None` for no intervals or
/// a level not strictly between 0 and 1.
pub fn bonferroni(level: f64, k: u64) -> Option<f64> {
    if !(level > 0.0 && level < 1.0) || k == 0 {
        return None;
    }
    if k == 1 {
        return Some(level);
    }
    Some(1.0 - (1.0 - level) / k as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two-sided 95% values computed outside StochCraft, as kept in `lib.rs`'s own test (the
    /// exact series for integer df, Abramowitz and Stegun 26.7.3, and the incomplete beta
    /// function, which agreed to 4e-13).
    const REFERENCE_95: [(u64, f64); 9] = [
        (1, 12.706205),
        (30, 2.042272),
        (31, 2.039513),
        (35, 2.030108),
        (39, 2.022691),
        (120, 1.979930),
        (121, 1.979764),
        (500, 1.964720),
        (999, 1.962341),
    ];

    #[test]
    fn the_incomplete_beta_function_matches_closed_forms() {
        // I_x(1, 1) = x; I_x(a, 1) = x^a; I_x(1, b) = 1 − (1 − x)^b.
        for x in [0.0, 0.1, 0.37, 0.5, 0.9, 1.0] {
            assert!(incomplete_beta(1.0, 1.0, x).is_some_and(|v| (v - x).abs() < 1e-14));
            let x3 = x * x * x;
            assert!(incomplete_beta(3.0, 1.0, x).is_some_and(|v| (v - x3).abs() < 1e-14));
            let y = 1.0 - x;
            let y4 = y * y * y * y;
            assert!(incomplete_beta(1.0, 4.0, x).is_some_and(|v| (v - (1.0 - y4)).abs() < 1e-14));
        }
        // Symmetry: I_x(a, b) = 1 − I_{1−x}(b, a).
        let (a, b, x) = (2.5, 7.25, 0.3);
        let sum = incomplete_beta(a, b, x).unwrap_or(f64::NAN)
            + incomplete_beta(b, a, 1.0 - x).unwrap_or(f64::NAN);
        assert!((sum - 1.0).abs() < 1e-14, "{sum}");
        assert_eq!(incomplete_beta(0.0, 1.0, 0.5), None);
        assert_eq!(incomplete_beta(1.0, 1.0, 1.5), None);
    }

    #[test]
    fn the_t_distribution_with_one_and_two_degrees_of_freedom_is_in_closed_form() {
        // df 1 is the Cauchy distribution: P(T > t) = 1/2 − atan(t)/π.
        // df 2: P(T > t) = (1 − t/√(2 + t²))/2.
        for t in [0.0, 0.3, 1.0, 2.5, 12.0, 400.0] {
            let cauchy = 0.5 - libm::atan(t) / std::f64::consts::PI;
            assert!(t_upper_tail(t, 1.0).is_some_and(|p| (p - cauchy).abs() < 1e-13));
            let two = 0.5 * (1.0 - t / (2.0 + t * t).sqrt());
            assert!(t_upper_tail(t, 2.0).is_some_and(|p| (p - two).abs() < 1e-13));
        }
        // Their quantiles in closed form: tan(π(p − 1/2)) and (2p − 1)√(2 / (4p(1 − p))).
        for p in [0.6, 0.9, 0.975, 0.995, 0.9995] {
            let cauchy = libm::tan(std::f64::consts::PI * (p - 0.5));
            assert!(t_quantile(p, 1.0).is_some_and(|t| (t - cauchy).abs() < 1e-9 * cauchy));
            let two = (2.0 * p - 1.0) * (2.0 / (4.0 * p * (1.0 - p))).sqrt();
            assert!(t_quantile(p, 2.0).is_some_and(|t| (t - two).abs() < 1e-10 * two));
            assert!(t_quantile(1.0 - p, 2.0).is_some_and(|t| (t + two).abs() < 1e-10 * two));
        }
        assert_eq!(t_quantile(0.5, 7.0), Some(0.0));
        assert_eq!(t_quantile(1.0, 7.0), None);
        assert_eq!(t_quantile(0.9, 0.0), None);
    }

    #[test]
    fn computed_quantiles_agree_with_values_computed_independently() {
        for (df, want) in REFERENCE_95 {
            let got = t_quantile(0.975, df as f64);
            assert!(
                got.is_some_and(|t| (t - want).abs() < 5e-7),
                "df {df}: {got:?} vs {want}"
            );
        }
        // Many degrees of freedom tend to the normal: 1.959964 for 0.975.
        assert!(t_quantile(0.975, 1e7).is_some_and(|t| (t - 1.959964).abs() < 1e-5));
    }

    /// Law, 5th ed., Table T.1 (p. 722), gives t at γ = 0.90, 0.95, 0.975, 0.9833 and 0.995 to
    /// three decimals; the computed values round to it. (Its 0.9833 column is that value exactly,
    /// not 1 − 1/60: at 9 df 1 − 1/60 gives 2.5096, the column 2.508.)
    #[test]
    fn computed_quantiles_round_to_the_published_table() {
        let rows: [(u64, [f64; 5]); 6] = [
            (1, [3.078, 6.314, 12.706, 19.043, 63.657]),
            (4, [1.533, 2.132, 2.776, 3.184, 4.604]),
            (9, [1.383, 1.833, 2.262, 2.508, 3.250]),
            (19, [1.328, 1.729, 2.093, 2.293, 2.861]),
            (30, [1.310, 1.697, 2.042, 2.230, 2.750]),
            (100, [1.290, 1.660, 1.984, 2.157, 2.626]),
        ];
        let gammas = [0.90, 0.95, 0.975, 0.9833, 0.995];
        for (df, row) in rows {
            for (p, want) in gammas.into_iter().zip(row) {
                let got = t_quantile(p, df as f64).unwrap_or(f64::NAN);
                assert!(
                    (got - want).abs() <= 5e-4 + 1e-12,
                    "df {df}, p {p}: {got} vs {want}"
                );
            }
        }
    }

    #[test]
    fn ninety_five_percent_is_the_table_exactly_and_other_levels_are_computed() {
        for df in [1, 2, 7, 9, 30, 31, 48, 120, 121, 999, 1_000_000] {
            assert_eq!(t_two_sided(0.95, df), crate::t_critical_95(df), "df {df}");
        }
        // 90% is the 0.95 quantile, 99% the 0.995.
        for df in [1, 4, 9, 30, 120] {
            let (a, b) = (
                t_two_sided(0.90, df).unwrap_or(f64::NAN),
                t_quantile(0.95, df as f64).unwrap_or(f64::NAN),
            );
            assert!((a - b).abs() < 1e-12 * b, "{a} {b}");
            let a = t_two_sided(0.99, df).unwrap_or(f64::NAN);
            let b = t_quantile(0.995, df as f64).unwrap_or(f64::NAN);
            assert!((a - b).abs() < 1e-12 * b, "{a} {b}");
        }
        assert_eq!(t_two_sided(0.9, 0), None);
        assert_eq!(t_two_sided(1.0, 5), None);
        assert_eq!(t_two_sided(0.0, 5), None);
    }

    #[test]
    fn bonferroni_divides_the_chance_of_missing_among_the_intervals() {
        // Three measures at 90% together: 96.667% each (Law, Example 9.34).
        assert!(bonferroni(0.90, 3).is_some_and(|l| (l - (1.0 - 0.1 / 3.0)).abs() < 1e-15));
        // Four comparisons with a standard at 90%: 97.5% each; all ten pairs of five systems at
        // 90%: 99% each (Law, Tables 10.6 and 10.7).
        assert!(bonferroni(0.90, 4).is_some_and(|l| (l - 0.975).abs() < 1e-15));
        assert!(bonferroni(0.90, 10).is_some_and(|l| (l - 0.99).abs() < 1e-15));
        assert_eq!(bonferroni(0.95, 1), Some(0.95));
        assert_eq!(bonferroni(0.95, 0), None);
        assert_eq!(bonferroni(1.0, 3), None);
        assert!(is_offered(0.9) && is_offered(0.95) && is_offered(0.99) && !is_offered(0.8));
    }
}
