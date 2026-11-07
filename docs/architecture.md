# Architecture: crate layers

Every workspace crate sits on a layer and may depend only on crates in strictly lower layers
(`cargo xtask layers` enforces this; the table lives in `xtask/src/layers.rs`). Keeping the
graph shallow is what keeps CI fast: `cargo xtask affected` retests a changed crate and the
crates above it, so a change near the top touches little and a change near the bottom is rare.

| Layer | Crates (`stochcraft-*`) | Contents |
|---|---|---|
| L0 | `random`, `stats` (standalone: no workspace dependencies) | Random-number generators and BPSim's distributions, statistics collectors |
| L1 | `des`, `model` | The discrete-event core (event queue, processes, resources, timeouts); the model in memory (elements with stable ids, connections, experiment, validation) |
| L2 | `blocks`, `agents`, `sd` | Modelling libraries: flow blocks (source, queue, delay, conveyor…), moving agents, stock-and-flow |
| L3 | `experiment`, `io` | Replications, parameter sweeps and optimisation; model and result files |
| L4 | `engine` | Commands, undo and redo, the journal and runs: the one way the CLI, the app and agents change or run a model |
| L5 | `render`, then `ui-egui` and `mcp` | Drawing, the window, and agents over MCP. The only layer allowed UI, windowing or GPU crates |
| — | `testkit` | Test tooling; other crates may use it only as a dev-dependency |
| apps | `stochcraft`, `stochcraft-cli`, `stochcraft-web` | Exempt from the rules |

Only `random`, `stats`, `des`, `model`, `blocks`, `experiment`, `io`, `engine`, `ui-egui` and `testkit` exist so far (with the three apps); the others are placed in advance so each crate's position is decided
before it is written.

## The app on craft-libs

The interface (`ui-egui`, the `stochcraft` and `stochcraft-web` apps) is built on
[craft-libs](https://github.com/storytold/craft-libs), the Craft suite's shared app foundation,
pinned to one commit in the workspace manifest with the egui fork it uses: craft-menu and
craft-keymap (menus and shortcuts), craft-settings, craft-window, craft-quit, craft-atomic,
craft-dialogs, craft-platform, craft-ui and craft-i18n. The interface owns only what is
StochCraft's: its commands, its document and its look.

### The model view

The Model tab is the flowchart canvas (`ui-egui/src/canvas.rs`) with the inspector beside it
(`editor.rs`, `form.rs`, `pools.rs`). Both edit only through engine commands, so every change is
one undo step and an agent can do the same. A gesture that makes several changes (paste, delete a
selection, align) sends them as one `batch`. Shape positions are part of the model
(`Model.layout`), read from and saved to the file's BPMN DI or XPDL graphics. The canvas keeps
only its view (pan, zoom), the selection and the clipboard.

### Watching a run

Model ▸ Watch a Run (`ui-egui/src/live.rs`) steps replication 1 on the UI thread, a slice per
frame, through the engine's `live_run`. It uses the same `blocks` runtime as Run, split so that
one replication can stop at any time and be read. A plain run is that split run taken straight
to its end, so the two cannot drift apart. Watching reads the model and never edits it.

### Translations

Every interface string is English source text looked up through craft-i18n (`ui-egui/src/i18n.rs`:
`t`, `tf` for `{name}` placeholders, `tn`/`tnf` for counts). StochCraft keeps its catalogs in this
repository, `i18n/<locale>.tsv` (i18n/README.md), in craft-i18n's format, and `ui-egui/build.rs`
always embeds them, on the web too: 44 languages. With `CRAFT_I18N_DIR` naming a checkout of the
craft-i18n data repository, each language's shared `common.tsv` is embedded beside its catalog,
for the words craft-libs' own dialogs use (the keyboard-shortcuts dialog); StochCraft's rows
win where both have one. CI and the web app are built without it, so every word StochCraft's own
window and menus show has a row in StochCraft's catalogs. Right-to-left locales
are not offered until the layout is mirrored. Numbers follow the region, not the language.

`tests/i18n.rs` draws every screen in craft-i18n's pseudo locale, which marks each looked-up
string, and fails on any label drawn without a lookup that is not data (the model's names and
ids, numbers, file names, shortcuts), and checks that every string passed to `t`, `tf`, `tc`,
`tn` and `tnf` in the interface and the shells, and every string looked up while the screens are
drawn, has a row in each catalog. The engine's own messages (validation, file notes) are not
translated yet.

## Determinism across targets

Every layered crate builds for `wasm32-unknown-unknown` (`cargo xtask wasm`), and a later wasm64
build will follow. A seeded model must give the same results on native, wasm32 and wasm64, and
on every operating system:

- Anything saved, sent, hashed or used to seed random numbers uses fixed-width integers, never
  `usize`.
- Logarithms, exponentials, powers and trigonometric functions come from
  `stochcraft_random::math` (the pure-Rust `libm`), never from `f64`'s methods, which call each
  platform's maths library and can differ in the last bit between glibc, musl (wasm), macOS and
  Windows. `clippy.toml` forbids the `f64` methods. Square roots are exactly rounded everywhere.
- Replications run in parallel but their results are combined in replication order.

`stochcraft-testkit` checks it: the golden models (`tests/models/`) are run through the engine
and their results reduced to digests pinned in `tests/golden/results.txt`. The test runs natively
in `cargo xtask ci` and as `wasm32-wasip1` under Node's built-in WASI in `cargo xtask golden`
(both in CI). Switching to `libm` changed the last bits of some samples, which the golden test
caught; that is the kind of drift it exists to catch.
