//! `rng [DRAWS] [ROUNDS]`: the generator's speed against a frozen copy of xoshiro256** as it was
//! before a stream could read a file of random bytes (a plain struct, no choice of source).
//!
//! Both draw `DRAWS` exponential times (one uniform and one logarithm each, the commonest draw
//! in a model) and `DRAWS` raw `u64`s from the same seed, in rounds that alternate which goes
//! first; the median of the rounds is reported. The check: both give exactly the same numbers,
//! so the stream itself is unchanged, and the report is the cost of the choice of source alone.

use std::hint::black_box;
use std::time::Instant;

use stochcraft_random::{Rng, SplitMix64, math};

/// xoshiro256** as `stochcraft-random` had it, frozen.
struct Frozen {
    s: [u64; 4],
}

impl Frozen {
    fn new(seed: u64) -> Self {
        let mut sm = SplitMix64::new(seed);
        Self {
            s: [sm.next_u64(), sm.next_u64(), sm.next_u64(), sm.next_u64()],
        }
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
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

    #[inline]
    fn uniform_open0(&mut self) -> f64 {
        ((self.next_u64() >> 11) + 1) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    #[inline]
    fn exponential(&mut self, mean: f64) -> f64 {
        -mean * math::ln(self.uniform_open0())
    }
}

fn time<T>(f: impl FnOnce() -> T) -> (f64, T) {
    let t = Instant::now();
    let out = f();
    (t.elapsed().as_secs_f64(), out)
}

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs.get(xs.len() / 2).copied().unwrap_or(f64::NAN)
}

/// Runs the comparison; false if the two generators ever disagree.
pub fn run(draws: u64, rounds: usize) -> bool {
    let (mut frozen_exp, mut new_exp, mut frozen_raw, mut new_raw) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut same = true;
    for round in 0..rounds.max(1) {
        let seed = 2026 + round as u64;
        let frozen = || {
            let mut r = Frozen::new(seed);
            let mut sum = 0.0;
            for _ in 0..draws {
                sum += r.exponential(1.0);
            }
            black_box(sum)
        };
        let new = || {
            let mut r = Rng::new(seed);
            let mut sum = 0.0;
            for _ in 0..draws {
                sum += r.exponential(1.0);
            }
            black_box(sum)
        };
        let frozen_u = || {
            let mut r = Frozen::new(seed);
            let mut x = 0u64;
            for _ in 0..draws {
                x ^= r.next_u64();
            }
            black_box(x)
        };
        let new_u = || {
            let mut r = Rng::new(seed);
            let mut x = 0u64;
            for _ in 0..draws {
                x ^= r.next_u64();
            }
            black_box(x)
        };
        let (a, b, c, d) = if round % 2 == 0 {
            let a = time(frozen);
            let b = time(new);
            let c = time(frozen_u);
            let d = time(new_u);
            (a, b, c, d)
        } else {
            let b = time(new);
            let a = time(frozen);
            let d = time(new_u);
            let c = time(frozen_u);
            (a, b, c, d)
        };
        same &= a.1.to_bits() == b.1.to_bits() && c.1 == d.1;
        frozen_exp.push(a.0);
        new_exp.push(b.0);
        frozen_raw.push(c.0);
        new_raw.push(d.0);
    }
    let per = |s: f64| s / draws as f64 * 1e9;
    let (fe, ne, fr, nr) = (
        median(frozen_exp),
        median(new_exp),
        median(frozen_raw),
        median(new_raw),
    );
    println!("generator: {draws} draws, median of {rounds} rounds (alternating order)");
    println!(
        "  exponential  frozen {:.3} ns/draw  now {:.3} ns/draw  ratio {:.4}",
        per(fe),
        per(ne),
        ne / fe
    );
    println!(
        "  raw u64      frozen {:.3} ns/draw  now {:.3} ns/draw  ratio {:.4}",
        per(fr),
        per(nr),
        nr / fr
    );
    if same {
        println!("  check: both give exactly the same numbers");
    } else {
        println!("  check FAILED: the streams differ");
    }
    same
}
