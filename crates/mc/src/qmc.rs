//! Randomised quasi-Monte Carlo (ROADMAP M4.1 part 2b): Sobol points, scrambled and shifted at
//! random for each batch.
//!
//! **Points.** The Sobol sequence (Sobol', 1967) in Gray-code order, one dimension for each
//! input in declaration order. Its direction numbers are the "new-joe-kuo-6.21201" set of Joe
//! and Kuo, "Constructing Sobol sequences with better two-dimensional projections", *SIAM
//! Journal on Scientific Computing* 30(5), 2635-2654 (2008), the first 64 dimensions, built
//! from the recurrence of their paper: with a primitive polynomial of degree s whose
//! coefficients a_1 ... a_{s-1} are the binary digits of `a`, and initial values m_1 ... m_s,
//! the direction integers are V_i = m_i · 2^(32 - i) for i ≤ s and
//! V_i = V_{i-s} ⊕ (V_{i-s} ≫ s) ⊕ (⊕ over k < s with a_k = 1 of V_{i-k}) after. Dimension 1
//! is the van der Corput sequence (every m_i = 1). Point n is the exclusive or of the V_i for
//! each set bit i - 1 of n ⊕ (n ≫ 1), and its first points are those the authors publish
//! (`tests/variance.rs`). A batch takes the points from index 0 on and none is dropped: dropping
//! the first points breaks the net property and slows the convergence of a randomised net (Owen,
//! "On dropping the first Sobol' point", 2020).
//!
//! **Licence.** The direction numbers are covered by the authors' BSD-style licence, kept below,
//! in NOTICE and in ATTRIBUTION.md.
//!
//! **Randomisation.** Each dimension of each batch gets a random linear matrix scramble followed
//! by a random digital shift ([`Scramble`], [`scrambled`]): Matoušek, "On the L2-discrepancy for
//! anchored boxes", *Journal of Complexity* 14, 527-556 (1998), called affine matrix scrambling
//! by Owen, "Variance with alternative scramblings of digital nets", *ACM TOMACS* 13(4), 363-378
//! (2003). With the binary digits of a coordinate written most significant first, the scrambled
//! digits are x = L·y ⊕ e, where y are the digits of the Sobol point, L is a random 32 × 32
//! lower-triangular binary matrix with ones on its diagonal and independent fair bits below it,
//! and e is a random 32-bit shift. Digit k of x so depends only on digits 1 ... k of y, through an
//! invertible map, so every elementary interval [j/2^k, (j+1)/2^k) goes onto another one whole
//! and the points keep the (t, m, s)-net property of the sequence: of the first 2^m points, each
//! such box of volume 2^(t-m) still holds exactly 2^t. As the map is linear it is applied to the
//! direction integers once (the scrambled generator matrix is L·C) and the Gray-code walk runs
//! over them unchanged, the way Hong and Hickernell, "Algorithm 823: Implementing scrambled
//! digital sequences", *ACM TOMS* 29(2), 95-109 (2003), scramble digital sequences through their
//! generator matrices. The shift makes every point uniform on its own (Owen, 2003, Proposition
//! 3.1), so the estimate of a mean is unbiased, and batches scrambled independently are
//! independent replicates, which is what the interval across batches needs. The coordinate is
//! taken at the middle of its 2^-32 cell, so it is never exactly 0 or 1.
//!
//! **Why the matrix.** A digital shift alone moves every point of a dimension by the same
//! exclusive or, so all of the first 2^m points keep one common offset inside their intervals of
//! width 2^-m: for f(x) = x the variance of the mean of 2^m shifted van der Corput points is
//! 1/(12 n²), where Owen's nested uniform scrambling ("Randomly permuted (t, m, s)-nets and
//! (t, s)-sequences", 1995) gives 1/(12 n³) (Owen, 2003, §3.1). The matrix mixes the leading
//! digits into the trailing ones, so each point gets an offset of its own. For a pair of points
//! whose digits first differ at position r, the scrambled digits after r differ by a fresh fair
//! bit of L's column r, independent of the shift, which is the same joint law nested uniform
//! scrambling gives the pair; the variance of a mean depends on the points only through such
//! pairs, so random linear scrambling with a digital shift has the sampling variance of nested
//! uniform scrambling (Owen, "Local antithetic sampling with scrambled nets", *Annals of
//! Statistics* 36(5), 2319-2343, 2008) at a cost of 32 words a dimension. For smooth
//! integrands that variance falls as O(n^-3 (log n)^(s-1)) at n = 2^m points (Owen, "Scrambled net
//! variance for integrals of smooth functions", *Annals of Statistics* 25(4), 1997). Applied
//! evidence for scrambled nets over independent draws: Sándor and Train, "Quasi-random simulation
//! of discrete choice models", *Transportation Research B* 38, 313-327 (2004), where, in
//! simulated maximum likelihood for a mixed logit model, 64 points of the better scrambled nets
//! in five dimensions did as well as 512 independent draws. Adding a uniform shift
//! modulo 1 instead (Tuffin, 1996; a Cranley-Patterson rotation) carries between digits, so a
//! box of side 2^-k moves across the edges of the boxes of the next coarser grid, and the net
//! property need not survive; the exclusive or never carries.
//!
//! A batch of `stochcraft_mc::BATCH` = 1,000 trials is not a power of two, so a run's batches get
//! the full net only for their first 512 points; the gain over the digital shift is smaller there
//! than at 1,024 points (`tests/scramble.rs`).
//!
//! **Streams.** The scramble of input `x` in batch `b` comes from the stream keyed
//! `mc/qmc/<b>/<x>`: its first draw gives the shift (the shift that stream gave before matrices
//! were added) and the next 31 give rows 2 to 32 of L, so adding an input leaves the other
//! inputs' points as they were (provided it is added after them: the dimension is the input's
//! place in the declaration order).
//!
//! [`shifted`] keeps the digital shift alone, for code that wants it.
//!
//! # Joe and Kuo's licence
//!
//! ```text
//! Copyright (c) 2008, Frances Y. Kuo and Stephen Joe
//! All rights reserved.
//!
//! Redistribution and use in source and binary forms, with or without
//! modification, are permitted provided that the following conditions are met:
//!
//!     * Redistributions of source code must retain the above copyright
//!       notice, this list of conditions and the following disclaimer.
//!
//!     * Redistributions in binary form must reproduce the above copyright
//!       notice, this list of conditions and the following disclaimer in the
//!       documentation and/or other materials provided with the distribution.
//!
//!     * Neither the names of the copyright holders nor the names of the
//!       University of New South Wales and the University of Waikato
//!       and its contributors may be used to endorse or promote products derived
//!       from this software without specific prior written permission.
//!
//! THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS ``AS IS'' AND ANY
//! EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
//! WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//! DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDERS BE LIABLE FOR ANY
//! DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
//! (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
//! LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
//! ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
//! (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
//! SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
//! ```

