//! Deterministic random numbers for StochCraft.
//!
//! The generator is xoshiro256** (Blackman and Vigna, "Scrambled linear pseudorandom number
//! generators", ACM TOMS 47(4), 2021), seeded by expanding a `u64` with SplitMix64 as the paper
//! recommends. Both are implemented here from the published description. Everything is `u64`
//! and `f64` arithmetic, so a seed gives the same stream on native, wasm32 and wasm64.
//!
//! A stream can instead read a segment of a file of random bytes ([`Rng::from_bytes`]): eight
//! bytes per draw, little-endian, mapped strictly inside (0, 1) by [`open_unit`]. Every stream
//! derived from it ([`StreamRoot`], [`Rng::derive`], [`Rng::fork`]) shares its one cursor, so a
//! replication reads its segment once, in the order its events draw, and running out is
//! reported ([`ByteHandle::exhausted`]) rather than the bytes reused.
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

/// A stream of random numbers: the xoshiro256** generator, or a segment of a file of random
/// bytes (see [`Rng::from_bytes`]).
///
/// A generator seeded with [`Rng::new`] gives exactly the stream it always has. A stream over
/// bytes reads them in order, eight at a time as a little-endian `u64`, through one cursor that
/// every stream derived from it shares ([`StreamRoot`], [`Rng::derive`], [`Rng::fork`]): one
/// replication reads its segment once, in the order its events draw. Deriving a stream reads
/// nothing.
#[derive(Debug, Clone)]
pub struct Rng {
    src: Source,
}

#[derive(Debug, Clone)]
enum Source {
    Xoshiro([u64; 4]),
    Bytes(ByteHandle),
}

// A replication's streams are made and used on one thread, but runs, plans and results that hold
// them move between threads.
const _: fn() = || {
    fn check<T: Send + Sync>() {}
    check::<Rng>();
};

/// One step of xoshiro256** on state `s`.
#[inline]
fn step(s: &mut [u64; 4]) -> u64 {
    let [s0, s1, s2, s3] = *s;
    let result = s1.wrapping_mul(5).rotate_left(7).wrapping_mul(9);
    let t = s1 << 17;
    let mut n = [s0, s1, s2 ^ s0, s3 ^ s1];
    n[1] ^= n[2];
    n[0] ^= n[3];
    n[2] ^= t;
    n[3] = n[3].rotate_left(45);
    *s = n;
    result
}

/// The state xoshiro256** starts from for `seed`, expanded by SplitMix64 (never all zero).
fn seeded(seed: u64) -> [u64; 4] {
    let mut sm = SplitMix64::new(seed);
    [sm.next_u64(), sm.next_u64(), sm.next_u64(), sm.next_u64()]
}

/// 2^-53, the spacing of the grids below.
const EPS53: f64 = 1.0 / (1u64 << 53) as f64;

/// A `u64` of random bits as a uniform number strictly inside (0, 1): u = (2·(x >> 12) + 1)·2^-53,
/// the midpoints of 2^52 equal cells. The smallest is 2^-53 and the largest 1 − 2^-53, both
/// exact, and 1 − u lies on the same grid, so neither u nor 1 − u is ever 0 or 1 (each value is
/// below 2^53, so it converts to `f64` exactly). How a stream over bytes maps its draws.
#[inline]
pub fn open_unit(x: u64) -> f64 {
    ((x >> 12) * 2 + 1) as f64 * EPS53
}

/// A stream's cursor in a file of random bytes, which every stream derived from it shares.
#[derive(Clone)]
pub struct ByteHandle(std::sync::Arc<Cursor>);

struct Cursor {
    feed: Feed,
    /// Bytes asked for so far, including any past the end of the segment.
    asked: std::sync::atomic::AtomicU64,
    /// Serves the draws a segment cannot: every draw of a pilot, and, once a segment has run
    /// out, throwaway values so the replication can stop cleanly (its results are discarded).
    /// Never a byte of the file again.
    spare: std::sync::Mutex<[u64; 4]>,
}

enum Feed {
    /// `len` bytes of `data` from `start`.
    File {
        data: std::sync::Arc<[u8]>,
        start: usize,
        len: u64,
    },
    /// A pilot: no file, the bytes only counted.
    Pilot,
}

impl std::fmt::Debug for ByteHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = match &self.0.feed {
            Feed::File { len, .. } => Some(*len),
            Feed::Pilot => None,
        };
        f.debug_struct("ByteHandle")
            .field("len", &len)
            .field("asked", &self.asked())
            .finish()
    }
}

