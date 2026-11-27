//! The generator of `stochcraft-random` as it was at main d4b0441a, before a stream could read a
//! file of random bytes: its xoshiro256** and SplitMix64 copied unchanged (the distributions left
//! out), for the `rng` benchmark to time the current crate against.

pub mod math;

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
        -mean * math::ln(self.uniform_open0())
    }

    /// Uniform on [min, max). Returns `min` if the range is empty or not finite.
    pub fn uniform_range(&mut self, min: f64, max: f64) -> f64 {
        if !(min.is_finite() && max.is_finite() && max > min) {
            return min;
        }
        min + (max - min) * self.uniform()
    }

    /// Triangular on [min, max] with peak at `mode`, by inverting its distribution function.
    /// Returns `min` unless min <= mode <= max with min < max, all finite.
    pub fn triangular(&mut self, min: f64, mode: f64, max: f64) -> f64 {
        let valid = [min, mode, max].iter().all(|v| v.is_finite())
            && min <= mode
            && mode <= max
            && min < max;
        if !valid {
            return min;
        }
        let u = self.uniform();
        let split = (mode - min) / (max - min);
        if u < split {
            min + (u * (max - min) * (mode - min)).sqrt()
        } else {
            max - ((1.0 - u) * (max - min) * (max - mode)).sqrt()
        }
    }

    /// An independent generator for a sub-stream (a replication or a process), derived from this
    /// one's next output.
    pub fn fork(&mut self) -> Rng {
        Rng::new(self.next_u64())
    }

    /// The generator for the sub-stream named `key` under `base` (one draw of a parent stream).
    ///
    /// Unlike [`Rng::fork`], it depends only on `base` and `key`, never on how many streams were
    /// made before it, so adding or removing a stream leaves every other one as it was. The key
    /// is hashed with 64-bit FNV-1a (Fowler, Noll and Vo; offset basis 0xcbf29ce484222325, prime
    /// 0x100000001b3), and the hash and `base` are mixed through SplitMix64 before seeding.
    pub fn keyed(base: u64, key: &str) -> Rng {
        let mixed = SplitMix64::new(fnv1a64(key)).next_u64();
        Rng::new(SplitMix64::new(base ^ mixed).next_u64())
    }
}

/// 64-bit FNV-1a of `s`'s bytes.
fn fnv1a64(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

