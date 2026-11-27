//! Runs from another tool (M6.0): a table of per-replication results written by any simulation
//! tool, spreadsheet or script that can write CSV, opened with File ▸ Analyse runs from another
//! tool… (or dropped on the window), its columns mapped, and the engine's answer shown the way
//! the Results tab shows a run's.
//!
//! The roles of the columns are guessed by the engine's own commands (`data.warmup` finds a time
//! series by its time column, `data.analyse` the scenario, replication, measure and value
//! columns), and the user can change them, choose the baseline, declare that the scenarios used
//! common random numbers and set a tolerance per measure. The answers then come from
//! `data.analyse`: what the runs found first, how sure the numbers are, the means with their
//! intervals, and each scenario against the baseline with its verdict in words (as the check
//! against observed data words them). A time series gets `data.warmup`'s averaged series instead,
//! with the MSER-5 cut marked.
//!
//! The table is not a model: opening it leaves the model as it is, and Close goes back to it.

use std::collections::BTreeMap;

use egui::RichText;
use serde_json::{Value, json};
use stochcraft_engine::Engine;

use crate::i18n::{t, tf};
use crate::{App, baseline, results, theme};

/// How the table is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// One row per replication, a column per measure.
    Wide,
    /// One row per value: a column naming the measure, a column holding the value.
    Long,
    /// One row per replication and time.
    Series,
}

impl Shape {
    fn label(self) -> &'static str {
        match self {
            Shape::Wide => t("One row per run"),
            Shape::Long => t("One row per value"),
            Shape::Series => t("A time series"),
        }
    }

    /// The roles a column can have in a table of this shape.
    fn roles(self) -> &'static [Role] {
        match self {
            Shape::Wide => &[Role::Scenario, Role::Run, Role::Measure, Role::Unused],
            Shape::Long => &[
                Role::Scenario,
                Role::Run,
                Role::MeasureName,
                Role::Value,
                Role::Unused,
            ],
            Shape::Series => &[
                Role::Scenario,
                Role::Run,
                Role::Time,
                Role::Measure,
                Role::Unused,
            ],
        }
    }
}

/// What a column holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Scenario,
    /// The replication's id.
    Run,
    /// A measure's values (a wide table or a time series).
    Measure,
    /// A long table's measure names.
    MeasureName,
    /// A long table's values.
    Value,
    Time,
    Unused,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Role::Scenario => t("Scenario"),
            Role::Run => t("Run number"),
            Role::Measure => t("Measure"),
            Role::MeasureName => t("Measure name"),
            Role::Value => t("Value"),
            Role::Time => t("Time"),
            Role::Unused => t("Not used"),
        }
    }

    /// At most one column holds it.
    fn single(self) -> bool {
        !matches!(self, Role::Measure | Role::Unused)
    }
}

/// A table of runs from another tool, its mapping and the engine's answer.
pub(crate) struct FromFile {
    /// The file's name, as shown.
    name: String,
    csv: String,
    /// The columns' names, as the engine names them (an empty header is "column n").
    header: Vec<String>,
    shape: Shape,
    /// Each column's role, in the order of `header`.
    roles: Vec<Role>,
    /// The scenario the others are compared with; none for the first.
    baseline: Option<String>,
    /// The user declared that the scenarios used common random numbers.
    paired: bool,
    /// Per measure, a tolerance set in advance (0 for none).
    tolerance: BTreeMap<String, f64>,
    /// The scenarios and measures of the last answer, for the baseline and the tolerances.
    scenarios: Vec<String>,
    measures: Vec<String>,
    /// The answers are shown, not the mapping.
    answers: bool,
    /// The engine's answer for the mapping as it is, or its error; none once the mapping changed.
    answer: Option<Result<Value, String>>,
}

/// Whether `name` is a table of runs rather than a model: by its extension.
pub(crate) fn is_runs_file(name: &str) -> bool {
    matches!(crate::extension(name).as_str(), "csv" | "tsv" | "txt")
}

impl FromFile {
    /// The table with the roles the engine guesses for its columns.
    fn new(engine: &mut Engine, name: String, csv: String, header: Vec<String>) -> Self {
        let mut f = Self {
            name,
            csv,
            roles: vec![Role::Measure; header.len()],
            header,
            shape: Shape::Wide,
            baseline: None,
            paired: false,
            tolerance: BTreeMap::new(),
            scenarios: Vec::new(),
            measures: Vec::new(),
            answers: false,
            answer: None,
        };
        f.guess(engine);
        f.refresh(engine);
        // Straight to the answers when the guess gives some.
        f.answers = matches!(f.answer, Some(Ok(_)));
        f
    }

