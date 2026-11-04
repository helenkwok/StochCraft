# Architecture: crate layers

Every workspace crate sits on a layer and may depend only on crates in strictly lower layers
(`cargo xtask layers` enforces this; the table lives in `xtask/src/layers.rs`). Keeping the
graph shallow is what keeps CI fast: `cargo xtask affected` retests a changed crate and the
crates above it, so a change near the top touches little and a change near the bottom is rare.

| Layer | Crates (`stochcraft-*`) | Contents |
|---|---|---|
| L0 | `random`, `stats` (standalone: no workspace dependencies) | Random-number generators and BPSim's distributions, statistics collectors |
| L1 | `des`, `model` | The discrete-event core (event queue, processes, resources, timeouts); the model in memory (elements with stable ids, connections, experiment, validation) |
| L2 | `blocks`, `agents`, `sd` | Modelling libraries: flow blocks (source, queue, delay, conveyor…), moving agents, stock-and-flow |
| L3 | `experiment`, `io` | Replications, parameter sweeps and optimisation; model and result files |
| L4 | `engine` | Commands, undo and redo, the journal and runs: the one way the CLI, the app and agents change or run a model |
| L5 | `render`, then `ui-egui` and `mcp` | Drawing, the window, and agents over MCP. The only layer allowed UI, windowing or GPU crates |
| — | `testkit` | Test tooling; other crates may use it only as a dev-dependency |
| apps | `stochcraft`, `stochcraft-cli`, `stochcraft-web` | Exempt from the rules |

Only `random`, `stats`, `des`, `model`, `blocks`, `experiment`, `io`, `engine` and `testkit` exist so far (with the `stochcraft-cli` app); the others are placed in advance so each crate's position is decided
before it is written.

## Determinism across targets

Every layered crate builds for `wasm32-unknown-unknown` (`cargo xtask wasm`), and a later wasm64
build will follow. A seeded model must give the same results on native, wasm32 and wasm64, and
on every operating system:

- Anything saved, sent, hashed or used to seed random numbers uses fixed-width integers, never
  `usize`.
- Logarithms, exponentials, powers and trigonometric functions come from
  `stochcraft_random::math` (the pure-Rust `libm`), never from `f64`'s methods, which call each
  platform's maths library and can differ in the last bit between glibc, musl (wasm), macOS and
  Windows. `clippy.toml` forbids the `f64` methods. Square roots are exactly rounded everywhere.
- Replications run in parallel but their results are combined in replication order.

`stochcraft-testkit` checks it: the golden models (`tests/models/`) are run through the engine
and their results reduced to digests pinned in `tests/golden/results.txt`. The test runs natively
in `cargo xtask ci` and as `wasm32-wasip1` under Node's built-in WASI in `cargo xtask golden`
(both in CI). Switching to `libm` changed the last bits of some samples, which the golden test
caught; that is the kind of drift it exists to catch.
