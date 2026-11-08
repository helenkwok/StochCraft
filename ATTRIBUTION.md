# Attribution

Everything in this repository that StochCraft contributors did not write is listed here with its
source and licence.

**Policy (mandatory):** StochCraft contains no code, icons, images, 3D models, fonts, templates,
example models or help text taken from AnyLogic, FlexSim, Autodesk or any other proprietary
simulation or CAD product. Code adapted from open-source projects is allowed only from projects
licensed compatibly with MIT OR Apache-2.0 (for example MIT, Apache-2.0, BSD, Zlib), and every
such file is listed below. GPL, LGPL, AGPL and similar sources are never read or copied.

## Adapted code

| File in StochCraft | Adapted from | Licence |
|---|---|---|
| xtask/src/layers.rs | FieldCraft `xtask/src/layers.rs` (https://github.com/tangency-labs/fieldcraft, commit 6d9d17ea), itself adapted from SolidCraft's xtask. Copyright (c) 2026 FieldCraft contributors. FieldCraft's SolidCraft-gateway rule removed; StochCraft's layer table and UI crate list. | MIT OR Apache-2.0 |
| xtask/src/affected.rs | FieldCraft `xtask/src/affected.rs` (same commit). Copyright (c) 2026 FieldCraft contributors. The solver/oracle flag replaced by `heavy`; `deny.toml` added to the run-everything paths. | MIT OR Apache-2.0 |
| xtask/src/main.rs | Command layout and the `root`, `cargo`, `metadata` helpers follow FieldCraft `xtask/src/main.rs` (same commit). | MIT OR Apache-2.0 |
| bench/src/main.rs, bench/simpy/mm1.py | The `store` workload (arrival process feeding a FIFO store, one server process, λ = 0.9, μ = 1, 1,000,000 customers) follows the model of Cimba's `benchmark/MM1_single.c` and `MM1_single.py` (https://github.com/ambonvik/cimba), so results are comparable; the code is written for StochCraft and SimPy. Copyright (c) Asbjørn M. Bonvik. | Apache-2.0 |
| .github/workflows/ci.yml | The plan job (base selection and `cargo xtask affected`) follows FieldCraft `.github/workflows/ci.yml` (same commit). | MIT OR Apache-2.0 |
| apps/stochcraft-web/Trunk.toml, apps/stochcraft-web/index.html | The layout of CADCraft's `apps/cadcraft-web` (https://github.com/storytold/cadcraft, commit 5caaf73e): Trunk settings, a full-page canvas and a loading message. Copyright (c) 2026 ArtCraft Team and the CADCraft contributors. | MIT OR Apache-2.0 |
| .github/workflows/pages.yml | Installing a pinned trunk release and building with `trunk build --release --public-url ./` follow SolidCraft's `.github/workflows/pages.yml` (https://github.com/tangency-labs/solidCraft, commit 653bc00b). Copyright (c) 2026 SolidCraft contributors. Publishing to a `gh-pages` branch with pull-request previews is StochCraft's. | MIT OR Apache-2.0 |
| .github/workflows/pages-deploy.yml | The build and deploy jobs follow the usage example in actions/deploy-pages' README (https://github.com/actions/deploy-pages, v5). Copyright (c) 2021 GitHub, Inc. | MIT |
| crates/ui-egui/build.rs (translations), crates/ui-egui/src/i18n.rs | The build option (`CRAFT_I18N_DIR`, `CRAFT_I18N_REQUIRED`, one `include_str!` entry per locale with the app's own catalog) follows craft-i18n's `docs/integration.md` (https://github.com/storytold/craft-i18n, commit 7dfcc8e), and the runtime wiring (`I18n::new` with the offered locales, a per-thread language) follows craft-libs' `crates/i18n/README.md` (commit d836bea). Copyright (c) 2026 Craft suite contributors. No catalog is copied into StochCraft. | MIT OR Apache-2.0 |
| crates/mcp/src/server.rs, crates/mcp/src/tools.rs, crates/mcp/src/backend.rs, crates/mcp/src/lib.rs, docs/mcp.md | FieldCraft's `crates/mcp` and `docs/mcp.md` (https://github.com/tangency-labs/fieldcraft, commit 46c3ebf): the JSON-RPC framing, line and batch limits and lifecycle (`server.rs`, close to FieldCraft's), the tool definitions' shape, argument checking and the `batch` tool (`tools.rs`), the headless and connected backends (`backend.rs`), the `--connect` fallback (`lib.rs`) and the layout of the documentation. Copyright (c) 2026 FieldCraft contributors. The tools are StochCraft's own (open, run, check_model…; FieldCraft's meshing, solving and picture tools are not used), as is the control channel. | MIT OR Apache-2.0 |
| crates/mcp/src/roots.rs | FieldCraft `crates/io/src/roots.rs` (same commit): read and write roots, links followed, whole-folder comparison. Copyright (c) 2026 FieldCraft contributors. Errors as plain messages; no "writes within" mode. | MIT OR Apache-2.0 |
| crates/model/src/calendar.rs (`days_from_civil`, `civil_from_days`) | The day-count algorithms of Howard Hinnant's "chrono-Compatible Low-Level Date Algorithms" (https://howardhinnant.github.io/date_algorithms.html), in Rust for StochCraft. | Public domain (as their author dedicates them) |
| i18n/*.tsv | The translations of general interface words (File, Edit, View, Save As…, Undo, Cancel…) and each language's name are taken from craft-i18n's shared catalogs, `locales/<locale>/common.tsv` (https://github.com/storytold/craft-i18n, commit 7dfcc8e), so StochCraft reads like the other Craft apps; the other rows are StochCraft's own (i18n/README.md). Copyright (c) 2026 craft-i18n contributors. | MIT OR Apache-2.0 |

The interface's design (crates/ui-egui) follows the guidance of Impeccable's design skill for
application screens ("Operate" mode, the craft floor, layout, typography and colour references;
https://github.com/pbakaus/impeccable, commit 695cea5, Apache-2.0). No code or text from it is
included.

## Assets

| File | Author | Source | Licence |
|---|---|---|---|
| assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf | IBM Corp. (IBM Plex Sans, variable wdth and wght) | https://github.com/IBM/plex (via google/fonts), unmodified, as SolidCraft ships it; licence text in assets/fonts/ibm-plex-sans/OFL.txt; the interface font | SIL OFL 1.1 |
| assets/fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf | IBM Corp. (IBM Plex Mono) | https://github.com/IBM/plex (via google/fonts), unmodified; licence text in assets/fonts/ibm-plex-mono/OFL.txt; numbers in results tables | SIL OFL 1.1 |
| assets/fonts/ibm-plex-sans/OFL.txt, assets/fonts/ibm-plex-mono/OFL.txt | IBM Corp. | Unmodified accompanying font licences | SIL OFL 1.1 |
| The web app's `fonts/` (not in this repository): BIZ UDPGothic Regular and Bold; Noto Sans CJK SC Regular; NanumGothic Regular; Noto Sans Arabic, Thai, Georgian, Hebrew, Devanagari, Symbols and Symbols 2 Regular | The BIZ UDGothic Project Authors (BIZ UDPGothic); the Noto CJK project, https://github.com/notofonts/noto-cjk (Noto Sans CJK SC; its OFL.txt names no copyright holder); NHN Corporation (NanumGothic, with the Reserved Font Names in its OFL.txt, served unmodified); the Noto Project Authors (the other Noto faces) | Copied unmodified at build time from craft-fonts (https://github.com/storytold/craft-fonts, commit aced413, pinned in .github/workflows/pages.yml), whose `fonts/manifest.txt` gives each file's upstream source and SHA-256; each face's OFL.txt is published beside it. Served beside the browser app and fetched when the interface needs those scripts (crates/ui-egui/build.rs). The desktop app embeds the same faces except the CJK ones when built with `CRAFT_FONTS_DIR` | SIL OFL 1.1 |