    fn column(&self, name: &Value) -> Option<usize> {
        let name = name.as_str()?;
        self.header.iter().position(|h| h == name)
    }

    /// The roles as the engine would take them without being told: a time series when
    /// `data.warmup` finds a time and a replication column, otherwise what `data.analyse` reads.
    fn guess(&mut self, engine: &mut Engine) {
        if let Ok(w) = engine.execute("data.warmup", json!({ "csv": self.csv })) {
            self.shape = Shape::Series;
            self.roles = vec![Role::Unused; self.header.len()];
            let c = &w["columns"];
            for m in c["measures"].as_array().into_iter().flatten() {
                self.set(m, Role::Measure);
            }
            self.set(&c["time"], Role::Time);
            self.set(&c["replication"], Role::Run);
            self.set(&c["scenario"], Role::Scenario);
            return;
        }
        if let Ok(a) = engine.execute("data.analyse", json!({ "csv": self.csv })) {
            let c = &a["columns"];
            if a["shape"] == "long" {
                self.shape = Shape::Long;
                self.roles = vec![Role::Unused; self.header.len()];
                self.set(&c["measure"], Role::MeasureName);
                self.set(&c["value"], Role::Value);
            } else {
                for i in c["ignored"].as_array().into_iter().flatten() {
                    self.set(i, Role::Unused);
                }
            }
            self.set(&c["scenario"], Role::Scenario);
            self.set(&c["replication"], Role::Run);
        }
    }

    fn set(&mut self, name: &Value, role: Role) {
        if let Some(i) = self.column(name)
            && let Some(r) = self.roles.get_mut(i)
        {
            *r = role;
        }
    }

    /// The column with `role`, by name.
    fn with(&self, role: Role) -> Option<String> {
        let i = self.roles.iter().position(|r| *r == role)?;
        self.header.get(i).cloned()
    }

    /// The command for the mapping as it is, and its parameters.
    fn request(&self) -> (&'static str, Value) {
        let measures: Vec<&String> = self
            .header
            .iter()
            .zip(&self.roles)
            .filter(|(_, r)| **r == Role::Measure)
            .map(|(h, _)| h)
            .collect();
        if self.shape == Shape::Series {
            return (
                "data.warmup",
                json!({
                    "csv": self.csv,
                    "time": self.with(Role::Time),
                    "replication": self.with(Role::Run),
                    "scenario": self.with(Role::Scenario),
                    "measures": measures,
                }),
            );
        }
        // Only the tolerances of measures there are, and only those set.
        let tolerance: BTreeMap<&String, f64> = self
            .tolerance
            .iter()
            .filter(|(m, v)| self.measures.contains(m) && v.is_finite() && **v > 0.0)
            .map(|(m, v)| (m, *v))
            .collect();
        let baseline = self.baseline.clone().filter(|b| self.scenarios.contains(b));
        let mut p = json!({
            "csv": self.csv,
            "scenario": self.with(Role::Scenario),
            "replication": self.with(Role::Run),
            "baseline": baseline,
            "paired": self.paired,
            "tolerance": tolerance,
        });
        if self.shape == Shape::Long {
            p["measure"] = json!(self.with(Role::MeasureName));
            p["value"] = json!(self.with(Role::Value));
        } else {
            p["measures"] = json!(measures);
        }
        ("data.analyse", p)
    }

    /// Asks the engine for the answer to the mapping as it is.
    fn refresh(&mut self, engine: &mut Engine) {
        let (command, params) = self.request();
        let answer = engine.execute(command, params).map_err(|e| e.message);
        if let Ok(a) = &answer
            && command == "data.analyse"
        {
            let names = |v: &Value| -> Vec<String> {
                v.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            };
            self.scenarios = names(&a["scenarios"]);
            self.measures = names(&a["measures"]);
        }
        self.answer = Some(answer);
    }

    fn set_shape(&mut self, shape: Shape) {
        self.shape = shape;
        let allowed = shape.roles();
        for r in &mut self.roles {
            if !allowed.contains(r) {
                *r = Role::Unused;
            }
        }
        self.answer = None;
    }

