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
- The rows for ISO 22400-2's OEE elements (PR "OEE availability, effectiveness and setup ratio":
  Planned run time per item, OEE availability, OEE effectiveness, Setup ratio and the hint to set
  a planned run time) were machine-drafted with Claude Code from the plain meaning of the English
  and each catalog's existing terms: the part word of "Energy per part" for "item", the setup word
  of "Setup", and the share or ratio word. "OEE" is kept as the acronym in every language, although
  some Wikipedia articles name a local one (fr TRS, sv TAK, fi KNL, hu TEH, de GAE, cs CEZ, es
  ETE). "OEE availability" uses the availability factor named by the interlanguage links of
  Wikipedia's "Overall equipment effectiveness" where there is one (de Verfügbarkeit, es
  Disponibilidad, fr disponibilité, it disponibilità, nl beschikbaarheid, pl Dostępność, pt
  Disponibilidade, sv Tillgänglighet, fi käytettävyys, hu rendelkezésre állás, cs dostupnost, ru
  Доступность, ja 可動率, ko 가용성, zh-hant 稼動率); where a catalog's "Availability" is a
  reliability ratio and no article exists (uk, sk, lt, et, th, lo) it is the everyday word for
  available, and zh-hans uses 时间开动率 from general mainland TPM usage rather than the
  article's 稼动率. "OEE effectiveness" renders ISO's "effectiveness" literally (Effektivität,
  efficacité, 有効性, результативность…), not the "performance" factor the Wikipedia articles name
  (Leistung, prestatie, performance/desempenho, 性能, производительность, Anläggningsutnyttjande,
  nopeus, teljesítmény, 產能效率), so the row and the hint use the same word as the English. Not
  checked against IATE or the standard's own translations, and not reviewed by native speakers.
  The least certain are lo, th, ta, te, ka, kk, bn, sq, et, lt and zh-hans.
- The rows for ISO 22400-2's quality elements (Quality, Scrap, Rework, Counts against, the
  last-task default, Rework on {id}, their two hints, Quality ratio, Scrap ratio, Rework ratio, OEE
  and the note that OEE counts quality as 100%) were machine-drafted with Claude Code from the
  plain meaning of the English and each catalog's existing terms: the words for Departures, task
  with a resource, flow, part, block and the share or ratio word, and the previous bullet's OEE
  wording. "OEE" stays the acronym. The scrap and rework words come from the quality sections of
  Wikipedia's "Overall equipment effectiveness" interlanguage articles where they name them (de
  Ausschuss and Nacharbeit, nl afkeur and herbewerking, hu selejt and újramunkálás, es desechadas
  and retrabajadas, pt retrabalho) and elsewhere from the language's general manufacturing words
  (fr rebut and retouche, it scarto and rilavorazione, sv kassation, cs zmetky, pl braki, ru and
  uk брак); Wikipedia's "Scrap" and "Rework" articles are about recycling and electronics, so they
  were not used. "Quality ratio" follows the quality factor those articles name where it is a rate
  (de Qualitätsrate, fr Taux de qualité, it Tasso di qualità), so de, fr and it use a rate word
  for all three ratios rather than the "Setup ratio" word; ru, uk and bg say "share of good
  parts" (Доля годных, Частка придатних, Дял на годните) and ja, ko, zh-hans and zh-hant the
  good-part rate (良品率, 양품률, 合格率, 良率), which is the ratio's definition rather than a
  literal "quality". Not checked against IATE or the standard's own translations, and not
  reviewed by native speakers. The least certain are lo, th, ta, te, ka, kk, bn, sq, et and lt as
  before, plus ja 廃棄 and ko 폐기 (which can read as the act of disposal rather than the scrapped
  item), sq Skarto (an Italian loan), hi's "Counts against" (phrased as a question), and pl,
  whose "Udział jakości" keeps the share word although Polish usage tends to say "wskaźnik".
