//! Empirical tests of random-number generators (Law, *Simulation Modeling and Analysis*, 5th ed.,
//! §7.4.1, pp. 409–413): the chi-square test of uniformity, the serial test in two and three
//! dimensions, the runs-up test and the lag-j correlation test.
//!
//! Each test is written once here and run on two generators:
//!
//! - the book's prime-modulus multiplicative generator of App. 7A, Zᵢ = 630,360,016 Zᵢ₋₁
//!   mod (2³¹ − 1), with Uᵢ = (⌊Zᵢ / 2⁷⌋ OR 1) / 2²⁴ and its streams 100,000 draws apart, on
//!   which Examples 7.6–7.9 print the statistics. They reproduce here, so the tests are checked
//!   against an outside answer before they judge anything;
//! - StochCraft's own generator (xoshiro256** on keyed streams), which must pass each test at
//!   level 0.001 on fixed keys. The keys and the level were fixed before the statistics were
//!   computed; the statistics are then pinned too, as docs/developers.md quotes them.

use stochcraft_random::Rng;

/// The cell of `u` in [0, 1) among `k` equal cells, numbered from 0.
fn cell(u: f64, k: usize) -> usize {
    ((u * k as f64) as usize).min(k - 1)
}

/// The counts of `n` non-overlapping `d`-tuples of draws over the k^d cells of the unit cube (the
/// serial test; d = 1 is the chi-square test of uniformity).
fn tuple_counts(mut next: impl FnMut() -> f64, k: usize, d: u32, n: usize) -> Vec<u64> {
    let mut counts = vec![0u64; k.pow(d)];
    for _ in 0..n {
        let c = (0..d).fold(0, |acc, _| acc * k + cell(next(), k));
        counts[c] += 1;
    }
    counts
}

/// Σ (f − e)² over cells each expected to hold `e`. With a whole `e` (8 in every test here) it is
/// a whole number, so the chi-square statistic Σ (f − e)² / e is checked exactly.
fn squared_deviations(counts: &[u64], expected: u64) -> u64 {
    counts
        .iter()
        .map(|&f| {
            let d = f.abs_diff(expected);
            d * d
        })
        .sum()
}

/// The chi-square statistic of `counts`, each cell expected to hold `expected`.
fn chi_square(counts: &[u64], expected: u64) -> f64 {
    squared_deviations(counts, expected) as f64 / expected as f64
}

/// The upper critical point of the chi-square distribution with `df` degrees of freedom whose
/// standard normal counterpart is `z`, by Wilson and Hilferty's approximation (the one Law uses
/// for large df, §7.4.1).
fn chi_square_critical(df: f64, z: f64) -> f64 {
    let c = 2.0 / (9.0 * df);
    let t = 1.0 - c + z * c.sqrt();
    df * t * t * t
}

/// z₀.₉₅, z₀.₉₉₉ and z₀.₉₉₉₅ of the standard normal distribution.
const Z_95: f64 = 1.644_853_626_951_472_2;
const Z_999: f64 = 3.090_232_306_167_813;
const Z_9995: f64 = 3.290_526_731_491_926;

/// The runs-up test's constants aᵢⱼ (to five significant digits) and bᵢ (Knuth, *The Art of
/// Computer Programming*, vol. 2, 3rd ed., §3.3.2; as printed in Law, §7.4.1).
const RUNS_A: [[f64; 6]; 6] = [
    [4_529.4, 9_044.9, 13_568.0, 18_091.0, 22_615.0, 27_892.0],
    [9_044.9, 18_097.0, 27_139.0, 36_187.0, 45_234.0, 55_789.0],
    [13_568.0, 27_139.0, 40_721.0, 54_281.0, 67_852.0, 83_685.0],
    [18_091.0, 36_187.0, 54_281.0, 72_414.0, 90_470.0, 111_580.0],
    [22_615.0, 45_234.0, 67_852.0, 90_470.0, 113_262.0, 139_476.0],
    [
        27_892.0, 55_789.0, 83_685.0, 111_580.0, 139_476.0, 172_860.0,
    ],
];
const RUNS_B: [f64; 6] = [
    1.0 / 6.0,
    5.0 / 24.0,
    11.0 / 120.0,
    19.0 / 720.0,
    29.0 / 5_040.0,
    1.0 / 840.0,
];

