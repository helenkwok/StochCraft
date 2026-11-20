//! Monte Carlo models in the window (ROADMAP M4.1): a third kind of document beside process
//! models, a calculation with uncertain inputs and no process.
//!
//! A model is a Monte Carlo one when it has the engine's Monte Carlo section and no blocks
//! (`Model::is_monte_carlo`, never stored here: undoing the edit that made one turns it back).
//! It shows two tabs, Model and Results, and none of a process model's views. The Model tab
//! edits the inputs (each a distribution, with the same editor as a task's time), the rank
//! correlations between them, the outputs (expressions over the inputs, with an optional target)
//! and the trials, seed, sampling and stop; each edit is one `mc.*` command, so one undo step. A
//! refused edit says why under its field, not only in the status bar.
//!
//! Run makes the trials a few batches a frame ([`stochcraft_engine::mc::McJob`]), so a long run
//! never holds up the window, the browser's page included (there the batches run one after
//! another on the page, not on its workers); the results are `mc.run`'s.
//!
//! The results answer the question first, as established Monte Carlo tools do: for each output
//! with a target, the chance of missing it in words ("about a 1 in 5 chance that cost is more than
//! 7,000 USD") with its interval; for each without, where 9 trials in 10 fall. Then each output's
//! mean with its interval, its percentiles, minimum and maximum, a histogram and the cumulative
//! curve with the target drawn on both, and how the run went (trials asked for and run, the
//! sampling, the seed, and a sequential stop's outcome).

use std::collections::HashMap;

use egui::{Align2, FontId, Rect, Stroke, pos2, vec2};
use serde_json::{Value, json};
use stochcraft_engine::mc::McJob;

use crate::charts::nice_step;
use crate::decide::{Bound, bound_choice, choose, grid, number, percent, row_label};
use crate::editor::{params_schema, section};
use crate::form::{self, Schema};
use crate::i18n::{t, tf, tn, tnf};
use crate::results::{grouped, round_sig, rounded};
use crate::{App, Request, Running, commands, comparable, theme};

/// The time a frame spends making trials before it draws.
const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(8);

/// The first distribution of a new input: a three-point estimate, the most common input.
fn first_distribution() -> Value {
    json!({ "type": "triangular", "min": 1.0, "mode": 2.0, "max": 4.0 })
}

/// What the Monte Carlo screens keep between frames.
#[derive(Default)]
pub(crate) struct Mc {
    /// The run in progress, made a few batches a frame.
    pub(crate) job: Option<McJob>,
    /// Why an edit was refused, by the key of the field or row it was made in.
    errors: HashMap<String, String>,
    /// The name typed for a new input, and for a new output.
    new_input: String,
    new_output: String,
    /// The pair chosen for a new correlation.
    pair: (Option<String>, Option<String>),
    /// A table of inputs pasted in, and whether it replaces every input.
    paste: String,
    replace: bool,
}

/// A number with its unit after it, when it has one.
fn with_unit(text: String, unit: &str) -> String {
    if unit.is_empty() {
        text
    } else {
        format!("{text} {unit}")
    }
}

/// A value for a sentence or a table: about four significant figures, in the user's region.
fn value_text(v: f64) -> String {
    grouped(round_sig(v, 4))
}

/// A chance as a percentage: to a tenth of a percent from 1% up, and below that to one
/// significant figure, so a small chance never reads as 0%.
fn chance(p: f64) -> String {
    let v = p * 100.0;
    if v > 0.0 && v < 1.0 {
        format!("{}%", grouped(round_sig(v, 1)))
    } else {
        percent(p)
    }
}

/// A percentile of output `o`'s results (`"5"`, `"95"`…).
fn percentile(o: &Value, p: &str) -> Option<f64> {
    o["percentiles"][p].as_f64()
}

/// The chance written as odds where they read naturally: "1 in 5" for 20%. Only for a chance from
/// 0.1% to 25%, where the nearest "1 in N" is within about 12% of it (the percentage is said
/// beside it anyway).
fn odds(chance: f64) -> Option<u64> {
    if !(0.001..=0.25).contains(&chance) {
        return None;
    }
    let n = (1.0 / chance).round();
    (n >= 4.0).then_some(n as u64)
}

/// An output's finding in words, and a second line on how sure it is: with a target, the chance
/// of missing it with its 95% interval; without one, where 9 trials in 10 fall and the mean.
/// `None` when the output has no value to speak of (no trial gave a number).
pub(crate) fn finding(o: &Value) -> Option<(String, Option<String>)> {
    let name = o["name"].as_str().unwrap_or("");
    let unit = o["unit"].as_str().unwrap_or("");
    let target = &o["target"];
    if target.is_object() {
        let (at_most, x) = match (target["at_most"].as_f64(), target["at_least"].as_f64()) {
            (Some(x), _) => (true, x),
            (None, Some(x)) => (false, x),
            (None, None) => return None,
        };
        let met = target["probability"].as_f64()?;
        let miss = (1.0 - met).clamp(0.0, 1.0);
        let value = with_unit(value_text(x), unit);
        let shown = chance(miss);
        let in_odds = odds(miss);
        let odds_text = in_odds.map_or_else(String::new, |n| grouped(n as f64));
        let trials = target["of"].as_u64().unwrap_or(0).to_string();
        let args = [
            ("output", name),
            ("value", value.as_str()),
            ("percent", shown.as_str()),
            ("odds", odds_text.as_str()),
            ("n", trials.as_str()),
        ];
        let sentence = match (miss <= 0.0, in_odds.is_some(), at_most) {
            (true, _, true) => tf(
                "{output} was more than {value} in none of the {n} trials.",
                &args,
            ),
            (true, _, false) => tf(
                "{output} was less than {value} in none of the {n} trials.",
                &args,
            ),
            (false, true, true) => tf(
                "About a 1 in {odds} chance that {output} is more than {value} ({percent}).",
                &args,
            ),
            (false, true, false) => tf(
                "About a 1 in {odds} chance that {output} is less than {value} ({percent}).",
                &args,
            ),
            (false, false, true) => tf(
                "A {percent} chance that {output} is more than {value}.",
                &args,
            ),
            (false, false, false) => tf(
                "A {percent} chance that {output} is less than {value}.",
                &args,
            ),
        };
        // The interval of the chance of missing: the met share's, turned round.
        let interval = target["interval"][0]
            .as_f64()
            .zip(target["interval"][1].as_f64())
            .map(|(lo, hi)| ((1.0 - hi).clamp(0.0, 1.0), (1.0 - lo).clamp(0.0, 1.0)));
        let sure = match interval {
            Some((_, hi)) if miss <= 0.0 => tf(
                "The chance is under {hi}, with 95% confidence.",
                &[("hi", &chance(hi))],
            ),
            Some((lo, hi)) => tf(
                "95% interval of the chance: {lo} to {hi}.",
                &[("lo", &chance(lo)), ("hi", &chance(hi))],
            ),
            None => t("No interval: too few batches of trials for one.").to_owned(),
        };
        return Some((sentence, Some(sure)));
    }
    let (low, high, mean) = (
        percentile(o, "5")?,
        percentile(o, "95")?,
        o["mean"].as_f64()?,
    );
    let low = with_unit(value_text(low), unit);
    let high = with_unit(value_text(high), unit);
    let sentence = match o["half_width"].as_f64() {
        Some(hw) => {
            let (m, h) = rounded(mean, hw);
            tf(
                "In 9 trials out of 10, {output} is between {low} and {high}; the mean is {mean} ± {half_width}.",
                &[
                    ("output", name),
                    ("low", &low),
                    ("high", &high),
                    ("mean", &with_unit(m, unit)),
                    ("half_width", &h),
                ],
            )
        }
        None => tf(
            "In 9 trials out of 10, {output} is between {low} and {high}; the mean is {mean}.",
            &[
                ("output", name),
                ("low", &low),
                ("high", &high),
                ("mean", &with_unit(value_text(mean), unit)),
            ],
        ),
    };
    Some((sentence, None))
}