    fn set_role(&mut self, column: usize, role: Role) {
        if role.single() {
            for r in &mut self.roles {
                if *r == role {
                    *r = Role::Unused;
                }
            }
        }
        if let Some(r) = self.roles.get_mut(column) {
            *r = role;
        }
        self.answer = None;
    }

    /// The mapping step: the shape, each column's role, the baseline, pairing and tolerances.
    fn mapping(&mut self, ui: &mut egui::Ui, numbers: &craft_ui::localized::LocaleNumbers) {
        ui.label(theme::strong(t("What each column holds")).size(15.0));
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("The roles are guessed from the column names; change any that are wrong."),
            ))
            .wrap(),
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for s in [Shape::Wide, Shape::Long, Shape::Series] {
                if ui.radio(self.shape == s, s.label()).clicked() && self.shape != s {
                    self.set_shape(s);
                }
            }
        });
        ui.add_space(6.0);
        let mut changed = None;
        egui::Grid::new("from-file-columns")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                for (i, (name, role)) in self.header.iter().zip(&self.roles).enumerate() {
                    ui.label(name.as_str());
                    let mut chosen = *role;
                    craft_ui::ComboBox::from_id_salt(format!("from-file-role-{i}"))
                        .selected_text(role.label())
                        .width(170.0)
                        .show_ui(ui, |ui| {
                            for r in self.shape.roles() {
                                if ui.selectable_label(chosen == *r, r.label()).clicked() {
                                    chosen = *r;
                                }
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name.as_str())
                        });
                    if chosen != *role {
                        changed = Some((i, chosen));
                    }
                    ui.end_row();
                }
            });
        if let Some((i, role)) = changed {
            self.set_role(i, role);
        }
        if self.shape != Shape::Series && self.scenarios.len() > 1 {
            ui.add_space(10.0);
            let shown = self
                .baseline
                .clone()
                .filter(|b| self.scenarios.contains(b))
                .or_else(|| self.scenarios.first().cloned())
                .unwrap_or_default();
            let mut chosen = None;
            ui.horizontal(|ui| {
                ui.label(t("Baseline"));
                craft_ui::ComboBox::from_id_salt("from-file-baseline")
                    .selected_text(shown.as_str())
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for s in &self.scenarios {
                            if ui.selectable_label(*s == shown, s.as_str()).clicked() {
                                chosen = Some(s.clone());
                            }
                        }
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Baseline"))
                    });
            });
            if let Some(b) = chosen.filter(|b| *b != shown) {
                self.baseline = Some(b);
                self.answer = None;
            }
            ui.add_space(6.0);
            if ui
                .checkbox(
                    &mut self.paired,
                    t("These scenarios used common random numbers (paired)"),
                )
                .changed()
            {
                self.answer = None;
            }
            ui.add(
                egui::Label::new(theme::weak(
                    ui,
                    t("StochCraft cannot check this from the numbers: tick it only if the tool that made the runs gave run i of each scenario the same random numbers. Runs are paired by their run number."),
                ))
                .wrap(),
            );
            if !self.measures.is_empty() {
                ui.add_space(10.0);
                ui.label(theme::strong(t("Tolerance (0 for none)")));
                egui::Grid::new("from-file-tolerances")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for m in &self.measures {
                            ui.label(m.as_str());
                            let before = self.tolerance.get(m).copied().unwrap_or(0.0);
                            let mut v = before;
                            crate::decide::number(
                                ui,
                                numbers,
                                &format!("from-file-tolerance-{m}"),
                                m,
                                &mut v,
                                90.0,
                            );
                            if v != before {
                                self.tolerance.insert(m.clone(), v.max(0.0));
                                self.answer = None;
                            }
                            ui.end_row();
                        }
                    });
            }
        }
        ui.add_space(10.0);
        if let Some(Err(e)) = &self.answer {
            theme::error_message(ui, e.clone());
            ui.add_space(6.0);
        }
        let ok = matches!(self.answer, Some(Ok(_)));
        if ui
            .add_enabled(ok, theme::primary(ui, t("Show the answers")))
            .clicked()
        {
            self.answers = true;
        }
    }
}

