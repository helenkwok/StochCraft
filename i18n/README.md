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
  Mean wait before giving up, Work wasted on those who gave up and Share of work wasted results;
  Work wasted on those who gave up was later renamed Work wasted on jobs that did not finish, see
  the health-check bullet) were machine-drafted with Claude Code from the plain meaning of the
  English and each catalog's existing words for entity, unit, flow, task, service, wait, share,
  mean, work, way out and model, taken from its rows for the Rework mark, "Each entity takes the
  first…", "Mean wait" and "Share of arrivals lost". Where a catalog already tells a unit from an
  entity by a longer word (da and nb ressourceenhed, hi संसाधन इकाई, ka რესურსის ერთეული, ko 자원
  단위, zh 资源单元/資源單元) the hint "once a unit takes it, the entity stays" uses that word, and
  pl, whose entity is "jednostka", says "zasób" as its catalog does.
  "Abandoned on {id}" follows each catalog's
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
- The rows for colour themes (PR "colour themes": Edit themes…, the Themes window and its hint,
  In use, Rename, Save name, the name check, the contrast check and its warning "{text} on
  {background}: {ratio}:1, needs {min}:1", the built-in themes High contrast, Warm and StochCraft,
  and the palette roles Header, Side rail, Canvas, Raised surfaces, Weak text, Borders, Table
  stripes, Accent, Soft accent, Chart hover and the Text on… roles) were machine-drafted with
  Claude Code from the plain meaning of the English and each catalog's existing words, taken from
  its rows for Text, Light, Dark, Duplicate, Name, Edit, Rename Model and Download charts (PNG):
  the roles that name text contain that catalog's "Text" word, because the warning puts the
  existing "Text" row next to the new ones; Chart and Chart hover use the word of "Download
  charts"; Rename follows "Rename Model"; "Warm" agrees with the catalog's Light and Dark. The
  sources checked were Wikipedia's articles on theme (computing), colour scheme, contrast
  (vision), contrast ratio, sidebar (computing), header (computing) and the HTML canvas element
  in their interlanguage versions, and the W3C's published WCAG 2 translations (ca, da, de, es,
  et, fi, fr, it, ja, ko, nl, no, pl, pt-BR, ru, sk, sv, th, uk, zh-Hans) for success criterion
  1.4.3 "Contrast (Minimum)". They agree with the drafted words (tema/thème/teema, barra lateral,
  thanh bên, 侧边栏/側邊欄, th ความเปรียบต่าง, the WCAG "Kontrast (minimum)" family), except
  that ru and uk use "контраст" in WCAG, hence Высокий/Високий контраст and "minimum requirements
  for contrast" in the contrast check, and ko uses 명도 대비 there. de keeps Farbschemata (de
  Wikipedia's article is "Skin", a narrower thing), and lo uses ຊຸດສີ ("colour set") because
  ທີມ also means team; kk uses Тақырып for theme and calls the header "top bar" (Жоғарғы жолақ) so
  the two don't meet. No article exists for accent colour, high-contrast mode or zebra striping,
  and the header and canvas articles are about data headers and the HTML element, so Header,
  Canvas (mostly "main surface"; ja, ko and zh say canvas), Raised surfaces, Table stripes,
  Accent and Soft accent are the plain meaning only. In languages with cases the warning names
  the background after a dash (in effect "{text} – background ‘{background}’") so neither role name has to
  change form; the ratio keeps "." as the decimal point, as the code formats it, and
  "StochCraft" stays as it is in every language. Not checked against IATE, and not reviewed by
  native speakers. The least certain are lo, th, ta, te, kk, ka, bn and sq, and the calques for
  Raised surfaces, Side rail, Table stripes and Soft accent everywhere.