/// The runs-up test: the numbers of runs up of length 1 to 5 and of 6 or more, and the statistic
/// R = (1/n) Σᵢⱼ aᵢⱼ (rᵢ − n bᵢ)(rⱼ − n bⱼ), approximately chi-square with 6 df.
fn runs_up(us: &[f64]) -> ([u64; 6], f64) {
    let mut r = [0u64; 6];
    let mut length = 1usize;
    for w in us.windows(2) {
        if w[1] > w[0] {
            length += 1;
        } else {
            r[length.min(6) - 1] += 1;
            length = 1;
        }
    }
    r[length.min(6) - 1] += 1;
    let n = us.len() as f64;
    let dev: Vec<f64> = r
        .iter()
        .zip(RUNS_B)
        .map(|(&ri, bi)| ri as f64 - n * bi)
        .collect();
    let sum: f64 = RUNS_A
        .iter()
        .zip(&dev)
        .map(|(row, di)| row.iter().zip(&dev).map(|(a, dj)| a * di * dj).sum::<f64>())
        .sum();
    (r, sum / n)
}

/// The lag-`j` correlation test's statistic Aⱼ = ρ̂ⱼ / √Var(ρ̂ⱼ), approximately standard normal,
/// with h = ⌊(n − 1)/j⌋ − 1, ρ̂ⱼ = 12/(h + 1) Σₖ₌₀ʰ U₁₊ₖⱼ U₁₊₍ₖ₊₁₎ⱼ − 3 and
/// Var(ρ̂ⱼ) = (13h + 7)/(h + 1)².
fn lag_correlation(us: &[f64], j: usize) -> f64 {
    let h = (us.len() - 1) / j - 1;
    let s: f64 = (0..=h).map(|k| us[k * j] * us[(k + 1) * j]).sum();
    let h1 = (h + 1) as f64;
    let rho = 12.0 / h1 * s - 3.0;
    rho / ((13.0 * h as f64 + 7.0) / (h1 * h1)).sqrt()
}

// ---- The generator of Law's App. 7A ----

/// Zᵢ = 630,360,016 Zᵢ₋₁ mod (2³¹ − 1), computed exactly in 64-bit integers.
struct LawGenerator(u64);

impl LawGenerator {
    const MULTIPLIER: u64 = 630_360_016;
    const MODULUS: u64 = 2_147_483_647;

    fn step(&mut self) -> u64 {
        self.0 = self.0 * Self::MULTIPLIER % Self::MODULUS;
        self.0
    }

    /// U = (⌊Z / 2⁷⌋ OR 1) / 2²⁴, as App. 7A returns it: never 0 or 1.
    fn uniform(&mut self) -> f64 {
        ((self.step() >> 7) | 1) as f64 / 16_777_216.0
    }

    /// Stream `s` (from 1): stream 1's default seed, 1,973,272,912, advanced 100,000 draws per
    /// stream, as App. 7A spaces them.
    fn stream(s: usize) -> Self {
        let mut g = Self(1_973_272_912);
        for _ in 0..(s - 1) * 100_000 {
            g.step();
        }
        Self(g.0)
    }

    fn uniforms(s: usize, n: usize) -> Vec<f64> {
        let mut g = Self::stream(s);
        (0..n).map(|_| g.uniform()).collect()
    }
}

/// The streams 100,000 draws apart start at the seeds App. 7A tabulates (Law, p. 420).
#[test]
fn law_app_7a_streams_start_at_the_tabulated_seeds() {
    assert_eq!(LawGenerator::stream(2).0, 281_629_770);
    assert_eq!(LawGenerator::stream(10).0, 604_901_985);
}