impl App {
    /// Opens a table of runs from another tool (`name` is its path, or its name where there are
    /// no paths) in the work area, over the model, which stays as it is.
    pub fn open_runs(&mut self, name: &str, bytes: &[u8]) {
        let file = crate::file_name(name).to_owned();
        let csv = String::from_utf8_lossy(bytes).into_owned();
        let header: Vec<String> = match stochcraft_engine::table::read(&csv) {
            Ok(table) => table
                .records
                .first()
                .map(|r| {
                    r.cells
                        .iter()
                        .enumerate()
                        .map(|(i, h)| {
                            if h.is_empty() {
                                format!("column {}", i + 1)
                            } else {
                                h.clone()
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
            Err(e) => {
                self.report_error(format!("{file}: {e}"));
                return;
            }
        };
        if header.is_empty() {
            // The engine's words for an empty table.
            if let Err(e) = self.engine.execute("data.analyse", json!({ "csv": csv })) {
                self.report_error(format!("{file}: {}", e.message));
            }
            return;
        }
        self.from_file = Some(FromFile::new(&mut self.engine, file.clone(), csv, header));
        self.info(tf("Opened {file}", &[("file", &file)]));
    }

    /// The work area while a table of runs is open: its mapping or its answers.
    pub(crate) fn runs_view(&mut self, ui: &mut egui::Ui) {
        let Some(mut f) = self.from_file.take() else {
            return;
        };
        let mut close = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(t("Runs from another tool"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = ui.add(theme::secondary(ui, t("Close"))).clicked();
                        if f.answers
                            && ui
                                .add(theme::secondary(ui, t("Change the columns")))
                                .clicked()
                        {
                            f.answers = false;
                        }
                    });
                });
                ui.label(theme::weak(ui, f.name.clone()));
                ui.add_space(12.0);
                let answer = match &f.answer {
                    Some(Ok(a)) if f.answers => Some(a.clone()),
                    _ => None,
                };
                match answer {
                    Some(a) if f.shape == Shape::Series => warmup_answers(ui, &a),
                    Some(a) => analysis_answers(ui, &a, &f.name),
                    None => f.mapping(ui, &self.numbers),
                }
            });
        if f.answer.is_none() {
            f.refresh(&mut self.engine);
            ui.ctx().request_repaint();
        }
        if !close {
            self.from_file = Some(f);
        }
    }
}

/// The text of a number list's entries.
fn list(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

/// The scenario's name as shown, or the file's for a table without scenarios.
fn scenario_name<'a>(v: &'a Value, file: &'a str) -> &'a str {
    v.as_str().unwrap_or(file)
}

/// A mean as the tables show it beside its interval: rounded to what the interval supports.
fn mean_text(r: &Value) -> String {
    match (r["mean"].as_f64(), r["half_width"].as_f64()) {
        (Some(m), Some(h)) => results::rounded(m, h).0,
        (Some(m), None) => results::grouped(m),
        _ => "–".to_owned(),
    }
}

/// The result of `measure` in `scenario`.
fn result_of<'a>(a: &'a Value, scenario: &Value, measure: &Value) -> Option<&'a Value> {
    list(&a["results"]).find(|r| r["scenario"] == *scenario && r["measure"] == *measure)
}

/// A difference with its interval as the comparison table writes it ("−1.6 ± 0.7").
fn change_text(c: &Value) -> String {
    match (c["difference"].as_f64(), c["half_width"].as_f64()) {
        (Some(d), Some(h)) => {
            let (m, h) = results::rounded(d, h);
            format!("{} ± {h}", baseline::signed(m, d))
        }
        (Some(d), None) => baseline::signed(results::grouped(d), d),
        _ => "–".to_owned(),
    }
}

/// A comparison's verdict in words (the check against observed data's), and whether it found
/// something: a difference, or agreement within the tolerance.
fn verdict(c: &Value) -> (String, bool) {
    match c["verdict"].as_str() {
        Some("differs") => (t("Differs").to_owned(), true),
        Some("agrees") => {
            let tolerance = c["tolerance"]
                .as_f64()
                .map_or_else(String::new, results::grouped);
            (
                tf("Agrees within ±{tolerance}", &[("tolerance", &tolerance)]),
                true,
            )
        }
        Some("no_difference_detected") => (t("No difference detected").to_owned(), false),
        _ => (t("Inconclusive at current precision").to_owned(), false),
    }
}

