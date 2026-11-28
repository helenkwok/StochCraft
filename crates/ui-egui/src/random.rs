//! Experiment settings ▸ Random numbers: where a run's random numbers come from (the generator,
//! a seed drawn from a source, a file of random bytes, or bytes fetched live), the estimate of
//! what a source is asked for, and the source manager, where keys are entered. A key goes from
//! its field straight to the key store ([`stochcraft_engine::Engine::set_random_key`]): no
//! command, journal or autosave sees it.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::i18n::{t, tf};
use crate::{App, editor::section, theme};

/// Where the next run's random numbers come from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Generator,
    Seed,
    File,
    Live,
}

/// The section's choices and what it last heard.
#[derive(Debug, Default)]
pub(crate) struct RandomUi {
    pub mode: Mode,
    pub source: String,
    pub file: String,
    pub offset: String,
    pub segment: String,
    /// The last seed drawn: its result (`seed`, `random`).
    pub drawn: Option<Value>,
    /// A draw in progress.
    pub drawing: bool,
    pub estimate: Option<String>,
    pub manager: bool,
    keys: HashMap<String, String>,
    new_id: String,
    new_url: String,
    new_header: String,
    new_path: String,
    new_encoding: String,
    pub note: Option<String>,
}

fn sources(app: &mut App) -> Vec<Value> {
    app.engine
        .execute("random.sources", json!({}))
        .ok()
        .and_then(|v| v["sources"].as_array().cloned())
        .unwrap_or_default()
}

/// `seconds` in the unit that reads best: seconds under a minute, then minutes, then hours (to
/// one decimal place), as SI symbols.
pub(crate) fn duration(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3600 => format!("{} min", seconds.div_ceil(60)),
        _ => format!("{:.1} h", seconds as f64 / 3600.0),
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "quantum" => t("quantum"),
        "beacon" => t("public beacon, for seeds only"),
        "not_quantum" => t("not quantum (atmospheric noise)"),
        _ => t("added by you"),
    }
}

impl App {
    /// How the app fetches from random number sources and where it keeps their keys.
    pub fn set_random_net(
        &mut self,
        fetch: Box<dyn stochcraft_qrng::Fetch>,
        keys: Box<dyn stochcraft_qrng::KeyStore>,
    ) {
        self.engine.set_random_net(fetch, keys);
    }

    /// Keeps `key` for source `id` in the key store (the source manager's Save key).
    pub fn set_random_key(&mut self, id: &str, key: &str) -> Result<(), String> {
        self.engine.set_random_key(id, key)
    }

    /// The `run.start` parameters the section asks for: none for the generator; `Err` with what
    /// is missing.
    pub(crate) fn random_run_params(&self) -> Result<Value, String> {
        let r = &self.random;
        let number = |s: &str| s.trim().parse::<u64>().ok();
        Ok(match r.mode {
            Mode::Generator => json!({}),
            Mode::Seed => match &r.drawn {
                Some(d) => json!({ "seed": d["seed"], "random": d["random"] }),
                None => return Err(t("Draw a seed first.").to_owned()),
            },
            Mode::File => {
                if r.file.trim().is_empty() {
                    return Err(t("Give the file of random bytes.").to_owned());
                }
                let mut random = json!({ "mode": "byte_file", "file": r.file.trim(),
                    "offset": number(&r.offset).unwrap_or(0) });
                if let Some(l) = number(&r.segment) {
                    random["segment_bytes"] = json!(l);
                }
                json!({ "random": random })
            }
            Mode::Live => {
                if r.file.trim().is_empty() {
                    return Err(t("Give the file of random bytes.").to_owned());
                }
                let mut random =
                    json!({ "mode": "live", "source": r.source, "file": r.file.trim() });
                if let Some(l) = number(&r.segment) {
                    random["segment_bytes"] = json!(l);
                }
                json!({ "random": random })
            }
        })
    }

    /// The next run uses the generator (and so may go to the shell's workers).
    pub(crate) fn random_is_generator(&self) -> bool {
        self.random.mode == Mode::Generator
    }