- The rows for health checks (PR "health check": the Health check section of a task, its checkbox
  Take servers out when overloaded and its hint, the fields Check every, Fails above, waiting per
  working server, Out for (restart) and Keep at least in service, the choice for a job under way
  on a server taken out, Retry elsewhere or Resume after restart, with their hints, and the Units
  taken out by health checks, Mean units in service, Time restarting and Work wasted on jobs that
  did not finish results) were machine-drafted with Claude Code from the plain meaning of the
  English and each catalog's existing words for unit, entity, client, job, queue, wait, mean and
  wasted work, taken from its rows for the patience hint "A client that times out…", "Units that
  several tasks share…", "Each entity takes the first…", "Mean queue length", "Mean number
  inside" and "Work wasted on those who gave up". That last row is gone from every catalog; its
  successor keeps the same words for wasted work. "Time restarting" is phrased like the catalog's
  "Time failed", and the longer unit words (da ressourceenhed, nb ressursenhet, hi संसाधन इकाई, ka
  რესურსის ერთეული, ko 자원 단위, zh 资源单元/資源單元, pl zasób) are kept. "In service" means in
  operation, not being served: de in Betrieb, nl in dienst, sv, da and nb i drift, cs v provozu,
  sk v prevádzke, ru в работе, uk у роботі, ja 稼働, zh-hans 在线, zh-hant 上線; es, ca, gl, pt-br,
  it, fr and tr keep their service word ("en servicio", "in servizio", "en service", "hizmet
  dışı"), as that is the idiom there. Most catalogs use one word for both work and job, so "Work
  wasted on jobs that did not finish" repeats it (es trabajo, it lavoro, cs and sk práce, pl
  praca, id pekerjaan, vi công việc, sq punë); bg, ru and uk say "unfinished service" and nl
  "unfinished bewerkingen" instead, and sl says opravila because "delih" would also read as
  "parts". No catalog had a word for server. The sources checked were Wikipedia's articles on
  load balancing (computing), server (computing), reboot, failover and downtime in their
  interlanguage versions, searched by keyword for the health check, and IATE for load balancing,
  load balancer, health check, server, reboot, restart, retry and out of service. IATE had
  nothing for load balancer or reboot, and for load balancing only fr équilibrage de la charge
  and nl load balancing. Server is the server article's title (bg сървър, el εξυπηρετητής, where
  IATE also has διακομιστής, fi palvelin, hr poslužitelj, sl strežnik, ta வழங்கி, tr sunucu, vi
  máy chủ, ja サーバ, zh 服务器, and server, serveur, servidor, serwer, сервер elsewhere), and IATE
  agrees for the EU languages; zh-hant says 伺服器 (Taiwan usage), id keeps "server" though its
  article says peladen, te says సర్వర్ though its article says పరిచారిక, sq, with no article, says
  server, and lo, with no article, says ແມ່ຂ່າຍ to pair with the catalog's ລູກຂ່າຍ for client. The
  keyword search found the health check named only in the de (Healthchecks), et
  (tervisekontroll), ru (Проверка работоспособности) and zh (健康检查) articles, and those catalogs
  use that term; sk's article keeps the English "health checking", ja's describes keepalive
  monitoring (監視) without a name for it, and IATE has health check only in medical and
  farm-policy senses, which were not used. Elsewhere the heading is the plain "check of the
  state" (Kontrola stavu, Comprobación de estado, Statuscontrole, 상태 검사, …), except the
  unsourced ja ヘルスチェック, uk Перевірка працездатності (ru's term; uk's article has none), and
  the calques fr contrôle de santé, sv hälsokontroll, da sundhedstjek and nb helsesjekk. The load
  balancer is the article's own noun where it has one: de, nl, bs and sk load balancer, fr
  répartiteur de charge, es balanceador de carga, ca distribuïdor de
  càrrega, cs vyvažovač zátěže, hu terheléselosztó, et koormusjaotur, tr yük dengeleyici, ru
  балансировщик, hi लोड बैलेंसर, ja ロードバランサ and zh 负载均衡器. pl system równoważenia
  obciążenia, gl and pt-br balanceador de carga, it bilanciatore di carico, fi kuormantasaaja, uk
  балансувальник навантаження, ko 부하 분산기, el εξισορροπητής φόρτου and nb lastbalanserer are
  formed from their articles' titles, whose text has no noun for the device; bg, bn, da, hr, id,
  ka, kk, lo, lt, ro, sl, sq, sr, sr-latn, sv, ta, te, th and vi have no load-balancing article,
  so their load balancer and health check were not checked against one. Restart follows IATE
  or the reboot article where either has it: de Neustart, fr redémarrage, it riavvio, nl herstart, el
  επανεκκίνηση, fi uudelleenkäynnistys, sl ponovni zagon, sv omstart, da genstart, pt-br
  reinício, ca reinici, ru перезагрузка, uk перезавантаження, ja 再起動, ko 재기동, vi khởi động
  lại, zh 重新启动/重新啟動. es and gl say reinicio where IATE gives rearranque and reanudación, id
  says mulai ulang for its article's "mulai-awal ulang", bn says পুনরায় চালু and cs restart
  where their articles' titles are রিবুট and Reboot, and ka says ხელახალი გაშვება because
  გადატვირთვა (reboot) also means "overloaded". IATE's retry gave es reintentar, el νέα
  προσπάθεια, nl opnieuw proberen and fi yrittää uudelleen, which are used, and its out of
  service gave de außer Betrieb, fr hors service, it fuori servizio, es fuera de servicio and
  retirar de servicio, pt fora de serviço and el εκτός λειτουργίας, also used; the other languages
  have the plain meaning. "Fails above, waiting per working server" is hard to say briefly: most
  catalogs put the "waiting per server in operation" part after a comma or in brackets, and bn,
  hi, hu, ja, kk, ko, ta, te, tr and zh begin with "failure threshold". The colon after "Job under
  way on a server taken out" follows each catalog's "Drag to add:" (fr with a space, zh
  full-width). Check every, Fails above…, Out for (restart) and Keep at least in service are
  field labels in form.rs, which the catalog test's source scan does not see and which none of
  the screens it draws shows (the section opens only for a task with a health check, and no
  example has one), so as far as can be told from reading the test, it does not check their rows.
  Not reviewed by native speakers. The least certain are lo, th, ta, te, bn, ka, kk and sq as
  before, the unsourced health-check words above, the load-balancer nouns formed from titles,
  the "Fails above…" label everywhere, and sr and sr-latn, written in ekavian but keeping the
  catalogs' Prosječan and Vrijeme in the rows that follow "Mean number inside" and "Time failed".
