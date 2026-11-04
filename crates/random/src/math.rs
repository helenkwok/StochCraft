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
    }
}
