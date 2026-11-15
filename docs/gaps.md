# Where StochCraft's window falls short

> **Last reviewed:** 2026-11-15 · **Last updated:** 2026-11-15 · **Target:** what users of FlexSim
> (User Manual 2026) and AnyLogic 8 (public help) see and do, and what a newcomer needs

Every known shortfall of the window, one entry each, ranked by how much it stops a switcher or a
newcomer from getting a trustworthy answer. Pick work from the top unless the owner says
otherwise. The numbers behind the ranking are in [target-app-parity.md](target-app-parity.md) and
the row-by-row checklist in [ui-parity.md](ui-parity.md).

Hours are estimates for one worker, sequential, tests included. Where the review gave an hour
range, it is used as given. Where it gave days or a size, the hours are converted and marked
"(converted)": a day is 8 h; S is under a day (up to 8 h), M a few days (16–40 h), L one to three
weeks (40–120 h).

## Ranked

P1 stops a user in the first session; P2 costs trust or time; P3 is depth for later projects.

| # | Sev | Gap | Fix | Hours |
|---|---|---|---|---|
| 1 | P1 | **Results don't answer the question, and trust is not at the top.** A run lands on numbers such as "80.696 ± 2.8373 %" and 11 rows per task; a single-server queue run from empty with no warm-up and a ±31% interval on the wait gave no warning | 2–4 finding sentences above the charts (the busiest block, the waits, what limits throughput); a trust line with one-click fixes ("±31%: run 40 replications", "no warm-up"); numbers rounded to what the interval supports; 4 key measures per block with the rest under "All measures"; OEE rows only when a planned run time is set | 24–48 (converted from 3–6 days) |
| 2 | P1 | **The canvas is a diagram, not a simulation.** Watching a run shows count badges; nothing moves, queues are not drawn, results never appear on the model; task labels are about 8 px | Entities moving along the flows and stacking in front of tasks (2D, on the flowchart, respecting reduced motion); busy, idle and down rings on blocks; results laid over the blocks after a run; larger shapes | 25–45 for 2D on the flowchart; a 2D layout with paths and nodes is M5 (80–150); 3D 150–300 |
| 3 | P1 | **The object library is a sideways-scrolling strip of nine text chips.** At 1440 px Departures, Batch and the splits are off-screen, and so are their focus states; no icons, groups or search; the names differ from "+ Add block" | A vertical library with BPMN glyphs, grouped (in and out, work, routing, batching), searchable, click to place and drag; one vocabulary everywhere; a quick add at the pointer | 20–35; a queue the user can see as an object, 10–20 more |
| 4 | P1 | **No what-if.** Editing throws the last answer away; the engine's paired comparison (`results.compare`) has no control in the window | Keep stale results visible and marked out of date; "Keep as baseline"; baseline against now with the paired difference and whether it is significant; then a scenario table (named rows, several parameters, replications each) | 24–40 for the first step (converted from 3–5 days); the scenario table 30–50 |
| 5 | P1 | **No help, and an idle first example.** Help holds only About; no glossary, tutorials or guided first model; the tandem-line example runs at 11% busy with no waiting, so the first result shows nothing to find; the start page leads with BPMN, XPDL and BPSim | An example gallery by question and industry (a coffee counter, a weld and paint line with breakdowns, Schmitt et al. 2026 Cases A and B, emergency triage, a call centre, an IT service with retries and health checks), each with its question and expected finding; a 5-step guided first model on the real canvas; glossary hovers; Help with Getting started and a user guide | 30–50 |
| 6 | P1 | **Accessibility** (quick fixes done in #140). The web app is one unlabelled canvas for screen readers; about 12 Tab stops show no focus; the focused header menu label is about 1.1:1; control borders 1.2–1.3:1 (3:1 needed); invalid input is a red outline only; errors and warnings differ by colour only; tooltips are mouse-only; no reflow at 360 px | Done (#140): a role and label on the canvas and a live region for the status bar; focus rings on blocks, and chips scrolled into view on focus; a control-border colour of 3:1 or more, in the theme checks; error text under number fields; an icon and text for errors and warnings. Left: explanations by keyboard, reflow at 360 px, and the real fix, a DOM mirror of the AccessKit tree on the web | the DOM mirror 40–120 (converted from L) |
| 7 | P1 | **Engine words leak.** New blocks were named `source-1`, `server-1`, `sink-1` (fixed in #139: "Arrivals 1", "Task with a resource 1"); messages say entities, element and sink, and use field keys ("service: mean must be 0 or more"); four to six words for the same thing (block, element, task, server; lost, turned away, abandoned, gave up); engine messages are English in every language | One vocabulary across the window, the results and the engine's messages; messages in the window's words that name the block and say how to fix it; engine messages through the catalogs (each rewrite is a row in 44 catalogs, so batch them) | 16–40 (converted from M), plus translation review |
| 8 | P2 | **Errors are let in, then reported far away.** Run stays enabled with problems and refuses with a status-bar line that names no block; negative means are accepted; the inspector jumps about 32 px when a problem line appears | Limits on fields; error text under the field; "Run (1 problem)" that selects the block; clickable status-bar errors; a fixed place for problems; Basics first in the inspector, the rest folded | 4–40 (converted from S–M) |
| 9 | P2 | **Nothing to download, and the README is not a landing page.** No release yet; about 230 words of crate names before any picture. (The stale status line and the "agent-based" claim were fixed in #139.) | Tag the first release from the release workflow (#130); the README as a landing page (one picture, what it answers, try in the browser, download) | up to 8 (converted from S) |
| 10 | P2 | **Charts are fixed.** Four charts; no choice of chart, pinned statistics or dashboards; state times only as numbers; queue over time only for the watched block; no event log or tracing | A chart picker on any measure (using the chart palettes already built); state charts per block and a state timeline; time plots; a dashboard layout saved with the model; an event log for the watched run later | 50–90 for chart choice and dashboards; state and time views 25–40; debugging views (event log, tracing one entity, console per replication) 20–35 |
| 11 | P3 | **Depth for real projects.** No named model parameters or user-defined outputs; no picture of a distribution while choosing it, and no fitting; no editing of several blocks at once; no export to a workbook or a report document | Named parameters and outputs that experiments, sweeps and charts use; a guided distribution choice with a live picture, option menus beside fields, a picker and multi-block editing; import of tables and fitted distributions from data; a workbook and a report document (inputs, run record, findings, charts) | 25–45 parameters and outputs; 25–45 the guided choice and editing; 40–70 data in; 25–45 export |

## Also open

Rows of the checklist that the ranking above does not cover:

| Gap | Hours |
|---|---|
| Pools and operators drawn on the canvas and linked to the tasks that use them | 15–30 |
| A New model button on the start screen; a set-up dialog for name, time unit and start date and time; templates in the window | 8–15 |
| A 2D floor plan under the model, paths, transporters, then a 3D view (M5 and later) | 230–450 |

## Order

1. **Quick wins (days):** the release and the README (9); the accessibility quick fixes (6, done); the
   vocabulary and messages (7); field errors (8).
2. **The answer:** findings, the trust line, rounding and folded measures (1); keep results and
   compare with a baseline (4).
3. **The first hour:** the example gallery and the guided first model, Help and the glossary (5);
   the library (3).
4. **The look of a simulation:** entities moving in 2D on the flowchart, results on the canvas (2).
5. **Depth:** the chart picker and dashboards, state charts, scenarios, named parameters (10, 11,
   and the scenario table in 4); then the 2D layout (M5) and 3D.

Steps 1–4 come to about 150–345 h (estimated, sum of the rows above at their first step).

## Keep

What works and should survive the changes: every mean has a 95% interval and its method is
stated; the run record and model digest; each edit is one named undo step; rebindable keys;
validation as you edit; click alternatives to dragging; target sizes pass WCAG 2.5.8; text contrast
passes in the built-in themes; the Analysis view's plain explanations; sensitivity and sizing need
no set-up; the app runs in the browser with no account.