/// Law, Example 7.6 (p. 410): stream 1, k = 4096 cells, n = 32,768 draws: χ² = 4141.0, below
/// the 5% critical point 4245.0 of 4095 df (Wilson–Hilferty).
#[test]
fn law_example_7_6_chi_square_test() {
    let mut g = LawGenerator::stream(1);
    let counts = tuple_counts(|| g.uniform(), 4_096, 1, 32_768);
    assert_eq!(squared_deviations(&counts, 8), 33_128);
    assert_eq!(chi_square(&counts, 8), 4_141.0);
    let critical = chi_square_critical(4_095.0, Z_95);
    assert_eq!((critical * 10.0).round(), 42_450.0, "{critical}");
}

/// Law, Example 7.7 (pp. 410–411): the serial test on stream 2 in pairs (k = 64) and on stream 3
/// in triples (k = 16), n = 32,768 tuples each: χ²(2) = 4016.5 and χ²(3) = 4174.5.
#[test]
fn law_example_7_7_serial_tests() {
    let mut g = LawGenerator::stream(2);
    let pairs = tuple_counts(|| g.uniform(), 64, 2, 32_768);
    assert_eq!(squared_deviations(&pairs, 8), 32_132);
    assert_eq!(chi_square(&pairs, 8), 4_016.5);
    let mut g = LawGenerator::stream(3);
    let triples = tuple_counts(|| g.uniform(), 16, 3, 32_768);
    assert_eq!(squared_deviations(&triples, 8), 33_396);
    assert_eq!(chi_square(&triples, 8), 4_174.5);
}

/// Law, Example 7.8 (p. 412): the runs-up test on stream 4, n = 5000: runs of lengths 1 to 5 and
/// 6 or more (808, 1026, 448, 139, 43, 4) and R = 9.3 (9.2537), below χ²₆,₀.₉₅ = 12.6.
#[test]
fn law_example_7_8_runs_up_test() {
    let (r, stat) = runs_up(&LawGenerator::uniforms(4, 5_000));
    assert_eq!(r, [808, 1_026, 448, 139, 43, 4]);
    assert_eq!((stat * 10.0).round(), 93.0, "{stat}");
    assert!((stat - 9.253_68).abs() < 1e-4, "{stat}");
}

/// Law, Example 7.9 (p. 413): the lag-j correlation test on stream 4 + j for lags j = 1 to 6,
/// n = 5000 each. The book prints 0.90, −1.03, −0.12, −1.32, 0.39 and 0.76. Five agree to the
/// printed two decimals. A₂ is −1.02498, which rounds to −1.02, not the printed −1.03. It lies
/// 0.00002 from the rounding boundary; the sum in single precision gives −1.02497, so precision
/// does not explain the difference in the last printed digit.
#[test]
fn law_example_7_9_correlation_tests() {
    let expected = [
        0.903_050_417,
        -1.024_977_782,
        -0.118_135_543,
        -1.324_472_504,
        0.387_990_759,
        0.757_064_351,
    ];
    let printed = [0.90, -1.03, -0.12, -1.32, 0.39, 0.76];
    for (j, (&want, &book)) in (1..=6).zip(expected.iter().zip(&printed)) {
        let a = lag_correlation(&LawGenerator::uniforms(4 + j, 5_000), j);
        assert!((a - want).abs() < 1e-8, "A_{j} = {a}");
        if j != 2 {
            assert_eq!(
                (a * 100.0).round(),
                (book * 100.0_f64).round(),
                "A_{j} = {a}"
            );
        }
        assert!(a.abs() < 1.96, "A_{j} = {a}");
    }
}

// ---- StochCraft's generator ----

/// `n` uniforms from the keyed stream `rng-check/<name>` under base 1.
fn engine_uniforms(name: &str, n: usize) -> Vec<f64> {
    let mut rng = Rng::keyed(1, &format!("rng-check/{name}"));
    (0..n).map(|_| rng.uniform()).collect()
}

/// The chi-square test of uniformity at level 0.001, as in Example 7.6: χ² = 4111.5.
#[test]
fn engine_generator_passes_the_chi_square_test() {
    let mut rng = Rng::keyed(1, "rng-check/chi-square");
    let counts = tuple_counts(|| rng.uniform(), 4_096, 1, 32_768);
    let stat = chi_square(&counts, 8);
    assert!(stat < chi_square_critical(4_095.0, Z_999), "{stat}");
    assert_eq!(squared_deviations(&counts, 8), 32_892);
}

