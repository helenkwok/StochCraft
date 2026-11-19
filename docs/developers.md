# StochCraft developers guide

How StochCraft is built, how to build and test it, and how to drive it from the command line or
another program. For using the app, see the [user guide](user-guide.md).

## Build and test

```bash
cargo xtask ci
```

runs the formatting check, the layer check, clippy with warnings as errors, and the tests. Run the desktop app
with

```bash
cargo run --release -p stochcraft
```

To build the browser app locally, install [trunk](https://trunkrs.dev) and run it from
`apps/stochcraft-web`:

```bash
trunk serve --release
```

In the browser, runs use a web worker for each core but one (up to 16), and the app can be
installed and used offline. Every pull request gets a preview build, linked from the pull
request.

### Fonts for the browser build

Without more, the browser app has only IBM Plex and egui's faces, so Japanese, Korean, Chinese,
Hindi, Thai and other scripts draw only with fonts the visitor's system has, or as boxes. Set
`CRAFT_FONTS_DIR` to a checkout of [craft-fonts](https://github.com/storytold/craft-fonts) (the
commit `.github/workflows/pages.yml` pins) and the build serves its faces beside the app as
`fonts/`, fetched when the interface language or a character needs them, as on the published
site (`crates/ui-egui/build.rs`). Bengali, Tamil, Telugu and Lao have no face there yet: they
draw only with fonts the visitor's own system has.

## The crates

- `stochcraft-des` runs SimPy-style async processes with resources on a deterministic event
  queue.
- `stochcraft-experiment` runs replications in parallel with reproducible random streams
  (`stochcraft-random`) and reports 95% confidence intervals (`stochcraft-stats`).
- `stochcraft-blocks` compiles models of sources, servers, tasks sharing resource pools, delays,
  sinks, decisions, splits and joins, batches and assemblies. It handles processing times by
  product type, finite buffers (fixed, by timetable or per working server, with a way out for
  those turned away), shifts, breakdowns and stop conditions, and reports per-element
  statistics.
- `stochcraft-io` reads and writes models as BPSim on BPMN or XPDL.
- `stochcraft-engine` makes every change and run a command with a JSON schema, undo and redo,
  and a replayable journal, and adds sweeps, sensitivity analysis, designed experiments and
  capacity sizing.
- `stochcraft-cli` runs models from the command line; the app (`apps/stochcraft`,
  `apps/stochcraft-web`, UI in `crates/ui-egui`) runs them in a window.

[architecture.md](architecture.md) has the layers and how they depend on each other, and
[ROADMAP.md](../ROADMAP.md) what is done and what comes next.

## The command line

```bash
cargo install --path apps/stochcraft-cli
```

Write a starting model (`--list` shows the templates), check it, and run it:

```bash
stochcraft-cli new --template mm1 queue.bpmn
```

```bash
stochcraft-cli validate queue.bpmn
```

Before running, `explain` works out each station's arrival rate, mean service time, units and
utilisation from the model's own numbers, marks it fine, congested (above `--target`, 85% by
default) or unstable, names the bottleneck and the units that would bring a station under the
target, and gives the mean wait where a queueing formula holds exactly (M/M/c, M/M/c/K, Erlang A,
balking, M/G/1), or says why none does. Rates that depend on the run (routing by condition, the
shortest queue, those giving up) are not guessed. `--format json` gives the engine's
`model.capacity` answer.

```bash
stochcraft-cli explain queue.bpmn
```

