# StochCraft and AI agents (MCP)

StochCraft speaks the [Model Context Protocol](https://modelcontextprotocol.io) over stdio
(JSON-RPC 2.0, one message per line; protocol revisions 2025-06-18, 2025-03-26 and 2024-11-05).
An agent builds, runs and compares models the way a person does: every tool runs engine
commands, the same ones the app, scripts and the command line run, and never opens a dialog.
The server is `crates/mcp` (`stochcraft-mcp`); `stochcraft-cli mcp` runs it.

## Start it

```sh
# Headless (the default): an engine in the server's own process, no window.
stochcraft-cli mcp

# Bridged to the running app, so you can watch the agent work:
stochcraft --control 7070 &
stochcraft-cli mcp --connect 127.0.0.1:7070
```

With `--connect`, an app that cannot be reached when the server starts gives a headless session
instead (a note goes to stderr), and so does one that is not running (no token file and no
token given). An app that is reachable but refuses the connection (a wrong token, all its places taken) is an error, not a headless session (see [The token](#the-token)).
stdout carries only protocol messages; errors go to stderr.

### Claude Code

```sh
cargo build --release -p stochcraft-cli
claude mcp add stochcraft -- /path/to/stochcraft/target/release/stochcraft-cli mcp
# bridged to a running app:
claude mcp add stochcraft -- /path/to/stochcraft-cli mcp --connect 127.0.0.1:7070
# kept to one project folder:
claude mcp add stochcraft -- /path/to/stochcraft-cli mcp --read-root /work/plant --write-root /work/plant/out
```

The same as a project `.mcp.json`, or in another client's configuration (Claude Desktop's
`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "stochcraft": { "command": "/path/to/stochcraft-cli", "args": ["mcp"] }
  }
}
```

## Confining file access

```sh
stochcraft-cli mcp --read-root /work/plant --write-root /work/plant/out
```

The two flags keep a headless agent to the folders you give it:

- Every file `file.open` (the `open` tool) reads must lie inside the **read root**.
- Every file `file.save` (the `save` tool) writes must lie inside the **write root**, including a
  save into the file the model was opened from.
- Read and write access are separate: give the same folder twice for both. A root left out
  grants none of its access, so with only `--read-root` nothing can be saved. Without either
  flag nothing is confined.
- Paths are checked as FieldCraft checks them:
  - A path is made absolute against the server's working folder, then its links are followed. A
    link that leads out of a root is outside it.
  - For a file that does not exist yet, the links of its nearest existing folder are followed
    and the rest must be plain names.
  - The result is compared with the root by whole folder names: `/work2/x` is not inside
    `/work`.
  - On Windows, `..` is removed from a path's text first (as `std::path::absolute` and Windows
    itself do), so `<root>\nope\..\x` is accepted as `<root>\x`, the file opening it would reach,
    even when `nope` does not exist. On Unix the system would look for `nope`, so the same path
    is refused there (#77).
- A refused path is an ordinary tool error the agent can read, such as
  `path refused: outside the write root /work/plant/out: /etc/x.bpmn`.
- The roots must be existing folders; a missing one stops the server with an error.

The roots cannot be combined with `--connect`: the app opens and saves files wherever its user
asks, so an agent driving it is not confined. What stops another account's programs from
driving the app is the channel's token (below).

## Tools

| Tool | Arguments | What it does |
|---|---|---|
| `list_commands` | `filter?` | Every engine command with its summary, its parameters as JSON Schema and an example; `filter` matches the name and summary. |
| `execute` | `command`, `params?` | Runs any command (`list_commands`). A failure leaves the model unchanged, and the reply has the engine's error `code`, `message` and `details` (the model's diagnostics, for example). |
| `batch` | `commands: [{command, params}]` | Runs the steps in order and stops at the first error. The reply has every result and the failing step (`failed.index`, `failed.command`, `failed.error`). The steps before it stay applied (`undo` takes them back). |
| `inspect_model` | — | `model.get` (elements, connections, pools, calendars, experiment) and `history` (what undo and redo would do, unsaved changes). |
| `check_model` | — | `model.validate`: every problem, errors first, with the element, a stable code and a message in English that starts with its severity, names the block by its name and says what to do. The same text comes as `parts` (each a template `key` and the `args` that fill it) and, for a problem with one field, `field`: the window builds its translated message from those, and a client may do the same. The code never changes with the wording. |
| `open` | `path`, `scenario?` | `file.open`: a `.bpmn` or `.xpdl` file with BPSim data. |
| `save` | `path?`, `as_new?` | `file.save`: into the model's own file (keeping what StochCraft does not understand), or to `path`. |
| `run` | `replications?`, `seed?` | `run`: each element's metrics as means with 95% confidence intervals, the warm-up MSER-5 suggests (`warmup_suggestion`), the warnings about the results, and the run record. The values over the run (`series`) are left out, since they multiply the reply's size; `execute` with `run` gives them. An experiment with sequential replications (`experiment.set` with `sequential`) adds replications until its key measures are precise enough, and `sequential` in the results says how many ran and why it stopped. |
| `undo`, `redo` | — | `undo`, `redo`. |

Real measurements to check a run against (black-box validation) are set with `execute`:
`observation.add`, `observation.set` and `observation.remove`. An observation is for one result,
a block's or pool's `element` and a `metric` such as `mean_wait`, `utilisation` or `left`, in the
metric's own unit (a share as a fraction), and is one of three `check`s: `value` (the observed
`value`, with the `half_width` of its own 95% interval where its data give one, and a `tolerance`
above 0 in the metric's unit, which only an observed value takes: any other check with one is
`invalid_params`), `at_most` or `at_least` (a `bound`). One for each result: `add` fails with
`conflict` if there is one, `set` with `not_found` if there is none. They are saved in the file
(`stochcraft:observed`) and come back in `model.get` as `experiment.observed`. A run's results
then have `observed`, one entry for each, those that `differ` or `fail` first, then those
`inconclusive` or `no_difference_detected`, then those that `agree` or `pass`, each with the
`observed` value (and its `half_width` and `tolerance`) or `bound`, the `simulated` mean with its
95% `half_width` and `n`, for an observed value the `difference` (simulated minus observed) with
its 95% `difference_half_width`, and the `verdict`: `agrees`, `differs`, `no_difference_detected`,
`passes`, `fails`, `inconclusive` or `no_result`. The difference's half-width is
`hypot(half_width, observed half_width)` (the usual large-sample approximation; the observed one
counts as 0 where not given). Without a `tolerance`, the interval excluding 0 is `differs` and
holding 0 is only `no_difference_detected`, never agreement: a wide interval holds 0 whatever the
truth. With a `tolerance` t it is an equivalence test: `agrees` when the whole interval is inside
[-t, +t], `differs` when wholly outside, else `inconclusive` (the 95% interval is conservative
against the 90% interval of the two one-sided tests). A bound passes only when the whole
simulated interval meets it and fails only when the whole interval violates it, and is
`inconclusive` where the interval straddles it (more replications would decide) or there are
fewer than 2. Observed data for a block that is gone is the warning `bad_observation` in
`check_model` (the run still goes, and says `no_result`).

Resources: `stochcraft://model` (`model.get`) and `stochcraft://commands` (the command catalogue).

Errors work as follows:
- An unknown tool, or arguments that do not match the tool's schema (a missing or unknown
  argument, a wrong type), is a JSON-RPC `-32602` error.
