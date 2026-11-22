module

public import Mathlib

/-!
# Little's law on one sample path over a finite horizon

Several tests turn a mean wait into a mean queue length (or back) with Little's law
`L = λ W`, for example `to_atm * wait` in `crates/engine/tests/examples.rs`. This file proves
the sample-path identity behind the empirical version: for finitely many customers, customer
`i` arriving at `a i` and leaving at `d i` with `0 ≤ a i ≤ d i ≤ T`, the integral over `[0, T]`
of the number in system equals the sum of the times in system. Dividing by `T` gives
`L̄ = λ̄ · W̄` with `L̄` the time-average number in system, `λ̄ = n/T` the arrival rate and `W̄`
the average time in system of the `n` customers.

It does not prove the limit form of Little's law (`L = λW` for stationary queues or as
`T → ∞`), and it does not prove that the simulator's statistics compute these quantities.
-/

@[expose] public section

namespace Formal

open MeasureTheory Set

/-- The number in system at time `t`: each customer adds 1 while `a i ≤ t < d i`. -/
noncomputable def numberInSystem {ι : Type*} (s : Finset ι) (a d : ι → ℝ) (t : ℝ) : ℝ :=
  ∑ i ∈ s, (Ico (a i) (d i)).indicator (1 : ℝ → ℝ) t

/-- `numberInSystem` counts the customers present at `t`. -/
theorem numberInSystem_eq_card {ι : Type*} (s : Finset ι) (a d : ι → ℝ) (t : ℝ) :
    numberInSystem s a d t = ((s.filter fun i => a i ≤ t ∧ t < d i).card : ℝ) := by
  classical
  simp [numberInSystem, Set.indicator_apply]

/-- **Little's law, sample path.** The integral of the number in system over `[0, T]` equals the
sum of the customers' times in system. -/
theorem integral_numberInSystem {ι : Type*} (s : Finset ι) (a d : ι → ℝ) (T : ℝ)
    (h0 : ∀ i ∈ s, 0 ≤ a i) (had : ∀ i ∈ s, a i ≤ d i) (hT : ∀ i ∈ s, d i ≤ T) :
    ∫ t in Icc 0 T, numberInSystem s a d t = ∑ i ∈ s, (d i - a i) := by
  unfold numberInSystem
  rw [integral_finsetSum]
  · refine Finset.sum_congr rfl fun i hi => ?_
    have hsub : Ico (a i) (d i) ⊆ Icc 0 T :=
      Ico_subset_Icc_self.trans (Icc_subset_Icc (h0 i hi) (hT i hi))
    rw [integral_indicator_one measurableSet_Ico, measureReal_restrict_apply measurableSet_Ico,
      inter_eq_left.mpr hsub, Real.volume_real_Ico_of_le (had i hi)]
  · intro i _
    exact (integrable_const (1 : ℝ)).indicator measurableSet_Ico

/-- **`L̄ = λ̄ · W̄`.** The time-average number in system over `[0, T]` equals the arrival rate
`n/T` times the average time in system, for `n ≥ 1` customers. -/
theorem little_sample_path {ι : Type*} (s : Finset ι) (a d : ι → ℝ) (T : ℝ)
    (h0 : ∀ i ∈ s, 0 ≤ a i) (had : ∀ i ∈ s, a i ≤ d i) (hT : ∀ i ∈ s, d i ≤ T)
    (hs : s.Nonempty) :
    (1 / T) * ∫ t in Icc 0 T, numberInSystem s a d t =
      ((s.card : ℝ) / T) * ((∑ i ∈ s, (d i - a i)) / s.card) := by
  rw [integral_numberInSystem s a d T h0 had hT]
  have hc : (s.card : ℝ) ≠ 0 := Nat.cast_ne_zero.mpr hs.card_pos.ne'
  have E : (s.card : ℝ)⁻¹ * (s.card : ℝ) = 1 := inv_mul_cancel₀ hc
  linear_combination (-(T⁻¹ * ∑ i ∈ s, (d i - a i))) * E

end Formal