/// The most dimensions (inputs) a Sobol run has: the first 64 of Joe and Kuo's set.
pub const MAX_DIMENSIONS: usize = 64;

/// Bits of each coordinate, and rows of a [`Scramble`] matrix.
pub const BITS: usize = 32;

/// 2^32, to scale a coordinate into [0, 1).
const SCALE: f64 = 4_294_967_296.0;

/// Dimensions 2 to 64: the degree s of the primitive polynomial, its number a, and the initial
/// direction numbers m_1 ... m_s (padded with 0 to nine) from "new-joe-kuo-6.21201".
#[rustfmt::skip]
const JOE_KUO: [(usize, u32, [u32; 9]); MAX_DIMENSIONS - 1] = [
    // dimension 2
    (1, 0, [1, 0, 0, 0, 0, 0, 0, 0, 0]),
    // dimension 3
    (2, 1, [1, 3, 0, 0, 0, 0, 0, 0, 0]),
    // dimension 4
    (3, 1, [1, 3, 1, 0, 0, 0, 0, 0, 0]),
    // dimension 5
    (3, 2, [1, 1, 1, 0, 0, 0, 0, 0, 0]),
    // dimension 6
    (4, 1, [1, 1, 3, 3, 0, 0, 0, 0, 0]),
    // dimension 7
    (4, 4, [1, 3, 5, 13, 0, 0, 0, 0, 0]),
    // dimension 8
    (5, 2, [1, 1, 5, 5, 17, 0, 0, 0, 0]),
    // dimension 9
    (5, 4, [1, 1, 5, 5, 5, 0, 0, 0, 0]),
    // dimension 10
    (5, 7, [1, 1, 7, 11, 19, 0, 0, 0, 0]),
    // dimension 11
    (5, 11, [1, 1, 5, 1, 1, 0, 0, 0, 0]),
    // dimension 12
    (5, 13, [1, 1, 1, 3, 11, 0, 0, 0, 0]),
    // dimension 13
    (5, 14, [1, 3, 5, 5, 31, 0, 0, 0, 0]),
    // dimension 14
    (6, 1, [1, 3, 3, 9, 7, 49, 0, 0, 0]),
    // dimension 15
    (6, 13, [1, 1, 1, 15, 21, 21, 0, 0, 0]),
    // dimension 16
    (6, 16, [1, 3, 1, 13, 27, 49, 0, 0, 0]),
    // dimension 17
    (6, 19, [1, 1, 1, 15, 7, 5, 0, 0, 0]),
    // dimension 18
    (6, 22, [1, 3, 1, 15, 13, 25, 0, 0, 0]),
    // dimension 19
    (6, 25, [1, 1, 5, 5, 19, 61, 0, 0, 0]),
    // dimension 20
    (7, 1, [1, 3, 7, 11, 23, 15, 103, 0, 0]),
    // dimension 21
    (7, 4, [1, 3, 7, 13, 13, 15, 69, 0, 0]),
    // dimension 22
    (7, 7, [1, 1, 3, 13, 7, 35, 63, 0, 0]),
    // dimension 23
    (7, 8, [1, 3, 5, 9, 1, 25, 53, 0, 0]),
    // dimension 24
    (7, 14, [1, 3, 1, 13, 9, 35, 107, 0, 0]),
    // dimension 25
    (7, 19, [1, 3, 1, 5, 27, 61, 31, 0, 0]),
    // dimension 26
    (7, 21, [1, 1, 5, 11, 19, 41, 61, 0, 0]),
    // dimension 27
    (7, 28, [1, 3, 5, 3, 3, 13, 69, 0, 0]),
    // dimension 28
    (7, 31, [1, 1, 7, 13, 1, 19, 1, 0, 0]),
    // dimension 29
    (7, 32, [1, 3, 7, 5, 13, 19, 59, 0, 0]),
    // dimension 30
    (7, 37, [1, 1, 3, 9, 25, 29, 41, 0, 0]),
    // dimension 31
    (7, 41, [1, 3, 5, 13, 23, 1, 55, 0, 0]),
    // dimension 32
    (7, 42, [1, 3, 7, 3, 13, 59, 17, 0, 0]),
    // dimension 33
    (7, 50, [1, 3, 1, 3, 5, 53, 69, 0, 0]),
    // dimension 34
    (7, 55, [1, 1, 5, 5, 23, 33, 13, 0, 0]),
    // dimension 35
    (7, 56, [1, 1, 7, 7, 1, 61, 123, 0, 0]),
    // dimension 36
    (7, 59, [1, 1, 7, 9, 13, 61, 49, 0, 0]),
    // dimension 37
    (7, 62, [1, 3, 3, 5, 3, 55, 33, 0, 0]),
    // dimension 38
    (8, 14, [1, 3, 1, 15, 31, 13, 49, 245, 0]),
    // dimension 39
    (8, 21, [1, 3, 5, 15, 31, 59, 63, 97, 0]),
    // dimension 40
    (8, 22, [1, 3, 1, 11, 11, 11, 77, 249, 0]),
    // dimension 41
    (8, 38, [1, 3, 1, 11, 27, 43, 71, 9, 0]),
    // dimension 42
    (8, 47, [1, 1, 7, 15, 21, 11, 81, 45, 0]),
    // dimension 43
    (8, 49, [1, 3, 7, 3, 25, 31, 65, 79, 0]),
    // dimension 44
    (8, 50, [1, 3, 1, 1, 19, 11, 3, 205, 0]),
    // dimension 45
    (8, 52, [1, 1, 5, 9, 19, 21, 29, 157, 0]),
    // dimension 46
    (8, 56, [1, 3, 7, 11, 1, 33, 89, 185, 0]),
    // dimension 47
    (8, 67, [1, 3, 3, 3, 15, 9, 79, 71, 0]),
    // dimension 48
    (8, 70, [1, 3, 7, 11, 15, 39, 119, 27, 0]),
    // dimension 49
    (8, 84, [1, 1, 3, 1, 11, 31, 97, 225, 0]),
    // dimension 50
    (8, 97, [1, 1, 1, 3, 23, 43, 57, 177, 0]),
    // dimension 51
    (8, 103, [1, 3, 7, 7, 17, 17, 37, 71, 0]),
    // dimension 52
    (8, 115, [1, 3, 1, 5, 27, 63, 123, 213, 0]),
    // dimension 53
    (8, 122, [1, 1, 3, 5, 11, 43, 53, 133, 0]),
    // dimension 54
    (9, 8, [1, 3, 5, 5, 29, 17, 47, 173, 479]),
    // dimension 55
    (9, 13, [1, 3, 3, 11, 3, 1, 109, 9, 69]),
    // dimension 56
    (9, 16, [1, 1, 1, 5, 17, 39, 23, 5, 343]),
    // dimension 57
    (9, 22, [1, 3, 1, 5, 25, 15, 31, 103, 499]),
    // dimension 58
    (9, 25, [1, 1, 1, 11, 11, 17, 63, 105, 183]),
    // dimension 59
    (9, 44, [1, 1, 5, 11, 9, 29, 97, 231, 363]),
    // dimension 60
    (9, 47, [1, 1, 5, 15, 19, 45, 41, 7, 383]),
    // dimension 61
    (9, 52, [1, 3, 7, 7, 31, 19, 83, 137, 221]),
    // dimension 62
    (9, 55, [1, 1, 1, 3, 23, 15, 111, 223, 83]),
    // dimension 63
    (9, 59, [1, 1, 5, 13, 31, 15, 55, 25, 161]),
    // dimension 64
    (9, 62, [1, 1, 3, 13, 25, 47, 39, 87, 257]),
];

