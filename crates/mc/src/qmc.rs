//! Randomised quasi-Monte Carlo (ROADMAP M4.1 part 2b): Sobol points with a random digital
//! shift for each batch.
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
//! (`tests/variance.rs`).
//!
//! **Licence.** The direction numbers are covered by the authors' BSD-style licence, kept below,
//! in NOTICE and in ATTRIBUTION.md.
//!
//! **Randomisation.** A random digital shift (Owen, "Randomly permuted (t, m, s)-nets and
//! (t, s)-sequences", 1995; L'Ecuyer and Lemieux, "Recent advances in randomized
//! quasi-Monte Carlo methods", 2002): each dimension's integer coordinates are exclusive-or'd
//! with one random 32-bit integer. Every shifted point is uniform on its own, so the estimate of
//! a mean is unbiased, and batches shifted independently are independent replicates, which is
//! what the interval across batches needs. The coordinate is taken at the middle of its 2^-32
//! cell, so it is never exactly 0 or 1.
//!
//! **Streams.** The shift of input `x` in batch `b` is the first draw of the stream keyed
//! `mc/qmc/<b>/<x>`, so adding an input leaves the other inputs' points as they were (provided
//! it is added after them: the dimension is the input's place in the declaration order).
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

/// Bits of each coordinate.
const BITS: usize = 32;

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
    let v = directions(dim)?;
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
    Some(out)
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