/// The serial test in pairs and in triples at level 0.001, as in Example 7.7: χ²(2) = 4155.5 and
/// χ²(3) = 4100.0.
#[test]
fn engine_generator_passes_the_serial_tests() {
    let mut rng = Rng::keyed(1, "rng-check/serial-2");
    let pairs = tuple_counts(|| rng.uniform(), 64, 2, 32_768);
    let mut rng = Rng::keyed(1, "rng-check/serial-3");
    let triples = tuple_counts(|| rng.uniform(), 16, 3, 32_768);
    let critical = chi_square_critical(4_095.0, Z_999);
    assert!(
        chi_square(&pairs, 8) < critical,
        "{}",
        chi_square(&pairs, 8)
    );
    assert!(
        chi_square(&triples, 8) < critical,
        "{}",
        chi_square(&triples, 8)
    );
    assert_eq!(squared_deviations(&pairs, 8), 33_244);
    assert_eq!(squared_deviations(&triples, 8), 32_800);
}

/// The runs-up test on 100,000 draws at level 0.001 (χ²₆,₀.₉₉₉ = 22.458): R = 8.45.
#[test]
fn engine_generator_passes_the_runs_up_test() {
    let (r, stat) = runs_up(&engine_uniforms("runs-up", 100_000));
    assert!(stat < 22.458, "{stat}");
    assert_eq!(r, [16_618, 20_929, 9_259, 2_542, 552, 132]);
}

/// The lag-j correlation test for j = 1 to 6, 100,000 draws each, at level 0.001: A₁ to A₆ are
/// −0.55, 0.10, 0.40, −0.11, −0.42 and −0.55.
#[test]
fn engine_generator_passes_the_correlation_tests() {
    let expected = [
        -0.546_32, 0.101_15, 0.400_16, -0.105_00, -0.420_76, -0.551_96,
    ];
    for (j, want) in (1..=6).zip(expected) {
        let a = lag_correlation(&engine_uniforms(&format!("lag-{j}"), 100_000), j);
        assert!(a.abs() < Z_9995, "A_{j} = {a}");
        assert!((a - want).abs() < 1e-5, "A_{j} = {a}");
    }
}

/// Two streams with neighbouring keys are not related: pairs (U from `element/a`, U from
/// `element/b`) drawn side by side pass the two-dimensional serial test at level 0.001:
/// χ² = 3954.0.
#[test]
fn engine_keyed_streams_side_by_side_pass_the_serial_test() {
    let mut a = Rng::keyed(1, "element/a");
    let mut b = Rng::keyed(1, "element/b");
    let mut counts = vec![0u64; 64 * 64];
    for _ in 0..32_768 {
        let (x, y) = (cell(a.uniform(), 64), cell(b.uniform(), 64));
        counts[x * 64 + y] += 1;
    }
    let stat = chi_square(&counts, 8);
    assert!(stat < chi_square_critical(4_095.0, Z_999), "{stat}");
    assert_eq!(squared_deviations(&counts, 8), 31_632);
}

/// A second-level test: the chi-square test of Example 7.6 on 100 keyed streams. At level 0.05
/// about 5 should reject; more than 13 has probability 0.00046 (binomial, n = 100, p = 0.05),
/// so the check's false-alarm level is below 0.001. Ten reject.
#[test]
fn engine_generator_rejects_at_the_expected_rate_over_many_streams() {
    let critical = chi_square_critical(4_095.0, Z_95);
    let rejected = (0..100)
        .filter(|i| {
            let mut rng = Rng::keyed(1, &format!("rng-check/chi-square/{i}"));
            chi_square(&tuple_counts(|| rng.uniform(), 4_096, 1, 32_768), 8) > critical
        })
        .count();
    assert!(rejected <= 13, "{rejected} of 100 rejected at level 0.05");
    assert_eq!(rejected, 10);
}
