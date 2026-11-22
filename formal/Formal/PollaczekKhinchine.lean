module

public import Mathlib

/-!
# Pollaczek–Khinchine mean wait: algebra and consistency checks

The tests use the M/G/1 mean wait `Wq = λ E[S²] / (2 (1 − λ E[S]))` (`pollaczek_khinchine` in
`crates/engine/tests/examples.rs`). This file proves only algebra about that formula:

* it is the unique solution of the mean-value equation `W = λ E[S] W + λ E[S²]/2` when
  `λ E[S] < 1` (that equation, from mean residual work and Little's law, is assumed, not
  proved);
* with exponential service (`E[S] = 1/μ`, `E[S²] = 2/μ²`) it reduces to the M/M/1 wait
  `ρ/(μ − λ)`;
* with the same mean, a larger second moment gives a longer wait;
* the bank example's value, 8.75 minutes.

It does not derive the formula from the M/G/1 queue and says nothing about the Rust code.
-/

@[expose] public section

namespace Formal

/-- The Pollaczek–Khinchine mean wait with arrival rate `lam`, mean service time `m` and second
moment of the service time `s2`. -/
noncomputable def pkWait (lam m s2 : ℝ) : ℝ := lam * s2 / (2 * (1 - lam * m))

/-- The formula solves the mean-value equation `W = λ m W + λ s2 / 2` when `λ m < 1`. -/
theorem pkWait_of_mean_value {lam m s2 W : ℝ} (hρ : lam * m < 1)
    (h : W = lam * m * W + lam * s2 / 2) : W = pkWait lam m s2 := by
  have hne : 2 * (1 - lam * m) ≠ 0 := mul_ne_zero two_ne_zero (sub_pos.mpr hρ).ne'
  rw [pkWait, eq_div_iff hne]
  linear_combination 2 * h

/-- Exponential service, `E[S] = 1/μ` and `E[S²] = 2/μ²`: the M/M/1 wait `ρ/(μ − λ)`. -/
theorem pkWait_exponential {lam mu : ℝ} (hmu : 0 < mu) (hlm : lam < mu) :
    pkWait lam (1 / mu) (2 / mu ^ 2) = lam / mu / (mu - lam) := by
  have hρ : lam * (1 / mu) < 1 := by
    rw [mul_one_div, div_lt_one hmu]
    exact hlm
  have h1 : 2 * (1 - lam * (1 / mu)) ≠ 0 := mul_ne_zero two_ne_zero (sub_pos.mpr hρ).ne'
  have h2 : mu - lam ≠ 0 := (sub_pos.mpr hlm).ne'
  rw [pkWait, div_eq_div_iff h1 h2]
  have E : mu⁻¹ * mu = 1 := inv_mul_cancel₀ hmu.ne'
  linear_combination (2 * lam * mu⁻¹) * E

/-- More spread, same mean: a larger second moment gives a strictly longer wait. -/
theorem pkWait_lt_of_second_lt {lam m s2 s2' : ℝ} (hl : 0 < lam) (hρ : lam * m < 1)
    (hs : s2 < s2') : pkWait lam m s2 < pkWait lam m s2' := by
  have hd : 0 < 2 * (1 - lam * m) := mul_pos two_pos (sub_pos.mpr hρ)
  unfold pkWait
  rw [div_lt_div_iff_of_pos_right hd]
  nlinarith

/-- The bank example: λ = 0.375 a minute at the ATM, triangular(1, 2, 4) service with mean
`(1 + 2 + 4)/3` and variance `(1 + 4 + 16 − 2 − 4 − 8)/18` (the test's moment formulas, taken as
given), so `Wq = 8.75` minutes. -/
example :
    pkWait 0.375 ((1 + 2 + 4) / 3) ((1 + 4 + 16 - 2 - 4 - 8) / 18 + ((1 + 2 + 4) / 3) ^ 2) =
      8.75 := by
  norm_num [pkWait]

end Formal