- An engine failure is a tool result with `isError: true` and the engine's error as JSON.

## The control channel

`stochcraft --control PORT` (1 to 65535) listens on `127.0.0.1:PORT` (the loopback interface
only), and the window runs the commands that arrive there, so the person at it sees each change.
On Windows a release build has no console, so the line saying where it listens is not shown; the
port is the one you gave.

Each request is one JSON line, and the app answers each with one line:

- `{"id", "method": "engine.execute", "params": {"command", "params"}}` runs a command. The
  answer is `{"id", "result"}` or `{"id", "error": {"code", "message", "details"?}}`.
- `{"id", "method": "engine.commands"}` answers with the command catalogue.

Commands from the channel behave in the window as they would from the person:
- A model the agent opens or starts replaces the one shown.
- `run` shows its results.
- Edits go on the undo history.
- The window's background runs (`run.start`, `run.status`, `run.cancel`) are its own and are
  refused; use `run`.

### The token

The loopback address limits where a connection comes from, not who makes it, so on a computer
shared by several accounts a program under another account could reach the port. The channel
therefore asks every connection for a token:

- When the app opens the channel it makes a random 256-bit token from the operating system's
  random source, shown as 64 hexadecimal digits, and writes `{"port": 7070, "token": "..."}` to a
  file only its user can read:
  - Linux and other Unix systems: `$XDG_RUNTIME_DIR/stochcraft/control.json`, or, where that is
    not set, `control.json` in `stochcraft` under the user's config folder (`~/.config` on Linux,
    `~/Library/Application Support` on macOS). The file is made with mode 0600, in a folder of
    mode 0700 (an existing folder with another mode is changed to 0700, and the app refuses to write the token there if it cannot), by writing a file of its own and renaming it over the old one, under a lock file beside it (`control.json.lock`) so two apps starting together cannot both take the file.
  - Windows: `%LOCALAPPDATA%\StochCraft\control.json`. The profile's default permissions already
    limit that folder to the user (and administrators), and StochCraft adds no permissions of its
    own there.
- The file is removed when the app closes. If another running app still answers on the port
  named in an existing file, a second app's `--control` fails and says so, rather than replace the
  first one's token; close the first one.