- The rows for takt (PR "takt from a customer demand on departures": the Takt section of a
  Departures block, its demand, period and time-available pickers, the per-shift hint, and the
  Takt time, Demand, Demand met and Cycle time results) were machine-drafted with Claude Code from
  the plain meaning of the English and each catalog's existing words for Departures, task, part,
  run, shift, period, day, week and share. "Takt time" follows the titles of the interlanguage
  articles of Wikipedia's "Takt time" where there is one (ca Temps de takt, de Taktzeit, es and fr
  Takt time, hi टैक्ट समय, ja タクトタイム, nb Takttid, pt Takt Time, tr Takt zamanı), the Wikidata
  labels of that item for zh (节拍时间, and 節拍時間 for zh-hant) and it (takt time), and the
  wording inside related articles for fi (tahtiaika, "Tahtituotanto"), hu (ütemidő, "Akadálytalan
  áramlás"), ko (택트 타임, "공정관리") and vi (thời gian Takt, "Công nghệ dòng nhu cầu");
  everywhere else it is the language's "takt" plus its time word from general lean usage, with no
  source checked (ru Время такта, pl Czas taktu, sv Takttid…), and nl Takttijd and sr Такт време
  keep that form although Wikidata's labels are "Takt time" and "takt tim". English Wikipedia's
  "Cycle time" is the software sense, so its links were not used: "Cycle time" follows the takt
  articles' own words for the manufacturing cycle where they give one (fr temps de cycle, ca temps
  de cicle, nb syklustid, tr döngü süresi, hi चक्र समय from "चक्रिय समय", ja サイクルタイム from
  the article "サイクル・タイム", de Zykluszeit as in German lean usage) and elsewhere is the
  language's cycle-plus-time compound. The bare "Takt" heading is the loanword where the language
  uses one, but ja, ko, lo and th repeat the full takt-time term because the bare loan reads as
  something else (ja タクト is a conductor's baton), fi and hu use their own words (Tahti, Ütem),
  and zh uses 节拍/節拍. "Per" is rendered as the catalog's period word (Zeitraum, Période, 期間,
  Jakso…) because the choices already carry the preposition. "The whole clock" says "all the time"
  rather than the metaphor. "Demand met" is the share of the demand met with the catalog's share
  word, or a fill rate in ja, ko and zh (需要充足率, 수요 충족률, 需求满足率, 需求達成率), and de
  Bedarfsdeckung. ko uses the catalog's 근무조 for shift, but nl says "ploeg" rather than its
  catalog's "werktijd", which does not take "per". Not checked against IATE, and not reviewed by
  native speakers. The least certain are lo and th (whose takt words ເວລາແທັກ and แท็กไทม์ are
  unsourced transliterations), ta, te, bn, ka, kk and sq as before, plus et, lt and the sr and
  sr-latn rows (written in ekavian although those catalogs mix in ijekavian forms).
- The rows for patience (PR "entities that give up waiting": the Patience section of a task, its
  two choices and their hints, the Abandoned mark on a flow, and the Gave up, Share that gave up,
  Mean wait before giving up, Work wasted on those who gave up and Share of work wasted results)
  were machine-drafted with Claude Code from the plain meaning of the English and each catalog's
  existing words for entity, unit, flow, task, service, wait, share, mean, work, way out and model,
  taken from its rows for the Rework mark, "Each entity takes the first…", "Mean wait" and "Share of
  arrivals lost". Where a catalog already tells a unit from an entity by a longer word (da and nb
  ressourceenhed, hi संसाधन इकाई, ka რესურსის ერთეული, ko 자원 단위, zh 资源单元/資源單元) the
  hint "once a unit takes it, the entity stays" uses that word, and pl, whose entity is
  "jednostka", says "zasób" as its catalog does. "Abandoned on {id}" follows each catalog's
  "Rework on {id}", and the hint naming "the way out marked Abandoned" quotes the same word as the
  mark. Most catalogs use one verb for giving up throughout; id, lt, ru, uk and te say "stop
  waiting" for the checkbox and a leave or withdraw verb for the rest. The queueing term is reneging (in call
  centres, abandonment); the sources checked were Wikipedia's articles on queueing theory, call
  centres, Erlang C and timeouts in their interlanguage versions, which give little: pt
  "desistir e sair do sistema" (hence pt-br desistir, Desistências), ro "părăsesc coada" for
  reneging, es "impaciencia del cliente", el "χαμένων κλήσεων (abandoned calls)", de Erlang C's
  callers who "auflegen" after a while, and a ko article's 포기호 (전화 포기), hence ko 포기. The
  timeout articles' titles give cs časová prodleva, ja タイムアウト, ko 타임아웃, ru Тайм-аут, tr
  zaman aşımı and uk тайм-аут; elsewhere "times out" is the language's plain words for its time
  running out, with "timeout" in brackets where that helps. Everything else is the plain meaning:
  es, ca, gl, it and fr abandon (fr Abandons for the mark), de aufgeben with the mark Abgebrochen,
  nl afhaken, ru and uk "leave" (Ушедшие, Пішли), cs and sk vzdát, pl rezygnować, bs, hr and sr
  odustati, sl odnehati, hu lemond (not "felad", which is also "send" next to the catalog's
  feladat for task), fi luopua, et loobuma, lt pasitraukti, sv, da and nb "give up", tr vazgeçmek,
  zh 放弃/放棄, ja 離脱. The heading "Patience" is the language's word for patience, except ja
  途中離脱, ko 대기 포기, th การเลิกรอ and lo ການເຊົາລໍຖ້າ, where the bare word reads as a virtue
  rather than a setting. "Time before giving up" is reached through a field label in form.rs that
  the catalog test does not scan, so the test does not check its row. Not checked against IATE,
  and not reviewed by native speakers. The least certain are lo, th, ta, te, bn, ka, kk and sq as
  before, plus hu lemond, id menyerah, vi bỏ đi and hi छोड़कर गईं, and the marks that are verb
  forms rather than nouns (da Gav op, nb Ga opp, sv Gav upp, cs and sk Vzdané, uk Пішли), and sr
  and sr-latn, written in ekavian but keeping the catalogs' "Prosječno čekanje" for the mean wait.

**Adding or changing a string:** add the English in the code through `i18n::t` (or `tf`, `tn`),
then add its row to each catalog here, or leave it out to show English until it is translated.
Right-to-left languages are not offered until the layout is mirrored.
