module

public import Mathlib

/-!
# Erlang B: the recurrence equals the closed form

The validation tests compute the Erlang B blocking probability with the recurrence
`B(0) = 1`, `B(c) = a·B(c−1) / (c + a·B(c−1))` (`erlang_b` in `crates/engine/tests/examples.rs`).
This file proves that the recurrence gives the textbook closed form
`B(c) = (a^c / c!) / ∑_{k ≤ c} a^k / k!` for every offered load `a ≥ 0`.

It is a statement about the formula only: it does not derive the formula from a loss system's
dynamics, and it says nothing about the Rust code or floating-point rounding.
-/

@[expose] public section

namespace Formal

open Finset

/-- The partial sum `∑_{k < n} a^k / k!` of the exponential series. -/
noncomputable def expPartial (a : ℝ) (n : ℕ) : ℝ :=
  ∑ k ∈ range n, a ^ k / (k.factorial : ℝ)

/-- Erlang B by the recurrence the tests use. -/
noncomputable def erlangB (a : ℝ) : ℕ → ℝ
  | 0 => 1
  | c + 1 => a * erlangB a c / ((c + 1 : ℝ) + a * erlangB a c)

/-- Erlang B in closed form: `(a^c / c!) / ∑_{k ≤ c} a^k / k!`. -/
noncomputable def erlangBClosed (a : ℝ) (c : ℕ) : ℝ :=
  a ^ c / (c.factorial : ℝ) / expPartial a (c + 1)

theorem expPartial_succ (a : ℝ) (n : ℕ) :
    expPartial a (n + 1) = expPartial a n + a ^ n / (n.factorial : ℝ) := by
  unfold expPartial
  rw [Finset.sum_range_succ]

theorem expPartial_nonneg {a : ℝ} (ha : 0 ≤ a) (n : ℕ) : 0 ≤ expPartial a n := by
  unfold expPartial
  exact Finset.sum_nonneg fun k _ => by positivity

theorem expPartial_succ_pos {a : ℝ} (ha : 0 ≤ a) (n : ℕ) : 0 < expPartial a (n + 1) := by
  induction n with
  | zero => simp [expPartial]
  | succ m ih =>
    rw [expPartial_succ]
    have hm : 0 ≤ a ^ (m + 1) / ((m + 1).factorial : ℝ) := by positivity
    linarith

/-- The one algebraic step of the induction, with the terms named. -/
theorem erlangB_step {a t S m : ℝ} (hS : S ≠ 0) (hm : m ≠ 0) (hd1 : m + a * (t / S) ≠ 0)
    (hd2 : S + a * t / m ≠ 0) :
    a * (t / S) / (m + a * (t / S)) = a * t / m / (S + a * t / m) := by
  rw [div_eq_div_iff hd1 hd2]
  have hS1 : S⁻¹ * S = 1 := inv_mul_cancel₀ hS
  have hm1 : m⁻¹ * m = 1 := inv_mul_cancel₀ hm
  linear_combination (a * t) * hS1 - (a * t) * hm1

/-- **Erlang B.** For `a ≥ 0` the recurrence `B(0) = 1`, `B(c) = a·B(c−1)/(c + a·B(c−1))`
equals `(a^c / c!) / ∑_{k ≤ c} a^k / k!` for every number of servers `c`. -/
theorem erlangB_eq_closed {a : ℝ} (ha : 0 ≤ a) (c : ℕ) : erlangB a c = erlangBClosed a c := by
  induction c with
  | zero => simp [erlangB, erlangBClosed, expPartial]
  | succ n ih =>
    have hS : 0 < expPartial a (n + 1) := expPartial_succ_pos ha n
    have ht : 0 ≤ a ^ n / (n.factorial : ℝ) := by positivity
    have hn : (0 : ℝ) < (n : ℝ) + 1 := by positivity
    have e : a ^ (n + 1) / ((n + 1).factorial : ℝ) = a * (a ^ n / (n.factorial : ℝ)) / (n + 1) := by
      rw [Nat.factorial_succ, Nat.cast_mul, ← div_div]
      push_cast
      ring
    have hd1 : (n : ℝ) + 1 + a * (a ^ n / (n.factorial : ℝ) / expPartial a (n + 1)) ≠ 0 := by
      have : 0 ≤ a * (a ^ n / (n.factorial : ℝ) / expPartial a (n + 1)) := by positivity
      linarith
    have hd2 : expPartial a (n + 1) + a * (a ^ n / (n.factorial : ℝ)) / ((n : ℝ) + 1) ≠ 0 := by
      have : 0 ≤ a * (a ^ n / (n.factorial : ℝ)) / ((n : ℝ) + 1) := by positivity
      linarith
    rw [erlangB, ih, erlangBClosed, erlangBClosed, expPartial_succ a (n + 1), e]
    exact erlangB_step hS.ne' hn.ne' hd1 hd2

end Formal