- Each connection's first line must be `{"token": "..."}`. The channel compares it with the
  token in constant time and answers `{"id": null, "result": {"authenticated": true}}`. A missing
  or wrong token, or no whole line (of at most 1 KiB) within 2 seconds of connecting, gets one error line with the code `unauthorized`,
  and the connection is closed without running anything.
- `stochcraft-cli mcp --connect` reads the token from that file by default. The environment
  variable `STOCHCRAFT_TOKEN` or `--token HEX` give one by hand, for when the file is not
  reachable (an app on another computer, reached by a tunnel, or under another account that
  shared the token on purpose); they take precedence over the file, `--token` first. On some
  systems other local users can see a program's arguments in the process list, so prefer
  `STOCHCRAFT_TOKEN` or the token file to `--token`. With no token given and no token file, the
  app is taken to be not running: the session is headless, with a note on stderr naming the
  file. An app that is there but answers the connection with an error (a wrong token, or all 8
  places taken) is an error that stops the server, never a headless session.

The token keeps other accounts out. A program running as the same user as the app can read the
file, as it could read that user's other files.

## Limits

The server treats its input as hostile:

- A message line may be at most 16 MiB. A longer line is answered with `request too large` and
  skipped, and the server keeps serving.
- A JSON-RPC batch holds at most 100 messages, and a `batch` call at most 10 000 commands.
- An engine failure leaves the model as it was.
- The control channel takes at most 8 clients at once. One more gets a `conflict` error naming
  the limit and is closed. Connections are not timed out: an agent keeps its connection, idle
  between calls, for its whole session.

## A typical session

```text
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"me","version":"1"}}}
→ {"jsonrpc":"2.0","method":"notifications/initialized"}
→ {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"batch","arguments":{"commands":[
     {"command":"block.add","params":{"kind":"source","id":"arrivals","interarrival":{"type":"exponential","mean":2}}},
     {"command":"block.add","params":{"kind":"server","id":"desk","service":{"type":"exponential","mean":1.5}}},
     {"command":"block.add","params":{"kind":"sink","id":"done"}},
     {"command":"connect","params":{"from":"arrivals","to":"desk"}},
     {"command":"connect","params":{"from":"desk","to":"done"}},
     {"command":"experiment.set","params":{"replications":20,"duration":480}}]}}}
→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"check_model","arguments":{}}}
← … {"diagnostics": []} …
→ {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"run","arguments":{}}}
← … "utilisation": {"mean": 0.74…, "half_width": 0.01…} …
→ {"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"execute","arguments":{"command":"block.set","params":{"id":"desk","capacity":2}}}}
→ {"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"run","arguments":{}}}
→ {"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"execute","arguments":{"command":"results.compare"}}}
← … "paired": true, … {"element": "desk", "metric": "mean_wait", "a": {"mean": …}, "b": {"mean": …}, "difference": -…, "half_width": …, "significant": true, …} …
→ {"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"save","arguments":{"path":"/work/desk.bpmn"}}}
```

`results.compare` compares the last run with the one before it (or any two runs' results given
as `a` and `b`). Runs with the same seed and replications share their random numbers, so it
compares them replication by replication (a paired-t interval); otherwise it uses Welch's
two-sample t interval and says so.

Each metric in `run`'s results carries `n`, the replications that had a value for it, which can
be fewer than `replications` (a server no entity reached, or replications that stopped inside
the warm-up). `experiment.size` takes `target_share`, the share of replications that must meet
the target; `confidence`, its name before #69, is still accepted, and the answer carries both.
The share is not a statistical confidence: the answer's `stochastic.interval` is its Wilson 95%
interval, and when its lower end is below `target_share` the share is not shown to be met.

`experiment.size` is a greedy heuristic: it adds a unit at the busiest resource until the share is
reached, and need not find the fewest units. It refuses a target that names nothing in the model
(`not_found`) and a metric that no replication reports for that element at the starting size
(`invalid_params`: the metric does not apply to that kind of element, or nothing reached it). With
`apply`, the answer becomes the model's only if it reached `target_share`; otherwise the model is
unchanged and the answer says `"applied": false` (`apply_unreached` applies the last size tried
anyway). The share and its interval come from the replications that guided the search, so check
the sizes it chose with a run on another seed.

