# StochCraft's translations

One catalog per language, `<locale>.tsv`, in craft-i18n's catalog format
(https://github.com/storytold/craft-i18n, `docs/catalog-format.md`): `context TAB English TAB
translation`, `@plural` rows with the language's CLDR plural forms, `{name}` placeholders kept, and
the language's own name in an `@id language.name` row. `crates/ui-egui/build.rs` embeds every file
here into the desktop and browser apps; View ▸ Language lists them.

Each catalog is complete: every string the window shows (`crates/ui-egui/tests/i18n.rs` checks
that every string goes through translation, and `i18n::tests::catalogs_are_clean` that every row
parses). A missing row shows in English.

**Where they come from:**
- Written with ChatGPT from the plain meaning of the English, then cleaned with Claude Code.
- Not reviewed by native speakers.
- German and both Chinese catalogs had their simulation terminology checked against published
  sources: Einschwingphase, Verweilzeit, Skalen- and Formparameter; Taiwan usage for `zh-hant`.
- Rows for general interface words (File, Edit, Save As…, Undo…) take the wording of craft-i18n's
  shared `common.tsv` for that language, so StochCraft reads like the other Craft apps (see
  ATTRIBUTION.md).

**Adding or changing a string:** add the English in the code through `i18n::t` (or `tf`, `tn`),
then add its row to each catalog here, or leave it out to show English until it is translated.
Right-to-left languages are not offered until the layout is mirrored.
