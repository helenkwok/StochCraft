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
instead (a note goes to stderr). stdout carries only protocol messages; errors go to stderr.

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
asks, so an agent driving it is not confined.

## Tools

| Tool | Arguments | What it does |
|---|---|---|
| `list_commands` | `filter?` | Every engine command with its summary, its parameters as JSON Schema and an example; `filter` matches the name and summary. |
| `execute` | `command`, `params?` | Runs any command (`list_commands`). A failure leaves the model unchanged, and the reply has the engine's error `code`, `message` and `details` (the model's diagnostics, for example). |
| `batch` | `commands: [{command, params}]` | Runs the steps in order and stops at the first error. The reply has every result and the failing step (`failed.index`, `failed.command`, `failed.error`). The steps before it stay applied (`undo` takes them back). |
| `inspect_model` | — | `model.get` (elements, connections, pools, calendars, experiment) and `history` (what undo and redo would do, unsaved changes). |
| `check_model` | — | `model.validate`: every problem, errors first, with the element, a stable code and a message. |
| `open` | `path`, `scenario?` | `file.open`: a `.bpmn` or `.xpdl` file with BPSim data. |
| `save` | `path?`, `as_new?` | `file.save`: into the model's own file (keeping what StochCraft does not understand), or to `path`. |
| `run` | `replications?`, `seed?` | `run`: each element's metrics as means with 95% confidence intervals, and the run record. |
| `undo`, `redo` | — | `undo`, `redo`. |

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

Any program on the computer that can reach the port can drive the app, as a person at the window
can.

## Limits

The server treats its input as hostile:

- A message line may be at most 16 MiB. A longer line is answered with `request too large` and
  skipped, and the server keeps serving.
- A JSON-RPC batch holds at most 100 messages, and a `batch` call at most 10 000 commands.
- An engine failure leaves the model as it was.

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

`crates/mcp/tests/session.rs` covers four areas:
- a session like this one, checked only through MCP;
- malformed messages, oversized lines and batches, and arguments that do not match the schema;
- the read and write roots;
- the same tools bridged through a control channel.

`crates/ui-egui/tests/control.rs` checks that an agent's commands show in the window.
