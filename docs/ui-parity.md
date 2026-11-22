# UI and interaction parity

> **Last reviewed:** 2026-11-15 · **Last updated:** 2026-11-20 · **Target:** what users of
> established commercial simulation tools see and do (from their public documentation) in a
> session, from the start screen to the report

This checklist grades StochCraft's window against what users of established commercial
simulation tools do in a typical session. It is a user-experience check, not a list of engine features: an engine command
with no control in the window counts as missing here. The headline numbers drawn from it, and how
they were measured, are in [target-app-parity.md](target-app-parity.md); the ranked work list is
[gaps.md](gaps.md).

Status: **done** (the window does what the vendor's user expects, possibly in a different way),
**partial**, **missing**, **done (ours)** (StochCraft does something the vendors do not, or do only
with extra set-up; these rows count towards readiness but not towards fidelity). **[1h]** marks a
row a switcher meets in the first hour; it counts three times in the scores. **[nv]** marks a row
whose vendor side was not checked; it is left out of fidelity.

Vendor behaviour comes from their public documentation, read as prose and described here in
generic terms (ROADMAP principle 6). The help pages of one flowchart-first tool refused automated reading, so its rows
rest on search summaries of those pages and say "from public summaries, not confirmed"; the only
exception is that vendor's features page, which was read directly. StochCraft was graded from
screenshots of a first session, the ROADMAP and the code in `crates/ui-egui/src`; the app was not
driven for this checklist.

**Summary (estimated):** 91 rows: 27 done, 10 done (ours), 26 partial, 28 missing. UI/UX fidelity
~44%, the 23 first-hour rows ~55%. Weakest: layout and animation (6), results and charts (10), help
(11), and the library (2), where the first impression is made.

**Since the review (2026-11-20).** The rows below are graded as the review found them, apart from
the run-control rows of area 7, updated with the run bar (#218), and the summary above was not
re-graded. Since then:

- **Start screen (area 1):** a start page that asks "What do you want to find out?", with examples
  by question and industry (#170), a "Start from a blank model" link, and a guided first model
  on the real canvas (#172). The examples went from three to eleven: five with checked answers
  (#171) and a parcel sorter (#207). Models have a length unit beside the time unit (#198).
- **Library (area 2):** ten kinds in the palette, with the Conveyor (#198). The library is still
  a strip of chips; its redesign is M5.5's U3.
- **Distributions (area 5):** fitting measured data by maximum likelihood, ranked by
  Anderson–Darling, in the engine and the command line (`stochcraft-cli fit`, #182); none in the
  window yet.
- **Layout, 2D/3D and animation (area 6):** a 3D view of the model with parametric low-poly
  shapes and totes on the conveyors of a watched run (#205, #208, #210, #213), and a read-only
  Layout view to scale (#216), both placed from the diagram for now. The Transport block and
  mobile pools are in the model and its files (#202), without a runtime yet. Entities moving on
  the flowchart are still missing (#151).
- **Statistics while running (area 8):** "This run" shows the selected block's waiting room over
  time.
- **Results (area 10):** what the run found in two to four sentences (#165), "How sure are these
  numbers?" with one-click fixes (#164), up to four key measures per block with the rest folded
  (#163), and two default charts with the rest under More charts (#165). A dashboard panel beside
  the model holds four fixed tiles (#209); charts and layout are not yet chosen by the user.
- **Help (area 11):** Help beside the work, with Getting started, a glossary, shortcuts and links
  to the user guide (#169); the Help menu leads to each. The "?" beside Capacity, Service time and
  Waiting room opens its explanation from the keyboard (#146).

## 1 Start screen, new model, examples

| Item | Status | Notes |
|---|---|---|
| A start screen with an obvious way to begin | partial | **[1h]** The 3D-first tool: a start screen with a new-model command. The flowchart-first tool: a welcome page with example models (from public summaries, not confirmed). StochCraft: an empty state, "Open a model to simulate it", with Open a model… and three examples, and + Add block in the side rail. There is no New model button there; File ▸ New exists |
| New-model set-up: name, time unit, other units, start date and time | partial | **[1h]** The 3D-first tool: a units dialog on every new model (time, length, fluid, start date and time). The flowchart-first tool: name and time unit when the model is created (from public summaries, not confirmed). StochCraft: File ▸ New gives "Untitled model"; the time unit is chosen in the model overview's Experiment section, with a caption that changing it does not convert times already entered. No start date or clock time, no length unit (nothing travels yet) |
| Example gallery | partial | The flowchart-first tool: a large gallery of example models (from public summaries, not confirmed; size not verified). The 3D-first tool: examples on the start screen not verified in its manual. StochCraft: three examples (M/M/1 queue, tandem line, every distribution) on the empty state and in File ▸ Open Example |
| Templates and industry set-ups | missing | The 3D-first tool: industry set-ups that rearrange the toolbar and library, for example for healthcare. The flowchart-first tool: industry libraries (material handling, pedestrians, rail, road traffic, fluids), from its features page. StochCraft: `stochcraft-cli new --template` has templates; the window has none |
| Recent files | done | [nv] File ▸ Open Recent on the desktop (not in the browser). Vendor behaviour not checked |
| First-run guidance (tour, tips, what next) | partial | The 3D-first tool: tutorials in the manual rather than an in-app tour. StochCraft: the empty and running states say what to do next ("No blocks yet. Open a model, or add blocks to build one."); no guided first model |

## 2 Library and canvas

| Item | Status | Notes |
|---|---|---|
| A docked object library with icons, beside the model | partial | **[1h]** The 3D-first tool: an icon grid filling the left pane, grouped by kind of object, changing with the active tool. The flowchart-first tool: a palette of process blocks opens with a new model (from public summaries, not confirmed). StochCraft: one horizontal strip of text chips above the canvas ("Drag to add:"); at 1440 px wide several, Departures among them, sit behind a sideways scroll; no icons, groups or search |
| Library breadth | partial | **[1h]** The 3D-first tool: sources, queues, processing stations, sinks, combining and separating stations, operators and transporters, robots, cranes, lifts, storage systems, conveyors, racks, vehicle paths, fluid objects and healthcare objects. The flowchart-first tool: sources, queues, delays, sinks, services, seizing and releasing resources, resource pools and time measurement (from public summaries, not confirmed; full list not verified). StochCraft: nine kinds in the palette: arrivals, task with a resource, task with shared resources, task, decision (one way), split or join, batch, split batches, departures. Assembly is compiled by the engine but not offered in the palette. A queue is a task's waiting room, not an object |
| Add an object at the pointer (quick add) | missing | The 3D-first tool: a searchable menu of objects at the pointer, and a way to add a connected object by clicking. StochCraft: clicking a chip adds the block after the selection; no add-at-pointer, no search |
| Click-to-place mode | missing | The 3D-first tool: a mode that adds one object per click, optionally copying an existing object's settings. StochCraft: drag, or click to append |
| Move, select, marquee, copy and paste, duplicate, align and distribute | done | Drag, Shift or Ctrl-click, rectangle selection, cut, copy, paste, duplicate, delete, align by edges and centres, spread evenly; snap to a 10-unit grid. The 3D-first tool documents copying objects and their settings; the vendors' align and distribute not verified |
| Pan and zoom | done | Scroll, middle or right drag, Ctrl+scroll, pinch, corner buttons and Fit. The 3D-first tool adds 3D rotation, which a 2D flowchart does not need |
| Rename and edit in place | done | [nv] A name field at the top of the inspector |
| Model outline (a tree of the objects) | done | [nv] The side rail lists the process and selects on click (arrow keys, Delete, Esc) |
| Hierarchy: containers, sub-flows, sub-models | missing | The 3D-first tool: containers, nested logic flows and sub-models. The flowchart-first tool: nested model components (from public summaries, not confirmed). StochCraft reads embedded BPMN sub-processes in line but cannot edit nested levels |
| Connectors with bends; lanes and pools; resizing shapes by hand | missing | "Not yet" in ROADMAP M3.4 |
| Undo and redo | done | [nv] Every edit is one engine command and one named undo step |
| Keyboard shortcuts and an editor for them | done | Settings ▸ Keyboard Shortcuts… records new bindings (how it handles conflicts not verified). The 3D-first tool lists fixed shortcuts |

## 3 Connecting and flow logic

| Item | Status | Notes |
|---|---|---|
| Connect two objects by pointing | done | **[1h]** The 3D-first tool: a connect mode, or a key held while clicking the two objects. The flowchart-first tool: click an output port, then an input port (from public summaries, not confirmed). StochCraft: drag from a shape's handle to the target; also From and To lists with Connect in the model overview |
| Disconnect | done | The 3D-first tool: a key held while clicking the two objects. StochCraft: select the connection and press Delete, or Remove in the overview's list |
| Routing rules on the way out (probability, condition, by type) | partial | The 3D-first tool: a menu of prepared routing rules that fill in the logic. The flowchart-first tool: a routing block (from public summaries, not confirmed). StochCraft: a decision block with a probability, a typed condition (a BPSim expression) and a default per way out; no menu of prepared rules |
| Resources and operators as objects on the model | partial | **[1h]** The 3D-first tool: operators and transporters placed in the model and linked to the stations they serve. The flowchart-first tool: a block for a pool of resources in the flowchart, used by the blocks that do the work (from public summaries, not confirmed). StochCraft: shared pools are rows in the model overview, named in a shared task's needs; nothing on the canvas shows a pool or links it to its tasks |
| Pull logic and lists | missing | The 3D-first tool: shared lists of waiting items with queries that choose which one to pull, used in its first tutorial. Not on StochCraft's roadmap |

## 4 Properties and parameters

| Item | Status | Notes |
|---|---|---|
| Properties of the selection in a docked pane | done | **[1h]** The 3D-first tool: a context-sensitive properties pane, also opened beside the pointer. The flowchart-first tool: a properties view (from public summaries, not confirmed). StochCraft: the inspector to the right of the canvas, with a breadcrumb (Model › Task with a resource) and Remove block |
| Option menus that fill in a property for you | partial | **[1h]** The 3D-first tool: a menu beside each property with prepared choices that write the expression or open a guided dialog. StochCraft: choice boxes for distributions, transfer time, planned run time, setup and waiting room; expressions for conditions and properties are typed |
| Pick an object in the model to fill a field | missing | The 3D-first tool: a picker beside many properties takes an object, label or table by clicking it |
| Edit several objects at once | missing | The 3D-first tool: changing a property with several objects of one type selected changes all of them. StochCraft: with several blocks selected, the inspector says how many and offers align, spread and delete, not their shared parameters |
| Named model parameters (inputs with bounds) | partial | The 3D-first tool: a table of model parameters with types and bounds, used by experiments. The flowchart-first tool: parameters on model components (from public summaries, not confirmed). StochCraft: the Analysis view picks any block parameter directly; no named parameter list, no bounds |
| User-defined outputs (KPIs) | partial | The 3D-first tool: a table of outputs the user defines, computed from the model and fed to experiments. The flowchart-first tool: user-defined outputs and statistics collected in the model (from public summaries, not confirmed). StochCraft: a fixed set of measures per block (entered, left, queue, wait, utilisation, work in progress, OEE, takt, energy and more); no user-defined measure |
| Expressions and code | partial | The 3D-first tool: a scripting language behind every property. The flowchart-first tool: Java in fields (from public summaries, not confirmed). StochCraft: BPSim expressions for conditions and properties, with errors shown on the block; no general code, by design |
| Entity attributes | done | Entity properties set per block (number, yes/no, text, random draw, expression), with their mean assigned value in the results |
| Shifts, timetables, breakdowns | done | [nv] Calendars in the model overview; breakdowns and availability per task; rules for work under way when a shift ends |
| Setup and changeovers | partial | [nv] Setup per task in the inspector. Sequence-dependent changeover tables in the window not verified |

## 5 Distributions and input data

| Item | Status | Notes |
|---|---|---|
| Choose a distribution in a guided dialog with a live picture | partial | **[1h]** The 3D-first tool: a guided dialog with about thirty distributions and a chart of the probabilities that updates as the parameters change. StochCraft: a distribution choice and its parameter fields in the inspector; no picture while choosing |
| Times in the model's units, shown beside the field | done | For example "0.2 h" in the field. The flowchart-first tool: a unit control per time field (from public summaries, not confirmed) |
| Fit a distribution from data | missing | The 3D-first tool: a fitting add-on appears in community answers but not in its manual's contents (not verified). The flowchart-first tool: empirical distributions from samples with a histogram preview, no built-in fitter found (from public summaries, not confirmed). StochCraft: none in the window; fitting from event logs is planned after M3 |
| Three-point estimate (minimum, most likely, maximum or 95th percentile) | done (ours) | "Estimate from three values…" fits a block's time and keeps the estimate as the time's pedigree. Not found in the vendor pages read |
| Import data tables (spreadsheet, CSV, database) | missing | The 3D-first tool: tables filled from spreadsheets or databases with an importer. The flowchart-first tool: a built-in database that imports spreadsheets (from public summaries, not confirmed). StochCraft: none |

## 6 Layout, 2D/3D and animation

| Item | Status | Notes |
|---|---|---|
| A 3D model view as the main workspace | missing | The 3D-first tool: the model is built in a 3D view, where items move during a run. The flowchart-first tool: 2D by default, 3D through a 3D view placed on the model (from public summaries, not confirmed). StochCraft: a 2D BPMN flowchart only; 3D comes later |
| Entities seen moving between stations and queuing | missing | **[1h]** The 3D-first tool: on the first tutorial's first run, items appear and pile up in the queue. The flowchart-first tool: entities animated along paths once the space is drawn and assigned to blocks (from public summaries, not confirmed). StochCraft: counts in badges and a busy bar per block; entities moving along the connections is "not yet" in ROADMAP M3.5 |
| Counts on the flowchart during a run | done | **[1h]** The flowchart-first tool: counts beside the blocks during a run (from public summaries, not confirmed). The 3D-first tool: tokens in the logic view stack and turn into a count. StochCraft: badges per block (created, waiting, busy, inside, done) |
| Floor plan or CAD drawing under the model | missing | The 3D-first tool: CAD drawings and floor plans imported as a guide. StochCraft: none (M5 mentions an optional background image) |
| Space: paths, nodes, travel networks, transporters | missing | The 3D-first tool: travel networks, path finding and vehicle paths. The flowchart-first tool: paths and nodes with attraction points (from public summaries, not confirmed). StochCraft: planned in M5 |
| 3D shapes and custom looks for objects and items | missing | The 3D-first tool: item shapes (a person instead of a box) and custom 3D objects. The flowchart-first tool: 3D figures such as people and machines (from public summaries, not confirmed) |
| Separate logic view and layout view | partial | The 3D-first tool: a 3D layout and a separate logic flowchart, linked. StochCraft has one view, the BPMN flowchart, which is both: familiar to flowchart users, unfamiliar to 3D-first users |

## 7 Running and run controls

| Item | Status | Notes |
|---|---|---|
| Run, stop or pause, step, from a control bar with shortcuts | done | **[1h]** The 3D-first tool: reset, run, stop, fast-forward, skip and step on a control bar. The flowchart-first tool: a control panel in the model window (from public summaries, not confirmed). StochCraft: Run (F5) runs all replications without animation; Watch a run gives Play or Pause (Space), Reset, Step, a speed with keys to halve or double it, As fast as possible, Pause at and Stop watching, in a run bar over the Model and 3D views. Two modes where the vendors have one |
| Run speed control | done | **[1h]** The 3D-first tool: a speed slider and menu. The flowchart-first tool: a real-time scale, slower, faster or as fast as possible (from public summaries, not confirmed). StochCraft: simulated time per second, by default about a minute for the whole run |
| Jump to a time | done (ours) | The 3D-first tool's manual describes stop times, speed and skipping ahead, not jumping to a chosen time (not found in the pages read). StochCraft: Pause at with a time field (plays on to the time, then pauses) |
| Stop time and warm-up | done | **[1h]** The 3D-first tool: on the control bar. StochCraft: Warm-up and Duration in the model overview, and stop conditions that end a replication early |
| A control panel of chosen model inputs | missing | The 3D-first tool: input widgets on a dashboard form a control panel for changing chosen values before each run. StochCraft: inputs are edited in the overview and inspector; changing anything a run depends on stops watching |
| Watch any replication | partial | The 3D-first tool animates the one run it runs. StochCraft watches replication 1 of N only, and says the others differ |
| Repeatable runs | done | The 3D-first tool: the same random streams every run. StochCraft: the experiment's seed |

## 8 Statistics on objects while running

| Item | Status | Notes |
|---|---|---|
| Click an object during a run to see its live statistics | done | **[1h]** The 3D-first tool: statistics in the properties pane during a run. The flowchart-first tool: a click opens a window with the block's statistics (from public summaries, not confirmed). StochCraft: the inspector's "This run" (in, out, lost) and a chart of its queue over time |
| Names and statistics shown on every object | done | The 3D-first tool: an option to show names and statistics. StochCraft: badges always on while watching |
| Pin a statistic to a dashboard | missing | **[1h]** The 3D-first tool: a pin beside each statistic adds a chart of it to a dashboard and sets up its collection |
| State breakdown of a resource while it runs (busy, idle, blocked, down) | partial | **[1h]** The 3D-first tool: state pie, bar and Gantt charts; its second tutorial builds a state pie. StochCraft: the results table has time starved, blocked and off shift, the share of units blocked and OEE; while watching, a busy bar only |

## 9 Experiments

| Item | Status | Notes |
|---|---|---|
| Replications and confidence intervals on every run | done (ours) | The 3D-first tool: a run in the window is one replication; replications come from the experiment tool. The flowchart-first tool: a simulation run is one run; replications come from multi-run experiments (from public summaries, not confirmed). StochCraft: every Run is N replications (10 by default) in parallel, each mean with a 95% interval |
| A scenario table: several named scenarios, several parameters, replications each | missing | The 3D-first tool: a table of named scenarios with one column per chosen parameter, replications per scenario, and a way to set the model to a scenario. The flowchart-first tool: parameter-variation and run-comparison experiments (from public summaries, not confirmed). StochCraft: one-parameter sweeps in the window; the engine's `experiment.design` (a full factorial or a Latin hypercube of several parameters, from the command line and MCP) and `results.compare` (paired and Welch differences) have no control in the window |
| Sweep one parameter | done | The 3D-first tool: an experiment over a range of values. The flowchart-first tool: a parameter-variation experiment (from public summaries, not confirmed). StochCraft: "Sweep a parameter", with the values typed as a list |
| Sensitivity: which inputs matter | done | The flowchart-first tool: a sensitivity experiment (from public summaries, not confirmed). The 3D-first tool: none found in the pages read. StochCraft: "Which parameters matter" (each changed by 10% down and up, ranked) |
| Optimisation | partial | The 3D-first tool: an optimiser, licensed separately, with objectives and a trade-off chart. The flowchart-first tool: an optimisation experiment (from public summaries, not confirmed). StochCraft: "Size the resources" adds units at the busiest task or pool until a share of replications meets a target; no general search over objectives |
| Replications until a precision is reached | missing | The flowchart-first tool: replications per scenario can stop once a confidence level is met (from public summaries, not confirmed). The 3D-first tool: optimisation can run replications until the objective's confidence is met. StochCraft: a fixed number |
| Results kept between runs; add replications later | missing | The 3D-first tool: a results file beside the model; finished tasks are not rerun, and replications or scenarios can be added later |
| Progress per scenario and replication | partial | The 3D-first tool: a status chart with one box per task. StochCraft: a progress bar and Cancel in the status bar |
| Run on other machines | partial | The 3D-first tool: runs spread over machines on a network. The flowchart-first tool: experiments in its cloud service (from public summaries, not confirmed). StochCraft: parallel threads on the desktop, web workers in the browser; no remote runs |

## 10 Results, charts, dashboards, export

| Item | Status | Notes |
|---|---|---|
| Useful results with no set-up | done (ours) | **[1h]** The 3D-first tool: statistics per object in the properties pane; charts need a dashboard first (its second tutorial spends six steps on it). The flowchart-first tool: charts are placed on the model and given expressions (from public summaries, not confirmed). StochCraft: the Results tab shows four charts (utilisation, mean wait, mean queue length, mean time in system, with 95% whiskers) and a table per block with the interval, the unit and the run record |
| Findings stated in words | partial | Only where replications split into two outcomes (#129): the result says so in a sentence, with a strip of every run's value. There is no general finding (the busiest block, a warm-up that is too short, an interval too wide to act on). Not found in the vendor pages read |
| Dashboards the user composes | missing | **[1h]** The 3D-first tool: dashboards built from a library of chart templates (output, time in system, content, state) that also hold inputs and text. The flowchart-first tool: bar, stack, pie, time and histogram charts placed anywhere on the model's animation (from public summaries, not confirmed). StochCraft: the four fixed charts; no choice of chart, no layout, nothing saved with the model |
| Time plots of a statistic | partial | **[1h]** The 3D-first tool: content over time from a pinned statistic. The flowchart-first tool: time plots (from public summaries, not confirmed). StochCraft: the watched replication's queue over time, in the inspector only |
| Gantt and state charts | missing | **[1h]** The 3D-first tool: Gantt charts, a timeline of states (one row per object, coloured by state), state pies and bars. StochCraft: the state times are numbers in the results table |
| Distribution of a result across replications; probability of meeting a target | done (ours) | Percentiles, the empirical distribution with its confidence band, and the probability of meeting a target. The flowchart-first tool: multi-run experiments collect outputs over runs (from public summaries, not confirmed; how they are charted not verified) |
| Compare scenarios side by side | partial | The 3D-first tool: a results window comparing scenarios per measure, with charts and tables. StochCraft: the sweep's chart for one result; no scenario-to-scenario view |
| Export | partial | The 3D-first tool: a report to a spreadsheet workbook; tables to spreadsheets, databases or files; charts as images. The flowchart-first tool: database tables to spreadsheets (from public summaries, not confirmed). StochCraft: the results table as CSV and charts as PNG |
| A report document | missing | The 3D-first tool: an older tool writes a summary report to a spreadsheet. StochCraft: the run record line; a credibility report is planned after M3 |
| Colour-blind-safe chart palettes and a contrast check | done (ours) | View ▸ Appearance ▸ Chart colours…, with Okabe–Ito and checked presets |

## 11 Help and learning

| Item | Status | Notes |
|---|---|---|
| Help menu with the manual | missing | The 3D-first tool: the user manual opens inside the program. The flowchart-first tool: help and example models from the Help menu (from public summaries, not confirmed). StochCraft: Help holds one item, About StochCraft |
| Step-by-step tutorials | missing | **[1h]** The 3D-first tool: a tutorial series from the basics to experiments. The flowchart-first tool: tutorials from a first queueing model to pedestrians, traffic and system dynamics (from public summaries, not confirmed). StochCraft: no tutorials; a task-by-task user guide (docs/user-guide.md) |
| Explanation of a field where it is | partial | StochCraft: sentences under section headings and some hover texts (by mouse only, not by keyboard). The 3D-first tool: a reference page per object in the manual |
| An assistant that explains and builds models | partial | The flowchart-first tool's features page (read directly) describes a built-in AI assistant that answers questions, explains models and builds simple ones from text; which versions have it not verified. StochCraft: an MCP server, so an outside AI agent can build and run models in the window; nothing inside the app |

## 12 Errors, validation, debugging

| Item | Status | Notes |
|---|---|---|
| Problems found before running, linked to the object | done (ours) | **[1h]** The flowchart-first tool: building lists the errors, each leading to its place (from public summaries, not confirmed). StochCraft: validation as you edit, with no build step: problems listed in the overview and outlined on their blocks. With errors, Run says how many and shows the next one's block and field instead of running; a chip above the canvas and the status bar lead to them too ([gaps](gaps.md) #8, #146) |
| Bad input in a field | done | The text stays in its field, outlined, until fixed; a number field says why under it, with an icon ("Not a number. Capacity is still 0."), since #140 |
| Errors during a run | done | [nv] A model whose time stops moving is reported within seconds; a failed run says why in the status bar (detail not verified) |
| Event list, event log, tracing one item | missing | The 3D-first tool: an event list, an event log and tracing of one token. The flowchart-first tool: a panel with the event queue and a console (from public summaries, not confirmed). StochCraft: the engine journal records commands, not events |
| Console output per replication | missing | The 3D-first tool: console output per replication in its experiment tool |

## 13 Files and projects

| Item | Status | Notes |
|---|---|---|
| Open, save, save as, unsaved-change prompts, recent files | done | [nv] ROADMAP M3.1a and M3.1b |
| Autosave and crash recovery | done | [nv] Every ten seconds on the desktop (the browser's interval not verified); each window keeps its own copy |
| An open file format other tools read | done (ours) | BPMN 2.0 or XPDL with BPSim; a model drawn in another BPMN editor opens with its layout. The 3D-first tool and the flowchart-first tool save to formats of their own (the flowchart-first one: from public summaries, not confirmed) |
| Open vendor models | missing | No import; a switcher rebuilds the model |
| Runs in a browser, offline | done (ours) | The GitHub Pages build installs and works offline. The flowchart-first tool's features page (read directly) offers modelling in the browser too; the 3D-first tool is a Windows desktop program |
| Version control friendly | done | Text XML files. The flowchart-first tool added Git support in a recent release (from public summaries, not confirmed) |
| Interface languages, light and dark themes | done (ours) | 44 translation catalogs besides English; right-to-left languages wait for a mirrored layout. The vendors' language counts not verified |
