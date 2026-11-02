//! Deterministic random numbers for StochCraft.
//!
//! The generator is xoshiro256** (Blackman and Vigna, "Scrambled linear pseudorandom number
//! generators", ACM TOMS 47(4), 2021), seeded by expanding a `u64` with SplitMix64 as the paper
//! recommends. Both are implemented here from the published description. Everything is `u64`
//! and `f64` arithmetic, so a seed gives the same stream on native, wasm32 and wasm64.

/// SplitMix64: used only to expand a seed into generator state.
#[derive(Debug, Clone)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// The xoshiro256** generator.
#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// A generator seeded from `seed` through SplitMix64 (never the all-zero state).
    pub fn new(seed: u64) -> Self {
        let mut sm = SplitMix64::new(seed);
        let s = [sm.next_u64(), sm.next_u64(), sm.next_u64(), sm.next_u64()];
        Self { s }
    }

    pub fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = self.s;
        let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s1 << 17;
        let mut s = [s0, s1, s2 ^ s0, s3 ^ s1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        self.s = s;
        result
    }

    /// Uniform on [0, 1), from the top 53 bits.
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform on (0, 1]: never zero, so safe under `ln`.
    pub fn uniform_open0(&mut self) -> f64 {
        ((self.next_u64() >> 11) + 1) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Exponential with the given mean, by inversion. A non-positive or non-finite mean gives 0.
    pub fn exponential(&mut self, mean: f64) -> f64 {
        if !(mean.is_finite() && mean > 0.0) {
            return 0.0;
        }
        -mean * self.uniform_open0().ln()
    }

    /// An independent generator for a sub-stream (a replication or a process), derived from this
    /// one's next output.
    pub fn fork(&mut self) -> Rng {
        Rng::new(self.next_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_matches_the_published_sequence_for_seed_zero() {
        // The first outputs of SplitMix64 from state 0 (widely published test values).
        let mut sm = SplitMix64::new(0);
        assert_eq!(sm.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(sm.next_u64(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(sm.next_u64(), 0x06c4_5d18_8009_454f);
    }

    #[test]
    fn same_seed_same_stream() {
        let (mut a, mut b) = (Rng::new(42), Rng::new(42));
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    #[test]
    fn uniform_ranges() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let u = r.uniform();
            assert!((0.0..1.0).contains(&u));
            let v = r.uniform_open0();
            assert!(v > 0.0 && v <= 1.0);
        }
    }

    #[test]
    fn exponential_mean_is_close() {
        let mut r = Rng::new(123);
        let n = 200_000;
        let mean = (0..n).map(|_| r.exponential(2.5)).sum::<f64>() / n as f64;
        // Standard error is 2.5 / sqrt(n) ~ 0.0056; allow five of them.
        assert!((mean - 2.5).abs() < 0.03, "mean {mean}");
        assert_eq!(r.exponential(0.0), 0.0);
        assert_eq!(r.exponential(f64::NAN), 0.0);
    }
}
