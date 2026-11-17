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
- [Ask what-if questions: the Analysis view](#ask-what-if-questions-the-analysis-view)
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
  on shift.

## Start: open an example

When StochCraft opens with no model, it shows **Open a model to simulate it**. Click
**Open a model…** to open your own file, or pick one of the examples under
**Or start from an example**. They are also under **File ▸ Open Example**:

| Example | What it shows |
| --- | --- |
| **M/M/1 queue** | One clerk, random arrivals: the textbook queue, checked against theory. |
| **Tandem line** | Two stations in series, read from an XPDL file. |
| **Every distribution** | One task per BPSim distribution, to compare their spread. |

To try the whole cycle, open **M/M/1 queue** and press **Run**. The results show a mean wait close
to 3 minutes and about 75% utilisation, which is what queueing theory predicts for this model.

![The start screen with the three examples](screenshots/start.png)

## Build a model

The window has three parts:

- the **left rail**, with the model's name, its blocks in order, and a summary of the experiment
  at the bottom;
- the **canvas** in the middle, where the flow is drawn;
- the **inspector** on the right, which shows the settings of what is selected. On a narrow
  window it moves under the canvas.

The tabs above the canvas switch between **Model**, **Results** and **Analysis**.

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

**Times.** Each time is a distribution: pick its type (Constant, Exponential, Uniform, Triangular,
Normal, Log-normal, Gamma, Erlang, Weibull and others) and fill in its values. If you only know a
minimum, a most likely value and a maximum, use **Three-point estimate ▸ Estimate from three
values…** and StochCraft fits the time for you.

More settings are folded under headings; open them when you need them:

- **Breakdowns and availability:** time to failure, repair time, and availability by calendar.
- **Patience:** entities give up waiting after a time.
- **Balking:** arriving entities may not join the queue, more often the longer it is. Choose
  **Table** (rows of **Number waiting** and the chance it **Joins with probability** from that
  number on; below the first row everyone joins) or **1/(n + 1)** (everyone joins an empty queue,
  half when one waits, a third when two do, and so on). Tick **Count those in service** to count
  the entities being served as well as those waiting. Those that balk take the flow marked
  **Balked**, or leave the model; results show **Balked** and **Share balked** for the task. To
  turn away everyone once a queue reaches a length, a **Waiting room** of that size with a flow
  marked **Turned away** does the same, and needs no balking.
- **Health check:** take servers out of service when they are overloaded, and back after a
  restart.
- **Properties:** values set on each entity as it enters, for example a product type that
  changes the service time (**Service time by property**).
- **Takt** (on Departures): the customer demand, to see takt time, cycle time and whether demand
  is met.
- **Quality** (on Departures): mark the parts that leave here as scrap.

On a decision, each outgoing flow has a probability **p** or a condition **if**, and you can set
a **Default flow**. A flow can be marked **Rework**, and on a task with a resource also
**Abandoned**, **Turned away** or **Balked**.

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
run time), **Breakdowns and availability**, **Patience** and **Health check**. A folded section
says what is set in it beside its title, for example **Breakdowns: Exponential 120 min,
Constant 6 min**, or **None**; a section with something set starts open. The **?** beside
**Capacity**, **Service time** and **Waiting room** opens a one-line explanation under the field;
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
- **Warm-up:** time simulated before measuring starts.
- **Duration:** how long each run is measured, after the warm-up.
- **Time unit:** the unit for every time in the model. Changing it does not convert the times
  already in the model.
- **Ends early:** stop each run when enough entities have left a block, or when a task
  empties, instead of after its duration.

The left rail's summary shows the current settings, for example "10 runs · seed 7".

## Run it

Press **Run** at the top right, or F5, or **Model ▸ Run**. A progress bar shows how many
runs have finished; **Cancel run** (or Esc) stops it. The status bar then says
"Finished 10 runs" and the **Results** tab opens.

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
   settled at ± 5% of its mean; a share, such as utilisation, at ± 1 percentage point.
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

To see the model move, click **Watch a run** above the canvas, or **Model ▸ Watch a Run**. It
plays the first run on the canvas: each block shows how many entities were created,
are waiting, are busy or are done.

- **Play/Pause** (Space), **Step** to the next event, and a **Speed** list.
- Type a time and click **Jump** to go straight there.
- Select a block to see its queue over time in the inspector, under **This run**.
- **Stop watching** returns to editing.

This is one run of many: its numbers differ from the others and from the averages in
**Results**.

![Watching a run: counts on each block and the run's clock above the canvas](screenshots/watch.png)

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
| F5 | Run |
| Esc | Cancel the run; clear the selection |
| Space | Play or pause while watching a run |
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
- **The results look unstable from run to run:** add runs, or a warm-up if the model
  starts empty.

## Limitations

- There is no downloadable desktop release yet; see the [installation guide](installation.md).
- A baseline compares two versions of a model, one change at a time; there is no table of named
  scenarios yet. For several values of one parameter, use the Analysis view's sweep.
- Charts can't be configured beyond their colours.
- The canvas is one drawn surface, so screen readers can't yet read its blocks; the menus,
  dialogs and status messages are readable.
- What is planned next is in the [roadmap](../ROADMAP.md).
