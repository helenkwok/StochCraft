# StochCraft roadmap

Where StochCraft is going next and in what order. Three milestones: **M1** model files, the
engine and a command line; **M2** the flow logic a factory or logistics model needs; **M3** the
desktop app (and the browser app built from it). Each milestone is a series of pull requests
small enough to review in one sitting; every one keeps `cargo xtask ci` green.

Decisions still open are marked **Decision** with a recommendation; the rest follows the
patterns of the sibling Craft apps (FieldCraft, CADCraft, SolidCraft).

## Where we are

| Crate | Layer | State |
|---|---|---|
| `random` | L0 | xoshiro256** streams; exponential, uniform, triangular; `Dist` |
| `stats` | L0 | `Tally`, `TimeWeighted`, Student-t 95% intervals, `Summary` |
| `des` | L1 | Event queue; async processes with `hold`, `Resource`, `Store`, `spawn` |
| `blocks` | L2 | Source, Server, Delay, Sink; per-block statistics; warm-up |
| `experiment` | L3 | Parallel replications with reproducible streams |
| `bench/` | outside | M/M/1 against simu-des and SimPy (54× SimPy) |

Models today are Rust code. Nothing can be saved, opened, run from a command line, driven by an
agent or seen on screen. M1 fixes the first four; M3 the last.

## Principles that hold across all three

1. **The model is data.** A model is a document (blocks, parameters, connections, resources,
   experiments) that the engine compiles into a simulation. The UI, the CLI, MCP agents and
   tests all produce and consume the same document. Nothing about a model lives only in UI state.
2. **One command layer.** Every change to a model and every run is an engine command
   (`engine.execute("block.add", params)`), as in FieldCraft and CADCraft. The desktop app, the
   CLI, scripts and the MCP server call the same commands, so undo, the journal and agent
   automation come for free and behave identically.
3. **Deterministic.** The same model, seed and replication count give bit-identical results on
   native, wasm32 and (later) wasm64, with any thread count. A golden-results test enforces it.
4. **Validated against theory.** Every block that has an analytic counterpart ships with a test
   whose 95% confidence interval must cover it (M/M/1, M/M/c, M/M/1/K, Erlang loss, Jackson
   networks). Blocks without one get exact deterministic tests.
5. **Layers stay shallow.** New crates get a row in `xtask/src/layers.rs` before code is written;
   the UI never reaches below the engine.
6. **Clean room.** Behaviour comes from textbooks, papers and vendors' public documentation read
   as prose. No AnyLogic or FlexSim names for features beyond generic terms, no copied UI layouts,
   icons, example models or help text (CONTRIBUTING.md).

## Layer table after M1

M1 adds `model` and `io`, an `engine` above the experiment runner, and the CLI. The table
changes from what `xtask/src/layers.rs` holds today:

| Layer | Crates | Change |
|---|---|---|
| L0 | `random`, `stats` (standalone) | unchanged |
| L1 | `des`, **`model`** | `model` moves up from L0 so it can use `random::Dist` (with a `serde` feature) instead of duplicating it |
| L2 | `blocks`, `agents`, `sd` | `blocks` compiles a `model::Model` into a simulation |
| L3 | `experiment`, **`io`** | `io` reads and writes model and result files (bytes in, bytes out, so the browser uses the same code) |
| L4 | **`engine`** | new: commands, history, journal, runs |
| L5 | `render`, `ui-egui`, `mcp` | moved up from L4; `UI_MIN_LAYER` becomes 5 |
| apps | `stochcraft`, `stochcraft-cli`, `stochcraft-web` | |

---

## M1: model files, the engine and a command line

**Outcome:** `stochcraft-cli run line.stochcraft --reps 100` runs a model file in parallel and
prints per-block results with confidence intervals; `stochcraft-cli new --template mm1` writes a
starting model; every model change is an engine command.

### M1.1 `stochcraft-model`: the model as data

- Types with `serde`: `Model { format, name, time_unit, blocks, connections, resources,
  experiment }`, `Block { id, name, kind }`, `BlockKind::{Source, Server, Delay, Sink}` with their
  parameters, `Connection { from: PortRef, to: PortRef }`, `Experiment { replications, seed,
  warmup, duration }`.
- **Stable string ids** (`"machine-1"`), not indices, so files diff cleanly, connections survive
  deletes and agents can name blocks.
- `Model::validate()` returns *every* problem (not the first) as structured diagnostics with the
  block id and a message: unconnected outputs, connections into sources, duplicate names,
  invalid distribution parameters, unknown ids. The UI shows them in place; the CLI prints them.
- `random::Dist` gains a `serde` feature with an explicit tagged form:
  `{"exponential": {"mean": 1.25}}`, `{"triangular": {"min": 1, "mode": 2, "max": 4}}`.
- **Decision (time units):** the model declares one time unit (seconds, minutes or hours) and
  stores every time in it as `f64`; the UI displays and accepts other units and converts.
  *Recommended*: yes. It is how practitioners think and avoids a units system before one is
  needed.

### M1.2 `blocks` compiles a `Model`