/// The run in a line: trials, seed and sampling.
fn record_line(r: &Value) -> String {
    let sampling = if r["sampling"] == "latin_hypercube" {
        t("Latin hypercube sampling")
    } else {
        t("random sampling")
    };
    tnf(
        r["trials"].as_u64().unwrap_or(0),
        "{n} trial · seed {seed} · {sampling}",
        "{n} trials · seed {seed} · {sampling}",
        &[("seed", &r["seed"].to_string()), ("sampling", sampling)],
    )
}

/// What the intervals are across, in words: trials, or batches (and why).
fn basis_line(r: &Value) -> String {
    let b = &r["interval_basis"];
    let count = b["count"].as_u64().unwrap_or(0);
    if b["across"] == "batches" {
        tnf(
            count,
            "Each ± is the half-width of the 95% interval of the mean across {n} independent batch of {size} trials. Latin hypercube sampling and correlated inputs make the trials within a batch depend on each other, so each batch counts as one.",
            "Each ± is the half-width of the 95% interval of the mean across {n} independent batches of {size} trials. Latin hypercube sampling and correlated inputs make the trials within a batch depend on each other, so each batch counts as one.",
            &[("size", &grouped(b["size"].as_f64().unwrap_or(1000.0)))],
        )
    } else {
        tn(
            count,
            "Each ± is the half-width of the 95% interval of the mean across {n} independent trial.",
            "Each ± is the half-width of the 95% interval of the mean across {n} independent trials.",
        )
    }
}

/// Whether the trials run differ from those asked for, and why, in words: whole batches.
fn rounding_line(r: &Value) -> Option<String> {
    let (run, asked) = (r["trials"].as_u64()?, r["trials_requested"].as_u64()?);
    (r["stop"].is_null() && run != asked).then(|| {
        tf(
            "{asked} trials were asked for; with Latin hypercube sampling or correlated inputs a run is whole batches of {size}, so {n} ran.",
            &[
                ("asked", &grouped(asked as f64)),
                ("size", &grouped(r["interval_basis"]["size"].as_f64().unwrap_or(1000.0))),
                ("n", &grouped(run as f64)),
            ],
        )
    })
}

/// How a sequential run stopped, in words.
fn stop_line(r: &Value) -> Option<String> {
    let stop = r["stop"].as_object()?;
    let n = r["trials"].as_u64().unwrap_or(0);
    let share = percent(stop.get("relative").and_then(Value::as_f64).unwrap_or(0.0));
    Some(
        if stop.get("reason").and_then(Value::as_str) == Some("target_met") {
            tnf(
                n,
                "Stopped after {n} trial: every output's mean is within ±{share} of itself.",
                "Stopped after {n} trials: every output's mean is within ±{share} of itself.",
                &[("share", &share)],
            )
        } else {
            tnf(
                n,
                "Stopped at the most allowed, {n} trial, before every output's mean was within ±{share} of itself.",
                "Stopped at the most allowed, {n} trials, before every output's mean was within ±{share} of itself.",
                &[("share", &share)],
            )
        },
    )
}

/// Text pasted as a table of inputs, made into the CSV `mc.inputs.import` reads: cells copied
/// from a spreadsheet come separated by tabs, and the header may be left out.
pub(crate) fn table_text(pasted: &str) -> String {
    let mut lines: Vec<String> = pasted
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            if l.contains('\t') {
                l.split('\t')
                    .map(|c| stochcraft_engine::wire::csv_field(c.trim()))
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                l.trim().to_owned()
            }
        })
        .collect();
    let headed = lines.first().is_some_and(|l| {
        l.to_ascii_lowercase()
            .replace(' ', "")
            .starts_with("name,distribution")
    });
    if !headed {
        lines.insert(0, "name,distribution,parameters,unit".to_owned());
    }
    lines.join("\n") + "\n"
}

impl App {
    /// The model is a Monte Carlo one.
    pub(crate) fn is_monte_carlo(&self) -> bool {
        self.engine.model().is_monte_carlo()
    }

    /// The results on show are a Monte Carlo run's.
    pub(crate) fn mc_results_shown(&self) -> bool {
        self.results
            .as_deref()
            .is_some_and(|r| r["kind"] == "monte_carlo")
    }

    /// Runs an `mc.*` edit made in the field or row `key`: its refusal is shown there.
    fn mc_edit(&mut self, key: &str, command: &str, params: Value) -> Option<Value> {
        match self.engine.execute(command, params) {
            Ok(v) => {
                self.mc.errors.remove(key);
                Some(v)
            }
            Err(e) => {
                self.mc.errors.insert(key.to_owned(), e.message);
                None
            }
        }
    }