impl ByteHandle {
    fn new(feed: Feed, spare_seed: u64) -> Self {
        Self(std::sync::Arc::new(Cursor {
            feed,
            asked: std::sync::atomic::AtomicU64::new(0),
            spare: std::sync::Mutex::new(seeded(spare_seed)),
        }))
    }

    /// The next eight bytes, little-endian.
    #[inline(never)]
    fn next(&self) -> u64 {
        let c = &self.0;
        let at = c.asked.fetch_add(8, std::sync::atomic::Ordering::Relaxed);
        if let Feed::File { data, start, len } = &c.feed
            && at.checked_add(8).is_some_and(|end| end <= *len)
            && let Some(i) = usize::try_from(at).ok().and_then(|a| start.checked_add(a))
            && let Some(bytes) = i.checked_add(8).and_then(|j| data.get(i..j))
        {
            let mut b = [0u8; 8];
            b.copy_from_slice(bytes);
            return u64::from_le_bytes(b);
        }
        let mut s = c.spare.lock().unwrap_or_else(|e| e.into_inner());
        step(&mut *s)
    }

    /// Bytes asked for so far: eight per draw, including any the segment could not give.
    pub fn asked(&self) -> u64 {
        self.0.asked.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Bytes of the file read so far (at most the segment's length; a pilot's are all counted).
    pub fn used(&self) -> u64 {
        match &self.0.feed {
            Feed::File { len, .. } => self.asked().min(*len),
            Feed::Pilot => self.asked(),
        }
    }

    /// Whether a draw has asked for more bytes than the segment has. A pilot never runs out.
    pub fn exhausted(&self) -> bool {
        match &self.0.feed {
            Feed::File { len, .. } => self.asked() > *len,
            Feed::Pilot => false,
        }
    }
}

impl Rng {
    /// A generator seeded from `seed` through SplitMix64 (never the all-zero state).
    pub fn new(seed: u64) -> Self {
        Self::from_state(seeded(seed))
    }

    /// The generator in state `s`.
    pub(crate) fn from_state(s: [u64; 4]) -> Self {
        Self {
            src: Source::Xoshiro(s),
        }
    }

    /// A stream over `len` bytes of `data` from byte `start` (as many as `data` has there, if
    /// fewer). It reads them in order, eight at a time as a little-endian `u64`, and maps each to
    /// a uniform number by [`open_unit`]. Once a draw asks for more than the segment has, the
    /// handle reports it [`ByteHandle::exhausted`] and the draws that follow are throwaway values
    /// from a generator of its own, never bytes of `data` read again.
    pub fn from_bytes(data: std::sync::Arc<[u8]>, start: u64, len: u64) -> Self {
        let have = u64::try_from(data.len()).unwrap_or(u64::MAX);
        let len = len.min(have.saturating_sub(start));
        let start = usize::try_from(start).unwrap_or(usize::MAX);
        let feed = Feed::File { data, start, len };
        Self {
            src: Source::Bytes(ByteHandle::new(feed, 0x9e37_79b9_7f4a_7c15)),
        }
    }

    /// A pilot stream: drawn as a stream over bytes is (one cursor shared by every derived
    /// stream, mapped by [`open_unit`]), from xoshiro256** seeded with `seed`, counting the bytes
    /// a file would have given ([`ByteHandle::asked`]). How the length of a replication's
    /// segment is estimated before a file is read.
    pub fn pilot(seed: u64) -> Self {
        Self {
            src: Source::Bytes(ByteHandle::new(Feed::Pilot, seed)),
        }
    }

    /// The cursor of a stream over bytes (or a pilot); `None` for xoshiro256**.
    pub fn byte_handle(&self) -> Option<ByteHandle> {
        match &self.src {
            Source::Xoshiro(_) => None,
            Source::Bytes(h) => Some(h.clone()),
        }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        match &mut self.src {
            Source::Xoshiro(s) => step(s),
            Source::Bytes(h) => h.next(),
        }
    }

    /// Uniform on [0, 1), from the top 53 bits; strictly inside (0, 1) over bytes
    /// ([`open_unit`]).
    #[inline]
    pub fn uniform(&mut self) -> f64 {
        match &mut self.src {
            Source::Xoshiro(s) => (step(s) >> 11) as f64 * EPS53,
            Source::Bytes(h) => open_unit(h.next()),
        }
    }

