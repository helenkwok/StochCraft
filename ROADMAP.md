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
| **BPSim 2.0** (WfMC, December 2016, CC BY 3.0) on **BPMN 2.0** (OMG; ISO/IEC 19510) or **XPDL 2.2** (WfMC) | The process model (start events, tasks, gateways, end events, sequence flows, resources, diagram layout) is BPMN or XPDL; BPSim adds scenarios of simulation parameters that reference its elements by id | **Native model file** (M1) |
| **XMILE 1.0** (OASIS, 2015) | System dynamics: stocks, flows, auxiliaries | Native file for the `sd` crate (later) |
| **CMSD** (SISO-STD-008-2010 and -01-2012) | Manufacturing data: resources, parts, routings, schedules | Import for factory models (later) |

### What BPSim 2.0 defines (checked against the specification)

From the BPSim 2.0 specification (WFMC-BPSWG-2016-1) and Implementer's Guide, both CC BY 3.0:

- **Where the data lives.** One `bpsim:BPSimData` root (namespace
  `http://www.bpsim.org/schemas/2.0`) inside a BPMN file (in `semantic:relationship
  type="BPSimData"` → `extensionElements`), inside an XPDL `Package`, or as a standalone XML file.
- **Scenarios.** `Scenario` (`id`, `name`, `author`, `vendor`, `version`, `inherits` to override
  another scenario, `result` to mark an output scenario) with `ScenarioParameters`: `start`,
  `duration`, `warmup`, `replication`, `seed`, `baseTimeUnit` (ms, s, min (default), hour, day,
  year), `baseCurrencyUnit`, `baseResultFrequency`, `traceOutput`/`traceFormat` (XES). The
  specification requires that the same model, seed and replication count give the same
  results, which is principle 3.
- **Element parameters** reference a BPMN or XPDL element by `elementRef`, in six groups:
  - *Time:* `transferTime`, `queueTime`, `waitTime`, `setupTime`, `processingTime`,
    `validationTime`, `reworkTime` (and `lagTime`, `duration`, `elapsedTime` as results only).
  - *Control:* `interTriggerTimer`, `triggerCount`, `probability`, `condition`.
  - *Resource:* `availability`, `quantity`, `selection`, `role`.
  - *Cost:* `fixedCost`, `unitCost`.
  - *Property:* named properties carried by each token (`long`, `double`, `boolean`, `string`,
    `duration`, `dateTime`), and `queueLength` as a result.
  - *Priority:* `interruptible`, `priority`.
- **Parameter values:** constants (`Numeric`, `Floating`, `Boolean`, `String`, `Duration` in ISO
  8601, `DateTime`), **distributions** (Beta, Binomial, Erlang, Gamma, LogNormal,
  NegativeExponential, Normal, Poisson, Triangular, TruncatedNormal, Uniform, Weibull, and
  `UserDistribution` from data points, discrete or continuous), `EnumParameter` (a sequence of
  historical values), and `ExpressionParameter` (XPath 1.0 with `bpsim:getProperty`,
  `getResource(name, qty)`, `getResourceByRoles(...)` and `orResource(...)`).
- **Calendars:** iCalendar (RFC 5545) `VEVENT`s at scenario level; any parameter value can be
  `validFor` a calendar (shifts, rates by time of day).
- **Results:** `ResultRequest` of `min`, `max`, `mean`, `count` or `sum` on any parameter, written
  back as a result scenario.
- **Extensions:** `VendorExtension` (name and value) on scenarios and element parameters. Tools
  must carry every vendor extension and never drop unknown ones.

The reference models that accompany the Implementer's Guide (car repair, home loan, technical
support) are XPDL 2.2 files; three use BPSim 2.0 and one BPSim 1.0
(`http://www.bpsim.org/schemas/1.0`). Between them they use inter-trigger timers, trigger
counts, branch probabilities and conditions, processing times (triangular and truncated
normal), token properties with expressions, resources chosen with `getResource`, queue length
results, daily shift calendars, a start date, duration and warm-up.

### Decisions that follow

- **A model is a BPMN 2.0 file with embedded BPSim 2.0** (`.bpmn`) when StochCraft creates it.
  StochCraft also **reads XPDL 2.2** with BPSim, since the reference models and several tools use
  it, and saves a file back in the format it came in. BPSim 1.0 files are read and upgraded.
- **StochCraft-only data** (finite buffers that block, breakdowns, batching, conveyors) is a BPSim
  `VendorExtension` named with a `stochcraft:` prefix, which every conformant BPSim tool must
  keep. Nothing else is StochCraft's own.
- **Expressions are BPSim's.** Conditions and property updates use BPSim's XPath 1.0 expressions,
  so StochCraft implements that language (the subset the specification and reference models
  use first) instead of inventing one. This replaces decision D6.
