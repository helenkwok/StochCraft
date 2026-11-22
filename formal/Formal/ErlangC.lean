module

public import Formal.ErlangB

/-!
# Erlang C: the tests' formula, the textbook form, and Erlang C from Erlang B

The tests compute the Erlang C probability of waiting (`erlang_c` in
`crates/engine/tests/examples.rs`, `crates/blocks/tests/queueing_cases.rs` and
`crates/blocks/tests/resources.rs`) as `last / (∑_{k < c} a^k/k! + last)` with
`last = (a^c / c!) / (1 − a/c)`. This file proves, for `0 < a < c`, that this equals the
textbook form with `c / (c − a)` and that it equals `c·B / (c − a(1 − B))`, where `B` is the
Erlang B value of `Formal.ErlangB` (both the recurrence and the closed form).

These are identities between formulas. They do not derive Erlang C from the M/M/c queue, and
the mean wait `Wq = C / (cμ − λ)` that the tests also use is not proved here.
-/

@[expose] public section

namespace Formal

open Finset

/-- Erlang C as the tests compute it: `last / (∑_{k < c} a^k/k! + last)` with
`last = (a^c / c!) / (1 − a/c)`. -/
noncomputable def erlangC (a : ℝ) (c : ℕ) : ℝ :=
  a ^ c / (c.factorial : ℝ) / (1 - a / c) /
    (expPartial a c + a ^ c / (c.factorial : ℝ) / (1 - a / c))

/-- **Erlang C, textbook form.** For `0 < a < c`,
`C = (a^c/c! · c/(c−a)) / (∑_{k<c} a^k/k! + a^c/c! · c/(c−a))`. -/
theorem erlangC_eq_standard {a : ℝ} {c : ℕ} (ha : 0 < a) (hac : a < c) :
    erlangC a c =
      a ^ c / (c.factorial : ℝ) * (c / (c - a)) /
        (expPartial a c + a ^ c / (c.factorial : ℝ) * (c / (c - a))) := by
  have hc0 : (c : ℝ) ≠ 0 := (ha.trans hac).ne'
  have h1 : 1 - a / c = (c - a) / c := by rw [sub_div, div_self hc0]
  have h : ∀ x : ℝ, x / (1 - a / c) = x * (c / (c - a)) := by
    intro x
    rw [h1, div_div_eq_mul_div, mul_div_assoc]
  unfold erlangC
  rw [h]

/-- The algebra behind `erlangC_eq_erlangB`, with `T = a^c/c!` and `P = ∑_{k<c} a^k/k!`. -/
theorem erlangC_step {a c T P : ℝ} (ha : 0 < a) (hac : a < c) (hT : 0 < T) (hP : 0 ≤ P) :
    T * (c / (c - a)) / (P + T * (c / (c - a))) =
      c * (T / (P + T)) / (c - a * (1 - T / (P + T))) := by
  have hc : 0 < c := ha.trans hac
  have hd : 0 < c - a := sub_pos.mpr hac
  have hQ : 0 < P + T := add_pos_of_nonneg_of_pos hP hT
  have hD1 : P + T * (c / (c - a)) ≠ 0 := by
    have := mul_pos hT (div_pos hc hd)
    linarith
  have hD2 : c - a * (1 - T / (P + T)) ≠ 0 := by
    have hx : 0 ≤ T / (P + T) := div_nonneg hT.le hQ.le
    have := mul_nonneg ha.le hx
    nlinarith
  rw [div_eq_div_iff hD1 hD2]
  have E1 : (c - a)⁻¹ * (c - a) = 1 := inv_mul_cancel₀ hd.ne'
  have E2 : (P + T)⁻¹ * (P + T) = 1 := inv_mul_cancel₀ hQ.ne'
  linear_combination (T - T ^ 2 * (P + T)⁻¹) * c * E1 - (T * c) * E2

/-- **Erlang C from Erlang B.** For `0 < a < c`, the tests' Erlang C equals
`c·B / (c − a(1 − B))` where `B` is Erlang B by the recurrence. -/
theorem erlangC_eq_erlangB {a : ℝ} {c : ℕ} (ha : 0 < a) (hac : a < c) :
    erlangC a c = c * erlangB a c / (c - a * (1 - erlangB a c)) := by
  have hT : 0 < a ^ c / (c.factorial : ℝ) := by positivity
  rw [erlangC_eq_standard ha hac, erlangB_eq_closed ha.le, erlangBClosed, expPartial_succ]
  exact erlangC_step ha hac hT (expPartial_nonneg ha.le c)

/-- The M/M/2 case of `crates/blocks/tests/queueing_theory.rs`: `C(2, 1.5) = 9/14` (the test's
`4.5/7`). -/
example : erlangC 1.5 2 = 9 / 14 := by
  norm_num [erlangC, expPartial, Finset.sum_range_succ, Nat.factorial_two]

end Formal