    /// Why the edit in `key` was refused, under it.
    fn mc_error(&self, ui: &mut egui::Ui, key: &str) {
        if let Some(m) = self.mc.errors.get(key) {
            theme::error_message(ui, m.clone());
        }
    }

    /// File ▸ New Monte Carlo Model: an empty model with a Monte Carlo section.
    pub(crate) fn new_monte_carlo(&mut self) {
        if self.execute("model.new", json!({})).is_some() {
            self.forget_view();
            if self.execute("mc.set", json!({})).is_some() {
                self.info(t("New Monte Carlo model"));
            }
        }
    }

    /// Model ▸ Run on a Monte Carlo model: starts the trials, made a few batches each frame. A
    /// model with errors doesn't run: its next error is shown instead.
    pub(crate) fn mc_run(&mut self) {
        if self.running.is_some() || self.go_to_problem() {
            return;
        }
        match self.engine.mc_start(json!({})) {
            Ok(job) => {
                let (done, total) = job.progress();
                self.mc.job = Some(job);
                self.show_results = true;
                self.run_model = Some(comparable(self.engine.model()));
                self.running = Some(Running {
                    done,
                    partial: 0.0,
                    total,
                    on_workers: false,
                });
                self.info(t("Running…"));
            }
            Err(e) => self.command_failed(e),
        }
    }

    /// Makes trials for about a frame's time; true while the run goes on.
    pub(crate) fn mc_poll(&mut self) -> bool {
        let Some(job) = self.mc.job.as_mut() else {
            return false;
        };
        // Natively the batches of a step are made on every core; in the browser one at a time,
        // so a frame is never held up by more than a batch.
        let batches = if cfg!(target_arch = "wasm32") { 1 } else { 16 };
        let start = web_time::Instant::now();
        let outcome = loop {
            let step = job.step(batches);
            if !matches!(step, Ok(false)) || start.elapsed() >= FRAME_BUDGET {
                break step;
            }
        };
        let (done, total) = job.progress();
        match outcome {
            Ok(false) => {
                if let Some(r) = &mut self.running {
                    r.done = done;
                    r.total = total;
                }
                true
            }
            Ok(true) => {
                let results = self
                    .mc
                    .job
                    .take()
                    .and_then(|mut job| self.engine.mc_results(&mut job));
                self.running = None;
                let model = self
                    .run_model
                    .take()
                    .unwrap_or_else(|| comparable(self.engine.model()));
                if let Some(results) = results {
                    let n = results["trials"].as_u64().unwrap_or(0);
                    self.set_results(results, model);
                    self.info(tn(n, "Finished {n} trial", "Finished {n} trials"));
                }
                false
            }
            Err(e) => {
                self.mc.job = None;
                self.running = None;
                self.report_error(e.message);
                false
            }
        }
    }