/// What the runs found, in sentences: each difference and agreement, then what the rest did,
/// then the measures whose runs split into two outcomes.
fn findings(a: &Value, file: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut others = 0;
    for c in list(&a["comparisons"]) {
        let measure = c["measure"].as_str().unwrap_or("");
        let block = scenario_name(&c["scenario"], file);
        let before = result_of(a, &c["baseline"], &c["measure"]).map(mean_text);
        let after = result_of(a, &c["scenario"], &c["measure"]).map(mean_text);
        match c["verdict"].as_str() {
            Some("differs") => {
                let text = if c["difference"].as_f64().is_some_and(|d| d < 0.0) {
                    "{measure} ({block}) fell from {before} to {after}, by {change}: more than chance would explain."
                } else {
                    "{measure} ({block}) rose from {before} to {after}, by {change}: more than chance would explain."
                };
                out.push(tf(
                    text,
                    &[
                        ("measure", measure),
                        ("block", block),
                        ("before", before.as_deref().unwrap_or("–")),
                        ("after", after.as_deref().unwrap_or("–")),
                        ("change", &change_text(c)),
                    ],
                ));
            }
            Some("agrees") => {
                let tolerance = c["tolerance"]
                    .as_f64()
                    .map_or_else(String::new, results::grouped);
                out.push(tf(
                    "{measure} ({block}) agrees with {baseline} within ±{tolerance}.",
                    &[
                        ("measure", measure),
                        ("block", block),
                        ("baseline", scenario_name(&c["baseline"], file)),
                        ("tolerance", &tolerance),
                    ],
                ));
            }
            _ => others += 1,
        }
    }
    if others > 0 {
        out.push(if out.is_empty() {
            t("None of the key measures moved by more than chance: the change may do little, or more runs may show it.").to_owned()
        } else {
            t("The other comparisons show no difference beyond chance, or are inconclusive: see the table below.").to_owned()
        });
    }
    for r in list(&a["results"]).filter(|r| !r["split"].is_null()) {
        out.push(tf(
            "{result} ({element}) split into two outcomes: see the table below.",
            &[
                ("result", r["measure"].as_str().unwrap_or("")),
                ("element", scenario_name(&r["scenario"], file)),
            ],
        ));
    }
    out
}

/// "How sure are these numbers?" for the table's results: the widest measure and about how many
/// runs would settle it, measures whose runs all gave the same value, and how many are settled.
fn trust(ui: &mut egui::Ui, p: &theme::Palette, a: &Value, file: &str) {
    let all: Vec<&Value> = list(&a["results"]).collect();
    if all.is_empty() {
        return;
    }
    let status = |r: &Value| r["precision"]["status"].as_str().unwrap_or("").to_owned();
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::strong(t("How sure are these numbers?")));
            ui.add_space(4.0);
            if all.iter().any(|r| r["n"].as_u64().is_some_and(|n| n < 2)) {
                results::trust_row(
                    ui,
                    p,
                    false,
                    t("One run gives no interval, so these numbers cannot be told from chance."),
                    None,
                );
            }
            let widest = all
                .iter()
                .filter(|r| status(r) == "wide")
                .max_by(|x, y| {
                    let s = |r: &Value| r["precision"]["score"].as_f64().unwrap_or(0.0);
                    s(x).total_cmp(&s(y))
                });
            if let Some(r) = widest {
                let (mean, hw) = match (r["mean"].as_f64(), r["half_width"].as_f64()) {
                    (Some(m), Some(h)) => results::rounded(m, h),
                    _ => ("–".to_owned(), "–".to_owned()),
                };
                // The score is the half-width over ± 5% of the mean.
                let score = r["precision"]["score"].as_f64().unwrap_or(0.0);
                let relative = results::grouped((score * 5.0).round());
                let value = format!("{mean} ± {hw}, ± {relative}%");
                let n = r["n"].as_u64().unwrap_or(0).to_string();
                let mut text = tf(
                    "{measure} ({block}) is known only to {value} after {n} runs.",
                    &[
                        ("measure", r["measure"].as_str().unwrap_or("")),
                        ("block", scenario_name(&r["scenario"], file)),
                        ("value", &value),
                        ("n", &n),
                    ],
                );
                if let Some(needed) = r["precision"]["runs_needed"].as_u64() {
                    text.push(' ');
                    text.push_str(&tf(
                        "About {n} runs would pin it to ± 5%.",
                        &[("n", &needed.to_string())],
                    ));
                }
                results::trust_row(ui, p, false, &text, None);
            }
            for r in all.iter().filter(|r| status(r) == "no_spread") {
                let text = tf(
                    "{measure} ({block}): every run gave the same value, which is not proof that it never varies.",
                    &[
                        ("measure", r["measure"].as_str().unwrap_or("")),
                        ("block", scenario_name(&r["scenario"], file)),
                    ],
                );
                results::trust_row(ui, p, false, &text, None);
            }
            let judged = all
                .iter()
                .filter(|r| matches!(status(r).as_str(), "settled" | "wide"))
                .count();
            let settled = all.iter().filter(|r| status(r) == "settled").count();
            if settled > 0 {
                let text = tf(
                    "{k} of {m} measures are settled: each is known to ± 5% or better.",
                    &[("k", &settled.to_string()), ("m", &judged.to_string())],
                );
                results::trust_row(ui, p, true, &text, None);
            }
        });
}

