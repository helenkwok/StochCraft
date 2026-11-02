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

## Results

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

## Cimba

Cimba (C, stackful coroutines) reports about 45× over SimPy with Python multiprocessing on its
M/M/1 benchmark, on different hardware. The figures above are not a head-to-head comparison
with it. To compare on the same machine, build Cimba (meson, ninja, nasm and a C23 compiler; see
its INSTALL.md) and run its `MM1_single` and `MM1_multi` benchmarks next to these.

## Not yet compared

NeXosim models systems as components exchanging messages over ports rather than as processes,
so an equivalent M/M/1 needs more care to be fair; it is a later addition.
