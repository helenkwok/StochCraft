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

#### Frame time, idle frames and widget ids

The window is immediate-mode: it lays everything out again each frame and, with AccessKit on,
builds the accessibility tree each frame for the platform adapter to compare with the last. Three
checks keep that in hand:

- `crates/ui-egui/tests/frame_time.rs` opens a model of 1,000 blocks and times frames on the
  canvas, in the 3D view and on the results page, with AccessKit off and on in alternating blocks
  (the app's pass and tessellation, plus the tree update handed to an `accesskit_consumer` tree;
  no GPU). It fails when AccessKit on takes more than 1.75 times as long as off (measured 1.07
  to 1.36), and warns when a median frame with AccessKit on exceeds its view's budget (about
  three times the slower GitHub Linux runner: canvas 250 ms, 3D view 20 ms, results 500 ms).
  The budgets only warn because runners differed by up to 1.6 times between runs, while the
  ratio, timed on one runner, held. Timings need release, so the test is
  `#[ignore]`d and runs in CI's heavy job, which prints the numbers in its log and job summary.
  Locally:

  ```bash
  cargo test --release -p stochcraft-ui-egui --test frame_time -- --ignored --nocapture
  ```

- `crates/ui-egui/tests/frames.rs` checks that the start page, the canvas, the 3D view and the
  results of a finished run settle and then ask for no further frames, and that an error
  appearing under a field leaves the other fields' AccessKit ids, and the keyboard, where they
  were. A field drawn after something that comes and goes needs an id of its own (`id_salt`), or
  the changing part goes in a container that is always there.

#### How the site is published

`.github/workflows/pages.yml` builds the browser app and keeps the site folder as an Actions artifact
(`site-main` for `main`, `site-pr-<number>` for each pull request from this repository, 30 days).
Nothing is committed to a branch. `.github/workflows/pages-deploy.yml` runs after every Pages run and
when a pull request closes (`pages-cleanup.yml` exists only to trigger it), takes the newest
`site-main` for the root and the newest `site-pr-<number>` of each open pull request for
`pr/<number>/`, and serves the result with `actions/deploy-pages`. A preview whose artifact has
expired or whose build failed is absent. If `main` has had no build for 30 days there is no root to
serve and the deploy fails, leaving the live site as it is: run the Pages workflow on `main` again.
The repository's Pages source is "GitHub Actions". The site is built only from workflow artifacts;
there is no `gh-pages` branch.

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
  (`stochcraft-random`), reports 95% confidence intervals (`stochcraft-stats`), and selects the
  best of a few systems with a stated probability (`select`, below).
- `stochcraft-model` is the model in memory (blocks with stable ids, connections, experiment,
  validation), with the capacity table, line balancing and `Path`, geometry with exact lengths.
- `stochcraft-blocks` compiles models of sources, servers, tasks sharing resource pools, delays,
  conveyors, sinks, decisions, splits and joins, batches and assemblies. It handles processing
  times by product type, finite buffers (fixed, by timetable or per working server, with a way
  out for those turned away), shifts, breakdowns and stop conditions, and reports per-element
  statistics.
- `stochcraft-mc` runs Monte Carlo models: inputs that are distributions, outputs that are
  expressions over them, plain, Latin hypercube, antithetic or Sobol sampling, rank correlations between inputs
  (Iman and Conover), and each output's mean, percentiles and chance of meeting a target.
- `stochcraft-sd` runs system dynamics models: stocks and flows read from XMILE 1.0, with delays,
  smooths, graphical functions and random functions, by Euler or fourth-order Runge-Kutta, and
  every variable over time with bands across replications.
