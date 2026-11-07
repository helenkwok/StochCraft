# StochCraft's translations

One catalog per language, `<locale>.tsv`, in craft-i18n's catalog format
(https://github.com/storytold/craft-i18n, `docs/catalog-format.md`): `context TAB English TAB
translation`, `@plural` rows with the language's CLDR plural forms, `{name}` placeholders kept, and
the language's own name in an `@id language.name` row. `crates/ui-egui/build.rs` embeds every file
here into the desktop and browser apps; View ▸ Language lists them.

Each catalog has a row for every string StochCraft's window, menus and desktop shell look up.
`crates/ui-egui/tests/i18n.rs` checks that every label is looked up (pseudo locale) and that every
string passed to `t`, `tf`, `tc`, `tn` or `tnf`, or looked up while the test draws the screens,
has a row in each catalog here; `i18n::tests::catalogs_are_clean` checks that every row parses.
A missing row shows in English.

Not covered here: the keyboard-shortcuts dialog draws its own words (craft-keymap's
`ui::STRINGS`), which belong in craft-i18n's shared `common.tsv`. That catalog is embedded only
when `CRAFT_I18N_DIR` is set at build time (not in CI or the published web app), and upstream it
has only some of those words for some languages, so the dialog is mostly English for now. The
engine's messages (validation, file notes) are English too.

**Where they come from:**
- Written with ChatGPT from the plain meaning of the English, then cleaned with Claude Code.
- Not reviewed by native speakers.
- German and both Chinese catalogs had their simulation terminology checked against published
  sources: Einschwingphase, Verweilzeit, Skalen- and Formparameter; Taiwan usage for `zh-hant`.
- Rows for general interface words (File, Edit, Save As…, Undo…, New, Open…, Light, Dark,
  Keyboard Shortcuts…, the macOS application menu) take the wording of craft-i18n's shared
  `common.tsv` for that language where it has one in the same sense, so StochCraft reads like
  the other Craft apps (see ATTRIBUTION.md). Where a catalog already had its own term, that term
  wins (Keyboard Shortcuts… in German, Spanish and Traditional Chinese); "Units" (a quantity) and
  "Zoom" (the window-size toggle) have their own wording where `common.tsv` means something else.
  Slovak has no `common.tsv`; its rows are StochCraft's own.
- The rows added for issue #39 (shared resource pools, the macOS and Windows menu words, the
  desktop file dialogs, time-unit abbreviations) were written with Claude Code from the plain
  meaning of the English, the existing terms of each catalog and `common.tsv`; uncertain domain
  terms (resource pool, needs, by role, priority…) were checked against dictionaries, Wikipedia,
  the EU's IATE term base and simulation literature, never against another product's
  translations.

**Adding or changing a string:** add the English in the code through `i18n::t` (or `tf`, `tn`),
then add its row to each catalog here, or leave it out to show English until it is translated.
Right-to-left languages are not offered until the layout is mirrored.