    /// The outline of a Monte Carlo model: its inputs and outputs, and how it runs.
    pub(crate) fn mc_outline(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let model = self.wire_model();
        let name = model["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .unwrap_or(t("Untitled model"))
            .to_owned();
        let overview = !self.show_results;
        if ui
            .add(
                egui::Button::selectable(overview, theme::strong(&name).size(17.0))
                    .frame_when_inactive(false),
            )
            .on_hover_text(t("The model: its inputs, outputs and trials"))
            .clicked()
        {
            self.show_results = false;
        }
        ui.label(theme::weak(ui, t("Monte Carlo model")));
        let mc = &model["monte_carlo"];
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(ui.available_height() - 110.0)
            .show(ui, |ui| {
                for (title, key) in [(t("Inputs"), "inputs"), (t("Outputs"), "outputs")] {
                    ui.add_space(12.0);
                    ui.label(theme::strong(title).size(12.0).color(p.text_weak));
                    ui.add_space(2.0);
                    let rows = mc[key].as_array().cloned().unwrap_or_default();
                    if rows.is_empty() {
                        ui.label(theme::weak(ui, t("None yet")));
                    }
                    for row in rows {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(row["name"].as_str().unwrap_or(""));
                            let kind = row["distribution"]["type"].as_str().map(form::choice_label);
                            if let Some(kind) = kind {
                                ui.label(theme::weak(ui, kind));
                            }
                        });
                    }
                }
            });
        let x = &model["experiment"];
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let sampling = if mc["sampling"] == "latin_hypercube" {
                t("Latin hypercube sampling")
            } else {
                t("random sampling")
            };
            ui.label(theme::weak(ui, sampling));
            if let Some(max) = mc["stop"]["max"].as_u64() {
                ui.label(theme::weak(
                    ui,
                    tf(
                        "then more until precise enough, at most {max}",
                        &[("max", &grouped(max as f64))],
                    ),
                ));
            }
            ui.label(tnf(
                x["replications"].as_u64().unwrap_or(0),
                "{n} trial · seed {seed}",
                "{n} trials · seed {seed}",
                &[("seed", &x["seed"].to_string())],
            ));
            ui.add_space(2.0);
            ui.label(theme::strong(t("Trials")).size(12.0).color(p.text_weak));
            ui.add_space(4.0);
            ui.separator();
        });
    }

    /// The Model tab of a Monte Carlo model.
    pub(crate) fn mc_model_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let model = self.wire_model();
        let mc = model["monte_carlo"].clone();
        let problems = self.problems();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(760.0));
                ui.heading(t("Monte Carlo model"));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(theme::weak(
                        ui,
                        t("A calculation with uncertain inputs, not a process: each trial draws every input once and works out every output from them."),
                    ))
                    .wrap(),
                );
                ui.add_space(8.0);
                let name = model["name"].as_str().unwrap_or("").to_owned();
                if let Some(n) = self.text_field(ui, "model/name", t("Name"), &name, true) {
                    self.execute("model.rename", json!({ "name": n }));
                }
                self.mc_inputs(ui, &p, &mc);
                self.mc_correlations(ui, &mc);
                self.mc_outputs(ui, &p, &mc);
                self.mc_settings(ui, &model);
                section(ui, t("Problems"));
                if problems.is_empty() {
                    ui.label(theme::weak(ui, t("None found. The model is ready to run.")));
                }
                for pr in &problems {
                    self.problem_row(ui, &p, pr, false);
                }
            });
    }

    /// A box around one input or output.
    fn mc_card(ui: &mut egui::Ui, p: &theme::Palette, add: impl FnOnce(&mut egui::Ui)) {
        egui::Frame::new()
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui);
            });
        ui.add_space(6.0);
    }

    /// The inputs: each with its unit and its distribution, a way to add one, and the table to
    /// paste or export.
    fn mc_inputs(&mut self, ui: &mut egui::Ui, p: &theme::Palette, mc: &Value) {
        section(ui, t("Inputs"));
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("Each input is a distribution: every trial draws one value from it."),
            ))
            .wrap(),
        );
        ui.add_space(4.0);
        let schema = params_schema("mc.input.set");
        let inputs = mc["inputs"].as_array().cloned().unwrap_or_default();
        let mut edits: Vec<(String, Value)> = Vec::new();
        let mut remove = None;
        for input in &inputs {
            let name = input["name"].as_str().unwrap_or("").to_owned();
            let unit = input["unit"].as_str().unwrap_or("").to_owned();
            let key = format!("mc/in/{name}");
            Self::mc_card(ui, p, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::strong(&name).size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(theme::secondary(ui, t("Remove")))
                            .on_hover_text(tf("Remove the input {name}", &[("name", &name)]))
                            .clicked()
                        {
                            remove = Some(name.clone());
                        }
                    });
                });
                if let Some(u) = self.text_field(ui, &format!("{key}/unit"), t("Unit"), &unit, true)
                {
                    edits.push((key.clone(), json!({ "name": name, "unit": u })));
                }
                let changed = {
                    let mut cx = form::Cx {
                        time_unit: &unit,
                        numbers: &self.numbers,
                        drafts: &mut self.drafts,
                        calendars: &[],
                    };
                    form::field(
                        ui,
                        &mut cx,
                        Schema::new(&schema),
                        &format!("{key}/distribution"),
                        "distribution",
                        &input["distribution"],
                        false,
                    )
                };
                if let Some(d) = changed {
                    edits.push((key.clone(), json!({ "name": name, "distribution": d })));
                }
                self.mc_error(ui, &key);
            });
        }
        if inputs.is_empty() {
            ui.label(theme::weak(
                ui,
                t("No inputs yet: add one below, or paste a table of them."),
            ));
        }
        for (key, params) in edits {
            self.mc_edit(&key, "mc.input.set", params);
        }
        if let Some(name) = remove {
            self.mc_edit("mc/in", "mc.input.remove", json!({ "name": name }));
        }
        self.mc_error(ui, "mc/in");
        // A new input, named here; it starts as a three-point estimate to change.
        let mut add = false;
        ui.horizontal(|ui| {
            row_label(ui, t("New input"));
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.mc.new_input)
                    .desired_width(180.0)
                    .hint_text(t("a name, such as labour")),
            );
            form::field_outline(ui, &r);
            r.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TextInput, true, t("New input"))
            });
            let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            add = ui.add(theme::secondary(ui, t("Add input"))).clicked() || entered;
        });
        let name = self.mc.new_input.trim().to_owned();
        if add && !name.is_empty() {
            let params = json!({ "name": name, "distribution": first_distribution() });
            if self
                .mc_edit("mc/new_input", "mc.input.set", params)
                .is_some()
            {
                self.mc.new_input.clear();
            }
        }
        self.mc_error(ui, "mc/new_input");
        ui.add_space(6.0);
        self.mc_table(ui);
    }

    /// Inputs from a pasted table (a spreadsheet's cells, or CSV), and the inputs as a CSV file.
    fn mc_table(&mut self, ui: &mut egui::Ui) {
        let mut import = false;
        egui::CollapsingHeader::new(t("Paste inputs from a table"))
            .id_salt("mc/paste")
            .default_open(false)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(theme::weak(
                        ui,
                        t("One input per line, as name, distribution, parameters and unit, copied from a spreadsheet or written as CSV: for example labour,triangular,min=80;mode=100;max=150,h."),
                    ))
                    .wrap(),
                );
                let r = ui.add(
                    egui::TextEdit::multiline(&mut self.mc.paste)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, t("Inputs as a table"))
                });
                ui.checkbox(&mut self.mc.replace, t("Replace every input"));
                import = ui
                    .add_enabled(
                        !self.mc.paste.trim().is_empty(),
                        theme::secondary(ui, t("Import inputs")),
                    )
                    .clicked();
                self.mc_error(ui, "mc/import");
            });
        if import {
            let params = json!({ "csv": table_text(&self.mc.paste), "replace": self.mc.replace });
            if let Some(r) = self.mc_edit("mc/import", "mc.inputs.import", params) {
                self.mc.paste.clear();
                self.info(tn(
                    r["inputs"].as_u64().unwrap_or(0),
                    "Imported {n} input",
                    "Imported {n} inputs",
                ));
            }
        }
        let export = if self.platform.file_system {
            t("Export inputs (CSV)")
        } else {
            t("Download inputs (CSV)")
        };
        if ui.add(theme::secondary(ui, export)).clicked()
            && let Some(r) = self.execute("mc.inputs.export", json!({}))
        {
            let name = format!("{}-inputs.csv", self.export_stem());
            let bytes = r["csv"].as_str().unwrap_or("").as_bytes().to_vec();
            self.requests.push(Request::Export { name, bytes });
        }
    }

    /// The rank correlations between inputs: each with its value, and a way to add one.
    fn mc_correlations(&mut self, ui: &mut egui::Ui, mc: &Value) {
        section(ui, t("Correlations"));
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("A rank correlation between two inputs, from -1 to 1: above 0 they tend to be high together, below 0 one tends to be high when the other is low. Inputs not paired here are independent; 0 removes a pair."),
            ))
            .wrap(),
        );
        ui.add_space(4.0);
        let pairs = mc["correlations"].as_array().cloned().unwrap_or_default();
        let mut set = None;
        for c in &pairs {
            let (a, b) = (
                c["a"].as_str().unwrap_or("").to_owned(),
                c["b"].as_str().unwrap_or("").to_owned(),
            );
            let before = c["rank"].as_f64().unwrap_or(0.0);
            let mut rank = before;
            let mut remove = false;
            ui.horizontal(|ui| {
                row_label(ui, &format!("{a} ↔ {b}"));
                number(
                    ui,
                    &self.numbers,
                    &format!("mc/corr/{a}/{b}"),
                    t("Rank correlation"),
                    &mut rank,
                    70.0,
                );
                remove = ui
                    .add(theme::secondary(ui, t("Remove")))
                    .on_hover_text(tf(
                        "Remove the correlation of {a} and {b}",
                        &[("a", &a), ("b", &b)],
                    ))
                    .clicked();
            });
            if remove {
                rank = 0.0;
            }
            if rank != before {
                set = Some(json!({ "a": a, "b": b, "rank": rank }));
            }
        }
        if let Some(params) = set {
            self.mc_edit("mc/corr", "mc.correlation.set", params);
        }
        let names: Vec<(String, String)> = mc["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["name"].as_str())
            .map(|n| (n.to_owned(), n.to_owned()))
            .collect();
        let mut add = false;
        ui.horizontal_wrapped(|ui| {
            row_label(ui, t("New correlation"));
            choose(
                ui,
                "mc/corr/a",
                t("First input"),
                &names,
                &mut self.mc.pair.0,
                140.0,
            );
            ui.label(theme::weak(ui, "↔"));
            choose(
                ui,
                "mc/corr/b",
                t("Second input"),
                &names,
                &mut self.mc.pair.1,
                140.0,
            );
            let ready = matches!(&self.mc.pair, (Some(a), Some(b)) if a != b);
            add = ui
                .add_enabled(ready, theme::secondary(ui, t("Add correlation")))
                .clicked();
        });
        if add && let (Some(a), Some(b)) = self.mc.pair.clone() {
            let params = json!({ "a": a, "b": b, "rank": 0.5 });
            if self
                .mc_edit("mc/corr", "mc.correlation.set", params)
                .is_some()
            {
                self.mc.pair = (None, None);
            }
        }
        self.mc_error(ui, "mc/corr");
    }

    /// The outputs: each with its expression, unit and target, and a way to add one.
    fn mc_outputs(&mut self, ui: &mut egui::Ui, p: &theme::Palette, mc: &Value) {
        section(ui, t("Outputs"));
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("Each output is worked out in every trial from the inputs and the outputs above it, each named as $name: for example $labour * $rate + $materials. Give it a target to see the chance of meeting it."),
            ))
            .wrap(),
        );
        ui.add_space(4.0);
        let outputs = mc["outputs"].as_array().cloned().unwrap_or_default();
        let mut edits: Vec<(String, Value, Option<(String, String)>)> = Vec::new();
        let mut remove = None;
        for o in &outputs {
            let name = o["name"].as_str().unwrap_or("").to_owned();
            let unit = o["unit"].as_str().unwrap_or("").to_owned();
            let key = format!("mc/out/{name}");
            Self::mc_card(ui, p, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::strong(&name).size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(theme::secondary(ui, t("Remove")))
                            .on_hover_text(tf("Remove the output {name}", &[("name", &name)]))
                            .clicked()
                        {
                            remove = Some(name.clone());
                        }
                    });
                });
                // The expression: a refused one stays in its field, with the reason under it.
                let field = format!("{key}/expression");
                let typed = ui
                    .horizontal(|ui| {
                        row_label(ui, t("Expression"));
                        let mut cx = form::Cx {
                            time_unit: "",
                            numbers: &self.numbers,
                            drafts: &mut self.drafts,
                            calendars: &[],
                        };
                        form::text(
                            ui,
                            &mut cx,
                            &field,
                            t("Expression"),
                            &o["expression"],
                            360.0,
                        )
                    })
                    .inner;
                if let Some(x) = typed {
                    let text = x.as_str().unwrap_or("").to_owned();
                    edits.push((
                        key.clone(),
                        json!({ "name": name, "expression": text }),
                        Some((field.clone(), text)),
                    ));
                }
                if let Some(u) = self.text_field(ui, &format!("{key}/unit"), t("Unit"), &unit, true)
                {
                    edits.push((key.clone(), json!({ "name": name, "unit": u }), None));
                }
                // The target: none, at most or at least a value.
                let (bound, value) = match (o["at_most"].as_f64(), o["at_least"].as_f64()) {
                    (Some(x), _) => (Bound::AtMost, x),
                    (None, Some(x)) => (Bound::AtLeast, x),
                    (None, None) => (Bound::None, 0.0),
                };
                let (mut b, mut v) = (bound, value);
                ui.horizontal(|ui| {
                    row_label(ui, t("Target"));
                    bound_choice(ui, &format!("{key}/bound"), t("Target"), &mut b, true);
                    if b != Bound::None {
                        number(
                            ui,
                            &self.numbers,
                            &format!("{key}/target"),
                            t("Target value"),
                            &mut v,
                            110.0,
                        );
                        if !unit.is_empty() {
                            ui.label(theme::weak(ui, &unit));
                        }
                    }
                });
                if (b, v) != (bound, value) {
                    let params = match b.key() {
                        Some(k) => json!({ "name": name, k: v }),
                        None => json!({ "name": name, "at_most": null }),
                    };
                    edits.push((key.clone(), params, None));
                }
                self.mc_error(ui, &key);
            });
        }
        if outputs.is_empty() {
            ui.label(theme::weak(ui, t("No outputs yet: add one below.")));
        }
        for (key, params, typed) in edits {
            if self.mc_edit(&key, "mc.output.set", params).is_none()
                && let Some((field, text)) = typed
            {
                // Kept in its field, to be fixed rather than typed again.
                self.drafts.insert(field, text);
            }
        }
        if let Some(name) = remove {
            self.mc_edit("mc/out", "mc.output.remove", json!({ "name": name }));
        }
        self.mc_error(ui, "mc/out");
        let mut add = false;
        ui.horizontal(|ui| {
            row_label(ui, t("New output"));
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.mc.new_output)
                    .desired_width(180.0)
                    .hint_text(t("a name, such as cost")),
            );
            form::field_outline(ui, &r);
            r.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TextInput, true, t("New output"))
            });
            let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            add = ui.add(theme::secondary(ui, t("Add output"))).clicked() || entered;
        });
        let name = self.mc.new_output.trim().to_owned();
        if add && !name.is_empty() {
            // A first expression to change: the first input, or nothing yet.
            let first = mc["inputs"][0]["name"]
                .as_str()
                .map_or_else(|| "0".to_owned(), |n| format!("${n}"));
            let params = json!({ "name": name, "expression": first });
            if self
                .mc_edit("mc/new_output", "mc.output.set", params)
                .is_some()
            {
                self.mc.new_output.clear();
            }
        }
        self.mc_error(ui, "mc/new_output");
    }

    /// The trials, seed and sampling, and a sequential stop.
    fn mc_settings(&mut self, ui: &mut egui::Ui, model: &Value) {
        section(ui, t("Trials and sampling"));
        let mc = &model["monte_carlo"];
        let x = &model["experiment"];
        let mut schema = params_schema("mc.set");
        schema["properties"]["trials"]["minimum"] = json!(1);
        let mut changed = None;
        for (f, current) in [
            ("trials", x["replications"].clone()),
            ("seed", x["seed"].clone()),
            ("sampling", mc["sampling"].clone()),
        ] {
            let mut cx = form::Cx {
                time_unit: "",
                numbers: &self.numbers,
                drafts: &mut self.drafts,
                calendars: &[],
            };
            if let Some(v) = form::field(
                ui,
                &mut cx,
                Schema::new(&schema),
                &format!("x/{f}"),
                f,
                &current,
                false,
            ) {
                changed = Some(json!({ f: v }));
            }
        }
        if let Some(params) = changed {
            self.mc_edit("mc/set", "mc.set", params);
        }
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("Latin hypercube sampling spreads each input's draws evenly over its range, so the results settle with fewer trials. With it, or with correlated inputs, trials run in whole batches of 1,000."),
            ))
            .wrap(),
        );
        // A sequential stop, as for a process model's runs.
        let stop = &mc["stop"];
        let on = stop.is_object();
        let kinds = [
            (false, t("Exactly that many")),
            (true, t("More until precise enough")),
        ];
        let mut chosen = on;
        ui.horizontal(|ui| {
            row_label(ui, t("How many trials"));
            craft_ui::ComboBox::from_id_salt("mc/stop")
                .selected_text(kinds.iter().find(|k| k.0 == on).map_or("", |k| k.1))
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (k, l) in kinds {
                        if ui.selectable_label(on == k, l).clicked() {
                            chosen = k;
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("How many trials"))
                });
        });
        if chosen != on {
            let spec = if chosen { json!({}) } else { Value::Null };
            self.mc_edit("mc/set", "mc.set", json!({ "stop": spec }));
        } else if on {
            let mut relative = stop["relative"].as_f64().unwrap_or(0.01) * 100.0;
            let mut max = stop["max"].as_f64().unwrap_or(1_000_000.0);
            let before = (relative, max);
            for (salt, label, value) in [
                (
                    "mc/stop/relative",
                    t("Within ± % of the mean"),
                    &mut relative,
                ),
                ("mc/stop/max", t("At most, trials"), &mut max),
            ] {
                ui.horizontal(|ui| {
                    row_label(ui, label);
                    number(ui, &self.numbers, salt, label, value, 90.0);
                });
            }
            ui.add(
                egui::Label::new(theme::weak(
                    ui,
                    t("The trials above come first; more follow in batches until every output's mean is within that share of itself, or the most is reached."),
                ))
                .wrap(),
            );
            if (relative, max) != before {
                let spec = json!({
                    "relative": (relative / 100.0).clamp(0.0001, 0.99),
                    "max": max.round().clamp(1.0, f64::from(u32::MAX)) as u64,
                });
                self.mc_edit("mc/set", "mc.set", json!({ "stop": spec }));
            }
        }
        self.mc_error(ui, "mc/set");
    }

    /// The Results tab before a Monte Carlo model's first run.
    pub(crate) fn mc_ready(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.heading(t("No results yet"));
        ui.add_space(6.0);
        let x = &self.engine.model().experiment;
        ui.label(theme::weak(
            ui,
            tnf(
                u64::from(x.replications),
                "Makes {n} trial from seed {seed}: each draws every input once and works out every output.",
                "Makes {n} trials from seed {seed}: each draws every input once and works out every output.",
                &[("seed", &x.seed.to_string())],
            ),
        ));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary(ui, t("Run"))).clicked() {
                self.run();
            }
            let key = self.shortcut(commands::RUN, ui.ctx()).unwrap_or_default();
            ui.label(theme::weak(ui, key));
        });
    }

    /// The Results tab of a Monte Carlo run: the findings first, then each output's numbers and
    /// charts, then how the run went.
    pub(crate) fn mc_results_view(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.results.clone() else {
            return;
        };
        let mut csv = false;
        let mut again = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(860.0));
                ui.horizontal(|ui| {
                    ui.heading(t("Results"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let table = if self.platform.file_system {
                            t("Export table (CSV)")
                        } else {
                            t("Download table (CSV)")
                        };
                        csv = ui.add(theme::secondary(ui, table)).clicked();
                    });
                });
                ui.label(theme::weak(ui, record_line(&r)));
                for line in [rounding_line(&r), stop_line(&r)].into_iter().flatten() {
                    ui.add(egui::Label::new(theme::weak(ui, line)).wrap());
                }
                if r["stop"].is_object() {
                    ui.add(
                        egui::Label::new(theme::weak(
                            ui,
                            t("The number of trials was chosen from the results, so an interval may cover the true value less often than 95% of the time."),
                        ))
                        .wrap(),
                    );
                }
                ui.add_space(6.0);
                if self.results_stale {
                    again = self.out_of_date_line(ui);
                    ui.add_space(6.0);
                }
                let outputs = r["outputs"].as_array().cloned().unwrap_or_default();
                ui.label(theme::strong(t("What the run found")));
                ui.add_space(2.0);
                for o in &outputs {
                    if let Some((sentence, sure)) = finding(o) {
                        ui.add(egui::Label::new(sentence).wrap());
                        if let Some(sure) = sure {
                            ui.add(egui::Label::new(theme::weak(ui, sure)).wrap());
                        }
                        ui.add_space(4.0);
                    }
                }
                for w in r["warnings"].as_array().into_iter().flatten() {
                    let text = crate::i18n::engine_message(w);
                    ui.add(egui::Label::new(theme::weak(ui, text)).wrap());
                }
                for o in &outputs {
                    output_section(ui, o);
                }
                section(ui, t("How sure are these numbers?"));
                ui.add(egui::Label::new(basis_line(&r)).wrap());
                let correlations = r["correlations"].as_array().cloned().unwrap_or_default();
                if !correlations.is_empty() {
                    section(ui, t("Correlations"));
                    for c in &correlations {
                        let shown = |v: &Value| v.as_f64().map_or_else(|| "–".to_owned(), |x| grouped(round_sig(x, 2)));
                        ui.label(tf(
                            "{a} and {b}: {achieved} in the trials, {target} asked for",
                            &[
                                ("a", c["a"].as_str().unwrap_or("")),
                                ("b", c["b"].as_str().unwrap_or("")),
                                ("achieved", &shown(&c["achieved"])),
                                ("target", &shown(&c["target"])),
                            ],
                        ));
                    }
                }
            });
        if again {
            self.run();
        }
        if csv {
            self.export_csv();
        }
    }
}

