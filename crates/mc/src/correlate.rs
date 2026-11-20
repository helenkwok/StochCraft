//! Rank correlation between inputs by the method of Iman and Conover, "A distribution-free
//! approach to inducing rank correlation among input variables", *Communications in Statistics
//! — Simulation and Computation* B 11(3), 311–334 (1982), written here from the paper's
//! description.
//!
//! Each input's values for a batch of trials are drawn as usual (plain or Latin hypercube); only
//! the order in which they are paired across inputs changes, so every input keeps exactly its
//! own sample. For the k correlated inputs and n trials:
//!
//! 1. Scores: the van der Waerden scores Φ⁻¹(i / (n + 1)), i = 1…n, in a random order for each
//!    input (an n × k matrix R).
//! 2. The target C (k × k) is factored as C = P Pᵀ (Cholesky). Since the columns of R are only
//!    nearly uncorrelated, their own correlation T = Q Qᵀ is factored too, and R* = R (P Q⁻¹)ᵀ,
//!    whose columns have exactly the correlation C (the paper's refinement).
//! 3. Each input's values are put in the order of the ranks of its column of R*.
//!
//! The scores are normal, so the rank (Spearman) correlation of R*'s columns is
//! (6/π) arcsin(c/2) for a Pearson correlation c (Pearson, 1907; Kruskal, "Ordinal measures of
//! association", *JASA* 53, 1958). The target given is a rank correlation ρ, so C holds
//! c = 2 sin(πρ/6), which makes the induced rank correlation ρ itself rather than slightly less.

use stochcraft_random::Rng;
use stochcraft_random::math::sin;
use stochcraft_stats::spread::normal_quantile;

/// The Pearson correlation of normal scores whose rank correlation is `rank`.
pub fn score_correlation(rank: f64) -> f64 {
    2.0 * sin(std::f64::consts::PI * rank / 6.0)
}

/// The lower-triangular L with L Lᵀ = `a` (symmetric), or `None` if `a` is not positive definite
/// (a pivot at or below 1e-10).
pub fn cholesky(a: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let k = a.len();
    let mut l = vec![vec![0.0; k]; k];
    for i in 0..k {
        for j in 0..=i {
            let dot: f64 = l[i][..j].iter().zip(&l[j][..j]).map(|(x, y)| x * y).sum();
            let s = a.get(i)?.get(j).copied()? - dot;
            if i == j {
                if s.is_nan() || s <= 1e-10 {
                    return None;
                }
                l[i][j] = s.sqrt();
            } else {
                l[i][j] = s / l[j][j];
            }
        }
    }
    Some(l)
}

/// The inverse of a lower-triangular matrix with a non-zero diagonal (forward substitution).
fn lower_inverse(l: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let k = l.len();
    let mut inv = vec![vec![0.0; k]; k];
    for col in 0..k {
        for i in col..k {
            let unit = if i == col { 1.0 } else { 0.0 };
            let dot: f64 = (col..i).map(|m| l[i][m] * inv[m][col]).sum();
            inv[i][col] = (unit - dot) / l[i][i];
        }
    }
    inv
}

/// The Pearson correlation matrix of `columns` (each of the same length).
fn correlation(columns: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let k = columns.len();
    let centred: Vec<Vec<f64>> = columns
        .iter()
        .map(|c| {
            let mean = c.iter().sum::<f64>() / c.len().max(1) as f64;
            c.iter().map(|x| x - mean).collect()
        })
        .collect();
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let norms: Vec<f64> = centred.iter().map(|c| dot(c, c).sqrt()).collect();
    let mut t = vec![vec![0.0; k]; k];
    for i in 0..k {
        t[i][i] = 1.0;
        for j in 0..i {
            let r = dot(&centred[i], &centred[j]) / (norms[i] * norms[j]);
            t[i][j] = r;
            t[j][i] = r;
        }
    }
    t
}