    /// Uniform on (0, 1]: never zero, so safe under `ln`; strictly inside (0, 1) over bytes
    /// ([`open_unit`]).
    #[inline]
    pub fn uniform_open0(&mut self) -> f64 {
        match &mut self.src {
            Source::Xoshiro(s) => ((step(s) >> 11) + 1) as f64 * EPS53,
            Source::Bytes(h) => open_unit(h.next()),
        }
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
    /// one's next output. Over bytes, the same cursor, reading nothing.
    pub fn fork(&mut self) -> Rng {
        if let Source::Bytes(h) = &self.src {
            return Rng {
                src: Source::Bytes(h.clone()),
            };
        }
        Rng::new(self.next_u64())
    }

    /// The stream for a sub-stream marked by `salt`, seeded from this one's next output without
    /// drawing it (so this stream goes on as it would have), mixed with `salt`. Over bytes, the
    /// same cursor, reading nothing.
    pub fn derive(&self, salt: u64) -> Rng {
        match &self.src {
            Source::Xoshiro(_) => Rng::new(self.clone().next_u64() ^ salt),
            Source::Bytes(_) => self.clone(),
        }
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

/// Where a set of keyed sub-streams comes from: one draw of a parent stream, under which each is
/// [`Rng::keyed`] by its name, or, over bytes, the parent's cursor itself.
#[derive(Debug, Clone)]
pub struct StreamRoot(Root);

#[derive(Debug, Clone)]
enum Root {
    Base(u64),
    Bytes(ByteHandle),
}

impl StreamRoot {
    /// The root under `parent`: its next draw (over bytes, its cursor, reading nothing).
    pub fn new(parent: &mut Rng) -> Self {
        if let Source::Bytes(h) = &parent.src {
            return Self(Root::Bytes(h.clone()));
        }
        Self(Root::Base(parent.next_u64()))
    }

    /// The root under `parent`'s next draw, without drawing it, mixed with `salt` (over bytes,
    /// its cursor, reading nothing).
    pub fn peek(parent: &Rng, salt: u64) -> Self {
        match &parent.src {
            Source::Xoshiro(_) => Self(Root::Base(parent.clone().next_u64() ^ salt)),
            Source::Bytes(h) => Self(Root::Bytes(h.clone())),
        }
    }

    /// The sub-stream named `key`.
    pub fn keyed(&self, key: &str) -> Rng {
        match &self.0 {
            Root::Base(base) => Rng::keyed(*base, key),
            Root::Bytes(h) => Rng {
                src: Source::Bytes(h.clone()),
            },
        }
    }
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

    /// Seeding never produces the all-zero state, which xoshiro256** cannot leave.
    #[kani::proof]
    #[kani::solver(cadical)]
    fn seeding_never_gives_the_all_zero_state() {
        let seed: u64 = kani::any();
        assert_ne!(seeded(seed), [0u64; 4]);
    }

    /// One xoshiro256** step takes a non-zero state to a non-zero state (the step is invertible).
    #[kani::proof]
    fn a_step_keeps_the_state_non_zero() {
        let mut s: [u64; 4] = kani::any();
        kani::assume(s != [0u64; 4]);
        step(&mut s);
        assert_ne!(s, [0u64; 4]);
    }

    /// `uniform` is in [0, 1) and `uniform_open0` in (0, 1], from any generator state.
    #[kani::proof]
    fn uniform_draws_stay_in_their_intervals() {
        let mut r = Rng::from_state(kani::any());
        let u = r.uniform();
        assert!(u >= 0.0 && u < 1.0);
        let v = r.uniform_open0();
        assert!(v > 0.0 && v <= 1.0);
    }

    /// Over bytes, a uniform draw is strictly inside (0, 1) whatever the eight bytes it reads
    /// (little-endian), and so is one minus it: neither is ever 0 or 1.
    #[kani::proof]
    fn a_draw_over_bytes_is_strictly_inside_0_1() {
        let bytes: [u8; 8] = kani::any();
        let u = open_unit(u64::from_le_bytes(bytes));
        assert!(u > 0.0 && u < 1.0);
        assert!(1.0 - u > 0.0 && 1.0 - u < 1.0);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)] // A missing handle fails the test.
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

    /// The first outputs of xoshiro256** seeded from 42 by SplitMix64, as the published C
    /// reference code gives them, so the generator itself is pinned, not only its seeding.
    #[test]
    fn xoshiro256_starstar_matches_the_reference_for_seed_42() {
        let mut r = Rng::new(42);
        assert_eq!(r.next_u64(), 1_546_998_764_402_558_742);
        assert_eq!(r.next_u64(), 6_990_951_692_964_543_102);
        assert_eq!(r.next_u64(), 12_544_586_762_248_559_009);
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

    /// The mapping of a draw over bytes at its extremes: all zero bits give 2^-53, all one bits
    /// 1 − 2^-53, and only the top 52 bits count.
    #[test]
    fn the_open_mapping_at_its_extremes() {
        let eps = 1.0 / (1u64 << 53) as f64;
        assert_eq!(open_unit(0), eps);
        assert_eq!(open_unit(u64::MAX), 1.0 - eps);
        assert!(open_unit(u64::MAX) < 1.0);
        assert_eq!(open_unit(0xfff), open_unit(0));
        assert_eq!(open_unit(1 << 12), 3.0 * eps);
        assert_eq!(1.0 - open_unit(u64::MAX), open_unit(0));
        assert_eq!(open_unit(1 << 63), 0.5 + eps);
    }

    /// Bytes are read eight at a time, little-endian, in order, through one cursor that every
    /// derived stream shares; deriving reads nothing.
    #[test]
    fn bytes_are_read_little_endian_through_one_shared_cursor() {
        let mut data: Vec<u8> = (1..=8).collect();
        data.extend(u64::MAX.to_le_bytes());
        data.extend(7u64.to_le_bytes());
        let mut r = Rng::from_bytes(data.into(), 0, 24);
        let handle = r.byte_handle().expect("a stream over bytes");
        let mut root_parent = r.clone();
        let root = StreamRoot::new(&mut root_parent);
        let mut child = root.keyed("element/a");
        let mut derived = r.derive(0x1234);
        let mut forked = r.fork();
        assert_eq!(handle.asked(), 0, "deriving reads nothing");
        assert_eq!(r.next_u64(), 0x0807_0605_0403_0201);
        assert_eq!(child.uniform(), 1.0 - 1.0 / (1u64 << 53) as f64);
        assert_eq!(derived.next_u64(), 7);
        assert_eq!(handle.used(), 24);
        assert!(!handle.exhausted());
        // The segment is used up: the next draw is a throwaway, and says so.
        let _ = forked.next_u64();
        assert!(handle.exhausted());
        assert_eq!(handle.used(), 24);
        assert_eq!(handle.asked(), 32);
    }

    /// A segment never reads past its own end, nor past the data: the bytes after it belong to
    /// another replication.
    #[test]
    fn a_segment_reads_only_its_own_bytes() {
        let data: std::sync::Arc<[u8]> = (0..64u8).collect::<Vec<u8>>().into();
        let mut second = Rng::from_bytes(data.clone(), 16, 16);
        assert_eq!(
            second.next_u64(),
            u64::from_le_bytes([16, 17, 18, 19, 20, 21, 22, 23])
        );
        assert_eq!(
            second.next_u64(),
            u64::from_le_bytes([24, 25, 26, 27, 28, 29, 30, 31])
        );
        let after = second.next_u64();
        assert_ne!(after, u64::from_le_bytes([32, 33, 34, 35, 36, 37, 38, 39]));
        assert!(second.byte_handle().is_some_and(|h| h.exhausted()));
        // Asked for more than the data holds, a segment is only what is there.
        let mut short = Rng::from_bytes(data, 60, 16);
        let _ = short.next_u64();
        assert!(short.byte_handle().is_some_and(|h| h.exhausted()));
    }

    /// Over xoshiro256**, deriving and the stream root give exactly the streams of the old
    /// formulas, so every pseudo-random run is unchanged.
    #[test]
    fn pseudo_random_derivations_keep_their_old_streams() {
        let mut a = Rng::new(99);
        let peek = a.clone().next_u64();
        assert_eq!(
            a.derive(5).next_u64(),
            Rng::new(peek ^ 5).next_u64(),
            "derive"
        );
        assert_eq!(
            StreamRoot::peek(&a, 9).keyed("k").next_u64(),
            Rng::keyed(peek ^ 9, "k").next_u64()
        );
        let mut b = a.clone();
        let base = b.next_u64();
        assert_eq!(
            StreamRoot::new(&mut a).keyed("element/x").next_u64(),
            Rng::keyed(base, "element/x").next_u64()
        );
        assert_eq!(a.next_u64(), b.next_u64(), "the root drew one value");
        let mut c = a.clone();
        let seed = c.next_u64();
        assert_eq!(a.fork().next_u64(), Rng::new(seed).next_u64(), "fork");
    }

    /// A pilot counts eight bytes a draw and never runs out.
    #[test]
    fn a_pilot_counts_its_bytes() {
        let mut p = Rng::pilot(3);
        let h = p.byte_handle().expect("a pilot has a handle");
        let mut child = p.derive(1);
        for _ in 0..10 {
            let u = p.uniform();
            assert!(u > 0.0 && u < 1.0);
            let _ = child.exponential(1.0);
        }
        assert_eq!(h.asked(), 160);
        assert!(!h.exhausted());
    }
}
