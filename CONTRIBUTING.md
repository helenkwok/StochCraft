# Contributing to StochCraft

## Licence of contributions

StochCraft is licensed MIT OR Apache-2.0. By submitting a contribution you agree that it is
licensed under both, without additional terms (see the Licence section of the README). Only
submit work you wrote yourself or have the right to license this way.

## Clean-room rules

1. **Behaviour comes from public sources only:** published papers and textbooks on simulation,
   vendors' public documentation read as prose, and running software you are licensed to use and
   observing what it does. Never decompile, disassemble or extract files from AnyLogic, FlexSim,
   Autodesk products or any other proprietary software, and never paste their code, help text,
   example models or images into this repository.
2. **Open-source code:** reading and adapting code from permissively licensed projects (MIT,
   Apache-2.0, BSD, Zlib and similar) is fine. Add a row to [ATTRIBUTION.md](ATTRIBUTION.md) and
   keep the original copyright notice in the adapted file. Never read or copy GPL, LGPL, AGPL or
   similarly licensed code, including desim and other GPL simulation libraries.
3. **Dependencies** must be licensed compatibly with MIT OR Apache-2.0. `cargo deny check licenses`
   (in CI) enforces the allow list in `deny.toml`; say in the PR why a new dependency is needed.
4. **Assets** (icons, images, models, fonts, example models) must be original or openly licensed,
   each with a row in ATTRIBUTION.md.

## Code rules

- No `unsafe` (the workspace forbids it).
- No `unwrap`, `expect` or `panic!` outside tests (Clippy denies them).
- Results must be identical on native, wasm32 and wasm64: use fixed-width integers (`u32`, `u64`),
  not `usize`, in anything saved, sent, hashed or used to seed random numbers.

## Crate layers

Crates sit on layers and may depend only on lower layers, so most changes retest only a few
crates. See [docs/architecture.md](docs/architecture.md); `cargo xtask layers` enforces it, and a
new crate must be added to the table in `xtask/src/layers.rs`.

## Before pushing

```bash
cargo xtask ci
```

That runs fmt, the layer check, Clippy with `-D warnings` and the tests. `cargo xtask ci
--affected origin/main` does the same for only the crates your change affects, which is what CI
runs on a pull request. `cargo xtask wasm` checks that the layered crates build for the browser.

`cargo xtask fetch-bpsim` downloads the BPSim reference models and XML Schema from bpsim.org into
`target/bpsim/` (they are never committed). Without them, and without `xmllint`, the
`stochcraft-io` reference tests skip locally with a message; CI always runs them.
