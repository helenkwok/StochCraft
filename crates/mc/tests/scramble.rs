//! The random linear matrix scramble with a digital shift of the Sobol points (`qmc::Scramble`,
//! Matoušek 1998; Owen 2003): the matrix arithmetic on small known inputs, the net property it
//! keeps in one and two dimensions, reproducibility from the keyed streams, the variance of a
//! smooth integrand's estimate against the digital shift alone, and the L2-star discrepancy of
//! the scrambled points against independent ones.
//!
//! Every threshold below leaves a wide margin over what an independent implementation of the
//! same scheme measured (numbers in each test's comment), so a correct implementation does not
//! fail on a seed.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use stochcraft_mc::qmc::{self, BITS, MAX_DIMENSIONS, Scramble};
use stochcraft_random::Rng;

/// A scramble drawn from the stream keyed `test/<label>` under `seed`.
fn draw(seed: u64, label: &str) -> Scramble {
    Scramble::draw(&mut Rng::keyed(seed, &format!("test/{label}")))
}

/// L·y worked out digit by digit from the matrix's entries: digit k (counting from 1, most
/// significant first) is the sum mod 2 of L[k][j] y_j over j ≤ k, with L[k][j] bit 32 - j of
/// row k - 1.
fn by_digits(s: &Scramble, y: u32) -> u32 {
    let digit = |x: u32, j: usize| (x >> (BITS - j)) & 1;
    let mut out = 0_u32;
    for k in 1..=BITS {
        let row = s.rows().get(k - 1).copied().unwrap_or(0);
        let mut sum = 0;
        for j in 1..=k {
            sum ^= digit(row, j) & digit(y, j);
        }
        out |= sum << (BITS - k);
    }
    out
}

#[test]
fn the_matrix_is_lower_triangular_with_ones_on_its_diagonal() {
    for s in [
        Scramble::new([u32::MAX; BITS], 0),
        Scramble::new([0; BITS], 0),
        draw(1, "a"),
        draw(2, "b"),
    ] {
        for (k, row) in s.rows().iter().enumerate() {
            let diagonal = 1_u32 << (BITS - 1 - k);
            assert_ne!(row & diagonal, 0, "row {k}");
            assert_eq!(
                row & (diagonal - 1),
                0,
                "row {k}: nothing below the diagonal"
            );
        }
    }
    // With every word all ones, row k has its k + 1 leading bits set.
    let full = Scramble::new([u32::MAX; BITS], 0);
    assert_eq!(full.rows()[0], 0x8000_0000);
    assert_eq!(full.rows()[1], 0xC000_0000);
    assert_eq!(full.rows()[31], u32::MAX);
}

