//! A seed from fetched bytes.

/// How the bytes of a draw become a seed, as the run record states it.
pub const RULE: &str = "the SHA-256 of every byte the draw fetched (a beacon pulse's 512 bits, or a service's answer), its first 8 bytes read as a little-endian u64, keeping the low 53 bits so the seed is exact wherever numbers are 64-bit floating point (the window, JavaScript)";

/// The largest seed a draw gives: 2^53 − 1.
pub const MAX: u64 = (1 << 53) - 1;

/// The seed `bytes` give ([`RULE`]).
pub fn from_bytes(bytes: &[u8]) -> u64 {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(bytes);
    let mut first = [0u8; 8];
    first.copy_from_slice(&d[..8]);
    u64::from_le_bytes(first) & MAX
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)] // A failed step fails the test.
mod tests {
    #[test]
    fn a_seed_is_53_bits_and_depends_on_every_byte() {
        let a = super::from_bytes(&[0u8; 64]);
        let mut other = [0u8; 64];
        other[63] = 1;
        assert_ne!(a, super::from_bytes(&other));
        assert!(a <= super::MAX);
        // SHA-256 of 64 zero bytes starts f5 a5 fd 42 d1 6a 20 30.
        assert_eq!(
            a,
            u64::from_le_bytes([0xf5, 0xa5, 0xfd, 0x42, 0xd1, 0x6a, 0x20, 0x30]) & super::MAX
        );
    }
}
