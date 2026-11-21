# StochCraft roadmap

Where StochCraft is going next and in what order. Seven milestones: **M1** model files, the
engine and a command line; **M2** the flow logic a factory or logistics model needs; **M3** the
desktop app (and the browser app built from it); **M4** Monte Carlo, system dynamics, agents and
hybrid models; **M5** transport and layout, led by a reference model, the sorter; **M6** Python
and plugins; **M7** industrial engineering (line balancing, material flow on the layout,
inventory, sharing a model). M1 and M3.1–M3.6 are done; M3.7, a redesign that makes results
answer the question, has steps 1–5 done, and M5's sorter is built alongside it (D21). M2.1–M2.8
are done, and so is M2.10 (abandonment and balking) apart from the gaps listed in it. M2.11
(changeovers and lean measures) has its changeovers, product-type times, takt and cycle times,
state times and most of OEE done; M2.9 (output analysis) has its recorder of results over time,
MSER-5's warm-up suggestion, its warnings, streams by element id and starting contents done. M4
is being planned, and M4.1 (Monte Carlo) has its engine, files and command line (part 1), its
window (part 2) and which inputs matter (part 3a: tornado and rank correlations) done.
M5 has its conveyors (M5.1), the operators in the model and files (M5.2, PR 1), a first 3D view
(M5.3), the dashboard (M5.4) and the sorter example in; M5.5, which reworks M5's geometry so the
physical layout is a model layer of its own, from which the plan and the 3D view are drawn, has
begun (L1, U1). M7.1's line balancing core, engine command and CLI are done. Each milestone is a
series of pull requests small enough to review in one sitting; every one keeps `cargo xtask ci`
green.

Decisions still open are marked **Decision** with a recommendation; the rest follows the
patterns of the sibling Craft apps (FieldCraft, CADCraft, SolidCraft). The published work behind
the credibility, experiment and digital-twin items (NASA-STD-7009B, DES practice at NASA, NNSA
and the Aerospace Corporation, digital-twin reviews) is summarised in
[docs/research.md](docs/research.md).

The window's UX parity and its gaps: [docs/target-app-parity.md](docs/target-app-parity.md).

## Where we are

| Crate | Layer | State |
|---|---|---|
| `random` | L0 | xoshiro256** streams; BPSim's distributions; `Dist`; fits to three-point estimates (`estimate`) |
| `stats` | L0 | `Tally`, `TimeWeighted`, Student-t 95% intervals, `Summary`; percentiles, empirical distributions with DKW bands, Wilson intervals (`spread`); paired and Welch differences between two runs (`difference`) |
| `des` | L1 | Event queue; async processes with `hold`, `spawn`, interrupts; priority and preemptive `Resource`; bounded and priority `Store`; waits with deadlines; `Signal`, `Gate`; runs that can be cancelled within a replication and stop when time stops moving (`run_watched`) |
| `model` | L1 | The model in memory: elements, connections with probabilities, conditions and quantities, finite buffers, shared resource pools, calendars, breakdowns, batches, stop conditions, three-point estimates, layout, entity properties, conveyors, the Transport block and mobile pools, `Path` (geometry with exact lengths), experiment, validation; BPSim's XPath expressions (`expr`); the capacity table (`capacity`); line balancing (`balance`: M7.1's core, the three heuristics, measures and theoretical minimum) |
| `blocks` | L2 | Compiles a model: source, server, shared task, delay, conveyor (accumulating or not, with divert routing), sink, exclusive and parallel gateways, merges, batch and unbatch, assembly (the Transport block has no runtime yet); finite buffers with blocking after service and loss; shared pools with roles, alternatives, priorities and shifts; breakdowns; calendars; stop conditions; work left inside at the end; entities carry typed properties; per-element statistics; warm-up; one replication stepped and read as it runs (`Live`) |
| `experiment` | L3 | Parallel replications with reproducible streams, progress and cancellation |
| `io` | L3 | BPSim on BPMN and XPDL: read (gateways, routing, embedded sub-processes in line, calendars, diagram layout, `stochcraft:` vendor extensions), write new, save edits into opened files |
| `engine` | L4 | Commands with JSON schemas, undo and redo, a journal with its format, version and starting state; runs on threads, on the page's workers or in slices; editing commands for blocks, flows, pools, calendars, properties, estimates and layout, and `batch` for several as one step; sweeps, sensitivity, designed experiments and sizing; `results.distribution` and `results.compare`; `model.capacity`, `estimate.fit` and `line.balance`; live runs |
| `stochcraft-cli` | app | `new`, `validate`, `explain`, `balance`, `run` with a run record, `design`, `fit`, `exec` (with `--journal`, `--all`, `--force`), `schema`, `mcp` |
| `render` | L5 | Parametric low-poly meshes (machines, straight, curved and spiral conveyors, racks, totes, operators, forklifts, AGVs) as plain vertex and index buffers |
| `ui-egui` | L5 | The app's window: menus and shortcuts; the flowchart canvas and inspector; runs with results, charts and the spread across replications; the Analysis and Capacity views; the 3D view and a read-only Layout view; the dashboard; watching a run from the run bar; autosave; 44 languages besides English |
| `mcp` | L5 | The MCP server: the engine's commands as tools for agents, headless (with optional read and write roots) or driving the app through its control channel |
| `stochcraft`, `stochcraft-web` | apps | The desktop shell (native file dialogs, `--control` for agents) and the browser shell, published on GitHub Pages |
| `bench/` | outside | M/M/1 against simu-des and SimPy (54× SimPy); tandem lines, the hold model and DEVStone LI, HI and HO |