- **Results are a BPSim result scenario**, with CSV export for spreadsheets.
- **The flowchart layout comes free** from BPMN DI or XPDL graphics information.
- **Code stays first-class.** As in SimPy, a model can also be written directly in Rust against
  the `blocks` API; that is how the tests and benchmarks are written.

### Mapping to StochCraft

| StochCraft | BPMN / XPDL element | BPSim parameters |
|---|---|---|
| Source | Start event | `interTriggerTimer`, `triggerCount`; initial token properties |
| Server (c units) | Task with a resource | `processingTime` (and the other time parameters); `selection` = `getResource(id, c)` |
| Delay | Task with no resource | `processingTime` |
| Sink | End event | `triggerCount` result; time in system from token creation |
| Split (M2) | Exclusive gateway | `probability` or `condition` on each outgoing sequence flow |
| Parallel split and join (M2) | Parallel gateway | none (BPMN semantics) |
| Resource pool (M2) | BPMN `resource` / XPDL `Participant` of type RESOURCE | `quantity`, `availability` with calendars, `role` |
| Entity attributes (M2) | Token | `PropertyParameters`, `ExpressionParameter` |
| Priorities, preemption (M2) | Task | `priority`, `interruptible` |
| Costs (later) | Task, resource | `fixedCost`, `unitCost` |
| Experiment | Scenario | `replication`, `seed`, `start`, `duration`, `warmup`, `baseTimeUnit` |
| Results | Result scenario | `ResultRequest` min, max, mean, count, sum |

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
- `random::Dist` grows to BPSim's full set of distributions (adding Beta, Binomial, Erlang,
  Gamma, LogNormal, Normal, Poisson, TruncatedNormal, Weibull and `UserDistribution`; it has
  exponential, triangular and uniform today), each with a mean test, so every BPSim model
  StochCraft reads can be sampled.
- **Time units follow BPSim:** each scenario has a `baseTimeUnit` (minutes by default) and any
  value may override it with its own `timeUnit` or an ISO 8601 duration (`PT24M`). The model
  stores times as `f64` in the scenario's base unit; this was decision D2 and is now settled
  by the standard.

### M1.2 `blocks` compiles a `Model`

- `blocks::compile(&model::Model) -> Result<Runnable, Diagnostics>`; the current builder API
  stays as a thin layer over the model types so existing tests keep working.
- Results become data too: `RunResult { blocks: BTreeMap<BlockId, BlockStats> }` with `serde`,
  plus `ExperimentResult` holding a `Summary` per block metric across replications.

### M1.3 `stochcraft-io`: BPSim on BPMN and XPDL

- Read BPSim 2.0 (and 1.0, upgraded on read) embedded in BPMN 2.0 or XPDL 2.2, or standalone;
  write BPMN 2.0 with embedded BPSim 2.0 for new models and the original format for opened ones.
  A permissively licensed XML crate (such as `quick-xml`); bytes in, bytes out. Written from the
  BPMN, XPDL and BPSim specifications.
- **Round-trip without loss:** elements, attributes, scenarios and vendor extensions StochCraft
  does not understand are kept and written back unchanged, as BPSim requires for vendor
  extensions. Elements StochCraft cannot simulate yet are reported as diagnostics, not dropped.
- StochCraft-only data is a BPSim `VendorExtension` named `stochcraft:…`.
- Scenarios: a file's scenarios are listed, one is chosen to run, and `inherits` is resolved.
- M1 supports what today's blocks need (start events with `interTriggerTimer` and
  `triggerCount`, tasks with `processingTime` and one resource selection, end events, exclusive
  gateways with `probability`, scenario parameters) and reports the rest; M2 widens it.
- **Test corpus:** hand-written models (M/M/1, a tandem line, each element kind) plus the
  Implementer's Guide reference models (car repair, home loan, technical support). Each must
  load, validate, round-trip byte-for-byte where untouched, and run to a pinned result digest.
  The reference files are the examples published by WfMC with the Implementer's Guide 2.0 on
  bpsim.org, under the guide's CC BY 3.0 licence; they are committed under
  `tests/models/bpsim-reference/` with an ATTRIBUTION.md row crediting WfMC. The bundle has each
  model (car repair, home loan, technical support) as both BPMN and XPDL, in BPSim 1.0 and 2.0,
  so both host formats and the 1.0 upgrade are tested from the start. Its Visio drawings and
  images are not needed.
- **Cross-check against another simulator:** the bundle includes result scenarios
  (`*_results.bpsim`) produced by Lanner's L-SIM for the three BPSim 2.0 models, with
  per-replication values (for example 151 car repair arrivals in each of three replications,
  and 107, 96 and 100 customers leaving). StochCraft's results for the same scenarios must agree
  with them statistically: deterministic quantities exactly, random ones within confidence
  intervals, since the random number generators differ. They are also the reference for how
  StochCraft writes its own result scenarios (`result`, `instance` per replication,
  `resultTimeStamp`).
- **Schema validation:** the BPSim 2.0 XML Schema (XSD, from bpsim.org) is used in tests to check
  that every file StochCraft writes is valid BPSim.
