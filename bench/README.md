# Benchmarks

M/M/1 queue, arrival rate λ = 0.9, service rate μ = 1, 1,000,000 customers, then the queue
drains (theory: mean time in system W = 10). Two workloads:

- **store**: an arrival process puts timestamps into an unbounded FIFO store; one server process
  takes them out. This is the model of Cimba's `benchmark/MM1_single.c` and `MM1_single.py`, so
  results line up with Cimba's published numbers.
- **resource**: the arrival process starts one process per customer, which acquires a single-unit
  resource. simu-des has no store, so this is the workload it is compared on.

The Rust engines all draw from the same generator (`stochcraft-random`'s xoshiro256**) with the
same seed, so they process identical customers. They report the same W to every printed digit,
which checks that their queueing behaviour matches. SimPy uses Python's own generator, so its W
differs slightly.

## Running

```bash
cargo run --release --manifest-path bench/Cargo.toml -- all 1000000 64
```

```bash
pip install simpy && python3 bench/simpy/mm1.py all 1000000
```

This is a separate Cargo project, outside the StochCraft workspace, so simu-des and its
dependencies never enter StochCraft's lock file or licence checks.

## Results (M/M/1)

Shared 4-vCPU cloud machine (noisy), 2 November 2026. Rust 1.97 release builds, the median of
three runs; SimPy 4.1.2 on Python 3.13, one run.

| Engine | Workload | Time | Customers/s | vs SimPy |
|---|---|---|---|---|
| StochCraft | store | 0.113 s | 8.8 M | **54× faster** |
| SimPy | store | 6.05 s | 0.17 M | 1× |
| StochCraft | resource | 0.31 s | 3.2 M | **31× faster** |
| simu-des 0.1.0 | resource | 0.75 s | 1.3 M | 13× faster |
| SimPy | resource | 9.65 s | 0.10 M | 1× |

Single-threaded, StochCraft is about 2.4× faster than simu-des on the same workload.

**Parallel replications** (`stochcraft-experiment`, store workload): 64 replications × 1,000,000
customers on 4 threads in 2.07 s, about 31 M customers/s, with W = 9.983 ± 0.045 (95% CI), which
covers the theoretical 10.

## Model benchmarks: tandem lines, the hold model and DEVStone

The M/M/1 workloads above measure one queue. These measure events per second on models of
different shapes and sizes, so that StochCraft can be compared across models and, with the same
models built elsewhere, across engines. Each run checks its answer against theory, and the
program exits with a failure if a check fails.

- **tandem**: lines of N = 5, 50 and 200 M/M/1 stations in series, λ = 0.9 and μ = 1 at every
  station (ρ = 0.9), 100,000 customers by default, then the line drains. Built twice: on the
  `des` crate directly (a process per station, a store in front of each, like the `store`
  workload) and through the `blocks` Builder (a source, N servers and a sink: how a StochCraft
  model runs). The line is a Jackson network, so the check is the mean time in system against
  W = N / (μ − λ), leaving out the first 10% of customers as a warm-up, and that every customer
  left. The band is three standard errors of one station's mean (from the 64 replications
  above, scaled by 1/√n): about 18% at 100,000 customers, since at ρ = 0.9 successive
  customers' times are strongly correlated. It catches a broken model, not a small bias. In one run
  of 1,000,000 customers with every customer counted (no warm-up), both paths came within 0.1%
  (`des`) and 2.7% (`blocks`) of theory at every N. The two paths draw from different streams (blocks gives each element its own), so they are
  checked against theory, not against each other.
- **hold**: the classic hold model. P = 10, 1,000 and 100,000 processes each hold for an
  exponential time (mean 1) over and over, about 2,000,000 events per population. The event list
  stays at P entries, so this measures the cost of one event at that size. Check: the number of
  events against P·(1 + T) for a run to time T, within 1%.
- **devstone**: DEVStone LI, HI and HO (Wainer, Glinsky and Gutierrez-Alcaraz, *Simulation*
  87(7), 2011) with no CPU delay in the transitions, for every width and depth w, d in 50, 100,
  150 and 200 by default. The model structure follows the description in Risco-Martín et al.,
  "Reconsidering the performance of DEVS modeling and simulation environments using the DEVStone
  benchmark" (arXiv:2402.05483): its atomic model (Algorithm 1) and the layouts of its Figures
  1 to 3. Each atomic model is a `des` process with a store as its input port. Couplings are
  flattened when the model is built: an output goes straight to the atomic models it reaches.
  A process engine has no coupled-model protocol, so HO costs what HI costs plus delivering the
  outputs that reach the top model's second port. Every run checks that its internal
  transitions, external transitions and events each equal the closed form: 1 + (d − 1)(w − 1)
  for LI and 1 + (d − 1)·w(w − 1)/2 for HI and HO. These are the paper's Eqs. 7 to 17, the same
  as Herguedas-Pinedo et al., ANNSIM'25, Eqs. 3 to 5. The time t of a model is building, running
  and tearing it down. Per type the times are aggregated as in Herguedas-Pinedo et al., Eq. 14:
  t̂ = exp(Σ C·ln(1 + t) / Σ C), with weight C = w·d for LI and w²·d for HI and HO. Over the
  three types they are combined with Eq. 16, the geometric mean. Because of the 1 + t, t̂ is
  never below 1 s, and runs well under a second all come out close to 1. t̂ depends on the grid,
  so it is comparable only between runs on the same grid. The paper used 100 to 1,200 in steps
  of 50, which here would take hours. No reference engine was run, so no RPU (Eq. 15) is
  given.

