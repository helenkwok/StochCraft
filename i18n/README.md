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
- The row for "Still inside at the end" (issue #68, entities still inside an element when a
  replication ends) was written with Claude Code from the plain meaning of the English and each
  catalog's own word for "inside" in "Mean number inside"; not checked against any other source.
- The rows added for issue #69 (how many replications a result is over, what the means leave
  out, sizing's target share and its 95% interval, the one-replication note in Watch a run) were
  machine-drafted with Claude Code from the plain meaning of the English, reusing each catalog's
  own words for replication, target, share, mean wait, mean time in system, stop condition, time
  limit and 95% interval; not checked against any other source, and not reviewed by native
  speakers. "Met in at least" and "Share of replications" were removed with the sizing field they
  labelled (now "Target share").
- The row for "Time off shift" (issue #88, the share of time a server's units are off shift)
  was machine-drafted from the plain meaning of the English, with the language's everyday word
  for a work shift; not checked against any other source, and not reviewed by native speakers.
- The rows added for issue #90 (energy, energy per part, and the time a server's units spend
  processing, starved, blocked and failed) were machine-drafted the same way, with the "Time …"
  rows phrased alike in each language; "starved" is rendered as time without work (waiting for
  parts). Not checked against any other source, and not reviewed by native speakers.
- The row for "Mean work in progress" (issue #87, the time-average number of parts inside an
  element) was machine-drafted with Claude Code using each language's term for work in process.
  The term comes from Wikipedia's interlanguage links where there was one (bn, ca, de, ja, nl,
  pl), and otherwise from general accounting and operations-management usage. It was not checked
  against IATE, and not reviewed by native speakers. The least certain are lo, ta, te, hi, ka,
  kk, sq and it.
- The rows for the breakdown, transfer and calendar fields and the pool and calendar undo names
  (PR "forms for breakdowns, transfer and calendars") were machine-drafted with Claude Code from
  the plain meaning of the English and each catalog's existing terms (pool, shift, failed, units,
  block): "Breakdowns" and "Time to failure" use the word of that catalog's "Time failed", and the
  undo names follow its "Change Block", "Add Block" and "Remove Block". Where a catalog's
  "Availability" is a reliability ratio (ja, ko, ru, uk, zh-hans, zh-hant, th, lo, cs, sk, lt,
  et), the calendar rows use the everyday word for being available instead. Serbian rows are
  Ekavian. Not checked against dictionaries, Wikipedia, IATE or any other source, and not
  reviewed by native speakers. The least certain are lo, th, ta, te, ka, kk, bn, sq and
  zh-hant ("行事曆" for calendar).
- The rows for the Calendars section of the model page (PR "Calendars section on the model
  page": periods, their repeats, model time 0) were machine-drafted with Claude Code the same
  way, from the plain meaning of the English and each catalog's existing terms, including the
  calendar, pool, shift, availability and arrivals words of the rows above. The weekday
  abbreviations are each language's usual short names, and "On" (before the weekday buttons) is
  rendered as "Days". Not checked against dictionaries, Wikipedia, IATE or any other source,
  and not reviewed by native speakers. The least certain are lo, th, ta, te, ka, kk, bn and sq.
- The rows for a property drawn as text values by shares (PR "product mix: a property drawn as
  text values by shares": Random, Mix, Share, Add value) were machine-drafted with Claude Code
  from the plain meaning of the English, reusing each catalog's word for random (from the
  textbook-queue example) and for share (from "Target share"). Not checked against any other
  source, and not reviewed by native speakers; "Mix" is the least certain.
- The rows for setups and changeovers (PR "setups and changeovers on a task with a resource")
  were machine-drafted with Claude Code from the plain meaning of the English and each
  catalog's existing terms, with "Time setting up" phrased like the other "Time …" rows. The
  manufacturing term for a setup follows Wikipedia's interlanguage links for "Changeover" and
  "Single-minute exchange of die" where they name one (nb Omstilling, sv Ställtid, ja 段取り,
  ru переналадка, zh 换模/换型); elsewhere it is the language's general word for machine set-up
  or preparation. Not checked against IATE, and not reviewed by native speakers. The least
  certain are lo, th, ta, te, ka, kk, bn, sq, ko and vi.

**Adding or changing a string:** add the English in the code through `i18n::t` (or `tf`, `tn`),
then add its row to each catalog here, or leave it out to show English until it is translated.
Right-to-left languages are not offered until the layout is mirrored.