/// The direction integers V_1 ... V_32 of dimension `dim` (counting from 0), at positions 0 to
/// 31, or `None` beyond [`MAX_DIMENSIONS`].
fn directions(dim: usize) -> Option<[u32; BITS]> {
    let mut v = [0_u32; BITS];
    if dim == 0 {
        for (i, slot) in v.iter_mut().enumerate() {
            *slot = 1 << (BITS - 1 - i);
        }
        return Some(v);
    }
    let &(s, a, m) = JOE_KUO.get(dim - 1)?;
    for i in 0..BITS {
        let value = if i < s {
            m.get(i).copied().unwrap_or(0) << (BITS - 1 - i)
        } else {
            let base = v.get(i - s).copied().unwrap_or(0);
            let mut x = base ^ (base >> s);
            for k in 1..s {
                if (a >> (s - 1 - k)) & 1 == 1 {
                    x ^= v.get(i - k).copied().unwrap_or(0);
                }
            }
            x
        };
        if let Some(slot) = v.get_mut(i) {
            *slot = value;
        }
    }
    Some(v)
}

/// The first `n` points of Sobol dimension `dim` (counting from 0) as 32-bit integer
/// coordinates, point 0 first; each is its coordinate times 2^32. `None` beyond
/// [`MAX_DIMENSIONS`].
pub fn integers(dim: usize, n: usize) -> Option<Vec<u32>> {
    Some(walk(&directions(dim)?, n))
}

