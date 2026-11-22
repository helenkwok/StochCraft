import Mathlib

/-!
# M/M/1: the geometric distribution and its means

The M/M/1 test in `crates/blocks/tests/queueing_theory.rs` compares the simulation with
`W = 1/(μ − λ)`, `Lq = ρ²/(1 − ρ)`, `Wq = Lq/λ` and utilisation `ρ = λ/μ`. Taking as given that
the stationary number in system is geometric, `P(n) = (1 − ρ) ρ^n` with `0 ≤ ρ < 1`, this file
proves that

* the probabilities sum to 1;
* they satisfy the birth–death balance equations `λ P(n) = μ P(n + 1)` when `ρ = λ/μ`;
* the server is busy with probability `1 − P(0) = ρ`;
* the mean number in system is `L = ρ/(1 − ρ)`;
* the mean number waiting is `Lq = ρ²/(1 − ρ)` (with `n + 1` in system, `n` wait);
* with Little's law taken as given, `W = L/λ = 1/(μ − λ)` and `Wq = Lq/λ = ρ/(μ − λ)`.

It does not prove that the M/M/1 queue has this stationary distribution (that needs the
Markov-chain theory of the queue), nor anything about the Rust code.
-/

namespace Formal

/-- The geometric distribution `P(n) = (1 − ρ) ρ^n`. -/
noncomputable def mm1Prob (ρ : ℝ) (n : ℕ) : ℝ := (1 - ρ) * ρ ^ n

/-- The probabilities sum to 1. -/
theorem mm1Prob_hasSum {ρ : ℝ} (h0 : 0 ≤ ρ) (h1 : ρ < 1) : HasSum (mm1Prob ρ) 1 := by
  have h := (hasSum_geometric_of_lt_one h0 h1).mul_left (1 - ρ)
  rw [mul_inv_cancel₀ (sub_pos.mpr h1).ne'] at h
  exact h

/-- The balance equations of the birth–death chain: `λ P(n) = μ P(n + 1)` when `ρ = λ/μ`. -/
theorem mm1Prob_balance {lam mu : ℝ} (hmu : mu ≠ 0) (n : ℕ) :
    lam * mm1Prob (lam / mu) n = mu * mm1Prob (lam / mu) (n + 1) := by
  have h : mu * (lam / mu) = lam := by
    rw [mul_div_assoc']
    exact mul_div_cancel_left₀ lam hmu
  simp only [mm1Prob, pow_succ]
  linear_combination (-((1 - lam / mu) * (lam / mu) ^ n)) * h

/-- The server is busy with probability `1 − P(0) = ρ`. -/
theorem mm1_busy (ρ : ℝ) : 1 - mm1Prob ρ 0 = ρ := by
  simp [mm1Prob]

/-- The mean number in system: `∑ n P(n) = ρ/(1 − ρ)`. -/
theorem mm1_mean_hasSum {ρ : ℝ} (h0 : 0 ≤ ρ) (h1 : ρ < 1) :
    HasSum (fun n : ℕ => (n : ℝ) * mm1Prob ρ n) (ρ / (1 - ρ)) := by
  have hn : ‖ρ‖ < 1 := by
    rw [Real.norm_eq_abs, abs_of_nonneg h0]
    exact h1
  have hne : (1 - ρ) ≠ 0 := (sub_pos.mpr h1).ne'
  have h := (hasSum_coe_mul_geometric_of_norm_lt_one hn).mul_left (1 - ρ)
  have hv : (1 - ρ) * (ρ / (1 - ρ) ^ 2) = ρ / (1 - ρ) := by
    rw [pow_two, mul_div_assoc', mul_div_mul_left _ _ hne]
  rw [hv] at h
  convert h using 1
  funext n
  simp only [mm1Prob]
  ring

/-- The mean number waiting: with `n + 1` in system, `n` wait, so
`Lq = ∑ n P(n + 1) = ρ²/(1 − ρ)`. -/
theorem mm1_queue_hasSum {ρ : ℝ} (h0 : 0 ≤ ρ) (h1 : ρ < 1) :
    HasSum (fun n : ℕ => (n : ℝ) * mm1Prob ρ (n + 1)) (ρ ^ 2 / (1 - ρ)) := by
  have h := (mm1_mean_hasSum h0 h1).mul_left ρ
  have hv : ρ * (ρ / (1 - ρ)) = ρ ^ 2 / (1 - ρ) := by ring
  rw [hv] at h
  convert h using 1
  funext n
  simp only [mm1Prob]
  ring

/-- With Little's law, `W = L/λ = 1/(μ − λ)` for `0 < λ < μ` and `ρ = λ/μ`. -/
theorem mm1_time_in_system {lam mu : ℝ} (hl : 0 < lam) (hlm : lam < mu) :
    lam / mu / (1 - lam / mu) / lam = 1 / (mu - lam) := by
  have hmu : 0 < mu := hl.trans hlm
  have hd : (mu - lam) ≠ 0 := (sub_pos.mpr hlm).ne'
  have h1 : 1 - lam / mu = (mu - lam) / mu := by rw [sub_div, div_self hmu.ne']
  rw [h1, div_div_eq_mul_div, div_eq_div_iff hl.ne' hd]
  have E1 : mu⁻¹ * mu = 1 := inv_mul_cancel₀ hmu.ne'
  have E2 : (mu - lam)⁻¹ * (mu - lam) = 1 := inv_mul_cancel₀ hd
  linear_combination (lam * ((mu - lam)⁻¹ * (mu - lam))) * E1 + lam * E2

/-- With Little's law, `Wq = Lq/λ = ρ/(μ − λ)` for `0 < λ < μ` and `ρ = λ/μ`. -/
theorem mm1_wait {lam mu : ℝ} (hl : 0 < lam) (hlm : lam < mu) :
    (lam / mu) ^ 2 / (1 - lam / mu) / lam = lam / mu / (mu - lam) := by
  have hmu : 0 < mu := hl.trans hlm
  have hd : (mu - lam) ≠ 0 := (sub_pos.mpr hlm).ne'
  have h1 : 1 - lam / mu = (mu - lam) / mu := by rw [sub_div, div_self hmu.ne']
  rw [h1, div_div_eq_mul_div, div_eq_div_iff hl.ne' hd]
  have E1 : mu⁻¹ * mu = 1 := inv_mul_cancel₀ hmu.ne'
  have E2 : (mu - lam)⁻¹ * (mu - lam) = 1 := inv_mul_cancel₀ hd
  linear_combination (lam ^ 2 * mu⁻¹ * ((mu - lam)⁻¹ * (mu - lam))) * E1 + (lam ^ 2 * mu⁻¹) * E2

/-- The test's numbers: `λ = 0.75`, `μ = 1` give `L = 3`, `Lq = 2.25`, `W = 4` and `Wq = 3`. -/
example : (0.75 : ℝ) / (1 - 0.75) = 3 ∧ (0.75 : ℝ) ^ 2 / (1 - 0.75) = 2.25 ∧
    (1 : ℝ) / (1 - 0.75) = 4 ∧ (0.75 : ℝ) / 1 / (1 - 0.75) = 3 := by
  norm_num

end Formal
