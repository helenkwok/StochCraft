# StochCraft parity with established tools

> **Last reviewed:** 2026-11-15 · **Last updated:** 2026-11-20 · **Target:** what users of established commercial simulation tools see and do (from their public documentation),
> from the start screen to the report

How close StochCraft's window is to what a user of an established tool expects, and to what a
newcomer to simulation needs. The checklist behind the numbers is [ui-parity.md](ui-parity.md);
the ranked work list is [gaps.md](gaps.md). This is about the user experience only. The engine's
results are checked against queueing theory and published papers (README, ROADMAP); no results of
the vendor tools are compared here.

Two specific commercial tools were assessed, called "the 3D-first tool" and "the flowchart-first
tool" below. Every estimate applies to those two tools only, not to every tool of either kind.

## Headline

| Number | Value | Kind |
|---|---|---|
| UI/UX fidelity, weighted to the first hour | **~44%** | estimated from the checklist (rule below) |
| Fidelity of the 23 first-hour rows alone | **~55%** | estimated (7 done, 2 done (ours), 9 partial, 5 missing) |
| The 3D-first tool's first tutorial (its first two tasks) done in StochCraft | **9.5 of 17 steps** | estimated, step by step |
| The flowchart-first tool's first tutorial (its first four phases) done in StochCraft | **6 of 11 steps** | estimated, step by step; the steps are from public summaries, not confirmed |
| Ready for a switcher from the 3D-first tool | **~51%** | estimated (area weights below) |
| Ready for a switcher from the flowchart-first tool | **~54%** | estimated |
| Ready for a newcomer to simulation | **~51%** | estimated |
| Nielsen's ten heuristics | **20 / 40** | judged; the bottom of the "acceptable" band (most real interfaces score 20–32) |
| Cognitive-load checklist | **5 of 8 failed** | judged; critical |
| WCAG 2.2 AA | **fails**: 2 critical, 8 major, 7 minor findings | tested on the browser app with the keyboard; see [Accessibility](#accessibility) |
| Rows graded | 91: 27 done, 10 done (ours), 26 partial, 28 missing | counted from the checklist |

**Verdict.** The engine is ahead of the interface. A switcher can do a little over half of their
usual first-hour steps, but the missing half is the visible half: the first run shows counts in
badges where the 3D-first tool shows items moving and piling up in a queue (and the flowchart-first tool, from public
summaries, not confirmed, animates them once the space is drawn); the library is a strip of text
chips where the vendors have a docked panel of icons; there is nothing like their user-built
dashboards; and Help holds only About. The visual style is calm and readable and does not look
machine-made. What is missing is the simulation-tool experience: a library to pick objects from,
things that move when the model runs, results that answer the question, a what-if you can compare,
help, and an app that works for keyboard and screen-reader users.

## Readiness by audience

| Audience | Ready | Work that dominates |
|---|---|---|
| Switcher from the 3D-first tool | ~51% | layout and animation weigh most (it is taught 3D first): moving entities ([gaps](gaps.md) #2), the library (#3), user-chosen charts and dashboards (#10); a 2D layout and 3D later |
| Switcher from the flowchart-first tool | ~54% | flowchart users lean on parameters and experiments (from public summaries, not confirmed): what-if comparison and scenarios (#4), named parameters and outputs (#11), the library (#3), moving entities (#2) |
| Newcomer to simulation | ~51% | results that answer the question (#1), help, a guided first model and examples that show something (#5), one vocabulary (#7), field errors (#8) |

Estimated remaining effort (one worker, sequential, tests included), summed from the hour ranges
of the parity rows in [gaps.md](gaps.md): **~175–315 h** to a credible first hour for a switcher
(the library with a visible queue, moving entities in 2D, help and tutorials, chart choice and
dashboards, pools on the canvas, the guided distribution choice and editing), and **~580–1,065 h**
to broad parity, adding scenarios, named parameters and outputs, state and time views, debugging
views, new-model set-up, data in, export, the 2D layout (M5) and 3D. Both leave out the findings,
accessibility, wording, field-error and release work, so they are a different grouping from the
"steps 1–4" total in [gaps.md](gaps.md#order).

## By area

Fidelity counts the done, partial and missing rows whose vendor side was checked; readiness also
counts StochCraft's own strengths ("done (ours)"). One partial row, findings stated in words, has
no vendor counterpart but counts in fidelity like any partial (see the end of this page).

| # | Area | 3D-first tool weight | Flowchart-first tool weight | Newcomer weight | Fidelity | Ready |
|---|---|---|---|---|---|---|
| 1 | Start screen, new model, examples | 5 | 5 | 10 | 44% | 50% |
| 2 | Library and canvas | 15 | 15 | 10 | 46% | 56% |
| 3 | Connecting and flow logic | 5 | 8 | 5 | 67% | 67% |
| 4 | Properties and parameters | 10 | 12 | 8 | 58% | 61% |
| 5 | Distributions and input data | 8 | 6 | 4 | 42% | 50% |
| 6 | **Layout, 2D/3D and animation** | 20 | 10 | 5 | **32%** | **32%** |
| 7 | Running and run controls | 8 | 8 | 8 | 88% | 88% |
| 8 | Statistics on objects while running | 6 | 6 | 6 | 55% | 55% |
| 9 | Experiments | 8 | 12 | 10 | 44% | 50% |
| 10 | **Results, charts, dashboards, export** | 10 | 10 | 14 | **23%** | 44% |
| 11 | **Help and learning** | 3 | 4 | 14 | **17%** | **17%** |
| 12 | Errors, validation, debugging | 2 | 4 | 6 | 0% (only the two missing debugging rows have a vendor side checked) | 71% |
| 13 | Files and projects | 0 | 0 | 0 | 50% | 86% |

Area 2 is where the first impression is made, so treat it as a fourth weak area beside 6, 10 and
11. Why these weights: the 3D-first tool is taught 3D first (its opening tutorial builds a 3D layout before
any logic), so layout and animation weigh most for its users; users of the flowchart-first tool start from a flowchart
of process blocks and lean on parameters, code and experiments (from public summaries, not
confirmed); a newcomer needs the start screen,
help, explained results and errors most. Files carry no weight: StochCraft's are already at par or
ahead, and a switcher does not judge a tool by them in the first hour.

## Where StochCraft is ahead

Checked in the code (`crates/ui-egui/src`) or the ROADMAP:

- **Every run is replicated.** Run gives 10 replications by default, in parallel, each mean with a
  95% interval and the method stated, with a run record and a digest of the model. In the 3D-first
  tool's window a run is one replication, and replications need an experiment set up first (the
  flowchart-first tool the same, from public summaries, not confirmed).
- **The spread across replications**: percentiles, the empirical distribution with its confidence
  band and the probability of meeting a target, with no set-up.
- **Sensitivity and sizing with no set-up**: rank every parameter by its effect on a result; add
  units at the busiest task or pool until a share of replications meets a target.
- **Pause at a time** while watching a run: play on to a typed time, then pause (not found in the
  vendor pages read).
- **Validation as you edit**, with no build step; each edit is one named undo step.
- **Three-point estimates**: fit a time to a minimum, a most likely value and a maximum or 95th
  percentile, and keep the estimate as the time's pedigree.
- **Open files and automation**: BPMN 2.0 or XPDL with BPSim, so a model drawn in another BPMN
  editor opens with its layout; the same engine from the command line and over MCP.
- **In the browser, offline, with no install or account**; 44 translation catalogs besides
  English; rebindable keyboard shortcuts; colour-blind-safe chart palettes with a contrast check.

A switcher meets most of these only after the first impression has formed.

At the review, findings in words existed only where replications split into two outcomes (#129).
Since then the Results tab states general findings too, such as which block limits the flow and
where the time goes (#165), and "How sure are these numbers?" flags an interval too wide to act on
and a start from empty that needs a warm-up (#164; [gaps](gaps.md) #1). The row stays graded as
the review found it.

## Nielsen's heuristics

| # | Heuristic | Score (0–4) | Main issue |
|---|---|---|---|
| 1 | Visibility of system status | 2 | Any edit drops the last results; Run with problems gives one status-bar line |
| 2 | Match with the real world | 2 | Engine words leak: new blocks named `source-1`, `server-1`, `sink-1`; messages about entities and sinks |
| 3 | User control and freedom | 3 | Named undo and redo, Esc, cancel; no context menu on the canvas; no way to keep results while editing |
| 4 | Consistency and standards | 2 | Different names for the same kind in the palette and the menu; result names that are easy to confuse |
| 5 | Error prevention | 2 | Run stays enabled with problems; negative means accepted, then flagged; no warning for a run from empty with no warm-up |
| 6 | Recognition rather than recall | 2 | Library chips off-screen; the connect handle appears only on the selected block |
| 7 | Flexibility and efficiency | 2 | Shortcuts, a keymap, CLI and MCP; no context menu, no command palette, no run comparison |
| 8 | Aesthetic and minimalist design | 2 | Calm look, but 11 result rows for a plain task and a small model in a large empty canvas |
| 9 | Help users recover from errors | 2 | Problems marked on blocks, but status-bar errors name no block and messages use field keys |
| 10 | Help and documentation | 1 | Help holds only About; no glossary or hover explanations for the statistical terms |
| | **Total** | **20 / 40** | |

## Accessibility

WCAG 2.2 AA fails. The two critical findings: the browser app exposes nothing to screen readers (one
unlabelled canvas; the desktop app has an accessibility tree, the web build does not), and about 12
Tab stops show no focus (library chips scrolled out of view, blocks on the canvas). The eight major
findings: an unreadable focused header menu label, control borders below 3:1, invalid input shown
by a red outline only, status messages not announced, errors and warnings told apart by colour
only, explanations in mouse-only tooltips, no reflow at 360 px, and results tables and charts
without table or chart semantics. Passes: target size (2.5.8), pointer alternatives to dragging
(2.5.7), and text contrast in the built-in themes.

Since the review, #140 fixed the quick part: the canvas has a role and a name and the status bar
is announced, Tab stops show their focus (checked on blocks, the block list, tabs, the library
chips and fields), the focused header item is readable, control borders
are 3:1 or more, a number field says why its text is not a number, and errors and warnings have an
icon and words. The scores above are those of the review and were not re-measured; the screen
reader still sees one canvas, not the controls in it (that needs a DOM mirror, [gaps](gaps.md) #6).

Limits: the desktop app was not tested with a screen reader; the contrast ratios were computed
with the WCAG formula by the reviewer and not re-checked; the empty accessibility tree on the web
agrees with eframe's web backend having no AccessKit adapter.

## Since the review (2026-11-20)

The numbers on this page, the verdict and the Nielsen scores are those of the review and were not
re-graded. Since then:

- **Help (area 11; heuristic 10):** Help beside the work with Getting started, a glossary,
  shortcuts and the user guide (#169), and a guided first model on the real canvas (#172). Help
  no longer holds only About.
- **Start screen (area 1):** a start page that asks what you want to find out (#170), and eleven
  examples, five of them checked against exact answers (#171) and a parcel sorter (#207); a search
  over every example, and the full list grouped by field.
- **Results (area 10; heuristics 1 and 8):** findings in words, how sure the numbers are, four key
  measures per block and two default charts (#163, #164, #165); results stay, marked out of date,
  after an edit, and compare with a kept baseline (#166, #167, #168); a dashboard panel with four
  fixed tiles (#209).
- **Errors (heuristics 5 and 9):** Run with problems goes to the next problem's block and field
  instead of running, and number fields refuse what the model would (#146); every problem's
  message names the block, uses the window's word for the field and says how to fix it (#221).
- **Words (heuristic 2):** one vocabulary across the window, the results and the engine's
  messages, which go through the catalogs (#139, #221).
- **Layout and animation (area 6):** a 3D view (#205, #208, #210, #213) and a read-only Layout
  view to scale (#216), both placed from the diagram; entities moving on the flowchart are still
  missing (#151).
- **Run controls (area 7):** a run bar with Play/Pause on Space, Reset, Step, a speed with keys to
  halve or double it, As fast as possible, Pause at a time and a clock (#218).
- **Release:** v0.2.0 downloads for Windows, macOS and Linux (#214).

## How it was measured, and its limits

- **Four independent reviews**, none seeing the others: a design critique (Nielsen's heuristics,
  a cognitive-load checklist, personas for a switcher, a newcomer and a keyboard and screen-reader
  user); the parity checklist; accessibility (WCAG 2.2 AA) and interface wording; positioning and
  onboarding. The design and accessibility reviews drove the browser app built from main
  `6292048` (at 1440 × 900, and at 640 and 360 px for reflow).
- **The parity checklist** was graded from screenshots of a first session, the ROADMAP's M3
  sections, the README and the module documentation in `crates/ui-egui/src`, at `7ed5776`, two
  commits before `6292048` (those two change the waiting-room and delay logic and seven lines of
  the inspector). The app itself was not driven for it, and no click counts were measured: step
  counts are the steps of the vendors' own first tutorials.
- **3D-first tool**: 39 pages of its public User Manual 2026 read as text; most cited pages read in full,
  about ten only in their opening paragraphs.
- **Flowchart-first tool**: its help pages refuse automated reading, so its rows rest on search summaries of
  the official help pages and say "from public summaries, not confirmed"; a person with a browser
  should confirm them. The vendor's features page was read directly. Rows describe its desktop
  version 8 unless they say otherwise.
- **Clean room** (ROADMAP principle 6): the vendors' documentation was read as prose and is
  described here in generic terms. Nothing was copied, no vendor software was run, and no vendor
  images or example models were used. A vendor behaviour not found in the pages read is "not found
  in the pages read", not "missing in the product".
- **Scoring.** done 1, partial 0.5, missing 0; first-hour rows **[1h]** count three times.
  Fidelity leaves out "done (ours)" rows and rows whose vendor side was not checked **[nv]**;
  readiness counts "done (ours)" as 1. Areas are weighted by the sum of the three audience weights
  for fidelity and by each audience's weights for readiness. Row statuses are judgements and the
  weights are written above to be argued with, so every percentage is an estimate.
- **Changed from the review.** The review listed "findings stated in words" as done (ours); the
  code has them only for split outcomes, so the row is partial here. That moves fidelity from ~43%
  to ~44% (the row now counts) and readiness for a switcher from the 3D-first tool from ~52% to ~51%.
- StochCraft has no users yet; nothing here is user research.