/// Small cases worked by hand, then the bit arithmetic against the digit-by-digit product.
#[test]
fn the_matrix_multiplies_the_digits() {
    // The identity matrix leaves y as it is; the scramble is then the digital shift alone.
    let id = Scramble::shift_only(0x1234_5678);
    for y in [0, 1, 0x8000_0000, 0xDEAD_BEEF, u32::MAX] {
        assert_eq!(id.linear(y), y);
        assert_eq!(id.apply(y), y ^ 0x1234_5678);
    }
    // All ones on and below the diagonal: digit k of L·y is y_1 ⊕ ... ⊕ y_k.
    let full = Scramble::new([u32::MAX; BITS], 0);
    assert_eq!(full.linear(0), 0);
    assert_eq!(full.linear(0x8000_0000), u32::MAX); // y_1 = 1 carries into every digit
    assert_eq!(full.linear(0x4000_0000), 0x7FFF_FFFF); // y_2 = 1: every digit from the second
    assert_eq!(full.linear(0xC000_0000), 0x8000_0000); // y_1 ⊕ y_2 = 0 from the second on
    assert_eq!(full.linear(1), 1); // the last digit touches only itself
    assert_eq!(full.linear(0b101), 0b110);
    // One entry below the diagonal: row 2 (digit 3) also takes digit 1.
    let mut words = [0_u32; BITS];
    words[2] = 0x8000_0000;
    let one = Scramble::new(words, 0);
    assert_eq!(one.linear(0x8000_0000), 0xA000_0000);
    assert_eq!(one.linear(0x2000_0000), 0x2000_0000);
    assert_eq!(one.linear(0xA000_0000), 0x8000_0000);

    // Random matrices against the product written out digit by digit, and linearity.
    for seed in 0..8 {
        let s = draw(seed, "matrix");
        let mut rng = Rng::keyed(seed, "test/ys");
        let mut ys: Vec<u32> = (0..BITS).map(|k| 1_u32 << k).collect();
        ys.extend([0, u32::MAX, 0xAAAA_AAAA, 0x5555_5555]);
        ys.extend((0..200).map(|_| u32::try_from(rng.next_u64() >> 32).unwrap_or(0)));
        for &y in &ys {
            assert_eq!(s.linear(y), by_digits(&s, y), "seed {seed}, y {y:#x}");
            assert_eq!(s.apply(y), s.linear(y) ^ s.shift());
        }
        for pair in ys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert_eq!(s.linear(a ^ b), s.linear(a) ^ s.linear(b));
        }
        // The leading m output digits depend only on the leading m input digits, one to one:
        // the 256 cells of width 2^-8 go onto the 256 cells, whatever the trailing digits.
        for low in [0_u32, 0x00FF_FFFF, 0x0012_3456] {
            let cells: BTreeSet<u32> = (0..256_u32)
                .map(|t| s.apply((t << 24) | low) >> 24)
                .collect();
            assert_eq!(cells.len(), 256, "seed {seed}");
            for t in 0..256_u32 {
                assert_eq!(s.apply((t << 24) | low) >> 24, s.apply(t << 24) >> 24);
            }
        }
    }
}

/// Scrambling the direction integers (L·C) and walking the Gray code gives each unscrambled
/// point scrambled; the identity scramble gives `shifted`'s points exactly.
#[test]
fn scrambled_points_are_the_scramble_of_each_point() {
    for dim in 0..MAX_DIMENSIONS {
        let s = draw(dim as u64, "walk");
        let plain = qmc::integers(dim, 300).unwrap_or_default();
        let want: Vec<u32> = plain.iter().map(|&y| s.apply(y)).collect();
        assert_eq!(
            qmc::scrambled_integers(dim, 300, &s),
            Some(want),
            "dim {dim}"
        );
        let only = Scramble::shift_only(0x9E37_79B9);
        assert_eq!(
            qmc::scrambled(dim, 300, &only),
            qmc::shifted(dim, 300, 0x9E37_79B9),
            "dim {dim}"
        );
    }
    let s = draw(0, "walk");
    assert!(qmc::scrambled_integers(MAX_DIMENSIONS, 1, &s).is_none());
    assert!(qmc::scrambled(MAX_DIMENSIONS, 1, &s).is_none());
}

/// Every scrambled coordinate is strictly inside (0, 1), and in every dimension the first 2^m
/// points, for each m up to 10, have exactly one point in each interval [k/2^m, (k+1)/2^m).
#[test]
fn every_dimension_keeps_one_point_in_each_interval() {
    for batch in 0..4 {
        for dim in 0..MAX_DIMENSIONS {
            let s = draw(11, &format!("{batch}/{dim}"));
            let ps = qmc::scrambled(dim, 1024, &s).unwrap_or_default();
            assert_eq!(ps.len(), 1024);
            assert!(ps.iter().all(|p| *p > 0.0 && *p < 1.0), "dim {dim}");
            let ints = qmc::scrambled_integers(dim, 1024, &s).unwrap_or_default();
            for m in 0..=10_usize {
                let first = ints.get(..1 << m).unwrap_or_default();
                let cells: BTreeSet<u64> =
                    first.iter().map(|&x| u64::from(x) >> (32 - m)).collect();
                assert_eq!(cells.len(), 1 << m, "batch {batch}, dim {dim}, m {m}");
            }
            // The floats fall in the same intervals as the integers.
            let cells: BTreeSet<usize> = ps.iter().map(|p| (p * 1024.0) as usize).collect();
            assert_eq!(cells.len(), 1024, "dim {dim}");
        }
    }
}