- `blocks::compile(&model::Model) -> Result<Runnable, Diagnostics>`; the current builder API
  stays as a thin layer over the model types so existing tests keep working.
- Results become data too: `RunResult { blocks: BTreeMap<BlockId, BlockStats> }` with `serde`,
  plus `ExperimentResult` holding a `Summary` per block metric across replications.

### M1.3 `stochcraft-io`: files

- **Decision (file format):** a single pretty-printed JSON file with sorted keys,
  `"format": "stochcraft/1"` and a `generator` field, extension `.stochcraft`.
  *Recommended*: start here, the same idea as FieldCraft's `model.json`. Move to a ZIP package
  (model + results + layout images, as `.fieldcraft` does) only when there is something
  binary to store. An unknown newer `format` is refused with a clear message; older ones are
  migrated, each migration a tested function.
- Golden model files under `tests/models/` (M/M/1, a tandem line, every block kind) that must
  load, validate, run to a pinned result digest and save byte-identically.
- Results export: JSON (complete) and CSV (one row per block metric, for spreadsheets).
- Saving through `craft-atomic` on native so a crash never truncates a model.

### M1.4 `stochcraft-engine`: commands

- `Engine { model, history, journal }` and `execute(id, params) -> Result<Value>` with a
  catalogue of commands: `model.new`, `model.rename`, `block.add`, `block.remove`,
  `block.set` (parameters), `block.rename`, `connect`, `disconnect`, `experiment.set`, `run`,
  `run.cancel`, `file.open`, `file.save`.
- Each command has a JSON schema (generated from the parameter types) that the CLI, MCP tool
  list and documentation are built from.
- Undo and redo by inverse commands; a journal of every command, replayable as a script.
- `run` executes on worker threads natively with progress and cancellation; in the browser in
  slices between frames. Results come back as `ExperimentResult`.

### M1.5 `apps/stochcraft-cli`

- `clap` subcommands:
  - `new --template <mm1|tandem|…> <file>`
  - `validate <file>`: diagnostics; exit code 1 if any
  - `run <file> [--reps N] [--seed S] [--threads T] [--warmup W] [--duration D] [--format table|json|csv] [--out file]`
  - `exec <file> <script.json>`: replay engine commands
  - `schema [command]`: print command schemas
- The table output reads like a textbook result: block, metric, mean, ±half-width, n.
- Integration tests run the binary on the golden models.

### M1.6 Determinism and the browser

- A `stochcraft-testkit` golden-results test: each golden model's results digest is pinned and
  must match on native and on wasm32 (run under a wasm runtime in CI; `wasm-bindgen-test` or
  `wasmtime`). wasm64 joins when its toolchain is checked (README of the wasm64 plan).

**M1 size:** about six PRs (M1.1 … M1.6). M1.1–M1.3 can land without the engine; M1.4–M1.5
depend on them.

---

## M2: flow logic for factories and logistics

**Outcome:** a model of a small production line or warehouse can be built from blocks: typed
parts with attributes, routing, finite buffers that block upstream machines, shared operators
and forklifts, shifts, breakdowns and assembly, each validated against theory or exact cases.

Every new block lands as one PR containing: the `des` primitive it needs (if any), the
`model` schema and validation, the `blocks` implementation, the engine commands, the theory or
deterministic tests, and a golden model file. The UI forms (M3) follow automatically from the
schema where possible.

### M2.1 Engine primitives in `des`

Needed by the blocks below, each with its own unit tests:

| Primitive | For |
|---|---|
| Bounded `Store` (`put().await` waits when full) | finite buffers, blocking lines |
| Priority `Resource` and `Store` (priority, then FIFO) | priority rules, urgent orders |
| Interrupt / preempt a process | breakdowns, preemptive priority |
| Timeouts on waits (`acquire` or `get` with a deadline) | reneging, balking |
| Condition / event (wait until signalled) | gates, schedules, assembly |

### M2.2 Entities with types and attributes

- `Entity { id, kind, created, entered, attributes }`, where `kind` is an index into the model's
  entity types (`"part-A"`, `"part-B"`) and `attributes` is a small vector of numbers declared
  per type.
- **Decision (attributes):** numbers only (with names) at first, text later.
  *Recommended*: numbers only; covers due dates, priorities, weights and process times without a
  general value system.
- Sources produce a mix of types (weights) and set initial attributes from distributions.

### M2.3 Routing

| Block | Behaviour | Validation |
|---|---|---|
| `Split` (probabilistic) | Output chosen by weights | Jackson network: throughput and W per node against product-form results |
| `Route` (by rule) | By entity type, attribute threshold, shortest queue or round robin | Deterministic cases; shortest-queue against a symmetric two-server simulation |
| `Merge` | Several inputs into one | Superposition of Poisson streams |

### M2.4 Buffers and blocking

- `Buffer` with finite capacity between machines; a full buffer blocks the upstream server
  (blocking after service).
- Validation: M/M/1/K loss probability and L; a two-machine line with a finite buffer against the
  published closed-form throughput.

### M2.5 Shared resources

- `ResourcePool` (operators, forklifts, fixtures) with capacity; a Server or Delay can require
  units from one or more pools for its duration; `Seize` and `Release` blocks for spans across
  several steps.