Models are saved and opened as BPSim files, changed through engine commands, and run from the
command line or in a window, natively or in the browser
([helenkwok.github.io/StochCraft](https://helenkwok.github.io/StochCraft/)), and edited in the
window on the flowchart canvas (M3.4).

## Principles that hold across all milestones

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
   as prose. No vendor product names for features, only generic terms, no copied UI layouts,
   icons, example models or help text (CONTRIBUTING.md).

## Model files: no new format

How other simulation tools keep models:

| Tool | Model is stored as |
|---|---|
| SimPy, simu-des, DESMO-J | Program code (Python, C, Rust, Java); no model file |
| JaamSim | Its own plain-text configuration file (`.cfg`) |
| Some commercial simulation tools | Their own proprietary files |
| Stella (system dynamics) | XMILE, an OASIS standard |

And the sibling Craft apps: CADCraft saves DXF natively and keeps CADCraft-only data inside it as
extension data; SolidCraft has its own `.sc` only because parametric history has no standard
form, and still reads and writes STEP, IGES and 3MF; FieldCraft has `.fieldcraft` but writes
the input decks of the common finite-element solvers.

Open standards exist for exactly what StochCraft models:

| Standard | Covers | Use in StochCraft |
|---|---|---|
| **BPSim 2.0** (WfMC, December 2016, CC BY 3.0) on **BPMN 2.0** (OMG; ISO/IEC 19510) or **XPDL 2.2** (WfMC) | The process model (start events, tasks, gateways, end events, sequence flows, resources, diagram layout) is BPMN or XPDL; BPSim adds scenarios of simulation parameters that reference its elements by id | **Native model file** (M1) |
| **XMILE 1.0** (OASIS, 2015) | System dynamics: stocks, flows, auxiliaries | Read and written by the `sd` crate (M4); a hybrid model's BPMN+BPSim file names its XMILE variables (D13) |
| **SCXML 1.0** (W3C Recommendation, 2015) | State charts: states, transitions, events, timers | Each agent type's behaviour (M4.3, D16) |
| **CMSD** (SISO-STD-008-2010 and -01-2012) | Manufacturing data: resources, parts, routings, schedules | Import for factory models (later) |
| **DMN 1.6** (OMG, adopted 2026) | Decisions: decision tables with hit policies, the FEEL expression language | Routing and classification rules, read from a `.dmn` file beside the BPMN and named from it by `stochcraft:` vendor extensions (M5.5, D30) |

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
| L2 | `blocks`, `agents`, `sd`, `mc` | `blocks` compiles a `model::Model` into a simulation |
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

- In-memory types, not a file format: `Model { name, elements, connections, experiment }`,
  `Element { id, name, kind }` with `ElementKind::{Source, Server, Delay, Sink}` (named after
  BPMN elements), `Connection { id, from, to }` (a sequence flow, with its own id), and
  `Experiment { time_unit, warmup, duration, replications, seed }`. `io` maps them to and from
  BPSim on BPMN or XPDL. Resource pools and gateways join in M2.
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

- `blocks::compile(&Model) -> Result<Compiled, Vec<Diagnostic>>` refuses a model with errors
  (returning every diagnostic) and keeps its warnings. `Compiled` is `Send + Sync`, so one
  compiled model is shared by every worker thread of `stochcraft-experiment`.
- The builder API becomes `Builder`, a thin layer that writes a `Model` (names are ids), so code,
  files and the engine all run through `compile`.
- Results are data: `Report` (per element, one replication) and `ExperimentSummary` (a `Summary`
  per element and `Metric` across replications, from `summarise`). Writing them to files is
  `stochcraft-io`'s job (M1.3), so no serialisation library is added here.

### M1.3 `stochcraft-io`: BPSim on BPMN and XPDL

Two PRs. **Part 1** (done): reading BPMN and XPDL with BPSim 2.0 and 1.0, writing new models as
BPMN with BPSim 2.0, saving unchanged opened files byte for byte, and StochCraft's own test
models. Saving *edits* into an opened file came with the engine (M1.4, part 2). **Part 2**
(done): `cargo xtask fetch-bpsim`, the reference corpus in CI (every model and scenario reads,
unchanged saves are byte-identical, BPMN and XPDL versions give the same model, the reference result
files parse) and XSD validation of everything StochCraft writes. The statistical cross-check
against the reference simulator needs gateways, which all three reference models use, so it moves to M2.3.

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
  bpsim.org. **They are not committed:** the specification and guide state CC BY 3.0, but the
  example files and the XSD carry no licence of their own, so StochCraft does not redistribute
  them. `cargo xtask fetch-bpsim` downloads the guide's zip and the XSD from bpsim.org into a
  git-ignored `target/bpsim/`, checks each against a pinned SHA-256 and refuses a mismatch:

  | File | URL | SHA-256 |
  |---|---|---|
  | Implementer's Guide 2.0 zip | `https://www.bpsim.org/implementers/2.0/BPSim%20Implementer's%20Guide%20v2.0.zip` | `2d82a8763e7c3356c215ae0b237c64a672400d5e2c339e7421c5e8c790aa7b00` |
  | BPSim 2.0 XSD | `https://www.bpsim.org/schemas/2.0/BPSim-2.0.xsd` | `35999c20096c2cba9e6012d8cbb1b37d2deaac152d8da2ea4f95eb5a990ffcd2` |

  CI runs it before the tests and caches the result keyed by those checksums, so bpsim.org is
  contacted only when the cache is cold and an outage of the site cannot fail a normal run. The
  tests that need the corpus skip with a clear message when it is missing locally, and fail in
  CI (where `STOCHCRAFT_REQUIRE_BPSIM=1`), so they can never silently stop running. The
  hand-written models (M/M/1, tandem line, each element kind) are StochCraft's own and are
  committed, so most file tests run without any download. The bundle has each
  model (car repair, home loan, technical support) as both BPMN and XPDL, in BPSim 1.0 and 2.0,
  so both host formats and the 1.0 upgrade are tested from the start. Its Visio drawings and
  images are not needed. The BPMN MIWG test suite's `Reference/` models (BPMN interchange tests,
  with no simulation data) have joined the reading and round-trip tests the same way
  (`io/tests/miwg.rs`, in CI): fetched at a pinned commit by `cargo xtask fetch-miwg`, never
  committed (D14).
- **Cross-check against another simulator:** the bundle includes result scenarios
  (`*_results.bpsim`) produced by another BPSim simulator for the three BPSim 2.0 models, with
  per-replication values (for example 151 car repair arrivals in each of three replications,
  and 107, 96 and 100 customers leaving). StochCraft's results for the same scenarios must agree
  with them statistically: deterministic quantities exactly, random ones within confidence
  intervals, since the random number generators differ. They are also the reference for how
  StochCraft writes its own result scenarios (`result`, `instance` per replication,
  `resultTimeStamp`). All three models use gateways, so the comparison runs from M2.3.
  It already raises one question (decision D8): the reference simulator counts 151 arrivals for an inter-trigger
  time of 24 minutes over 60 hours, that is 150 intervals plus an arrival at time 0, while the
  specification describes the event as occurring "after the specified time interval".
- **Lenient reading, strict writing.** The BPSim 2.0 XML Schema (XSD, bpsim.org) fixes the order
  of child elements and the allowed property types, and two of the three reference models break
  it: the car repair model declares a property as `int` (the schema allows `long`), and the
  technical support model and its results put parameter groups and scenario parameters in a
  different order. StochCraft therefore reads any element order and the common aliases (`int` →
  `long`), reporting each deviation as a warning, and always writes elements in schema order.
  Every file and result scenario StochCraft writes is validated against the XSD in tests. A file
  whose BPSim data StochCraft did not change is saved back unchanged; one it changed has its
  BPSim data written in schema order.
- Results: written as a BPSim result scenario (min, max, mean, count, sum per requested
  parameter), and exported as CSV for spreadsheets.
- Saving through `craft-atomic` on native so a crash never truncates a model.

### M1.4 `stochcraft-engine`: commands

In two parts. **Part 1** (done): the command layer, undo and redo, the journal and runs.
**Part 2** (done): saving edits into opened files while keeping everything StochCraft does not
understand.

- `Engine { model, history, journal }` and `execute(id, params) -> Result<Value>` with a
  catalogue of commands: `model.new`, `model.set`, `model.get`, `model.validate`,
  `model.rename`, `block.add`, `block.remove`, `block.set` (parameters), `block.rename`,
  `connect`, `disconnect`, `experiment.set`, `undo`, `redo`, `history`, `run`, `run.start`,
  `run.status`, `run.cancel`, `file.open`, `file.save`.
- Each command has a JSON schema, generated from its parameter types with `schemars`, a summary
  and an example; the CLI, the MCP tool list and the documentation are built from them. A test
  runs every example, in catalogue order, on a new engine. Parameters are strict: a misspelt
  one is an error, never ignored. Errors carry a stable code (`not_found`, `invalid_model`…)
  and, for a model that cannot run, its diagnostics. The JSON forms (`stochcraft_engine::wire`)
  belong to the engine; the crates below it stay free of serialisation, and files stay BPSim.
- Undo and redo by exact inverses: every edit returns the edit that undoes it, kept as a typed
  value rather than JSON, so undo restores the model exactly (a randomised test checks
  undo-then-redo after every edit). `model.new` and `file.open` start a new document and clear
  the history; `history` reports what can be undone and whether there are unsaved changes.
- A journal of every command except queries, replayable as a script (`Engine::replay`, and
  `stochcraft-cli exec` in M1.5); undo and redo are journalled as commands, so a replay ends
  in the same state.
- `run` returns the results at once (`ExperimentResult` as JSON: per element and metric, the
  mean, 95% half-width, standard deviation and n). `run.start` runs in the background, on
  worker threads natively and in slices (a bounded number of events per `run.status`, see
  M3.6) in the browser;
  both give exactly the results of `run`. `run.cancel` stops new replications from starting
  (`stochcraft-experiment` gains a `Control` with progress and cancellation), and stops those
  already running too: a replication asks whether it has been cancelled every 1,024 events
  (`des::Sim::run_watched`). A replication whose clock stops moving (more than
  `blocks::MAX_EVENTS_AT_ONE_TIME`, 10,000,000, events at one time) ends with an error naming
  the time and the element of the loop (#49, #52).
- `file.save` writes edits into the file the model was opened from (`stochcraft-io`'s
  `Opened::save`), changing only what the edit needs:
  - names; parameters and experiment settings in the scenario that was opened (where it
    inherits a value, it gets its own); resource quantities for capacities;
  - added and removed elements and connections, as BPMN or XPDL elements in the model's order,
    with `incoming`/`outgoing` references, a resource for each new server, diagram shapes and
    edges next to their neighbours, and the parameters of removed ones taken out of every
    scenario;
  - indentation and prefixes copied from the file, compact XML kept compact, and new names as
    character references in a file declared in an encoding other than UTF-8;
  - a new time unit, which relabels the model's times (their numbers stay): `baseTimeUnit`
    changes, and each value in the scenario that names its own unit keeps that unit with its
    number rescaled, so it reads as the model says (`mean="80" timeUnit="s"` under minutes
    becomes `mean="4800" timeUnit="s"` under hours, `PT1000M` becomes `PT60000M`; a duration of
    several parts is written in its smallest one). A scenario without a duration gets one, since
    the 8 hours it would run is another number in the new unit. A rescaled value that no decimal
    reads back as exactly may read back within 1e-12 of itself; every other value must read back
    exactly. Scenarios inheriting from the opened one keep their meaning: one that took its base
    unit from it gets the old unit written in, with the rescaled values as they were.

  Everything else is kept byte for byte: the XML writer now reproduces a parsed file exactly
  (each start tag's own text, `<a/>` or `<a></a>`, the whitespace around the root, and a
  changed tag keeps the layout of its other attributes), which also fixed a doubled `xml` in
  re-written declarations. The saved file is read back before it is returned, and an edit that
  would not read back as the model is refused with the reason (for example changing the time
  unit of a scenario that inherits values with their own units, reordering elements, or changing the
  capacity of a resource several tasks share, which waits for M2.5); `"as_new": true` then
  writes a new BPMN file from the model alone. Tests: edits on the hand-written models with a
  bound on the lines changed, and 180 edits across every reference model and scenario, each
  reading back as the edited model, leaving non-inheriting scenarios as they were, and keeping
  a schema-valid file valid (120 checked with `xmllint`).

### M1.5 `apps/stochcraft-cli`

Done. Every subcommand goes through the engine's commands, as the app and agents will.

- `clap` subcommands:
  - `new [--template mm1|mm2|tandem] [--list] [--force] <file.bpmn>`: built from engine
    commands and saved as a new BPMN file
  - `validate <file> [--scenario S] [--format table|json]`: the notes from reading the file and
    the model's diagnostics; exit code 1 if any is an error
  - `run <file> [--scenario S] [--reps N] [--seed S] [--threads T] [--warmup W] [--duration D] [--format table|json|csv] [--out file]`
  - `exec <script.json> [--open file]`: replay engine commands (a journal), one line of JSON
    per step; exit code 1 at the first step that fails (since #217: `--journal out.json`,
    `--all` and `--force`; see docs/developers.md)
  - `schema [command]`: print command schemas
- The table output reads like a textbook result: block, metric, mean, ±half-width, n.
- **A run record with every result** (NASA-STD-7009B [M&S 24], [M&S 25], [M&S 27], [M&S 34];
  see docs/research.md): StochCraft's version, a digest of the model, the file and scenario, the
  seed, replications, warm-up and duration, the diagnostics and file notes seen, and how the
  uncertainty was estimated ("95% Student-t interval over n independent replications"). The JSON
  output carries it in full; the table prints it as a header. Results can be reproduced from the
  record alone, since a seeded run gives the same numbers on any machine. The record is built by
  the engine (`run` results carry it), so the app and agents get it too.
- Integration tests run the binary: every template written, validated and run; the M/M/1
  template against queueing theory; results independent of threads and following the seed;
  table, CSV and `--out`; exit codes for errors; scripts replayed; schemas printed.

### M1.6 Determinism and the browser

Done.

- A `stochcraft-testkit` golden-results test: each golden model (`mm1.bpmn`, `tandem.xpdl`, and
  `distributions.bpmn` with an element for every BPSim distribution) is run through the engine,
  and its results digest is pinned in `tests/golden/results.txt`. It must match natively and as
  `wasm32-wasip1`, run by `cargo xtask golden` under Node's built-in WASI (no runtime to
  install; CI's wasm job runs it). `STOCHCRAFT_BLESS=1` rewrites the digests when a change is
  meant to alter results.
- Elementary functions come from the pure-Rust `libm` through `stochcraft_random::math`, and
  `clippy.toml` forbids `f64::ln` and the like, so results do not depend on the platform's maths
  library (docs/architecture.md).
- wasm64 joins when its toolchain is checked (README of the wasm64 plan).

**M1 size:** about six PRs (M1.1 … M1.6). M1.1–M1.3 can land without the engine; M1.4–M1.5
depend on them. **M1 is complete.**

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

Done. Needed by the blocks below, each with its own unit tests; the bounded store and the
preemptive resource are also checked against M/M/1/K blocking and two-class preemptive-resume
priority theory:

| Primitive | For | API |
|---|---|---|
| Bounded `Store` (waits when full) | finite buffers, blocking lines | `Store::bounded`, `put_wait().await`, `try_put` |
| Priority `Resource` and `Store` (priority, then FIFO) | priority rules, urgent orders | `acquire_with_priority`, `Store::with_priority` |
| Interrupt / preempt a process | breakdowns, preemptive priority | `Ctx::interrupt`, `hold_interruptible`, `Resource::preemptive` |
| Timeouts on waits | reneging, balking | `acquire_within`, `get_within` → `Err(TimedOut)` |
| Condition / event (wait until signalled) | gates, schedules, assembly | `Signal` (`notify_all`), `Gate` (`open`, `close`) |

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

**Done.** `stochcraft_model::expr` parses BPSim's XPath subset once and evaluates it with
XPath 1.0's conversions:
- numbers, strings, arithmetic, comparisons, `and`/`or` and `getProperty`;
- `not`, `boolean`, `number`, `string`, `floor`, `ceiling`, `round`, `concat`, `contains`,
  `starts-with` and `string-length`.

Paths, variables, other functions and `getResource` outside a resource selection are
diagnostics that give the character position.

Elements carry `Property` assignments, read from and saved to `PropertyParameters`. Types come
from the declaration, or else from the first value. `int` and other spellings outside the
schema are read with a warning.
- **When:** an element assigns as an entity enters it, and a source as it creates one.
- **How:** all of an element's assignments are made together, from the values the entity
  arrived with.
- **Random values:** drawn from streams of their own, so adding a property changes no other
  result. (The elements' own streams are keyed by their ids since #152, so adding, removing or
  reordering an element leaves the others' streams as they were; see M2.9.)

Each element reports the mean value it assigned to each property. The engine has
`property.set`, `property.remove` and `property.list`, and `model.get` and `run` include
properties.

Every expression in the reference models' properties and branch conditions parses. The repair
loop itself needs gateways, so the car repair count cross-check moves to M2.3 with D8; its
mechanics (a count set on arrival, decremented per repair, tested for zero) are checked on an
unrolled line in `stochcraft-blocks`.

### M2.3 Routing

| Block | Behaviour | Validation |
|---|---|---|
| Exclusive gateway, by `probability` | Output chosen by weights (BPSim default: the rest share evenly) | Jackson network: throughput and W per node against product-form results |
| Exclusive gateway, by `condition` | First outgoing flow whose expression is true | Deterministic cases; the car repair loop |
| Parallel gateway (split and join) | Copies a token to every branch; joins wait for all | Deterministic fork-join cases; the home loan model |
| Merge (several flows into one element) | Tokens from several inputs | Superposition of Poisson streams |

**Done:**
- **Gateways:** `exclusive_gateway` and `parallel_gateway` element kinds. Connections carry a
  BPSim `Probability` and `Condition`, read from and written to BPMN and XPDL (an XPDL
  `OTHERWISE` transition, or BPMN's `default` attribute, is the default flow).
- **Exclusive gateway:**
  - If any outgoing connection has a condition, the first in model order whose condition holds
    is taken, else the default, else the entity leaves the model there, with a warning when the
    model is checked.
  - Otherwise one connection is drawn from the gateway's own stream: connections without a
    probability share evenly what the others leave, and probabilities that do not add up to 1
    are used in proportion, with a warning.
  - With a single way out, nothing is drawn.
- **Parallel gateway:** copies each entity, as new entities with the same creation time and
  properties, down every outgoing connection. With several incoming connections it is a join: it
  waits for one entity on each and sends on the one from the first incoming connection, with any
  property it lacks taken from the others in that order.
- **Merges:** any element may have several incoming connections.
- **Validation:** probabilities outside 0 to 1, malformed conditions, conditions that read unknown
  properties, routing given where it has no effect, a default that is not an outgoing connection,
  and joins that wait on a connection no entity reaches.
- **Embedded sub-processes** (BPMN `subProcess`, XPDL `BlockActivity`) with one plain start event
  and one end event are simulated in line: their start and end events pass entities straight
  through. Event sub-processes are left out with a note.
- **Commands:** `flow.set` sets a connection's probability and condition; `connect` takes both;
  `block.set` takes an exclusive gateway's `default`.

**Validation** (`blocks/tests/routing.rs`, `io/tests/cross_check.rs`):
- **Against theory:** a Jackson network (λ = 1, μA = 4, μB = 2, half fed back) matches product
  form for the arrivals at each station, utilisation, waits and time in system (2.01 ± 0.03
  against 2). Two merged Poisson streams give M/M/1 with λ = 0.75 (W = 4.09 ± 0.13 against 4).
- **Exact cases:** the condition loop, a fork and join, routing with no default, and the even
  share of unset probabilities.
- **Car repair against another BPSim simulator's published results for scenario S1**, from 400 replications of
  StochCraft's against the reference simulator's three:
  - **Arrivals:** 150 by default, 151 with the first arrival at time 0, as the reference simulator counts in every
    replication.
  - **Rejected estimates, repairs started, completed and customers leaving:** within 0.15
    standard errors (0.01 with the first arrival at 0).
  - **Issues repaired:** 345 against the reference simulator's 306 (−1.2 standard errors), within the check. The
    model's `noOfIssues` is `int`, a truncated normal; StochCraft rounds it to the nearest whole
    number, and the reference simulator's three replications cannot tell whether it does the same.
- **Home loan:** its two exclusive and two parallel gateways now read, but it also needs
  intermediate message events and a boundary timer, so its cross-check waits for event support.

### M2.4 Buffers and blocking

- Finite buffers between tasks, as a `stochcraft:` vendor extension (BPSim has no capacity on
  sequence flows); a full buffer blocks the upstream task (blocking after service).
- Validation: M/M/1/K loss probability and L; a two-machine line with a finite buffer against the
  published closed-form throughput.

**Done:**
- **Where the capacity lives:** on the receiving server's queue (`queue_capacity`, the most
  entities waiting for a unit, not counting those in service), not on a connection. A BPMN task
  has one queue shared by all its incoming sequence flows, and a sequence flow holds nothing, so
  a capacity per flow would describe a store BPMN does not have. Left out, the queue is unlimited
  as before; 0 means no waiting room (an entity gets in only when a unit is free).
- **When the buffer is full**, what the sender does depends on what it is:
  - **a server** keeps the served entity and its unit until there is room (blocking after
    service); the unit is blocked, not busy;
  - **a delay** keeps the entity, past its delay;
  - **a gateway** waits, holding up the entities behind it, which queue before the gateway
    without limit (a warning says so);
  - **a source** loses the arrival (M/M/1/K's loss); an arrival at the instant a place frees gets
    in, whatever the order of the two events.

  The runtime gives each finite buffer a pool of places, one per unit plus the waiting room. An
  entity takes one before it is sent and gives it back as it leaves, after any blocking of its
  own.
- **Results**, reported only where a finite buffer is involved, so results and golden digests
  of models without one are unchanged:
  - **Blocking**, for servers and delays sending to a finite buffer:
    - `mean_blocked_time`: per entity that left;
    - `blocking_probability` (servers): the time-average fraction of units blocked.
  - **Loss**, for servers with a finite buffer:
    - `lost`;
    - `loss_probability`: lost over lost plus entered.
- **File:** `<bpsim:VendorExtension name="stochcraft:queueCapacity" xmlns:stochcraft="https://helenkwok.github.io/StochCraft/bpsim/1" stochcraft:value="3"/>`
  last in the task's `ElementParameters`, in BPMN and XPDL, BPSim 2.0 and 1.0. BPSim checks a
  vendor extension's value strictly against a schema for its namespace, so StochCraft ships one
  (`crates/io/schemas/stochcraft-bpsim-1.xsd`). The schema test validates against it, imported
  next to the BPSim schema.
  - **Saving in place:** adding one adds that line, and changing it changes only its value.
    Removing it gives back the original bytes.
  - **Reading:** a bad value is an error note. On a task without a resource the extension is
    ignored, with a warning. Other `stochcraft:` names are kept, with a warning that this version
    does not know them.
- **Validation:** `buffer_behind_gateway` (above) and `blocking_cycle`, a loop of finite-buffer
  servers that deadlocks when every buffer in it is full. Both are warnings.
- **Transfer time** (issue #89): a part needs a fixed or random time to reach a server (a short
  conveyor, a hand transfer), set on the receiving server (`transfer`, a distribution).
  - The part takes its place in the receiving buffer when its transfer starts, so the capacity
    counts parts in transit as well as those waiting, and joins the queue when the transfer
    ends. Its wait (`mean_wait`) counts from then.
  - A server or a task with shared resources sending it waits after its job until there is
    room (blocking after service, as before), then its unit stays occupied through the whole
    transfer. Sources, delays, gateways, batches and splits hold nothing: the part travels on
    its own (a source still loses its arrival when there is no room).
  - **Results:** a part in transit counts once, in the receiving server's work in progress
    (`entered` there counts it as the transfer starts), not in its queue length or the number
    in service, and not at the sender. The sender's `blocking_probability` counts its units
    held through a transfer as blocked; `mean_blocked_time` stays the wait for room alone.
  - Transfer times come from a stream of their own per receiving server, forked after all the
    others, so models without transfers draw exactly as before.
  - **A minimum time in the buffer** (issue #96): a transfer that does not hold the sender
    (`holds_sender` false). The sending unit is free as soon as the part is in; the part counts
    against the capacity from then and can be taken only after the time. It matters: on the
    assembly line of Case B (Schmitt et al. 2026, two presses of 175 s behind 32 s buffers), an
    independent model gives about 23 accepted parts/h when the presses are held through the
    transfer and 27.0 when they are not, against 27.26 published.
  - A unit that fails while holding a part through a transfer lets the transfer finish on
    time (it is already on its way); the unit is failed from then on and takes no new part
    until repaired.
  - **File:** `<bpsim:VendorExtension name="stochcraft:transferTime" xmlns:stochcraft="…"><stochcraft:transferTime><bpsim:FloatingParameter value="10"/></stochcraft:transferTime></bpsim:VendorExtension>`
    in the receiving task's `ElementParameters` (any BPSim distribution in place of the
    constant), with `holdsSender="false"` on `stochcraft:transferTime` for a minimum time in
    the buffer; read, written, saved in place and in the vendor schema; `block.set` and
    `block.add` take `transfer` (`null` removes it) and `transfer_holds_sender` (needs a
    transfer; a new `transfer` keeps it). Not editable in the window yet.
  - **Checks** (`blocks/tests/transfers.rs`, worked by hand): two machines with fixed cycles
    and a buffer of two, the second's buffer never empty, give exactly 1 / max(cycle₁ +
    transfer, cycle₂) parts per time unit with either machine the bottleneck, and
    1 / max(cycle₁, cycle₂) with a minimum time in the buffer instead; the first machine is
    blocked exactly its transfers' share of the time (none with a minimum stay); a part in
    transit is counted once, at the receiving machine; and, stepping through a run, the
    receiving machine never holds more than its unit plus its waiting room, transit included.
- **Routing to the shortest queue** (issue #96): an exclusive gateway with `routing`
  `shortest_queue` (`stochcraft:routing="shortestQueue"`) sends each entity to the way with
  room whose next element has the fewest entities waiting or in transit (those in service not
  counted); a tie goes to the way taken longest ago, so equal ways take turns.
  - The element sending into the gateway does the routing itself, so it is the one that waits
    while no way has room: a server stays blocked after its job (and a transfer into the way
    chosen then holds its unit, if the transfer holds senders), and a source loses its arrival
    (counted lost at the gateway and the source). Nothing waits before the gateway, so
    `buffer_behind_gateway` does not apply to it.
  - A freed place goes first to a plain waiter for that buffer (an element with that single
    way), then to entities routed through the gateway.
  - Probabilities, conditions and a default on its ways have no effect (`routing_ignored`).
    Standard gateways keep their behaviour: they pick a way first, then wait for room there.
  - **File:** `<bpsim:VendorExtension name="stochcraft:routing" xmlns:stochcraft="…" stochcraft:value="shortestQueue"/>`
    in the gateway's `ElementParameters`; on any other element it is ignored with a warning.
    `block.set` and `block.add` take `routing` (`standard` or `shortest_queue`). Not editable in
    the window yet.
  - **Checks** (`blocks/tests/shortest_queue.rs`, worked by hand): ties take turns and the
    shorter queue wins; a full way is passed over and the sender is blocked until either way
    frees, for exactly the times worked out; a source loses its arrival when every way is
    full; and a transfer into the way taken holds the sender or not.
- **Commands:**
  - `block.set` takes `queue_capacity`; `null` removes it, and undo is exact.
  - `block.add` and `model.set` take it with the other server parameters.
  - Model JSON has the key only when it is set.

**Validation** (`blocks/tests/buffers.rs`, 16 replications of 20,000 after a warm-up of 1,000):
- **M/M/1/K:**

  | Case | Loss probability, theory | Loss probability, StochCraft | L, theory | L, StochCraft |
  |---|---|---|---|---|
  | λ = 0.9, μ = 1, K = 5 | 0.12602 | 0.12613 ± 0.00229 | 2.19478 | 2.19085 ± 0.01590 |
  | λ = 1.5, μ = 1, K = 3 | 27/65 ≈ 0.41538 | 0.41450 ± 0.00243 | 129/65 ≈ 1.98462 | 1.98126 ± 0.00647 |

  Lq, utilisation and Wq are within their intervals too.
- **Two exponential machines with b waiting places between them**, the first never starved. The
  line is a birth-death chain on 0..b+2, so its throughput is μ2 (1 − p0) (docs/research.md).
  Measured from the parts leaving over 20,000:

  | Case | Throughput, theory | Throughput, StochCraft | P(first machine blocked), theory | P(first machine blocked), StochCraft |
  |---|---|---|---|---|
  | μ1 = 1, μ2 = 1.25, b = 2 | 0.87815 | 0.87937 ± 0.00289 | 0.12185 | 0.12216 ± 0.00162 |
  | μ1 = μ2 = 1, b = 1 | 3/4 | 0.75073 ± 0.00248 | 1/4 | 0.25011 ± 0.00229 |

  Mean blocked time, both utilisations and the second machine's queue are within their
  intervals too.
- **Exact cases:**
  - a constant line worked by hand, checking blocked times, waits and times in system;
  - loss with no waiting room, including an arrival at the instant the unit frees;
  - a delay blocked after its delay;
  - a gateway held up by a full buffer;
  - no buffer metrics without buffers.

**In the window:** a task with a resource has a **Waiting room** parameter: unticked it is
unlimited, ticked it starts at room for ten. Results label the buffer metrics (mean time blocked,
share of units blocked, lost, share of arrivals lost) and chart the two shares where a run has
them.

### M2.5 Shared resources

**Done.**

- **Pools:** BPMN `resource` and XPDL `Participant` (type RESOURCE) elements become pools in the
  model (`Model::resources`) with BPSim `quantity` (default 1) and `role` (any number), when a
  task shares them.
- **Shared tasks:** a task takes its units from the pools by its `selection`, parsed in
  `io/src/selection.rs`:
  - `getResource(id, qty)`;
  - `getResourceByRoles([role, …], qty)`, which takes units from the pools that play every role,
    in model order (`('a', 'b')` and a flat list are read too);
  - `orResource(…)` for alternatives, tried in order.
- **When a task stays a server:** a task that alone uses one unit of one resource stays a server
  with its own capacity, as before, so existing models and their golden results are unchanged.
  Anything else (a resource used by several tasks, roles, several units, alternatives) makes it a
  shared task (`ElementKind::SharedTask`).
- **How units are given out:**
  - each entity takes all its units at once, or none, and holds them for its service and while
    blocked after it;
  - when units come free they go to waiting entities by their task's BPSim `priority` (higher
    first), then by arrival;
  - an entity that cannot be served now does not hold back one behind it that can (no
    head-of-line blocking).
- **Results:** each pool reports its utilisation and mean units busy, as an extra entry of kind
  `resource` after the elements, so the CLI, CSV and window show pools without a new format.
- **Validation:**
  - a pool with no units, a need for an unknown pool, roles no pool plays, and a need larger than
    the pools can ever meet (an error when no alternative can be met, else a warning);
  - a priority that is not a finite number;
  - a pool no task uses (a warning).
- **Files:** pools, roles, selections and priorities are written to new files and saved into
  opened ones. In an opened file, a pool's quantity and a shared task's service, needs and
  priority save in place. Adding or removing pools, renaming a pool, changing its roles, or
  turning a task into a shared task or back is for a new file: the app gives the reason and offers to save a new BPMN file.
- **Commands:**
  - `resource.add`, `resource.set` and `resource.remove` (refused while a task names the pool),
    with exact undo;
  - `block.add` with `"kind": "shared_task"`;
  - `block.set` with `needs` and `priority`;
  - model JSON has `resources` only when there are some.
- **Not here:** resources held across several tasks (BPMN sub-processes); calendars
  (`availability`, quantity by shift) came in M2.6. Erlang loss (M/M/c/c) needs a waiting-room
  limit on a shared task, which this design does not have; a server with `queue_capacity` 0
  already gives M/M/c/c (M2.4).

**Validation** (`blocks/tests/resources.rs`, 16 replications; docs/research.md, "Shared resources"):

| Case | Theory | StochCraft |
|---|---|---|
| Two tasks sharing 3 units, λ = 0.9 + 1.2, μ = 1 (one M/M/3): wait of each | 0.54705 | 0.54246 ± 0.01372, 0.54082 ± 0.01297 |
| Same: pool utilisation | 0.7 | 0.69903 ± 0.00217 |
| Non-preemptive priority M/M/1, λ = 0.3 (urgent) + 0.45, μ = 1: urgent wait | 1.07143 | 1.06908 ± 0.02114 |
| Same: routine wait | 4.28571 | 4.30888 ± 0.18513 |
| Machine repair M/M/2//6, λ = 0.25, μ = 1: repairmen utilisation | 0.57642 | 0.57644 ± 0.00115 |
| Same: machines running | 4.61138 | 4.61198 ± 0.00414 |

Exact cases worked by hand check alternatives in order, roles across two pools, several units
taken together, and a large need waiting without holding back a smaller one.

### M2.6 Time-dependent behaviour

- **Calendars** (done): BPSim calendars are iCalendar (RFC 5545) `VEVENT`s; a parameter value
  can be `validFor` one. `stochcraft_model::calendar` reads and writes the subset shifts and
  opening hours need:
  - `DTSTART`, `DTEND`, and `RRULE` with `FREQ=DAILY` or `WEEKLY`, `INTERVAL`, `BYDAY` (plain
    weekdays), `WKST`, `UNTIL` and `COUNT`;
  - indented and folded lines as XML files carry them;
  - descriptive properties (`UID`, `DTSTAMP`, `SUMMARY`, …) are skipped; anything else that
    would change when a calendar is active (`EXDATE`, `RDATE`, other components, `TZID`) is
    reported and ignored, and an event with an unsupported rule is left out with an error.

  Times are wall-clock times in one zone. Model time 0 is the scenario's `Start`, or midnight
  of the day the earliest event starts. Where several calendars are active, the first value
  listed applies; outside them, the value without `validFor`.
  - **Arrivals:** `interTriggerTimer` values for calendars. When the value changes, the
    pending arrival is dropped and the next is drawn from the new value at once (piecewise
    Poisson for exponential times). With no value outside the calendars there are no arrivals
    there.
  - **Pools:** `quantity` by shift and `availability` (true/false, by calendar). A resource with
    either is a shared pool even for one task. When its units go down, those in use finish their
    job first and nobody new starts. Utilisation is busy units over the units there, both
    averaged over time.
  - **Files:** calendars, `Start`, and values for calendars are read, written for new files,
    and saved into opened files (an edited calendar in place; one from an inherited scenario
    only with Save As). Other parameters' values for calendars are ignored, with a warning.
  - **Commands:** `calendar.add`, `calendar.set` and `calendar.remove` (refused while a value
    is valid for it); `block.set` with `timed` and `idle_outside` on sources; `resource.set`
    with `shifts`, `available` and `availability`; `experiment.set` with `start`. Model JSON
    has `calendars` only when there are some.
  - **Not here:** editing calendars in the window (the engine's commands are ready for it).
    The technical support model still does not run, for reasons unrelated to calendars
    (percentages as probabilities, left-out intermediate events); its shifts are checked
    instead.

  **Validation** (`blocks/tests/calendars.rs`, 16 replications; `io/tests/reference.rs`):

  | Case | Theory | StochCraft |
  |---|---|---|
  | Poisson arrivals at 0.5/min, 2/min from 8 to noon: a whole day | 1080 | 1070 ± 18.8 |
  | Same: 8 to noon | 480 | 473.1 ± 11.3 |
  | Same: noon to midnight | 360 | 361.6 ± 8.1 |
  | Technical support S3: arrivals in the first day, seven shifts each drawn afresh | 2206 | 2206 |
  | 1 unit, 3 from 8 to 4 pm, saturated, service 1: jobs in a day | 2400 | 2400 |

  Exact cases worked by hand also check a rate change taking effect at once, no arrivals
  outside the calendars, and a unit finishing its job after its shift has ended.
- **Breakdowns on servers** (done): a `stochcraft:breakdown` vendor extension, since BPSim has
  none, with a `timeToFailure` and a `repairTime` distribution; BPSim's own `Interruptible`
  chooses whether a failure interrupts the job under way.
  - Each unit fails on its own, in elapsed time (busy or idle) since its last repair.
  - Not yet: **failures counted in working time.** A choice of clock for the time to failure:
    elapsed time (today's rule) or only the time the unit is busy, since the last repair. Machines
    that wear with use fail by working hours, so a slower line fails less often per day; with
    the elapsed-time clock it fails as often idle as busy. Saved as an attribute of
    `stochcraft:breakdown` (default: elapsed, so files read as before), with a check against
    the closed form for a single machine whose failures count in busy time (availability
    against the share of time worked). Also offered by desflow (MIT), seen 2026-11.
  - Interruptible: the job stops and resumes on the same unit after the repair, with the
    service time it had left (preemptive resume). Otherwise the failure waits for the job to
    finish, and the unit takes no new job meanwhile.
  - Waiting entities go first come first served to the first unit that is up and idle.
    Servers without breakdowns run exactly as before.
  - **With a finite buffer** (issue #96): a unit that is down and holds no entity keeps its own
    place in the buffer free until its repair, so the buffer holds only its waiting room
    meanwhile (a broken machine takes no part). A unit that fails while holding a finished
    entity, waiting for room in the next buffer, hands it on as soon as there is room, under
    repair (`hands_on_while_down`, the default), or keeps it until repaired
    (`stochcraft:breakdown handsOnWhileDown="false"`). No source states which (the papers
    behind Case B below are silent), so it is the model's choice; on Case B the difference is
    0.2% of throughput.
  - Results: a server's `availability` (the time-average fraction of units up; "Availability"
    in the window, in every catalog). Utilisation counts a unit holding an interrupted job as
    busy.
  - Read, written, saved in place, and in the vendor schema
    (`crates/io/schemas/stochcraft-bpsim-1.xsd`); `block.set` and `block.add` take
    `breakdown` (`null` removes it).
  - **Not here:** failures counted in busy time only, breakdowns on shared pools, and editing
    breakdowns in the window (the engine's commands are ready for it).

  **Validation** (`blocks/tests/breakdowns.rs`, 16 replications):

  | Case | Theory | StochCraft |
  |---|---|---|
  | One unit, failures exp(mean 100), repairs exp(mean 10): availability | 0.90909 | 0.90574 ± 0.00472 |
  | Same, saturated, service rate 1, interruptible: jobs in 20,000 | 18,181.8 | 18,157.9 ± 129.0 |
  | λ = 0.3, failures exp(mean 50), repairs exp(mean 5), interruptible: mean in service, λ E[S](1 + E[R]/MTBF) | 0.33 | 0.33126 ± 0.00217 |
  | Constant: failure 10 after repair, repair 2, jobs of 3, waiting for the job: jobs by 70 | 20 | 20 |
  | Same, interruptible: jobs by 69 | 19 | 19 |
- **Calendars on servers** (issue #88): a server's units follow an availability calendar, as a
  pool's do, alongside any breakdowns.
  - **Model:** `available` (outside the calendars, default true) and `availability` (on or off
    while each calendar is active, the first active one applying) on the server; all its units
    follow them. Varying the number of units by calendar (`quantity` shifts) stays a pool's
    (until a task's own units by shift, M2.10).
  - **Semantics:**
    - A unit checks the calendar only when it takes its next entity, so a job under way when a
      break starts finishes first (as a pool's unit does).
    - A job a failure interrupted resumes on its unit after the repair, even if the break has
      begun meanwhile.
    - Failures keep coming in elapsed time through the breaks, and a repair goes on through
      them. Counting failures in busy time only could be an option later.
    - Servers with neither a calendar nor breakdowns run exactly as before; servers with
      breakdowns and no calendar too.
  - **States:** at every instant each unit is in exactly one state, the first that applies of
    failed (or under repair), processing, blocked, off shift and starved (idle, on shift and
    up). Off shift means only idle because the calendar has the units unavailable: a job that
    runs on into a break, or resumes off shift after a repair, is processing, and a finished
    entity held during a break is blocked. Results add `time_off_shift`, the time-average fraction
    of units off shift, for servers with a calendar ("Time off shift" in the window,
    in every catalog).
    - `availability` stays the fraction of units not failed, whatever the calendar.
    - `utilisation` stays the fraction holding a job, through breaks and repairs alike.
      (Changed in M2.10: over the units on shift, as for a pool; the breaks are in the unit
      states.)
    - The saturation warning judges a server busy over the time its units were up and on
      shift.
  - **File:** BPSim's own `Availability` on the task's resource (`ResourceParameters`, values
    `validFor` a calendar), read, written and saved in place. A resource of one task that is
    only available or not by calendar is now read as that task's own, a server with a
    calendar. Before, it became a one-task pool. A resource whose quantity varies by calendar
    is still a pool. `block.set` and `block.add` take `available` and `availability`. Not
    editable in the window yet.

  **Validation** (`blocks/tests/calendars.rs`):

  | Case | Theory | StochCraft |
  |---|---|---|
  | Jobs of 1.5 waiting from 0, off shift 10 to 20: jobs by 29.5 (the one under way at 10 finishes at 10.5) | 13 | 13 |
  | Same: off-shift share (from 10.5), utilisation (over all time before M2.10: 20/29.5; over the time on shift since, 20/20) | 9.5/29.5, 1 | exact |
  | Jobs of 4, failure 7 after each repair, repairs of 5, interruptible, off shift 10 to 20: jobs by 30 (the job interrupted at 7 resumes at 12, off shift) | 3 | 3 |
  | Same: availability, off-shift share (the resumed job [12, 13) is processing) | 20/30, 6/30 | exact |
  | Saturated, failures exp(mean 100), repairs exp(mean 10), off shift midnight to 8: availability A | 0.90909 | 0.90942 ± 0.00437 |
  | Same: off-shift share, A/3 less the time finishing jobs in breaks | 0.30303 | 0.30249 ± 0.00290 |
  | Same: jobs in 20 days, 28,800 × 2/3 × A (plus jobs finishing after a close) | 17,454.5 | 17,497.1 ± 112.8 |
- **Energy per server state** (issue #90, a first step towards the energy and CO₂ measures
  under Later):
  - **Model:** `power` on a server, the kilowatts each unit draws in each state: `processing`
    and `idle` (needed), and `blocked`, `failed` and `off_shift`, each `idle` when left out.
    Tasks with shared resources, delays and pools have no power yet.
  - **States:** at every instant each unit is in exactly one state, the first that applies of
    failed (or under repair), processing (a job running on into a break, or resumed off shift
    after a repair, included), blocked (holding a finished entity while the next buffer is
    full, or through a transfer, in a break too), off shift (idle because the calendar has the
    units unavailable) and starved (idle, on shift and up).
    These are the unit-state counts of the calendars above, which every server keeps and the
    warm-up resets with the other statistics: there is one accounting.
  - **Energy:** each state's power times the exact time the units spent in it, cut at the end
    of the warm-up and at the end of the run (not booked when a state ends), converted from the
    model's time unit to hours: kilowatt-hours.
  - **Results**, only for servers with a power, so other models report as before:
    - `energy` (kWh, "Energy");
    - `time_processing`, `time_starved`, `time_blocked`, `time_failed` and `time_off_shift`,
      the time-average fraction of units in each state. They add up to 1.
      - `time_blocked` leaves out a failed unit holding an entity, which
        `blocking_probability` counts.
      - `time_failed` is 1 − `availability`.
      - `time_processing` leaves out an interrupted job's repairs, which `utilisation`
        counts.
    - **Energy per part** (`specific_energy`, "Energy per part"): at every sink of a model with
      a power, the energy of all servers with a power over the window, divided by the entities
      that reached that sink in it.
      - Choosing a sink means reading that sink's value; with several sinks, each reports its
        own ratio.
      - A replication with nothing out has no ratio (never 0). The summary is the mean of the
        others' ratios, and its `n` says how many there were.
  - **File:** `<bpsim:VendorExtension name="stochcraft:power" xmlns:stochcraft="…"><stochcraft:power processing="12.5" idle="2" failed="0.5"/></bpsim:VendorExtension>`
    in the task's `ElementParameters`, read, written, saved in place and in the vendor schema.
    A negative or non-numeric power is an error (`bad_power`). `block.set` and `block.add`
    take `power`, and `null` removes it. Not editable in the window yet.

  **Validation** (`blocks/tests/energy.rs`, worked by hand):

  | Case | Theory | StochCraft |
  |---|---|---|
  | Jobs of 1 min every 4, 10 kW processing, 2 kW idle, window [32.5, 92.5) cutting a job at each end: energy, energy per part | 4 kWh, 4/15 | exact |
  | Same, times in seconds, hours and days | 4 kWh | exact |
  | The failing server on a calendar above (processing 14, failed 10, off shift 6 of 30), 10/2/0.5/0.2 kW | 146.2/60 kWh | exact |
  | Same, failed and off shift left out (idle power) | 172/60 kWh | exact |
  | The transfer line above: the first machine processing 48 and blocked 32 of 80 at 5 and 3 kW; the second processing 32 at 4 kW and idle 48 at 0.5 kW; 16 parts | 336/60, 152/60 kWh; 488/60/16 per part | exact |
  | Rare arrivals, 20 replications, some with nothing out: the summary's mean and `n` are those of the replications with a ratio | | exact |
- **Time-average work in progress** (issue #87): `mean_work_in_progress` ("Mean work in
  progress") on servers, shared tasks, delays, batches and joins. It is the time-average number
  of entities inside over the window, each counted once wherever it is in the element: in
  transfer to it, waiting, being served or delayed, or held while blocked. Summed over a line's
  elements, it gives the line's work in progress. `work_in_progress` is still the count left
  inside at the end. Worked by hand in `blocks/tests/transfers.rs`: a part counts at the first
  machine until its transfer starts, then at the second.
- **Industrial benchmark: Case A of Schmitt et al. (2026)** (issue #87). Paper: "LLM-driven
  discrete-event simulation: A generative AI framework for automated model generation,
  adaptation, and evaluation in manufacturing", *Journal of Manufacturing Systems* 85, 642–661,
  https://doi.org/10.1016/j.jmsy.2026.02.015 (CC BY 4.0), Table 4. It is a truck maker's line of
  four machines with exponential failures and repairs, buffers of 5 with a 10 s transfer,
  weekend breaks and power per machine.
  - **In StochCraft:** `stochcraft-cli new -t case-a`, committed as `tests/models/case-a.bpmn`
    and run by `engine/tests/benchmarks.rs` (25 replications of 30 days, 1 day of warm-up, 0.2 s in
    a release build).
    - Material is always there: a supply buffer kept full feeds the first machine as soon as it
      has room. Its arrivals turned away show as `lost` at the supply, and it is left out of
      the line's work in progress.
    - The behaviour the paper leaves open is set as its published model does it. The failure
      clock runs through breaks, and an interrupted job resumes with the time it had left. A
      buffer's capacity counts parts in transit, and the sending machine is held through the
      transfer. A machine checks the calendar as it takes its next part.
  - **Compared with:** the authors' published model (GPL-3.0), run on its own inputs outside
    the repository; none of its source was used in StochCraft. Its throughput matches the
    paper's. Its WIP samples once a minute and counts a part in transit twice, and its energy
    books each state when the state ends. So the WIP and energy-per-part references are that
    model measured exactly: the same 25 seeds, distinct parts, and power integrated over exact
    state times.

  | Measure | Paper | Published model, exact | StochCraft |
  |---|---|---|---|
  | Throughput (parts/h) | 7.38 | 7.384 [7.335, 7.433] | 7.370 ± 0.056 |
  | Work in progress (parts) | 4.47 | 4.402 [4.322, 4.482] | 4.386 ± 0.093 |
  | Energy per part (kWh) | 14.22 | 14.141 [14.105, 14.177] | 14.150 ± 0.042 |

  Each machine's share of time processing, blocked (transfers included) and failed agrees
  within the half-width of the difference of two means of 25 replications. The test checks
  these shares too. Starved and off shift are compared as one: the published model counts a
  machine idle in a break as starved until it takes a part, while StochCraft counts it off shift.
  For the last machine that is 74.7% starved and 15.9% off shift, against 86.0% and 4.6%. The
  sum is 90.59% against 90.62%, and energy is the same either way.
- **Industrial benchmark: Case B of the same paper** (issue #96), Table 6: an automated
  assembly line of seven machines, Loading, Conveyor, Washing and Handling in series, two
  presses in parallel, then Quality, which rejects 8.9%. Buffers of 2 with 10 s and of 3 with
  32 s; the same weekend breaks; 1 day of warm-up then 7 measured (the paper's "7-day horizon":
  with 7 in total its published model falls 3% short of its throughput).
  - **In StochCraft:** `stochcraft-cli new -t case-b`, committed as `tests/models/case-b.bpmn`
    and run by `engine/tests/benchmarks.rs` (25 replications; ignored in debug builds, run in
    release by CI). Parts go to the press whose buffer has the shorter queue, and each delay is
    a minimum time in the buffer. The test also runs it with transfers that hold the sender.
  - **Compared with:** an independent model of the written specification, outside the
    repository (100 replications), since the paper gives no intervals and its published model
    adds storage Table 6 does not list. With the same rules, StochCraft agrees on every
    measure and on each station's work in progress. The two rules no source fixes were
    settled this way: a broken empty machine takes no part, and a machine under repair hands
    on its finished part (keeping it instead lowers throughput by 0.045 parts/h, in both).

  | Measure | Paper | Independent model | StochCraft |
  |---|---|---|---|
  | Throughput, minimum time in buffer (accepted parts/h) | 27.26 | 27.091 [27.056, 27.127] | 27.150 ± 0.056 |
  | Work in progress (parts) | 18.22 | 18.346 [18.303, 18.389] | 18.362 ± 0.105 |
  | Energy per accepted part (kWh) | 0.4510 | 0.43974 [0.43926, 0.44022] | 0.43904 ± 0.00070 |
  | Throughput, transfer holding the sender | – | 23.073 [23.043, 23.103] | 23.115 ± 0.049 |
  | Work in progress (parts) | – | 17.340 [17.300, 17.380] | 17.364 ± 0.085 |
  | Energy per accepted part (kWh) | – | 0.49192 [0.49136, 0.49249] | 0.49128 ± 0.00087 |

  The paper's own two tools differed by 0.8% in throughput on this line. A delay that holds the
  sending machine costs the presses 32 s of every 175 s job: 15% of throughput, far from the
  paper's figure, so the paper's delays are read as minimum times in the buffer.
- Not yet: **a serial line with finite buffers, task T1.2 of Pernas-Álvarez and Crespo-Pereira
  (arXiv 2611.10602, 2026).** Four machines in series with exponential processing (means 3.3,
  4.3, 3.0 and 3.7 min), raw material always in front of the first, buffers of 3 between them,
  blocking after service; throughput per hour and each machine's utilisation. The paper gives
  no reference values, so the targets are exact: the line's continuous-time Markov chain
  (machine states working, blocked or starved, and the three buffer levels) solved numerically
  here from first principles, as for Case A, with each StochCraft result's 95% interval to
  contain them. Also a template and a committed model, as Cases A and B. The paper's simulator
  (SimuLean) is GPL-3.0: its paper is used, its code never opened.

### M2.7 Assembly and batching

- Batching and assembly: BPMN's own constructs where they fit (a parallel join for assembly,
  multi-instance activities), otherwise `stochcraft:` vendor extensions for batch size and bill
  of materials. Validation: exact deterministic flows; throughput bounded by the
  slowest input.

**Done:**
- **Batches:** `ElementKind::Batch { size }` collects `size` entities and sends them on together
  as one, the first carrying the others. `ElementKind::Unbatch` sends them on one by one again.
  - Both are BPMN tasks with a StochCraft vendor extension: `stochcraft:batchSize`, or
    `stochcraft:unbatch` set to `true`.
  - BPSim has no batching, and a BPMN multi-instance activity runs one activity many times
    rather than grouping entities, so it is not used for this.
- **Assemblies:** a parallel join with a bill of materials. `Connection.quantity` is how many
  entities each incoming connection gives each one sent on; the parts other than the first are
  used up. In files this is `stochcraft:quantity` on the sequence flow's `ElementParameters`.
- **Counts:** a batch counts each entity in and each batch out, and reports the mean number
  waiting for their batch to fill. An assembly counts every part in.
- **Files:** read, written and saved in place, and declared in the vendor schema's notes. The
  reference-corpus edits turn delays into batches and splits and give joins a bill of
  materials, and the files stay schema-valid.
- **Engine and window:**
  - `block.add` and `block.set` take `kind: "batch"` with `size`, and `kind: "unbatch"`.
  - `connect` and `flow.set` take `quantity`; `null` removes it.
  - The window can add both kinds and edit a batch's size.
- **Checks:** a batch of 0 and a quantity of 0 are errors. A quantity on a connection that
  does not go into a parallel gateway, and a split in a model with no batches, are warnings.

**Validation** (`blocks/tests/assembly.rs`, 16 replications; docs/research.md, "Batching and
assembly"):

| Case | Theory | StochCraft |
|---|---|---|
| Poisson arrivals at rate 1 in batches of 4, one machine of mean 3 (E₄/M/1): wait | 4.94292 | 4.85129 ± 0.30316 |
| Same: utilisation | 0.75 | 0.74892 ± 0.00654 |
| Same: mean number waiting for their batch to fill | 1.5 | 1.50074 ± 0.00599 |
| Same, split again: entities out in 20,000 | 20,000 | 20,003.8 ± 54.1 |
| Assembly of 2 A (rate 1) + 1 B (rate 0.4): assemblies in 20,000 | 8,000 | 8,024.6 ± 62.5 |
| Assembly of 2 A (rate 0.6) + 1 B (rate 0.4): assemblies in 20,000 | 6,000 | 6,011.8 ± 29.0 |

Deterministic cases worked by hand check batches of 3 (counts, the mean waiting 1, each
entity's own time in system after the split) and an assembly of 2 + 1 parts (15 in 30, each
after 1).

**Not yet:** a batch that leaves part-full after a timeout, batches sized by a property, a
window form for a connection's quantity, and assemblies whose parts are matched by a property
(such as an order number).

### M2.8 Experiments for decisions

From DES practice at NASA, NNSA and the Aerospace Corporation (docs/research.md):

- **Terminating experiments:** run each replication until an event (a target count, an element
  emptying, a time limit) instead of for a fixed duration after a warm-up; time-to-event
  metrics. Validation: the completion time of a deterministic activity network; the maximum of
  exponential durations against its closed form.
- **Distributions of results, not only means:** percentiles and the empirical cumulative
  distribution across replications, with a confidence band (Dvoretzky–Kiefer–Wolfowitz), and the
  **probability of meeting a target** ("at least 100 batches a week") with its interval.
- **Three-point estimates:** a distribution fitted from minimum, most likely and maximum (or 95th
  percentile) as elicited from experts (triangular, PERT as a scaled beta, lognormal, Weibull),
  written as a standard BPSim distribution; the triad and its source are kept as the parameter's
  pedigree (a `stochcraft:` vendor extension).
- Partly done (#182): **fitting a distribution to measured data**, in the engine
  (`estimate.fit`) and the CLI (`stochcraft-cli fit`), by maximum likelihood, ranked by
  Anderson–Darling with the Kolmogorov–Smirnov distance beside it, with a minimum sample size.
  Not yet: the window's view, bounds for censored or clipped data, and the fit kept as the
  parameter's pedigree. The design: a column of measured times (CSV, or
  pasted) fitted to each candidate family by maximum likelihood, ranked by a goodness-of-fit
  measure (Anderson–Darling, with the Kolmogorov–Smirnov distance beside it), with bounds for
  censored or clipped data and a minimum sample size below which no fit is offered. The best
  fit is written as a standard BPSim distribution, with the data's file, count and fit kept
  as the parameter's pedigree, as for three-point estimates. In the window: the histogram with
  the fitted density over it, and the empirical distribution beside the fitted one; in the
  CLI, a `fit` command; to agents, an engine command. Closes the fitting part of docs/gaps.md
  row 11. Also offered by desflow (MIT, `desflow fit`), seen 2026-11.
- **Sweeps and sensitivity:** a parameter varied over a range, results per point, and a record of
  which parameters drive which results (NASA-STD-7009B [M&S 30]).
- **Capacity sizing:** add a unit at the bottleneck (the busiest resource) and run again, until
  the share of replications meeting the target reaches a target share. Reported with the
  deterministic answer beside it, which is too small.
- **Results as inputs:** a metric's values across replications as an empirical BPSim user
  distribution for a coarser model.
- Acceptance model: a small multi-product plant written for StochCraft (two products sharing
  mixers and ovens, working hours, a weekly target met in 70% of weeks), sized automatically.

**Done (first part):**
- **Terminating runs:** `Experiment.stop` ends each replication on an event, with the warm-up
  plus the duration as its time limit:
  - `Stop::Count`: once N entities have left an element since time 0 (for a sink: arrived).
  - `Stop::Empty`: the first time a task holds nothing after it has taken an entity in.
  - The condition is checked after each instant, so the events and their order are exactly
    those of a run without it; a condition never met changes nothing.
  - Each report gives `stopped_at`, the time to the event. A live run (M3.5) ends there too.
  - In files: `stochcraft:stopCount` or `stochcraft:stopWhenEmpty` on the watched element.
    Engine: `experiment.set` with `stop`, `null` to remove it. Removing the element removes the
    stop, in the same undo step.
  - Results gain `stop`: how many replications met it, the mean time with its interval, and
    each replication's time.
  - A stop can come before the warm-up ends (#67). Such a replication never discards its
    warm-up, so its statistics describe the start-up, not the system: they are left out of
    every metric (its stop time still counts), and the results warn (`ended_in_warmup`).
    Validation warns whenever a stop is combined with a warm-up (`stop_with_warmup`). Results
    gain `measured`: what each replication's statistics actually cover (`from` the warm-up's
    end, or 0, `to` its end, and whether they are `used`) and the mean time measured, which the
    app and the command line show in place of the configured duration.
- **Overload and work left inside (#68):** a new metric, `work_in_progress`, counts the
  entities still inside each server, shared task, delay, batch and join at the end of a
  replication. Validation warns `overloaded` when a server fed straight from steady sources is
  offered at least its capacity (arrival rate × mean service ÷ units ≥ 1). After the run,
  `no_steady_state` flags a server busy at least 98% of its time up with at least 10, and at
  least 5% of its departures, still waiting at the end (`SATURATED_UTILISATION`,
  `BACKLOG_MIN`, `BACKLOG_SHARE` in `blocks`). Tasks with shared resources are not judged.
- **Distributions of results:** results now carry every replication's value of every metric
  (`values`, outside `elements`, so the golden digests are unchanged). `results.distribution`
  asks about one of them, or `stop_time`, from the last run:
  - percentiles (Hyndman and Fan's definition 7);
  - the empirical cumulative distribution with its Dvoretzky–Kiefer–Wolfowitz band (Massart's
    constant), replications that never met the stop counting as censored at the time limit;
  - the probability of meeting a target (`at_least` or `at_most`) with Wilson's interval;
  - `as_distribution`: the values as a continuous BPSim user distribution, ready for
    `block.set` in a coarser model (results as inputs).
- **Three-point estimates:** `random::ThreePoint` fits a minimum, a most likely value and a
  maximum or 95th percentile:
  - triangular: BPSim's own, the maximum solved for when a 95th percentile is given;
  - PERT, the scaled beta: a BPSim user distribution of its percentiles;
  - log-normal and Weibull from the minimum, mode and 95th percentile: BPSim's own when the
    minimum is 0, otherwise a user distribution of percentiles.

  `estimate.set` fits an element's time distribution (a source's arrivals, a task's service, a
  delay) and keeps the triad and its source as the parameter's pedigree,
  `stochcraft:estimate` in files (declared in the vendor schema). A distribution changed
  afterwards no longer matches its estimate, which is a warning. `estimate.remove` forgets it.
- The CLI's results table states the stop and when it was met.
- **Results that are easy to misread (#69):**
  - A metric only some replications have a value for shows "n = k of N" in the window (its
    `n` was already in the results); the CLI's `n` column reads "k of N" then.
  - Notes under the results say that the mean wait and the mean time in system leave out
    entities still inside at the end (when any were), and that the mean time to the stop is
    over the replications that met it (when not all did).
  - Validation warns `normal_often_negative` when a time is drawn from a normal distribution
    whose mean is less than Φ⁻¹(0.99) = 2.326 standard deviations: more than 1% of its draws
    are below 0 and count as 0, which lengthens the mean. It suggests a truncated normal with
    min 0 or a log-normal.
  - Watch a run says that it shows one replication of N, and that the others differ.
  - `clippy.toml` forbids `f64::powi` and `f64::mul_add`, whose results can differ between
    targets; squares are written as products.

**Validation** (`blocks/tests/terminating.rs`, `random/src/estimate.rs`,
`stats/src/spread.rs`, `engine/tests/engine.rs`):

| Case | Theory | StochCraft |
|---|---|---|
| Activity network A(3)→B(4), A→E(5), C(6), D(2) after B and C: completion | 9 (critical path A–B–D) | 9 exactly |
| Completion of 3 parallel exponential activities of mean 10 (2,000 replications) | 10 (1 + 1/2 + 1/3) = 18.333 | 18.142 ± 0.492 |
| Same: largest gap between the empirical and the true distribution (1 − e^(−t/10))³ | within the 95% DKW band, 0.0304 | 0.0130 |
| Same, 200 experiments of 50 replications: the 95% band contains the true distribution | at least 95% | 190 of 200 |
| Same through the engine (4,000 replications): P(T ≤ 30) = (1 − e^−3)³ = 0.858 | inside Wilson's interval | yes |
| 20 orders a minute apart, 2 minutes each: the server empties | 41 | 41 exactly |
| PERT(10, 20, 60): mean (min + 4 mode + max) / 6 | 25 | 25.04 (the 1% table) |
| Log-normal and Weibull fits: mode and 95th percentile | as given | to 10⁻⁹ |
| Wilson's interval for 8 of 10 at 95% | [0.4902, 0.9433] | same |

**Done (second part):**
- **Sweeps:** `experiment.sweep` runs the experiment at each value of one parameter, using the
  same seed so the random streams are shared (common random numbers). The parameter is a
  task's `capacity`, a pool's `quantity`, a `queue_capacity`, or the `mean` of a time
  distribution (`Dist::scaled` keeps its shape). It reports the results asked for at each
  value, and how much each moved across the range, with whether that is more than the noise
  (a paired-t interval on the replications' differences, issue #66).
- **Comparing two runs:** `results.compare` gives, per element and metric, both runs' means
  and the difference with its 95% interval: paired-t when the runs share their seed and
  replications, Welch's two-sample t otherwise (issue #66).
- **Common random numbers and the model's shape:** each element's streams are keyed by its id
  and their purpose (#152, M2.9), so a sweep's points share their streams, and so does a variant
  that adds, removes or reorders an element, for every element it leaves in place (#70).
- **Sensitivity:** `experiment.sensitivity` moves each parameter down and up by a fraction,
  one at a time, and ranks the results' elasticities. Every sweep and sensitivity analysis is
  recorded, with the model's digest, in the `sensitivity` list of later runs' records
  (NASA-STD-7009B [M&S 30]).
- **Capacity sizing:** `experiment.size` takes a target (a metric `at_least` or `at_most` a
  value) and a `target_share`. It runs the experiment and adds a unit to the busiest task or
  pool, then runs again, until the share of replications meeting the target reaches the target
  share. Beside it is the answer with every time at its mean, from one replication. `apply`
  makes the answer the model's, as one step to undo.
  - The target share was called `confidence` until #69, which it is not: 15 of 20 weeks (75%)
    has a Wilson 95% interval of 0.53 to 0.89, so a 70% share is not shown to be met. The old
    name is still read, and answers carry both keys. The window shows the interval of the share
    found and says when it reaches below the target share.
- **Acceptance model** (`tests/models/plant.json`, an engine script for `stochcraft-cli exec`):
  - A bakery with bread and cake orders that arrive on weekdays from 8 to 4 (70 and 50 a week).
  - Shared mixers and ovens that work those hours.
  - Mixing and baking times from three-point estimates, PERT and triangular, kept as their
    pedigree.
  - The target: at least 110 orders packed a week, in 70% of weeks. Sized from one mixer and
    one oven:


| Mixers, ovens | Weeks meeting the target (20) | Wilson 95% interval |
|---|---|---|
| 2, 5 | 15% | 0.05–0.36 |
| 3, 5 (the answer from mean times) | 50% | 0.30–0.70 |
| 3, 6 | 65% | 0.43–0.82 |
| 3, 7 (StochCraft's answer) | 75% | 0.53–0.89 |

With every time at its mean, 3 mixers and 5 ovens look enough. With the variability they meet
the target in only half the weeks, and 7 ovens are needed: the deterministic answer is too small,
as the NNSA study found. (Seed 7, 20 replications, with each block's random stream keyed by its
id since #152; before that the same search stopped at 6 ovens.)

**Validation** (`engine/tests/engine.rs`):

| Case | Theory | StochCraft |
|---|---|---|
| Sweep of the help desk's servers (λ = 0.5, mean service 1.5): wait, 1 server | 4.5 | 4.406 ± 0.180 |
| Same, 2 servers (Erlang C) | 0.2455 | 0.2410 ± 0.0064 |
| Same, 3 servers | 0.0294 | 0.0288 ± 0.0013 |
| Elasticity of M/M/1 time in system to the service time (±10%) | 4.40 | 4.32 |
| Same, to the time between arrivals | −3.57 | −3.42 |

**In the window** (`ui-egui/src/decide.rs`):
- **Experiment:** the model overview's "Ends early" sets the stop condition: when enough
  entities have left a block, or when a task empties.
- **Three-point estimate:** a timed block's inspector fits its time to a three-point estimate,
  with the family, the upper value (a maximum or a 95th percentile) and the source. Once
  fitted, the inspector shows the estimate, which can be changed or forgotten.
- **Results:**
  - A line says when the stop condition was met.
  - "Spread across replications" shows any one result's percentiles and its empirical
    distribution, drawn with its confidence band, and the time to the stop.
  - With a target, it gives how many replications met it, as a share with its interval.
- **Analysis**, a third view beside Model and Results:
  - a sweep of one parameter, with the result at each value and whether it moved more than
    chance;
  - every parameter ranked by elasticity, with which ones matter;
  - sizing to a target met in a chosen share of replications, beside the answer from mean
    times, with a button that applies it as one undo step.

  An answer is shown only while the model is the one it was found for. Analyses run at once on
  the window's thread.
- **Engine:**
  - A window's background run now keeps its results for `results.distribution`.
  - Sensitivity no longer takes a unit count below one: a parameter already at its floor is
    compared from its own value up.
- **Tests:** `ui-egui/tests/decide.rs`; the strings in all 44 catalogs.

**Done (third part): designed experiments with several factors at once.**
`experiment.design` varies several parameters together, as Python users do with
`itertools.product` or a grid drawn as a heatmap. Each factor is a parameter as a sweep takes it
(`{id, field}`), with its levels or its range.
- **Full factorial** (the default): every combination of the levels, the last factor changing
  fastest, as `itertools.product` does. Each factor's **main effect** is the mean of a result at
  its highest level minus at its lowest, over the other factors' levels. For factors with two
  levels, each pair's **two-way interaction** is Law's e12 for a 2^k design (*Simulation
  Modeling and Analysis*, 5th ed., chapter 12): half the change in one factor's effect between
  the other's two levels.
- **Latin hypercube** (McKay, Beckman and Conover, *Technometrics* 21(2), 1979): `points`
  points, each factor's range cut into that many strata, each used once. Each factor draws its
  permutation and offsets from its own stream, `Rng::keyed(seed, "design/<id>.<field>")`, so the
  same seed gives the same design, and adding a factor leaves the other columns alone. A count
  given as a range takes its whole numbers. Each factor's rank (Spearman) correlation with each
  result is reported.
- Every point runs the same replications on the same seed (common random numbers, #66), so each
  effect is worked out replication by replication and judged by its paired 95% t interval, with
  `tests` and the Bonferroni note as in a sensitivity analysis. Points run one after another,
  each with its replications in parallel, as in a sweep.
- A design of more than 10,000 runs (points × replications, `design::MAX_RUNS`) is refused,
  with the number it would take. Designs are recorded in later runs' records, as sweeps are.
- `stochcraft-cli design MODEL --factor id.field=1,2,3 --factor id.field=min..max --result
  element.metric`: a table of the points, then the main effects and interactions (or the
  correlations); `--format csv` gives one row per point and result.

**Validation** (`engine/tests/engine.rs`, `stochcraft-cli/tests/cli.rs`):

| Case | Theory | StochCraft |
|---|---|---|
| Help desk 2 × 2: time between arrivals a ∈ {2, 3}, mean service s ∈ {0.8, 1.2}; M/M/1 wait in queue s² / (a − s) at the four points | 0.533, 1.8, 0.291, 0.8 | 0.531 ± 0.010, 1.794 ± 0.052, 0.286 ± 0.009, 0.789 ± 0.021 |
| Same: main effects of a and s | −0.621, +0.888 | −0.625 ± 0.024, +0.882 ± 0.026 |
| Same: their interaction | −0.379 | −0.380 ± 0.019 |
| A second server's service time on the first server's wait | 0 | 0, not significant |
| M/M/1 of issue #66, 2 × 2 of small changes: half-width of a main effect, paired against independent points | narrower | 0.068 and 0.062 against 0.21 |
| Latin hypercube, 8 points: values per eighth of each range | one each | one each; same seed, same design |

**Not yet:** sizing several resources a step at a time, or by cost; saving targets in the model
file; running analyses in the background or on the browser's web workers. For designed
experiments: the window's form and a heatmap of a two-factor grid; fitting a response surface
(a regression on the factors); and fractional factorials (2^(k−p)) for many two-level factors.

### M2.9 Output analysis

Help with the choices that decide whether results mean anything: how long a warm-up, how many
replications, and whether variants are compared on the same random numbers (#70, now split into
#224, #225 and #226). The book
treats these as standard practice (Robinson, ch. 2; Morecroft and Robinson, ch. 9 §9.5; Elder,
ch. 10 §10.4, in Brailsford, Churilov and Dangerfield, eds., 2014). Related and already done:
`overloaded` and `no_steady_state` (M2.8, #68, #75) and `ended_in_warmup` (#67).

- **Choosing a warm-up.** The default warm-up is 0. Done in the engine and the CLI; the window's
  Welch plot is not yet (M3.7 step 3 draws its charts over the run from the same series).
  - **Done:** a recorder of values at fixed times in every replication (`blocks/src/series.rs`,
    `Compiled::record_every`): each element's queue length, number inside, busy units, and
    counts entered and left since time 0, and each pool's busy units, at time 0 and every
    interval to the end, read between events so that no event, draw or result changes. The
    interval is the scenario's `baseResultFrequency` (an `xs:duration`, now read, inherited and
    written), taken in whole multiples beyond 1,000 points, or else a hundredth of the warm-up
    plus the duration. `run` and `run.start` record unless given `"series": false` (it costs
    about 4% of run time on case A, 0.1% on mm1.bpmn).
  - **Done:** across replications, each quantity's mean, its 95% Student-t half-width and the
    2.5th and 97.5th percentiles at each time, in the results as `series` (the data for Welch's
    plot; the MCP `run` tool leaves it out for its size).
  - **Done:** MSER-5 (White 1997; White, Cobb and Spratt 2000; `stats/src/warmup.rs`) on each
    task's mean number inside suggests a warm-up, in the results as `warmup_suggestion` and in
    the CLI's text; with a hundredth of the run between points, it places it to within 5%.
  - **Done:** the `no_warmup` warning, for a run with no stop condition and no warm-up where
    MSER-5 places a start-up and the run starts lower than its second half by more than chance
    (a window fixed in advance, since MSER-5's own cut finds a difference in noise about a third
    of the time). Measured on M/M/1 started empty: 3 to 9% false alarms at loads of 0.3 to 0.7;
    at 0.9 over 2,000 time units it warns in 9, 50 and 94% of runs with 3, 10 and 30
    replications. A start-up too small to tell from the noise of few replications is not warned
    about; the suggestion is there whenever the series has at least 20 points (a 50-unit run
    recorded every 5 units has 11, and gets none).
  - Tests: `blocks/tests/series.rs` (a D/D/1 trace; the values a live run shows at each time;
    no result changes, in slices too; M/M/1's mean queue ρ²/(1 − ρ) at late times; the
    start-up at ρ = 0.9 and none at 0.3; the warnings), `engine/tests/series.rs` (the JSON, the
    file's frequency, threads, slices and workers alike).
- **Choosing the number of replications.** The default is 10.
  - **Done:** the `one_replication` warning for a run with one replication.
  - **Done (#224):** sequential replications. A run adds replications in batches until every
    key measure that varies has a 95% half-width within its target (a share of its mean, or
    percentage points for a share), or a maximum is reached, after Hoad, Robinson and Davies
    (2010). Zero spread never settles a result; a mean at or near zero never meets a share of
    itself; the run says why it stopped, and calls its intervals estimated precision. In the
    engine, the file (`stochcraft:sequential`), the command line (`--until`, `--max`, `--fixed`)
    and the window (**How many runs** in the model overview). Sweeps, designs, sensitivity and
    sizing still run a fixed number.
- **Streams by element id:** done (#152). Each block's streams are keyed by its id and their
  purpose, derived from the seed and the replication, not its place in the model, so that a
  variant that adds, removes or reorders an element keeps common random numbers for the others,
  and an unrelated edit no longer changes results with the same seed. The golden digests changed
  once, with the reason in the commit.
- **Validation against observed data** (#226). Compare a run's results with held-out real data
  (black-box validation, ch. 6 §6.5.5 and ch. 10 §10.4.1).
  - Not yet: **checks that pass or fail.** An experiment can carry checks on its results: a
    bound ("mean wait under 2 min", "throughput at least 400 a day", with the 95% interval
    deciding, not the mean alone), a comparison of two scenarios with common random numbers
    ("the third packer cuts the wait"), or observed values with their spread. A run reports
    each check as met, not met, or not decided (the interval straddles the bound), and
    `stochcraft-cli check` exits non-zero when a check is not met, so a model can be tested in
    CI. The checks are saved with the experiment (a `stochcraft:` vendor extension) and can
    write a short report (Markdown) of the checks, the settings and the results. Also offered
    by desflow (MIT, `desflow experiment`), seen 2026-11.
  - **Done:** **behaviour checks after every run** (`blocks/src/checks.rs`). Checks that need
    no reference, made on each replication's results and reported as warnings, one per element
    and check, saying in how many replications it failed (in the run's `warnings`, so the CLI's
    table, the window's results and agents get them): every element's entities add up, with
    those inside as the statistics started (`unbalanced`), and the model's (created or inside
    at the start = reached an exit + left elsewhere + still inside: `unaccounted`); some
    entity leaves the model, at an exit or by any other way out (`nothing_out`);
    Little's law holds at every server, shared task and delay (`littles_law`: the time-average
    inside against the entities' own times inside, from when each came in and went out, those
    inside at the window's start and end counted to it, so it is exact over any window and the
    tolerance is rounding only); a server counts busy exactly the units processing or setting
    up, none idle, blocked, off shift on a break, failed or restarting (`busy_not_working`), and its units' shares of time in each state
    add up to 1 (`states_total`); utilisation and the other shares of time stay within 0–100%
    (`out_of_range`). Pernas-Álvarez and Crespo-Pereira (arXiv 2611.10602, 2026) found that
    agents' wrong models were all "silent" (they built and ran, and answered wrongly); checks
    like these caught about half, the most common being a break counted as busy time.
  - Every entity is counted in one place, as in the common tools: an entity that gives up
    during its job (patience until done) is counted as abandoned then, though its unit carries
    on with the job; one waiting for room on a way marked for those giving up, balking or turned
    away is counted as leaving the element it left; copies, batches and unbatched entities are
    new entities, and those combined into a batch ended. So both counts are exact, where before
    they allowed ± a server's units or were not checked. Still not checked: the model's count
    where a parallel gateway joins, or a shortest-queue gateway sends to another; each
    element's count at parallel gateways, unbatches and shortest-queue gateways; Little's law
    at batches and joins.
  - Any way out of the model is a way out for `nothing_out`, as reaching any sink is in the
    common tools; a gateway where no condition holds and there is no default is warned about
    as the model is checked (`no_default`; BPMN 2.0.2 §13.4.2 makes it an exception).
  - **Done (#235):** closed loops. A model whose entities go round for ever, a fixed number of
    them (a closed queueing network, Gordon and Newell 1967, or a finite-source model such as
    the machine repair crew, M/M/c//N), needs no exit and none of its entities leaves. It is
    closed (`Model::is_closed`) when entities start somewhere (a source or starting contents),
    every source makes a limited number, there is no exit, no block copies, combines or splits
    entities (parallel gateway, batch, unbatch), and from every block where entities enter,
    every block they can reach leads back to it. A closed model is not warned for having no
    exit (`no_sink`); a model with no exit that is not closed (a source with no limit, a block
    passed once, a loop that drains into another and never returns) still is. In a closed
    model `nothing_out` gives way to `nothing_moved`: the entities are stuck when some block
    holds entities at the end and none of those blocks finished any in the measured window.
    Only the blocks holding entities count, since each entity's first pass through the blocks
    before a jam would otherwise hide it. Entities that give up, balk or find no way at a
    gateway leave without an exit, so a loop with such ways is closed only as far as its
    blocks and connections go; models that are partly open and partly closed are checked as
    open ones.
  - Experiment designs (`experiment.sweep`, `.sensitivity`, `.design`, `.size`) check every
    run they make as a single run is checked, and give each point the warnings its run gives
    (`warnings`): each run of an experiment is an ordinary run. They record no results over
    time, so the warm-up suggestion (`no_warmup`) is left to a run of the model itself. The
    window does not show them yet.
  - Utilisation counts a unit failed or restarting part-way through a job as not busy: that
    time is capacity lost, still in the units' time (Law, *Simulation Modeling and Analysis*,
    5th ed., §14.4.2; ISO 22400-2's busy time), and planned time off shift is left out of both.
    Most tools do the same by default; some count it busy, or let the modeller choose. Until
    2026-11 StochCraft counted it busy, which put a failing machine's utilisation above its
    processing share (on the weld and paint test model, paint: 73.2% against 65.2%).
- **A capacity table.** Before a run, each station's offered load and utilisation worked out
  from the model's own numbers (arrival rates, mean service times, units), shown beside the
  simulated values after the run. The gap between the two is what variability and blocking
  cost, which is what the simulation adds. It extends the `overloaded` check, which today is
  part of the model's validation (`Model::check_load` in `model/src/lib.rs`) and covers only
  servers with an unlimited queue fed straight from sources with steady arrivals; the table
  needs the load passed on through routing, joins and shared pools too.
  - For each station: arrival rate, service rate, units and utilisation, marked as fine,
    congested (above a target, 85% by default) or unstable (at 100% or more), the bottleneck
    named, and the units that would bring it under the target. Where a queueing formula holds
    (M/M/c, M/G/1, Erlang A with patience), its mean wait too; where none does, why not.
  - Available without running: in the window beside the model, as `stochcraft-cli explain`,
    and as an engine command for agents. A verify option runs replications and says, for each
    number, whether the formula falls inside the simulated 95% interval.
  - Also offered by desflow (MIT, `desflow explain`), seen 2026-11.
  - Done: the table itself (`Model::capacity` in `model/src/capacity.rs`), as
    `stochcraft-cli explain` and the engine command `model.capacity`.
    - Rates pass through the routing by the traffic equations: shares by probability, copies
      at a parallel gateway, the least of a join's inputs, a batch's size, and rework loops.
      Where a rate depends on the run, the table says on what and does not guess: a condition,
      the shortest queue, or those giving up, balking or turned away. Rates after entities are
      lost, or after a station that may be at 100%, are given as at most: one sent more than
      it certainly carries (its availability or service time not known exactly, units on a
      calendar, a task with alternatives, a pool whose load is not bounded above).
    - Utilisation is counted as the results count it, failed time not busy. The load that
      marks a station is taken over the units' availability, mean up over mean up plus repair.
    - A formula is given only where it is exact. M/M/c (Erlang C) holds through rework loops
      when every station feeding it is exponential with an unlimited queue (a Jackson
      network). M/M/c/K, Erlang A and balking (as a birth–death queue) and M/G/1
      (Pollaczek–Khinchine) need Poisson arrivals. A station whose units something further on
      can hold up gets none, nor does one that holds up the station sending to it. That
      something is a full room, or a transfer that keeps the sending unit. Every other
      station gets the reason. Each number is defined as the result of the same name is: the
      mean wait is that of the entities served, as in Garnett, Mandelbaum and Reiman's M/M/N+M.
      Every family is checked against the simulator's 95% intervals
      (`blocks/tests/capacity_theory.rs`).
    - The bottleneck is named only when its load is surely the highest. A load that is not
      known, or only bounded, that could be higher leaves it unnamed.
    - Still to do: showing the table in the window beside the model and the simulated values
      beside it after a run; the verify option; Cobham's formula for a pool shared by
      priorities; and time averages over calendars, where today the most units on shift are
      used.
    - The `overloaded` warning comes from the table (settled 2026-11-18). A server or pool is
      warned about when it is surely at 100% or more, however its arrivals reach it: through
      routing, delays, joins or shared pools. Arrivals from a source with a limit or a
      calendar are at most a rate, and give no warning.

- **Starting contents (#225).** **Done:** a run can start with entities already inside tasks,
  delays and batches, the other answer to a warm-up: a count per block (`Model::starting`, the
  `stochcraft:startingContents` extension in a scenario's element parameters, inherited like
  any other; `block.set`'s `starting`; **Inside at the start** under the inspector's More
  settings). At time 0 they join the block's queue in order, as arrivals would (its properties
  assigned, a place taken in a finite waiting room): the model check refuses more than an
  arrival at time 0 would find room for, the places of a fixed room, or for a limit by
  timetable or per working unit the units on shift at time 0 plus that limit at time 0. Then
  they start:
  as many as there are free units start a fresh service drawn from the block's own stream, and
  the rest wait. Tools that offer a start that is not empty do it the same way: entities put
  into a queue or a block at time 0 by start-up logic, or queues filled before the run. They
  count as entered, and as `started` (only where some were placed); the behaviour checks count
  them as coming into the model there, and with a warm-up those still inside at its end are
  inside at the start like any others. A block with starting contents is where entities come
  from, so a model needs no source.
  - Not yet: in-service entities start a fresh service; elapsed or remaining service times, for
    starting from an observed state, come later with digital twins (After M3). Entities placed
    at the start carry only the properties their block assigns, not those a source would have
    given them. The `no_warmup` warning still speaks of a start from empty.

### M2.10 Abandonment

Entities that give up waiting: a common feature of service models, and of IT systems whose
clients time out and retry. Garnett, Mandelbaum and Reiman (2002) call impatient customers
leaving before their service begins a central feature that Erlang B and C lack.

- **Reneging:** done. A task with a resource of its own can have a patience
  (`stochcraft:patience`): a distribution, drawn for each entity as it enters, with its clock
  starting then (before any transfer in). Until its service *starts* (a caller who hangs up
  while on hold), or until its service is *done* (a client that times out). The deadline is
  checked wherever the entity is: waiting, it leaves the queue (freeing its place in a finite
  buffer); in transfer, it gives up as it arrives; in service or held blocked after it (patience
  until done), it gives up at the deadline and a stand-in (a new entity with its properties,
  creation time and last server) takes the abandoned way at once, while the unit finishes the
  job anyway and discards it. A unit taking an entity at its deadline serves it (the `des` rule
  for deadlines; `Ctx::hold_late` runs the timer after everything else at that instant). A
  patience of 0 gives up unless a unit is free at once. Results per server: `abandoned` (counted
  once each, not as left, so entered = left + lost + abandoned + inside), `abandoned_share`
  (abandoned over abandoned plus left), `abandoned_wait`, and with patience until done `wasted`
  (the service time spent on jobs given up) and `wasted_share`. A server's units serve one
  entity at a time either way (FIFO), and failures, calendars and setups work with it.
- **Where abandoned entities go:** done. They leave the model, or take the server's way out
  marked `stochcraft:abandoned` (one at most; a server may then have two ways out). Retries need
  no element of their own: the abandoned way leads to a delay (the back-off) and back to the
  task, or to a decision that retries with a probability and otherwise gives up for good; a
  reconnection that needs a handshake goes through a task with a setup first.
- **Files:** done. Patience and the abandoned way are `stochcraft:` vendor extensions
  (`stochcraft:patience until="start|done"` with a distribution inside, on the task's
  `ElementParameters`; `stochcraft:abandoned` on a sequence flow), read, written, saved in
  place, set by `block.set` and `flow.set`, and edited in the inspector. BPSim 2.0 has no
  parameter for them: its `queueTime` and `waitTime` are durations the modeller sets, parts of
  `lagTime` (6.2.4), and an `interTriggerTimer` on a boundary event is the interval between
  repeated occurrences of that event (6.2.5), not a patience drawn for each entity.
- **Checks with known answers:** done (`blocks/tests/abandonment.rs`). M/M/c+M (Poisson
  arrivals, exponential service and patience, unlimited room; Garnett, Mandelbaum and Reiman,
  M&SOM 4(3):208–227, 2002, as the model's reference) against its birth–death chain, solved in
  the test from first principles (death rate min(n, c)·μ + max(n − c, 0)·θ; P(abandon) =
  θ·E[Nq]/λ): mean queue, share abandoned and utilisation for three cases, one overloaded (ρ =
  1.5, which abandonment keeps stable), and once more through the unit server; patience 0 against
  Erlang B; the tie at the deadline; patience until done in service and while blocked; a patience
  running out in transfer; retry loops whose counts balance on every element.
- **Balking:** done. A task with a resource of its own can have balking
  (`stochcraft:balking`): an arriving entity joins with a chance b(n) of the number n it finds,
  else it balks: counted as balked (it never enters, so entered = left + lost + abandoned +
  inside still holds) and takes the server's way out marked `stochcraft:balked` at once (one at
  most, never also the abandoned or turned-away way), or leaves the model. b(n) is a table (rows
  of `waiting` and `probability`: from each row's number on, its chance; below the first row, 1)
  or 1/(n + 1) (`reciprocal`). n counts those waiting (in the queue or in transfer, as a waiting
  room counts them), or with `counts_in_service` everyone inside, as Gross and Harris do. The
  choice is made as the entity arrives, from any element, before it asks for room in a finite
  waiting room (one that joins and finds none is turned away, or waits, as before); an arrival
  from a source that balks has left its source. A chance strictly between 0 and 1 is drawn from
  the server's own keyed stream (`balking/<id>`), so models without balking draw exactly as
  before (golden digests unchanged). Results per server: `balked` and `balked_share` (balked
  over balked plus lost plus entered). File: `stochcraft:balking` with `stochcraft:join` rows,
  `rule="reciprocal"` and `countsInService="true"`; `stochcraft:balked` on a sequence flow; read,
  written, saved in place (removal gives back the original bytes), in the vendor schema; set by
  `block.set` (`balking`, `{type: table, join: [...]}` or `{type: reciprocal}`) and `flow.set`
  (`balked`), and in the inspector's Balking section, drawn from the command schema, with a
  Balked tick on the flow. Validation: `balking_probability`, `balking_rows_order`,
  `balked_without_balking`, `several_balked_ways`, `balked_and_other_way`.
  - **Not joining at or above a length L** needs no balking: it is a waiting room of L
    (`queue_capacity`) with a way out marked turned away (they count as lost, not balked).
    Without that way only arrivals from a source are lost; anything else waits for room. A
    table of 1 below L and 0 from L gives the same results run for run, balked equal to lost
    (checked exactly).
  - **Checked** (`blocks/tests/balking.rs`) against birth–death chains solved in the test (rate
    λ·b(n) up, min(k, c)·μ down; the share balked 1 − Σ p(k)·b(n(k)) by PASTA), 64 replications
    of 20,000 after a warm-up of 1,000. Gross and Harris's M/M/1 with b(n) = 1/(n + 1), n in the
    system: p(k) = e^(−ρ) ρ^k / k!, so L = ρ, the share balked is 1 − (1 − e^(−ρ))/ρ and the
    throughput μ(1 − e^(−ρ)) (the chain agrees with these to 1e-9). With n those waiting the
    same rule gives p(k) = p(0) ρ^k / (k − 1)! for k ≥ 1, p(0) = 1/(1 + ρe^ρ). Queue length,
    utilisation and every element's balance are checked too; a table always joining gives the
    same results to the last bit as none, and balking at one server leaves another's draws
    alone.

  | Case | Quantity | Theory | StochCraft (95%) |
  |---|---|---|---|
  | M/M/1, b = 1/(n + 1), n in the system, λ 0.8, μ 1 | L; share balked; throughput | 0.8; 0.31166; 0.55067 | 0.7993 ± 0.0025; 0.31091 ± 0.00085; 0.55048 ± 0.00095 |
  | Same, λ 2 (stable only by balking) | L; share balked; throughput | 2; 0.56767; 0.86466 | 2.0027 ± 0.0064; 0.56774 ± 0.00067; 0.86433 ± 0.00131 |
  | M/M/1, b = 1/(n + 1), n waiting, λ 1.5, μ 1 | L; share balked; throughput | 2.17627; 0.41966; 0.87051 | 2.1792 ± 0.0060; 0.41973 ± 0.00100; 0.86990 ± 0.00138 |
  | Two units, λ 2.4, μ 1, b 1 below 2 waiting, 0.5 from 2, 0.2 from 5 | L; share balked; throughput | 3.42061; 0.26294; 1.76895 | 3.4266 ± 0.0069; 0.26318 ± 0.00083; 1.76789 ± 0.00191 |
  | b 1 below 3 waiting, 0 from 3, λ 0.9, μ 1: M/M/1/4 | L; share balked (= blocking); throughput | 1.79029; 0.16022; 0.75581 | 1.7888 ± 0.0054; 0.15954 ± 0.00109; 0.75526 ± 0.00107 |

- **Not yet:** balking through a shortest-queue gateway (an entity routed by one to a server
  with balking always joins: the gateway picks a way with room and the entity is sent there);
  balking at tasks with shared resources; a chance of joining that depends on anything but the
  number found (the wait expected, the entity's properties); patience on tasks with shared
  resources; an entity that gives up while held blocked before a shortest-queue gateway still
  counts as passing through that gateway.
- **Servers taken out by a failed health check:** done. A load balancer's health check, and the
  metastable failure it can cause (servers taken out under load leave fewer to serve the queue,
  which grows, so more fail; the collapse can outlast its cause). A task with a resource of its
  own can have a health check (`stochcraft:healthCheck`): each unit is checked every `interval`
  (its own checks), and fails when more entities wait than `limit` per working unit (up, not
  out, on shift): the timeout of a probe that waits in the same queue, times one unit's service
  rate. A unit that fails is out for `restart`, then comes back; one off shift, failed or out is
  not checked, and `keep` units at least stay in service. The job under way on a unit taken out
  goes back to the head of the queue at once (`retry`, the default: a dropped connection; a
  fresh patience if the task has one, and its work so far counts as wasted), or waits on the
  unit and resumes after the restart (`resume`). Failures go on in elapsed time while a unit is
  out. The mean wait runs from arrival to the start of the attempt that finished. Results per
  server: `taken_out`, `mean_in_service`, and `time_restarting` in where the time goes (a state
  of its own, after failed); a unit out restarting draws its idle power. File format: the
  attributes `limit` (`INF` for none), `keep` and `onTakeout="retry|resume"`, with
  `stochcraft:interval` and `stochcraft:restart` inside, each a BPSim distribution; set by
  `block.set` (`health_check`) and edited in the inspector's Health check section. BPSim 2.0 has
  nothing for it. Checked (`blocks/tests/health_check.rs`): a limit never reached changes nothing
  (the checks draw from a stream of their own); retrying with exponential checks, restarts and
  service against the chain of (number inside, units out), solved in the test (mean queue,
  takeouts, units in service, utilisation, time restarting); collapse near full load and none at
  half; keep at least; the retried job first in line, with a fresh patience; a resumed job
  finishing with what was left.
- **A health check with a finite buffer:** done. An entity holds its place in the buffer
  throughout: a retried job keeps it back in the queue, a resumed one keeps it waiting on its
  unit. `placeWhileOut` says where the unit taken out stands while it restarts: `reserved` (the
  default) keeps a place for it, as for a failed unit (once the job under way, if any, is back
  in the queue, it takes a place free at that moment, if one is, and gives it back when the
  restart ends), so the buffer holds fewer while units are out; `open` keeps none, as for a unit
  off shift. A unit out holding a resumed job keeps no place of its own (the job's place is the
  unit's); one holding a finished entity it cannot hand on yet keeps that entity's place once
  it leaves, as a failed unit does. With an unlimited buffer the option changes nothing. On the
  wire `place_while_out` (`reserved`, `open`); in the inspector, asked only with a finite buffer.
  Checked (`blocks/tests/health_check.rs`): both rules against the exact chain of (number
  inside, units working, places kept), with three units, retrying, two cases (two waiting
  places, limit 0.5; six, limit 1, keep 1): mean queue, units in service, takeouts, loss
  probability, arrivals lost and utilisation, each inside its 95% interval (the heavy tests); a
  short run with constant times where only the kept place turns an arrival away; a resumed job
  that keeps no extra place; the two rules identical to the last bit with an unlimited buffer.
- **Not yet (health checks):** several failed checks in a row before a takeout, and a slow start after a restart; the case 5
  demonstration (a short loss of servers tipping a healthy server into a collapse that persists)
  as an example model; an exact check of the resume mode (only sanity checks so far).
- **A task's own units by shift, and the job under way at a shift's end:** done. Case 5 of the
  outage study loses 6 of 10 servers for a while, not all of them. A task's own resource can
  have units by shift, as a pool's: BPSim `Quantity` values `validFor` a calendar (`shifts`;
  the default value outside every shift, 0 when there are only shift values), the units on
  shift being the shift's while the resource is available, else none. What happens to a job
  under way on a unit that goes off shift, fewer on shift or the availability ending, is the
  task's choice (`on_shift_end`, `stochcraft:onShiftEnd`):
  - `finish` (the default, and the rule so far and pools' rule): the unit finishes the job, then
    goes off, counting as on shift until then;
  - `retry`: the job stops at once and the entity goes back to the head of the queue for any
    unit, its work wasted, with a fresh patience (the health check's retry);
  - `resume`: the job stops at once; the entity waits on its unit and carries on with what was
    left once that unit is on shift (and up, and back) again.
  - **Which units go off** when the count drops: idle units up and in service first, the
    highest numbered first; then units failed or out restarting; then busy units, those holding
    a finished entity first (it is handed on first), then the job started last first (the least
    work lost). When it rises, units still finishing a job before going off stay on first; then
    units come back, those holding a paused job first, then the lowest numbered (a failed or out
    unit comes back still failed or out until its repair or restart ends).
  - **Health checks** count the units working as on shift, up and not out, and check no unit off
    shift (one finishing its job after its shift included: it takes no more entities). A unit
    taken out keeps its restart whatever the shift does.
  - **Results:** **utilisation is now over the units on shift** for every server whose units
    go off shift (shifts, or an availability calendar, as for a pool): time-averaged units
    with a job over time-averaged units on shift (those finishing a job after their shift
    included; a job paused off shift is neither working nor on shift). Before, a server with an
    availability calendar reported busy over all its units through the breaks. Servers with no
    calendar report as before. A server with shifts reports its cycle time over the mean units
    on shift (an availability calendar's still over all its units). The unit states are shares
    of all its units (the most on shift at once), off shift being the units the timetable has
    off, a paused job's unit included. Retrying at a shift's end counts in the work wasted.
  - **File:** the resource's `Quantity` with values `validFor` calendars, as a pool's, and
    `stochcraft:pool` with the value `false` on it, so it reads back as the task's own: without
    that marker a resource whose quantity varies by calendar is still read as a pool, so files
    from elsewhere read as before. `stochcraft:onShiftEnd` (value `finish`, `retry` or
    `resume`) on the task, written only when not `finish`. Set by `block.set` (`shifts`,
    `on_shift_end`) and edited in the inspector (units by calendar next to the units, and the
    choice for a job under way once the server can go off shift). Servers without shift
    values, or with `finish`, run exactly as before.
  - **Checked** (`blocks/tests/server_shifts.rs`):

  | Case | Theory | StochCraft |
  |---|---|---|
  | 3 units, jobs of 6 arriving every minute, 1 on shift from 10.5 to 20.5, finish: jobs by 27 (units 2 and 1, started last, go off) | 10 | 10 |
  | Same: utilisation, off-shift share | 63/69, 12/81 | exact |
  | Same, retry: jobs by 27, work wasted, utilisation | 8, 4, 55/61 | exact |
  | Same, resume: jobs by 27, utilisation, off-shift share | 8, 55/61, 20/81 | exact |
  | One job of 30 on 3 units, 1 on shift from 10.5: the idle units go off, nothing wasted | 0 | 0 |
  | Units only in a shift (2 from 10 to 20), jobs of 4: jobs by 30, off-shift share | 6, 36/60 | exact |
  | Health check (limit 0, keep 2), 4 units, 2 on shift from 10.5 to 40.5, checks at 15, 30, 45: taken out; mean in service | 2; 122/46 | exact |
  | One unit, off shift 10 to 20 by availability, jobs of 1.5: jobs by 29.4 finish / retry (1 wasted) / resume | 13 / 12 / 12 | exact |
  | One unit, jobs of 4, failure 7 after each repair, repair 5, off 10 to 20: jobs by 30 finish / retry (3 wasted) / resume; off-shift share | 3 / 2 / 3; 6/30, 7/30, 7/30 | exact |
  | M/M/c, λ 7, μ 1, c 10 then 4 from 60 to 80, then 10, measured 60 to 90, retry: mean queue, mean held, jobs out | transient chain (36.75, 5.98, 179.3) | within 95% |
  | Same, resume (the paused jobs set apart) | transient chain (34.91, 8.04, 179.1) | within 95% |

- **Not yet (units by shift):** a resource's units by shift on a task with a finite buffer are
  simulated (each unit keeps its place), but not checked against theory; editing a pool's
  shifts and a server's are separate (a pool used by one task stays a pool); a file from
  elsewhere whose one-task resource varies by calendar is still read as a pool.
- **Waiting rooms by timetable or per working server:** done. A server's finite waiting room
  (`queueCapacity`) is fixed as before, or set by timetable (the most waiting while each
  calendar is active, the value outside them) or per working server (the value times the units
  up, not taken out and on shift now, so it follows failures, health checks, shifts and returns
  by itself; a whole number). The two new ways hold no places: an entity is admitted when those
  waiting (in the queue, or on their way in across a transfer) less the idle working units are
  fewer than the limit now, so it gets in when an idle working unit can take it at once, or
  when fewer than the limit wait; an entity whose job is paused on its unit is in service. When
  the limit drops (a step of the timetable, a unit taken out, a failure, a shift's end), those
  already waiting stay; only new arrivals are turned away until fewer wait than the limit.
  Senders held up look again whenever a unit's state changes, an entity gives up or the
  timetable steps. No unit keeps a place of its own (`placeWhileOut` changes nothing there, and
  the inspector asks it only of a fixed room): per working server, a unit failed or out already
  lowers the limit. File format: `stochcraft:perWorkingServer="true"` on
  `stochcraft:queueCapacity`, or a `stochcraft:byShift` child holding BPSim values `validFor`
  calendars; versions without them read the value as a fixed room. On the wire
  `queue_per_working_server` and `queue_shifts` (`{calendar, places}`), exclusive; in the
  inspector a choice under the waiting room (fixed, by calendar once the model has a calendar,
  per working server) with the table of places by calendar.
- **A way out for those turned away:** done. A server with a finite waiting room (any of the
  three ways) can have one way out marked `stochcraft:turnedAway` (`turned_away` on the wire, a
  tick on the flow in the inspector). Every entity finding no room, an arrival from a source or
  an entity from any other element, takes it at once instead of being lost or waiting before the
  server, so a gateway or a server feeding it is never held up, and no `buffer_behind_gateway`
  warning or blocking loop is reported through it. They count in the existing `lost` (each
  time, so with retries it counts attempts) and `loss_probability` (lost over lost plus
  entered); a source still counts its own arrivals turned away as lost, which tells new
  arrivals from retries. Without the way nothing changes. Validation: at most one per server
  (`several_turned_away_ways`), only out of a server with a finite room
  (`turned_away_without_room`), never also the abandoned way (`turned_away_and_abandoned`); a
  room by timetable needs a calendar's value (`queue_by_shift_empty`). An entity turned away
  or giving up into another finite room is let in or turned away there by that room's rule.
- **Checks with known answers (waiting rooms):** done (`blocks/tests/admission.rs`). By hand:
  admission at the limit; those waiting staying as a shift ends and as the timetable lowers the
  limit; a sender blocked until the timetable opens the room; failures and a takeout per working
  server, the place rule changing nothing; the way out from a source, a gateway (which never
  waits) and a server (never blocked); a way never taken changing nothing. Heavy tests against
  exact chains solved separately:

  | Case | Quantity | Exact | StochCraft (95%) |
  |---|---|---|---|
  | A: c 3, λ 1.8, μ 1, health check (rate 1, limit 1.5, restart mean 2, retry), 2 waiting per working server; turned away wait (mean 3) and try again with probability 0.9 | mean queue | 1.8564 | 1.868 ± 0.036 |
  | | mean in service (units working) | 2.4058 | 2.400 ± 0.014 |
  | | busy (utilisation × 3) | 1.6007 | 1.599 ± 0.005 |
  | | mean waiting to try again | 5.9793 | 6.014 ± 0.178 |
  | | turned away per time unit (all; new arrivals) | 1.9931; 0.4996 | 2.004 ± 0.058; 0.504 ± 0.012 |
  | | gave up, taken out, served per time unit | 0.1993, 0.2971, 1.6007 | 0.2007 ± 0.0057, 0.2993 ± 0.0068, 1.5999 ± 0.0055 |
  | B: c 2, λ 1.5, μ 1, from empty, 1 waiting over [0, 50), 6 over [50, 100), run to 100 | turned away | 15.8626 | 15.831 ± 0.090 |
  | | mean queue; mean in system | 0.6407; 1.9559 | 0.6401 ± 0.0042; 1.9551 ± 0.0055 |
  | | in system at the end | 2.6240 | 2.631 ± 0.030 |

- **Not yet (waiting rooms):** a fractional number per working server; a shortest-queue
  gateway still chooses only among ways with room and waits (or, for a source, loses the
  arrival at the gateway) when none has, rather than using a task's way for those turned away;
  a sender held up and let in that then waits for its own unit's repair before handing on can
  let one more in than the limit.

### M2.11 Changeovers and lean measures

A line that makes several products, and results in the terms lean practice uses.

- **Product mix and routing by type:** already possible. A source sets a property (a product
  type) on each entity it creates, and an exclusive gateway's conditions read it with
  `getProperty` (M2.2, M2.3). A product mix is one setting: a property whose value is a mix,
  text values each drawn by its share (`{"mix": [...]}`), written as BPSim's own discrete
  `UserDistribution` of `StringParameter`s (BPSim 2.0, 6.2.8 and 6.3.5.13). Done.
- **Changeovers:** done. A server's `setup` is either a setup before every job, BPSim's own
  `SetUpTime` ("The time expended prior to performing the actual work", part of each job's
  duration, BPSim 2.0, 6.2.4; a setup of 0, BPSim's default, is none), or a changeover, the
  vendor extension `stochcraft:changeover`: a unit sets up only when a property's value differs
  from the last part's (its first part needs none), with times for particular changes from one
  value to another (sequence-dependent setups). Setting up is a unit state of its own,
  reported as `time_setup`; it counts in utilisation, draws processing power, and a failure
  interrupts it like a job. Setups draw from their own random stream, so service times stay
  those of the same model without setups.
- **Processing time by product type:** done. A task (a server, a task with shared resources
  or a delay) can name a property and give a processing time for particular values of it: a
  part whose value has a time takes it, and any other part (another value, or none) takes the
  task's own processing time. The vendor extension `stochcraft:processingTimeBy`, since BPSim
  2.0 has one processing time per task. The times for values draw from a stream of the task's
  own, so a model without them, or whose values no part has, gives exactly the results it gave
  before, and the other parts keep the task's own draws. A job a failure, a health check or a
  shift's end stops resumes with the part's own remaining work, and one retried draws afresh
  from the part's own time. Checked against M/G/1 with
  hyperexponential service (Pollaczek-Khinchine), overall and per type. Not here: results per
  product type (route by type to separate sinks for each type's time in system).
- **Lean results:**
  - takt and cycle time: done (#123). A departures block can have a customer demand, a
    quantity per run, day, week or shift of a calendar (the vendor extension
    `stochcraft:demand`). It then reports `takt` (the available time divided by the demand, the
    Lean Enterprise Institute's Lean Lexicon: "the available production time divided by
    customer demand"; with a calendar, only the time it is active), `demand` and `demand_met`,
    and every server reports `cycle_time` (each part's setup and processing over its units) to
    compare with takt. Set in the window, in every catalog. Not yet: a cycle time for the line
    as a whole;
  - each station's time split into states: done. Every server reports `time_processing` and
    `time_starved` (idle, on shift and up), with `time_blocked`, `time_failed`,
    `time_off_shift`, `time_setup` and `time_restarting` where the server can be in that state;
    the shares add up to 1 (the states of calendars on servers and of energy per server state,
    under M2.6);
  - OEE from ISO 22400-2:2014+A1:2017 (KPIs availability, effectiveness, quality ratio, OEE
    index, setup ratio). Done: per server, `oee_availability` (production time over planned
    busy time, off shift being planned downtime), `oee_effectiveness` (from a planned run time
    per item, the vendor extension `stochcraft:plannedRunTime`; not reported without one, and
    the results say how to get it) and `setup_ratio`. Also done: `quality_ratio`, `scrap_ratio`
    and `rework_ratio` from scrap sinks (`stochcraft:scrap`) and rework connections
    (`stochcraft:rework`) the user marks, as established tools do it with percentage outputs,
    by a probability route; charged to the last server the part finished
    at (it travels as properties do), or one the scrap sink names; each pass counts as produced,
    and good = produced − scrap − rework (deduced from ISO 22400-2's definitions in 5.5). And
    `oee`, availability × effectiveness × quality ratio, with quality counted as 1 (and said so)
    when the model marks no scrap or rework. Not yet: ISO's NEE index and per-product plans.
- **Current and future state:** a current-state and a future-state value stream map as two
  scenarios of one model, compared with `results.compare` (paired when they share their seed
  and replications, #73).

**Later in M2:** costs (BPSim `fixedCost`, `unitCost`); simulation traces in XES (BPSim
`traceOutput`). Conveyors and travel networks are M5; agent behaviours and stock-and-flow are
M4.

**M2 size:** about nine PRs for M2.1–M2.8 and one or two each for M2.9, M2.10 and M2.11, each independently mergeable after M1. **M2.1–M2.8 are complete**, and so is M2.10 apart from the gaps listed in it.
What each leaves for later is under its own "Not yet" or "Not here". The rest of M2.9 (output
analysis: the window's Welch plot, sequential replications, validation against observed data,
the capacity table) and the rest of M2.11 (a line cycle time, ISO's NEE index, per-product plans, current and future
state) are next.

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
  `apps/stochcraft-web` the wasm32 shell. The interface never touches files or dialogs itself: it
  asks its shell (pick a file, write these bytes, offer a download), so it is the same code and
  the same tests on both.
- **The browser build from the first PR** (moved forward from M3.6): GitHub Pages publishes
  `main` at the site's root and every pull request at `pr/<number>/`, linked from the pull
  request, so each interface change can be tried before it is merged.
- All UI changes go through engine commands (principle 2), so undo, redo and the journal work
  from the first window.

### M3.1 Shell and documents

**M3.1a (done):** `ui-egui` with the window (menus, keyboard shortcuts, the model outline,
results, a status bar with run progress and cancel), open, save, save as, built-in example
models, unsaved-change prompts (also for a file dropped on the window, and the browser tab's
close), the About box with licences; the desktop shell on eframe with craft-dialogs; the browser
shell (file picker, downloads, dropped files); the Pages workflow with pull-request previews.
The look follows the Craft suite (IBM Plex) and Impeccable's guidance for application screens:
an ink header with the primary action, a cooler side rail showing the process as its BPMN
shapes, results in the model's words with units and aligned numbers, empty and running states
that say what to do next, and light and dark themes composed separately.
Interaction tests drive the window through egui_kittest's accessibility tree, natively and in
the browser's mode (runs in slices, saving as downloads).

**M3.1b, the shell on craft-libs (done):** every command runs from one catalogue
(`ui-egui/src/commands.rs`): craft-keymap holds the shortcuts and dispatches keys, craft-menu
lays the menus out per platform (About, Settings and Quit where each keeps them; on the web
without the browser's own shortcuts, Quit or file paths) and draws the in-window bar, with the
macOS menu bar through its native backend. craft-settings keeps the preferences (appearance,
recent files) in the platform's config folder; the browser keeps them in its storage. File ▸
Open Recent and View ▸ Appearance are new. craft-window decides who draws the window frame and,
where the app does (GNOME on Wayland), the header becomes the title bar with the desktop's window
buttons; craft-quit quits the platform's way; craft-atomic makes saves crash-safe;
craft-platform downloads files in the browser.

**M3.1c, controls on craft-ui (first part done):** number fields are craft-ui's `NumberField`,
reading and writing numbers as the user's region does (craft-i18n's `Region::system()`: decimal
mark, grouping, digits), stepping with the arrow keys; choice boxes are craft-ui's `ComboBox`; the
Model and Results tabs are craft-ui `Tab`s; fonts come from craft-ui's stack (IBM Plex first,
egui's faces for symbols, craft-fonts' faces where `CRAFT_FONTS_DIR` names a checkout, then the
installed system fonts on the desktop). The desktop embeds the craft-fonts faces except the CJK
ones; the web app serves them beside the wasm (CJK included) and fetches a face when the
interface language or a character needs it. On the web, View ▸ Language names a language in
English (Japanese, Korean, Hindi…) until the face for its own script has been fetched, so opening
the menu fetches nothing.

**M3.1d (keyboard shortcuts done):** Settings ▸ Keyboard Shortcuts… (Preferences in Edit on
Windows and Linux, the app menu on macOS) opens craft-keymap's dialog: every command grouped by
its menu, recording, conflicts, undo and a summary; the user's shortcuts are saved with the
preferences (`config.toml` on the desktop, the browser's storage on the web).

**M3.1e (autosave done):** while a model has unsaved changes, a recovery copy is written every
ten seconds on the desktop (atomically, to `recovery.json` in the user's local data folder) and
every half minute and on closing the page on the web (the browser's storage), and removed once
the model is saved, closed or discarded on quit. If StochCraft stops with changes unsaved, the
next start asks whether to recover them as an untitled model or discard them.

**M3.1f (translations):** every interface string goes through craft-i18n, with a pseudo-locale
test that fails on any that doesn't. StochCraft's own catalogs (`i18n/`, 44 left-to-right
languages) are built in, each with a row for every string the window and the desktop shell look
up (a test checks it); View ▸ Language picks one or follows the system's, and the
choice is saved with the preferences.

**Still to come:** native-speaker review of the catalogs; the engine's own messages through the
catalog; the keyboard-shortcuts dialog's own words, which come from craft-i18n's shared catalog
(not embedded in CI or the web app, and incomplete upstream); faces on the web for Bengali, Tamil,
Telugu and Lao, which craft-fonts doesn't have yet (they draw with the browser's fonts where the
visitor's system has them, and as boxes elsewhere); Traditional Chinese forms on the web (Noto
Sans CJK SC maps the characters, but egui doesn't apply the OpenType `locl` feature, so zh-Hant
shows Simplified-style shapes where regional forms differ); smaller CJK downloads on the web (the
first use downloads the whole face: about 16 MB for Chinese, 9 MB for Japanese, regular and
bold, and 2 MB for Korean; a cut to the catalogs' characters waits for craft-fonts' interface
faces); screenshot tests in craft-ui's six test
languages; panes and icons.

### M3.2 Model editing without a canvas

**Done:** the process list selects blocks (arrow keys, Delete, Esc) and adds them; the inspector
edits a block's name, parameters and flows; the model overview edits the name and experiment,
lists every connection with a way to join two blocks, and shows the model's problems, which also
mark their blocks in the list. The forms are generated from the engine's command schemas
(`ui-egui/src/form.rs`): numbers commit on Enter or leaving the field, text that is not a number
stays in its field marked until fixed, a distribution's kind is a choice that keeps its scale,
and times show the model's unit. Each edit is one command and one undo step. A Model and a
Results tab divide the work area. Tests drive every path through the accessibility tree.

Entity properties (M2.2) are edited in the inspector from `property.set`'s schema (a value is a
number, yes or no, text, a random draw or an expression; malformed expressions show as the
block's problem), and runs report each property's mean assigned value.

Routing (M2.3) is edited there too: Add block offers a decision (exclusive gateway) and a split
or join (parallel gateway), drawn as BPMN diamonds; a decision's ways out take a probability and
a condition (blank when unset), and a default flow; a flow's problems (a condition that can't be
read) show on its block.

**Resource pools (M2.5):** the model overview lists the shared pools, each with its name, units
and roles, with Add pool and Remove. **+ Add block** offers a task with shared resources (needing
one unit of the first pool, made if there is none), whose inspector lists its needs: one row per
alternative, by pool or by role, with its units, in the order they are tried.

### M3.3 Running and results

**Done:** experiment settings (M3.2's overview), Run with progress and cancel (M3.1a), the results
table with confidence intervals, and charts above it: utilisation, mean wait, mean queue length
and mean time in system per block, one measure per chart, the 95% interval as a whisker and a
tooltip with the interval on hover (`ui-egui/src/charts.rs`, following the dataviz rules: one
hue validated per theme, thin bars, recessive grid, the table as the accessible view). The table
exports as CSV (as `stochcraft-cli run --format csv`) and the charts as PNG, saved where the user
chooses on the desktop and downloaded in the browser.

**Moved to M3.5 (done there):** queue length over time for a single replication, which needs a
trace of one run from the engine; the live view of a run produces exactly that.

### M3.4 The flowchart canvas

Blocks drawn as BPMN shapes, drag to place, drag between them to connect, pan and zoom,
multiple selection, copy and paste, align. Positions are BPMN DI, already part of the model file,
so a model drawn in another BPMN editor opens with its layout.

**Done:**
- **Layout in the model:** `Model.layout` holds each element's shape bounds.
  - Read from BPMN DI (`BPMNShape`/`dc:Bounds`) or XPDL `NodeGraphicsInfo`.
  - Saved back in place, and edges to a moved shape redrawn straight.
  - A file without a diagram gets one once a shape is placed.
  - Elements without a shape are placed by `Model::placed`: columns by distance from the
    sources, as new files already were.
  - The read-back check after saving compares the shapes too.
- **Engine:**
  - `layout.set` moves or resizes shapes in one step.
  - `block.add` takes `x` and `y`; in a laid-out model it places a new block to the right of
    the one before it.
  - `batch` runs several editing commands as one undo step, and applies none of them if one
    fails.
  - Elements carry `bounds` on the wire.
- **The canvas** (`ui-egui/src/canvas.rs`), beside the inspector (below it in a narrow window):
  - Shapes in BPMN notation: events as circles (end events thick), tasks as rounded
    rectangles with their kind, decisions with an ×, splits and joins with a +. Errors and
    warnings colour the outline.
  - Gateway branches show their probability.
  - Drag a shape to move it (snapped to 10 units), together with the rest of the selection.
  - Drag from a shape's handle to another shape to connect them.
  - Drag a kind from the palette to add a block where it is dropped.
  - Click, Shift- or Ctrl-click, or drag a rectangle to select; click a connection to select
    it.
  - Cut, Copy, Paste, Duplicate, Delete and Select All in the Edit menu, with the usual
    shortcuts. Copies get new ids and keep the connections between them.
  - Several selected blocks can be aligned by edges, centres or middles, or spread out evenly.
  - Scroll or drag with the middle or right button to pan; Ctrl+scroll, pinch or the corner
    buttons to zoom; Fit.
- **Tests:** `ui-egui/tests/canvas.rs` drives these with the pointer.

**Not yet:** edges with bends (a moved shape's edges are redrawn straight; other edges keep
their waypoints), resizing shapes by hand (`layout.set` can), and lanes and pools.

### M3.5 Live view of a run

Step a single replication at a chosen speed (the engine already supports `run_until`
in slices) and show counts, queue lengths and utilisation on the canvas as it runs; pause, step
and jump to a time.

**Done:**
- **Stepping a run** (`blocks`): `Compiled::live` gives a `Live` run of one replication.
  - Advance it to a time or by one event, read each block's and pool's state now, and get the
    statistics so far.
  - Tests check it is the same replication: run to its end in uneven slices, or one event at a
    time, it gives exactly the report `Compiled::run` gives with the same random stream.
  - `Engine::live_run(i)` is replication *i* of the experiment, so watching shows replication 1
    of Run.
- **Watching in the window** (`ui-egui/src/live.rs`), from the bar above the canvas or
  Model ▸ Watch a Run:
  - Play and Pause (also Space), Step to the next event, a speed in simulated time per second
    (about a minute for the whole run by default), and Jump to a time. Since M5.5's U1 (#218),
    the run bar has Play/Pause on Space, Reset, Step on `.`, a speed as ×N halved and doubled
    with `[` and `]`, As fast as possible, and Pause at a time in place of Jump.
  - A clock bar shows where the run is, and when it is in its warm-up.
  - On the canvas:
    - each task shows the entities waiting and its units busy, with a bar for busy units;
    - delays show the entities inside;
    - sources show how many were created, and departures how many are done.
  - The inspector shows the selected block's counts and a chart of its queue (or what is inside)
    over time. This is the queue-length-over-time view moved here from M3.3.
- **A frame's budget** (#76, #80): watching runs on the window's own thread, so each frame runs
  events for at most about 8 ms (`FRAME_BUDGET`), in chunks of 1,000 between looks at the clock.
  A step or a jump that doesn't fit carries on over the next frames, with the same events in the
  same order; playing falls behind its speed rather than freezing the window.
- **Model changes:** changing anything the run depends on stops watching; moving shapes does
  not.
- **Tests:** `blocks/tests/live.rs`, `engine/tests/engine.rs`, `ui-egui/tests/live.rs`.

**Not yet:** entities drawn moving along the connections, watching in a web worker, and
replications other than the first.

### M3.6 Browser and agents

- `apps/stochcraft-web` (in M3.1a): replications in web workers, installable as an app, and
  working offline (done).
- `crates/mcp` and `stochcraft-cli mcp`: the engine's commands as MCP tools, headless or bridged
  to a running app, following FieldCraft's MCP design (done).

**Done (workers and offline):**
- **Replications on every core in the browser.** A run started with `"mode": "workers"` hands
  its replications to the caller's workers (`engine/src/workers.rs`).
  - `Engine::worker_task` gives the run as text: the model as JSON, the seed and the number of
    replications.
  - Each worker makes a `Worker` from it and runs the replications it is given. It sends back
    each report with every number as its bits.
  - `Engine::deliver` takes the answers in any order.
  - The results are exactly those of a run on threads.
- **The page's workers.** The shell gives the app a `Workers` (`ui-egui`); in the browser that is
  a pool of web workers (`stochcraft-web/src/pool.rs`).
  - There is one worker per core but one, up to 16.
  - Each worker loads `stochcraft-worker` (`src/worker.rs`, 0.75 MB of WebAssembly), and is
    given the next replication as soon as it answers.
  - Where workers cannot start, the page runs the replications itself between frames, in
    slices of events (#43): each frame runs at most `SLICE_EVENTS` (5,000) events, and a
    replication that does not finish carries on in the next frame (the `Live` run is kept in the
    engine between polls). So one long replication no longer freezes the tab, Cancel acts within
    a frame, and the progress bar moves within a replication. The stuck-time limit counts events
    at one time across slices (`des::Sim::run_watched_slice`), so it fires as before. Results
    are exactly those of threads whatever the slice size: `engine/tests/slices.rs` checks slices
    of 1, 7, 1,000 and unlimited events on every committed model, plus models with a warm-up, a
    shared pool and stop conditions. The slice size is an estimate: natively (release, one core
    of an i7-12700K) the test models run at 1,900 to 7,200 events per millisecond; assuming
    WebAssembly on a laptop at about a third of that, 5,000 events take at most about 8 ms, half
    a 60 fps frame.
- **Installable and offline.**
  - A web app manifest and an icon make the page installable.
  - A service worker (`sw.js`) keeps the page and every file it names when it installs, so the
    app opens and runs models offline from the first visit on.
  - The service worker is network first, so a new build is used as soon as it is published.
  - Each pull request's preview keeps its own cache.
- **Tests:** `engine/tests/workers.rs` runs every committed and BPSim reference model that
  StochCraft can run (11 with the reference models) on two workers, answering out of order. The
  results must equal those of threads, and each model must come back from its JSON unchanged.
  Also covered: cancelling, failing, and refusing bad answers. `ui-egui/tests/app.rs` checks a
  run on the shell's workers, falling back when there are none, and cancelling. Checked by hand
  in Chromium: three workers ran the M/M/1 example, and offline the page reloaded from the cache
  and ran the tandem line.

**Done (agents):** docs/mcp.md.
- **`crates/mcp`** (`stochcraft-mcp`): MCP over stdio (JSON-RPC 2.0, protocol revisions
  2025-06-18, 2025-03-26 and 2024-11-05), following FieldCraft's server.
  - Tools: `list_commands`, `execute`, `batch`, `inspect_model`, `check_model`, `open`, `save`,
    `run`, `undo` and `redo`.
  - Resources: the model and the command catalogue.
  - Input is treated as hostile: lines are capped at 16 MiB, JSON-RPC batches at 100 messages
    and `batch` calls at 10 000 steps, and arguments are checked against each tool's schema.
- **`stochcraft-cli mcp`** runs it headless. `--read-root` and `--write-root` confine the files
  the agent opens and saves (links followed, whole-folder comparison, as in FieldCraft).
- **Watching an agent:** `stochcraft --control PORT` listens on the loopback interface, and
  `stochcraft-cli mcp --connect 127.0.0.1:PORT` drives the window through it. A model the agent
  opens replaces the one shown, its runs' results are shown, and its edits can be undone.
- **Tests:** `mcp/tests/session.rs` covers four areas:
  - an agent's session (open, batch build, check, run against M/M/1 theory, undo, save,
    resources);
  - hostile input;
  - the roots;
  - the same tools through a control channel.

  `ui-egui/tests/control.rs` checks that an agent's commands show in the window. Checked by hand:
  the desktop app on a virtual display, driven by `stochcraft-cli mcp --connect`, opened the
  tandem line and showed its results.

**Later:** 2D layout with entities moving between stations, and 3D (the `render` crate):
now step 3 of M5's sorter (M5.3).

**M3 size:** about six PRs; M3.1–M3.3 make a usable app, M3.4–M3.5 make it a modelling tool.
**M3.1–M3.6 are complete.** CI runs the full suite on Linux; on Windows it lints and tests the
desktop app, the MCP server and the reference models, and on macOS it lints the desktop app
(#45). The full test suite also passes on Windows (checked by hand).

### M3.7 Redesign: results that answer

**Outcome:** a newcomer or a switcher from another simulation tool opens an example, sees it
move, and gets an answer they can trust in their first session. The window explains instead of
assuming the user already knows. It follows the UX evaluation of 2026-11-15
([docs/gaps.md](docs/gaps.md), ranked), and
comes before M4, whose models need the same results over time and a second kind of diagram.

**Words (settled 2026-11-15):** "run" is one pass of the model with its own random
numbers, and "experiment" is all the runs together, so the window says "10 runs", "run 3 of 10"
and "Use 40 runs". The glossary says "Run (also called a replication)". "Block" names every
object on the canvas, "entity" what flows, and "waiting room (queue)" a task's buffer. Only the
window's words change: the command line, the JSON results and the file keep `replications`.

The steps, in the order of gaps.md, each a series of pull requests:

1. **Words and messages** (gaps 7). One vocabulary in the window, the results and the engine's
   messages, which name the block and say how to fix the problem; engine messages through the
   catalogs. Every rewritten string is a row in each language's catalog, translated from
   dictionaries, Wikipedia, IATE or the literature only. **Done** (#139, #221; issue #145).
2. **Problems where they are** (gaps 8). Limits on fields; "Run (1 problem)" that selects the
   block and field; a problems chip and clickable status-bar errors; a fixed place for problems
   so the inspector never jumps; Basics first in the inspector (name, capacity, times), the rest
   folded into rows that show a summary when set. **Done** (issue #146).
3. **Results that answer** (gaps 1, screen 1a). In this order: two to four finding sentences,
   each linked to its evidence; "How sure are these numbers?" with one-click fixes ("Use 40
   runs", "Set a 1 h warm-up") and what is already settled; four key measures per block, the
   rest under "All measures"; two default charts chosen for the question. Numbers are rounded by
   one rule: the half-width to one significant figure (two if it starts with 1), the mean to the
   same place. The chart of a measure over the run needs values recorded at fixed times, the
   recorder that M2.9's warm-up plot and M4's results over time also need: built with M2.9's
   warm-up (`series` and `warmup_suggestion` in the results), so here only its charts remain.
   **Done** (#163, #164, #165; issue #147); the findings link to the key measures, not yet to
   the model.
4. **Baseline and now** (gaps 4, screen 1b). Results stay visible and marked out of date after an
   edit; "Keep as baseline"; then baseline, now, the paired difference with its interval (the
   engine's `results.compare`, on the same seeds), and a verdict in words and an icon ("Real:
   lower", "Could be chance"). The scenario table follows later. **Done** (#166, #167, #168;
   issue #148), apart from the scenario table.
5. **The first hour** (gaps 5, screens 4a and 4b). A start page that asks "What do you want to
   find out?", with examples by question and industry, each with its question, what it teaches,
   and a proof tag where one applies; doors for newcomers (a guided 5-step first model on the real
   canvas), for people coming from another tool, and for opening a file. Help as a side panel:
   Getting started, a glossary, keyboard shortcuts and the user guide; a term's definition on
   hover and on keyboard focus. New examples join only with a checked answer, as "Examples for
   teaching" requires. **Done** (#169, #170, #171, #172; issue #149), apart from a term's
   definition on hover where the word appears in the window.
6. **The library** (gaps 3, screen 2a). A Library | Outline switch in the rail; the library
   grouped (flow in and out, work, routing, batching), searchable, each kind with an original
   BPMN-style glyph, its name and a one-line hint; click or drag to place, and a quick add at the
   pointer. Larger blocks with their names inside. Moved into M5.5's U3 (issue #150, closed).
7. **The look of a simulation** (gaps 2, screens 2b and 3). Results laid over the model after a
   run (busy, waiting, time in the line, or off), with the bottleneck named; while watching,
   entities move along the flows and stack in front of tasks, unit slots show busy, idle and
   down, and badges pair a shape with words; reduced motion starts a run paused. Open (#151).

**Design rules** for every step: the existing look (`ui-egui/src/theme.rs`: the ink header, one
accent, IBM Plex) with the proposed tokens for control borders and soft error and warning
tints; colour never alone; each screen keyboard-reachable with one visible focus ring; no
modal where a panel or an inline row will do.

**Size:** about 150–345 h for steps 1–7 (gaps.md's estimates at their first step).


---

## M4: Monte Carlo, system dynamics, agents and hybrid models

**Outcome:** besides discrete-event models, StochCraft runs the three other kinds of simulation
that people expect from a simulation tool, and that they can already write in Python:

- **Monte Carlo:** a calculation with uncertain inputs, sampled many times, with no clock (M4.1);
- **system dynamics:** stocks and flows over time (M4.2);
- **agents:** individuals with their own state and behaviour, in a flow, in a population or in
  space (M4.3);

and any of them together in one model, on one clock (M4.4). All of them are saved in open
formats, run with the same replications, seeds and determinism as M2's models, and report
results with their confidence intervals.

**The bar is what Python users already have.** Someone who can write the model in a few dozen
lines of Python with numpy and scipy (Monte Carlo), PySD (system dynamics) or Mesa (agents) has
no reason to use StochCraft unless it does at least as much, with less effort, and explains
what it finds. Each part below lists what that means for its kind of model.

**Order.** After the redesign of the results and the canvas, because all three need it (results
over time, a second kind of diagram on the canvas). Then M4.1, the smallest and the one that
reuses most of what exists; then M4.2 and M4.3 (agents in flows first); then M4.4. Agents in
space are built with M5, which needs the same layout and travel.

M4.1 parts 1, 2 and 3a are done (below); the rest is not started. D15–D18 are settled; D10–D13 (#71 has
them) are to settle first.

### M4.1 Monte Carlo

**Status:** part 1 done: the `mc` crate (inputs as distributions, outputs as BPSim expressions
extended with `min`, `max`, `sum`, `abs`, `sqrt`, `exp`, `log`, `pow` and `if`, and `$name` for
inputs), plain random and Latin hypercube sampling, rank correlation by Iman and Conover, results
(mean with its interval, percentiles, minimum and maximum, a target's probability with its
interval, histogram and cumulative curve; the intervals across trials for plain random
sampling, and across independent batches of 1,000 for Latin hypercube sampling or correlated
inputs, which make trials within a batch dependent: replicated Latin hypercube sampling, Owen
1992, Glasserman 2003 §4.4) and a sequential stop on the means; files as D15 says,
with CSV import and export of the input table; the `mc.*` engine commands with undo; `run` and
`inputs` on the command line; checks against the exact answers below but Sobol's. Part 2, the
window, is done too: a Monte Carlo model is its own kind of document (File ▸ New Monte Carlo
Model, or a file with no process), with a Model tab (inputs with the distribution editor tasks
use, correlations, outputs with targets, trials, seed, sampling and a sequential stop; a table of
inputs pasted from a spreadsheet or exported as CSV; each edit one undo step, a refused one
explained in place) and a Results tab that answers first (the chance of missing each target in
words, "about a 1 in 5 chance that cost is more than 46,000 USD (21%)", with its interval; where 9
trials in 10 fall otherwise), then each output's mean with its interval, percentiles, histogram
and cumulative curve with the target on both, and how the run went. Runs are made a few batches a
frame (`stochcraft_mc::Stepper`), so the browser's page stays responsive; there the batches run
one after another on the page, not on its workers. The gallery has a project budget example,
checked against its exact means. Part 3a, which inputs matter, is done: for each output a tornado
chart (the output with each input that varies at its 10th and its 90th percentile and the others
at their medians, from the inputs' quantile functions with no sampling; the decision-analysis
swing, Clemen and Reilly, *Making Hard Decisions*), the rank correlation (Spearman) of each input
with the output from the run's own trials, ranked a piece at a time like the rest, and in words the
input it depends on most; the results say that a correlated input's rank correlation includes the
effect of the inputs it is correlated with, and that the bars leave out what correlated inputs do
together. Part 3b, Sobol indices, is built separately and comes next.

**What it is:** a model of a calculation, not a process. Inputs are distributions (a cost, a
duration, a demand), outputs are expressions over them (a total cost, a finish date, a profit),
and each trial draws every input once and evaluates every output. There is no clock and no
queue. Typical uses are cost and schedule risk, capacity and budget checks, and the outer loop
of parameter uncertainty around a discrete-event or hybrid model (D10).

**What Python users already have, and so the minimum:**

- every distribution in scipy.stats that BPSim also has, which `stochcraft-random` already
  implements, plus discrete and empirical (data) inputs, which BPSim's `UserDistribution`
  covers;
- correlated inputs: a rank correlation between inputs, as the Iman–Conover method gives
  (Iman and Conover, *A distribution-free approach to inducing rank correlation among input
  variables*, Communications in Statistics B 11(3), 1982);
- plain random sampling and Latin hypercube sampling (shared with M2.8's planned Latin hypercube
  designs);
- many trials fast: 10⁶ trials of a small model in seconds, in parallel on every core and in
  the browser's workers;
- results: mean with its confidence interval, percentiles (P5, P50, P95 and any other), the
  probability of meeting a target with its interval (M2.8's code), a histogram and the
  cumulative curve;
- which inputs matter: a tornado chart from one-at-a-time swings, rank correlation of each input
  with the output, and variance-based (Sobol) indices for models with few inputs (Saltelli et
  al., *Global Sensitivity Analysis: The Primer*, Wiley, 2008);
- convergence: the interval's half-width as trials grow, and a stop rule ("until P95 is known
  within ±1%").

**What StochCraft adds:** findings in words ("a 1 in 5 chance the project overruns"), the
inputs that matter named first, every number with its interval, and the same model reused as
the uncertainty around a process model.

**In the window:** a third kind of document beside process models. A table of inputs (name,
distribution, unit, correlation) and outputs (name, expression, unit, target), using the
existing distribution editor and expression language; then Run and the results above. A model
can be built from a spreadsheet's columns by pasting them.

**Engine:** a new L2 crate, `mc`, on `stochcraft-random` and `stochcraft-stats`; each trial is a
replication with its own random stream, so results are reproducible and independent of the
number of cores. Expressions use BPSim's XPath 1.0 subset (D6), extended with the arithmetic
and functions a cost model needs (min, max, if, round, sums over a list).

**Checks with known answers:** the sum of independent normals (exact mean and variance); the
product of log-normals (exact); a PERT schedule's finish on a single path against its
distribution; the induced rank correlation against its target; Sobol indices of the Ishigami
function against their closed form.

### M4.2 System dynamics

**What Python users already have, and so the minimum:** PySD runs models from the common
system dynamics formats, with their stocks, flows, auxiliaries, delays, lookups and arrays, and
returns every variable over time; runs with changed parameters are a loop away. So StochCraft
reads and writes XMILE (OASIS, 2015) models, runs them with the numerics below, draws stocks and
flows on the canvas, plots any variable over time, and runs the same model many times with
uncertain parameters (M4.1) to give bands over time instead of a single line.

These are the decisions to settle before the `sd` crate is written (#71); none is a bug in
today's code. The clock, replications and results over time are shared with M4.3 and M4.4.
Chapter references are to Brailsford, Churilov and Dangerfield (eds.),
*Discrete-Event Simulation and System Dynamics for Management Decision Making* (Wiley, 2014).

- **One clock** (D11). `blocks`, `agents` and `sd` all sit on L2, and `INTRA_LAYER_ORDER` in
  `xtask/src/layers.rs` allows no dependency between them, so nothing owns a shared clock yet.
  - Each SD step is a recurring event on the `des` event list.
  - A fixed rule says which goes first when an SD step and a DES event fall at the same time
    (to confirm).
  - Coupling lives either in an L3 crate or in an `sd` → `blocks` order in `layers.rs`.
  - Pidd, ch. 15 §15.5.1; Borshchev, ch. 12 §12.2.
- **Replications** (D10). Pure SD runs once; a hybrid model has its own SD state per
  replication. Uncertainty about parameters is an outer sample around the inner replications,
  not mixed into them, or the results multiply past interpreting (ch. 9 Table 9.3; ch. 15
  §15.5.2).
- **Results over time.** Results are one number per element per replication today; SD output is
  behaviour over time, and the hybrid case studies report monthly measures.
  - Values recorded at a save interval (BPSim's `baseResultFrequency`), with percentile bands per
    time point from M2.8's band code.
  - One recorder, shared with M2.9's Welch plot, which needs the same values at fixed times:
    built in M2.9 (`blocks/src/series.rs`); SD adds its stocks and flows to what it records.
  - Ch. 3 §3.2.1; ch. 14 §14.2 and §14.4.5.
- **Units.** Both sides of every SD equation balance their units, and the SD time unit is
  reconciled with `baseTimeUnit` (the class of bug #40 fixed for time units; ch. 3 §3.3.4,
  ch. 11 §11.5.1).
- **Bulk flow as exact events** (D12). Rates between changes are usually constant, so the time a
  tank reaches a level can be worked out and scheduled as a `des` event; a fixed-step `sd` finds
  "tank full" up to one step late. So tanks and pipes are best built as exact level-crossing
  events in `des` and `blocks`, not as stocks in a fixed-step `sd` (ch. 12 §12.1).
- **Numerics** (ch. 3 §3.3–3.4):
  - Euler, with a time step that is a power-of-two fraction and time computed as start + n·DT
    from a counter, so it is exact; RK4 optional for pure SD.
  - Initial values; material and information delays (their order, and whether they conserve
    material); lookup tables, clamped at their ends with a warning; IF, PULSE and STEP; arrays.
  - A mass-balance check, and stocks that cannot go below 0.
  - A fixed evaluation order: topological, ties broken by id.
  - A command that halves the step, to check that results do not depend on it.
  - Tests: a delay's pulse response against the Erlang density, logistic growth against its
    closed form, Euler's error halving when the step halves; golden SD and hybrid models.
- **From the book's worked models** (mostly ch. 9's fisheries, modelled both as SD and as DES):
  - **Noise.** An SD model with a random function is replicated like any other model; D10's
    "runs once" holds only for one without. A random function draws at its own sampling
    interval, apart from DT, so halving DT does not double the draws (ch. 9 §9.5.1.3 and §9.8;
    ch. 15 §15.5.2).
  - **Bands with the median.** Results over time show the median with the bands, not only the
    mean: runs that cycle out of phase flatten when averaged, and a mean can sit far above the
    median (ch. 9 Figs 9.10, 9.14 and 9.15).
  - **Stocks at zero.** An outflow is limited to what its stock holds in that step, so stocks
    stay at or above 0 without breaking the mass balance.
  - **Steps that are not powers of two.** An imported model runs with its own DT (0.1 day in
    ch. 6 §6.5.4), time still computed from a counter, with a warning when DT is large against
    the shortest time constant.
  - **Order of updates within a step.** A discrete-time model whose equilibrium depends on one
    flow being updated after another (ch. 9 §9.5.2.2: regrowth from the stock left after the
    catch) needs sequential updates or an explicit intermediate stock; a test pins it.
  - **Lint.** A lookup that changes direction is a warning, never an error (ch. 9 §9.5.1.1);
    "no rate-to-rate links" allows delays and co-flows (ch. 3 §3.4.2), so units include
    compound units and the dimensionless 1.
  - **Docking.** The same linear model with first-order delays built as SD and as DES: the DES
    mean equals the SD path at every time (ch. 8; "docking", ch. 13 Table 13.1). A test, and a
    teaching example.
### M4.3 Agents

**What it is:** individuals that each keep their own state and memory and decide what to do
next, rather than being moved by the process. Three kinds, planned together and built in this
order:

1. **Agents in a flow:** today's entities and resources given behaviour. A client that retries
   with growing back-off and gives up, a patient whose condition worsens while waiting, a
   machine with its own modes. They still pass through the blocks; their state chart decides
   what they do between and inside them. This is the first step because it builds on the
   blocks, and it is what the retry-storm and triage cases need.
2. **Populations:** agents outside any flow that meet and influence each other, through a
   network (who knows whom) or through contacts at random or by place: product adoption, word of
   mouth, an epidemic, a workforce. Results are counts over time, as in system dynamics, and
   the classic checks compare the two (D11's clock).
3. **Agents in space:** agents with a position that move: people walking through a building,
   vehicles and robots on a layout. Built with M5, which needs the same layout, distances and
   travel; continuous space and grids first, the walking of crowds later.

**What Python users already have, and so the minimum:** Mesa gives agents with their own step,
activation in a fixed or random order, grids, continuous space and networks, a data collector
for any agent or model variable over time, batch runs over parameters, and a browser view.
StochCraft matches that and adds:

- **Behaviour without code:** state charts drawn on the canvas, with states, transitions on a
  timeout (fixed or random), a condition, a message or a rate, and actions in the expression
  language; no Python.
- **Time that is exact:** agents act at the times their events fall, on the `des` event list,
  not in fixed steps (a stepped mode is available for models written that way).
- **Replications and intervals:** every result over time comes with its band across
  replications, like the rest of StochCraft, which a hand-written Mesa loop does not give.
- **Reproducible:** one random stream per agent population and purpose, so adding an agent does
  not change every other agent's draws.

**Building blocks** (generic terms; what each needs from `des` and `blocks` is in #71 §9):

- **Agent type:** its variables (with BPSim's property types, D3), its state chart, and the
  statistics to collect.
- **Population:** a number of agents of one type, created at the start, from arrivals, or by
  other agents; removed when their state chart ends.
- **State chart:** states nested and in parallel, entry and exit actions, transitions on a
  timeout, a condition, a message, an arrival rate, or another agent's change; history states.
- **Messages:** to one agent, to a random one, to neighbours (in the network or within a
  distance), or to all of a population.
- **Environment:** a network (random, small-world, scale-free, or read from a file), a grid, or
  continuous space with M5's layout.
- **Links to the flow:** an agent enters a block as an entity, waits for a resource, leaves;
  a block can send an agent a message; a resource unit can be an agent.
- **Statistics:** counts by state over time, means over a population, and any agent variable.

**Engine:** a new L2 crate, `agents`, beside `blocks` and `sd`, all on the `des` clock (D11).
State charts follow SCXML's semantics: run to completion, a fixed order for conflicting
transitions, internal events before external ones, which makes each agent deterministic for a
given stream (W3C SCXML 1.0, §3.13 and Appendix D).

**In the window:** the agent types and their state charts drawn on the canvas with the redesign's
blocks; populations and the environment set in the inspector; Watch a run shows agents in their
states (coloured by state, with words), and moving when they have a place.

**Checks with known answers:**

- the SIR epidemic in a large, well-mixed population against its system dynamics equations
  (Kermack and McKendrick, 1927), and its final size against the closed-form equation;
- the Bass diffusion model of adoption as agents against its closed-form curve (Bass, *A new
  product growth for model consumer durables*, Management Science 15(5), 1969);
- a population of independent agents each failing at a constant rate: the count still working
  is binomial, exactly;
- agents in a flow: M/M/1 with clients that give up after an exponential patience against
  M2.10's M/M/1+M results, now with the patience as a state chart.

### M4.4 Hybrid models

System dynamics and agents together with discrete blocks in one model, on one clock (D11),
with the couplings declared in the file rather than written as code; and Monte Carlo (M4.1) as
the outer loop of parameter uncertainty around any of them (D10).

- **Coupling primitives**, declared in the file rather than written as code (ch. 12 §12.2):
  - an SD threshold triggers a DES event;
  - DES writes only stocks;
  - SD reads queue sizes;
  - statistics over agents;
  - discrete entities to continuous amounts and back without losing any.
  - how often each coupling exchanges, and how DES output is reduced over that interval (a
    count or sum, a time average, the last value, the mean over runs): ch. 13 §13.4, and both
    ch. 14 cases exchange monthly;
  - mean-field coupling beside per-replication coupling: both worked hybrids feed SD the mean
    of 20 or 40 inner runs (ch. 14 Fig. 14.7 and §14.4.5); the two agree only when the coupling
    is linear.
- **File format** (D13): a BPMN and BPSim file names XMILE variables through a `stochcraft:`
  vendor extension.
- **Linking runs** (ch. 3 §3.4.5, ch. 11 §11.5.2.1, ch. 13 §13.4, ch. 15 §15.6 and the ch. 14
  cases):
  - starting a run from a given state;
  - monthly input series;
  - scalars passed into SD constants;
  - calibration to historical series, with the fit reported by Theil's inequality statistics
    (the bias, variance and covariance shares of the mean square error; ch. 3 §3.4.4);
  - a one-way link, one model's output as the other's input, which needs no shared clock and so
    can come before D11 (ch. 15 §15.6, ch. 13 §13.4).

Already in place: arrival rates are redrawn at each rate change (M2.6), and an agent over MCP
can open, edit and run models and drive the window (M3.6). The journal records its format, version and starting
state, so it can serve as an audit trail (#44).

---

## M5: transport and layout

**Outcome:** operators, forklifts and AGVs or AMRs that carry items between places on the
layout, with travel times from distances and speeds, and results that show where the time goes.

**Reference model: the sorter.** What people ask a factory-simulation tool to show first, and
what StochCraft cannot show today: products of several colours enter on a main loop conveyor
(with a spiral that climbs to a second level), are diverted by colour onto roller-conveyor
lanes, and two operators walk to the lane ends, pick the products up and carry them away. Beside
the layout, four charts: products sorted per hour; each operator's time as a stacked bar
(travel empty, travel loaded, loading, unloading, idle, with percentages); a histogram of each
product's time on the conveyors; and the number of products on the conveyors over time. The
sorter is built in four steps, each a series of pull requests with checks that have known
answers, and ships as an example model with its write-up ("Examples for teaching").

**Status:** step 1, the conveyors, is done (#194, #195, #197, #198); step 2 has its first PR, the
mobile pool, the Transport block and the scale in the model and files (#202), with the runtime
not yet merged (branch `operators-runtime`); step 3 has parametric meshes (#191) and a first 3D
view from the diagram's positions (#205, #208, #210, #213), to be reworked by M5.5; step 4's
dashboard is in with four tiles (#209); the sorter example is in (#207), without operators yet.

1. **Conveyors** (M5.1). The design, settled before the code:
   - **The block.** A new kind, `Conveyor { length, speed, accumulating, item_length, gap }`:
     length in the model's length unit (a new `length_unit`, metres by default, beside
     `time_unit`), speed in that length per model time unit, and every item taking a slot of
     pitch p = item length + gap. One conveyor is one straight run as far as the run is
     concerned: curves and spirals are drawn from the same length (M5.3), and a line of several
     sections is several conveyors joined by connections. Items of different sizes come later.
     (Superseded by M5.5 once D23 is accepted: the length comes from the conveyor's path, and
     sections join at junctions.)
   - **Positions.** An item enters at 0 and leaves at the length L; its position is that of its
     front, and its slot reaches p behind it.
     - It may enter only when the item before it has moved p along (so slots never overlap)
       and fewer than N = ⌊L / p⌋ items are on it; otherwise the sender waits, as for a full
       waiting room (a source loses the arrival, as it does there), and waiting senders (a
       merge) enter in the order they asked.
     - At L it is offered to what follows, with the hand-over a full waiting room uses today.
   - **Accumulating** (items slip on a stopped item ahead): an item that cannot leave stops at
     L; the ones behind keep moving until each stops a pitch behind the one ahead. When the
     front item leaves, the next moves on at once. So item i reaches the end at
     max(tᵢ + L / v, dᵢ₋₁ + p / v), where tᵢ is when it entered and dᵢ₋₁ when the one ahead left
     the end, and it may enter when fewer than N are on it and p / v has passed since the last
     entry (both hold exactly because all items move at one speed).
   - **Not accumulating** (one belt): while the front item cannot leave, the whole belt stops,
     so items keep their spacing. The runtime keeps the distance the belt has moved, s(t); an
     item that entered when the belt had moved s₀ is at s(t) − s₀, leaves at s₀ + L, and the
     next may enter when s(t) has moved p since the last entry.
   - **The runtime** (`blocks`): one process per conveyor holding its items in order with
     their entry times (and, not accumulating, the belt's distance), with the next event the
     earliest of the front item reaching L and the entrance freeing. No time steps: each event
     is computed from the formulas above, so a run is exact and as fast as a delay's.
   - **Diverts and loops.** A divert is an exclusive gateway after a conveyor, and a loop is
     conveyors and a gateway joined in a cycle. A new routing,
     `stochcraft:routing="divert"`: the gateway chooses a way as it does today (a condition
     such as the product, or probabilities); if that way cannot take the item now, the item
     takes the default way instead (round the loop), and only if the default cannot take it
     either does the conveyor stop. The conveyor itself routes through the gateway at its end,
     as senders do with shortest-queue routing today, so the item never leaves the belt before
     a way has room.
   - **Merges** need nothing new: two conveyors into one compete for its entrance, first asked,
     first in.
   - **Results.** Time on the conveyor (mean, and every item's time for the histogram of
     M5.4), items on it over time (the recorder's work in progress), throughput, and the share
     of time blocked (accumulating: the front item waiting at the end; not accumulating: the
     belt stopped).
   - **The capacity table.** A conveyor is a station of a new kind: it carries at most v / p
     items per unit of time, so its load is λ p / v and it is marked as servers are. Its time
     on is L / v exactly when nothing after it can hold it up (as `held_up_by` judges), at
     least L / v otherwise. What follows it is bounded when the load may reach 1, as for any
     station.
   - **Files.** A BPMN task with a `stochcraft:conveyor` vendor extension (`length`, `speed`,
     `accumulating`, `itemLength`, `gap`); the length unit as `stochcraft:lengthUnit` on the
     scenario. A task carrying it reads as a conveyor; other tools see a plain task.
   - **The window.** A Conveyor in the block palette with its five fields; the canvas shows how
     many items are on it during a run; Help explains accumulating.
   - **Checks with known answers:**
     - not accumulating and never blocked: every item's time on it is L / v;
     - accumulating, exit shut: it holds exactly N = ⌊L / p⌋ items, and the next sender waits;
     - the most it carries is v / p items per unit of time: from a source faster than that it
       takes one every p / v and the source loses the rest, and the capacity table marks it
       overloaded before the run;
     - a constant-time server after it, slower than the items come: accumulating, the items
       queue a pitch apart and item i leaves at max(tᵢ + L / v, dᵢ₋₁ + p / v), worked by hand
       for the first ten items; not accumulating, the belt stops each time and every item's
       time on it grows by the server's excess, worked by hand;
     - two conveyors merging into one: the shares follow the order of asking, worked by hand
       with constant arrivals;
     - divert by product into lanes, one lane always full: its items go round the loop, and
       each round adds the loop's length over the speed to their time.
   - **The order of PRs:**
     1. the block in `model` and `io` with validation and the capacity row;
     2. the runtime, results and the checks above (`blocks`);
     3. divert routing;
     4. the window (palette, form, canvas count, Help, all catalogs).
     The first and fourth can run alongside the second.
2. **Operators who walk or drive** (M5.2). The design, settled before the code:
   - **The pool.** A pool may be mobile: `Mobility { speed_empty, speed_loaded, home,
     after_job }`, speeds in the model's length unit per time unit, `home` the element its
     units start at, and `after_job` either stay where they put the item down or go home. Its
     units then have a place, and the pool keeps every unit's.
   - **Where things are.** Each element is at the centre of its shape on the canvas
     (`Model.layout`), times a new `layout_scale` (length units per drawing unit, beside the
     length unit). Distance is the straight line between two centres; the travel network of
     paths with the shortest route (below) comes after, in its own PR, and replaces the straight
     line where a model has one. (M5.5 replaces canvas centres with layout places and walkways;
     `layout_scale` then serves only models without a layout, D28.)
   - **The block.** A new kind, `Transport { pool, load, unload }` (`load` and `unload`
     distributions): an item reaching it is carried from the element before it to the element
     after it by a unit of the pool. It waits there for a unit (its queue, unbounded at first);
     the unit travels empty from where it is to the pickup, loads, travels loaded to the drop-off,
     unloads, and is then free there (or walks home, as `after_job` says, free to be taken on
     the way only at home).
   - **Which unit.** Requests are served first come, first served; a request takes the idle
     unit nearest its pickup (ties: the one idle longest, then the first). A unit freed takes
     the oldest request waiting.
   - **The unit's states**, as the sorter's bar names them: idle, travelling empty, loading,
     travelling loaded, unloading (and off shift where the pool follows a calendar). They add up
     to all of the time.
   - **Results.** For the pool: each state's share of the time and the distance travelled
     empty and loaded per unit. For the block: the mean wait for transport (from reaching it
     until loading starts, the unit's walk to it included) and the mean time from pickup to
     drop-off.
   - **The capacity table.** A transport's work per item is at least load + unload + the loaded
     distance over the loaded speed (the empty trip depends on where the unit was), so its
     pool's load is at least that; the status is unstable when even that reaches 100%, else not
     known before a run.
   - **Files.** A `stochcraft:transport` vendor extension on the task (its pool, load and
     unload); the pool's mobility as a `stochcraft:mobility` extension on its resource; the
     scale as `stochcraft:layoutScale` on the scenario.
   - **Checks with known answers:**
     - one operator, pickup and drop-off 10 apart, speeds 1 empty and 2 loaded, load 1, unload
       1, staying where it puts the item down, items always waiting: each cycle is walk back 10,
       load 1, carry 5, unload 1, so 17, the throughput 1/17, and the states 10/17, 1/17, 5/17,
       1/17 and no idle;
     - the same with items arriving every 40: idle 23/40 of the time, and each item waits 10,
       the walk back, since its wait for transport runs until loading starts;
     - two operators at different places and a request between them: the nearer one goes,
       worked by hand;
     - going home after each job: the walk home counts as travelling empty;
     - the states add up to all of the time in every run.
   - **The order of PRs:** 1. the mobile pool, the block, the scale, validation, files and the
     capacity row; 2. the runtime, results and the checks; 3. the window (pool and block forms,
     the stacked state bar per pool); 4. the travel network with shortest routes.
3. **The layout view, 2D then 3D** (M5.3; its geometry is reworked by M5.5, where the layout is
   a model layer of its own rather than the diagram's positions): stations, conveyors and paths
   drawn at their layout positions to scale, with entities moving along conveyors and paths, the moving entities of
   M3.7.7 (#151) taken from the flowchart onto the layout. Then 3D, as parametric low-poly
   models generated by code (settled 2026-11-19):
   - **Every 3D object is a low-poly mesh generated in Rust from its parameters; there are no
     asset files.** A straight belt or roller section from its length, width and height; a
     curve from its radius and angle; a spiral from its radius, rise and number of turns; side
     guards and legs at a set spacing; roller lanes; a machine or station as a box with a few
     details; racks and shelves from their bays and levels; a pallet, tote or box for each
     entity, coloured by product; an operator (a capsule body and a head); a forklift and an
     AGV; walls and a floor grid.
   - **Parameters come from the model.** The length and path a conveyor has in the run are the
     numbers that shape its mesh, so the 3D view cannot drift from the model.
   - **Mesh generation** is pure, testable and independent of any renderer: plain vertex and
     index buffers, in a module or crate of its own (in the `render` layer, L5). Checks: vertex
     and face counts, and bounding boxes, match the parameters; normals face outward; meshes
     are closed where they should be.
   - **Rendering: the Craft apps' shared viewport, from SolidCraft** (decision D22). SolidCraft
     (`tangency-labs/solidcraft`, MIT OR Apache-2.0 like StochCraft) has the viewport the Craft
     apps share: `viewport` (orbit camera, projection, picking rays, the scene of meshes and
     lines, surface shading, and a CPU rasterizer that draws a scene to a PNG without a GPU),
     `viewport-gpu` (its pipelines on wgpu) and `viewport-egui` (drawn in egui-wgpu paint
     callbacks); its browser app runs the same viewport. The live view is its `Frame`,
     `GpuScene` and `Renderer` (`viewport-gpu/src/view.rs`), with each entity's mesh drawn once
     per placement through `InstancedMesh`; `Scene` is the description of a still picture (the
     headless GPU capture and the CPU rasterizer), which also gives tests a picture to check and
     the report a 3D image. StochCraft hands it the meshes it generates rather than writing a
     renderer of its own.
   - **Clean room:** shapes come from real-world dimensions only, never traced or copied from
     any commercial tool's models.
   - **Shared code** (checked 2026-11-19): the viewport above. The parametric meshes are
     StochCraft's own (SolidCraft models solids with a B-rep kernel, which a conveyor does not
     need), built as the viewport's mesh type. CADCraft is 2D drafting with no meshes, and no
     `craft-factory` exists in any Craft repository.
   - **What reuse costs, to settle first:**
     - only if the CPU renderer is measured too slow (D27): the apps move from eframe's glow
       backend to wgpu, as SolidCraft runs, and the browser build is checked again on WebGPU
       and on wgpu's WebGL2 fallback;
     - both apps must build the same egui: StochCraft pins the storytold fork at a revision,
       SolidCraft vendors its own copy, so the viewport crates come in at a matching revision
       (or move to `craft-libs` first);
     - the viewport's mesh type lives in `solidcore-record`, part of SolidCraft's vendored
       kernel: what it pulls in is to measure before depending on it, and a small mesh crate
       split out of it is the fallback;
     - many moving entities: `InstancedMesh` keeps one copy of a mesh on the GPU and re-sends
       only the placements each frame; 100 and 5,000 moving totes are measured on WebGL2 and
       an integrated GPU before the 3D view depends on it;
     - the egui-wgpu changes SolidCraft's viewport relies on (the drawable acquired before the
       paint callbacks are prepared, the multisampling clamp) go into the shared egui fork;
     - its mesh type lives in `solidcore-tess`, which compiles SolidCraft's B-rep kernel: the
       mesh type is asked to move into a small crate of its own, and meanwhile the cost is
       measured;
     - picking meshes is ours to write (the camera gives the ray).
     The spike of 2026-11-19 found the apps' move to wgpu small (one eframe feature and one
     function signature in each app), with egui 0.37.0 and wgpu 30.0.1 on both sides.

   Checks for the view, read from a live run in a test: an entity moving freely is drawn at its
   entry point plus speed × (time moving) along the conveyor's path; one stopped (its exit
   blocked, or queued behind the item ahead) is drawn where it stopped, at the exit or one item
   length plus gap behind the next, until it moves again; the 2D and 3D views place every
   entity at the same layout point.
4. **The sorter's dashboard** (M5.4): the four charts, mostly from what exists today:
   - products sorted per hour: departures over the time run (exists);
   - the operators' stacked state bars: the state times every server reports today (exists)
     plus the new travel and handling states of step 2;
   - the products on the conveyors over time: M2.9's recorder of each element's number inside
     (exists), summed over a chosen group of elements (missing: groups of elements);
   - the histogram of each product's time on the conveyors (missing: today `results.distribution`
     is over replications' values; a histogram over every entity's time on a group of elements,
     with its bins merged across replications, is new).

   Checks: the throughput tile equals the departures count over the duration; each operator's
   bar adds up to 100%; the histogram's mean equals the mean time on the conveyors that the
   results report, and on step 1's never-blocked conveyor all of it sits in the bin holding
   length / speed.

The work below is what the four steps need, in more detail.

A milestone of its own rather than an M2 part. Today's resources stay where they are: a unit is
taken, held and given back, with no place and no travel (M2.5). Moving resources need a place
for every station, a network to travel on, and new `des` and `blocks` behaviour, and they lean on
the canvas (M3.4) for the layout. Conveyors join here too, since they need the same lengths,
speeds and positions.

- **Moving resources:** a pool whose units are at a place, travel to pick an item up, carry it
  and put it down.
  - **Task sequences:** load, travel loaded, unload, then return or wait for the next job;
    travel empty and loaded kept apart.
  - **Dispatching rules:** nearest idle unit, first come first served, and balanced (the unit
    with the least work so far).
- **Travel network:** paths between nodes, with a length each and a speed per resource; travel
  time is the shortest path's length over the speed. Free-space pathfinding comes later.
- **Layout:** stations at their canvas positions. Today `Model.layout` keeps each element's shape
  bounds from BPMN DI or XPDL graphics (`model::Bounds`: x, y, width, height); travel needs a
  scale from those drawing units to lengths, and the network's paths. An optional floor plan as
  the canvas's background image (there is none today).
- **Conveyors:** length, speed and accumulation (moved here from "Later in M2").
- **Files:** `stochcraft:` vendor extensions for the network, the scale and each resource's
  speeds and rules, as for the other StochCraft-only data (decision D1); BPSim's own
  `TransferTime` is read today and ignored with a warning ("not simulated yet",
  `io/src/read.rs`), and whether it fits a travel time is to verify from the specification.
- **Results:** each moving resource's time loaded, empty and idle; distance travelled; and each
  station's wait for transport.
- **Checks with known answers:**
  - one vehicle serving fixed stations with constant times: the cycle is the sum of its travel,
    load and unload times, worked by hand;
  - machine repair with travel (the M/M/R//N case of M2.5 with a repairman who travels): a
    known result to find (to verify).

- **Files for the sorter:** a conveyor is a BPMN task with a `stochcraft:conveyor` vendor
  extension (length, speed, accumulating, item length and gap, the points along it where
  diverts and merges sit); positions and paths on the layout, with their heights, in a
  `stochcraft:layout` extension beside BPMN DI (which is a 2D diagram, not a floor plan), as
  for the other StochCraft-only data (D1). Look-only scenery goes in a `stochcraft:scene` extension in the same file (M5.5,
  D24).
- **Later:** free-space pathfinding; vehicles that block each other on a path; power-and-free
  and overhead conveyors.

### M5.5 Physical layout, the Layout view and generated 3D

**Why.** The first 3D view (#205, #208, #210) guesses geometry from the BPMN diagram. Belts run
between block centres, a loop is an invented ring, and lanes fan out from one point. A user who
tried it said it is still far from what anyone expects. Established tools, open and commercial
alike, do not do this. They keep the physical layout as a model of its own, in metres, and draw
both the plan and the 3D view from it. The process logic then names layout objects; it never
supplies their shape. Of the open tools, JaamSim puts geometry on each object in SI units,
openTCS keeps a plant model in millimetres apart from its drawing, and Casymda, which also
starts from BPMN, moves its entities on a tile map of its own rather than on the diagram. M5.5 makes the
layout the source of truth for geometry. It supersedes, where they differ, the geometry parts of
M5.1 ("one conveyor is one straight run"), M5.2 (positions from canvas centres times
`layout_scale`) and M5.3 (3D from layout positions), once D23 to D28 are accepted.

**Outcome:** a model has a to-scale layout that the user draws or generates, the run reads its
lengths and distances from it, and the 2D plan and the 3D view are two pictures of that one
layout. The flagship example is a parcel super hub.

#### Three layers with stable ids

| Layer | Holds | Source of truth for |
|---|---|---|
| **Logic** | BPMN blocks, sequence flows, gateways, BPSim parameters, DMN decisions | what happens, in what order, and how long it takes when no distance is involved |
| **Layout** | paths, places (points and footprints), junctions, zones, levels; metres, `z` up | every length, distance, position and height |
| **Appearance** | parametric low-poly meshes by kind, materials, levels of detail | only the picture |

- Every layout object has a stable id (`conveyor-loop-3`, `chute-17`, `walk-zone-2`). A block
  refers to layout objects by id: a conveyor to its path, a server to its place, a transport
  pool to its walkway network and home. Appearance refers to layout objects the same way.
- **An unplaced block** is listed as "not placed" in the Layout view and drawn nowhere in 3D. It
  still runs; it just has no position. Nothing is guessed. So a transport cannot reach it: in a
  model with walkways, a transport that serves an unplaced block is a model problem ("place this
  block or stop serving it with a transport"), and the run does not start until it is fixed.
  Without walkways the straight-line fallback needs positions too, so the same rule holds.
- **Motion that takes no simulated time is never animated as travel.** A sequence flow is a
  hand-over at one instant, so an entity crossing it jumps. Only a path with a length and a speed
  shows movement.
- The run never reads the Appearance layer, so a look-only edit never marks results out of date
  (see "Tagged fields" below).

#### `Path`: geometry with exact lengths

- A `Path` is a list of segments in metres: `Line { length, rise }`, `Arc { radius, angle,
  rise }` (degrees, positive turns left) and `Helix { radius, rise, turns }` (`rise` is always the segment's total rise, for a helix over
  all its turns), each continuing from
  the end frame of the one before, from a start frame (x, y, z, heading). A `Line`'s `length` is its run on the plan; the path's `length()` is measured
  along the slope, so an incline is a line with a rise and a spiral is a helix. A clothoid
  segment can come later.
- `length()` and `point_at(s)` (position and heading at distance `s` from the start) are exact,
  worked out per segment, never from a flattened polyline. The same functions give the run its
  travel distance, the mesh its sweep and the 3D view each item's position.
- **What owns a path:** conveyors, walkways, roads, rails, AGV guide lines, queue lanes, pipes and
  walls.
- **Positions are a distance `s` along a path:** stations, diverts, merges, photo-eyes, operator
  work spots, doors in a wall. Moving a path moves everything on it.
- **Travel time is length over speed,** on a conveyor and on a walkway alike.
- **What changes:** `model::ConveyorShape` (`Straight`, `Curve` in degrees, `Spiral`) and the
  mesh module's own curve (in radians) fold into `Path`, with degrees at every public surface and
  one conversion inside. Old files with a `shape` attribute still read as a one-segment path.
- **Length becomes the path's** (D23). A conveyor may keep a typed `length` as an override; the
  form then shows it beside the drawn length with a warning, and the 3D view stretches nothing.
  An existing file whose `length` differs from its path length reads that `length` as the
  override, so no validated result moves on opening.

#### Conveyors: sections joined at junctions

- **A section** is one `Conveyor` block with one path, one speed and one kind. A line, a loop or
  a sorter is a graph of sections joined at junctions; the BPMN sequence flows between the blocks
  stay the logic links.
- **Points along a section, not cuts.** A divert, a merge or a station at `s` is an event point
  on its section, not a cut: items keep one position along the whole section and may span a
  point, so a 1 m item passes a station 0.5 m from the entrance exactly as on the plain
  conveyor. M5.1's runtime is extended so that an item's front reaching a point is an event
  (offer to the divert, start the station's work), and capacity and spacing are counted over
  the whole section. One non-accumulating belt still stops as a whole. Mixed item lengths change
  M5.1's pitch per item (rule 2 below), with their own checks.
- **Section kinds:**
  - *accumulating roller:* items stop one gap behind the item ahead (M5.1);
  - *non-accumulating belt:* the whole section stops when the front item cannot leave (M5.1);
  - *fixed-cell* (tilt-tray or cross-belt): cells of a set pitch move without stopping; an item
    loads only into an empty cell passing the induct point, and a cell whose divert is refused
    carries its item on round the loop. New.
- **Junction kinds,** each with a transfer time (0 by default):
  - *inline:* end to start, same heading; the item keeps its orientation;
  - *side transfer:* at an angle; the item turns, so its width becomes its length on the next
    section;
  - *spur or merge:* a section joins another part-way along, at `s`; it does not cut the main
    section in two;
  - *turntable:* any number of sections at any angle, with a turn time.
- **Rules, written down** (JaamSim's change log and users' forum questions are the checklist of
  what goes wrong when they are left vague):
  1. An item's reference point is its front, as in M5.1. It occupies `[s − its length, s]`.
  2. Spacing is a distance, never a time: an item stops `gap` behind the rear of the item ahead,
     whatever their lengths. Pitch p = length + gap holds for items of one length; mixed lengths
     use each item's own.
  3. Crossing a junction takes travel. When an item's front reaches the end of a section, the
     next section takes the front (after the junction's transfer time, 0 by default) and the
     rear follows: the item occupies both sections until its rear is across, moving at the
     lower of the two speeds and stopping if either section stops. Both sections count it while
     it spans the junction, so entry needs room on the next one. Positions never jump, and a
     path's travel time does not depend on how many sections it crosses.
  4. Entry, at the start or at a merge `s`, needs the stretch it will occupy plus the gap clear;
     otherwise the sender waits, first asked, first in.
  5. All items on a section share its speed. A speed change takes effect at the event that makes
     it, and every position stays piecewise linear in time, so the view interpolates exactly.
  6. No overtaking on a section.
  7. A stopped accumulating item moves again the moment the gap ahead opens. A restart delay is
     a later option.
  8. Sensors (photo-eyes) sit at `s`. They report the front arriving and the rear clearing, and
     "blocked for longer than t", and take no time.
- **A divert at `s`** is a point on a section with a rule:
  - a destination table (a DMN decision, or a plain table: destination to chute);
  - weights (probabilities);
  - shortest queue among the lanes allowed.
  The logic stays an exclusive gateway with M5.1's `stochcraft:routing="divert"`; the gateway is
  now placed at a divert point (D25). The rule chooses where the item is going; the route then
  takes the branches that lead there.
- **Lane full, stay on the loop.** Built in: when the chosen lane cannot take the item, it stays
  on the loop and is offered again next time round. Each item counts its rounds; after a set
  number it goes to an exception lane. Results report the recirculation count per divert and
  per item. In general-purpose tools this is usually built by the user from a photo-eye on
  the lane and a condition at the divert; built in, a sorter needs no scripting.
- **Default routing is the shortest path** over the section graph to the item's destination
  (a place id carried as an entity property). Explicit sequences and "avoid" lists come later.
- **What stays and what changes:**
  - stays: the `Conveyor` block and its five fields; divert routing (#197); the exact
    event-time runtime (#194, #195), extended with event points; `Live::conveyor_items` (#203), which
    gives each item's fraction of the belt and becomes `s` = fraction × path length;
  - changes: length from the path; points along a section (items may span them);
    one drive per belt section; sections join at junctions with transfer times; diverts and merges
    sit at `s`; mixed item lengths; fixed-cell is a new kind; recirculation counts and the
    exception lane are new; routing by destination is new;
  - the operators runtime (branch `operators-runtime`, M5.2 PR 2, not yet merged) adds
    `Live::operators`, each unit's progress from one element to the next. It becomes progress
    along a walkway path once units walk on one.

#### Operators on a walkway network

- **Walkways** are paths joined at nodes; a place connects to its nearest node through a work
  spot at its own `s`. Travel goes by the shortest route, at the pool's walking speed. Where a
  model has no walkways, the straight line between places stays (M5.2), flagged in the view.
- **A unit** has a home place, an empty and a loaded speed, and belongs to a pool. Its states are
  M5.2's: idle, travelling empty, loading, travelling loaded, unloading (and off shift). Each unit
  also reports its walking distance, empty and loaded.
- **A state table says what counts as busy,** written out in Help and in the results:

| State | Counts as |
|---|---|
| loading, unloading, travelling loaded | busy |
| travelling empty | busy (walking to work is work); shown apart in the state bar |
| idle | not busy |
| off shift | excluded from the share |

- **Zones:** a unit may serve only the places in its zone (S: four workers, ten chutes each).
  Zones are layout objects, so they are drawn.
- **Assignment tables** are data, not model edits: destination to chute, and worker to zone.
  An experiment can swap a table, which is how S tests re-assigning destinations.
- **Chute rules from S:** the nearest non-empty chute in the zone (S's baseline); and S's scored
  rule, which weighs each chute's distance against its remaining room with an exponent and moves
  only when the best chute's remaining room is below a threshold (S, Eq. 1). Both are built-in
  choices on the pool. A chute is a finite buffer at a place; a visit empties it, at a handling
  time per item, into a container beside it. That is a new kind of job: many items, no loaded
  walk. M5.2's `Transport` carries one item from one place to another, so the chute visit is
  added beside it.

#### Low-poly models by code

- **`Params` structs are the data.** Every object is a struct with documented defaults in
  metres, serde, and a `validate()` that says why a value is wrong ("radius 0.2 m is less than
  half the width 0.3 m"). A pure `build(params, lod)` makes triangles; the struct alone gives
  bounds, ports and path without making any.
- **Ports** are named frames with a kind and a size: conveyor end, road end, rail end, door, dock,
  work spot, pick point, floor. Connecting places the second object so its port meets the first,
  records the link, and copies width and height on if unset. The same ports drive snapping.
- **Arrays and fit-to-length:** linear, grid, polar and along-a-path arrays; and a one-dimensional
  split of a length into fixed, flexible and repeated parts. A rack of 12 m fits as many bays as
  it can and reports the slack. Walls get door and window openings by splitting, not by mesh
  booleans.
- **Three levels of detail:** full (legs, guards, rollers), simplified (merged boxes, rollers as
  a striped slab) and proxy (a box, slab or dot), chosen by the object's size on screen, with
  separate thresholds up and down so nothing flickers, under a triangle budget per frame.
- **A material slot per triangle** in `render::Mesh` (the one structural change it needs), mapped
  to a palette of about 16 semantic colours (floor, marking, wall, glass, steel, machine body,
  machine accent, conveyor frame, belt, safety yellow, safety red, wood, vehicle, person,
  highlight) plus the Okabe–Ito product colours. Themes map slots to colours. A per-instance tint
  carries product, state and selection.
- **The style kit,** all per face or per object, so the CPU renderer can afford it: a sky and
  ground ambient plus a wrapped key light; a darker band near the floor and a soft contact-shadow
  quad under every object; crease and outline strokes at full detail; back faces culled and
  objects sorted before their triangles; small chamfers on large objects only; a person and a
  1 m grid in every example scene for scale.
- **Three ways in, one data:** a Rust builder (`Conveyor::straight().length(6.0).width(0.6)`),
  a scene file (below), and the Python package (M6.1:
  `sc.conveyor.straight(length=6, width=0.6, at=(2, 0, 0), yaw=90)`). All three produce and read
  the same `Params` values; nothing is lost in a round trip.
- **Tagged fields.** Each field is tagged *sim* (the run reads it: a path, a station's place when
  a transport serves it, a chute's capacity) or *look* (colour, leg spacing, guard height, level
  of detail). Only sim fields enter the results baseline (D28).
- **The first catalogue,** about 30 objects, built in this order: `Path`, ports and the material
  slot; conveyors on paths; building shell; service furniture; roads and docks; the rest.

| Group | Objects |
|---|---|
| Manufacturing and industry | path conveyor (straight, curve, spiral as presets), transfer or divert unit, machine or workstation, buffer area, pallet rack, carton-flow shelf, tank, pallet, tote or carton, chute |
| Transport and logistics | road, junction piece (corner, T, cross), parking area, loading dock, truck or trailer, van, forklift, AGV or AMR, walkway or guide line |
| Buildings | floor or zone, wall with openings, door, window, stair or ramp, column grid |
| Business and service | counter or desk, seat row, queue barrier lane |
| People | operator (hi-vis, carrying, poses: stand, walk, carry), customer |

- **Later:** robot cell, rail, room kit, kiosk, tables; glTF export with each node's type and
  parameters in its `extras`, so a file opened elsewhere looks right and comes back parametric;
  glTF import of static scenery.

#### Where layout data lives

- **Everything is in the BPMN file, as `stochcraft:` extensions (D24).** No file of our own.
  Sim data: `stochcraft:path` on a conveyor (segment children; the old `shape`
  attribute still read), `stochcraft:place` on a block (x, y, z, heading, or a path id and `s`),
  walkways and junctions in `stochcraft:layout` on the definitions.
- **Scenery** (walls, floors, doors, racks that are not resources, parking, furniture) goes in a
  `stochcraft:scene` extension on the definitions: one element per object with its kind and only
  its non-default parameters. A TOML or JSON sidecar was weighed and dropped: the syntax would be
  standard, but its vocabulary would be ours, a new format in all but name. The extension is
  written for diffs: objects sorted by id, attributes in schema order, floats rounded to 0.1 mm
  or 0.01°, a schema version so a changed default is a migration. A scenario may carry
  overrides for it, as for inherited scenarios (#117). The run never reads scenery.
- **Exchange is glTF 2.0** (the Khronos standard): the scene exports as glTF with each node's
  kind, parameters and element id in `extras`, so any 3D tool shows it and StochCraft can read
  its own objects back. Import of plain glTF meshes as scenery comes later.
- `Model.layout` (the BPMN diagram bounds) stays the flowchart's layout and nothing else.

#### The Layout view and the 3D view

- **One canvas, three modes:** Process (the flowchart), Layout (a 2D plan) and 3D, switched by
  keys 1, 2 and 3 and a control on the canvas. Selection is shared: a block picked in one mode is
  picked in the others. The separate 3D tab goes.
- **The Layout view** is a top-down plan in metres:
  - a grid in metres with snap (a modifier key turns it off while dragging), a scale bar, the
    cursor's x and y in the status bar, zoom to fit;
  - a background image or DXF drawing, scaled and placed by picking two points a known distance
    apart; it is a tracing aid only and the run never reads it;
  - drawing and editing paths, places, junctions and zones; ports snap;
  - an "unplaced" list of blocks with no layout object, each one click from placing.
- **Flows on the plan:**
  - *before a run:* arrows from the routing probabilities and arrival rates the capacity table
    already works out, width by expected flow;
  - *after a run:* the observed flows, with counts;
  - *cost:* distance × quantity per pair and in total (M7.2's from-to chart, drawn here);
  - *variants:* the total against a saved baseline, as a percentage change.
  Flows with no route on the walkway or conveyor network are drawn as a dashed straight line, so
  the gap shows.
- **3D from the same data:** every mesh is built from the layout object's `Params` and placed by
  its frame; items are drawn at `point_at(s)` on their path; operators at their walkway position.
  Named cameras (overview, sorter, chutes) and "follow this operator". A presentation key hides
  the panels.
- **What happens to today's 3D code:** `view3d.rs` (#205, #208) stays as the renderer: the CPU
  projection, culling, painter's sort and egui meshes. Its scene builder is replaced: it reads
  the Layout instead of inferring from the diagram, and its own `METRES_PER_UNIT` goes. #210's
  edge-to-edge belts and loop ring were the last step of inference; once the layout's first PR
  lands, they are kept only until the sorter example has a drawn layout, then removed. D22's
  move to the shared wgpu viewport waits until the CPU renderer is measured short (D27).

#### The whole screen

From the research into how established tools lay out the window. The large changes are phased
(see the order below); each phase is usable on its own.

- **Left:** a searchable library with icons, grouped (flow, resources, conveyors and transport,
  layout and scenery, charts), with hover descriptions and F1 help; tabbed with a model tree
  that lists every object, including resources, calendars, experiments and charts, with rename
  and find.
- **Centre:** the main view (Process, Layout or 3D).
- **Right:** properties of the selection only, in collapsible groups, with values changed from
  the default in bold and a reset per field; units typed with the value; errors in the field.
  With nothing selected, the model's own settings. The experiment fields and the global
  connections table leave this panel.
- **A run bar always on screen:** run or pause (space), reset, step to the next event, speed as
  ×N with keys to halve or double it, "as fast as possible", the model clock with its unit,
  per cent done, "pause at" a time; the warm-up shown on the clock while it lasts.
- **The experiment as its own editor:** runs, seed, warm-up, duration, stop rules, and a scenario
  grid (scenarios as rows, changed inputs and key results as columns). "Run the experiment" always
  runs fast without animation; the run bar is for watching one run.
- **Connections on the canvas:** drag from a block's output handle to another's input, or a
  connect mode that chains clicks in flow order; a block's links shown as its own fields; a
  keyboard way to add and remove links, for accessibility.
- **A spreadsheet view per block type:** every server, conveyor, divert or chute in one grid,
  edited in place and selected in step with the canvas. DMN decision tables open here too.
- **During a run:** click a block for a card of live values (state, count inside, queue,
  utilisation, a sparkline); pin a value to the dashboard.
- **Results:** below the plain-language findings, a statistics table by block, measure and
  statistic, sortable and filterable, exportable as CSV, showing mean ± half-width, minimum and
  maximum; one run shows "insufficient" rather than an interval. For scenarios, a plot per
  scenario of the spread of outcomes (a box), the mean with its interval and each run as a dot
  (Nelson's MORE plot), beside the paired-difference forest plot.
- **What stays:** the plain-language findings first; out-of-date marking; the docked dashboard;
  the start page and guided first model; units in the model with rescaling; three themes with
  WCAG 2.2 AA contrast; keyboard use and screen-reader labels; 44 languages.

#### Outputs and the dashboard

- **A chart is a binding** (object, measure, kind), never custom code. Every block, path and pool
  exposes a uniform set of measures: count inside over time, state times, throughput, time in
  (as a histogram over entities), and for conveyors fill and blocked time. Kinds: tile, time
  series, histogram, state bar, bar per unit. Bindings are saved with the model and appear in
  the spreadsheet view.
- **Defaults per object type:** a sorter or line: output per hour; a conveyor: items on it over
  time; a lane or chute: fill over time; a pool: a stacked state bar per unit and distance
  walked; entities: a histogram of time in system.
- **For the parcel hub:** parcels per hour (tile and time series); per worker, distance walked
  and parcels handled (bars); worker imbalance (max − min across workers, the measure S uses);
  chute fill over time and full-chute events; conveyor load over time against its capacity;
  recirculations per hour and per item.
- A binding can be picked from the canvas (the pin on the live card) or from the model tree.

#### Automation and AI in the hub

Parcel hubs increasingly add sorting machines, robots and software that assigns work. Public
coverage of large new hubs speaks of advanced sortation and less manual handling, at headline
level only; nothing here describes what any particular hub's system does. What StochCraft adds
is a way to test such choices before they are built: *test the hub's AI and automation before
you build it.*

- **Robot stations** for unloading, singulation and picking: a server with a pick time and a
  success probability per attempt (a distribution across item classes, if given). A failed pick
  retries up to a set number of times, then goes to a manual station. Check with a known answer:
  with a constant pick time t and success p, at most k tries, throughput is 1 / (t × expected
  tries), and the manual share is (1 − p)^k.
- **Assignment policies as plug-in decision rules,** in three tiers, all behind one interface
  (destination to chute, worker to zone or chute, truck to door):
  1. a fixed table (data);
  2. a rule: a DMN decision table, or a built-in dynamic rule such as S's fill-threshold rule and
     periodic re-balancing;
  3. an external or learned policy, as an M6.3 logic plugin called at the decision point, with
     random numbers only from the engine's streams. Python (M6.1) trains it outside the run,
     exports it as a table or a plugin, and drives the experiments; no Python runs inside a run
     (D19).
  The experiment compares the tiers with paired runs on common random numbers and reports the
  differences with their intervals (M2.8, M2.9).
- **Forecast-driven inputs:** arrivals by hour of day (rates valid for calendar periods, M2.6)
  and staffing by shift (pool calendars) are scenario inputs, so "next peak's forecast with this
  roster" is a scenario row in the grid.
- **Autonomous vehicles and robotic pallet movers** are transports: a mobile pool (M5.2) on a
  road or aisle network with its own speeds, load and unload times. Vehicles that block each
  other on a path stay "later" (M5).

#### Decision models for sorting and classifying

- **DMN decision tables** (settled 2026-11-19; D30). DMN (OMG Decision Model and Notation,
  1.6, adopted 2026) is BPMN's sibling standard for decisions, so it fits "no new format": a `.dmn` file
  is read beside the BPMN, and other tools can open it.
  - A BPMN `businessRuleTask`, an exclusive gateway or a divert names a decision in the `.dmn`
    file through a `stochcraft:decision` vendor extension (file and decision id).
  - Inputs are entity properties (size, weight, postcode, service level); outputs are a class, a
    route or chute, or a priority, written back as entity properties. A gateway can route on an
    output directly.
  - First, decision tables with the hit policies UNIQUE, FIRST, PRIORITY and COLLECT, and a small
    subset of FEEL for input entries: comparisons, ranges (`[1..5]`), lists, `-` and `not(...)`.
    Literal expressions, decision requirement graphs and the rest of FEEL come later. FEEL is
    used only inside `.dmn` files; BPSim's XPath (D6) stays for everything else.
  - The same table runs in the simulation, is edited as a grid in the spreadsheet view, and is
    saved back as DMN. Validation reports overlapping rules under UNIQUE and inputs no rule
    covers.
  - Open DMN engines exist: DSNTK, in Rust, is MIT OR Apache-2.0. Whether to depend on it or
    write the small subset ourselves is measured in the first PR. The community DMN test kit
    (TCK) is a source of test files, fetched at a pinned commit as in D7 and D14 if its licence
    allows.
- **Classifiers with errors.** A classify step (a scanner, a camera, a model) assigns a class
  from a confusion matrix: for each true class, the share assigned to each class, plus a no-read
  share. Its draws come from its own keyed stream.
  - A wrong class leads to a missort: the wrong chute, then rework time and re-induction, or a
    manual exception lane. A no-read goes to a manual coding station.
  - Results then price accuracy: "what is 2% better recognition worth in recirculations,
    walking and manual handling?" is a sweep over the matrix.
  - Check with a known answer: with accuracy a for every class and no reads r, the missort share
    is (1 − r)(1 − a) and the manual share r, within the run's intervals.
- **The three tiers again:** a fixed table, a DMN or built-in rule, and a learned policy through
  the plugin API, compared with paired runs and intervals.

#### The flagship example: a parcel super hub

A hub sized from public figures, built from two published studies. Every number below is
labelled: **[S]** or **[B]** from the paper, **[derived]** our arithmetic from it, **[assumed]**
our choice, to be stated in the example's write-up (STRESS-DES).

- **Scale context.** A planned hub in Adelaide is reported to handle up to 400,000 parcels a day
  on 83,000 m² (Australia Post's announcement). Over about 20 operating hours that is about
  20,000 parcels an hour **[assumed hours, derived rate]**, some nine times the rate of S's
  segment. The example models one inbound side and one outbound segment, not the whole hub.
- **Inbound, after B:** a U-shaped hub with 8 inbound doors, 27 outbound doors and 4 nested loop
  conveyors **[B]**; 19 trucks of 650 parcels, 70% at the start and the rest within the first
  periods **[B]**; unloading at 13 parcels a minute per truck **[B]**. B gives no dimensions, so
  door pitch, conveyor lengths and speeds are **[assumed]**. B states the conveyor capacity as
  85 parcels a minute; its bounds and makespans fit 85 per 5-minute period **[derived]**, which
  the example uses, flagged. B gives no time a parcel spends on a conveyor; it is **[assumed]**.
  This layout is B's worked example (§6.1). B's test instances, used for the check below, are
  another set with other door counts, and B gives 27 outbound destinations in its text but 29 in
  its Table 3; the example follows the text, flagged.
- **Sorter and outbound, after S:** one sorter segment at 2.7 m/s with carriers at 0.72 m
  **[S]**, so 13,500 carriers an hour **[derived]**, modelled as fixed-cell **[assumed: S does
  not name the sorter type]**; 40 chutes of 40 parcels in four zones of ten, one worker each
  **[S]**; walking at 4 km/h **[S]**; one 8-hour shift without breaks **[S]**; about 20,000 and
  about 10,000 parcels a shift in the two load cases **[S]**; destinations assigned to chutes at
  random **[S]**. Missing from S and **[assumed]**: chute pitch and the walkway layout (they set
  every walking distance); handling time per parcel (about 4.3 s **[derived]** from S's worker
  1 at high load, if all non-walking time is handling); what a full chute does (recirculation,
  our choice); arrival pattern; destination shares; warm-up and seeds.
- **What we can check, per subsystem only** (no paper models the hub dock to dock):
  - S, Table III: at high load, idle time near zero for every worker; at half load, more walking
    per worker (16.4 to 16.9 km against 10.2 to 11.1 km) despite half the parcels **[S]**;
  - S, Table IV: the threshold rule cuts walking by about 7 to 13% at high load and about 70% at
    half load, with throughput within about 3% **[S]**;
  - S, Table V: re-assigning destinations to chutes cuts the mass imbalance across workers in
    every design, and the walking imbalance in all but one (at half load, the design tuned for
    throughput raises it by about 29%) **[S]**;
  - B, Table 5: on-time shares by policy, for example 79.0% first come first served against
    83.3% for B's best policy in the small instances **[B]**, rebuilt from B's published instance
    data with the assumed conveyor time.
  The checks are patterns and directions, not S's absolute metres, since S's layout is unknown.
  B's worked example in §6.1 is probably not one of its test instances and is not a check. The
  instance archives are still to open and check.
- **Licences:** B is CC BY 4.0 (its figures may be adapted with credit, recorded in
  ATTRIBUTION.md) and its instance data CC0. S carries its repository's standard licence: we cite
  its numbers and describe its rules, and copy no figure or text.

#### Checks with known answers

- **Paths:** an arc's length is √((r × θ)² + rise²) (θ in radians; r × θ when flat), and
  `point_at` measures distance along that slope; a helix's is √((2πr × turns)² + rise²), ending `rise` above its start; a
  line with a rise is √(run² + rise²); `point_at(length())` is the end frame, worked by hand for a
  line, a quarter arc and two helix turns.
- **Split stretches:** a drawn belt cut at a divert gives the same item times as M5.1's single
  conveyor when the divert sends nothing; one stopped stretch stops the whole belt.
- **Fixed-cell:** the most it carries is v / cell pitch items per unit of time (S's sorter:
  13,500 an hour); from a faster source the induct waits.
- **Recirculation:** with one lane always full, each item's time grows by the loop length over
  the speed per round, and after the set number of rounds it is in the exception lane.
- **Walkways:** a shortest route on a five-node walkway, worked by hand; a unit's walking
  distance equals the sum of its routes.
- **DMN:** one table per hit policy (UNIQUE, FIRST, PRIORITY, COLLECT) with inputs whose outputs
  are worked by hand, and an overlapping UNIQUE table reported as invalid.
- **3D and plan:** every item is drawn at the same `point_at(s)` in both views.

#### Order of work

Small PRs, each visible on screen. Sizes are estimates: S one or two days, M three to five. Rows
marked "series" are several PRs of that size, each with its own visible step.

| # | PR | Needs | Size | What the user sees |
|---|---|---|---|---|
| L1 | `Path` in `model` (segments, `length`, `point_at`; `ConveyorShape` folded in, degrees everywhere) and the read-only Layout view: grid, scale bar, paths and places to scale, the unplaced list | – | M | a 2D plan |
| L2 | Files: `stochcraft:path` and `stochcraft:place`, `stochcraft:scene`, glTF export, sim and look tags in the baseline; length from the path with the flagged override (D23, D24, D28) | L1 | M | the override warning |
| L3 | 3D fed from the Layout; #210's inference removed once the sorter is drawn | L1, L2 | M | the sorter as drawn |
| L4 | Editing the layout: draw and drag paths, then snap and ports, then the background by two points | L1, U1, U2 | series of 3 × M | drawing a layout |
| L5 | Splitting at points, junctions, divert at `s`; then recirculation and the exception lane; then shortest-path routing | L2 | series of 3 × M | the loop that recirculates |
| L6 | Fixed-cell sections | L5 | M | a tilt-tray sorter |
| L7 | Walkways and walking distance; then zones, assignment tables, the chute visit and S's rules (after `operators-runtime` merges) | L2, M5.2 | series of 2 × M | workers walking their zone |
| L8 | Material slot, palette, style kit, levels of detail; then the catalogue in waves | L3 | M, then S per wave | lit scenes with shadows |
| L9 | Flows on the plan before and after a run, distance × quantity, variants (with M7.2) | L1 | M | flow arrows |
| L10 | DMN decision tables: read, evaluate, and use from a gateway, a divert and a business rule task; the table grid | – | M | a routing table |
| L11 | Classify step with a confusion matrix; robot station with retries | L10 | M | missorts and manual share |
| L12 | The parcel hub example, outbound (S), then inbound (B), each with its write-up | L5–L7, L10 | series of 2 × M | the flagship |

The whole screen goes alongside as its own series (D26), each one PR: U1 the run bar (M); U2
properties of the selection, with bold and reset (M); U3 the library and model tree (M; with #150: a Library | Outline switch, the library grouped,
searchable and with glyphs, add by click, Enter or drag, and larger blocks); U4
connecting on the canvas (M); U5 the experiment editor and scenario grid (M, perhaps two); U6 the
spreadsheet view (M); U7 the live card and pins (S); U8 the statistics table and the scenario
plot (M). U1 and U2 come before L4, since layout editing leans on them; the rest interleave.

**Status:** L1 is done (#216): `model::Path` with exact `length` and `point_at`, and the read-only
Layout view with its grid, scale bar, conveyors and footprints to scale, and the unplaced list.
A conveyor still stores its `ConveyorShape`, which gives its `Path`; folding the shape into the
path waits for L2's file format, and until then the Layout view places blocks from the diagram,
as the 3D view does. U1, the run bar, is done (#218).

#### Sources

- S: H. Steinkellner, D. Kaever, C. Rockenschaub, "Enhancing Social Sustainability in Parcel Hubs
  using Discrete Event Simulation and Multi-Objective Optimization", IMHRC 2025, paper 28:
  <https://digitalcommons.georgiasouthern.edu/pmhr_2025/28>
- B: S. Bugow, C. Kellenbrink, "The parcel hub scheduling problem with limited conveyor capacity
  and controllable unloading speeds", *OR Spectrum* 45:325–357, 2023, CC BY 4.0:
  <https://doi.org/10.1007/s00291-022-00702-y>; instance data, CC0:
  <https://doi.org/10.25835/0090537>
- Scale figures: Australia Post newsroom, 16 October 2025:
  <https://newsroom.auspost.com.au/australia-post-to-build-500-million-parcel-super-hub-for-south-australia>
- JaamSim User Manual (2021-04) and Release Notes (2026-08), Apache-2.0 project:
  <https://jaamsim.com/docs/JaamSim%20User%20Manual%202021-04.pdf>,
  <https://jaamsim.com/docs/JaamSim%20Release%20Notes%202026-08.pdf>
- Salabim manual, trajectories and monitors: <https://www.salabim.org/manual/Trajectory.html>,
  <https://www.salabim.org/manual/Monitor.html>
- openTCS user guide (CC BY 4.0), plant model reference:
  <https://github.com/openTCS/opentcs/blob/master/opentcs-documentation/src/docs/asciidoc/users-guide-03_reference.adoc>
- Casymda (BPMN to SimPy, with a tile map for movement): <https://fladdimir.github.io/post/casymda>
- ASAM OpenDRIVE 1.8 (reference line, `s` along it):
  <https://publications.pages.asam.net/standards/ASAM_OpenDRIVE/ASAM_OpenDRIVE_Specification/1.8.0/>
- build123d joints (ports as frames): <https://build123d.readthedocs.io/en/stable/joints.html>
- P. Müller et al., "Procedural Modeling of Buildings", *ACM Transactions on Graphics* 25(3),
  2006 (split and repeat): <https://doi.org/10.1145/1141911.1141931>
- glTF 2.0 (metres, instancing, `extras`): <https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>
- B. L. Nelson, "The MORE plot: displaying measures of risk & error from simulation output",
  Proc. Winter Simulation Conference 2008, pp. 413–416: <https://doi.org/10.1109/WSC.2008.4736095>
- OMG DMN (1.6, adopted 2026): <https://www.omg.org/spec/DMN>; DSNTK: <https://github.com/DecisionToolkit/dsntk>

## M6: Python and plugins

**Outcome:** people who already work in Python script StochCraft from Python, and anyone can
extend it with plugins, without forking it, while every run stays reproducible.

Python is how many simulation users already work (SimPy, numpy, pandas, notebooks), and the
Craft apps already extend themselves with sandboxed plugins (FigCraft and PhotoCraft: WebAssembly
modules run in `wasmi`, acting through the app's commands). StochCraft takes both, in this order:
the Python package first, since it only wraps the engine (M1.4); command plugins after M3.7;
logic plugins with or after M4.3, which needs the same calls into a running model.

### M6.1 The Python package (settled 2026-11-16)

- `pip install stochcraft`: the Rust engine compiled into a Python module with PyO3 and maturin,
  as abi3 wheels for Windows, macOS (Apple silicon and Intel) and Linux (x86-64 and ARM64), built
  and published by the release workflow. No Python inside the engine, so results, seeds and
  speed are the same as in the app and the command line.
- **What a script can do:** open, build and save models (BPMN and XPDL with BPSim, so the app
  opens what a script saves); every engine command, with its JSON schema, as a method; run an
  experiment and get the results as a table (a pandas DataFrame when pandas is installed, plain
  lists otherwise) with the means, half-widths and the run record; sweeps, sensitivity, sizing
  and paired comparisons; the per-replication values for the user's own statistics and plots.
- **Shape:** a thin, typed Python layer over the engine (`stochcraft.Model`, `model.run(...)`,
  `results.table()`), with type stubs and docstrings generated from the command schemas, so
  editors and AI assistants complete it; examples as notebooks.
- **The same rules as the rest:** no hidden randomness (a seed is part of every run record);
  long runs release Python's lock and use every core, as the CLI does; errors are Python
  exceptions carrying the engine's message and code.
- **Not in M6.1:** writing a block's logic in Python and calling it during a run. That would be
  slow, would not run in the browser and would break reproducibility across machines; custom
  logic is M6.3's job.
- **Checks:** the Python tests run every example of the user guide and compare a run's results
  with `stochcraft-cli run` on the same model and seed, value for value.

### M6.2 Command plugins

As in FigCraft:

- A plugin is a WebAssembly module with a JSON manifest, run in `wasmi` (a pure-Rust interpreter,
  MIT OR Apache-2.0) in the desktop app and the browser alike, with no WASI: no file system,
  network, clock or randomness of its own. A run has an instruction budget and a memory cap.
- It acts only through the engine's commands, within the permissions its manifest asks for (read
  only, or edit; optionally a list of commands). Everything one run does is one undo step, named
  after the plugin; a failed run puts the model back as it was.
- Uses: building a model from a company's data, checks of a company's own modelling rules,
  custom reports and charts from results, importers for other formats.
- In the window: Plugins in the menu, a dialog generated from the plugin's parameters, Manage
  plugins (install from a file, remove); later a plugin directory, as FigCraft's.
- A Rust SDK and example plugins in `plugins/`; other languages that compile to WebAssembly work
  too.

### M6.3 Logic plugins

Custom behaviour inside a run, the logic SimPy users write by hand:

- **Hooks:** a routing choice (which way an entity goes), a time (a service, a setup or a delay
  computed from the entity and the model's state), a condition, and later a block of its own
  with its own ports and statistics.
- **Deterministic:** each call gets the entity's properties, the time and the state it asks for,
  and random numbers only from a stream the engine gives it (keyed by the block and the hook, as
  since #152), so the same seed gives the same results on every machine and in the browser.
- **Bounded:** an instruction budget per call and a memory cap, as in M6.2, which also applies
  when a module is instantiated (the memory it declares counts); running out of either stops the
  replication with an error that names the plugin and the block.
- **In the file:** a block names the plugin and its version through a `stochcraft:` vendor
  extension; a model whose plugin is missing opens, and says what it cannot run.
- **Speed:** `wasmi` interprets; a hook is called once per event, so the cost is measured on the
  benchmark models before this ships, and a compiled runtime is considered only if it is too slow
  and still deterministic.

## M7: industrial engineering

**Outcome:** what industrial engineers and students ask a free factory-simulation tool for
(the request that prompted this was to replace commercial factory-simulation and
layout-planning tools for coursework and an engineering portfolio) can be done in StochCraft,
in the browser as well as on the desktop.

Where each part of that request sits:

- **Process simulation:** M2, done (servers, pools, routing, batches, buffers, breakdowns,
  calendars, changeovers, lean measures), with the capacity table before a run (M2.9).
- **Layout and material flow:** M5, led by the sorter; the from-to chart below adds flows and
  distances on the layout.
- **Watching a model run:** M3.7.7 (#151) on the flowchart, then M5.3's layout view, 2D and 3D.
- **Line balancing and supply chains:** new, below.
- **Learning and a portfolio:** free and open, in the browser at
  [helenkwok.github.io/StochCraft](https://helenkwok.github.io/StochCraft/); course-ready models
  ("Examples for teaching"); sharing a model and its results, below.

### M7.1 Line balancing

- **Status:** the core is done in `model::balance`: the three heuristics, the measures, the
  theoretical minimum, validation and the ranked positional weights, with hand-worked and
  randomised checks; Groover's Examples 15.1, 15.2 and 15.4 checked
  (`model/src/balance/tests.rs`); the engine command `line.balance` and `stochcraft-cli balance`
  (#192). Not yet: the window view and "Build the line".
- **Outcome:** given the work elements of an assembly, their times and which must come before
  which, a station assignment for a cycle time from each heuristic below, with the theoretical
  minimum beside it so any gap shows (the heuristics do not promise the fewest stations: times
  6, 5, 3, 2, 2 and 2 with a cycle of 10 take three stations by the largest candidate rule,
  where two suffice, 6 + 2 + 2 and 5 + 3 + 2; exact methods come later), and the line built as
  a model to run with variability.
- **Input:** a precedence graph of work elements with times; the cycle time is the takt of the
  line's demand (M2.11's `stochcraft:demand` and `takt`, done) or a number given.
- **Heuristics:** the largest candidate rule, Kilbridge and Wester's column method and Helgeson
  and Birnie's ranked positional weights (Groover, *Automation, Production Systems, and
  Computer-Integrated Manufacturing*, 4th ed., Pearson, 2015, ISBN 978-0-13-349961-2, ch. 15
  "Manual Assembly Lines": Example 15.1 and the measures in §15.2, pp. 403–405, the methods in
  §15.3 "Line Balancing Algorithms", pp. 405–411; M. D. Kilbridge and
  L. Wester, "A heuristic method of assembly line balancing", *Journal of Industrial
  Engineering* 12(4):292–298, 1961 (Groover's reference list gives No. 6, other listings No. 4;
  the pages fit No. 4); W. B.
  Helgeson and D. P. Birnie, "Assembly line balancing using the ranked positional weight
  technique", *Journal of Industrial Engineering* 12(6):394–398, 1961). Exact methods (A. Scholl,
  *Balancing and Sequencing of Assembly Lines*, 2nd rev. ed., Physica-Verlag, 1999, ISBN
  978-3-7908-1180-3) later.
- **Measures:** the theoretical least number of stations (the total work content over the cycle
  time, rounded up; Groover eq. 15.7), balance efficiency (the work content over the number of
  stations times the longest station time, eq. 15.14), balance delay (its complement, eq.
  15.15), and the smoothness index (the square root of the sum of squared differences between
  the longest station time and each station's; not in Groover, used as in Bhattacharjee et al.'s
  comparison, *Journal of Modelling in Management*, 2018, doi 10.1108/JM2-03-2017-0027).
- **Then simulate it:** "Build the line" writes the balanced stations as servers in series
  (with buffers if asked), so the capacity table and a run show what the averages hide:
  blocking and starving when element times vary.
- **Where it lives:** a `balance` module in `model` (no run needed), an engine command
  `line.balance`, `stochcraft-cli balance`, and a view in the window; the precedence graph is a
  `stochcraft:workElements` vendor extension on the line's tasks.
- **Checks:** Groover's Example 15.1 (Table 15.4: twelve elements, 4.0 min of work, a service
  time of 1.00 min): the theoretical minimum is 4 stations; the largest candidate rule
  (Example 15.2, Table 15.6) gives 5 stations with times 1.0, 0.81, 0.98, 0.59 and 0.62 min and
  a balance efficiency of 0.80; Kilbridge and Wester (Example 15.3) gives 5 stations at 0.80;
  ranked positional weights (Example 15.4, Tables 15.9 and 15.10) gives the weights listed
  there (element 1: 3.30 … element 12: 0.12) and 5 stations with times 0.90, 0.91, 0.92, 0.65
  and 0.62 min, 0.87 at the longest station's 0.92 min. Always: the stations never break
  precedence and never exceed the service time, and the theoretical minimum is never beaten.

### M7.2 Material flow on the layout

- **Outcome:** how much moves between which stations, how far, and what that costs, drawn on the
  layout, so that layouts can be compared.
- **From-to chart:** the flow from each station to each other, from the routing rates the
  capacity table already works out (before a run) or from a run's counts; distances between
  stations from M5.5's layout (straight line, rectilinear, or along the walkways and paths), or,
  for a model with no layout, from the diagram's centres times `layout_scale` (flagged); flow × distance per pair and in total.
- **On the canvas:** a flow diagram drawn on the layout, line width by flow. M5.5's Layout view
  draws it (flows before and after a run, variants against a baseline); M7.2 adds the chart and
  its measures.
- **Later:** the activity relationship chart of Systematic Layout Planning (Muther, *Systematic
  Layout Planning*, Industrial Education Institute, 1961; 2nd ed., Cahners Books, 1973);
  Tompkins, White, Bozer and Tanchoco, *Facilities Planning*, 4th ed., Wiley, 2010, ISBN
  978-0-470-44404-7, for the measures.
- **Checks:** a three-station example worked by hand (flows, rectilinear distances, total
  flow × distance); the before-a-run chart and a long run's counts agree within the run's
  intervals.

### M7.3 Inventory and the supply chain

- **Outcome:** stock points that order from a supplier or an upstream stock point by a policy,
  with lead times, so that a model can reach from the factory through the supply chain.
- **Policies:** base stock, (s, S) and (R, Q) (reorder point and order quantity), reviewed
  continuously or every period; unmet demand backordered or lost.
- **Results:** fill rate, service level (cycles without a stockout), mean stock on hand,
  backorders, orders placed.
- **Where it lives:** a new block kind, the stock point, with a `stochcraft:inventory` vendor
  extension (BPMN has no stock), and its demand from a source or from downstream orders.
  Multi-echelon chains later.
- **Checks**, continuous review, Poisson demand of single units, a fixed lead time L, one
  order per unit taken (base stock S):
  - demand not met at once is backordered: an order is filled at once when lead-time demand is
    at most S − 1, so the fill rate is that Poisson probability with mean λL (Hopp and
    Spearman, *Factory Physics*, 3rd ed., 2008, §2.4.2 "The Base Stock Model");
  - demand not met at once is lost: the units on order behave as an Erlang loss system with S
    servers (Palm's theorem), so the fill rate is 1 − B(S, λL), Erlang's loss formula; for
    S = 1 and λL = 1 it is 1/2, not the e⁻¹ of the backorder case;
  - (Q, r) with backorders against the same book's §2.4.3 "The (Q, r) Model".

### M7.4 Sharing a model

- **Outcome:** a student or an engineer sends one link that opens a model, and its results if
  saved with it, in the browser app, and prints a one-page report of the question, the model and
  the findings.
- **Scope:** small: a model (and optionally its results) passed in the link or as a file the
  link points to; the report from M3.7's findings. Nothing is stored on a server of ours.
- **Every file the model needs travels with it.** A model that names decisions in a `.dmn` file
  (M5.5) is shared as a bundle of the BPMN and each `.dmn` it references. If a referenced file
  cannot be found, sharing stops with a message naming the missing file and the task, gateway or
  divert that uses it, rather than sending a model that cannot run.

### Order

See D21: the sorter (M5) first, alongside M3.7; then line balancing (M7.1), the from-to chart
(M7.2, which needs M5's layout scale), inventory (M7.3) and sharing (M7.4).

---

## Examples for teaching

A goal rather than a milestone: course-ready example models that a teacher can hand out as they
are.

- **Each example comes with:**
  - its source;
  - a checked answer: an exact formula, or the published result, checked against the source
    with its page;
  - a write-up following STRESS-DES, one of the three 20-item reporting checklists of Monks,
    Currie, Onggo, Robinson, Kunc and Taylor, *Strengthening the reporting of empirical
    simulation studies: Introducing the STRESS guidelines*, Journal of Simulation 13(1):55–67,
    2019, doi 10.1080/17477778.2018.1442155.
- **Candidates from the common textbooks** (Banks et al., Law, Robinson) join only once their
  parameters are taken from the books themselves.
- **Examples:** six more on the start page, each a file in `tests/models/` whose answer
  `crates/engine/tests/examples.rs` checks within √2 times its 95% half-width, with comparison
  scenarios inheriting the first. No STRESS-DES write-ups yet.
  - **Bank with an ATM and cashiers** (`bank.bpmn`): the parameters of Borshchev and Grigoryev,
    *The Big Book of Simulation Modeling*, ch. 2, Example 2.2: 45 customers an hour, half to the
    ATM (triangular 1, 2, 4 minutes), 30% of those then to the cashiers, the other half straight
    to five cashiers with one queue (triangular 3, 5, 20). The book says "an ATM", so the model
    has one. By flow balance the cashiers are busy 0.91 (0.758 with a sixth, a scenario) and the
    ATM 0.875; the ATM, M/G/1, waits 8.75 minutes by Pollaczek–Khinchine, with 3.28 waiting. The
    cashiers' wait (M/G/5) has no closed form and is not checked.
  - **Hospital beds** (`beds.bpmn`): Erlang B, exact for M/G/c/c whatever the length of stay.
    Four admissions a day and stays of 5 days on average (log-normal, standard deviation 4), no
    waiting room: 24 beds turn away 6.61%, the same with exponential stays of that mean (a
    scenario), and 28 beds 1.88% (a scenario), the fewest under 1 in 50.
  - **Machine repair crew** (`machine-repair.bpmn`): M2.5's M/M/2//6 (λ = 0.25, μ = 1; fitters
    busy 0.57642, 4.61138 machines running), with one fitter (0.88284, 3.53135) and three
    (0.39814, 4.77763) as scenarios.
  - **Inspection with rework** (`rework.bpmn`): M2.3's Jackson network (λ = 1, μ = 4 at
    inspection and 2 at rework, half fed back: 2 minutes in the system), and a quarter fed back
    as a scenario (0.7 minutes).
  - **Project finish date** (`project.bpmn`): three exponential activities of mean 10 days in
    parallel, a terminating run (M2.8); the finish is their maximum: 18.33 days on average, by
    day 30 with chance 0.858. Its card says "Checked against probability theory".
  - **Same mean, different spread** (`spread.bpmn`): three M/G/1 desks at ρ = 0.8 with a mean
    service time of 1 minute, fixed, exponential and gamma (shape 0.25): Pollaczek–Khinchine's
    waits of 2, 4 and 10 minutes. It is the featured lesson on variability; "Every distribution"
    stays as a reference model.
- **Examples, second batch:** four more, checked the same way. Only exact checks are claimed;
  what a model shows beyond them is said to be unchecked on its card.
  - **Polling place** (`polling.bpmn`, featured, a new "Public services" industry): T. T.
    Allen, *Introduction to Discrete Event Simulation and Agent-based Modeling*, Springer 2011,
    ch. 6 and 7. From the book: ballots of 19 initiatives take 12.5 minutes on average and of 8
    initiatives 6.5 (§7.3.3), the day is 13 hours (§7.3.4), and machines are allocated by its
    utilisation rule, roundup(ballot minutes × voters / (ρ₀ × 13 h × 60)) with ρ₀ = 0.85 to
    start (§7.3.5). Deduced, not printed: 72 voters an hour, from about half a million voters at
    532 locations (§7.3.1; 72.3 an hour), so 936 in a day. The rule gives 18 machines for the
    long ballot and 10 for the short. The voting times are exponential, as Erlang C and the
    Kolesar–Green rule of §6.2 assume; the study used truncated normal times whose spread the
    book does not give. Checked against Erlang C: 18 machines busy 0.833, a wait of 1.506
    minutes and 1.807 waiting; 16 machines (a scenario) 0.9375, 9.126 and 10.95; the short
    ballot with 10 (a scenario) 0.78, 1.081 and 1.298. Two scenarios run election day: 936
    voters (a source with a limit, so the last arrives at 13 hours only on average, give or
    take about 25 minutes) and a stop when all have voted, with 18 machines and with the 14 that
    allocating by voters alone gives (28 shared by two places of the same size). With v0.2.0 the
    last voter left after 822 and 885 minutes and the mean waits were 1.5 and 34.8 minutes; the
    test asserts only that 14 machines are later and wait longer. §6.1's example says a
    1,000-hour period but works its arrival rate (1.667 a minute) from 100 hours.
  - **Mass vaccination hub** (`vaccination-hub.bpmn`): Hanly, Churches, Fitzgerald, Caterson,
    MacIntyre and Jorm, *BMC Health Services Research* 22:1059 (2022), doi
    10.1186/s12913-022-08447-8, open access; every input checked against the paper's own
    tables. Table 1: each time a minimum plus an exponential of rate λ (preparation 1 + λ 3,
    entrance 2 + λ 1, registration 3 + λ 0.7, assessment 2 + λ 1, vaccination 3 + λ 1), and
    observation normal (20, 0.5) or, for the 2% with a reaction, 20 + λ 0.1. Table 2: 60
    bookings an hour at the small hub, 180 at the large; 2% do not come (Methods, "Arrival
    times"). Table 3: staff 2, 4, 6, 4 and 5 with 25 observation seats, and three times as many.
    BPSim has no shifted distributions, so each time is a continuous user distribution through
    its quantiles (every 2%, then the 99th, 99.5th, 99.9th and 99.99th percentiles, the way the
    three-point estimates write a shifted log-normal), the observation time the mixture's; their
    means are within 0.1% of the paper's, and the check uses them as written. Each patient's
    dose is prepared alongside (a parallel split and join before vaccination). Checked by flow
    balance, exact whatever the queueing: each station busy λ E[S] / c (small hub 0.654, 0.736,
    0.724, 0.736, 0.784 and 0.792 for preparation to observation, all under the paper's 80%, the
    same at the large hub, a scenario; with 70 bookings an hour, a scenario, vaccination 0.915),
    and patients out at λ: 470.4 in 8 hours (the paper's "around 500", Results and Fig. 3) and
    1,411.2 at the large hub ("1400"). Not checked: the paper's median time in the hub of 52
    minutes (95% within 67) and its surge results (109 and 55 minutes, Fig. 4). Its arrival
    noise around the hourly slots and its walking times between stations are not in the paper or
    its supplement, so arrivals here are Poisson and there is no walking; with v0.2.0 the mean
    time inside was 36.7 minutes at the small hub. Table 1's hub row for observation prints
    percentiles (19.8 to 20.2 for the 5th to 95th) that fit a standard deviation near 0.12, not
    its 0.5; the clinic row (19.2 to 20.8) fits 0.5, which the model uses.
  - **Prison population** (`prison.bpmn`, "Public services"): Brailsford, Churilov and
    Dangerfield, *Discrete-Event Simulation and System Dynamics for Management Decision Making*,
    Wiley 2014, ch. 8 §8.3.1 (the case of Tako and Robinson after Grove, MacLeod and Godfrey
    1998): 3,000 petty and 650 serious first offenders a year, sentences of 5 and 20 years on
    average, reoffenders back after 2 years on average, and 50,000 petty and 26,000 serious
    offenders inside. The share who reoffend is deliberately not given. **Deduced:** if those
    numbers are the equilibrium, it is 0.70 for petty offenders and 0.50 for serious ones, with
    14,000 and 1,300 out before returning. The model is scaled down 100 times (30 and 6.5 first
    offences a year; 500, 140, 260 and 13), which keeps all four counts whole and a test run
    short. Every stay is a delay with no limit, so in steady state each number inside is Poisson
    with mean (arrivals including returns) × mean stay, whatever the stays' distribution. The
    stays are exponential and the run starts with those means inside (starting contents), so the
    expected numbers stay there from time 0 and the base scenario has no warm-up. With 60% of
    petty offenders reoffending (a scenario), 375 inside and 90 out, reached after a 100-year
    warm-up (the slowest time constant is then 15.9 years). From empty with a 300-year warm-up,
    v0.2.0 gave 501.3 ± 3.6, 140.5 ± 1.3, 262.0 ± 4.8 and 13.36 ± 0.36, and 374.9 ± 3.2 and
    90.6 ± 1.0 for the scenario. Fixed sentences would give the same long-run means but release
    each starting cohort together, so they are left out.
  - **Penny Fab** (`penny-fab.bpmn`): W. J. Hopp and M. L. Spearman, *Factory Physics*, ch. 7:
    four stations in a line, one machine each, 2 hours a job, so r_b = 0.5 a hour, T₀ = 8
    hours and W₀ = 4. The w jobs start at the first station (starting contents) and go round
    (a closed loop, #237). With exponential times the line is the practical worst case, exact
    for a balanced closed line of exponential stations: throughput r_b w / (W₀ + w − 1), each
    station busy twice that, cycle time T₀ + (w − 1) / r_b and, by symmetry, a wait of
    (w − 1) / 2 hours at each station. Checked for 1, 4, 8 and 16 jobs (0.125, 0.286, 0.364
    and 0.421 jobs an hour), and for 4 jobs with fixed times, the best case: every station
    always busy, 0.5 jobs an hour. A v0.2.0 run of the same line fed by a source of w jobs at
    time 0 matched each (4 jobs: busy 0.5714 ± 0.0021 against 0.5714).
  - Not built in this batch: **the daily peak** (arrivals rising and falling through the day,
    with an exact mean number inside for infinite servers).
- **Printed answers are checked, not copied.** T. T. Allen, *Introduction to Discrete Event
  Simulation and Agent-based Modeling*, Springer 2011: §13.6 answers §6.6's problems 5
  (λ = 0.7, μ = 0.1, c = 8) and 7 (λ = 0.8, μ = 0.1, c = 9) with a chance of waiting of 0.875
  and 0.889, which is ρ, and waits of 38.1 and 39.2 minutes, six times Erlang C's 6.35 and 6.53
  (the chance of waiting is 0.635 and 0.653). The worked example of §6.4 (λ = 0.5, μ = 0.1,
  c = 6) also gives ρ, 0.83333, as the chance of waiting, against Erlang C's 0.5875; its queue
  length and wait (2.93 and 5.9 minutes) are right.
- Done in `bench/` (#183), as the program there rather than a `cargo xtask bench` command:
  **a speed benchmark** (`cargo run --release --manifest-path bench/Cargo.toml`, see
  [bench/README.md](bench/README.md)): events per second
  on M/M/1, tandem lines of 5 to 200 stations, and the DEVStone models LI, HI and HO at set
  widths and depths (Wainer, Glinsky and Gutierrez-Alcaraz, Simulation 87(7), 2011; Cárdenas
  et al., ACM TOMACS 2022), whose event counts are known in closed form and so also check the
  run. Times aggregated as Herguedas-Pinedo et al. propose (ANNSIM'25): a geometric mean of
  ln(1 + t) weighted by each model's order of complexity. Comparable with the 3.0–3.3 million
  events per second reported for M/M/1 in arXiv 2611.10602, and with SimPy. DEVStone is
  implemented from the papers; no DEVS engine's code is read.
- **Cross-checks against open-source simulators:** SimPy (already compared on M/M/1 in `bench/`)
  and Salabim. Salabim
  is MIT, but its licence text is `salabim/LICENSE.txt`, not at the repository's root, and the
  package also ships font files (`calibri.ttf` among them) whose own licences are to verify.
  Its models are described and reimplemented, and none of its assets are ever copied.

---

## After M3: credibility and digital twins

Not scheduled yet; recorded so earlier work does not close these doors (docs/research.md):

- **Credibility reports:** `stochcraft-cli report`, a results assessment in NASA-STD-7009B's
  terms from the run records, sensitivity records and diagnostics. Models can declare validated
  ranges for their parameters, and results from outside them are placarded with the limit
  exceeded and by how much ([M&S 26]).
- **Digital twins:** starting a run from an observed state (queue contents, busy units, elapsed
  times) rather than empty; fitting input distributions from event logs; the engine's journal as
  the audit trail of what-if decisions (it records its version and the state it started
  from, #44).
- **Industry 5.0 measures:** workers as resources with skills and shifts; energy and CO₂ per
  part and per element (BPSim cost parameters plus a vendor extension); disruption scenarios.
- **Bulk flow:** rates, splits by fraction, tanks and batch treatment for process industries,
  built on M4's exact level-crossing events (D12) rather than on `sd`'s fixed steps.
- **Explanations:** agents over MCP describe results in plain language (the bottleneck, what
  limits throughput, which inputs matter most).
- Not planned: parallelising a single run (optimistic Time Warp). Replications in parallel
  already use every core for the models StochCraft targets.

## Order of work

```
M1.1 model ─ M1.2 compile ─ M1.3 io ─ M1.4 engine ─ M1.5 cli ─ M1.6 determinism
                                          │
                     ┌────────────────────┴────────────────────┐
              M2.1 primitives                            M3.1 shell, web, Pages
              M2.2 properties, expressions               M3.2 editing
              M2.3 … M2.8 (each one PR)                  M3.3 results
              M2.9 output analysis                       M3.4 canvas
              M2.10 abandonment                          M3.5 live view
              M2.11 changeovers, lean measures           M3.6 workers + MCP
                     │                                         │
                     └──────────── new blocks appear ──────────┘
                                   in M3 forms
                                         │
                     ┌───────────────────┴───────────────────┐
              M4 Monte Carlo, SD, agents, hybrid      M5 transport and layout
                                                      (the sorter: M5.1 conveyors ─ M5.2 operators
                                                       ─ M5.3 layout view, 2D then 3D ─ M5.4 dashboard;
                                                       started alongside M3.7, D21)
                                                      M5.5 layout layer, Layout view, generated 3D,
                                                       DMN (L1 … L12), with the screen series U1 … U8
                                                       (D26)
                                                              │
                                                      M7.1 line balancing ─ M7.2 from-to chart
                                                      ─ M7.3 inventory ─ M7.4 sharing

M6.1 the Python package: any time after M1.4 (it wraps the engine)
M6.2 command plugins: after M3.7     M6.3 logic plugins: with or after M4.3
```

Examples for teaching are added alongside, as each feature they need lands.

M1 comes first because both M2 and M3 build on the model document and the engine. After M1, M2
and M3 proceed in parallel, as two series of pull requests open at the same time: M2 changes
`des`, `model`, `blocks` and the engine's commands; M3 changes `ui-egui` and the apps, so they
rarely touch the same files. Each M2 block becomes editable in M3 as soon as its schema lands,
and the Pages preview of an M3 pull request shows the blocks already merged.

## Decisions to confirm

| # | Question | Recommendation |
|---|---|---|
| D1 | Model file format | No new format: BPSim 2.0 embedded in BPMN 2.0 (`.bpmn`, for new models) or XPDL 2.2 (read and kept); StochCraft-only data as BPSim vendor extensions; XMILE and CMSD later |
| D2 | Time units | Settled by BPSim: scenario `baseTimeUnit`, per-value overrides |
| D3 | Entity attributes | Settled by BPSim: token properties of BPSim's types |
| D4 | First UI milestone | Forms and results before the canvas (M3.2–M3.3 before M3.4) |
| D5 | Browser build | Settled: from M3.1 on, published on GitHub Pages with a preview per pull request; wasm32 first, wasm64 when its toolchain is verified |
| D6 | Conditions and logic | Settled by BPSim: its XPath 1.0 expressions, the subset models use first |
| D7 | Reference models as test files | Settled: WfMC's examples and the XSD are downloaded from bpsim.org by `cargo xtask fetch-bpsim` (pinned SHA-256, cached in CI), never committed, because they carry no licence of their own |
| D8 | First arrival of a start event | Open: at time 0 (as the reference simulator does, giving 151 car repair arrivals) or after the first interval (the specification's wording, and StochCraft today, giving 150). Evidence from M2.3's cross-check: with the first arrival at 0, the car repair counts match the reference simulator's exactly for arrivals and within 0.01 standard errors for the others; after the first interval they are one arrival short and within 0.15. Both pass |
| D9 | Where credibility data live (parameter pedigree, validated ranges) | In the model file, as `stochcraft:` BPSim vendor extensions, so they travel with the model and other tools keep them |
| D10 | Replications in hybrid models (M4) | Each replication has its own SD state; uncertainty about parameters is an outer sample around the inner replications, never mixed into them |
| D11 | The shared clock, and where coupling lives (M4) | SD steps are recurring events on the `des` event list; coupling lives in an L3 crate or an `sd` → `blocks` order in `xtask/src/layers.rs`; which goes first when an SD step and a DES event fall at the same time is a fixed rule, to confirm |
| D12 | Bulk flow (tanks, pipes) (M4) | Exact events: with rates constant between changes, the time a level is reached is worked out and scheduled in `des`, rather than found up to one step late by a fixed-step `sd` |
| D13 | Hybrid model files (M4) | One BPMN and BPSim file, naming XMILE variables through a `stochcraft:` vendor extension, with the couplings declared in it |
| D14 | The BPMN MIWG test suite (github.com/bpmn-miwg/bpmn-miwg-test-suite, CC BY 3.0 by its `LICENSE.txt`) as test files | Settled: fetched in CI at a pinned commit and never committed, as for D7, so the repository stays under one licence; bpmn.io's bpmn-js-integration (MIT) does the same, cloning its own copy of the suite into `tmp/` in its CI. Only its `Reference/` folder is used, for reading, round-trip and diagnostics tests; the other folders are exports from commercial modelling tools and are not used. If a few files are ever needed offline, only those reference models are committed, each with a row in ATTRIBUTION.md and the CC BY 3.0 notice |
| D15 | Monte Carlo model files (M4.1) | Settled (2026-11-15): a BPMN file with no process, whose BPSim scenario holds the inputs as parameters with their distributions and the outputs as `stochcraft:` vendor extensions (expressions, units, targets, correlations), so D1 holds; CSV import and export of the input table |
| D16 | Agent behaviour in files (M4.3) | Settled (2026-11-15): each agent type's state chart is a W3C SCXML 1.0 document (Recommendation, 1 September 2015), referenced from the BPMN file by a `stochcraft:` vendor extension; populations, the environment and the links to blocks are `stochcraft:` extensions |
| D17 | SCXML data model and timing (M4.3) | Settled (2026-11-15). SCXML lets a platform define its own data model (§5.1, Appendix B), and the Recommendation defines only the null and ECMAScript ones in full. So: a `stochcraft` data model using BPSim's XPath 1.0 subset (D6) for `cond`, `expr` and `location`, with SCXML's `In()`; random timeouts through `<send delayexpr>` calling the distribution functions, since `delay` itself is a fixed CSS2 time; no `<script>`. Other tools then read the charts' structure, but not our expressions |
| D18 | Agent scheduling and random streams (M4.3) | Settled (2026-11-15). Agents run event-driven on the `des` event list; a stepped mode only for models written that way, activating agents in a set or a random order each step. Events at the same time run by time, then priority, then an order the experiment sets openly: first scheduled first (the default, as today, so validated results do not change), last scheduled first, or random from a stream of its own, so a model can be checked for depending on the order. Initial events, step activations and broadcasts are scheduled in population, then agent order. Each `des` event delivered to an agent is one SCXML external event, handled by a complete zero-time macrostep; `<raise>` and `#_internal` events stay inside the agent; delayed and zero-delay `<send>`s become `des` events, and `<cancel>` marks them stale; a limit on microsteps per macrostep stops a run that loops. Each agent has its own random stream, derived from the replication's seed, its population and its serial number, with a stream per purpose, so adding an agent never changes another's numbers |
| D19 | Python (M6.1) | Settled (2026-11-16): a native package, `pip install stochcraft`, the engine built into a Python module with PyO3 and maturin; Python drives models from outside, and logic inside a run is M6.3's, not Python's |
| D20 | Plugins (M6.2, M6.3) | Settled (2026-11-16): sandboxed WebAssembly plugins in `wasmi`, as the other Craft apps have; command plugins first (through the engine's commands, one undo step, permissions in a manifest), then logic plugins called during a run with the engine's random streams, an instruction budget and a memory cap |
| D21 | What comes after the capacity table, and where the sorter sits | Recommended: the sorter (M5.1 conveyors, M5.2 operators, M5.3 the layout view in 2D then 3D, M5.4 its dashboard) first, built alongside M3.7 because it changes `model`, `blocks` and the engine while M3.7 changes `ui-egui`; M5.3 follows #151's moving entities so it reuses them. Then M7.1 line balancing (small, needs no run, reuses takt and the capacity table), M7.2 the from-to chart (reuses the capacity table's rates, and M5.5's layout distances; `layout_scale` only for a model with no layout), M7.3 inventory, M7.4 sharing. M4 waits until the sorter's first two steps are in. M5.5's layout layer then comes before M7.2, which draws on it |
| D22 | Moving the apps from eframe's glow backend to wgpu | **Superseded by D27**, the single renderer decision. Kept for the record: it recommended the move at M5.3, as its own PR before it, so the 3D view could reuse the Craft apps' shared viewport from SolidCraft (camera, picking, shading, a headless rasterizer, a browser build already working), which runs on wgpu. The alternative, a renderer of our own through an `egui_glow` paint callback, keeps today's backend but rewrites all of that. The 3D view then shipped on a CPU renderer (`view3d.rs`), and D27 keeps it until a measurement shows it too slow |
| D23 | Conveyor length: typed or from its path (M5.5) | **Recommended: from the path.** The path's exact length is the length the run uses; a typed length stays as an override, shown beside the drawn length with a warning. An existing file whose `length` differs from its path reads that `length` as the override, so no validated result moves |
| D24 | Where layout data lives (M5.5) | **Recommended:** all of it in the BPMN file as `stochcraft:` extensions (what the run reads, and scenery in `stochcraft:scene`), with glTF 2.0 as the exchange format. No sidecar: a TOML or JSON file would use a standard syntax but a vocabulary of our own, against principle 1. Cost: larger BPMN files and noisier diffs that other BPMN tools carry blindly |
| D25 | A divert: a point on a conveyor, or a gateway block linked to one (M5.5) | **Recommended: both, one per layer.** The logic stays an exclusive gateway (M5.1's divert routing, conditions, DMN), so BPMN tools still read it; the gateway is placed at a divert point at `s` on a section, which gives it its position and lane. A divert point without a gateway is not allowed |
| D26 | The whole-screen redesign: before or after the layout work (M5.5) | **Recommended: interleaved.** U1 (run bar) and U2 (properties of the selection) first, since layout editing leans on them; the library and model tree with the Layout view; the experiment editor, spreadsheet view, live card and statistics table as their own PRs alongside L4 to L12 |
| D27 | The 3D renderer (M5.5) | **Recommended:** keep `view3d.rs`'s CPU renderer, fed from the Layout, with levels of detail and a triangle budget; D22's move to the shared wgpu viewport waits until the CPU renderer is measured too slow (for example 5,000 moving totes in the browser) |
| D28 | Which layout data marks results out of date; the fate of `layout_scale` (M5.5) | **Recommended:** every field is tagged sim or look; sim fields (paths, places that transports use, chute capacities, walkways) enter the results baseline and look fields never do. `layout_scale` stays only for models without a layout, where distances fall back to the diagram's centres, flagged in the view |
| D29 | Recirculation on a sorter loop (M5.5) | **Recommended:** built in at every divert: a full lane sends the item round again; after a set number of rounds (default 3) it goes to an exception lane; counts per divert and per item are reported |
| D30 | Decision rules for routing and classification (M5.5) | **Settled (2026-11-19):** DMN decision tables, read from a `.dmn` file beside the BPMN, named from a business rule task, gateway or divert by a `stochcraft:` extension; first the hit policies UNIQUE, FIRST, PRIORITY and COLLECT and a small FEEL subset for input entries |
| D31 | The parcel hub example's scope (M5.5) | **Recommended:** S's outbound segment first (its checks are the stronger), then B's inbound side; whole-hub figures only as labelled scale context, never as a check |