/// The means with their intervals, a table per scenario, each split into two outcomes said in
/// words under its row.
fn means(ui: &mut egui::Ui, p: &theme::Palette, a: &Value) {
    ui.label(theme::strong(t("Means, rounded to what the interval supports")).size(15.0));
    ui.add_space(4.0);
    let scenarios: Vec<&Value> = list(&a["scenarios"]).collect();
    for s in scenarios {
        if let Some(name) = s.as_str() {
            ui.add_space(4.0);
            ui.label(theme::strong(name));
        }
        results::row(
            ui,
            p,
            None,
            [t("Measure"), t("Mean"), t("± 95% CI"), "", ""],
        );
        for (i, r) in list(&a["results"])
            .filter(|r| r["scenario"] == *s)
            .enumerate()
        {
            let (mean, hw) = match (r["mean"].as_f64(), r["half_width"].as_f64()) {
                (Some(m), Some(h)) => results::rounded(m, h),
                (Some(m), None) => (results::grouped(m), "–".to_owned()),
                _ => ("–".to_owned(), "–".to_owned()),
            };
            let n = r["n"].as_u64().unwrap_or(0);
            let missing = r["missing"].as_u64().unwrap_or(0);
            let n_of = if missing > 0 {
                tf(
                    "n = {k} of {n}",
                    &[("k", &n.to_string()), ("n", &(n + missing).to_string())],
                )
            } else {
                String::new()
            };
            let measure = r["measure"].as_str().unwrap_or("");
            results::row(ui, p, Some(i % 2 == 0), [measure, &mean, &hw, "", &n_of]);
            if let Some(text) = results::split_sentence(r, 1.0, "") {
                ui.add(egui::Label::new(theme::weak(ui, text)).wrap());
            }
        }
    }
}

