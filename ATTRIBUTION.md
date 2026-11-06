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