- Results: written as a BPSim result scenario (min, max, mean, count, sum per requested
  parameter), and exported as CSV for spreadsheets.
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

### M2.2 Token properties and expressions

- Entities carry BPSim **properties**: named values of BPSim's property types (`long`, `double`,
  `boolean`, `string`, `duration`, `dateTime`), set by `PropertyParameters` on start events and
  updated as a token enters each element (decision D3 is settled by the standard: all BPSim
  types, stored compactly).
- **Expressions:** an evaluator for BPSim's XPath 1.0 expressions: arithmetic, comparisons and
  boolean logic, `getProperty`, and the resource functions (M2.5). It covers what the
  specification and the reference models use, and reports anything outside that subset as a
  diagnostic. Validation: the car repair model's repair loop (property decremented per repair,
  exit when it reaches zero) against its expected count.

### M2.3 Routing

| Block | Behaviour | Validation |
|---|---|---|
| Exclusive gateway, by `probability` | Output chosen by weights (BPSim default: the rest share evenly) | Jackson network: throughput and W per node against product-form results |
| Exclusive gateway, by `condition` | First outgoing flow whose expression is true | Deterministic cases; the car repair loop |
| Parallel gateway (split and join) | Copies a token to every branch; joins wait for all | Deterministic fork-join cases; the home loan model |
| Merge (several flows into one element) | Tokens from several inputs | Superposition of Poisson streams |

### M2.4 Buffers and blocking

- Finite buffers between tasks, as a `stochcraft:` vendor extension (BPSim has no capacity on
  sequence flows); a full buffer blocks the upstream task (blocking after service).
- Validation: M/M/1/K loss probability and L; a two-machine line with a finite buffer against the
  published closed-form throughput.

### M2.5 Shared resources

- Resource pools from BPMN `resource` or XPDL `Participant` (type RESOURCE) elements with BPSim
  `quantity` and `role`; tasks acquire them by `selection` (`getResource(id, qty)`,
  `getResourceByRoles`, `orResource` for alternatives), with `priority` deciding who goes first.
  Spans across several tasks follow BPMN's own constructs (a resource held across a
  sub-process).
- Validation: Erlang loss (M/M/c/c) for a pool with no queue; a machine-interference
  (repairman) model against its closed form.

### M2.6 Time-dependent behaviour

- **Calendars:** BPSim calendars are iCalendar (RFC 5545) `VEVENT`s; a parameter value can be
  `validFor` one. StochCraft implements the iCalendar subset needed for shifts and opening
  hours (`DTSTART`, `DTEND`, `RRULE` with daily and weekly frequency, `BYDAY`, `UNTIL`,
  `COUNT`), reports the rest, and uses it for resource `availability` and `quantity` and for
  `interTriggerTimer` (arrival rate by time of day, which changes immediately as BPSim
  specifies). Validation: the technical support model's shifts; arrivals per interval against
  the integrated rate.
- Breakdowns on servers (a `stochcraft:` vendor extension, since BPSim has none): time between
  failures and repair time distributions, preempting or waiting for the current job (BPSim
  `interruptible`). Validation: availability MTBF / (MTBF + MTTR) and the effective
  service rate.

### M2.7 Assembly and batching

- Batching and assembly: BPMN's own constructs where they fit (a parallel join for assembly,
  multi-instance activities), otherwise `stochcraft:` vendor extensions for batch size and bill
  of materials. Validation: exact deterministic flows; throughput bounded by the
  slowest input.

**Later in M2 or M4:** conveyors with length, speed and accumulation; travel networks for
forklifts and AGVs; agent behaviours (`agents` crate); stock-and-flow (`sd` crate, XMILE);
costs (BPSim `fixedCost`, `unitCost`); simulation traces in XES (BPSim `traceOutput`).

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
              M2.2 properties, expressions               M3.2 editing
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
| D1 | Model file format | No new format: BPSim 2.0 embedded in BPMN 2.0 (`.bpmn`, for new models) or XPDL 2.2 (read and kept); StochCraft-only data as BPSim vendor extensions; XMILE and CMSD later |
| D2 | Time units | Settled by BPSim: scenario `baseTimeUnit`, per-value overrides |
| D3 | Entity attributes | Settled by BPSim: token properties of BPSim's types |
| D4 | First UI milestone | Forms and results before the canvas (M3.2–M3.3 before M3.4) |
| D5 | Browser build | Part of M3 (M3.6), wasm32 first, wasm64 when its toolchain is verified |
| D6 | Conditions and logic | Settled by BPSim: its XPath 1.0 expressions, the subset models use first |
| D7 | Reference models as test files | Settled: WfMC's examples from the Implementer's Guide 2.0 (bpsim.org, CC BY 3.0), BPMN and XPDL, BPSim 1.0 and 2.0, plus L-SIM results as a cross-check; committed with attribution |
