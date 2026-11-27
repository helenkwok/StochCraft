//! Rounding a result to what its interval supports, the rule every table of results uses: the
//! 95% half-width to one significant figure (two where that figure would be a 1), and the mean
//! to the same decimal place, so 38.4 ± 12.3 reads "38 ± 12" and 2.04 ± 0.61 reads "2.0 ± 0.6".
//!
//! The numbers only: how a number is then written (the decimal mark, grouping of thousands) is
//! the caller's. [`fixed`] writes one plainly, with a point and no grouping, for files, the
//! command line and agents.
//!
//! Powers of ten are found by repeated scaling and multiplication rather than `log10` and
//! `powi`, which can differ between platforms, so the result is the same everywhere.

/// The power of ten of `v`'s first significant figure (`v` finite and not zero): 0 for 3.2, 2
/// for 380, −2 for 0.012.
pub fn magnitude(v: f64) -> i32 {
    let mut m = 0i32;
    let mut a = v.abs();
    while a >= 10.0 && m < 400 {
        a /= 10.0;
        m += 1;
    }
    while a < 1.0 && m > -400 {
        a *= 10.0;
        m -= 1;
    }
    m
}

/// Ten to the power `e`, by repeated multiplication; exact for the few places a table rounds to.
pub fn pow10(e: i32) -> f64 {
    let f = (0..e.unsigned_abs().min(400)).fold(1.0, |f, _| f * 10.0);
    if e < 0 { 1.0 / f } else { f }
}

/// `v` rounded to `places` decimal places (to tens, hundreds… where `places` is negative).
pub fn round_to(v: f64, places: i32) -> f64 {
    let f = pow10(places);
    (v * f).round() / f
}

/// `v` to `figures` significant figures (60.0014 → 60.0 to three), for a number with no interval
/// to round it by.
pub fn round_sig(v: f64, figures: i32) -> f64 {
    if !v.is_finite() || v == 0.0 {
        return v;
    }
    round_to(v, figures - 1 - magnitude(v))
}

/// The decimal places a mean with the 95% half-width `half_width` is shown to (negative for
/// tens, hundreds…): the place of the half-width's first significant figure, one place further
/// where, once rounded there, it reads as a 1 (0.96 → 1.0, 0.14 → 0.14, 12.3 → 12). `None` for
/// a half-width that is zero (every replication gave the same value), negative or not finite,
/// which gives no place to round to.
pub fn places(half_width: f64) -> Option<i32> {
    if !(half_width.is_finite() && half_width > 0.0) {
        return None;
    }
    let first = |v: f64| -magnitude(v);
    let mut places = first(half_width);
    let hw = round_to(half_width, places);
    if (hw / pow10(magnitude(hw))).round() as i64 == 1 {
        places = first(hw) + 1;
    }
    Some(places)
}

/// `v` written with `places` decimals (none where `places` is negative: `v` is then expected to
/// be rounded to tens or more already), with a point and no grouping, and never as "-0".
pub fn fixed(v: f64, places: i32) -> String {
    let places = places.clamp(0, 12) as usize;
    let v = if v == 0.0 { 0.0 } else { v };
    let s = format!("{v:.places$}");
    // A small negative number that rounds to zero.
    match s.strip_prefix('-') {
        Some(rest) if rest.chars().all(|c| c == '0' || c == '.') => rest.to_owned(),
        _ => s,
    }
}

/// `mean` and `half_width` rounded together by [`places`] and written by [`fixed`]: ("38",
/// "12") for 38.4 ± 12.3. `None` where the half-width gives no place to round to.
pub fn interval(mean: f64, half_width: f64) -> Option<(String, String)> {
    if !mean.is_finite() {
        return None;
    }
    let p = places(half_width)?;
    Some((
        fixed(round_to(mean, p), p),
        fixed(round_to(half_width, p), p),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(m: f64, h: f64) -> String {
        interval(m, h).map_or_else(|| "none".to_owned(), |(m, h)| format!("{m} ± {h}"))
    }

    #[test]
    fn the_half_width_sets_the_place() {
        assert_eq!(r(80.696, 2.8373), "81 ± 3");
        assert_eq!(r(2.04, 0.61), "2.0 ± 0.6");
        assert_eq!(r(0.3042, 0.0813), "0.30 ± 0.08");
        // Two figures where the first is a 1, also once rounded (0.96 → 1.0).
        assert_eq!(r(38.4, 12.3), "38 ± 12");
        assert_eq!(r(5.03, 0.14), "5.03 ± 0.14");
        assert_eq!(r(7.37, 0.96), "7.4 ± 1.0");
        // Tens and hundreds where the interval is that wide; no grouping.
        assert_eq!(r(14990.0, 312.0), "15000 ± 300");
        assert_eq!(r(1234.0, 56.0), "1230 ± 60");
        // No "-0".
        assert_eq!(r(-0.004, 0.2), "0.0 ± 0.2");
        assert_eq!(r(-1.6, 0.67998), "-1.6 ± 0.7");
        // No place to round to.
        assert_eq!(r(12.5, 0.0), "none");
        assert_eq!(r(12.5, f64::INFINITY), "none");
        assert_eq!(r(f64::NAN, 1.0), "none");
    }

    #[test]
    fn places_and_powers() {
        assert_eq!(places(12.3), Some(0));
        assert_eq!(places(0.96), Some(1));
        assert_eq!(places(0.14), Some(2));
        assert_eq!(places(312.0), Some(-2));
        assert_eq!(places(0.0), None);
        assert_eq!(places(-1.0), None);
        assert_eq!(magnitude(380.0), 2);
        assert_eq!(magnitude(0.012), -2);
        assert_eq!(pow10(3), 1000.0);
        assert_eq!(pow10(-2), 0.01);
        assert_eq!(round_sig(60.0014, 3), 60.0);
        assert_eq!(fixed(-0.0, 2), "0.00");
        assert_eq!(fixed(1500.0, -2), "1500");
    }
}
