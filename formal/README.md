# Formal proofs of the theory formulas

StochCraft's validation tests run a model and require each 95% confidence interval to cover a
value from queueing theory. This folder is a small Lean 4 project, built on
[Mathlib](https://github.com/leanprover-community/mathlib4), that proves those **formulas**:
identities between the expressions the tests compute and their textbook forms, and the algebra
that follows from stated assumptions.

## What is and is not proved

The proofs establish that the formulas are what they claim to be, given their stated
assumptions. They do **not**:

- derive any formula from the dynamics of a queue (that M/M/1 has a geometric stationary
  distribution, or that M/G/1 satisfies the mean-value equation, is assumed where it is used);
- prove anything about the Rust code: the tests' functions are restated in Lean by hand, and
  floating-point rounding is not modelled;
- prove anything about the simulator or its statistics.

Formulas the tests use but this project does not prove: the M/M/c mean wait
`Wq = C/(cμ − λ)`, the triangular distribution's mean and second moment, M/M/1/K, Erlang A,
priority queues, machine repair, Jackson networks and the other cases in
[docs/developers.md](../docs/developers.md).

## Theorems

All theorems are in the namespace `Formal`; real numbers throughout.

### `Formal/ErlangB.lean` (`erlang_b` in `crates/engine/tests/examples.rs`)

- `erlangB_eq_closed`: for offered load `a ≥ 0` and every number of servers `c`, the recurrence
  `B(0) = 1`, `B(c) = a·B(c−1) / (c + a·B(c−1))` equals `(a^c/c!) / ∑_{k≤c} a^k/k!`.

### `Formal/ErlangC.lean` (`erlang_c` in the engine and blocks tests)

The tests compute `C = last / (∑_{k<c} a^k/k! + last)` with `last = (a^c/c!)/(1 − a/c)`.

- `erlangC_eq_standard`: for `0 < a < c` this equals
  `(a^c/c! · c/(c−a)) / (∑_{k<c} a^k/k! + a^c/c! · c/(c−a))`.
- `erlangC_eq_erlangB`: for `0 < a < c` it equals `c·B / (c − a(1 − B))` with `B` the Erlang B
  value from the recurrence.
- An `example` checks the M/M/2 test's value `C(2, 1.5) = 9/14`.

### `Formal/MM1.lean` (`crates/blocks/tests/queueing_theory.rs`)

With `P(n) = (1 − ρ)ρ^n` and `0 ≤ ρ < 1` (the geometric distribution is assumed, not derived):

- `mm1Prob_hasSum`: the probabilities sum to 1.
- `mm1Prob_balance`: with `ρ = λ/μ`, `λ P(n) = μ P(n + 1)` (the birth–death balance equations).
- `mm1_busy`: `1 − P(0) = ρ`, the utilisation.
- `mm1_mean_hasSum`: the mean number in system is `ρ/(1 − ρ)`.
- `mm1_queue_hasSum`: the mean number waiting, `∑ n·P(n + 1)`, is `ρ²/(1 − ρ)`.
- `mm1_time_in_system`, `mm1_wait`: for `0 < λ < μ`, dividing by `λ` (Little's law, taken as
  given here) gives `W = 1/(μ − λ)` and `Wq = ρ/(μ − λ)`.

Sums are stated with Mathlib's `HasSum`, so convergence is part of each statement.

### `Formal/Little.lean`

For finitely many customers, customer `i` present on `[a i, d i)` with `0 ≤ a i ≤ d i ≤ T`:

- `numberInSystem_eq_card`: the number in system at `t` is the count of customers with
  `a i ≤ t < d i`.
- `integral_numberInSystem`: the integral of the number in system over `[0, T]` equals
  `∑ (d i − a i)`, the total time in system.
- `little_sample_path`: hence `L̄ = λ̄ · W̄` with `L̄` the time average over `[0, T]`,
  `λ̄ = n/T` and `W̄` the mean time in system of the `n ≥ 1` customers.

This is the finite-horizon sample-path identity, not the limit theorem `L = λW`.

### `Formal/PollaczekKhinchine.lean` (`pollaczek_khinchine` in `crates/engine/tests/examples.rs`)

With `Wq = λ E[S²] / (2(1 − λ E[S]))`:

- `pkWait_mean_value`: if `λ E[S] < 1`, the formula satisfies the mean-value equation
  `W = λ E[S] W + λ E[S²]/2`.
- `pkWait_of_mean_value`: if `λ E[S] < 1` and `W = λ E[S] W + λ E[S²]/2` (the mean-value
  equation from mean residual work and Little's law, assumed), then `W` is the formula.
- `pkWait_exponential`: with `E[S] = 1/μ`, `E[S²] = 2/μ²` and `λ < μ` it is `ρ/(μ − λ)`, the
  M/M/1 wait.
- `pkWait_lt_of_second_lt`: with the same mean, a larger second moment gives a longer wait
  (the claim of `the_same_mean_with_more_spread_waits_longer_by_pollaczek_khinchine`).
- An `example` checks the bank example's 8.75 minutes from the test's triangular moments.

## Checking the proofs

Install [elan](https://github.com/leanprover/elan), then in this folder:

```sh
lake exe cache get   # download Mathlib's prebuilt files (about 9,000 files) instead of compiling it
lake build --wfail   # warnings, including any `sorry`, are errors
```

`lean-toolchain` pins Lean and `lakefile.toml` pins the Mathlib release that matches it; `lake-manifest.json` records the exact commits of
Mathlib and its dependencies (Mathlib's own manifest at that tag). CI
(`.github/workflows/lean.yml`) runs the same build when `formal/` changes. Mathlib is
Apache-2.0; it is fetched at build time into `.lake/` and not committed.