A file read with errors (`open` returns them in `notes`, with severity `error`) does not run:
`run`, `run.start` and the `experiment.*` commands fail with `invalid_model` and the errors in
`details`, because the model holds a stand-in for each part StochCraft could not read. Run it
anyway with `allow_approximations: true` (through `execute`); the run record then lists the
substitutions in `approximations`. An error the reader pinned to one field (a task's processing time, `field` in the note) stops
counting while that field is set to something else (undo brings it back), and nothing of the
replaced stand-in is recorded; renaming the block or editing another property does not clear it,
and any other error stands until the file is opened again.

`experiment.design` varies several parameters at once. Each factor is a parameter as a sweep
takes it (`{id, field}`) with its `levels`, or its range as `min` and `max`. The default design,
`full_factorial`, runs every combination of the levels (the last factor changing fastest);
`latin_hypercube` with `points` samples each range in that many strata, the same design for the
same seed. Every point runs the same replications on the same seed, so a full factorial's
`main_effects` and two-way `interactions` (between factors with two levels) come with paired 95%
intervals, and a Latin hypercube's `correlations` give each factor's rank (Spearman) correlation
with each result. A design of more than 10,000 runs (points × replications) is refused with the
number it would take, so an agent can drop levels, points or replications and try again.

A Monte Carlo model (a calculation with no process) is built and run through `execute`:
`mc.input.set` and `mc.output.set` add inputs (distributions) and outputs (expressions over
them, with `$name`), `mc.correlation.set` correlates two inputs, `mc.set` picks the sampling
(`random`, `latin_hypercube`, `antithetic` or `sobol`) or a sequential stop, and `mc.run` gives each output's mean with its interval,
percentiles and the chance of meeting its target. `mc.inputs.export` and `mc.inputs.import` move
the input table as CSV text; they read and write no files. `mc.sobol` gives each input's share of
each output's variance, alone and with its interactions (it needs independent inputs). `mc.output.set` also takes `controls`, control variates that narrow the interval of the output's mean and target chance: `[{"input": "labour"}, {"output": "base", "expected": 5200}]` names inputs (whose own mean is known; counts with a very large spread and constants are refused) and earlier outputs with a mean you give, replacing the output's present controls (`[]` removes them). `mc.run` then adds a `controlled` object to the output: the controls with their coefficients, and `controlled.estimate` with the controlled `mean`, `half_width` and `narrowing` (how much narrower the controlled interval is than the plain one on the same trials; `helped` is at least 10%), and for a target `controlled.estimate.target` with its `probability` and `interval`. `controlled.estimate` is `null` when there is no controlled estimate (`unavailable` says why). A control object has the fields of one form only: `{"input": ...}` or `{"output": ..., "expected": ...}`. The coefficients are fitted on a pilot that the controlled estimate leaves out: the first 1,000 trials, or the first half of a random run under 2,000 trials (whole pairs for antithetic sampling); with Latin hypercube or Sobol sampling or correlated inputs the pilot is the whole first batch, so one batch gives no controlled estimate (two are needed, three for its interval). The estimate is unbiased for a run of a fixed number of trials, not for a run that stops at a precision target, and a trial with no number in the output or a control withdraws it (`unavailable` says so). A replacing `mc.inputs.import` removes the controls of inputs it drops. Percentiles, the histogram and the extremes are not controlled.

A system dynamics model (stocks and flows, an XMILE file) is opened with `file.open` or `sd.open`
(the text of the file), run with `sd.run` (`replications` and `seed` for a model with a random
function, `dt`, `dt_check` to run again at half the step, `time_unit`) and read with `sd.get`. A
run gives every variable over time: the mean, and for a replicated model the median and the 5th
and 95th percentiles. `run` refuses such a model and names `sd.run`. `sd.new` starts an empty model
(name, start, stop, `dt`, method, time unit) and `sd.variable.add`, `sd.variable.set` (with
`rename`), `sd.variable.remove`, `sd.specs.set` and `sd.layout.set` edit the open one. An edit is
compiled before it is kept: a bad equation, a name that is not there or an algebraic loop is
refused with the compile error and the model is unchanged, so add a stock before the flows that
name it (`into_stock` and `from_stock` attach a new flow) and set the equations that need
variables not yet there afterwards. A removal is refused while another equation or a stock uses
the variable, and the error names who. Edits go on the same `undo` and `redo` as the process
commands, make the last results stale (`sd.get` says so), and `file.save` writes the model as an
XMILE file with its diagram. `sd.get` gives each variable's position (and a flow's pipe) and the
information links. The sd edits are not `batch`able. In the app, a model an agent opens with `file.open` or `sd.open`
replaces the one shown, and the results of its `sd.run` appear on the Results tab, as for the other
kinds of model. Its edits show on the Model tab, mark the model edited (the window's Save writes it
as XMILE; into a file with content StochCraft does not keep, it asks the user first, whereas
`file.save` writes at once) and mark the results on show out of date until `sd.run` runs again.

`crates/mcp/tests/session.rs` covers four areas:
- a session like this one, checked only through MCP;
- malformed messages, oversized lines and batches, and arguments that do not match the schema;
- the read and write roots;
- the same tools bridged through a control channel.

`crates/ui-egui/tests/control.rs` checks that an agent's commands show in the window.