/// How many of the first 2^m points fall in each elementary box of a pair of dimensions with
/// sides 2^-j by 2^-(m-j), for every j from 0 to m, as the sorted counts of the boxes.
fn box_counts(a: &[u32], b: &[u32], m: usize) -> Vec<Vec<u32>> {
    (0..=m)
        .map(|j| {
            let mut boxes: BTreeMap<(u64, u64), u32> = BTreeMap::new();
            for (&x, &y) in a.iter().zip(b).take(1 << m) {
                let key = (u64::from(x) >> (32 - j), u64::from(y) >> (32 - (m - j)));
                *boxes.entry(key).or_default() += 1;
            }
            // Empty boxes count too: there are 2^m of them in all.
            let mut counts: Vec<u32> = boxes.into_values().collect();
            counts.resize(1 << m, 0);
            counts.sort_unstable();
            counts
        })
        .collect()
}

/// The first two Sobol dimensions are a (0, 2)-sequence: the first 2^m points put exactly one in
/// each elementary box of volume 2^-m, and scrambled they still do. Other pairs have a t above 0
/// for some m, so for them the check is that the scramble only moves elementary boxes onto
/// elementary boxes: the counts of the boxes are those of the unscrambled points.
#[test]
fn pairs_of_dimensions_keep_their_stratification() {
    for batch in 0..4 {
        let scramble = |dim: usize| draw(23, &format!("{batch}/{dim}"));
        let points =
            |dim: usize| qmc::scrambled_integers(dim, 1 << 10, &scramble(dim)).unwrap_or_default();
        let (a, b) = (points(0), points(1));
        for m in 1..=10 {
            for (j, counts) in box_counts(&a, &b, m).iter().enumerate() {
                assert!(
                    counts.iter().all(|c| *c == 1),
                    "batch {batch}, m {m}, j {j}: {counts:?}"
                );
            }
        }
        for (d1, d2) in [(2, 3), (0, 4), (4, 9), (7, 30), (62, 63)] {
            let plain = (
                qmc::integers(d1, 1 << 10).unwrap_or_default(),
                qmc::integers(d2, 1 << 10).unwrap_or_default(),
            );
            let (x, y) = (points(d1), points(d2));
            for m in 1..=8 {
                assert_eq!(
                    box_counts(&x, &y, m),
                    box_counts(&plain.0, &plain.1, m),
                    "batch {batch}, dims {d1} and {d2}, m {m}"
                );
            }
        }
    }
}

/// The same stream gives the same scramble and points; another key gives others. The stream's
/// first draw is the shift and the next 31 are rows 2 to 32, so the shift is the one the digital
/// shift alone drew from that stream.
#[test]
fn scrambles_are_reproducible_from_their_stream() {
    let key = "mc/qmc/3/demand";
    let a = Scramble::draw(&mut Rng::keyed(42, key));
    let b = Scramble::draw(&mut Rng::keyed(42, key));
    assert_eq!(a, b);
    assert_eq!(qmc::scrambled(5, 1_000, &a), qmc::scrambled(5, 1_000, &b));
    assert_ne!(a, Scramble::draw(&mut Rng::keyed(43, key)));
    assert_ne!(a, Scramble::draw(&mut Rng::keyed(42, "mc/qmc/4/demand")));

    let mut rng = Rng::keyed(42, key);
    let top = |r: &mut Rng| u32::try_from(r.next_u64() >> 32).unwrap_or(0);
    assert_eq!(a.shift(), top(&mut rng));
    let mut words = [0_u32; BITS];
    for word in words.iter_mut().skip(1) {
        *word = top(&mut rng);
    }
    assert_eq!(a, Scramble::new(words, a.shift()));
}