/// One output's numbers and charts.
fn output_section(ui: &mut egui::Ui, o: &Value) {
    let name = o["name"].as_str().unwrap_or("");
    let unit = o["unit"].as_str().unwrap_or("");
    let title = if unit.is_empty() {
        name.to_owned()
    } else {
        format!("{name} ({unit})")
    };
    section(ui, &title);
    let mean = match (o["mean"].as_f64(), o["half_width"].as_f64()) {
        (Some(m), Some(h)) => {
            let (m, h) = rounded(m, h);
            format!("{m} ± {h}")
        }
        (Some(m), None) => value_text(m),
        _ => "–".to_owned(),
    };
    let shown = |v: &Value| v.as_f64().map_or_else(|| "–".to_owned(), value_text);
    let rows = [
        (t("Mean").to_owned(), mean),
        (form::label("std_dev"), shown(&o["std_dev"])),
        (form::label("min"), shown(&o["min"])),
        (form::label("max"), shown(&o["max"])),
    ];
    grid(ui, &format!("mc/result/{name}"), 2, |ui| {
        for (label, value) in rows {
            ui.label(theme::weak(ui, label));
            ui.label(with_unit(value, unit));
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    let mut ps: Vec<(f64, f64)> = o["percentiles"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.parse::<f64>().ok()?, v.as_f64()?)))
        .collect();
    ps.sort_by(|a, b| a.0.total_cmp(&b.0));
    grid(ui, &format!("mc/percentiles/{name}"), ps.len() + 1, |ui| {
        ui.label(theme::weak(ui, t("Percentile")));
        for (q, _) in &ps {
            ui.label(theme::weak(ui, grouped(*q)));
        }
        ui.end_row();
        ui.label(theme::weak(ui, t("Value")));
        for (_, v) in &ps {
            ui.label(value_text(*v));
        }
        ui.end_row();
    });
    if let Some(n) = o["invalid"].as_u64().filter(|n| *n > 0) {
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                tn(
                    n,
                    "{n} trial gave no number (a division by zero, say) and is left out.",
                    "{n} trials gave no number (a division by zero, say) and are left out.",
                ),
            ))
            .wrap(),
        );
    }
    ui.add_space(6.0);
    let target = o["target"]["at_most"]
        .as_f64()
        .or_else(|| o["target"]["at_least"].as_f64());
    let wide = ui.available_width() >= 700.0;
    let width = if wide {
        (ui.available_width() - 24.0) / 2.0
    } else {
        ui.available_width().min(520.0)
    };
    let charts = |ui: &mut egui::Ui| {
        ui.vertical(|ui| {
            ui.set_width(width);
            histogram_chart(ui, o, target, unit, width);
        });
        ui.vertical(|ui| {
            ui.set_width(width);
            cdf_chart(ui, o, target, unit, width);
        });
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            charts(ui);
        });
    } else {
        charts(ui);
    }
}

