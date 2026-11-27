//! `rng [DRAWS] [ROUNDS]`: the generator's speed against the same crate as it was before a stream
//! could read a file of random bytes (its generator at main d4b0441a, copied unchanged into
//! `random-before`), compiled the same way in the same process.
//!
//! Each draws `DRAWS` exponential times (one uniform and one logarithm each, the commonest draw in
//! a model), `DRAWS` uniforms and `DRAWS` raw `u64`s from the same seed, in rounds that alternate
//! which goes first; the median of the rounds is reported. The check: both give exactly the same
//! numbers, so the stream itself is unchanged, and the report is the cost of the choice of source
//! alone.

use std::hint::black_box;
use std::time::Instant;

use stochcraft_random::Rng;
use stochcraft_random_before::Rng as Before;

fn time<T>(f: impl FnOnce() -> T) -> (f64, T) {
    let t = Instant::now();
    let out = f();
    (t.elapsed().as_secs_f64(), out)
}

fn median(mut xs: Vec<f64>) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs.get(xs.len() / 2).copied().unwrap_or(f64::NAN)
}

/// One kind of draw, timed on both: (seconds before, seconds now, same numbers).
fn race(
    seed: u64,
    before_first: bool,
    before: impl FnOnce(Before) -> u64,
    now: impl FnOnce(Rng) -> u64,
) -> (f64, f64, bool) {
    let (b, n) = if before_first {
        let b = time(|| before(Before::new(seed)));
        (b, time(|| now(Rng::new(seed))))
    } else {
        let n = time(|| now(Rng::new(seed)));
        (time(|| before(Before::new(seed))), n)
    };
    (b.0, n.0, b.1 == n.1)
}

/// Runs the comparison; false if the two generators ever disagree.
pub fn run(draws: u64, rounds: usize) -> bool {
    let names = ["exponential", "uniform", "raw u64"];
    let mut times: Vec<(Vec<f64>, Vec<f64>)> = names.iter().map(|_| (vec![], vec![])).collect();
    let mut same = true;
    for round in 0..rounds.max(1) {
        let seed = 2026 + round as u64;
        let first = round % 2 == 0;
        let results = [
            race(
                seed,
                first,
                |mut r| {
                    let mut s = 0.0;
                    for _ in 0..draws {
                        s += r.exponential(1.0);
                    }
                    black_box(s).to_bits()
                },
                |mut r| {
                    let mut s = 0.0;
                    for _ in 0..draws {
                        s += r.exponential(1.0);
                    }
                    black_box(s).to_bits()
                },
            ),
            race(
                seed,
                first,
                |mut r| {
                    let mut s = 0.0;
                    for _ in 0..draws {
                        s += r.uniform();
                    }
                    black_box(s).to_bits()
                },
                |mut r| {
                    let mut s = 0.0;
                    for _ in 0..draws {
                        s += r.uniform();
                    }
                    black_box(s).to_bits()
                },
            ),
            race(
                seed,
                first,
                |mut r| {
                    let mut x = 0u64;
                    for _ in 0..draws {
                        x ^= r.next_u64();
                    }
                    black_box(x)
                },
                |mut r| {
                    let mut x = 0u64;
                    for _ in 0..draws {
                        x ^= r.next_u64();
                    }
                    black_box(x)
                },
            ),
        ];
        for (slot, (b, n, s)) in times.iter_mut().zip(results) {
            slot.0.push(b);
            slot.1.push(n);
            same &= s;
        }
    }
    let per = |s: f64| s / draws as f64 * 1e9;
    println!("generator: {draws} draws, median of {rounds} rounds (alternating order)");
    for (name, (b, n)) in names.iter().zip(times) {
        let (b, n) = (median(b), median(n));
        println!(
            "  {name:<12} before {:.3} ns/draw  now {:.3} ns/draw  ratio {:.4}",
            per(b),
            per(n),
            n / b
        );
    }
    if same {
        println!("  check: both give exactly the same numbers");
    } else {
        println!("  check FAILED: the streams differ");
    }
    same
}
