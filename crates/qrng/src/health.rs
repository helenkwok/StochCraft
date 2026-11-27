//! A quick health check on fetched bytes: the frequency (monobit) test and the runs test of
//! NIST SP 800-22 Rev. 1a (sections 2.1 and 2.3), and a check for 32-byte blocks that repeat.
//! It warns and never blocks: a source that fails it may still be used, and passing it says
//! little (it would pass a good pseudo-random generator, and catches only gross faults, such as
//! a service answering with the same bytes twice or bytes of a constant).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use stochcraft_random::math::erfc;

/// The significance level the tests are judged at (SP 800-22's usual 0.01).
pub const ALPHA: f64 = 0.01;

/// The fewest bits the two tests are run on (SP 800-22 recommends at least 100).
pub const MIN_BITS: u64 = 100;

/// The size of the blocks checked for repeats.
pub const BLOCK: usize = 32;

/// What the check found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub bits: u64,
    /// The frequency (monobit) test's P-value; `None` with too few bits.
    pub frequency_p: Option<f64>,
    /// The runs test's P-value; `None` with too few bits. 0 when the frequency prerequisite
    /// fails.
    pub runs_p: Option<f64>,
    /// 32-byte blocks (at 32-byte boundaries) that appeared before.
    pub repeated_blocks: u64,
    /// No test failed at [`ALPHA`] and no block repeated.
    pub passed: bool,
    /// What failed, in plain words.
    pub warnings: Vec<String>,
}

fn bit(bytes: &[u8], i: usize) -> bool {
    (bytes[i / 8] >> (7 - i % 8)) & 1 == 1
}

/// SP 800-22 §2.1 over the first `n` bits of `bytes` (most significant bit of each byte first).
pub fn frequency_p(bytes: &[u8], n: usize) -> f64 {
    let ones = (0..n).filter(|&i| bit(bytes, i)).count() as f64;
    let s = 2.0 * ones - n as f64;
    let s_obs = s.abs() / (n as f64).sqrt();
    erfc(s_obs / std::f64::consts::SQRT_2)
}

/// SP 800-22 §2.3 over the first `n` bits of `bytes`. 0 when the proportion of ones is so far
/// from a half that the frequency test would fail (its prerequisite, τ = 2/√n).
pub fn runs_p(bytes: &[u8], n: usize) -> f64 {
    let nf = n as f64;
    let pi = (0..n).filter(|&i| bit(bytes, i)).count() as f64 / nf;
    if (pi - 0.5).abs() >= 2.0 / nf.sqrt() {
        return 0.0;
    }
    let changes = (1..n)
        .filter(|&k| bit(bytes, k) != bit(bytes, k - 1))
        .count() as f64;
    let v = changes + 1.0;
    let q = pi * (1.0 - pi);
    erfc((v - 2.0 * nf * q).abs() / (2.0 * (2.0 * nf).sqrt() * q))
}

/// The check on `bytes`.
pub fn check(bytes: &[u8]) -> Health {
    let n = bytes.len().saturating_mul(8);
    let bits = n as u64;
    let mut warnings = Vec::new();
    let (frequency, runs) = if bits >= MIN_BITS {
        (Some(frequency_p(bytes, n)), Some(runs_p(bytes, n)))
    } else {
        warnings.push(format!(
            "too few bytes to test ({} bits; the tests need at least {MIN_BITS})",
            bits
        ));
        (None, None)
    };
    if frequency.is_some_and(|p| p < ALPHA) {
        warnings.push(
            "the frequency test failed: the bytes have too many ones or too many zeros".into(),
        );
    }
    if runs.is_some_and(|p| p < ALPHA) {
        warnings.push(
            "the runs test failed: the bits change from 0 to 1 too often or too seldom".into(),
        );
    }
    let mut seen = HashSet::new();
    let repeated = bytes
        .chunks_exact(BLOCK)
        .filter(|b| !seen.insert(*b))
        .count() as u64;
    if repeated > 0 {
        warnings.push(format!(
            "{repeated} block(s) of {BLOCK} bytes repeat earlier ones: the source may have answered with the same bytes twice"
        ));
    }
    let passed = frequency.is_none_or(|p| p >= ALPHA)
        && runs.is_none_or(|p| p >= ALPHA)
        && repeated == 0
        && bits >= MIN_BITS;
    Health {
        bits,
        frequency_p: frequency,
        runs_p: runs,
        repeated_blocks: repeated,
        passed,
        warnings,
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)] // A failed step fails the test.
mod tests {
    use super::*;

    /// Bits written as text, packed most significant first.
    fn pack(bits: &str) -> Vec<u8> {
        let b: Vec<bool> = bits.bytes().map(|c| c == b'1').collect();
        b.chunks(8)
            .map(|c| {
                c.iter()
                    .enumerate()
                    .fold(0u8, |a, (i, &x)| a | (u8::from(x) << (7 - i)))
            })
            .collect()
    }

    /// The worked examples of SP 800-22 §2.1.4 (ε = 1011010101, P-value = 0.527089) and §2.3.4
    /// (ε = 1001101011, P-value = 0.147232).
    #[test]
    fn the_standards_worked_examples() {
        let f = frequency_p(&pack("1011010101"), 10);
        assert!((f - 0.527_089).abs() < 1e-6, "{f}");
        let r = runs_p(&pack("1001101011"), 10);
        assert!((r - 0.147_232).abs() < 1e-6, "{r}");
    }

    #[test]
    fn constant_and_repeated_bytes_fail_and_good_bytes_pass() {
        let zeros = check(&[0u8; 256]);
        assert!(!zeros.passed && zeros.frequency_p == Some(0.0) && zeros.repeated_blocks == 7);
        let mut r = stochcraft_random::Rng::new(7);
        let good: Vec<u8> = (0..512).flat_map(|_| r.next_u64().to_le_bytes()).collect();
        let h = check(&good);
        assert!(h.passed, "{h:?}");
        let mut twice = good.clone();
        twice.extend_from_slice(&good);
        assert_eq!(check(&twice).repeated_blocks, 128);
        assert!(!check(&[1, 2, 3]).passed);
    }
}
