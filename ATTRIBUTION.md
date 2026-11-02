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
| .github/workflows/ci.yml | The plan job (base selection and `cargo xtask affected`) follows FieldCraft `.github/workflows/ci.yml` (same commit). | MIT OR Apache-2.0 |

## Assets

| File | Author | Source | Licence |
|---|---|---|---|
| _none yet_ | | | |
