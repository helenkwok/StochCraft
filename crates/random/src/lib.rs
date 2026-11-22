//! Deterministic random numbers for StochCraft.
//!
//! The generator is xoshiro256** (Blackman and Vigna, "Scrambled linear pseudorandom number
//! generators", ACM TOMS 47(4), 2021), seeded by expanding a `u64` with SplitMix64 as the paper
//! recommends. Both are implemented here from the published description. Everything is `u64`
//! and `f64` arithmetic, so a seed gives the same stream on native, wasm32 and wasm64.
//!
//! [`Dist`] holds the distributions of the BPSim 2.0 specification (WfMC, 2016), each sampled
//! exactly (no approximations) by the published method named on its variant. [`estimate`]
//! fits them to experts' three-point estimates.
//!
//! Logarithms, exponentials, powers and trigonometric functions come from [`math`] (pure Rust),
//! never from the platform's maths library, whose results can differ in the last bit between
//! glibc, musl (wasm), macOS and Windows. Square roots are exactly rounded everywhere.

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
        Rng::new(keyed_seed(base, fnv1a64(key)))
    }
}

/// The seed of the stream whose key has FNV-1a hash `key_hash`, under `base`.
fn keyed_seed(base: u64, key_hash: u64) -> u64 {
    let mixed = SplitMix64::new(key_hash).next_u64();
    SplitMix64::new(base ^ mixed).next_u64()
}

/// 64-bit FNV-1a of `s`'s bytes.
fn fnv1a64(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

mod dist;
pub use dist::{Dist, DistError, Requirement, ln_binomial_pmf, ln_gamma, ln_poisson_pmf};
pub mod estimate;
pub mod fit;
pub use estimate::{Family, FitError, ThreePoint, Upper};

/// Machine-checked properties (Kani, `cargo kani -p stochcraft-random`; see docs/developers.md).
#[cfg(kani)]
mod proofs {
    use super::*;

    const GOLDEN: u64 = 0x9e37_79b9_7f4a_7c15;

    /// The inverse of `x -> x ^ (x >> s)` on `u64` (s >= 1).
    fn undo_xorshift(z: u64, s: u32) -> u64 {
        let mut x = z;
        let mut done = s;
        while done < 64 {
            x = z ^ (x >> s);
            done += s;
        }
        x
    }

    /// The inverse of multiplication by the odd `c` modulo 2^64 (Newton's iteration).
    fn mul_inverse(c: u64) -> u64 {
        let mut inv = c;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(c.wrapping_mul(inv)));
        }
        inv
    }

    /// The inverse of SplitMix64's output function (the part after the state's increment).
    fn unmix(out: u64) -> u64 {
        let z = undo_xorshift(out, 31);
        let z = z.wrapping_mul(mul_inverse(0x94d0_49bb_1331_11eb));
        let z = undo_xorshift(z, 27);
        let z = z.wrapping_mul(mul_inverse(0xbf58_476d_1ce4_e5b9));
        undo_xorshift(z, 30)
    }

    /// SplitMix64's output function is a bijection of its state, shown by an explicit inverse:
    /// the first output determines the seed, so different seeds never give the same first output.
    #[kani::proof]
    #[kani::solver(cadical)]
    #[kani::unwind(70)]
    fn splitmix64_first_output_determines_the_seed() {
        let seed: u64 = kani::any();
        let out = SplitMix64::new(seed).next_u64();
        assert_eq!(unmix(out), seed.wrapping_add(GOLDEN));
    }

    /// For a fixed key hash, the stream seed determines the base: different bases give
    /// different stream seeds.
    #[kani::proof]
    #[kani::solver(cadical)]
    #[kani::unwind(70)]
    fn keyed_seed_determines_the_base() {
        let (base, h): (u64, u64) = (kani::any(), kani::any());
        let mixed = SplitMix64::new(h).next_u64();
        assert_eq!(
            unmix(keyed_seed(base, h)).wrapping_sub(GOLDEN),
            base ^ mixed
        );
    }

    /// For a fixed base, different key hashes give different stream seeds. (Different key
    /// strings can still share a 64-bit FNV-1a hash; this is about the mixing after it.)
    #[kani::proof]
    #[kani::solver(cadical)]
    fn keyed_seed_is_injective_in_the_key_hash() {
        let (base, h1, h2): (u64, u64, u64) = (kani::any(), kani::any(), kani::any());
        kani::assume(h1 != h2);
        assert_ne!(keyed_seed(base, h1), keyed_seed(base, h2));
    }

    /// Seeding never produces the all-zero state, which xoshiro256** cannot leave.
    #[kani::proof]
    #[kani::solver(cadical)]
    fn seeding_never_gives_the_all_zero_state() {
        let seed: u64 = kani::any();
        assert_ne!(Rng::new(seed).s, [0u64; 4]);
    }

    /// One xoshiro256** step takes a non-zero state to a non-zero state (the step is invertible).
    #[kani::proof]
    fn a_step_keeps_the_state_non_zero() {
        let s: [u64; 4] = kani::any();
        kani::assume(s != [0u64; 4]);
        let mut r = Rng { s };
        r.next_u64();
        assert_ne!(r.s, [0u64; 4]);
    }

    /// `uniform` is in [0, 1) and `uniform_open0` in (0, 1], from any generator state.
    #[kani::proof]
    fn uniform_draws_stay_in_their_intervals() {
        let mut r = Rng { s: kani::any() };
        let u = r.uniform();
        assert!(u >= 0.0 && u < 1.0);
        let v = r.uniform_open0();
        assert!(v > 0.0 && v <= 1.0);
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

    #[test]
    fn distribution_ranges_and_bad_parameters() {
        let mut r = Rng::new(5);
        for _ in 0..10_000 {
            let t = r.triangular(1.0, 2.0, 6.0);
            assert!((1.0..=6.0).contains(&t));
            let u = r.uniform_range(-1.0, 1.0);
            assert!((-1.0..1.0).contains(&u));
        }
        assert_eq!(r.triangular(1.0, 7.0, 6.0), 1.0);
        assert_eq!(r.uniform_range(3.0, 3.0), 3.0);
        assert_eq!(Dist::Constant(-2.0).sample(&mut r), 0.0);
        assert_eq!(Dist::exponential_rate(0.0), Dist::Exponential { mean: 0.0 });
    }

    #[test]
    fn fnv1a64_matches_the_published_test_vectors() {
        // From the FNV test suite (Fowler, Noll and Vo): "" and "a".
        assert_eq!(fnv1a64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn keyed_streams_depend_only_on_base_and_key() {
        let draws = |mut r: Rng| [r.next_u64(), r.next_u64()];
        assert_eq!(
            draws(Rng::keyed(7, "element/a")),
            draws(Rng::keyed(7, "element/a"))
        );
        assert_ne!(
            draws(Rng::keyed(7, "element/a")),
            draws(Rng::keyed(7, "element/b"))
        );
        assert_ne!(
            draws(Rng::keyed(7, "element/a")),
            draws(Rng::keyed(8, "element/a"))
        );
    }
}