```bash
cargo run --release --manifest-path bench/Cargo.toml -- models           # all three, defaults
cargo run --release --manifest-path bench/Cargo.toml -- tandem 1000000   # customers per line
cargo run --release --manifest-path bench/Cargo.toml -- hold 2000000     # events per population
cargo run --release --manifest-path bench/Cargo.toml -- devstone 400 100 # w, d in 100..=400 step 100
```

`cargo test --release --manifest-path bench/Cargo.toml` checks the DEVStone counts on small
models, including the edge cases d = 1 and w = 2.

These cover the roadmap's "a speed benchmark" item for StochCraft alone. No other engine was run
on these models. SimPy is not installed on the machine below, so there is no SimPy tandem twin
yet.

### Results on a desktop

Windows 11 Pro desktop, Intel Core i7-12700K (8 performance and 4 efficiency cores, 20 threads),
64 GB RAM, 18 November 2026. Rust 1.95.0 release builds, the median of three runs of `models`
(the 400 grid ran once). This is a different machine from the cloud machine of the M/M/1 table
above, so do not compare the two tables row by row.

| Model | Path | Time | Station visits/s | Events/s | Check |
|---|---|---|---|---|---|
| Tandem N = 5 | des | 0.068 s | 7.4 M | 9.6 M | W 49.4 vs 50 |
| Tandem N = 5 | blocks | 0.191 s | 2.6 M | 3.9 M | W 48.8 vs 50 |
| Tandem N = 50 | des | 0.945 s | 5.3 M | 6.0 M | W 486 vs 500 |
| Tandem N = 50 | blocks | 2.31 s | 2.2 M | 2.5 M | W 483 vs 500 |
| Tandem N = 200 | des | 5.37 s | 3.7 M | 4.1 M | W 2,028 vs 2,000 |
| Tandem N = 200 | blocks | 9.98 s | 2.0 M | 2.2 M | W 1,926 vs 2,000 |
| Hold P = 10 | des | 0.217 s | | 9.2 M | +0.02% events |
| Hold P = 1,000 | des | 0.513 s | | 3.9 M | +0.02% events |
| Hold P = 100,000 | des | 2.32 s | | 0.91 M | +0.01% events |

Events are process activations on `des` and the block layer's events on `blocks`, so they are
not counted the same way on the two paths. Station visits (customers × N) are, and through
`blocks` a line costs about 2 to 3 times what it costs on `des` alone. The cost of an event
grows with the number of processes waiting (the hold model, and longer lines).

| DEVStone grid | LI t̂ | HI t̂ | HO t̂ | LI, HI, HO t̂ | Transitions/s (LI, HI, HO) |
|---|---|---|---|---|---|
| w, d in 50..=200 step 50 (default) | 1.037 s | 1.709 s | 1.744 s | 1.457 s | 1.2 M, 5.9 M, 5.6 M |
| w, d in 100..=400 step 100 | 1.171 s | 6.873 s | 7.886 s | 3.989 s | 1.1 M, 5.1 M, 4.3 M |

Every DEVStone run matched the closed forms exactly; HI and HO at w = d = 400 are 31,840,201
events each and took 13.8 s and 16.1 s. LI moves few events per atomic model, so its rate is
dominated by building about (w − 1)(d − 1) processes.

On this machine the M/M/1 workloads (`all 1000000 64`, median of three) ran at 5.6 M customers/s
(store), 2.6 M (resource), 1.06 M for simu-des (resource), and 51.5 M customers/s for the 64
parallel replications on 20 threads.

## Cimba

Cimba (C, stackful coroutines) reports about 45× over SimPy with Python multiprocessing on its
M/M/1 benchmark, on different hardware. The figures above are not a head-to-head comparison
with it. To compare on the same machine, build Cimba (meson, ninja, nasm and a C23 compiler; see
its INSTALL.md) and run its `MM1_single` and `MM1_multi` benchmarks next to these.

## Not yet compared

NeXosim models systems as components exchanging messages over ports rather than as processes,
so an equivalent M/M/1 needs more care to be fair; it is a later addition.