/// The first `n` points of the digital sequence with direction integers `v`, in Gray-code order.
fn walk(v: &[u32; BITS], n: usize) -> Vec<u32> {
    let mut out = Vec::with_capacity(n);
    let mut x = 0_u32;
    for i in 0..n {
        if i > 0 {
            // Gray code: point i differs from point i - 1 by the direction of the lowest zero
            // bit of i - 1.
            let c = (i - 1).trailing_ones() as usize;
            x ^= v.get(c).copied().unwrap_or(0);
        }
        out.push(x);
    }
    out
}

/// The first `n` points of Sobol dimension `dim` in [0, 1), unshifted (the first is 0).
pub fn points(dim: usize, n: usize) -> Option<Vec<f64>> {
    Some(
        integers(dim, n)?
            .into_iter()
            .map(|x| f64::from(x) / SCALE)
            .collect(),
    )
}

/// The first `n` points of Sobol dimension `dim` with the digital shift `shift`, each at the
/// middle of its cell, so strictly inside (0, 1).
pub fn shifted(dim: usize, n: usize, shift: u32) -> Option<Vec<f64>> {
    Some(
        integers(dim, n)?
            .into_iter()
            .map(|x| (f64::from(x ^ shift) + 0.5) / SCALE)
            .collect(),
    )
}