/// Each scenario against the baseline, measure by measure: the two means, the difference with
/// its interval, its mark against a zero line, and the verdict in words.
fn comparisons(ui: &mut egui::Ui, p: &theme::Palette, a: &Value, file: &str) {
    let rows: Vec<&Value> = list(&a["comparisons"]).collect();
    if rows.is_empty() {
        return;
    }
    let paired = a["paired"] == true;
    ui.label(
        theme::strong(tf(
            "Each scenario against {baseline}",
            &[("baseline", scenario_name(&a["baseline"], file))],
        ))
        .size(15.0),
    );
    ui.add_space(4.0);
    let wide = ui.available_width() >= baseline::COLUMNS[5].1;
    // The change as a share of the baseline's mean, for the mark.
    let relative = |c: &Value| {
        let base = result_of(a, &c["baseline"], &c["measure"])?["mean"].as_f64()?;
        let d = c["difference"].as_f64()?;
        (base != 0.0).then(|| {
            (
                d / base.abs(),
                c["half_width"].as_f64().unwrap_or(0.0) / base.abs(),
            )
        })
    };
    let (lo, hi) = rows
        .iter()
        .copied()
        .filter_map(&relative)
        .fold((0.0_f64, 0.0_f64), |(lo, hi), (d, hw)| {
            (lo.min(d - hw), hi.max(d + hw))
        });
    let scale = (-baseline::nice(-lo), baseline::nice(hi));
    let difference = if paired {
        t("Difference (paired, 95%)")
    } else {
        t("Difference (95%)")
    };
    let header = [
        t("Measure"),
        t("Baseline"),
        t("Scenario"),
        difference,
        if wide { t("Change from baseline") } else { "" },
        t("Verdict"),
    ];
    let _ = baseline::row_frame(ui, p, None, wide, |ui, cell| {
        for (c, text) in header.iter().enumerate() {
            if !text.is_empty() {
                cell(ui, c, RichText::new(*text).size(12.0).color(p.text_weak));
            }
        }
    });
    for (i, c) in rows.iter().enumerate() {
        let measure = tf(
            "{block}: {measure}",
            &[
                ("block", scenario_name(&c["scenario"], file)),
                ("measure", c["measure"].as_str().unwrap_or("")),
            ],
        );
        let before =
            result_of(a, &c["baseline"], &c["measure"]).map_or_else(|| "–".into(), mean_text);
        let after =
            result_of(a, &c["scenario"], &c["measure"]).map_or_else(|| "–".into(), mean_text);
        let change = change_text(c);
        let (words, found) = verdict(c);
        let rect = baseline::row_frame(ui, p, Some(i % 2 == 0), wide, |ui, cell| {
            cell(ui, 0, RichText::new(measure.as_str()));
            cell(ui, 1, RichText::new(before.as_str()).color(p.text_weak));
            cell(ui, 2, theme::strong(after.as_str()));
            cell(ui, 3, RichText::new(change.as_str()));
            let color = if found { p.accent } else { p.text_weak };
            cell(ui, 5, RichText::new(words.as_str()).color(color));
        });
        let at = egui::pos2(
            rect.left() + baseline::column(5, wide).0 - 10.0,
            rect.center().y,
        );
        if found {
            theme::paint_check_mark(ui.painter(), at, 5.5, p);
        } else if c["verdict"] == "no_difference_detected" {
            baseline::paint_about_equal(ui.painter(), at, p);
        }
        if wide {
            let mark = egui::Rect::from_x_y_ranges(
                rect.left() + baseline::COLUMNS[4].0..=rect.left() + baseline::COLUMNS[4].1,
                rect.y_range(),
            );
            baseline::paint_mark(ui, p, mark, relative(c), scale, &change);
        }
    }
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(theme::weak(
            ui,
            t("Differs: the 95% interval leaves out zero, or lies wholly outside the tolerance. Agrees: it lies wholly within the tolerance. No difference detected: it takes in zero, which is not evidence of agreement."),
        ))
        .wrap(),
    );
}

/// The answers of `data.analyse`: what the runs found, how sure the numbers are, the means and
/// the comparisons with the baseline.
fn analysis_answers(ui: &mut egui::Ui, a: &Value, file: &str) {
    let p = theme::palette(ui.ctx());
    let found = findings(a, file);
    if !found.is_empty() {
        ui.label(theme::strong(t("What the run found")));
        ui.add_space(2.0);
        for text in found {
            ui.add(egui::Label::new(text).wrap());
        }
        ui.add_space(10.0);
    }
    if a["paired"] == true {
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("Compared in pairs, as declared: StochCraft cannot check that the scenarios used common random numbers; if they did not, these intervals are too narrow."),
            ))
            .wrap(),
        );
        ui.add_space(6.0);
    } else if list(&a["notes"]).next().is_some() {
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("Every scenario has the same run numbers: if the runs used common random numbers, tick paired for a much tighter comparison."),
            ))
            .wrap(),
        );
        ui.add_space(6.0);
    }
    trust(ui, &p, a, file);
    ui.add_space(12.0);
    means(ui, &p, a);
    ui.add_space(14.0);
    comparisons(ui, &p, a, file);
}