    /// Experiment settings ▸ Random numbers.
    pub(crate) fn random_section(&mut self, ui: &mut egui::Ui) {
        section(ui, t("Random numbers"));
        let before = self.random.mode;
        ui.vertical(|ui| {
            let m = &mut self.random.mode;
            ui.radio_value(m, Mode::Generator, t("Generator"));
            ui.radio_value(m, Mode::Seed, t("Seed from a source"));
            ui.radio_value(m, Mode::File, t("File of random bytes"));
            ui.radio_value(m, Mode::Live, t("Live from a source"));
        });
        if self.random.mode != before {
            self.random.estimate = None;
            self.random.source = match self.random.mode {
                Mode::Seed => "nist-beacon".into(),
                _ => "anu".into(),
            };
        }
        let list = sources(self);
        match self.random.mode {
            Mode::Generator => {
                ui.label(theme::weak(
                    ui,
                    t("xoshiro256**, seeded from the experiment's seed: repeatable, and what simulation normally uses."),
                ));
            }
            Mode::Seed => {
                self.source_choice(ui, &list, false);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.random.drawing, egui::Button::new(t("Draw a seed")))
                        .clicked()
                        && self
                            .execute(
                                "random.seed.draw",
                                json!({ "source": self.random.source, "wait": false }),
                            )
                            .is_some()
                    {
                        self.random.drawing = true;
                    }
                });
                if let Some(d) = &self.random.drawn {
                    // The source's name, wrapped in the panel; its address (one long word) only
                    // on hover.
                    let source = d["random"]["source"].as_str().unwrap_or("");
                    ui.add(
                        egui::Label::new(tf(
                            "Seed {seed} taken from {source}",
                            &[
                                ("seed", &d["seed"].to_string()),
                                ("source", crate::results::source_name(source)),
                            ],
                        ))
                        .wrap(),
                    )
                    .on_hover_text(source);
                }
                self.follow_draw(ui);
            }
            Mode::File => {
                self.file_fields(ui, true);
                self.estimate_row(ui, "file");
            }
            Mode::Live => {
                self.source_choice(ui, &list, true);
                self.file_fields(ui, false);
                self.estimate_row(ui, "live");
                ui.label(theme::weak(
                    ui,
                    t("ANU's free tier (100 requests a month) covers seeds, not live runs: a live run needs its paid tier. Each run waits for its bytes, and every byte is recorded in the file, so the run repeats from it."),
                ));
            }
        }
        ui.label(theme::weak(
            ui,
            t("Numbers from a physical source are unpredictable and say where they came from. Against a good generator, studies so far find no difference in accuracy; the guide sums up the evidence."),
        ));
        if let Some(n) = &self.random.note {
            ui.label(theme::weak(ui, n.clone()));
        }
        let label = if self.random.manager {
            t("Hide the sources")
        } else {
            t("Sources and keys…")
        };
        if ui.link(label).clicked() {
            self.random.manager = !self.random.manager;
        }
        if self.random.manager {
            self.source_manager(ui, &list);
        }
    }

    fn source_choice(&mut self, ui: &mut egui::Ui, list: &[Value], for_bytes: bool) {
        let usable: Vec<&Value> = list
            .iter()
            .filter(|s| s["enabled"] == true && (!for_bytes || s["seed_only"] != true))
            .collect();
        let name_of = |id: &str| {
            usable
                .iter()
                .find(|s| s["id"] == id)
                .and_then(|s| s["name"].as_str())
                .unwrap_or(id)
                .to_owned()
        };
        ui.horizontal(|ui| {
            ui.label(t("Source"));
            egui::ComboBox::from_id_salt("random/source")
                .selected_text(name_of(&self.random.source))
                .show_ui(ui, |ui| {
                    for s in &usable {
                        let id = s["id"].as_str().unwrap_or("").to_owned();
                        let text = format!(
                            "{} ({})",
                            s["name"].as_str().unwrap_or(""),
                            kind_label(s["kind"].as_str().unwrap_or(""))
                        );
                        ui.selectable_value(&mut self.random.source, id, text);
                    }
                });
            if let Some(s) = usable
                .iter()
                .find(|s| s["id"] == self.random.source.as_str())
                && s["key"] == "absent"
            {
                ui.label(theme::weak(ui, t("No key")));
            }
        });
    }

    fn file_fields(&mut self, ui: &mut egui::Ui, offset: bool) {
        ui.horizontal(|ui| {
            ui.label(t("File"));
            ui.add(egui::TextEdit::singleline(&mut self.random.file).id_salt("random/file"));
        });
        ui.horizontal(|ui| {
            if offset {
                ui.label(t("From byte"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.random.offset)
                        .id_salt("random/offset")
                        .desired_width(80.0),
                );
            }
            ui.label(t("Bytes per run"));
            ui.add(
                egui::TextEdit::singleline(&mut self.random.segment)
                    .id_salt("random/segment")
                    .hint_text(t("from a pilot"))
                    .desired_width(160.0),
            );
        });
    }

    fn estimate_row(&mut self, ui: &mut egui::Ui, mode: &str) {
        {
            if ui.button(t("Estimate")).clicked() {
                let mut p = json!({ "mode": mode, "source": self.random.source });
                if let Ok(l) = self.random.segment.trim().parse::<u64>() {
                    p["segment_bytes"] = json!(l);
                }
                let numbers = self.numbers.clone();
                let n = move |v: &Value| numbers.format(v.as_f64().unwrap_or(0.0), 0..=0);
                self.random.estimate = self.execute("random.estimate", p).map(|v| {
                    let mut s = tf(
                        "{bytes} bytes for {runs} runs: {requests} requests, at least {time}",
                        &[
                            ("bytes", &n(&v["bytes"])),
                            ("runs", &n(&v["replications"])),
                            ("requests", &n(&v["requests"])),
                            (
                                "time",
                                &duration(v["seconds_at_least"].as_u64().unwrap_or(0)),
                            ),
                        ],
                    );
                    if v["within_free_tier"] == false {
                        // Its own line: the estimate has no full stop to end on.
                        s.push('\n');
                        s.push_str(t("More than a month of the free tier allows."));
                    }
                    s
                });
            }
            if let Some(e) = &self.random.estimate {
                ui.add(egui::Label::new(e.clone()).wrap());
            }
        }
    }

    /// Follows a seed draw in progress, a frame at a time.
    fn follow_draw(&mut self, ui: &egui::Ui) {
        if !self.random.drawing {
            return;
        }
        let Some(s) = self.execute("random.status", json!({})) else {
            self.random.drawing = false;
            return;
        };
        match s["state"].as_str() {
            Some("running") => ui
                .ctx()
                .request_repaint_after(std::time::Duration::from_millis(100)),
            Some("finished") => {
                self.random.drawing = false;
                self.random.drawn = Some(s["result"].clone());
                self.random.note = s["result"]["health"]["warnings"]
                    .as_array()
                    .filter(|w| !w.is_empty())
                    .map(|w| {
                        w.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("; ")
                    });
            }
            _ => {
                self.random.drawing = false;
                self.report_error(s["error"]["message"].as_str().unwrap_or("").to_owned());
            }
        }
    }

    fn source_manager(&mut self, ui: &mut egui::Ui, list: &[Value]) {
        if let Some(w) = self.engine.random_key_warning() {
            ui.label(egui::RichText::new(t(w)).color(theme::palette(ui.ctx()).warning));
        }
        for s in list {
            let id = s["id"].as_str().unwrap_or("").to_owned();
            ui.horizontal_wrapped(|ui| {
                let mut on = s["enabled"] == true;
                if ui
                    .checkbox(&mut on, s["name"].as_str().unwrap_or(""))
                    .changed()
                {
                    self.execute("random.source.set", json!({ "id": id, "enabled": on }));
                }
                ui.label(theme::weak(
                    ui,
                    format!(
                        "{} · {}",
                        kind_label(s["kind"].as_str().unwrap_or("")),
                        s["url"].as_str().unwrap_or("")
                    ),
                ));
                match s["key"].as_str() {
                    Some("present") => {
                        ui.label(t("Key kept"));
                        if ui.button(t("Forget key")).clicked() {
                            self.engine.forget_random_key(&id);
                        }
                    }
                    Some("absent") => {
                        let draft = self.random.keys.entry(id.clone()).or_default();
                        ui.add(
                            egui::TextEdit::singleline(draft)
                                .password(true)
                                .id_salt(format!("random/key/{id}"))
                                .hint_text(t("Key"))
                                .desired_width(160.0),
                        );
                        if ui.button(t("Save key")).clicked() {
                            let key = self.random.keys.remove(&id).unwrap_or_default();
                            if let Err(e) = self.set_random_key(&id, &key) {
                                self.report_error(e);
                            }
                        }
                    }
                    _ => {}
                }
            });
        }
        ui.label(theme::strong(t("Add a source")));
        let r = &mut self.random;
        egui::Grid::new("random/new").num_columns(2).show(ui, |ui| {
            ui.label(t("Name"));
            ui.add(
                egui::TextEdit::singleline(&mut r.new_id)
                    .id_salt("random/new/id")
                    .hint_text("my-qrng"),
            );
            ui.end_row();
            ui.label(t("Address, with {n} for the count"));
            ui.add(
                egui::TextEdit::singleline(&mut r.new_url)
                    .id_salt("random/new/url")
                    .hint_text("https://…?length={n}"),
            );
            ui.end_row();
            ui.label(t("Key header"));
            ui.add(
                egui::TextEdit::singleline(&mut r.new_header)
                    .id_salt("random/new/header")
                    .hint_text("x-api-key"),
            );
            ui.end_row();
            ui.label(t("JSON path"));
            ui.add(
                egui::TextEdit::singleline(&mut r.new_path)
                    .id_salt("random/new/path")
                    .hint_text("data"),
            );
            ui.end_row();
            ui.label(t("Encoding"));
            egui::ComboBox::from_id_salt("random/new/encoding")
                .selected_text(if r.new_encoding.is_empty() {
                    "hex"
                } else {
                    r.new_encoding.as_str()
                })
                .show_ui(ui, |ui| {
                    for e in ["hex", "base64", "numbers", "raw"] {
                        ui.selectable_value(&mut r.new_encoding, e.to_owned(), e);
                    }
                });
            ui.end_row();
        });
        if ui.button(t("Add")).clicked() {
            let r = &self.random;
            let encoding = if r.new_encoding.is_empty() {
                "hex"
            } else {
                r.new_encoding.as_str()
            };
            let format = match (r.new_path.trim(), encoding) {
                (_, "raw") => json!({ "type": "raw" }),
                ("", e) => json!({ "type": e }),
                (p, e) => json!({ "type": "json", "path": p, "encoding": e }),
            };
            let params = json!({
                "id": r.new_id.trim().to_ascii_lowercase().replace(' ', "-"),
                "url": r.new_url.trim(),
                "key_header": r.new_header.trim(),
                "format": format,
                "max_units": if r.new_url.contains("{n}") { 1024 } else { 1 },
            });
            if self.execute("random.source.set", params).is_some() {
                self.random.new_id.clear();
                self.random.new_url.clear();
                self.random.new_header.clear();
                self.random.new_path.clear();
            }
        }
    }
}
