# StochCraft roadmap

Where StochCraft is going next and in what order. Three milestones: **M1** model files, the
engine and a command line; **M2** the flow logic a factory or logistics model needs; **M3** the
desktop app (and the browser app built from it). Each milestone is a series of pull requests
small enough to review in one sitting; every one keeps `cargo xtask ci` green.

Decisions still open are marked **Decision** with a recommendation; the rest follows the
patterns of the sibling Craft apps (FieldCraft, CADCraft, SolidCraft). The published work behind
the credibility, experiment and digital-twin items (NASA-STD-7009B, DES practice at NASA, NNSA
and the Aerospace Corporation, digital-twin reviews) is summarised in
[docs/research.md](docs/research.md).

## Where we are

| Crate | Layer | State |
|---|---|---|
| `random` | L0 | xoshiro256** streams; BPSim's distributions; `Dist`; fits to three-point estimates (`estimate`) |
| `stats` | L0 | `Tally`, `TimeWeighted`, Student-t 95% intervals, `Summary`; percentiles, empirical distributions with DKW bands, Wilson intervals (`spread`) |
| `des` | L1 | Event queue; async processes with `hold`, `spawn`, interrupts; priority and preemptive `Resource`; bounded and priority `Store`; waits with deadlines; `Signal`, `Gate` |
| `model` | L1 | The model in memory: elements, connections with probabilities and conditions, finite buffers, shared resource pools, experiment, entity properties, validation; BPSim's XPath expressions (`expr`) |
| `blocks` | L2 | Compiles a model: source, server, shared task, delay, sink, exclusive and parallel gateways, merges; finite buffers with blocking after service and loss; shared pools with roles, alternatives and priorities; entities carry typed properties; per-element statistics; warm-up |
| `experiment` | L3 | Parallel replications with reproducible streams, progress and cancellation |
| `io` | L3 | BPSim on BPMN and XPDL: read (gateways, routing, embedded sub-processes in line, `stochcraft:` vendor extensions), write new, save edits into opened files |
| `engine` | L4 | Commands with JSON schemas, undo and redo, journal, background runs; `flow.set` for routing; `queue_capacity` on servers; `resource.add`/`set`/`remove` and shared tasks |
| `stochcraft-cli` | app | `new`, `validate`, `run` with a run record, `exec`, `schema`, `mcp` |
| `ui-egui` | L5 | The app's window: menus, shortcuts, model outline, background runs, results, unsaved-change prompts |
| `mcp` | L5 | The MCP server: the engine's commands as tools for agents, headless (with optional read and write roots) or driving the app through its control channel |
| `stochcraft`, `stochcraft-web` | apps | The desktop shell (native file dialogs, `--control` for agents) and the browser shell, published on GitHub Pages |
| `bench/` | outside | M/M/1 against simu-des and SimPy (54× SimPy) |