/// The x range two charts of an output share: its values and its target, with a margin.
fn x_range(o: &Value, target: Option<f64>) -> Option<(f64, f64)> {
    let lo = o["min"].as_f64()?;
    let hi = o["max"].as_f64()?;
    let (lo, hi) = match target {
        Some(x) => (lo.min(x), hi.max(x)),
        None => (lo, hi),
    };
    let span = if hi > lo { hi - lo } else { lo.abs().max(1.0) };
    Some((lo - span * 0.03, hi + span * 0.03))
}

/// Round values along the x axis, labelled under the plot, and the target's line.
fn x_axis(
    painter: &egui::Painter,
    plot: Rect,
    (lo, hi): (f64, f64),
    target: Option<f64>,
    unit: &str,
) {
    let p = theme::palette(painter.ctx());
    let x = |v: f64| plot.left() + ((v - lo) / (hi - lo)) as f32 * plot.width();
    let step = nice_step(hi - lo, 4.0);
    let mut v = (lo / step).ceil() * step;
    let grid = Stroke::new(1.0, p.border);
    let mut ticks = 0;
    while v <= hi && ticks < 12 {
        painter.line_segment([pos2(x(v), plot.top()), pos2(x(v), plot.bottom())], grid);
        painter.text(
            pos2(x(v), plot.bottom() + 3.0),
            Align2::CENTER_TOP,
            grouped(v),
            FontId::proportional(10.0),
            p.text_weak,
        );
        v += step;
        ticks += 1;
    }
    if let Some(tx) = target {
        let at = x(tx);
        painter.line_segment(
            [pos2(at, plot.top()), pos2(at, plot.bottom())],
            Stroke::new(2.0, p.warning),
        );
        painter.text(
            pos2(at + 3.0, plot.top()),
            Align2::LEFT_TOP,
            tf(
                "Target {value}",
                &[("value", &with_unit(value_text(tx), unit))],
            ),
            FontId::proportional(10.0),
            p.warning,
        );
    }
}

