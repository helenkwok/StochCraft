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
- `stochcraft-model` is the model in memory (blocks with stable ids, connections, experiment,
  validation), with the capacity table, line balancing and `Path`, geometry with exact lengths.
- `stochcraft-blocks` compiles models of sources, servers, tasks sharing resource pools, delays,
  conveyors, sinks, decisions, splits and joins, batches and assemblies. It handles processing
  times by product type, finite buffers (fixed, by timetable or per working server, with a way
  out for those turned away), shifts, breakdowns and stop conditions, and reports per-element
  statistics.
- `stochcraft-mc` runs Monte Carlo models: inputs that are distributions, outputs that are
  expressions over them, plain or Latin hypercube sampling, rank correlations between inputs
  (Iman and Conover), and each output's mean, percentiles and chance of meeting a target.
- `stochcraft-sd` runs system dynamics models: stocks and flows read from XMILE 1.0, with delays,
  smooths, graphical functions and random functions, by Euler or fourth-order Runge-Kutta, and
  every variable over time with bands across replications.
- `stochcraft-io` reads and writes models as BPSim on BPMN or XPDL.
- `stochcraft-engine` makes every change and run a command with a JSON schema, undo and redo,
  and a replayable journal, and adds sweeps, sensitivity analysis, designed experiments and
  capacity sizing.
- `stochcraft-render` generates the parametric low-poly meshes the 3D view draws.
- `stochcraft-mcp` serves the engine's commands to AI agents over the Model Context Protocol.
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

To run until the numbers are precise enough rather than a fixed number of times, give a target:

```bash
stochcraft-cli run queue.bpmn --reps 10 --until 5% --max 200
```

A sequential run makes the `--reps` replications first, then adds more in batches until every
key measure (up to four results per block, the ones the window shows first) has a 95% half-width
within 5% of its mean, or `--max` replications have run. A share such as a utilisation is judged
in percentage points instead (`--until-points`, 1 by default), since a small share is never
within 5% of itself. Each batch is sized from the widest result so far: the interval narrows
with the square root of the replications, so about n × (half-width ÷ target)² replications are
needed, rounded up to a multiple of ten and at most `--max`; where a result unsettled gives
nothing to estimate from, the count at least doubles. A result whose replications all gave the
same value is never taken as settled (a rare event may not have happened yet). It does not hold
the run back once the results that vary are settled, so a count the model fixes, such as
arrivals on a timetable, does not force `--max`; the results name it and give how often per run
an event none of the runs met may still happen (about 3/n after n runs, at 95% confidence). If
nothing watched varies, the run goes on to `--max`. A mean at or near zero never meets a share of
itself either; the run then goes on to `--max` and says why. Replication i is the same as in a fixed run, so a run that stops at n replications
gives exactly the results of `--reps n`. The run record says how many replications ran, after
which looks and why the run stopped, and calls the interval *estimated precision after adaptive
stopping*: choosing the count from the results means the interval's coverage is not guaranteed to
be 95%. A scenario can make its runs sequential itself (the engine's `experiment.set` with
`sequential`, saved in the file as the `stochcraft:sequential` extension on the scenario);
`--fixed` runs exactly `--reps` replications whatever the scenario says. The method follows Hoad,
Robinson and Davies (2011), "Automated selection of the number of replications for a
discrete-event simulation", *Journal of the Operational Research Society* 62(11).

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

### Monte Carlo models

A Monte Carlo model is a calculation rather than a process: inputs are distributions (a cost, a
duration, a demand) and outputs are expressions over them (a total cost, a finish, a profit).
Each trial draws every input once and evaluates every output in order; there is no clock and no
queue. It is built with the engine's `mc.*` commands and saved as a BPMN file with no process: the
inputs are the BPSim scenario's own properties (`ScenarioParameters/PropertyParameters`, each a
BPSim distribution), the rest is the `stochcraft:monteCarlo` extension on the scenario, and the
scenario's `replication` is the number of trials.

