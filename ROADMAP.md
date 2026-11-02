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

1. **The model is data, in an existing open format.** A model is a document (blocks,
   parameters, connections, resources, experiments) that the engine compiles into a
   simulation. The UI, the CLI, MCP agents and tests all produce and consume the same document,
   and nothing about a model lives only in UI state. StochCraft does not invent a file format
   for it: models are BPMN 2.0 files with BPSim 2.0 simulation parameters (see
   [Model files](#model-files-no-new-format)), the way CADCraft keeps DXF as its native format.
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

## Model files: no new format

How other simulation tools keep models:

| Tool | Model is stored as |
|---|---|
| SimPy, simu-des, DESMO-J | Program code (Python, C, Rust, Java); no model file |
| JaamSim | Its own plain-text configuration file (`.cfg`) |
| AnyLogic, FlexSim, Arena, Simio | Their own proprietary files |
| Stella (system dynamics) | XMILE, an OASIS standard |

And the sibling Craft apps: CADCraft saves DXF natively and keeps CADCraft-only data inside it as
extension data; SolidCraft has its own `.sc` only because parametric history has no standard
form, and still reads and writes STEP, IGES and 3MF; FieldCraft has `.fieldcraft` but writes
Abaqus, Nastran and LS-DYNA input decks.

Open standards exist for exactly what StochCraft models:

| Standard | Covers | Use in StochCraft |
|---|---|---|
| **BPMN 2.0** (OMG; ISO/IEC 19510) with **BPSim 2.0** (WfMC, 2016, CC BY 3.0) | Process flow (start events, tasks, gateways, end events, sequence flows), diagram layout (BPMN DI), and BPSim simulation parameters on each element: arrival timers and counts, processing times as distributions, branch probabilities, resources and their quantities, calendars, and scenarios with replications, seed, duration and warm-up | **Native model file** (M1) |
| **XMILE 1.0** (OASIS, 2015) | System dynamics: stocks, flows, auxiliaries | Native file for the `sd` crate (later) |
| **CMSD** (SISO-STD-008-2010 and -01-2012) | Manufacturing data: resources, parts, routings, schedules | Import for factory models (later) |

So:

- **A model is a `.bpmn` file.** StochCraft reads and writes BPMN 2.0 XML with BPSim parameters.
  Any BPMN tool (bpmn.io, Camunda Modeler, Enterprise Architect and others) can open and draw
  it, and BPSim-aware tools can read the simulation parameters.
- **What BPMN and BPSim cannot say** (finite buffers that block, breakdowns, batching, conveyors)
  goes into BPMN's own extension mechanism (`extensionElements`) under a `stochcraft` XML
  namespace, which other tools ignore. That is the DXF-extension-data approach CADCraft uses.
- **The flowchart layout comes free.** BPMN DI already stores where each shape and arrow is drawn,
  so the canvas (M3.4) needs no layout format of its own.
- **Code stays first-class.** As in SimPy and Cimba, a model can also be written directly in
  Rust against the `blocks` API; that is how the tests and benchmarks are written.
- **Outputs are not model formats.** Results are exported as CSV (spreadsheets) and JSON (tools).

Mapping of today's blocks, to be checked element by element against the BPSim 2.0 specification
when M1.3 starts (the specification is at bpsim.org; this session could not reach it, so element
names below are the concepts, not yet verified spellings):

| StochCraft | BPMN element | BPSim parameters |
|---|---|---|
| Source | Start event | Inter-trigger timer (distribution), trigger count (limit) |
| Server (c units) | Task | Processing time (distribution); resource requirement with quantity |
| Delay | Task with no resource | Processing time |
| Sink | End event | (time in system is a BPSim result) |
| Split (M2) | Exclusive gateway | Probability on each outgoing sequence flow |
| Resource pool (M2) | Resource | Quantity (availability), calendar |
| Experiment | Scenario | Replications, seed, duration, warm-up, base time unit |

## Layer table after M1

M1 adds `model` and `io`, an `engine` above the experiment runner, and the CLI. The table
changes from what `xtask/src/layers.rs` holds today:

| Layer | Crates | Change |
|---|---|---|
| L0 | `random`, `stats` (standalone) | unchanged |
| L1 | `des`, **`model`** | `model` moves up from L0 so it can use `random::Dist` instead of duplicating it |
| L2 | `blocks`, `agents`, `sd` | `blocks` compiles a `model::Model` into a simulation |
| L3 | `experiment`, **`io`** | `io` reads and writes BPMN + BPSim files and exports results (bytes in, bytes out, so the browser uses the same code) |
| L4 | **`engine`** | new: commands, history, journal, runs |
| L5 | `render`, `ui-egui`, `mcp` | moved up from L4; `UI_MIN_LAYER` becomes 5 |
| apps | `stochcraft`, `stochcraft-cli`, `stochcraft-web` | |

---

## M1: model files, the engine and a command line

**Outcome:** `stochcraft-cli run line.bpmn --reps 100` runs a BPMN + BPSim model in parallel and
prints per-block results with confidence intervals; `stochcraft-cli new --template mm1` writes a
starting model that other BPMN tools can open; every model change is an engine command.

### M1.1 `stochcraft-model`: the model in memory

- In-memory types, not a file format: `Model { name, time_unit, blocks, connections, resources,
  experiment }`, `Block { id, name, kind }`, `BlockKind::{Source, Server, Delay, Sink}` with their
  parameters, `Connection { from, to }`, `Experiment { replications, seed, warmup, duration }`.
  `io` maps them to and from BPMN + BPSim.
- **Stable string ids**, which become the BPMN element ids (`id="machine-1"`), so files diff
  cleanly, connections survive deletes and agents can name blocks.
- `Model::validate()` returns *every* problem (not the first) as structured diagnostics with the
  block id and a message: unconnected outputs, connections into sources, duplicate names,
  invalid distribution parameters, unknown ids. The UI shows them in place; the CLI prints them.
- Every `random::Dist` maps to a BPSim distribution; a distribution with no BPSim counterpart is
  refused in validation rather than written in a private form.
- **Decision (time units):** the model declares one time unit (seconds, minutes or hours) and
  stores every time in it as `f64`; the UI displays and accepts other units and converts.
  *Recommended*: yes. It is how practitioners think, avoids a units system before one is
  needed, and matches BPSim's base time unit for a scenario.

### M1.2 `blocks` compiles a `Model`

- `blocks::compile(&model::Model) -> Result<Runnable, Diagnostics>`; the current builder API
  stays as a thin layer over the model types so existing tests keep working.
- Results become data too: `RunResult { blocks: BTreeMap<BlockId, BlockStats> }` with `serde`,
  plus `ExperimentResult` holding a `Summary` per block metric across replications.

### M1.3 `stochcraft-io`: BPMN + BPSim files

- Read and write BPMN 2.0 XML with BPSim 2.0 parameters (a permissively licensed XML crate such
  as `quick-xml`; bytes in, bytes out). Written from the published BPMN and BPSim
  specifications (clean room: no other tool's code or example files).
- **Round-trip without loss:** BPMN elements, attributes and extensions StochCraft does not
  understand are kept and written back unchanged, so opening and saving a file made in another
  BPMN tool never destroys its content. Elements StochCraft cannot simulate are reported as
  diagnostics, not dropped.
- StochCraft-only data goes in `extensionElements` under one versioned `stochcraft` XML
  namespace, the only place StochCraft defines anything of its own.
- Golden models under `tests/models/` (M/M/1, a tandem line, every block kind) written by hand
  from the specifications, plus BPMN files saved by other open-source BPMN editors (licence
  recorded in ATTRIBUTION.md), that must load, validate, run to a pinned result digest and
  round-trip.
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
  - `new --template <mm1|tandem|…> <file.bpmn>`
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

Blocks drawn as BPMN shapes, drag to place, drag between them to connect, pan and zoom,
multiple selection, copy and paste, align. Positions are BPMN DI, already part of the model file,
so a model drawn in another BPMN editor opens with its layout.

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
| D1 | Model file format | No new format: BPMN 2.0 + BPSim 2.0 (`.bpmn`), StochCraft-only data in `extensionElements`; XMILE for system dynamics and CMSD import later |
| D2 | Time units | One unit per model, converted for display |
| D3 | Entity attributes | Named numbers only at first |
| D4 | First UI milestone | Forms and results before the canvas (M3.2–M3.3 before M3.4) |
| D5 | Browser build | Part of M3 (M3.6), wasm32 first, wasm64 when its toolchain is verified |
| D6 | Conditions and logic | Fixed rule types in M2; an expression language later |
