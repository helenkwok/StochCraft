# StochCraft user guide

StochCraft simulates how work flows through a process: parts through a line, patients through a
clinic, requests through a service. You draw the flow, say how long each step takes and how much
it varies, and StochCraft runs the model many times and reports each result with how sure it is.

This guide covers the app, in the browser or on the desktop; both work the same way. Where they
differ, it says so. To get the app, see the [installation guide](installation.md).

- [Words used in this guide](#words-used-in-this-guide)
- [Start: open an example](#start-open-an-example)
- [Build a model](#build-a-model)
- [Set up the experiment](#set-up-the-experiment)
- [Run it](#run-it)
- [Read the results](#read-the-results)
- [Watch a run](#watch-a-run)
- [Other views of the model](#other-views-of-the-model)
- [Ask what-if questions: the Analysis view](#ask-what-if-questions-the-analysis-view)
- [Monte Carlo models](#monte-carlo-models)
- [Save, open and export](#save-open-and-export)
- [Change how it looks and which keys do what](#change-how-it-looks-and-which-keys-do-what)
- [Keyboard shortcuts](#keyboard-shortcuts)
- [Troubleshooting](#troubleshooting)
- [Limitations](#limitations)

## Words used in this guide

- **Block:** one step of the process on the canvas: arrivals, a task, a decision, departures.
- **Entity:** what flows through the blocks: a part, a patient, an order.
- **Run:** one pass of the model from start to end with its own random numbers (textbooks call
  it a replication). An **experiment** is all the runs together.
- **95% confidence interval (± 95% CI):** the range the true mean most likely lies in, given the
  runs. More runs make it narrower.
- **Warm-up:** time at the start of each run that is simulated but not measured, so an
  empty model filling up does not distort the results.
- **Utilisation:** the share of the time a task's units are busy, counted over the time they are
  on shift. Busy means processing or setting up: a unit that breaks down part-way through a job
  is not busy while it is broken, and one holding a finished entity it cannot pass on (blocked)
  is not busy either.

## Start: open an example

When StochCraft opens with no model, it asks **What do you want to find out?** Each example is a
card with its question, its field, about how long it takes, a strip of its blocks, what it
teaches, and, where its answer has been checked, a tag: **Checked against queueing theory** or
**Matches a published study**. Click a card to open its example. The question buttons (**Where is
the bottleneck?**, **How many people or machines?**, …) and the **Industry** list narrow the
cards; **See all examples** adds the reference models. Every example with a tag has a test that
runs it and compares its results with the exact answer (`crates/engine/tests/examples.rs` and
`benchmarks.rs`).

Beside the examples:

- **New to simulation?** **Build a first model** opens an empty canvas with Getting started in
  Help beside it, and the guide points at the first step (see Get help in the window).
- **Coming from another simulation tool?** **Read the two-page guide** opens
  [Coming from another simulation tool](switching.md): the usual words mapped onto StochCraft's,
  how runs and results work here, and how to bring in a process diagram.
- **Open a file:** **Open a file…**, your recent files (on the desktop), and **Start from a blank
  model**.

The examples are also under **File ▸ Open Example**:

| Example | What it asks |
| --- | --- |
| **Coffee counter** | One barista or two at the morning rush? Checked against the M/M/1 and M/M/2 (Erlang C) formulas. |
| **Emergency triage** | Who waits how long when urgent patients go first? Checked against Cobham's priority-queue waits. |
| **Call centre** | How many callers hang up before an agent answers? Checked against the M/M/c+M (Erlang-A) chain. |
| **Weld and paint line** | Where is the bottleneck, and what do breakdowns at paint cost? Checked against the line's exact Markov chain. |
| **Web service with health checks** | Do health checks that restart busy servers make the queue worse? Checked against the service's exact Markov chain. |
| **Bank with an ATM and cashiers** | Are five cashiers enough when half the customers use the ATM first? From Borshchev and Grigoryev, *The Big Book of Simulation Modeling*, Example 2.2; checked by flow balance (cashiers 91% busy) and, at the one ATM, the Pollaczek–Khinchine wait. A second scenario adds a sixth cashier. |
| **Hospital beds** | How many beds so that fewer than 1 in 50 patients are turned away? Checked against Erlang B, with log-normal stays and, in a second scenario, exponential ones of the same mean; a third has 28 beds. |
| **Machine repair crew** | How many fitters keep six machines running? Checked against the finite-source queue M/M/R//6, with one, two or three fitters as scenarios. |
| **Inspection with rework** | How much longer do parts take when half of them need rework? Checked against the product form of a Jackson network; a second scenario reworks a quarter. |
| **Project finish date** | When will the project finish? Three ten-day activities in parallel, run until the project ends; checked against the maximum of exponential durations (18.3 days on average). |
| **Same mean, different spread** | Does it matter how much service times vary when the average is the same? Three desks with fixed, exponential and very uneven times, checked against Pollaczek–Khinchine (waits of 2, 4 and 10 minutes). |
| **Polling place** | How many voting machines does a polling place need? From Allen, *Introduction to Discrete Event Simulation and Agent-based Modeling*, ch. 6 and 7: ballots of 12.5 or 6.5 minutes, machines allocated by the book's utilisation rule (18 for the long ballot, 10 for the short); checked against Erlang C (a 1.5-minute wait with 18 machines, 9.1 with 16). Two election-day scenarios run until the last of 936 voters has voted; they show the direction only and are not checked. |
| **Mass vaccination hub** | How many doses a day can a vaccination hub give? The inputs of Hanly et al., *BMC Health Services Research*, 2022 (a small hub of 21 staff and a large one of 63); each station's utilisation checked by flow balance, all under 80%, and about 470 doses in an 8-hour day for the small hub. Arrivals are random rather than by appointment and walking times are left out, so the time inside is not checked against the paper. |
| **Prison population** | How many prison places are needed? The case of Brailsford, Churilov and Dangerfield, ch. 8, scaled down 100 times, starting at its equilibrium; the numbers inside checked against the infinite-server network's means (500 petty and 260 serious offenders). A second scenario has fewer petty offenders reoffend. |
| **Penny Fab** | How much work in process does a line need? Hopp and Spearman's four-station line with a fixed number of jobs going round; checked against the practical worst case with random times (1, 4, 8 and 16 jobs) and the best case with fixed times. |
| **M/M/1 queue** | How long do customers wait at one busy clerk? Checked against queueing theory. |
| **Four-machine line with weekend breaks** | What do failures and weekend breaks do to a line's output and energy per part? Schmitt et al. 2026, Case A: matches the published throughput. |
| **Seven-machine line with two presses** | Which machine holds back a line with two presses in parallel? Schmitt et al. 2026, Case B. |
| **Tandem line** | Two stations in series, read from an XPDL file (a reference model). |
| **Every distribution** | One task per BPSim distribution, to compare their spread (a reference model). |
| **Parcel sorter** | How many parcels go round the loop because their lane is full? |

To try the whole cycle, open **M/M/1 queue** and press **Run**. The results show a mean wait close
to 3 minutes and about 75% utilisation, which is what queueing theory predicts for this model.

![The start page: examples by question, each with what it teaches and how its answer was checked, beside ways in for newcomers, people from other tools and opening a file](screenshots/start.png)

## Build a model

The window has three parts:

- the **left rail**, with the model's name, its blocks in order, and a summary of the experiment
  at the bottom;
- the **canvas** in the middle, where the flow is drawn;
- the **inspector** on the right, which shows the settings of what is selected. On a narrow
  window it moves under the canvas.

The tabs above the canvas switch between **Model**, **Results**, **Analysis**, **Capacity**,
**3D** and **Layout** (see [Other views of the model](#other-views-of-the-model)). **Dashboard**,
beside the tabs, opens a panel of charts beside the model.

![A model with a block selected: the canvas in the middle and the block's settings on the right](screenshots/model.png)

### Add blocks

Start a new model with **File ▸ New**. Then add blocks in either of these ways:

- **+ Add block** at the top of the left rail lists every kind.
- The strip above the canvas, **Drag to add:**, has one chip per kind. Drag a chip onto the
  canvas, or click it to add the block after the selected one.

A new block is added after the selected block and connected from it, and gets a name such as
"Arrivals 1". Rename it in the inspector's **Name** field.

| Block | What it does |
| --- | --- |
| **Arrivals** | Entities enter the model here, at random or fixed times. |
| **Task with a resource** | Work that needs one of the task's own units (a machine, a clerk). Entities wait in its waiting room when every unit is busy. |
| **Task with shared resources** | Work that takes units from a resource pool shared with other tasks. Add a pool first. |
| **Task** | A delay that needs no resource, such as cooling or travel. |
| **Decision (one way)** | Sends each entity down one of its flows, by probability or by a condition. |
| **Split or join (all ways)** | Sends a copy down every flow, or waits for one from each incoming flow. |
| **Batch (collect, then send together)** | Collects a number of entities and sends them on as one. |
| **Split batches** | Splits a batch back into its entities. |
| **Conveyor** | A belt that carries entities from one end to the other, with a **Length**, a **Speed**, an **Item length** and a **Gap**. **Accumulating** ticked: entities queue up at the end while the rest keep moving; unticked: the whole belt stops when the entity at the end cannot leave. |
| **Departures** | Entities leave the model here. |

### Connect blocks

- **On the canvas:** drag from the handle on a block's edge to another block.
- **In the inspector:** with nothing selected, the model overview's **Connections** section has
  **From** and **To** lists and a **Connect** button. With a block selected, its **Flows** section
  has **Add a flow from…** and **Add a flow to…**.

Each connection has a **Remove** button.

### Move around the canvas

- Click a block to select it. Shift-click or Ctrl-click adds to the selection, and dragging on
  the background selects everything in a rectangle.
- Scroll, or drag with the middle or right mouse button, to pan. Ctrl+scroll or pinch to zoom.
- The buttons at the bottom right zoom out, show the actual size, zoom in, and **Fit** the whole
  diagram in view.
- With several blocks selected, the inspector can **Align** them and **Spread out** them evenly.

### Set what each block does

Select a block to see its settings. The most used ones come first, under **Parameters**:

- **Arrivals:** **Time between arrivals**, and an optional **Arrival limit**.
- **Task with a resource:** **Capacity** (how many units work in parallel), **Service time**,
  **Waiting room** (unlimited unless set), **Transfer time**, **Setup**, and **Planned run time
  per item** for OEE.
- **Task with shared resources:** **Service time** and **Priority**; what it needs from the
  pools is under **Needs**.
- **Task:** **Duration**.
- **Batch:** **Batch size**.
- **Inside at the start** (under **More settings** on a task or a batch): how many entities are
  already inside when each run starts, instead of an empty start. They join the waiting room at
  time 0 in order: as many as there are free units start a fresh service at once, and the rest
  wait. With a finite **Waiting room**, at most the units plus the room. Results show them as
  **Inside at the start**, and count them as entered.
- **Conveyor:** **Length**, **Speed**, **Accumulating**, **Item length** and **Gap**. Lengths are
  in the experiment's **Length unit**, the speed in that unit per time unit.

**Times.** Each time is a distribution: pick its type (Constant, Exponential, Uniform, Triangular,
Normal, Log-normal, Gamma, Erlang, Weibull and others) and fill in its values. If you only know a
minimum, a most likely value and a maximum, use **Three-point estimate ▸ Estimate from three
values…** and StochCraft fits the time for you.

More settings are folded under headings; open them when you need them:

- **Breakdowns and availability:** time to failure, repair time, and availability by calendar.
- **Patience:** entities give up waiting after a time.
- **Balking:** arriving entities may not join the waiting room, more often the more are waiting.
  Choose **Table** (rows of **Number waiting** and the chance it **Joins with probability** from
  that number on; below the first row everyone joins) or **1/(n + 1)** (everyone joins an empty
  waiting room, half when one waits, a third when two do, and so on). Tick **Count those in
  service** to count the entities being served as well as those waiting. Those that balk take the
  flow marked **Balked**, or leave the model; results show **Balked** and **Share balked** for
  the task. To turn away everyone once a given number are waiting, a **Waiting room** of that
  size with a flow marked **Turned away** does the same, and needs no balking.
- **Health check:** take servers out of service when they are overloaded, and back after a
  restart.
- **Properties:** values set on each entity as it enters, for example a product type that
  changes the service time (**Service time by property**).
- **Takt** (on Departures): the customer demand, to see takt time, cycle time and whether demand
  is met.
- **Quality** (on Departures): mark the parts that leave here as scrap.

On a decision, each outgoing flow has a probability **p** or a condition **if**, and you can set
a **Default flow**. A flow can be marked **Rework**, and on a task with a resource also
**Gave up**, **Turned away** or **Balked**.

### Shared resources and shifts

With nothing selected, the inspector shows the whole model:

- **Resource pools:** units that several tasks share, such as a team of operators. Click
  **Add pool**, set its **Units**, and give it **Roles** if tasks ask for people by role.
- **Calendars:** periods when availability, arrivals or a pool's shifts change, such as a day
  shift from 08:00 to 16:00 on weekdays. Click **Add calendar**, then **Add period**. Tasks, pools
  and arrivals can then follow the calendar.

### Fix problems

The model overview's **Problems** section lists every error and warning, each with a **Show**
link that selects the block. Blocks with a problem are also marked in the left rail and on the
canvas. When it says **None found. The model is ready to run.**, you are set.

While the model has errors, the header's button says how many, for example **Run (2 problems)**,
and a chip above the canvas names the blocks, for example **2 problems: Paint, Inspect**.
Clicking either (or pressing F5, or **Model ▸ Run**) does not run: it selects the block with the
next error, focuses its field when it is a number such as the capacity or the main time, and
says what is wrong in the status bar. Click again for the next one. An error in the status bar
that belongs to a block selects it when clicked. Warnings alone do not stop a run.

A block's own problems show at the top of its inspector, in a place of fixed height, so the
fields below do not move when one appears.

Fields check what you type: a value that is not a number, or one the model cannot use (a
negative time or mean, a capacity or batch size below 1, a probability outside 0 to 1, such as
a decision's), is outlined and explained under the field, and the model keeps its old value
until you fix it.
Esc, or selecting another block, puts the model's value back in the field.

### The inspector: basics first

A block's inspector shows its basic fields first: its name, capacity and main time (and a task's
waiting room). The rest is folded: **More settings** (transfer, setup, times by property, planned
run time, inside at the start), **Breakdowns and availability**, **Patience** and **Health
check**. A folded section says what is set in it beside its title, for example **Breakdowns:
Exponential 120 min, Constant 6 min**, or **None**; a section with something set starts open.
The **?** beside **Capacity**, **Service time**, **Waiting room** and a task's **Inside at the
start** opens a one-line explanation under the field;
it can be reached with Tab and opened with Enter or Space.

### Undo

Every change can be undone with **Edit ▸ Undo** (Ctrl+Z, ⌘Z on a Mac). The menu names the change,
for example **Undo Rename Block**.

## Set up the experiment

With nothing selected, the **Experiment** section of the inspector sets how the model is run:

- **Runs:** how many independent runs. More runs give narrower confidence intervals but
  take longer.
- **Seed:** the starting point of the random numbers. The same model and seed always give the same
  results.
- **Warm-up:** time simulated before measuring starts. The other way to avoid results biased by
  a start from empty is to start full: set **Inside at the start** on the busy tasks to about
  the numbers they hold in a typical run. Their services start afresh at time 0.
- **Duration:** how long each run is measured, after the warm-up.
- **Time unit:** the unit for every time in the model. Changing it does not convert the times
  already in the model.
- **Length unit:** the unit for conveyor lengths (metres unless set).
- **Ends early:** stop each run when enough entities have left a block, or when a task
  empties, instead of after its duration.
- **How many runs:** **Exactly that many**, or **More until precise enough**: the runs set above
  come first, then more follow in batches until every key measure that varies is within its
  target (± 5% of its mean, ± 1 point for a share such as utilisation, by default), or the most
  runs (200 by default) is reached. A result that is the same in every run, such as a count the
  model fixes, is never counted as settled, but it does not hold back the others. A run that
  stops at 40 gives exactly the results of 40 runs set by hand.

The left rail's summary shows the current settings, for example "10 runs · seed 7".

## Run it

Press **Run** at the top right, or F5, or **Model ▸ Run**. A progress bar shows how many
runs have finished; **Cancel run** (or Esc) stops it. The status bar then says
"Finished 10 runs" and the **Results** tab opens. Before the first run, the **Results** tab says
**No results yet**, with a **Run** button.

In the browser, the runs go in parallel on every processor core but one, up to 16.

Results belong to the model as it was run. When you change the model, they stay on the
**Results** tab, marked **Out of date: the model has changed since this run**, with **Run again**
(the tab says "· out of date"). Undoing the change makes them current again; moving a block does
not count as a change.

## Read the results

![Results: what the run found, how sure the numbers are, key measures per block, and two charts](screenshots/results.png)

The **Results** tab shows, from the top:

1. **The run record:** runs, seed, warm-up and measured time, so you can reproduce the
   numbers.
2. **What the run found:** two to four sentences that answer first: which block limits the flow
   (or is only the busiest, when what comes after it holds it back), how much of an entity's time
   in the system is waiting and where, and what leaves against what arrives. **See the numbers**
   scrolls to the block behind each one. If the runs split into two different outcomes (some
   lines collapse while others recover), it says so instead of reporting a mean that fits
   neither.
3. **How sure are these numbers?** The key measure known least well, with a button to **Use N
   runs** that brings it to about ± 5%, a button to **Set a warm-up** when the start from empty
   pulls the means away, and how many key measures are already settled. A time or a count is
   settled at ± 5% of its mean; a share, such as utilisation, at ± 1 percentage point. After
   **More until precise enough**, it says why the runs stopped and after how many, offers
   **Allow up to N runs** when the most was reached first, names the key measures not yet within
   the run's targets (it judges them by its own targets, not the ± 5% above), and any result that was the
   same in every run, with how often something none of the runs met could still happen (about
   3 in N runs at most). The ± is then called the estimated precision: the number of runs was
   chosen from the results, so the interval may cover the true mean less often than 95%.
4. **Key measures:** up to four per block (for a task: utilisation, mean wait, mean queue length,
   and time failed when it fails). Every mean is rounded to what its interval supports: "38 ± 12"
   rather than "38.42 ± 12.31".
5. **Two charts:** where an entity's time in the system goes (time worked on and waiting at each
   task, for a model that is one straight line; the mean wait per block otherwise), and the
   waiting before the busiest block over the run, with its 95% band and the warm-up shaded.
   Hover for the numbers. The other charts (utilisation, queue length, time in system, blocked and
   lost) are under **More charts**.
6. **All measures:** every other result in a table per block, folded. Tasks with a resource also
   report the time spent processing, starved, blocked, failed, off shift and setting up, and OEE
   when a planned run time is set. Departures with a demand report takt time, demand met and
   cycle time.
7. **Spread across runs:** pick a result to see its percentiles and how often it met a
   target, for example "the wait is at most 5 minutes in 14 of 16 runs".

**How sure is a number?** "2.99 ± 0.09" means the mean is most likely between 2.90 and 3.08. If
the ± is large compared with the difference you care about, add runs. Notes under the
table explain anything left out, such as entities still inside at the end.

### Compare with a baseline

To see what a change does, click **Keep as baseline** under the run record, change the model
(for example a faster service time), and run it again. Over the new results you then see:

- **Changed since the baseline:** what is different in the model, block by block, with both
  values, for example "Paint › Service time, Mean: 5.2 → 4.6". A change that was undone is not
  listed; moving blocks is not a change.
- **How the runs were compared.** With the same seed and number of runs, run 1 of each meets the
  same random numbers, and so on, and the runs are compared in pairs: the difference comes from
  the change, not from chance draws. With another seed or number of runs (for example after
  **Use N runs**), they are compared as independent runs, and the intervals are wider.
- **What the change did:** the largest real changes in sentences, for example "Mean wait (Paint)
  fell from 38 min to 17 min, by −21 ± 6 min: more than chance would explain".
- **Baseline and now, key measures:** each block's key measures in both runs, the difference with
  its 95% interval (shares in percentage points), a mark of the change against a zero line, and a
  verdict: **Real: lower**, **Real: higher**, **Could be chance**, **No change** (every run gave
  the same value), or **Too few runs to tell**. A verdict is real when the difference's interval
  leaves out zero. Each row is judged on its own, so with many rows one may look real by chance.

**Keep this as the baseline** makes the new run the baseline; **Clear baseline** removes it.
Opening another model or file clears it too.

## Watch a run

To see the model move, click **Watch a run** in the run bar above the Model and 3D views, press
Space on either of those views, or choose **Model ▸ Watch a Run**. It plays the first run on the
canvas: each block shows how many entities were created, are waiting, are busy or are done. The
run bar is for watching one run; **Run** in the header runs the whole experiment, fast and
without animation.

- **Play/Pause** (Space), **Reset** (back to the start, paused), and **Step** (`.`) to the next
  event.
- **Speed** as ×N, in model time units per second: `[` halves it, `]` doubles it, and **As fast as
  possible** runs flat out.
- The clock shows the model time, how much of the run is done, and **warm-up** while it lasts.
- Type a time and click **Pause at** to play on and pause when the clock gets there.
- Select a block to see its waiting room over time in the inspector, under **This run**.
- **Stop watching** returns to editing.

This is one run of many: its numbers differ from the others and from the averages in
**Results**.

![Watching a run: counts on each block and the run's clock above the canvas](screenshots/watch.png)

## Other views of the model

- **Capacity:** what each station can carry, worked out from the model before any run: its
  arrivals, service, units, utilisation and load, a status (**Fine**, **Congested**, **Unstable**
  or **Unknown**) against a **Target utilisation** (85% unless changed), and the mean wait where a
  queueing formula holds. Under the table it names the bottleneck and says what cannot be known
  before a run. Once the model has results, the simulated utilisation and mean wait stand beside
  the worked-out ones. `stochcraft-cli explain` prints the same table.
- **3D:** the blocks drawn as simple 3D shapes, with conveyors as belts. Drag to orbit, scroll to
  zoom, Shift-drag to pan, and **Fit** to see it all. While a run is watched, each entity on a
  conveyor is drawn as a tote where the run has carried it.
- **Layout:** a plan of the model from above, to scale in metres, with a grid and a scale bar.
  Conveyors are drawn as their belts and blocks as their footprints. It is for looking only: drag
  to pan, scroll to zoom, **Fit**. Positions come from the diagram for now (the view says
  **Positions from the diagram**), and an **Unplaced** list names any block that is not drawn.
- **Dashboard** (beside the tabs, or **View ▸ Dashboard**): a panel beside the model with tiles
  for throughput, utilisation by resource, time on conveyors, and entities on the conveyors (or
  in the model) over time. Run the model, or watch a run, to fill it. While a run is watched, the
  throughput is that run's so far; the resource and conveyor charts show the last full run until
  the watched one ends.

## Ask what-if questions: the Analysis view

The **Analysis** tab runs the experiment several times with the same seed, so the differences
come from your change, not from chance. Each analysis can take a while.

- **Sweep a parameter:** pick a **Parameter** (for example a task's units or mean service time)
  and a **Result**, type the **Values** to try ("1; 2; 3"), and click **Run the sweep**. It says
  whether the result changed by more than chance would explain.
- **Which parameters matter:** moves each parameter down and up by a percentage and ranks them by
  how much the chosen result moves.
- **Size the resources:** adds a unit at the busiest task or pool until enough runs meet a
  target, such as "mean wait at most 5 minutes in 70% of runs". **Use these units in the
  model** applies the answer as one change you can undo.

## Monte Carlo models

A Monte Carlo model is a calculation, not a process: a project's cost, a budget, a capacity
check. Its **inputs** are uncertain numbers, each a distribution (a duration between 8 and 16
days, most likely 10); its **outputs** are worked out from them (the cost is the duration times
the day rate, plus materials). Each **trial** draws every input once and works out every output;
thousands of trials show how the outputs spread, and how likely each is to miss its target.

Open the example **Will the project come in under budget?** from the start page, or start one
with **File ▸ New Monte Carlo Model**. A Monte Carlo model has two tabs, **Model** and
**Results**, and no canvas.

On the **Model** tab:

- **Inputs:** each has a unit and a distribution, edited the same way as a task's time. Type a
  name under the inputs and click **Add input** for another. **Paste inputs from a table** takes
  lines of name, distribution, parameters and unit, copied from a spreadsheet or written as CSV
  (`labour,triangular,min=80;mode=100;max=150,h`); **Export inputs (CSV)** saves the same table.
- **Correlations:** two inputs that tend to move together (a busy market raises both the day
  rate and the price of materials) get a rank correlation from -1 to 1. Correlations that cannot
  all hold together are refused, with the reason under them.
- **Outputs:** each is an expression over the inputs and the outputs above it, each named as
  `$name`: `$duration * $day_rate + $materials`. An expression that cannot be read stays in its
  field with the reason under it. Give an output a **Target**, **At most** or **At least** a
  value, to see the chance of missing it.
- **Trials and sampling:** the number of trials (10,000 for a new model), the seed, and the
  sampling. **Latin hypercube** cuts each input's range into equally likely strata (the same
  chance of a draw in each, not equally spaced values) and draws once from each, which usually
  makes the results more precise; it does not promise fewer trials for every output. **How many
  trials** can be **More until precise enough**: trials are added until every output's mean is
  within the share you set, or until the most you allow has run; in that case the results say
  the target was not met. With Latin hypercube sampling or correlated inputs the trials run in
  whole batches of 1,000, so the most must be at least 1,000, and a most that is not a whole
  number of batches is rounded down: a most of 1,500 stops at 1,000 trials, a most of 2,500 at
  2,000. With plain random sampling the most is used as it is.

Every change is one step you can undo. **Run** (F5) makes the trials a few batches at a time,
so the window stays responsive, also in the browser.

The **Results** tab answers the question first:

- **What the run found:** for an output with a target, the chance of missing it in words, such as
  "About a 1 in 5 chance that cost is more than 46,000 USD (21%)", with the 95% interval of that
  chance. For an output without one, where 9 trials in 10 fall and its mean.
- For each output: the mean ± the half-width of its 95% interval, the standard deviation, the
  minimum and maximum, the 5th, 10th, 50th, 90th and 95th percentiles, a histogram (how many
  trials gave each value) and the cumulative curve (the share of trials at or below each value),
  with the target drawn on both.
- **How sure are these numbers?** says what each ± is across. With plain random sampling and no
  correlations it is across the trials. Latin hypercube sampling and correlations make the trials
  within a batch of 1,000 depend on each other, so there it is across batches, and the trials run
  in whole batches: 2,500 trials asked for run as 3,000, and the results say so.
- The line under **Results** gives the trials, the seed and the sampling, and how a run with
  **More until precise enough** stopped. **Export table (CSV)** saves a row per output.
- Which inputs matter most (a tornado chart, rank correlations of inputs with each output, and
  variance-based indices) is not in the window yet.

## Save, open and export

StochCraft reads and writes **BPMN** and **XPDL** files with **BPSim** simulation data, the open
standards that process-modelling tools share.

| | Desktop | Browser |
| --- | --- | --- |
| Open | **File ▸ Open…**, or drop a file on the window | **File ▸ Open…**, or drop a file on the page |
| Save | **File ▸ Save** writes back to the file; **Save As…** writes a new one | **File ▸ Download** saves a copy to your downloads |
| Recent files | **File ▸ Open Recent** | not available |

- A new model is saved as BPMN. An XPDL file can be saved back as XPDL.
- If a file you opened holds things StochCraft does not understand, saving asks before writing a
  new BPMN file, which leaves them out.
- **Unsaved changes are kept.** If StochCraft closes with unsaved changes, it offers to
  **Recover** them the next time it starts.
- **Export results:** on the **Results** tab, **Export table (CSV)** and **Export charts (PNG)**
  (in the browser: **Download**). The CSV has one row per block and metric, with the mean, the
  half-width of its confidence interval, the standard deviation and the number of runs.
  **Export charts (PNG)** needs every chart on screen: in a window too short for them, the status
  bar says to make the window taller, and nothing is exported.

## Get help in the window

Press **F1**, or choose **Help ▸ Help**, to open Help beside the model. It does not cover the
canvas, so you can follow it while you work.

- **Getting started:** five steps to a first model (add arrivals, a task, departures, run it, read
  what it found), each ticked as the model gets there. **Show me**, beside the next step, rings
  the control it uses (a block in the palette, the dot to drag a connection from, **Run**, the
  **Results** tab) with a line saying what to do. You make each change; the guide moves on with
  the model, and **Esc** or **×** stops it.
- **Glossary:** the window's words and what they mean, from Block to Could be chance. Type in
  **Search help and terms** to narrow it.
- **Keyboard shortcuts:** the main keys; **Change keys** opens the full list.
- **User guide**, **What the results mean** and **Report a problem** open in your web browser.

The **Help** menu also leads straight to Getting Started, the Glossary, the results section of
this guide, Keyboard Shortcuts and Report a Problem. F1 or the × closes the panel.

## Change how it looks and which keys do what

- **View ▸ Appearance:** Light, Dark or the system setting, and a colour theme: StochCraft, High
  contrast or Warm. **Edit themes…** makes your own (duplicate a built-in theme, then change its
  colours; it checks the text contrast for you). **Chart colours…** picks the palette for charts,
  with colour-blind-safe palettes built in.
- **View ▸ Language:** the interface in English or one of 44 other languages. Numbers follow your
  region's format.
- **Edit ▸ Preferences ▸ Keyboard Shortcuts…** (on a Mac, in the StochCraft menu): change any
  shortcut.

## Keyboard shortcuts

Ctrl on Windows and Linux is ⌘ on a Mac.

| Key | Does |
| --- | --- |
| F1 | Help: open or close it beside the model |
| F5 | Run |
| Esc | Cancel the run; clear the selection |
| Space | Watch a run, or play or pause it (on the Model and 3D views) |
| `.` | Step to the next event while watching |
| `[`, `]` | Halve or double the speed while watching |
| Ctrl+N | New model (desktop only; in the browser it opens a new window) |
| Ctrl+O | Open |
| Ctrl+S | Save (Download in the browser) |
| Ctrl+Shift+S | Save As (desktop) |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z, Ctrl+Y | Redo |
| Ctrl+X, Ctrl+C, Ctrl+V | Cut, copy, paste blocks |
| Ctrl+D | Duplicate |
| Delete, Backspace | Delete the selection |
| Ctrl+A | Select all |
| Up, Down | Select the previous or next block |

## Troubleshooting

- **Run is greyed out:** the model has no blocks yet, or a dialog is open. While a run is going, the button reads **Cancel run** instead.
- **Run says "Run (1 problem)" and does not run:** the model has an error. The click selected the
  block and field concerned; fix it, or click Run again for the next problem (see Fix problems).
- **The results say "Out of date":** the model changed after the run. Click **Run again**, or undo
  the change.
- **A number field has a red outline:** the text is not a valid value. The message under it says
  why; the model keeps the old value until you fix it.
- **Japanese, Chinese, Thai or other text shows as boxes in the browser:** the font is fetched
  when the language needs it; give it a moment online. Bengali, Tamil, Telugu and Lao use the
  fonts on your own system.
- **The results look unstable from run to run:** add runs, and a warm-up (or **Inside at the
  start** on the busy tasks) if the model starts empty.

## Limitations

- A baseline compares two versions of a model, one change at a time; there is no table of named
  scenarios yet. For several values of one parameter, use the Analysis view's sweep.
- Charts can't be configured beyond their colours.
- The canvas is one drawn surface, so screen readers can't yet read its blocks; the menus,
  dialogs and status messages are readable.
- What is planned next is in the [roadmap](../ROADMAP.md).