```bash
stochcraft-cli exec cost.json
```

with `cost.json`:

```json
[
  {"command": "model.new", "params": {"name": "Project cost"}},
  {"command": "mc.input.set", "params": {"name": "labour", "unit": "h",
    "distribution": {"type": "triangular", "min": 80, "mode": 100, "max": 150}}},
  {"command": "mc.input.set", "params": {"name": "rate", "unit": "USD/h",
    "distribution": {"type": "uniform", "min": 40, "max": 50}}},
  {"command": "mc.output.set", "params": {"name": "cost", "unit": "USD",
    "expression": "$labour * $rate", "at_most": 5000}},
  {"command": "mc.correlation.set", "params": {"a": "labour", "b": "rate", "rank": 0.3}},
  {"command": "file.save", "params": {"path": "cost.bpmn"}}
]
```

`run` finds that the file is a Monte Carlo model and runs its trials (`--reps` is the number of
trials, 10,000 for a new model):

```bash
stochcraft-cli run cost.bpmn --reps 100000
```

Each output gets its mean with a 95% interval, standard deviation, P5, P50, P95, minimum and
maximum, and, with a target (`at_most` or `at_least`), the share of trials meeting it with its
95% interval; `--format json` adds a histogram, the cumulative curve, the percentiles
P10 and P90. For each output the table and the JSON also say which inputs matter: a tornado of
swings (the output with each input at its 10th and 90th percentile, the others at their
medians) and the rank correlation of each input with it. `--until 1% --max 1000000` adds trials
until every output's mean is within ±1% of itself. Outputs are BPSim expressions with the inputs (and earlier outputs) as `$name`, the
arithmetic of XPath and `min`, `max`, `sum`, `abs`, `sqrt`, `exp`, `log`, `pow`, `sin`, `cos`, `round` and
`if(condition, then, else)`; a condition such as `$cost > 6000` gives 1 or 0 in each trial, so its
mean is a probability. A trial whose output is not a number (a division by zero) is counted apart.