/// A random permutation of `v` (Fisher–Yates).
pub(crate) fn shuffle<T>(v: &mut [T], rng: &mut Rng) {
    for i in (1..v.len()).rev() {
        let j = (rng.next_u64() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

/// Indices of `xs` in ascending order of value (ties in index order).
fn order(xs: &[f64]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..xs.len()).collect();
    idx.sort_by(|&a, &b| xs[a].total_cmp(&xs[b]).then(a.cmp(&b)));
    idx
}

/// Reorders `columns` (one per correlated input, each the batch's n values) so that their rank
/// correlations follow `p`, the Cholesky factor of the target matrix of score correlations.
pub fn induce(columns: &mut [Vec<f64>], p: &[Vec<f64>], rng: &mut Rng) {
    let k = columns.len();
    let n = columns.first().map_or(0, Vec::len);
    if k < 2 || n < 2 || p.len() != k {
        return;
    }
    let scores: Vec<f64> = (1..=n)
        .map(|i| normal_quantile(i as f64 / (n + 1) as f64).unwrap_or(0.0))
        .collect();
    let r: Vec<Vec<f64>> = (0..k)
        .map(|_| {
            let mut c = scores.clone();
            shuffle(&mut c, rng);
            c
        })
        .collect();
    // S = P Q⁻¹ where T = Q Qᵀ is the scores' own correlation; with too few trials for T to be
    // positive definite, S = P.
    let s: Vec<Vec<f64>> = match cholesky(&correlation(&r)) {
        Some(q) => {
            let qi = lower_inverse(&q);
            (0..k)
                .map(|i| {
                    (0..k)
                        .map(|j| (0..k).map(|m| p[i][m] * qi[m][j]).sum())
                        .collect()
                })
                .collect()
        }
        None => p.to_vec(),
    };
    for (j, column) in columns.iter_mut().enumerate() {
        // Column j of R* = R Sᵀ.
        let target: Vec<f64> = (0..n)
            .map(|t| (0..k).map(|m| r[m][t] * s[j][m]).sum())
            .collect();
        let mut sorted = column.clone();
        sorted.sort_by(f64::total_cmp);
        for (rank, &at) in order(&target).iter().enumerate() {
            column[at] = sorted[rank];
        }
    }
}

/// Average ranks (1-based) of `xs`, ties sharing the mean of their ranks.
fn ranks(xs: &[f64]) -> Vec<f64> {
    let idx = order(xs);
    let mut r = vec![0.0; xs.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && xs[idx[j + 1]] == xs[idx[i]] {
            j += 1;
        }
        let rank = (i + j) as f64 / 2.0 + 1.0;
        for &at in &idx[i..=j] {
            r[at] = rank;
        }
        i = j + 1;
    }
    r
}

/// Spearman's rank correlation of `x` and `y` (the Pearson correlation of their average ranks);
/// `None` with fewer than two pairs or a variable with no spread.
pub fn rank_correlation(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() != y.len() || x.len() < 2 {
        return None;
    }
    let t = correlation(&[ranks(x), ranks(y)]);
    let r = t[1][0];
    r.is_finite().then_some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::needless_range_loop)] // Matrix products read most plainly by index.
    fn cholesky_and_its_inverse() {
        let a = vec![
            vec![4.0, 2.0, 0.4],
            vec![2.0, 2.0, 0.5],
            vec![0.4, 0.5, 1.0],
        ];
        let l = cholesky(&a).unwrap_or_default();
        for i in 0..3 {
            for j in 0..3 {
                let back: f64 = (0..3).map(|m| l[i][m] * l[j][m]).sum();
                assert!((back - a[i][j]).abs() < 1e-12, "{i},{j}");
            }
        }
        let li = lower_inverse(&l);
        for i in 0..3 {
            for j in 0..3 {
                let id: f64 = (0..3).map(|m| l[i][m] * li[m][j]).sum();
                assert!((id - if i == j { 1.0 } else { 0.0 }).abs() < 1e-12);
            }
        }
        // Correlations that cannot all hold: a with b and b with c strongly positive, a with c
        // strongly negative.
        let bad = vec![
            vec![1.0, 0.9, -0.9],
            vec![0.9, 1.0, 0.9],
            vec![-0.9, 0.9, 1.0],
        ];
        assert!(cholesky(&bad).is_none());
    }

    #[test]
    fn ranks_share_ties() {
        assert_eq!(ranks(&[3.0, 1.0, 3.0, 2.0]), vec![3.5, 1.0, 3.5, 2.0]);
        // Exactly 1 up to rounding.
        assert!(
            rank_correlation(&[1.0, 2.0, 3.0], &[10.0, 20.0, 30.0])
                .is_some_and(|r| (r - 1.0).abs() < 1e-12)
        );
        assert_eq!(rank_correlation(&[1.0, 1.0], &[1.0, 2.0]), None);
    }

    #[test]
    fn the_scores_correlation_for_a_rank_correlation() {
        assert_eq!(score_correlation(0.0), 0.0);
        assert!((score_correlation(1.0) - 1.0).abs() < 1e-15);
        // ρ = 0.8 asks for c = 2 sin(0.8π/6) ≈ 0.813.
        assert!((score_correlation(0.8) - 0.813_473).abs() < 1e-6);
    }
}