/// How many trials gave each value: the output's histogram, with its target.
fn histogram_chart(ui: &mut egui::Ui, o: &Value, target: Option<f64>, unit: &str, width: f32) {
    let edges: Vec<f64> = o["histogram"]["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    let counts: Vec<u64> = o["histogram"]["counts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect();
    let Some(range) = x_range(o, target) else {
        return;
    };
    if counts.is_empty() || edges.len() != counts.len() + 1 {
        return;
    }
    let caption = t("How many trials gave each value");
    ui.label(theme::strong(caption).size(13.0));
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(width, 160.0), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, caption));
    let plot = Rect::from_min_max(
        pos2(rect.left() + 4.0, rect.top() + 14.0),
        pos2(rect.right() - 4.0, rect.bottom() - 18.0),
    );
    let painter = ui.painter_at(rect.expand(2.0));
    let (lo, hi) = range;
    let x = |v: f64| plot.left() + ((v - lo) / (hi - lo)) as f32 * plot.width();
    let most = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    for (i, c) in counts.iter().enumerate() {
        let (Some(a), Some(b)) = (edges.get(i), edges.get(i + 1)) else {
            continue;
        };
        let top = plot.bottom() - *c as f32 / most * plot.height();
        let bar = Rect::from_min_max(
            pos2(x(*a), top),
            pos2(x(*b).max(x(*a) + 1.0), plot.bottom()),
        );
        painter.rect_filled(bar, 0.0, p.accent.gamma_multiply(0.75));
    }
    painter.line_segment(
        [
            pos2(plot.left(), plot.bottom()),
            pos2(plot.right(), plot.bottom()),
        ],
        Stroke::new(1.0, p.text_weak),
    );
    x_axis(&painter, plot, range, target, unit);
}