Models are saved and opened as BPSim files, changed through engine commands, and run from the
command line or in a window, natively or in the browser
([helenkwok.github.io/StochCraft](https://helenkwok.github.io/StochCraft/)). Editing in the window
comes with M3.2.

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
unchanged saves are byte-identical, BPMN and XPDL versions give the same model, the L-SIM result
files parse) and XSD validation of everything StochCraft writes. The statistical cross-check
against L-SIM needs gateways, which all three reference models use, so it moves to M2.3.

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
  images are not needed.
- **Cross-check against another simulator:** the bundle includes result scenarios
  (`*_results.bpsim`) produced by Lanner's L-SIM for the three BPSim 2.0 models, with
  per-replication values (for example 151 car repair arrivals in each of three replications,
  and 107, 96 and 100 customers leaving). StochCraft's results for the same scenarios must agree
  with them statistically: deterministic quantities exactly, random ones within confidence
  intervals, since the random number generators differ. They are also the reference for how
  StochCraft writes its own result scenarios (`result`, `instance` per replication,
  `resultTimeStamp`). All three models use gateways, so the comparison runs from M2.3.
  It already raises one question (decision D8): L-SIM counts 151 arrivals for an inter-trigger
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
  (`stochcraft-experiment` gains a `Control` with progress and cancellation); one already
  running finishes first.
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
    per step; exit code 1 at the first step that fails
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
  result.

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
- **Car repair against L-SIM's published results for scenario S1**, from 400 replications of
  StochCraft's against L-SIM's three:
  - **Arrivals:** 150 by default, 151 with the first arrival at time 0, as L-SIM counts in every
    replication.
  - **Rejected estimates, repairs started, completed and customers leaving:** within 0.15
    standard errors (0.01 with the first arrival at 0).
  - **Issues repaired:** 345 against L-SIM's 306 (−1.2 standard errors), within the check. The
    model's `noOfIssues` is `int`, a truncated normal; StochCraft rounds it to the nearest whole
    number, and L-SIM's three replications cannot tell whether it does the same.
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
  turning a task into a shared task or back is for a new file (Save As), with the reason given.
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
  - Interruptible: the job stops and resumes on the same unit after the repair, with the
    service time it had left (preemptive resume). Otherwise the failure waits for the job to
    finish, and the unit takes no new job meanwhile.
  - Waiting entities go first come first served to the first unit that is up and idle.
    Servers without breakdowns run exactly as before.
  - Results: a server's `availability` (the time-average fraction of units up; "Availability"
    in the window, in every catalog). Utilisation counts a unit holding an interrupted job as
    busy.
  - Read, written, saved in place, and in the vendor schema
    (`crates/io/schemas/stochcraft-bpsim-1.xsd`); `block.set` and `block.add` take
    `breakdown` (`null` removes it).
  - **Not here:** failures counted in busy time only, and breakdowns on shared pools.

  **Validation** (`blocks/tests/breakdowns.rs`, 16 replications):

  | Case | Theory | StochCraft |
  |---|---|---|
  | One unit, failures exp(mean 100), repairs exp(mean 10): availability | 0.90909 | 0.90574 ± 0.00472 |
  | Same, saturated, service rate 1, interruptible: jobs in 20,000 | 18,181.8 | 18,157.9 ± 129.0 |
  | λ = 0.3, failures exp(mean 50), repairs exp(mean 5), interruptible: mean in service, λ E[S](1 + E[R]/MTBF) | 0.33 | 0.33126 ± 0.00217 |
  | Constant: failure 10 after repair, repair 2, jobs of 3, waiting for the job: jobs by 70 | 20 | 20 |
  | Same, interruptible: jobs by 69 | 19 | 19 |

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
- **Sweeps and sensitivity:** a parameter varied over a range, results per point, and a record of
  which parameters drive which results (NASA-STD-7009B [M&S 30]).
- **Capacity sizing:** add a unit at the bottleneck (the busiest resource) and run again, until
  the probability of meeting the target reaches the confidence asked for. Reported with the
  deterministic answer beside it, which is too small.
- **Results as inputs:** a metric's values across replications as an empirical BPSim user
  distribution for a coarser model.
- Acceptance model: a small multi-product plant written for StochCraft (two products sharing
  mixers and ovens, working hours, a weekly target at 70% confidence), sized automatically.

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
- **Sensitivity:** `experiment.sensitivity` moves each parameter down and up by a fraction,
  one at a time, and ranks the results' elasticities. Every sweep and sensitivity analysis is
  recorded, with the model's digest, in the `sensitivity` list of later runs' records
  (NASA-STD-7009B [M&S 30]).
- **Capacity sizing:** `experiment.size` takes a target (a metric `at_least` or `at_most` a
  value) and a confidence. It runs the experiment and adds a unit to the busiest task or pool,
  then runs again, until the share of replications meeting the target reaches the confidence.
  Beside it is the answer with every time at its mean, from one replication. `apply` makes the
  answer the model's, as one step to undo.
- **Acceptance model** (`tests/models/plant.json`, an engine script for `stochcraft-cli exec`):
  - A bakery with bread and cake orders that arrive on weekdays from 8 to 4 (70 and 50 a week).
  - Shared mixers and ovens that work those hours.
  - Mixing and baking times from three-point estimates, PERT and triangular, kept as their
    pedigree.
  - The target: at least 110 orders packed a week, in 70% of weeks. Sized from one mixer and
    one oven:


| Mixers, ovens | Weeks meeting the target (20) | Wilson 95% interval |
|---|---|---|
| 2, 5 | 20% | 0.08–0.42 |
| 3, 5 (the answer from mean times) | 55% | 0.34–0.74 |
| 3, 6 (StochCraft's answer) | 80% | 0.58–0.92 |

With every time at its mean, 3 mixers and 5 ovens look enough. With the variability they meet
the target in only 55% of weeks, and 6 ovens are needed: the deterministic answer is too small,
as the NNSA study found.

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

**Not yet:** sizing several resources a step at a time, or by cost; designed experiments with
several factors at once (factorial, Latin hypercube); saving targets in the model file; and
running analyses in the background or on the browser's web workers.

**Later in M2 or M4:** conveyors with length, speed and accumulation; travel networks for
forklifts and AGVs; agent behaviours (`agents` crate); stock-and-flow (`sd` crate, XMILE);
costs (BPSim `fixedCost`, `unitCost`); simulation traces in XES (BPSim `traceOutput`).

**M2 size:** about nine PRs, each independently mergeable after M1.

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
egui's faces for symbols, craft-fonts' faces where `CRAFT_FONTS_DIR` embeds them, then the
installed system fonts on the desktop).

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
(not embedded in CI or the web app, and incomplete upstream); CJK, Indic, Thai and Lao fonts on
the web; screenshot tests in craft-ui's six test languages; panes and icons.

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
    (about a minute for the whole run by default), and Jump to a time.
  - A clock bar shows where the run is, and when it is in its warm-up.
  - On the canvas:
    - each task shows the entities waiting and its units busy, with a bar for busy units;
    - delays show the entities inside;
    - sources show how many were created, and departures how many are done.
  - The inspector shows the selected block's counts and a chart of its queue (or what is inside)
    over time. This is the queue-length-over-time view moved here from M3.3.
- **Model changes:** changing anything the run depends on stops watching; moving shapes does
  not.
- **Tests:** `blocks/tests/live.rs`, `engine/tests/engine.rs`, `ui-egui/tests/live.rs`.

**Not yet:** entities drawn moving along the connections, watching in a web worker, and
replications other than the first.

### M3.6 Browser and agents

- `apps/stochcraft-web` (in M3.1a): replications in web workers, installable as an app, and
  working offline.
- `crates/mcp` and `stochcraft-cli mcp`: the engine's commands as MCP tools, headless or bridged
  to a running app, following FieldCraft's MCP design.

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

**Later:** 2D layout with entities moving between stations; 3D (the `render` crate, wgpu);
FlexSim-style factory layouts via the shared `craft-factory` data from CADCraft.

**M3 size:** about six PRs; M3.1–M3.3 make a usable app, M3.4–M3.5 make it a modelling tool.

---

## After M3: credibility and digital twins

Not scheduled yet; recorded so earlier work does not close these doors (docs/research.md):

- **Credibility reports:** `stochcraft-cli report`, a results assessment in NASA-STD-7009B's
  terms from the run records, sensitivity records and diagnostics. Models can declare validated
  ranges for their parameters, and results from outside them are placarded with the limit
  exceeded and by how much ([M&S 26]).
- **Digital twins:** starting a run from an observed state (queue contents, busy units, elapsed
  times) rather than empty; fitting input distributions from event logs; the engine's journal as
  the audit trail of what-if decisions.
- **Industry 5.0 measures:** workers as resources with skills and shifts; energy and CO₂ per
  part and per element (BPSim cost parameters plus a vendor extension); disruption scenarios.
- **Bulk flow:** rates, splits by fraction, tanks and batch treatment for process industries,
  with stock-and-flow in the `sd` crate.
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
                     │                                   M3.4 canvas
                     └──────────── new blocks appear ────M3.5 live view
                                   in M3 forms           M3.6 workers + MCP
```

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
| D8 | First arrival of a start event | Open: at time 0 (as L-SIM does, giving 151 car repair arrivals) or after the first interval (the specification's wording, and StochCraft today, giving 150). Evidence from M2.3's cross-check: with the first arrival at 0, the car repair counts match L-SIM's exactly for arrivals and within 0.01 standard errors for the others; after the first interval they are one arrival short and within 0.15. Both pass |
| D9 | Where credibility data live (parameter pedigree, validated ranges) | In the model file, as `stochcraft:` BPSim vendor extensions, so they travel with the model and other tools keep them |