- `stochcraft-agents` runs state charts with the semantics of W3C SCXML 1.0, their data in BPSim's
  expressions and their timeouts and rates on the `des` clock (see [Agents](#agents) below).
- `stochcraft-io` reads and writes models as BPSim on BPMN or XPDL.
- `stochcraft-engine` makes every change and run a command with a JSON schema, undo and redo,
  and a replayable journal, and adds sweeps, sensitivity analysis, designed experiments and
  capacity sizing.
- `stochcraft-render` generates the parametric low-poly meshes the 3D view draws.
- `stochcraft-mcp` serves the engine's commands to AI agents over the Model Context Protocol.
- `stochcraft-cli` runs models from the command line; the app (`apps/stochcraft`,
  `apps/stochcraft-web`, UI in `crates/ui-egui`) runs them in a window.

### What a shell does for recovery and late files

The window (`stochcraft-ui-egui`'s `App`) decides what is worth keeping; a shell decides where.
Every window or tab is a session with a slot of its own: `App::autosave` returns the copy to
write (`Autosave::Write`), to remove (`Clear`) or nothing (`Keep`), and the shell reports a
failure with `App::autosave_write_failed` or `App::autosave_clear_failed` so the call is made
again. At start the shell offers each copy left by a session that is gone with
`App::offer_recovery(slot, copy)`, and after each autosave call removes the slots from
`App::take_dropped_copies`. The desktop shell (`apps/stochcraft/src/recovery.rs`) tells a gone
session by its lock file (`File::try_lock`); the browser shell (`apps/stochcraft-web`,
`storage.rs`) by a time stamp the tab refreshes, with a few minutes' grace. A shell that reads a
file in the background calls `App::begin_open` when it starts and hands the ticket back to
`App::finish_open` with the bytes, which drops a stale completion.

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

A file that StochCraft read with errors (`validate` lists them: a processing-time expression it
cannot evaluate, say) is not run, in any output format, in the window or through MCP: reading it
substitutes something for each part it could not read (a time of zero for that expression), so
the numbers would be of another model. `run --allow-approximations` (the parameter
`allow_approximations` of `run`, `run.start` and the `experiment.*` commands) runs the
substitution deliberately: each substitution is printed on stderr and listed in the run record
(`approximations`, in addition to `file_notes`). An error pinned to one field (a task's processing time) stops counting while that field is set
to something else (undo brings it back), since the edit replaces the stand-in; renaming the block or
editing another property does not clear it, and any other error stands until the file is opened
again. A Monte Carlo model needs no
such gate: reading its inputs and outputs raises warnings only, never an error that substitutes
something.

`--out` (also `design --out`) refuses a path that is the model file it reads, and writes through
a temporary file that is renamed over the target, so a crash or a full disk leaves the old file
whole.

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

A scenario can also carry real measurements to check a run against (black-box validation, Law,
*Simulation Modeling and Analysis*, 5th ed., section 5.6.2): the engine's `observation.add`,
`observation.set` and `observation.remove`, saved in the file as the `stochcraft:observed`
extension on the scenario (an `observation` for each result, with its `element` and `metric` and
one of `value` with an optional `halfWidth` and an optional `tolerance` above 0, `atMost` and
`atLeast`, in the metric's own unit; only elements in StochCraft's namespace count, and the
extension's value `off` clears what an inherited scenario sets). `run` prints a **checked against
observed data** table after the results: each observation, the simulated mean with its 95%
half-width, the difference with its own interval for an observed value, and `agrees within
tolerance`, `differs`, `no difference detected`, `passes`, `fails` or `inconclusive at current
precision`, those that differ or fail first, then those not decided, then those that agree or
pass. The rules are in `stochcraft-stats` (`observed`). The difference interval of an observed
value is (simulated mean − observed) ± hypot(hs, ho). Without a tolerance it `differs` when the
interval excludes 0 and otherwise only detects no difference, which is not agreement (a wide
interval holds 0 whatever the truth; Sargent 2013). With a tolerance t it is an equivalence test
in the manner of two one-sided tests: `agrees` when the whole interval is inside [-t, +t],
`differs` when wholly outside, else `inconclusive`; the 95% interval is conservative against the
90% interval of the exact test. A bound passes when the whole simulated interval meets it, fails
when the whole interval violates it (the edge itself meets it), and is inconclusive otherwise,
never decided by the mean alone; fewer than 2 replications make every check inconclusive.
`--format json` has the checks as `observed`. The engine adds them in `wire::results`
(`engine/src/observed.rs`), so `run`, `run.start` and every client get them.

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

For an optimiser of your own, `evaluate` runs given values of the decisions for a range of
replications and prints the engine's `experiment.evaluate` answer as one JSON line: each
result's value in every replication, with their mean and 95% interval.

```bash
stochcraft-cli evaluate queue.bpmn --decide server.mean=0.8 --range server.mean=0.5..1.2   --result server.mean_wait --first 0 --count 10
```

Without `--decide` it reads one evaluation per line of standard input (a JSON object with the
command's parameters: `decisions`, and optionally `results`, `first`, `count`, `seed`) and writes
one line back for each, the arguments giving what a line leaves out; a refused line gets
`{"error": {...}}` naming the decision, and the next is read. Replications count from 0, and
replication i depends only on the seed and i, so replications evaluated in stages are exactly
those of one evaluation of them all. The user guide has worked examples with Optuna and pymoo
(`examples/optimisers/`), which CI runs.

To find the best whole-number decisions, `optimize` runs the engine's `experiment.optimise`: a
search over the decisions' ranges, then the finalists compared again on fresh replications, with
how sure the answer is.

```bash
stochcraft-cli optimize tandem.bpmn --decide desk1.capacity=11..40 --decide desk2.capacity=6..35 \
  --minimize 'desk1.mean_queue_length*50' --minimize 'desk2.mean_queue_length*50' \
  --cost desk1.capacity=20 --cost desk2.capacity=20 --cost-cap 420 --delta 2 --max-runs 20000
```

A decision is `id.field=min..max` or `id.field=min..max:step`, the field one of `capacity`,
`quantity` and `queue_capacity`. It starts from the model's value, or from `--start
id.field=value`; a starting value that is not one of the range's values (a model's 12 for
`11..40:2`, say) is refused before anything runs, naming the decision. `--minimize` (or `--maximize`)
takes `element.metric`, or `element.metric*weight` for a cost per unit of the result, repeated for
a weighted sum; `--cost id.field=cost` is a decision's cost per unit, and `--cost-cap` caps their
sum (designs over it are never run). `--constraint 'element.metric<=limit~tolerance'` (or `>=`;
quote it, since the shell reads `<` and `>` as redirections) keeps a result's mean within a limit. `--delta` is δ, the smallest difference in the objective
worth telling apart. It prints the answer in words and the finalists, or JSON (`--format json`);
`--time-limit` stops the search, and its JSON answer's `repeat` gives the `--max-runs` and
`--reserve-runs` that repeat it exactly. See "Whole-number decisions" below.

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
medians) and the rank correlation of each input with it. The table says "changes most with" for
the widest swing, which is the largest change between those two endpoints and not the largest
share of the variance (that is the Sobol share), and names each input whose output at its
median is outside its two endpoint values (`not_monotone` in the JSON), where the swing
understates or misses its effect. `--until 1% --max 1000000` adds trials
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
and the `constants` left out. An output with a row whose value is not a finite number in any of
the evaluations the estimators compare (`rows_dropped` above 0) gets no indices, with the counts
and the reason in `unavailable`: which rows are missing depends on the inputs, so leaving them
out conditions the inputs and what is left does not estimate the shares of independent inputs.
The other outputs keep their indices. The estimates are not clamped: a small sample can give one a little
below 0 or above 1. The indices need independent inputs, so a model with correlated inputs is
refused with an `invalid_model` error that says so; `run --sobol` then prints the reason (under
`sobol.unavailable` in the JSON, on stderr for `--format csv`) and still prints the normal
results. The calculation is `stochcraft_mc::sobol` (Saltelli's design; first order by Saltelli et
al. 2010, total by Jansen 1999).

`mc.set` chooses Latin hypercube sampling (`"sampling": "latin_hypercube"`): within each batch of
1,000 trials, each input's range is cut into 1,000 equally likely strata and each is drawn once.
Two more samplings narrow the intervals of suitable models for the same trials. `"antithetic"`
makes trials in pairs, trial 2j + 1 using 1 - u for every input where trial 2j used u, through
the inverse distribution function (Hammersley and Morton, 1956; Glasserman, 2003, §4.2); it is
refused with correlated inputs, whose reordering would break the pairs. `"sobol"` is randomised
quasi-Monte Carlo (`stochcraft_mc::qmc`): each batch takes the first 1,000 points of the Sobol
sequence with Joe and Kuo's direction numbers (2008, "new-joe-kuo-6.21201", the first 64
dimensions; their licence is in NOTICE), one dimension per input in declaration order, and
randomises each dimension with a random linear matrix scramble and a random digital shift
(Matoušek, 1998; Owen, 2003: a random lower-triangular binary matrix with ones on its diagonal
applied to the direction integers, then an exclusive or with a random 32-bit integer), drawn from
the stream keyed by the seed, the batch and the input's name; more than 64 inputs are refused.
The scramble keeps the net property and has the variance of nested uniform scrambling (Owen,
2008); `qmc::shifted`, the digital shift alone, stays for other callers. The file format keeps the sampling as `sampling="antithetic"` or `"sobol"` on
`stochcraft:monteCarlo`; a reader from before them warns that it does not know the value and
samples at random. On the command line `run --sampling random|latin-hypercube|antithetic|quasi-random`
chooses it for one run (`sobol` is accepted for `quasi-random`; it is not `--sobol`, the shares of
the variation), and `mc.set` and `mc.run` take `sobol`. `mc.correlation.set` gives two inputs a rank correlation, induced by the method of Iman and
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
batches of 1000 trials`. Sobol sampling is judged the same way (each batch's scramble is
independent, so its batch means are independent replicates; `interval_basis` is `batches`).
Antithetic trials depend on their twin and not on the other pairs, so the interval is Student-t
across the pairs, with one degree of freedom fewer than there are pairs (Glasserman, 2003,
§4.2; `interval_basis` is `pairs`, and runs are whole pairs). The reported mean is the mean of
all the valid trials. A pair with one trial that gave no number is a cluster of one trial, and
averaging the pair means would give that lone trial the weight of a whole pair (the mean
would then differ from the mean of the trials, and from a target's share of them), so the
interval is the ratio estimator's, R = Σy_j / Σn_j over the pairs j that have a valid trial,
with variance Σ(y_j − R n_j)² / ((k − 1) k n̄²) (Cochran, *Sampling Techniques*, 3rd ed., 1977,
§6.4; the pairs are independent clusters of random size); where every pair is complete it is
the interval on the pair means. A target's probability gets the same on the pairs' hits. A
sequential stop judges the same numbers. The standard deviation is always across trials: it
describes the output, not the precision of its mean. Where every value the interval is across
is the same, `half_width` is `null` and `no_variation` is `true`: a width of 0 would claim the
mean is exact, which a run cannot know (a rare event the trials did not reach gives the same
picture). A sequential stop does not wait on such an output once another has met its precision
(more trials of the same value would not give it an interval), but it does not count it as met
either: `stop.met` lists the outputs within the share asked for and `stop.not_established` the
others, and the window and the command line say so by name. An outcome rarer than about 3 in n
trials (the rule of three) can go unseen in n trials: a 1-in-10,000 event does in 1,000 trials
about 90% of the time.

**Control variates** ([`stochcraft_mc::control`](../crates/mc/src/control.rs); Law, *Simulation
Modeling and Analysis*, 5th ed., §11.3; Glasserman, 2003, §4.1; Nelson, 1990). An output can name
`controls` (`McOutput::controls`, in `mc.output.set` as `[{"input": "labour"}, {"output": "base",
"expected": 5200}]`, in the file as `stochcraft:control` children of the output): an input, whose
known mean is `Dist::mean`, or an earlier output with the mean the modeller types. The mean of an
input is only known where the sampler draws exactly that distribution: nothing is clamped,
resampled or truncated (a normal input can go negative), and every kind is checked to draw its
stated mean over 200,000 trials (`crates/mc/tests/control.rs`); a constant, a distribution with no
spread and a count with a standard deviation above 10,000 (whose quantile function, used by Latin
hypercube, antithetic and Sobol sampling, is an expansion within 1 of the exact value) are
refused with `inexact_control` or `constant_control`, as are an unknown name, an output control
that is the output itself or later (`control_not_earlier`) and a repeated one. The controlled value
is Y − bᵀ(C − μ) with b the least-squares coefficients; fitting them on the trials they are then
applied to makes the estimate slightly biased, so they are fitted on a pilot, the first batch of
1,000 trials (the first half of a random run under 2,000, rounded to whole pairs for antithetic
sampling), which the controlled estimate leaves out; where trials are grouped by batch (Latin
hypercube, Sobol, correlated inputs) the pilot is the whole first batch, so one batch gives no
estimate, two give a mean and three give its interval. The unbiasedness is for a run of a fixed
number of trials: a sequential stop judges the same trials the estimate uses, so it can select on
them and bias the result even with the pilot left out. The coefficients are fitted on columns
divided by their pilot standard deviation (so a control in small units is not lost beside one in
large units), and a trial with no number in the output or a control, in the pilot or after it,
withdraws the estimate (`unavailable` says so), because the controls' known means are those of
every trial. An input mean must be finite (`exact_mean`). The controlled mean and its interval are Student-t across the remaining trials, batch means or pair
means, as the run's `interval_basis` says; a target's probability is controlled the same way (the
indicator of meeting it, with coefficients of its own), kept within 0 and 1. Percentiles, the
histogram and the extremes are not controlled, and a sequential stop is judged on the plain mean.
It is worked out in the summarise step of each output, one linear pass over its trials after a
fit on at most 1,000 rows, so it adds no `Stepper` phase for a caller to drive. Each result's
`controlled` gives the `controls` (`kind`, known `mean`, `coefficient`, the controls' `sample_mean`
over the trials used and `mean_mismatch`, set when the control's average over the run's independent
units (trials, batches or antithetic pairs, as for the interval) is more than five standard errors
of those units from the stated mean, which for an output control means a mistyped mean and a biased result), the
`pilot_trials`, the multiple `correlation` with the output on the pilot, and an `estimate` with
the controlled `mean` and `half_width`, the `uncontrolled_mean` and `uncontrolled_half_width` on
the same trials, `variance_ratio` (plain over controlled variance of the estimate), `narrowing`
(1 − controlled width / plain width; negative when the control made it wider; `null` where the
controlled interval has no width) and `helped` (at
least 10% narrower); its `target` has the same for the probability. With too few trials, or a
run of one batch where intervals are across batches, `estimate` is `null` and `unavailable` says
why. An input named only as a control is not reported as `unused_input`, and a replacing
`mc.inputs.import` drops the controls of inputs it removes, as it drops their correlations; a
control object with fields of both forms is refused. `controlled.mean_mismatch` is true when any
control's is: the controlled estimate is then biased, and neither the window nor the command line
lets it displace the plain headline (the window keeps the plain mean and chance and shows the
controlled ones beside them; the table labels them "biased, not used"). The window makes the
controlled results the headline where `helped` and no control is flagged, words a controlled chance as an estimate (never as a count of trials),
and says so in words otherwise; `stochcraft_engine::mc::control_inputs` lists the inputs that can be controls.

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
file (OASIS, 2015). It is opened, edited, run, saved and read through the engine and the command
line; the window shows, runs and edits it (the Model and Results tabs, `crates/ui-egui/src/sd.rs`, and
the Diagram tab, the canvas, `crates/ui-egui/src/sd_canvas.rs`).

The Diagram tab draws what `sd.get` gives (each variable's centre, size and, for a flow, its pipe, the
stocks it leaves and enters, and the links) and sends every change as one engine command, so it
undoes in one step: a drag on release is `sd.layout.set` (the engine moves the pipes at a stock),
an added stock or auxiliary is `sd.variable.add` with a `position`, an added flow is
`sd.variable.add` with `from_stock`/`into_stock`, a field of the side panel is `sd.variable.set`, a
link is `sd.layout.set` with `add_connectors`, and Delete is `sd.variable.remove`. The undo labels
are the commands' names, which `change_label` in `crates/ui-egui/src/lib.rs` words ("Add Variable",
"Edit Variable", "Remove Variable", "Simulation Settings", "Change Diagram"). The canvas keeps its
view, tool, selection and half-typed fields in `SdCanvas` and re-reads `sd.get` only when the
engine's revision changes.

```bash
stochcraft-cli run tests/models/sir.xmile                     # each variable at the end of the run
stochcraft-cli run tests/models/sir.xmile --format csv        # every variable over time
stochcraft-cli run model.xmile --format json --dt 0.125       # another time step
stochcraft-cli run model.xmile --dt-check                     # and again at half the step
stochcraft-cli run noisy.xmile --reps 500 --seed 3            # a model with a random function
```

The engine commands are `sd.open` (the text of an XMILE file; `file.open` also opens one from
disk), `sd.new` (an empty model: name, start, stop, `dt`, method, time unit), `sd.run`
(`replications`, `seed`, `dt`, `dt_check`, `time_unit`) and `sd.get` (the model's variables with
where each is drawn and the pipes of the flows, the information links and checks, and with
`variable` its values from the last run, marked stale if the model was edited since). `run`
refuses such a model, naming `sd.run`. In Rust: `stochcraft_sd::{read_xmile, compile, run}`;
`Engine::has_system_dynamics`, `Engine::system_dynamics_path` and
`Engine::system_dynamics_results_stale` (`crates/engine/src/sd_window.rs`) tell a caller that holds
the engine whether the document is one, what it was opened under and whether its last results are
out of date. The window saves it through `Engine::save_bytes` (XMILE) and `Engine::saved_to` like
a process model, with a `.xmile` or `.stmx` name (Save As never rewrites a chosen name: any other
is refused, nothing written); `Engine::save_drops` lists what a plain Save into the opened file
would leave out (the reader's notes, any of which means something was not kept or was run another
way, as the notes are free text), empty once the model has been written to it, and the window
asks first (the `NewFile` question, with Replace, Save As and Cancel); its unsaved state is `Engine::is_dirty`, and its
recovery copy is skipped (recovery copies are process models only). The window's run is a job
made a few replications a frame, as Monte Carlo's is: `stochcraft_sd::SdJob` (`new` checks the
run as `run` does, `step(n)` makes up to `n` whole replications, `progress`, `finish`; `run` is
`new` and `finish`, so there is one code path, and each replication's streams are keyed by the seed
and its number alone, so the results are the same however the replications are spread over steps).
The engine's `sd::SdJob` wraps it: `Engine::sd_start` takes `sd.run`'s parameters, and
`Engine::sd_results` makes the results the model's last (as `sd.run` does, so `sd.get` and the
stale marking behave the same) and journals an `sd.run` step; it is refused with `conflict` if the
model was changed since the job started (`SdJob::revision`), which the window checks every frame
and answers by cancelling the run. The window (`App::sd_poll`) steps one replication at a time for
about the Monte Carlo frame budget (8 ms) and paints the first frame as running before the first
replication, which may take long, is made. A replication is not interrupted, so stepping inside
one is not offered.

**Editing.** `sd.variable.add` (stock, flow or aux with its equation, units, a graphical function,
a stock's flows, a position; `into_stock` and `from_stock` attach a new flow), `sd.variable.set`
(equation, units, doc, a stock's flows, `non_negative`, graphical function, `rename`),
`sd.variable.remove`, `sd.specs.set` and `sd.layout.set` (move or resize shapes, set a flow's
pipe, add and remove information links). They are `Action` commands, so `batch` does not take
them. Each works on a copy of the model and compiles it: an equation that does not read, a name
that is not there, an algebraic loop or a duplicate name is refused with the compile error and the
model is as it was (units mismatches stay warnings). An equation can only name variables that
already exist, so add a flow's stock first, or add the variables with a placeholder equation and
set the real one. A rename rewrites every equation (by canonical name, in bare or quoted form) and
flow list that names the variable and moves its shape and links. A removal is refused while an
equation names the variable or a stock lists it, naming who; `force` clears the stocks' lists only.
An edit's undo entry is a snapshot of the whole `SdModel` before it (`Undo::Sd` in the engine, the
counterpart of `Op::SetModel`), on the same stacks as process edits; undo swaps it in and compiles
it. The model is dirty while it, diagram included, differs from the one opened or last saved.
`file.save` writes XMILE (`format` is `xmile`); an unchanged model is written as the bytes it was
read from, and saving a changed model into the file it was read from, when the reader noted
content it does not keep (a second view, display settings), is refused with `conflict` and the
list in `details.would_lose` unless `allow_loss` is `true` (a process model saved with `as_new`
over its own file that has notes is refused the same way). A journal that began with `sd.new` records its
parameters in the start record and replays from it.

**The diagram** is `SdModel.layout`: a shape per variable by canonical name (the centre; a stock
and an auxiliary have a size, a flow's position is its valve on its pipe of right-angled points)
and the information links. A link is added for each use of one variable in another's equation, and
follows the equation when it is set. A model read from a file with no diagram, or built without
one, gets `Layout::auto`.

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
- **Diagram:** the first `stock_flow` view: the `x`, `y` (top left when the shape has a `width`
  and `height`, else the centre), size, a flow's `pts` and the `connector`s. A cloud is the end of a
  flow's pipe that no stock holds. Written back with the model.
- **Not kept, and said so in `notes`:** a second view or one of another type, the colours, label
  places and other display attributes of a shape, aliases, styles, display settings of a variable,
  another vendor's extensions and groups.
- **Extensions of StochCraft:** `stochcraft:save_step` in `sim_specs` (the interval results are
  saved at, by default the step), and a random function's fourth argument, the interval between
  draws: `NORMAL(mean, std_dev, seed, interval)` (by default one draw a time step of the file).

**Numerics.** Time is `start + n * DT` from an integer counter. Variables are worked out in
topological order of what their equations name, ties broken by name; an algebraic loop is an error
naming the cycle. A stock that cannot go negative limits its outflows, in the order it lists them,
to what it holds at the start of the pass, so the stock that receives a flow gets the same
limited flow and material is conserved. A flow that comes out negative runs backwards, and the
stocks it enters are the ones it takes from: they limit it (in the order the flows are worked
out), so no such stock goes below 0 and nothing is clipped afterwards. A flow that is not a
number is not limited (the limit would turn it into a transfer): it stays not a number, with a
warning, and so do the stocks it touches. A flow listed twice at one stock is refused. Under
Runge-Kutta a `STEP` on a step boundary is integrated as under Euler (the stage at the end of a
step sees the value from the left); a jump inside a step, or made with `IF` on `TIME`, makes that
step first order: put jumps on step boundaries, use Euler, or lower the step. `DELAYn` and `SMTHn` are hidden stocks (their time
constant is never taken below the step), `PULSE` delivers its volume in one step under either
method, a graphical function is clamped at its ends with a warning, and a random function holds one
draw for its interval and takes it from a stream keyed by variable, call and draw number, so
results do not shift when another variable is added or the step is halved. A model with no random
function runs once; one with a random function runs `replications` times (100 by default) and
gives the mean, the median and the 5th and 95th percentiles at each saved time. Units are checked
on both sides of each equation (compound units, and `1`), and a flow against its stock's units per
the model's time unit; a mismatch is a warning. `time_unit` reports times in another unit.

`dt_check` runs the model again at half the step, to the same end time, and gives, for each stock,
the largest relative difference between the two mean paths (relative to the larger of the value
and 0.1% of the stock's peak), compared at the saved times the two runs share; within 1% is
called independent of the step. A stock with no shared time after the start, or a value that is
missing or not a number at one, is `inconclusive` (and so is the check, which is then never
`within_threshold`), not a difference of 0. `draws_matched` is false when a random function draws
at different times at the two steps (its interval is a whole number of steps, so a half step can
move it); the random values are then not the same ones and `notes` says so. `time_unit` converts
`dt` and the halved `dt` as well as the times. XMILE elements are recognised by their namespace
URI, whatever prefix the file uses; a file whose root is in another namespace is refused.

### Agents

`crates/agents` holds M4.3's first three stages: state charts (A1), populations of agents
exchanging messages (A2), and networks and grids (A3), built in code, with no files yet. A chart is a `ChartSpec` (the tree of an SCXML
document: states, parallel and final states, shallow and deep history, transitions with event
descriptors, conditions, targets and the type internal or external, and `onentry`, `onexit` and
transition content of `<assign>`, `<raise>`, `<send>`, `<cancel>` and `<log>`). `Chart::new`
checks it (targets, ids, declared variables, `In()` names) and compiles it; an `Agent` runs it by
the algorithm of SCXML's Appendix D, and a `Driver` runs agents on `stochcraft_des::EventQueue`,
each event handled by one complete macrostep in zero time (D18).

- Expressions are BPSim's (`stochcraft_model::expr::parse_with_functions`), reading variables as
  `$x` or `getProperty('x')`, with `In('state')`, `time()` and the distribution functions (`exponential(m)`,
  `uniform(a, b)`, `triangular`, `normal`, `erlang`, ... and `random()`), each expression drawing
  from its own stream `Rng::keyed(seed, "<n>:<agent key>/<place in the chart>")`, `<n>` the key's
  length in bytes and each state id in the place likewise written after its length (D17, D18).
- A `<send>` to the agent itself has a fixed `delay`, a `delayexpr`, or a rate (`SendSpec::at_rate`, an
  extension): an exponential delay with mean 1 / rate, drawn again when a variable the rate reads
  has changed at the end of a macrostep. `<cancel>` leaves the event on the queue, skipped when
  popped. A send may carry a number (`SendSpec::with_data`), read as `getProperty('_event.data')`.
- A macrostep that takes more than 10,000 microsteps (`Agent::with_microstep_limit`) stops the run
  with `RunError::MicrostepLimit`, which names the latest transitions.
- A failing action (a negative delay or rate) raises `error.execution` and skips the rest of its block, as SCXML §4.9 says.

A `Model` (module `population`) holds populations of agents of one chart each, an optional
environment agent, statistics and an optional stepped mode; a `Simulation` runs one replication
from a seed, and `Model::run` returns its `Run` (the statistics over time and each population's
`Counts`).

- Agents are created at the start (a count), by arrivals at a rate (exponential gaps from the stream
  `arrivals/<n>:<population>`), or by another agent (`Action::create`); each gets the next serial
  number of its population and the key `<population>/<serial>`, so its streams follow D18 and adding
  an agent never changes another's draws. An agent leaves when its chart reaches a top-level final
  state, or when another agent removes it (`Action::remove`, which runs its exit actions).
- Messages go to one agent (`SendSpec::to_agent`, the serial an expression), a random other agent
  (`to_random`, the pick from the sender's own stream) or every other agent (`to_all`), each as an
  event of its own at the current time after the current one, a broadcast in serial order. They take
  no delay. A message no transition takes is counted as dropped, one that reaches no agent as
  undelivered.
- `serial()`, `population-size('p')`, `population-count('p', 'state')`, `population-sum('p', 'x')`,
  `population-mean('p', 'x')` and `agent-value('p', n, 'x')` read populations, with their names as
  quoted literals, checked when the model is built. An eventless condition that reads a population is
  tested again after every event that changes it (an agent created, removed or taking a transition):
  the agents waiting on it in population, then serial order, then the environment, until nothing
  changes (at most 1,000,000 changes per event, `RunError::RetestLimit`). A rate may not read a
  population or the time, since it is drawn again only when the agent's own variables change.
- `Statistic`s (count, sum, mean, minimum and maximum of an expression over the agents meeting a
  condition) are recorded at the start, every `save_every` and at the end, after every event at that
  time; `bands` gives the mean, median and 5% to 95% band across replications.
- The stepped mode (`Stepping`) sends, every step, `before-step` to the environment, `before-step`
  then `step` to every agent (in population, then serial order, or one random order per step from
  the stream `stepping/order`), and `after-step` to the environment.

A population can have a `Network` and a `Grid` (module `network`, stage A3):

- Networks are built at the start, over the agents created then, from the population's own stream
  `network/<n>:<population>`, so other populations never change them: `Network::random(d)`
  (Erdős–Rényi G(n, M) with M = round(n·d/2), drawn by Floyd's algorithm, so the mean degree is d
  exactly when n·d/2 is whole), `ring(k)` (k links each side), `small_world(k, p)` (Watts and
  Strogatz 1998: each lattice link's far end rewired with probability p, lap by lap round the
  ring, never to the agent itself or to one it is linked to), `scale_free(m0, m)` (Barabási and
  Albert 1999: a complete network of m0, then each agent linked to m distinct earlier ones with
  probability proportional to degree), `distance(w, h, r)` (positions uniform in a w × h
  rectangle, in the variables `x` and `y`, linked within r; every pair compared), and a list of
  links, `links(...)` or `csv(text)` (`read_links` refuses a bad line, a self-link, a link given
  twice and a serial of no agent, with its line number; `write_links` writes them back). Links are
  undirected, never to the agent itself and never twice. Agents created later have no links (in a
  distance network they get a position and the links within the range), and an agent that leaves
  loses its links.
- Grids have rows × columns cells, Moore or von Neumann neighbours, wrapped or not, one agent per
  cell or many; agents are placed in order or at random (stream `grid/<n>:<population>`), their
  cell in the variables `row` and `column`. An agent's neighbours are the agents in its
  neighbouring cells and the others in its own cell.
- `Action::connect`, `disconnect`, `move_to` and `move_random` take effect when the acting agent's
  macrostep ends, in the order its actions ran; `SendSpec::to_linked`, `to_linked_random`,
  `to_neighbours` and `to_neighbour_random` send along links and to neighbours, in serial order,
  the random pick from the sender's own stream as for `to_random`. `degree('p')`,
  `linked-count('p', 's')`, `linked-sum('p', 'x')`, `neighbours('p')`, `neighbour-count('p', 's')`
  and `neighbour-sum('p', 'x')` read an agent's neighbourhood in conditions and statistics; a
  condition that calls them is tested again when the population changes, links made and removed
  and moves included; an agent that moved is itself tested again first (its conditions on `row`
  and `column` taken, its rates that read them drawn again). An agent that ends keeps its links and
  cell while its last messages go out, in order, then loses them. A model whose charts name a
  network or grid their population lacks, whose grid or distance network has no variables for the
  cell or position, or whose chart assigns those variables (they are the model's), is refused.
  Bounded draws use all 64 bits with rejection (unbiased), and distances are scaled so they neither
  underflow nor overflow.

The tests (`crates/agents/tests`) work out the order of actions by hand for transitions between
cousins, branches, history, parallel regions, internal transitions and conflicting transitions,
and check fixed timeouts, cancels, and rates (and redrawn rates) against the exponential by their
mean and a Kolmogorov–Smirnov test, with fixed seeds. For populations they check broadcasts,
random picks, re-tested conditions, removal, arrivals and the stepped mode's phases by hand, and
four models against exact finite-population answers: independent failures (binomial at t = 5),
Bass diffusion as agents (the pure-birth chain's mean times to half and to all adopters; N = 1,000
in the heavy tests), SIR's final size for N = 100 and R0 = 1.5 (the embedded jump chain), and
Pólya's urn in the stepped mode (uniform). For networks and grids they check a ring lattice of 100
with 2 links each side (clustering exactly 1/2, a mean shortest path of 127,500 / 9,900 = 12.8788
by breadth-first search), the small world with rewiring 0 (the lattice) and 1 (as many links, none
repeated), the random network's exact mean degree, preferential attachment's least degree m and
its tail exponent (the maximum-likelihood estimate against its value for the exact degree
distribution 2m(m + 1)/(k(k + 1)(k + 2)), whose tail falls as k^−3), the distance network pair by
pair, links through CSV and back, links made and removed during a run, Moore and von Neumann
neighbourhoods by hand, moves, messages to neighbours, a Game of Life glider that is its own shape
moved one cell diagonally every 4 steps, and SIR on a complete network, which is A2's well-mixed run
draw for draw and matches its exact mean final size of 20.3696. Not there yet: `<if>`,
`<foreach>`, `<invoke>`, `<donedata>`, `<initial>` elements with content, late binding, and reading
and writing SCXML files (A4).

### Selecting the best

`stochcraft_experiment::select` holds M8's first stage (O1): procedures that choose the best of a
few systems, or decide which meet constraints, with a stated probability of being right. They are
model-free: each takes a sampler `FnMut(system, replications) -> values` (one row of values per
replication for the constraint procedures), so the same code runs on simulations and on
synthetic normal data. Every system is asked for replication j under the same index, so a sampler
that seeds replication j alike for every system gives common random numbers, which all of them
allow.

- `select_best` is Kim and Nelson's fully sequential procedure KN (2001), with its constants
  η = ½ [(2α/(k − 1))^(−2/(n₀ − 1)) − 1] and h² = 2η(n₀ − 1) as Tsai and Nelson restate them, and
  the pairwise variances of differences. It chooses the best, or one within δ of it, with
  probability at least 1 − α (`Kn`: δ, confidence, n₀, sense, sampling, `max_runs`).
- `check_feasibility` is Batur and Kim's check F_B (2010) for several constraints on means, each
  with a limit and a tolerance ε (`Constraint`); each system is `Feasible`, `Infeasible` or, when
  the budget runs out, `Undecided`.
- `select_feasible` runs the check, then KN among the systems found feasible on fresh
  replications, with α split between them (half each by default), so the stated confidence
  covers both.
- `compare_with_standard` is Kim's comparison with a standard (2003): system 0 is the standard,
  kept unless an alternative is better; it is the local-optimality test O2 needs.

The answers say what they claim: `Outcome::Selected` with the confidence, `ComparedExactly` when
every first-stage value of every system is the same (no randomness; no probability stated), and
`NotProven` with the systems still in contention when `max_runs` ran out, beside the chosen
system, the replications and the mean of each system. All use the triangular region with c = 1.
Kim's 2003 paper prints its elimination bound as max{0, −a + λr}; the code uses min{0, −a + λr},
Kim and Nelson's region, since the printed form would eliminate both systems of a close pair.
The checks (`crates/experiment/tests/select.rs`, heavy) run 2,000 macro-replications each: KN in
the slippage configuration (k = 10, n₀ = 10, independent and with correlation 0.5, which must
use fewer replications), every mean within δ (replications bounded by where the region closes),
the check with every system ε from its limits, KN among the feasible, and the comparison with a
standard with all means equal and with one alternative δ better. A rate passes when the upper end
of its 95% Wilson interval is at least 0.95; the log prints each rate, interval and mean runs.

### Racing and the clean-up

`stochcraft_experiment::race` holds M9's second stage (P2): racing the candidates a search
proposes, then the clean-up of the finalists by KN. It is model-free and works with any search,
M9's or an outside one: a `Race` takes candidates as they come (`add`), says which replications
to run next (`ask`, a `Request` of a candidate and a range of replications) and takes their
values (`tell`); `run` drains the requests with a sampler, and `race` races a fixed set of
candidates. Each candidate gets a first stage, then more stages while it stays in contention. It
is dropped only when the one-sided paired t interval of the leader (the best mean in contention)
against it, over the replications they share, lies wholly beyond δ: racing (Birattari, Stützle,
Paquete and Varrentrapp 2002) with a paired t interval in place of their Friedman test, since
common random numbers pair the replications.

The race's error is held by Bonferroni's inequality over the planned looks and pairs: the race is
planned for at most M candidates (`max_candidates`, more are refused) and L stages each
(`stages`), and each test is run at β = α_race / ((M − 1)·L). The best candidate tried can only be
dropped against one of the M − 1 others at one of the L shared counts, so the race drops it with
probability at most α_race, whichever candidate leads at each look and in whatever order the
requests are told (that order can change who leads, which pairs are tested and who reaches the
clean-up). The bound treats each paired test as a fixed test on fixed data, so it holds only for
candidates chosen without looking at the values told on the replication indices they will share:
candidates fixed in advance, or proposed from a fixed design, a random or Sobol sequence or a
list. An adaptive search that reads the race's results and then proposes a candidate to run on
the same indices breaks it, and then the bound on dropping the best is not claimed. The paired t
interval and KN also need the vectors of all candidates' values in a replication to be
independent across replications and jointly (multivariate) normal; `Sampling::Common` only sets
KN's constants and does not remove that requirement.

`clean_up` then runs KN (`select_best`) among every candidate still in contention, on
replications numbered from the first index the race never asked for, at confidence 1 − α_clean.
α is split between the two (`racing_share`, half each by default). KN's statement about its
finalists holds whatever chose them. The answer (`RaceAnswer`, also as words) states the chosen
candidate, "of these k finalists, the best or within δ of it, with 1 − α confidence", how many
candidates the search tried and racing dropped, and that it does not prove that none of the
others is better. `finalists` can cap the clean-up at the k best race means (`None`, no cap, by
default); that cut has no probability behind it, so a capped answer lists the candidates it left
out (`left_out`) and says that its confidence covers only the finalists. Candidates whose first
stage never ran (the racing budget ran out, or the clean-up was called while they waited) are
listed in `untested`, and the answer says that its confidence covers only the candidates that
raced. KN's `ComparedExactly` and `NotProven` pass through. `max_race_runs` stops the race early
and the clean-up runs on what is left; a `max_cleanup_runs` that cannot pay for KN's first stage
gives a not-proven answer. The checks (`crates/experiment/tests/race.rs`, heavy) run 1,000
macro-replications each: the slippage configuration (ten candidates, independent and with
correlation 0.5) and a feed of 50 candidates arriving one at a time with the best arriving late,
where racing must also use fewer runs than running every candidate to the end with the same
clean-up, both without a cap. Since the procedure is conservative, each rate must itself be at
least 0.95; the log prints it with its 95% Wilson interval.

### Whole-number decisions

`stochcraft_experiment::compass` holds M8's second stage (O2): COMPASS (Hong and Nelson 2006)
with the pruning, local-optimality test and clean-up of Industrial Strength COMPASS (Xu, Nelson
and Hong 2010), model-free on a sampler `FnMut(stream, design, replications) -> rows` (each row
the objective, then one value per constraint). A design is a vector of whole numbers in a box
(`Space::bounds`) under linear limits (`Space::limits`, a cost cap).

- *The most promising area* around the best visited design x̂* is every design at least as close
  to it as to any other visited design: the halfspaces 2 (y − x̂*)·x ≤ ‖y‖² − ‖x̂*‖², in whole
  numbers. New designs are sampled from it by a walk over the coordinates (ten moves per
  decision, each a whole number uniform over what the coordinate may take).
- *Pruning*: a visited design whose halfspace is implied by the others, the box and the limits is
  not active. A small linear programme (simplex with Bland's rule, on the area relaxed to real
  numbers, so a design is only dropped when it is surely implied) tests each one. Only x̂*, the
  active designs and the new samples get replications.
- *Replications*: at step k of a start, every active design is brought to n(1 + ⌊ln k⌋), n the
  replications per visit (10). That grows without bound, as COMPASS's convergence proof needs,
  and slowly.
- *Local optimality*: when every neighbour of x̂* has been visited, the area is x̂* alone, and
  Kim's comparison with a standard (`select::compare_with_standard`) decides whether x̂* stays.
  It reuses the search's replications, so it is a stopping rule. The guarantee is the clean-up's.
- *Starts*: the starting design and a few Latin hypercube points (`optimize/start/<id>.<field>`
  streams), each with its own visited set, taking a step each in turn and sharing replications.
- *The clean-up*: the search's best, the designs active around it, each start's final design and
  the starting design, on fresh replications (`Stream::CleanUp`), by `select_best` or, with
  constraints, `select_feasible` (α split equally). A space is not searched, and every
  design goes to the clean-up, only when it can be counted (its ranges hold at most a million
  points, so a box larger than that takes the search path even when the cap leaves few designs),
  it has at most `all_designs_up_to` (50) designs, and the budget pays for the clean-up's first
  stage (10 replications) of every one; otherwise it is searched. Without randomness each design
  runs once and they are compared by value.
- *The budget*: `search_runs` caps the search and the rest of `max_runs` is the clean-up's. A step
  that does not fit ends the search, and nothing runs past the cap. The caller's `stop` (a time
  limit) is asked only after steps that ran replications, so a search it stopped after S runs is
  repeated exactly with `search_runs` = S.

`engine::optimize` wires it to models as `experiment.optimise`. Each design is the model with each
decision set by `Param::set`, compiled once and run on the search's seed, or on
`Rng::keyed(seed, "optimize/clean-up")` in the clean-up. The objective of a replication is the
weighted results plus the decisions' costs. The cost cap is a limit of the space, so no design
over it is run. A model is compared exactly only when nothing in it is drawn and the starting
design's first two replications agree. `engine/src/drawn.rs` decides the first from the model's
types, taking every struct apart and matching every kind of element in full, so a new field or
kind does not compile until it says whether it draws. Drawn means a distribution other than a
constant, a probability strictly between 0 and 1, an exclusive gateway whose way is not decided
by conditions, balking, scrap, a mix of values or an expression; a name never counts. An
applied answer is journalled as the search without `apply` and a `decisions.set` edit, inside a
batch too, and an edits-only replay of a batch leaves out the steps it skips. The answer gives the claim in words and fields, the chosen
design's objective with its interval, the paired difference against the starting design, each
constraint's decision, the finalists, the trace and the runs. A time limit is refused in the
browser build, which has no clock here.

The checks (`crates/experiment/src/compass.rs`'s unit tests, and `crates/engine/tests/optimise.rs`,
whose heavy tests run 100 seeds each) cover: pruning that leaves the area exactly as its
definition gives it on random lattices, and walks that reach all of the area. On the two M/M/c desks in tandem
(Erlang C optimum (14, 8) at 475.71 an hour), the search must choose it or a design within
δ = 2, and under a cap of 21 servers (13, 8) or (14, 7). On a near tie on one desk, δ = 0.1 must
give 6 servers or "not proven". The fewest servers for a mean wait of at most 0.05 hour must be
14. Constant times must give "compared exactly", and a search stopped by its time limit must
repeat exactly. Each rate must itself be at least 0.95; the log prints it with its 95% Wilson
interval.

### The journal

The engine records every command that is not a query in its journal. A new or opened document
(`model.new`, `file.open`, and opening a file in the app) starts a new journal, so a journal is
the history of one document. `exec --journal out.json` writes the journal of what the script did,
also when a step failed (atomically, and never over the script, the file given with `--open` or
the file the script opened or saved: those are refused), as a JSON object:

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
  apply. The command that started the document is not an entry: `start` records it. `document` is
  `process` or `system_dynamics`; for a system dynamics model `model_digest` is the digest of the
  model without its diagram (the one its run records carry), not of an empty process model. A
  journal written before `document` existed is a legacy one: a process journal replays as before,
  a legacy journal that began with `sd.new` rebuilds its model from that step, and one that began
  from an opened XMILE file cannot vouch for its model and is refused unless `--force`.
- `engine` names the build that wrote it (the version, the platform and, when the build was given
  `STOCHCRAFT_REVISION`, the source revision). A replay prints, on stderr, how many edits it
  replayed and how many runs it made again, and whether this is the same build (`build_match`:
  `same` only when both ids carry the same source revision, `different` when the version, platform
  or revision differ, `unknown` when a revision is missing, since one version on one platform can
  be built from commits that sample differently): runs are made again, not compared with the
  original numbers, and only the same build is expected to give the same ones. Release and Pages
  builds are stamped with the commit they were built from; a local build is not, unless
  `STOCHCRAFT_REVISION` is set.
- `entries` are the commands in order, as `{"command", "params"}`. A script for `exec` is the same
  array on its own, and `exec` runs that in full.

Replaying a journal (`exec journal.json`) first checks that the model is the one it started from
(`--open` the same file, which opens the scenario the journal records in `start.scenario`; a
journal that began with `model.new` starts its own model) and refuses
with a `conflict` error if not, unless `--force`. The same model from other bytes (the file was
edited since) and a document of the other kind are refused too. If the file has no scenario of that name, `exec`
stops with the error from opening it; with `--force` it replays onto the file's first scenario.
It then re-applies the edits, undo and redo, and
skips the saves, runs and experiments (`file.save`, `run`, `mc.run`, `mc.sobol`, `sd.run`, `run.start`,
`run.cancel`, `experiment.sweep`, `experiment.sensitivity`, `experiment.design`,
`experiment.size`, `experiment.evaluate`, `experiment.optimise`), printing `"skipped": true` for each; `--all` carries them out too, and a
replayed `run.start` is run to completion so later steps find its results. In Rust,
`Engine::journal_document` gives the journal and `Engine::replay_journal` replays it
(`ReplayMode::EditsOnly` or `All`); `Engine::replay` still runs a plain list of steps in full.

## Using it from an AI agent

`stochcraft-cli mcp` serves the engine's commands over the Model Context Protocol, so an agent
such as Claude Code can open, build, run and compare models:

```bash
claude mcp add stochcraft -- /path/to/stochcraft-cli mcp
```

Start the app with `stochcraft --control 7070` and add `--connect 127.0.0.1:7070` to watch the
agent work in the window; the app writes a token to a file only its user can read, and `--connect`
reads it from there. [mcp.md](mcp.md) has the tools, the options and how the token works.

## How it is checked

Results are checked against queueing theory and exact results: M/M/1, M/M/c (Erlang C), M/M/1/K,
priority queues, machine repair, a Jackson network, a two-machine line with a finite buffer,
E₄/M/1 batches, assembly, arrivals by time of day, breakdowns, activity networks and two servers
that differ in speed behind one queue (Able and Baker, a discrete-time Markov chain). Monte Carlo
models are checked against exact answers: the sum of independent normals, the product of
log-normals, a PERT schedule's finish on a single path, rank correlations induced against their
targets, and Latin hypercube strata holding one value each. They are
also checked against another BPSim simulator's published results for the BPSim car repair model. StochCraft runs
about 54× faster than SimPy on an M/M/1 benchmark ([bench/](../bench/README.md)).

Some of the formulas behind the theory values are also proved, independently of the code:
[formal/](../formal/README.md) is a Lean 4 project with Mathlib that restates them by hand and
proves them (the Erlang B recurrence against its closed form, Erlang C in the tests' form and from
Erlang B, the M/M/1 geometric distribution's sum, balance equations and means, Little's law on one
sample path, and algebra about the Pollaczek–Khinchine mean wait). It proves those restatements
over the reals, not the values the Rust tests compute, their floating-point arithmetic or the
simulator; its README says what each theorem does and does not establish. A separate CI workflow
(`.github/workflows/lean.yml`) builds it when `formal/` changes.

### Machine-checked proofs

A few invariants are proved, not only tested, with [Kani](https://github.com/model-checking/kani)
(a bounded model checker for Rust). The harnesses sit in `#[cfg(kani)] mod proofs` beside the code
they check, so normal builds never see them. CI runs them in `.github/workflows/kani.yml`, on pull requests that change `crates/` and nightly on main; locally,
after installing Kani (`cargo install --locked kani-verifier && cargo kani setup`):

```bash
cargo kani -p stochcraft-random
cargo kani -p stochcraft-stats
```

Proved, for every input the harness allows:

- Random streams (`crates/random/src/lib.rs`): seeding never gives the all-zero state and a
  xoshiro256** step keeps a non-zero state non-zero; `uniform` is in [0, 1) and `uniform_open0`
  in (0, 1] from any state.
- Distribution parameters (`crates/random/src/dist.rs`): when `Dist::check` accepts a Uniform,
  Triangular, Truncated normal, Exponential, Poisson, Normal, Log-normal, Erlang, Gamma, Beta,
  Weibull or Binomial, its parameters are finite and in the documented ranges, for any `f64`
  including NaN and infinities; a constant never samples NaN, infinite or negative.
- Output analysis (`crates/stats`): an observed value agrees only inside its tolerance and
  differs only outside it, and never agrees without a usable tolerance; a bound passes only when
  the mean meets it and fails only when the mean violates it; fewer than two replications are
  always inconclusive; the 95% critical value exists for 1 to 30 degrees of freedom; the
  half-width of two replications of moderate size (up to 1e6) is finite and not negative.

Not proved: that different seeds, bases or keys give different streams (the 64-bit multiplications
of SplitMix64 are too slow for the checker; key strings also hash to 64 bits, so collisions exist), the statistical quality of the generator, the shape of any distribution's
samples or the bounds of `uniform_range` and `triangular` (floating-point arithmetic on symbolic
values is beyond the checker in CI time; transcendental functions, rejection samplers and loops
are out of its reach too; the statistical tests cover them), confidence intervals for more than two replications,
and anything in the engine, the block runtime or the interface (entity accounting is checked by
the run-time behaviour checks, not proved). Proofs are bounded by the stated input ranges and by
the checker's model of floating point.

### Pull request reviews

Two automated reviewers comment on pull requests. CodeRabbit is a GitHub app and needs nothing
from the repository. The Gemini CLI review is the workflows `.github/workflows/gemini-dispatch.yml`
and `gemini-review.yml` (adapted from the `run-gemini-cli` examples, which are Apache-2.0). It
stands in for CodeRabbit: it runs by itself only when CodeRabbit's comment on a pull request from a
branch of this repository says it is rate limited, and then at most once per head commit (its
acknowledgement carries the commit, and a second "rate limited" edit for the same commit is
skipped), which also keeps it inside the free tier's daily requests. It also runs when a repository
owner, member or collaborator starts a comment or review with `@gemini-cli /review` (any text
after it is passed to the reviewer as extra focus, for example `@gemini-cli /review check error
handling`), or when someone with write access runs the "Gemini Dispatch" workflow by hand with a
pull request number. Pull requests from forks are not reviewed, because a fork's run must not see
secrets. The review needs a repository secret named `GEMINI_API_KEY` (a Gemini API key from
Google AI Studio); without it the run fails and posts a failure comment. It posts with the
workflow's own token and reads the pull request through GitHub's MCP server in a container. Its
comments are suggestions, not checks: CI is the gate.

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for the clean-room and licence rules.