/// The answers of `data.warmup`: per scenario and measure, the series averaged over the runs with
/// the MSER-5 cut marked, and the suggested warm-up in words.
fn warmup_answers(ui: &mut egui::Ui, a: &Value) {
    let p = theme::palette(ui.ctx());
    ui.label(theme::strong(t("Warm-up")).size(15.0));
    ui.add_space(4.0);
    for s in list(&a["series"]) {
        let measure = s["measure"].as_str().unwrap_or("");
        let title = match s["scenario"].as_str() {
            Some(name) => format!("{measure} ({name})"),
            None => measure.to_owned(),
        };
        ui.add_space(6.0);
        ui.label(theme::strong(title));
        let times: Vec<f64> = list(&s["times"]).filter_map(Value::as_f64).collect();
        let average: Vec<f64> = list(&s["average"]).filter_map(Value::as_f64).collect();
        let w = &s["warmup"];
        let cut = w["time"].as_f64().filter(|_| w["status"] == "found");
        series_plot(ui, &p, &times, &average, cut);
        let text = match w["status"].as_str() {
            Some("found") => tf(
                "MSER-5 suggests a warm-up of {time}: leave out the first {k} times.",
                &[
                    ("time", &cut.map_or_else(String::new, results::grouped)),
                    ("k", &w["observations"].as_u64().unwrap_or(0).to_string()),
                ],
            ),
            Some("too_short") => {
                t("The series is too short to show where the start-up ends: record longer runs.")
                    .to_owned()
            }
            _ => t("Fewer than 20 times: too few to suggest a warm-up.").to_owned(),
        };
        ui.add(egui::Label::new(text).wrap());
    }
}

/// A line of the averaged series over time, its axes' ends labelled, the part before the
/// warm-up cut shaded and the cut marked.
fn series_plot(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    times: &[f64],
    values: &[f64],
    cut: Option<f64>,
) {
    let width = ui.available_width().min(720.0);
    let (outer, response) = ui.allocate_exact_size(egui::vec2(width, 200.0), egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Image,
            true,
            t("Averaged over the runs at each time, with the warm-up cut marked"),
        )
    });
    let painter = ui.painter_at(outer);
    let plot = egui::Rect::from_min_max(
        outer.min + egui::vec2(56.0, 8.0),
        outer.max - egui::vec2(8.0, 22.0),
    );
    painter.rect_stroke(
        plot,
        2.0,
        egui::Stroke::new(1.0, p.border),
        egui::StrokeKind::Inside,
    );
    let (Some(t0), Some(t1)) = (times.first().copied(), times.last().copied()) else {
        return;
    };
    let (lo, hi) = values
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(*v), hi.max(*v))
        });
    if !(lo.is_finite() && hi.is_finite()) {
        return;
    }
    let (lo, hi) = if hi > lo {
        (lo, hi)
    } else {
        (lo - 1.0, hi + 1.0)
    };
    let x = |v: f64| {
        if t1 > t0 {
            plot.left() + ((v - t0) / (t1 - t0)) as f32 * plot.width()
        } else {
            plot.center().x
        }
    };
    let y = |v: f64| plot.bottom() - ((v - lo) / (hi - lo)) as f32 * plot.height();
    if let Some(c) = cut {
        let shade = egui::Rect::from_x_y_ranges(plot.left()..=x(c), plot.y_range());
        painter.rect_filled(shade, 0.0, p.stripe);
    }
    let points: Vec<egui::Pos2> = times
        .iter()
        .zip(values)
        .map(|(tm, v)| egui::pos2(x(*tm), y(*v)))
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, p.chart)));
    if let Some(c) = cut {
        painter.line_segment(
            [
                egui::pos2(x(c), plot.top()),
                egui::pos2(x(c), plot.bottom()),
            ],
            egui::Stroke::new(2.0, p.accent),
        );
        painter.text(
            egui::pos2(x(c) + 4.0, plot.top() + 2.0),
            egui::Align2::LEFT_TOP,
            "MSER-5",
            egui::FontId::proportional(11.0),
            p.accent,
        );
    }
    let font = egui::FontId::proportional(11.0);
    for (v, at, align) in [
        (
            hi,
            egui::pos2(plot.left() - 4.0, plot.top()),
            egui::Align2::RIGHT_TOP,
        ),
        (
            lo,
            egui::pos2(plot.left() - 4.0, plot.bottom()),
            egui::Align2::RIGHT_BOTTOM,
        ),
        (
            t0,
            egui::pos2(plot.left(), plot.bottom() + 4.0),
            egui::Align2::LEFT_TOP,
        ),
        (
            t1,
            egui::pos2(plot.right(), plot.bottom() + 4.0),
            egui::Align2::RIGHT_TOP,
        ),
    ] {
        painter.text(at, align, results::grouped(v), font.clone(), p.text_weak);
    }
}