/// A random linear matrix scramble with a digital shift (Matoušek, 1998) of one dimension: the
/// 32-bit integer coordinate y becomes L·y ⊕ `shift`, its binary digits taken most significant
/// first.
///
/// Row k of L (counting from 0) is `rows[k]`, a mask over the input bits: its own diagonal bit
/// 31 - k is always set and only the bits above it (the more significant digits) may be, so L is
/// lower triangular with ones on the diagonal and invertible. Output bit 31 - k is the parity of
/// `y & rows[k]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scramble {
    rows: [u32; BITS],
    shift: u32,
}

impl Scramble {
    /// The scramble whose row k keeps the bits of `words[k]` above the diagonal, with the
    /// diagonal bit set and the bits below it cleared (so `words[0]` is not used), and the shift
    /// `shift`.
    pub fn new(words: [u32; BITS], shift: u32) -> Self {
        let mut rows = [0_u32; BITS];
        for (k, (row, word)) in rows.iter_mut().zip(words).enumerate() {
            // Bits 32 - k ... 31: the k digits before digit k + 1. No bits for k = 0.
            let above = !(u32::MAX >> k);
            let diagonal = 1_u32 << (BITS - 1 - k);
            *row = (word & above) | diagonal;
        }
        Self { rows, shift }
    }

    /// The identity matrix with the shift `shift`: the digital shift alone, as [`shifted`].
    pub fn shift_only(shift: u32) -> Self {
        Self::new([0; BITS], shift)
    }

    /// Draws a scramble from `rng`: the first draw gives the shift, the next 31 rows 2 to 32 of
    /// the matrix (each the top 32 bits of a draw).
    pub fn draw(rng: &mut stochcraft_random::Rng) -> Self {
        let mut top = || u32::try_from(rng.next_u64() >> 32).unwrap_or(0);
        let shift = top();
        let mut words = [0_u32; BITS];
        for word in words.iter_mut().skip(1) {
            *word = top();
        }
        Self::new(words, shift)
    }

    /// The rows of L.
    pub fn rows(&self) -> &[u32; BITS] {
        &self.rows
    }

    /// The shift.
    pub fn shift(&self) -> u32 {
        self.shift
    }

    /// L·y over the binary digits, without the shift.
    pub fn linear(&self, y: u32) -> u32 {
        let mut out = 0_u32;
        for (k, row) in self.rows.iter().enumerate() {
            out |= ((y & row).count_ones() & 1) << (BITS - 1 - k);
        }
        out
    }

    /// L·y ⊕ shift: one integer coordinate scrambled.
    pub fn apply(&self, y: u32) -> u32 {
        self.linear(y) ^ self.shift
    }
}

/// The first `n` points of Sobol dimension `dim` under `scramble`, as 32-bit integers. The
/// matrix is applied to the direction integers (L·C) and the Gray-code walk runs over them, then
/// the shift; by linearity each point is `scramble.apply` of the unscrambled one. `None` beyond
/// [`MAX_DIMENSIONS`].
pub fn scrambled_integers(dim: usize, n: usize, scramble: &Scramble) -> Option<Vec<u32>> {
    let mut v = directions(dim)?;
    for d in v.iter_mut() {
        *d = scramble.linear(*d);
    }
    let shift = scramble.shift();
    Some(walk(&v, n).into_iter().map(|x| x ^ shift).collect())
}

/// The first `n` points of Sobol dimension `dim` under `scramble`, each at the middle of its
/// cell, so strictly inside (0, 1). `None` beyond [`MAX_DIMENSIONS`].
pub fn scrambled(dim: usize, n: usize, scramble: &Scramble) -> Option<Vec<f64>> {
    Some(
        scrambled_integers(dim, n, scramble)?
            .into_iter()
            .map(|x| (f64::from(x) + 0.5) / SCALE)
            .collect(),
    )
}