- The rows for chart palettes (PR "chart palettes": Chart colours… in View > Appearance, the Chart
  colours window and its hint, the headings Categorical, Sequential and Diverging, Use for charts,
  Add colour, Remove colour, the colour slots "Colour {n}", Checks, On the light canvas and On the
  dark canvas, All checks pass., the level Fail beside the existing Warning, "{word}: {message}",
  and the seven check messages: contrast with the chart background, a colour that reads as grey, the
  difference for normal colour vision and for red-green colour blindness, the lightness step and a
  ramp that turns back) were machine-drafted with Claude Code from the plain meaning of the English
  and each catalog's existing words, taken from its colour-themes rows: the hint is "Built-in themes
  are read-only. Duplicate one to change its colours." with the palette word in place of the theme
  word; chart and table come from Chart and Export table (CSV); the canvas rows reuse Canvas (mostly
  "main surface"; ja, ko and zh say canvas) with Light and Dark agreeing with it; contrast comes
  from "All text meets the contrast minimums.", and the "needs" word from "{text} on {background}:
  {ratio}:1, needs {min}:1" (fr minimum, hu legalább … kell, tr en az … gerekli), which also gives
  the background word in the catalogs that name the background there after a dash (elsewhere
  background is the plain meaning). In use, Duplicate, Rename, Delete, Name, the name check, Save
  name, Cancel and Warning are the existing rows, unchanged. The palette names (Okabe–Ito, Tol
  bright, ColorBrewer Dark2, Viridis, …) are proper names that the code shows without a lookup, so
  they are not translated. The sources checked were Wikipedia's articles on color blindness,
  congenital red–green color blindness, dichromacy, color vision, color scheme, contrast (vision),
  lightness, colorfulness, categorical variable, palette (computing) and grey in their interlanguage
  versions (the color-blindness and colorfulness articles searched by keyword for red-green and
  chroma), IATE for colour blindness, red-green colour blindness, colour vision deficiency, colour
  vision, lightness, chroma, contrast ratio and colour scheme, and English Wiktionary's translation
  tables for categorical, sequential, divergent, lightness, palette and color blindness. Red-green
  colour blindness is the article's own term where it has one: ca daltonisme vermell-verd, es
  daltonismo rojo-verde, fr daltonisme rouge-vert, it discromatopsia rosso-verde, da rød-grøn
  farveblindhed, nb rød-grønn fargeblindhet, nl rood-groenkleurenblindheid, pl ślepota
  czerwono-zielona, ru красно-зелёный дальтонизм, bs crveno-zelena sljepoća za boje, el αχρωματοψία
  στο κόκκινο-πράσινο, sq verbëri e ngjyrave të kuqe-të gjelbër, tr kırmızı-yeşil renk körlüğü, te
  ఎరుపు-ఆకుపచ్చ రంగు అంధత్వం, vi mù màu đỏ-lục, ko 적록 색각 이상, zh 红绿色盲/紅綠色盲, ja 赤緑 with the title
  色覚異常, bn লাল-সবুজ and hi लाल … वर्णान्धता; de Rot-Grün-Sehschwäche and hu vörös-zöld színtévesztés
  are the titles of their red-green articles, and sv joins its article's röd-grön to IATE's
  färgblindhet. Elsewhere (bg, cs, et, fi, gl, hr, id, ka, kk, lt, pt-br, ro, sl, sr, sr-latn, ta,
  th, uk) "red-green" is joined to the color-blindness article's title, and sk to Wiktionary's
  farbosleposť, as sk has no such article; lo has no article. IATE's one entry covers colour
  blindness and colour vision deficiency together, so the catalogs do not tell them apart. Normal
  colour vision uses the color-vision titles or IATE (de Farbsehen, fr vision des couleurs, es
  visión del color, nl kleurenzien, sv färgseende, da farvesans, fi väriaisti, et värvitaju, ru
  цветовое зрение, uk кольоровідчуття, sr vid u boji, ja 色覚; zh 色觉/色覺 is the plain meaning, as zh's
  title is 彩色视觉). Lightness follows IATE and Wiktionary's colour sense (fr clarté, de Helligkeit, nl
  lichtheid, sv ljushet, da lyshed, fi vaaleus, it chiarezza, el φωτεινότητα) or the lightness
  article (es luminosidad, pt-br luminosidade, et heledus, lt šviesumas, ru светлота, ja and zh 明度);
  uk світлота and ko 명도 are from the text of their colorfulness articles (ko's lightness article is
  titled 밝기), and bg светлота is the plain meaning (Wiktionary gives светлост). Chroma has its own
  word where a source gave one: fr chroma, es and ca croma, tr kroma, et kromaatilisus, id
  kemarakan, de Buntheit (a synonym in de's article), ja, ko and zh 彩度/채도, and from IATE ro cromă,
  el χρωματική ένταση, pl nasycenie and fi värikkyys (in its chrominance entry); gl and pt-br croma,
  it croma, hi क्रोमा and bn ক্রোমা follow es and ca without a source of their own; elsewhere it is
  the saturation word that the colorfulness article is titled with (ru and uk say that is chroma in
  CIELAB), so it may read as saturation. The en color-scheme article describes qualitative,
  sequential and diverging schemes, but none of its five other versions (es, hi, ja, ko, zh) does,
  so the three headings are dictionary words: Wiktionary's categorical (fr catégoriel, it
  categoriale, ro categorial), the categorical-variable titles (de, es, pt, nl, ko 범주형, uk
  категорійна, zh 分类; et calls it kvalitatiivne), Wiktionary's sequential (bg, ca, de, es, fi
  peräkkäinen, ro) and divergent (bg разходящ, cs, fi hajaantuva, hu, ka განშლადი, pl rozbieżny, ru
  расходящийся, zh 发散/發散), agreeing with the palette word where the language inflects for it.
  Palette is Wiktionary's or the palette article's word (it tavolozza, zh 调色板/調色盤, kk палитра) and
  differs from the theme word (de Paletten beside Farbschemata, lo ຈານສີ beside ຊຸດສີ). In languages
  with cases the slot names stay in the nominative, so the messages start "{colour} – …" or "{first}
  and {second} – …", with the dash of that catalog's contrast row (bg, bn, cs, el, et, fi, hi, hu,
  ka, kk, lt, pl, sk, sl, sq, ta and te, ru and uk with "—", and bs, hr, sr and sr-latn where a
  message needs it), so that a message shown after "{word}: " does not pile up colons; tr, which has
  no dash there, uses a colon, and ko, ta and te write "{first}, {second}" so that no particle has
  to follow a number. The limits 3:1, 0.10, 15, 8 and 0.06 keep "." as the decimal point, as the
  code formats the measured values; "{word}: {message}" follows each catalog's colon (fr " : ", zh
  full width), and el uses the ano teleia for the semicolon. "Tell them apart only with labels" is
  an instruction in most catalogs and "can be told apart only by labels" in th and zh. Chart
  colours… is a menu item, which the catalog test's source scan does not see; as far as can be told
  from reading the test, it is checked through the menus the test lays out, as Edit themes… is. Not
  reviewed by native speakers. The least certain are lo, th, ta, te, bn, ka, kk and sq as before,
  the red-green terms formed from titles (et, fi, ka, kk and lt most), the saturation words standing
  for chroma and the unsourced chroma loanwords, the Sequential and Diverging headings everywhere
  (no source uses them for colour schemes), and Fail, chosen per catalog as a status word beside
  Warning.

**Adding or changing a string:** add the English in the code through `i18n::t` (or `tf`, `tn`),
then add its row to each catalog here, or leave it out to show English until it is translated.
Right-to-left languages are not offered until the layout is mirrored.
