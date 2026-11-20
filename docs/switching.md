# Coming from another simulation tool

A short guide for people who have built discrete-event models in another tool. It maps the usual
words onto StochCraft's, says how runs and results work here, and how to bring in a process you
already have. The [user guide](user-guide.md) has the details.

## The words

| You may know it as | In StochCraft |
| --- | --- |
| Source, generator, arrivals | **Arrivals**: the time between one arrival and the next, a limit, arrivals by calendar |
| Queue, buffer | The **Waiting room** of a task (with or without a limit), or a separate task |
| Server, processor, machine, station, activity | **Task with a resource**: a number of **units** that each work on one entity at a time |
| Delay, travel | **Task**: takes time, with no limit on how many entities at once |
| Conveyor, belt | **Conveyor**: a length and a speed, each entity taking its item length plus a gap, accumulating or not |
| Shared resource, operator team, pool | A **resource pool**, used by a **Task with shared resources** |
| Sink, exit, absorber | **Departures**: counts entities and their time in the system |
| Router, decision point, branch | **Decision (one way)**: by chance or by a condition |
| Combiner, separator, assembly | **Split or join**, **Batch** and **Split batches** |
| Entity, item, agent, flow item | **Entity** |
| Replication | **Run**; all runs together are the **experiment** |
| Run length, stop time | **Duration** (after the **warm-up**), or **Ends early** on a target count or an empty task |
| Schedule, shift pattern, timetable | A **calendar**, which tasks, pools and arrivals follow |
| Downtime, MTBF and MTTR | **Breakdowns**: time to failure and repair time |

## How runs and results work

- **Every result has a 95% confidence interval.** An experiment is several runs with different
  random numbers; each mean comes with ± its 95% interval, and is rounded to what that interval
  supports. One run gives no interval, so the window offers more runs.
- **The answer comes first.** The Results tab opens with what the run found (which block limits
  the flow, where the time goes), then how sure the numbers are, with one-click fixes such as more
  runs or a warm-up, then each block's key measures. Everything else is folded under All measures.
- **Warm-up is part of the experiment**, not a separate statistics setting, and the window
  suggests one when the start from empty pulls the means.
- **Comparing two versions is built in.** Keep the results as a baseline, change the model, run it
  again: the window shows the paired difference of each key measure and whether it is real or
  could be chance. With the same seed and number of runs, run 1 of each meets the same random
  numbers, so the comparison is much tighter.
- **The Analysis tab** sweeps a parameter, ranks which parameters matter, and sizes resources to a
  target.
- **The same model and seed give the same results**, run after run.

## The keys

F5 runs, F1 opens Help beside the model, Ctrl+Z and Ctrl+Shift+Z undo and redo (⌘ on a Mac), and
Space plays or pauses a watched run. **Help ▸ Keyboard Shortcuts…** lists every key, and any of
them can be changed.

## Bringing in a process you already have

StochCraft reads and writes process diagrams in the open BPMN 2.0 format with BPSim 2.0
simulation data, and reads XPDL 2.2 with BPSim. A diagram drawn in a process-modelling tool opens
with its layout, and what StochCraft cannot use is listed when the file opens. Times,
distributions, capacities and calendars come from the BPSim data; where a diagram has none, the
blocks open with defaults to fill in.

Models from simulation tools with their own file formats cannot be opened directly. Rebuild them
from the table above: most models of queues, machines and staff take a few minutes.
