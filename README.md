# StochCraft

Discrete-event and agent-based simulation for factories, logistics and queueing systems: an
open-source, clean-room simulation application in pure Rust, for native desktops and the browser.

**Status:** just started. `stochcraft-des` runs SimPy-style async processes with resources on a deterministic event queue; `stochcraft-experiment` runs replications in parallel with reproducible random streams (`stochcraft-random`) and reports 95% confidence intervals (`stochcraft-stats`). `stochcraft-blocks` compiles models of sources, servers, delays and sinks and reports per-element statistics; `stochcraft-io` reads and writes them as BPSim on BPMN or XPDL, the open standards other process tools use; `stochcraft-engine` makes every change and run a command with a JSON schema, undo and redo, and a replayable journal; `stochcraft-cli` runs models from the command line, and the app runs them in a window on the desktop or [in the browser](https://helenkwok.github.io/StochCraft/). Checked against M/M/1 and M/M/2 (Erlang C) queueing theory; about 54× faster than SimPy on an M/M/1 benchmark ([bench/](bench/README.md)).

## Building

```bash
cargo xtask ci
```

## The app

Try it in the browser at [helenkwok.github.io/StochCraft](https://helenkwok.github.io/StochCraft/)
(every pull request also gets a preview, linked from the pull request), or run it on the
desktop:

```bash
cargo run --release -p stochcraft
```

Open a BPMN or XPDL model with BPSim data (or one of the examples under File ▸ Open Example), or
build one from blocks with **+ Add block**. Select a block to edit its parameters and flows,
run with F5 and read the results with their confidence intervals, as charts and a table; export
the table as CSV and the charts as PNG. Every edit can be undone.

To build the browser app locally, install [trunk](https://trunkrs.dev) and run it from
`apps/stochcraft-web`:

```bash
trunk serve --release
```

In the browser, runs use a web worker per core, and the app can be installed and used offline.

## Using the command line

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

```bash
stochcraft-cli run queue.bpmn --reps 20
```

`run` prints each element's statistics with the half-width of a 95% confidence interval, under a
run record (version, model digest, scenario, seed, replications, warm-up and duration) that is
enough to reproduce the numbers. `--format json` or `--format csv` gives machine-readable
results, `--out` writes them to a file, and `--seed`, `--warmup` and `--duration` override the
model's scenario for that run. Any BPMN or XPDL file with BPSim data can be run the same way.

Changes can be scripted as engine commands (`stochcraft-cli schema` lists them with their
parameters) and replayed:

```bash
stochcraft-cli exec changes.json --open queue.bpmn
```

## Using it from an AI agent

`stochcraft-cli mcp` serves the engine's commands over the Model Context Protocol, so an agent
such as Claude Code can open, build, run and compare models:

```bash
claude mcp add stochcraft -- /path/to/stochcraft-cli mcp
```

Start the app with `stochcraft --control 7070` and add `--connect 127.0.0.1:7070` to watch the
agent work in the window. [docs/mcp.md](docs/mcp.md) has the tools and options.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for the clean-room and licence rules.

## Licence

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT licence ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option. Third-party code and assets keep their documented licences; see
[NOTICE](NOTICE) and [ATTRIBUTION.md](ATTRIBUTION.md).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
StochCraft by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without
any additional terms or conditions.

StochCraft is independent and not affiliated with any simulation or CAD vendor; product names
are trademarks of their owners (see [NOTICE](NOTICE)).
