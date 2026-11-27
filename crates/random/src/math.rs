//! The elementary functions StochCraft samples with, from the pure-Rust `libm` (a port of musl's),
//! so that a seed gives bit-identical samples on every platform and target. The methods of `f64`
//! (`x.ln()` and so on) call the platform's maths library instead, whose last bit can differ
//! between systems; the workspace's `clippy.toml` forbids them.

/// Natural logarithm.
#[inline]
pub fn ln(x: f64) -> f64 {
    libm::log(x)
}

/// e to the power `x`.
#[inline]
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

/// `x` to the power `y`.
#[inline]
pub fn powf(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}

#[inline]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}

#[inline]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}

/// The tangent of `x` (radians).
#[inline]
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}

/// The angle of the point `(x, y)` from the `+x` axis, in radians, from −π to π.
#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// The error function.
#[inline]
pub fn erf(x: f64) -> f64 {
    libm::erf(x)
}

/// The complementary error function, 1 − erf(x), accurate far into the tail.
#[inline]
pub fn erfc(x: f64) -> f64 {
    libm::erfc(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agree_with_known_values() {
        assert_eq!(ln(1.0), 0.0);
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(powf(2.0, 10.0), 1024.0);
        assert!((ln(std::f64::consts::E) - 1.0).abs() <= f64::EPSILON);
        assert!((sin(std::f64::consts::FRAC_PI_2) - 1.0).abs() <= f64::EPSILON);
        assert!(cos(std::f64::consts::PI) + 1.0 <= f64::EPSILON);
        assert_eq!(erf(0.0), 0.0);
        assert_eq!(erfc(0.0), 1.0);
        assert!((erf(1.0) - 0.842_700_792_949_714_9).abs() <= f64::EPSILON);
        assert!((erfc(5.0) / 1.537_459_794_428_034_8e-12 - 1.0).abs() <= 1e-14);
    }
}