/// The product of (1 + (x_j - 0.5) / j) over five dimensions: smooth, with integral exactly 1.
fn product(x: &[f64]) -> f64 {
    x.iter()
        .enumerate()
        .map(|(j, u)| 1.0 + (u - 0.5) / (j + 1) as f64)
        .product()
}

/// The sample variance, over `batches` independent randomisations, of the mean of `product` at
/// the first `n` points, scrambled with the matrix or shifted only.
fn variance(n: usize, batches: u64, matrix: bool) -> (f64, f64) {
    let dims = 5;
    let means: Vec<f64> = (0..batches)
        .map(|b| {
            let columns: Vec<Vec<f64>> = (0..dims)
                .map(|dim| {
                    let label = format!("variance/{n}/{matrix}/{b}/{dim}");
                    let s = draw(7, &label);
                    let s = if matrix {
                        s
                    } else {
                        Scramble::shift_only(s.shift())
                    };
                    qmc::scrambled(dim, n, &s).unwrap_or_default()
                })
                .collect();
            let mut x = vec![0.0; dims];
            let mut sum = 0.0;
            for i in 0..n {
                for (slot, column) in x.iter_mut().zip(&columns) {
                    *slot = column.get(i).copied().unwrap_or(f64::NAN);
                }
                sum += product(&x);
            }
            sum / n as f64
        })
        .collect();
    let mean = means.iter().sum::<f64>() / means.len() as f64;
    let var = means.iter().map(|m| (m - mean).powi(2)).sum::<f64>() / (means.len() - 1) as f64;
    (mean, var)
}

/// For a smooth integrand the scramble's estimate varies far less than the shift's. The shift
/// moves every point of a dimension alike, so for f(x) = x on 2^m points its variance is
/// 1/(12 n²) where nested uniform scrambling, which the matrix scramble matches in variance, has
/// 1/(12 n³) (Owen 2003, §3.1); the product of linear factors has such terms in every
/// dimension and their products. An independent implementation of both schemes measured the
/// ratio of the variances (matrix over shift, 100 batches each) at about 0.019 at 256 points,
/// 0.19 at 1,000 (not a power of two: only the first 512 points are a whole net), 0.0046 at
/// 1,024 and 0.0017 at 4,096. The thresholds are 10 or more times above those, while the
/// sample variance of 100 batches is within about ±40% of its expectation.
#[test]
fn the_scramble_narrows_the_variance_of_a_smooth_integrand() {
    for (n, most) in [(256, 0.25), (1_000, 0.6), (1_024, 0.05), (4_096, 0.02)] {
        let (shift_mean, shift_var) = variance(n, 100, false);
        let (matrix_mean, matrix_var) = variance(n, 100, true);
        let ratio = matrix_var / shift_var;
        println!(
            "{n} points: variance {matrix_var:.3e} scrambled, {shift_var:.3e} shifted, ratio {ratio:.4}"
        );
        assert!(ratio <= most, "{n} points: ratio {ratio}");
        assert!(matrix_var > 0.0, "{n} points");
        // Both are unbiased: their means over 100 batches are within five standard errors of 1.
        assert!((shift_mean - 1.0).abs() <= 5.0 * (shift_var / 100.0).sqrt() + 1e-12);
        assert!((matrix_mean - 1.0).abs() <= 5.0 * (matrix_var / 100.0).sqrt() + 1e-12);
    }
}

/// The squared L2-star discrepancy of points x_1 ... x_n in [0, 1)^d, by Warnock's formula
/// (Warnock 1972; Morokoff and Caflisch, "Quasi-random sequences and their discrepancies", *SIAM
/// J. Sci. Comput.* 15(6), 1994): 3^-d - (2^(1-d) / n) Σ_i Π_k (1 - x_ik²)
/// + (1 / n²) Σ_i Σ_j Π_k (1 - max(x_ik, x_jk)).
fn l2_star_squared(points: &[Vec<f64>]) -> f64 {
    let n = points.len() as f64;
    let d = points.first().map_or(0, Vec::len);
    let single: f64 = points
        .iter()
        .map(|x| x.iter().map(|u| 1.0 - u * u).product::<f64>())
        .sum();
    let mut double = 0.0;
    for x in points {
        for y in points {
            double += x
                .iter()
                .zip(y)
                .map(|(a, b)| 1.0 - a.max(*b))
                .product::<f64>();
        }
    }
    let d = i32::try_from(d).unwrap_or(0);
    3_f64.powi(-d) - 2_f64.powi(1 - d) / n * single + double / (n * n)
}