To balance work elements of an assembly line across stations for a cycle time, `balance` reads a
CSV file (`id,time,after`, with `;`-separated predecessors) and compares the largest candidate
rule, Kilbridge and Wester, and ranked positional weights against the theoretical minimum
(`--heuristic` runs one; `--format json` gives the engine's `line.balance` answer):

```bash
stochcraft-cli balance elements.csv --cycle 1.0
```

```bash
stochcraft-cli run queue.bpmn --reps 20
```

`run` prints each element's statistics with the half-width of a 95% confidence interval, under a
run record (version, model digest, scenario, seed, replications, warm-up and duration) that is
enough to reproduce the numbers. `--format json` or `--format csv` gives machine-readable
results, `--out` writes them to a file, and `--seed`, `--warmup` and `--duration` override the
model's scenario for that run. Any BPMN or XPDL file with BPSim data can be run the same way.

To vary several parameters at once, `design` runs every combination of their levels (a full
factorial) on the same seed, and gives each result at each point, each factor's main effect and,
for factors with two levels, their two-way interactions, each with a paired 95% interval:

```bash
stochcraft-cli design queue.bpmn --factor arrivals.mean=1.33,2 --factor server.mean=0.8,1 \
  --result server.mean_wait --reps 20
```

A factor is `id.field=levels` or `id.field=min..max`, the field one of `capacity`, `quantity`,
`queue_capacity` and `mean`. `--design latin-hypercube --points 20` samples the ranges instead
and gives each factor's rank correlation with each result. A design may take at most 10,000 runs
(points × replications).

To choose a distribution for measured times, `fit` fits one column of a CSV file (or a plain list,
one value per line; `-` reads standard input) to each family by maximum likelihood and ranks the
fits by Anderson–Darling A², with the Kolmogorov–Smirnov distance beside it:

```bash
stochcraft-cli fit service-times.csv --column minutes
```

`--families gamma,log_normal,weibull` limits the families, and `--format json` gives the engine's
`estimate.fit` answer. Fewer than 20 values get no fit (`--min-count` lowers that, to 5 at least),
and families that need values above 0 are left out, with the reason, when a value is 0 or less.
The best fit's distribution is printed as `block.set` takes it.

Changes can be scripted as engine commands (`stochcraft-cli schema` lists them with their
parameters) and replayed:

```bash
stochcraft-cli exec changes.json --open queue.bpmn
```

### The journal

The engine records every command that is not a query in its journal. A new or opened document
(`model.new`, `file.open`, and opening a file in the app) starts a new journal, so a journal is
the history of one document. `exec --journal out.json` writes the journal of what the script did,
also when a step failed, as a JSON object:

```json
{
  "format": 1,
  "stochcraft": "0.2.0",
  "start": {
    "kind": "file",
    "model_digest": "sha256:...",
    "name": "queue.bpmn",
    "file_digest": "sha256:...",
    "scenario": "Scenario 1"
  },
  "entries": [
    {"command": "block.set", "params": {"id": "serve", "capacity": 2}},
    {"command": "run", "params": {"replications": 20}}
  ]
}
```

- `format` is the version of this layout (1). A journal in a newer format is refused.
- `stochcraft` is the version that wrote it.
- `start` is where the journal began. `kind` is `engine` (a model the engine was given), `new`
  (`model.new`, with the model's `name`) or `file` (with the name the file was opened under,
  the digest of its bytes and the scenario). `model_digest` is the digest the run records carry,
  of the model at the start. `file_digest`, `name` and `scenario` are left out when they do not
  apply. The command that started the document is not an entry: `start` records it.
- `entries` are the commands in order, as `{"command", "params"}`. A script for `exec` is the same
  array on its own, and `exec` runs that in full.

Replaying a journal (`exec journal.json`) first checks that the model is the one it started from
(`--open` the same file; a journal that began with `model.new` starts its own model) and refuses
with a `conflict` error if not, unless `--force`. It then re-applies the edits, undo and redo, and
skips the saves, runs and experiments (`file.save`, `run`, `run.start`, `run.status`,
`run.cancel`, `experiment.sweep`, `experiment.sensitivity`, `experiment.design`,
`experiment.size`), printing `"skipped": true` for each; `--all` carries them out too. In Rust,
`Engine::journal_document` gives the journal and `Engine::replay_journal` replays it
(`ReplayMode::EditsOnly` or `All`); `Engine::replay` still runs a plain list of steps in full.

## Using it from an AI agent

`stochcraft-cli mcp` serves the engine's commands over the Model Context Protocol, so an agent
such as Claude Code can open, build, run and compare models:

```bash
claude mcp add stochcraft -- /path/to/stochcraft-cli mcp
```

Start the app with `stochcraft --control 7070` and add `--connect 127.0.0.1:7070` to watch the
agent work in the window. [mcp.md](mcp.md) has the tools and options.

## How it is checked

Results are checked against queueing theory and exact results: M/M/1, M/M/c (Erlang C), M/M/1/K,
priority queues, machine repair, a Jackson network, a two-machine line with a finite buffer,
E₄/M/1 batches, assembly, arrivals by time of day, breakdowns and activity networks. They are
also checked against L-SIM's published results for the BPSim car repair model. StochCraft runs
about 54× faster than SimPy on an M/M/1 benchmark ([bench/](../bench/README.md)).

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the clean-room and licence rules.