`--sobol` also works out each input's share of each output's variance (Sobol indices), from
`--sobol-rows N` rows of each of two base samples (1,000 by default, at least 10), so N × (inputs
+ 2) evaluations of the model; the table gives, for each output, each input's share alone (first
order) and with its interactions (total), as percentages with 95% bootstrap intervals, and the
JSON has a `sobol` field. The command behind it is `mc.sobol` (`rows`, `seed`, which is the
model's own by default, and `bootstrap`, 1,000 by default): per output its `variance`,
`rows_used`, `rows_dropped`, `unavailable` (why it has no indices, if it has none) and for each
input `first` and `total`, each with a `value` and an `interval`, besides `rows`, `evaluations`
and the `constants` left out. The estimates are not clamped: a small sample can give one a little
below 0 or above 1. The indices need independent inputs, so a model with correlated inputs is
refused with an `invalid_model` error that says so; `run --sobol` then prints the reason (under
`sobol.unavailable` in the JSON, on stderr for `--format csv`) and still prints the normal
results. The calculation is `stochcraft_mc::sobol` (Saltelli's design; first order by Saltelli et
al. 2010, total by Jansen 1999).

`mc.set` chooses Latin hypercube sampling (`"sampling": "latin_hypercube"`): within each batch of
1,000 trials, each input's range is cut into 1,000 equally likely strata and each is drawn once.
`mc.correlation.set` gives two inputs a rank correlation, induced by the method of Iman and
Conover (1982); correlations that cannot all hold together are refused. Trials come in batches
of 1,000, each with its own random streams keyed by the seed, the batch and the input's name, so
results are the same on any number of threads and adding an input leaves the others' draws as
they were.

How the intervals are worked out depends on whether the trials are independent. With plain
random sampling and no correlations they are: the mean's interval is Student-t across the trials
and a target's share has Wilson's score interval. Latin hypercube sampling and correlations make
the trials within a batch dependent (a batch of 1,000 Latin hypercube draws of a uniform holds
exactly 500 below its median), so there each batch counts as one independent replicate
(replicated Latin hypercube sampling: Owen, 1992; Glasserman, *Monte Carlo Methods in Financial
Engineering*, 2003, §4.4): the mean's interval is Student-t on the batch means, and a target's
share's is Student-t on the batch shares, clamped to 0 and 1, with one degree of freedom fewer
than there are batches. Such runs are whole batches, so the trials asked for are rounded up to a
multiple of 1,000 (the results and the run record give both numbers), a run of one batch has no
interval, and a sequential run judges its outputs on the intervals across batches. The run
record's `interval` says which, for example `Student-t across 12 independent Latin hypercube
batches of 1000 trials`. The standard deviation is always across trials: it describes the
output, not the precision of its mean.

The input table goes out and back in as CSV:

```bash
stochcraft-cli inputs cost.bpmn --out inputs.csv
stochcraft-cli inputs cost.bpmn --import inputs.csv
```

The table's header is `name,distribution,parameters,unit`, with the parameters as
`min=80;mode=100;max=150`.

`mc.run`'s results give `seed`, `sampling` and `interval_basis` (`{"across": "trials", "count":
n}` or `{"across": "batches", "count": b, "size": 1000}`) beside the run record, and
`stochcraft_engine::mc::results_csv` writes them as the CSV the app and `run --format csv` share.
The app does not call `mc.run`, which makes every trial before it returns: it starts the same run
with `Engine::mc_start` and makes a few batches a frame with `McJob::step` (on every core natively,
one batch at a time in the browser, where the page has no threads), then takes the results with
`Engine::mc_results`. Both are `stochcraft_mc::Stepper`, which asks for batches, takes them in
order and looks at the results only once every batch it asked for is in, so a sequential run looks
at the same numbers of trials and the results are the same however the batches are spread.
The app's Sobol shares are made the same way: `Engine::sobol_start` gives a `SobolJob`, whose
`step(pieces)` evaluates a batch of rows or works out one output (on the calling thread) until it
returns true, and whose `results()` are `mc.sobol`'s; `mc.sobol` drives the same job to the end,
so the two agree. `Engine::sobol_evaluations` says what the default rows would cost.

Changes can be scripted as engine commands (`stochcraft-cli schema` lists them with their
parameters) and replayed:

```bash
stochcraft-cli exec changes.json --open queue.bpmn
```

### System dynamics models

A system dynamics model is stocks (levels) and flows (rates) over time, read from an XMILE 1.0
file (OASIS, 2015). There is no window or canvas for it yet: it is opened, run and read through
the engine and the command line.

```bash
stochcraft-cli run tests/models/sir.xmile                     # each variable at the end of the run
stochcraft-cli run tests/models/sir.xmile --format csv        # every variable over time
stochcraft-cli run model.xmile --format json --dt 0.125       # another time step
stochcraft-cli run model.xmile --dt-check                     # and again at half the step
stochcraft-cli run noisy.xmile --reps 500 --seed 3            # a model with a random function
```

The engine commands are `sd.open` (the text of an XMILE file; `file.open` also opens one from
disk), `sd.run` (`replications`, `seed`, `dt`, `dt_check`, `time_unit`) and `sd.get` (the model's
variables and checks, and with `variable` its values from the last run). `run` and `file.save`
refuse such a model, naming `sd.run`; it cannot be edited or saved yet. In Rust:
`stochcraft_sd::{read_xmile, compile, run}`.

- **Read:** the header, `sim_specs` (`start`, `stop`, `dt` and its `reciprocal`, `method` Euler or
  RK4, `time_units`), stocks (`eqn` as the initial value, `inflow`, `outflow`, `non_negative`,
  also as a `behavior` default), flows, auxiliaries, graphical functions (`gf`: `xscale` or
  `xpts`, `ypts`, continuous, extrapolate or discrete), `units` and `doc`. Equations have the
  operators of XMILE 3.3 (`^` tighter than a minus sign, `MOD`, `AND`, `OR`, `NOT`, `IF ... THEN
  ... ELSE`) and `IF_THEN_ELSE`, `MIN`, `MAX`, `ABS`, `SQRT`, `EXP`, `LN`, `PULSE`, `STEP`,
  `RAMP`, `TIME`, `DT`, `STARTTIME`, `STOPTIME`, `DELAY1`, `DELAY3`, `SMTH1`, `SMTH3` (and
  `SMOOTH`, `SMOOTH3`), `NORMAL` and `RANDOM` (`UNIFORM` too), and a graphical function called by
  its name.
- **Refused, by name:** arrays (`dimensions`, `element`, subscripts, `uses_arrays`), modules and
  more than one `model`, macros, conveyors and queues, MathML equations, a flow's `multiplier`,
  `overflow` and `leak`, event posters, any other function (`SUM`, `SIN`, `INIT`...), an
  integration method other than Euler or the Runge-Kutta family (RK2, RK45 and Gear run as RK4,
  which XMILE allows, and the notes say so).
- **Not kept, and said so in `notes`:** the diagrams (`views`), styles, display settings,
  another vendor's extensions and groups.
- **Extensions of StochCraft:** `stochcraft:save_step` in `sim_specs` (the interval results are
  saved at, by default the step), and a random function's fourth argument, the interval between
  draws: `NORMAL(mean, std_dev, seed, interval)` (by default one draw a time step of the file).

**Numerics.** Time is `start + n * DT` from an integer counter. Variables are worked out in
topological order of what their equations name, ties broken by name; an algebraic loop is an error
naming the cycle. A stock that cannot go negative limits its outflows, in the order it lists them,
to what it holds at the start of the pass, so the stock that receives a flow gets the same
limited flow and material is conserved. `DELAYn` and `SMTHn` are hidden stocks (their time
constant is never taken below the step), `PULSE` delivers its volume in one step under either
method, a graphical function is clamped at its ends with a warning, and a random function holds one
draw for its interval and takes it from a stream keyed by variable, call and draw number, so
results do not shift when another variable is added or the step is halved. A model with no random
function runs once; one with a random function runs `replications` times (100 by default) and
gives the mean, the median and the 5th and 95th percentiles at each saved time. Units are checked
on both sides of each equation (compound units, and `1`), and a flow against its stock's units per
the model's time unit; a mismatch is a warning. `time_unit` reports times in another unit.

`dt_check` runs the model again at half the step and gives, for each stock, the largest relative
difference between the two mean paths (relative to the larger of the value and 0.1% of the
stock's peak); within 1% is called independent of the step.

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
(`--open` the same file, which opens the scenario the journal records in `start.scenario`; a
journal that began with `model.new` starts its own model) and refuses
with a `conflict` error if not, unless `--force`. If the file has no scenario of that name, `exec`
stops with the error from opening it; with `--force` it replays onto the file's first scenario.
It then re-applies the edits, undo and redo, and
skips the saves, runs and experiments (`file.save`, `run`, `mc.run`, `mc.sobol`, `sd.run`, `run.start`,
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
E₄/M/1 batches, assembly, arrivals by time of day, breakdowns and activity networks. Monte Carlo
models are checked against exact answers: the sum of independent normals, the product of
log-normals, a PERT schedule's finish on a single path, rank correlations induced against their
targets, and Latin hypercube strata holding one value each. They are
also checked against another BPSim simulator's published results for the BPSim car repair model. StochCraft runs
about 54× faster than SimPy on an M/M/1 benchmark ([bench/](../bench/README.md)).

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the clean-room and licence rules.