- Validation: Erlang loss (M/M/c/c) for a pool with no queue; a machine-interference
  (repairman) model against its closed form.

### M2.6 Time-dependent behaviour

- Arrival schedules: piecewise-constant rate by time of day, sampled as a non-homogeneous
  Poisson process by thinning (Lewis and Shedler). Validation: count per interval against the
  integrated rate.
- Shift calendars that change a server's or pool's capacity over time.
- Breakdowns on servers: time between failures and repair time distributions, preempting or
  waiting for the current job. Validation: availability MTBF / (MTBF + MTTR) and the effective
  service rate.

### M2.7 Assembly and batching

- `Batch` (N entities into one), `Unbatch`, and `Assemble` (a bill of materials from several
  inputs into one product). Validation: exact deterministic flows; throughput bounded by the
  slowest input.

**Later in M2 or M4:** conveyors with length, speed and accumulation; travel networks for
forklifts and AGVs; agent behaviours (`agents` crate); stock-and-flow (`sd` crate); an
expression language for conditions (**Decision**, recommended later, not in M2: fixed rule types
cover most models and keep files analysable).

**M2 size:** about eight PRs, each independently mergeable after M1.

---

## M3: the desktop app

**Outcome:** open or create a model, edit it, run experiments and read results in a native
window on macOS, Windows and Linux, and in the browser, using the same craft-libs foundation and
look as the other Craft apps.

### Foundation

- `eframe` and `egui` 0.37 on the storytold forks, pinned to the same revisions as CADCraft;
  `craft-ui` (controls, icons, theme), `craft-window` (frame decisions), `craft-settings`,
  `craft-dialogs` (non-blocking open and save), `craft-menu` and `craft-keymap`, `craft-i18n`,
  `craft-atomic`, `craft-update` later. Git dependencies pinned by commit, all bumped together.
- `crates/ui-egui` (L5) holds the interface; `apps/stochcraft` is the native shell;
  `apps/stochcraft-web` the wasm32 shell.
- All UI changes go through engine commands (principle 2), so undo, redo and the journal work
  from the first window.

### M3.1 Shell and documents

Window, menus, keyboard shortcuts, open, save and recent files, settings, unsaved-change prompts,
autosave, the About box with licences. Snapshot tests with `egui_kittest`.

### M3.2 Model editing without a canvas

A block list with a property inspector (forms generated from the command schemas, with units and
inline validation diagnostics), a connections table, entity types and resource pools. Usable end
to end before any drawing exists, and accessible from the keyboard.

### M3.3 Running and results

Experiment settings (replications, seed, warm-up, duration), a run button with progress and
cancel, a results table with confidence intervals, and charts (time in system, utilisation per
block, queue length over time for a single replication) with export to CSV and PNG.

### M3.4 The flowchart canvas

Blocks drawn as nodes with ports, drag to place, drag between ports to connect, pan and zoom,
multiple selection, copy and paste, align. A layout is stored in the model file (`layout`
section) so the canvas is part of the document.

### M3.5 Live view of a run

Step a single replication at a chosen speed (the engine already supports `run_until`
in slices) and show counts, queue lengths and utilisation on the canvas as it runs; pause, step
and jump to a time.

### M3.6 Browser and agents

- `apps/stochcraft-web`: the same interface built for wasm32, runs single-threaded (web workers
  for replications later), files through the browser's open and download.
- `crates/mcp` and `stochcraft-cli mcp`: the engine's commands as MCP tools, headless or bridged
  to a running app, following FieldCraft's MCP design.

**Later:** 2D layout with entities moving between stations; 3D (the `render` crate, wgpu);
FlexSim-style factory layouts via the shared `craft-factory` data from CADCraft.

**M3 size:** about six PRs; M3.1–M3.3 make a usable app, M3.4–M3.5 make it a modelling tool.

---

## Order of work

```
M1.1 model ─ M1.2 compile ─ M1.3 io ─ M1.4 engine ─ M1.5 cli ─ M1.6 determinism
                                          │
                     ┌────────────────────┴────────────────────┐
              M2.1 primitives                            M3.1 shell
              M2.2 entities                              M3.2 editing
              M2.3 … M2.7 blocks (each one PR)           M3.3 results
                     │                                   M3.4 canvas
                     └──────────── new blocks appear ────M3.5 live view
                                   in M3 forms           M3.6 web + MCP
```

M1 comes first because both M2 and M3 build on the model document and the engine. After M1, M2
and M3 proceed in parallel: each M2 block becomes editable in M3 as soon as its schema lands.

## Decisions to confirm

| # | Question | Recommendation |
|---|---|---|
| D1 | Model file format | Single JSON file, `format: "stochcraft/1"`, ZIP package later |
| D2 | Time units | One unit per model, converted for display |
| D3 | Entity attributes | Named numbers only at first |
| D4 | First UI milestone | Forms and results before the canvas (M3.2–M3.3 before M3.4) |
| D5 | Browser build | Part of M3 (M3.6), wasm32 first, wasm64 when its toolchain is verified |
| D6 | Conditions and logic | Fixed rule types in M2; an expression language later |