/// The share of trials at or below each value: the output's cumulative curve, with its target.
fn cdf_chart(ui: &mut egui::Ui, o: &Value, target: Option<f64>, unit: &str, width: f32) {
    let points: Vec<(f64, f64)> = o["cdf"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p[0].as_f64()?, p[1].as_f64()?)))
        .collect();
    let Some(range) = x_range(o, target) else {
        return;
    };
    if points.is_empty() {
        return;
    }
    let caption = t("Share of trials at or below each value");
    ui.label(theme::strong(caption).size(13.0));
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(width, 160.0), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, caption));
    let plot = Rect::from_min_max(
        pos2(rect.left() + 36.0, rect.top() + 14.0),
        pos2(rect.right() - 4.0, rect.bottom() - 18.0),
    );
    let painter = ui.painter_at(rect.expand(2.0));
    let (lo, hi) = range;
    let x = |v: f64| plot.left() + ((v - lo) / (hi - lo)) as f32 * plot.width();
    let y = |f: f64| plot.bottom() - f.clamp(0.0, 1.0) as f32 * plot.height();
    for f in [0.0, 0.5, 1.0] {
        painter.line_segment(
            [pos2(plot.left(), y(f)), pos2(plot.right(), y(f))],
            Stroke::new(1.0, p.border),
        );
        painter.text(
            pos2(plot.left() - 4.0, y(f)),
            Align2::RIGHT_CENTER,
            percent(f),
            FontId::proportional(10.0),
            p.text_weak,
        );
    }
    x_axis(&painter, plot, range, target, unit);
    let mut line: Vec<egui::Pos2> = vec![pos2(plot.left(), y(0.0))];
    line.extend(points.iter().map(|(v, f)| pos2(x(*v), y(*f))));
    line.push(pos2(plot.right(), y(1.0)));
    painter.add(egui::Shape::line(line, Stroke::new(2.0, p.accent)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(target: Value, probability: f64, interval: Value) -> Value {
        let mut t = target;
        t["probability"] = json!(probability);
        t["interval"] = interval;
        t["of"] = json!(10_000);
        json!({
            "name": "cost", "unit": "USD", "mean": 6_500.0, "half_width": 12.3,
            "percentiles": { "5": 5_000.0, "95": 8_000.0 },
            "target": t,
        })
    }

    /// A target missed one time in five reads as odds, with the percentage and its interval
    /// turned round from the share that met it.
    #[test]
    fn a_target_reads_as_the_chance_of_missing_it() {
        let o = output(json!({ "at_most": 7_000.0 }), 0.8, json!([0.78, 0.82]));
        let (sentence, sure) = finding(&o).unwrap_or_default();
        // Numbers as written with no region set (four-digit numbers are not grouped).
        assert_eq!(
            sentence,
            "About a 1 in 5 chance that cost is more than 7000 USD (20%)."
        );
        assert_eq!(
            sure.as_deref(),
            Some("95% interval of the chance: 18% to 22%.")
        );
        // At least: the chance of falling short; a large chance as a percentage.
        let o = output(json!({ "at_least": 6_000.0 }), 0.4, Value::Null);
        let (sentence, sure) = finding(&o).unwrap_or_default();
        assert_eq!(sentence, "A 60% chance that cost is less than 6000 USD.");
        assert_eq!(
            sure.as_deref(),
            Some("No interval: too few batches of trials for one.")
        );
        // Never missed: the bound of the chance, which is small but not 0%.
        let o = output(json!({ "at_most": 9_000.0 }), 1.0, json!([0.9996, 1.0]));
        let (sentence, sure) = finding(&o).unwrap_or_default();
        assert_eq!(
            sentence,
            "cost was more than 9000 USD in none of the 10000 trials."
        );
        assert_eq!(
            sure.as_deref(),
            Some("The chance is under 0.04%, with 95% confidence.")
        );
    }

    /// Without a target: where 9 trials in 10 fall, and the mean rounded to its interval.
    #[test]
    fn an_output_without_a_target_reads_as_its_range() {
        let o = json!({
            "name": "days", "unit": "", "mean": 41.37, "half_width": 0.123,
            "percentiles": { "5": 35.2, "95": 48.9 },
        });
        let (sentence, sure) = finding(&o).unwrap_or_default();
        assert_eq!(
            sentence,
            "In 9 trials out of 10, days is between 35.2 and 48.9; the mean is 41.37 ± 0.12."
        );
        assert!(sure.is_none());
        assert!(finding(&json!({ "name": "x" })).is_none());
    }

    #[test]
    fn odds_only_where_they_read_naturally() {
        assert_eq!(odds(0.2), Some(5));
        assert_eq!(odds(0.01), Some(100));
        assert_eq!(odds(0.4), None);
        assert_eq!(odds(0.0001), None);
    }

    /// Cells copied from a spreadsheet, with or without the header, become the CSV the engine
    /// reads.
    #[test]
    fn a_pasted_table_becomes_csv() {
        assert_eq!(
            table_text(
                "labour\ttriangular\tmin=80;mode=100;max=150\th\n\nrate\tuniform\tmin=40;max=50\tUSD/h"
            ),
            "name,distribution,parameters,unit\nlabour,triangular,min=80;mode=100;max=150,h\nrate,uniform,min=40;max=50,USD/h\n"
        );
        let csv = "name,distribution,parameters,unit\nx,normal,mean=1;std_dev=2,\n";
        assert_eq!(table_text(csv), csv);
    }
}
