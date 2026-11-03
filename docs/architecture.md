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
| L4 | `render`, then `ui-egui` and `mcp` | Drawing, the window, and agents over MCP. The only layer allowed UI, windowing or GPU crates |
| — | `testkit` | Test tooling; other crates may use it only as a dev-dependency |
| apps | `stochcraft`, `stochcraft-cli`, `stochcraft-web` | Exempt from the rules |

Only `random`, `stats`, `des`, `model`, `blocks` and `experiment` exist so far; the others are placed in advance so each crate's position is decided
before it is written.

## Determinism across targets

Every layered crate builds for `wasm32-unknown-unknown` (`cargo xtask wasm`), and a later wasm64
build will follow. A seeded model must give the same results on native, wasm32 and wasm64, so
anything saved, sent, hashed or used to seed random numbers uses fixed-width integers, never
`usize`.