/// The first `n` scrambled points of dimensions 0 to d - 1, as rows.
fn scrambled_rows(n: usize, d: usize, label: &str) -> Vec<Vec<f64>> {
    let columns: Vec<Vec<f64>> = (0..d)
        .map(|dim| qmc::scrambled(dim, n, &draw(31, &format!("{label}/{dim}"))).unwrap_or_default())
        .collect();
    (0..n)
        .map(|i| {
            columns
                .iter()
                .map(|c| c.get(i).copied().unwrap_or(f64::NAN))
                .collect()
        })
        .collect()
}

/// The most the squared L2-star discrepancy of 1,024 scrambled points may be, as a share of its
/// mean for 1,024 independent points, in 2, 5 and 10 dimensions. An independent implementation
/// of the scheme measured about 0.0039, 0.082 and 0.59 (three sets each, within ±5%; the digital
/// shift alone gives about the same: many scramblings leave the mean squared L2 discrepancy as it
/// is, Owen 2003, §3.4, while they change the variance of a given integrand). The advantage under this measure
/// shrinks as the dimension grows at a fixed number of points, so the 10-dimensional limit has
/// the least margin, still about six times the spread measured.
const DISCREPANCY_LIMITS: [(i32, f64); 3] = [(2, 0.02), (5, 0.2), (10, 0.8)];

/// Warnock's formula is right: for one point it is 3^-d - 2^(1-d) Π (1 - x²) + Π (1 - x), and
/// on independent uniform points its mean is (2^-d - 3^-d) / n (Morokoff and Caflisch, 1994).
/// The scrambled points' discrepancy is well below that ([`DISCREPANCY_LIMITS`]).
#[test]
fn scrambled_points_have_a_low_l2_star_discrepancy() {
    let one = vec![vec![0.25, 0.5]];
    let want = 1.0 / 9.0 - 0.5 * (1.0 - 0.0625) * (1.0 - 0.25) + 0.75 * 0.5;
    assert!((l2_star_squared(&one) - want).abs() < 1e-15);

    let n = 1_024;
    // Independent points average to their expectation: 100 sets of 128 points in 2 dimensions,
    // within 30% (each set's squared discrepancy has a coefficient of variation under 1, so the
    // mean of 100 is within about 10%).
    let mut rng = Rng::keyed(5, "test/random-points");
    let sets: Vec<f64> = (0..100)
        .map(|_| {
            let pts: Vec<Vec<f64>> = (0..128)
                .map(|_| vec![rng.uniform(), rng.uniform()])
                .collect();
            l2_star_squared(&pts)
        })
        .collect();
    let random_mean = sets.iter().sum::<f64>() / 100.0;
    let expected = (0.25 - 1.0 / 9.0) / 128.0;
    assert!(
        (random_mean / expected - 1.0).abs() < 0.3,
        "independent points: {random_mean} against {expected}"
    );

    for (d, most) in DISCREPANCY_LIMITS {
        let expected = (2_f64.powi(-d) - 3_f64.powi(-d)) / n as f64;
        for batch in 0..2 {
            let pts = scrambled_rows(n, d as usize, &format!("disc/{d}/{batch}"));
            let ratio = l2_star_squared(&pts) / expected;
            println!("{d} dimensions, batch {batch}: L2-star² ratio to random {ratio:.4}");
            assert!(
                ratio > 0.0 && ratio <= most,
                "{d} dimensions: ratio {ratio}"
            );
        }
    }
}
