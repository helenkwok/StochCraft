//! Experiments for decisions (M2.8) in the window:
//!
//! - in the model overview, an event that ends each replication early (a terminating experiment);
//! - in a timed block's inspector, its time fitted from a three-point estimate, which is kept as
//!   the time's pedigree;
//! - in the results, when the stop was met, and the distribution of any one result across the
//!   replications: percentiles, the empirical distribution with its confidence band, and the
//!   probability of meeting a target;
//! - the Analysis view: a sweep of one parameter, the sensitivity of a result to every parameter,
//!   and sizing the busiest resource until a target is met often enough.
//!
//! Each is one engine command (`experiment.set`, `estimate.set`, `results.distribution`,
//! `experiment.sweep`, `experiment.sensitivity`, `experiment.size`), so the window, scripts and
//! agents get the same answers. Sweeps, sensitivity and sizing run at once, on this thread.

use craft_ui::localized::NumberField;
use egui::{Align2, FontId, Rect, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::editor::section;
use crate::i18n::{t, tf, tnf};
use crate::results::{grouped, metric_label, unit_label};
use crate::{App, theme};

/// Whether a target is a floor or a ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Bound {
    #[default]
    None,
    AtLeast,
    AtMost,
}

impl Bound {
    fn key(self) -> Option<&'static str> {
        match self {
            Bound::None => None,
            Bound::AtLeast => Some("at_least"),
            Bound::AtMost => Some("at_most"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Bound::None => t("No target"),
            Bound::AtLeast => t("At least"),
            Bound::AtMost => t("At most"),
        }
    }
}

/// A three-point estimate being typed in, for one block.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EstimateForm {
    id: String,
    family: &'static str,
    min: f64,
    mode: f64,
    /// The upper value, a maximum or (when `p95`) the 95th percentile.
    upper: f64,
    p95: bool,
    source: String,
}

/// What the M2.8 screens remember between frames: the choices made, and the last answers.
#[derive(Default)]
pub(crate) struct Decide {
    /// The result whose distribution the results show: element (empty for the stop time) and
    /// metric.
    dist: Option<(String, String)>,
    dist_bound: Bound,
    dist_target: f64,
    /// The last answer, with the parameters it is for.
    dist_answer: Option<(Value, Option<Value>)>,

    sweep_param: Option<String>,
    sweep_values: String,
    sweep_out: Option<String>,
    sweep_answer: Option<Answer>,

    sens_out: Option<String>,
    sens_change: f64,
    sens_answer: Option<Answer>,

    size_out: Option<String>,
    size_bound: Bound,
    size_target: f64,
    size_confidence: f64,
    size_answer: Option<Answer>,

    estimate: Option<EstimateForm>,
}

/// An analysis's result, with the model (by digest) and the request it was made for.
struct Answer {
    digest: String,
    request: Value,
    result: Value,
}

const FAMILIES: [&str; 4] = ["triangular", "pert", "lognormal", "weibull"];

fn family_label(f: &str) -> &'static str {
    match f {
        "triangular" => t("Triangular"),
        "pert" => t("PERT"),
        "lognormal" => t("Log-normal"),
        "weibull" => t("Weibull"),
        _ => "",
    }
}

/// An element's display name.
fn name_of(model: &Value, id: &str) -> String {
    model["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(model["resources"].as_array().into_iter().flatten())
        .find(|e| e["id"] == id)
        .and_then(|e| e["name"].as_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(id)
        .to_owned()
}

fn kind_of(model: &Value, id: &str) -> String {
    if model["resources"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|r| r["id"] == id)
    {
        return "resource".into();
    }
    model["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|e| e["id"] == id)
        .and_then(|e| e["kind"].as_str())
        .unwrap_or("")
        .to_owned()
}

/// The metrics worth asking about for an element of `kind`, when no run has listed them.
fn metrics_for(kind: &str) -> &'static [&'static str] {
    match kind {
        "server" | "shared_task" => &["mean_wait", "mean_queue_length", "utilisation", "left"],
        "delay" | "batch" | "unbatch" => &["mean_contents", "left"],
        "sink" => &["mean_time_in_system", "entered"],
        "resource" => &["utilisation"],
        "exclusive_gateway" => &["left", "lost"],
        _ => &["left"],
    }
}

/// Every result that can be asked about, as `element/metric` keys with their labels: from the
/// last run's values where there is one, otherwise by kind.
fn outputs(model: &Value, results: Option<&Value>) -> Vec<(String, String)> {
    let ids = model["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(model["resources"].as_array().into_iter().flatten())
        .filter_map(|e| e["id"].as_str());
    let mut out = Vec::new();
    for id in ids {
        let kind = kind_of(model, id);
        let listed: Vec<String> = results
            .and_then(|r| r["values"][id].as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_else(|| metrics_for(&kind).iter().map(|m| (*m).to_owned()).collect());
        for m in listed {
            let label = format!("{} · {}", name_of(model, id), metric_label(&m, &kind));
            out.push((format!("{id}/{m}"), label));
        }
    }
    out
}

fn split(key: &str) -> (String, String) {
    key.split_once('/')
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
        .unwrap_or_default()
}

/// The parameters a sweep or a sensitivity analysis can move, as `id/field` keys with labels.
fn parameters(model: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in model["elements"].as_array().into_iter().flatten() {
        let id = e["id"].as_str().unwrap_or("");
        let block = name_of(model, id);
        let what: &[(&str, &str)] = match e["kind"].as_str().unwrap_or("") {
            "server" if e["queue_capacity"].is_number() => &[
                ("capacity", "{block}: units"),
                ("mean", "{block}: mean service time"),
                ("queue_capacity", "{block}: waiting room"),
            ],
            "server" => &[
                ("capacity", "{block}: units"),
                ("mean", "{block}: mean service time"),
            ],
            "shared_task" => &[("mean", "{block}: mean service time")],
            "source" => &[("mean", "{block}: mean time between arrivals")],
            "delay" => &[("mean", "{block}: mean duration")],
            _ => &[],
        };
        for (field, label) in what {
            out.push((format!("{id}/{field}"), tf(label, &[("block", &block)])));
        }
    }
    for r in model["resources"].as_array().into_iter().flatten() {
        let id = r["id"].as_str().unwrap_or("");
        out.push((
            format!("{id}/quantity"),
            tf("{block}: units", &[("block", &name_of(model, id))]),
        ));
    }
    out
}

/// A choice from `options` (key, label), naming itself `label` for assistive technology.
fn choose(
    ui: &mut egui::Ui,
    salt: &str,
    label: &str,
    options: &[(String, String)],
    value: &mut Option<String>,
    width: f32,
) {
    let shown = value
        .as_ref()
        .and_then(|v| options.iter().find(|(k, _)| k == v))
        .map_or_else(|| t("Choose…").to_owned(), |(_, l)| l.clone());
    craft_ui::ComboBox::from_id_salt(salt)
        .selected_text(shown)
        .width(width)
        .show_ui(ui, |ui| {
            for (k, l) in options {
                if ui
                    .selectable_label(value.as_deref() == Some(k), l)
                    .clicked()
                {
                    *value = Some(k.clone());
                }
            }
        })
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label));
}

fn bound_choice(ui: &mut egui::Ui, salt: &str, label: &str, value: &mut Bound, with_none: bool) {
    let options: &[Bound] = if with_none {
        &[Bound::None, Bound::AtLeast, Bound::AtMost]
    } else {
        &[Bound::AtLeast, Bound::AtMost]
    };
    craft_ui::ComboBox::from_id_salt(salt)
        .selected_text(value.label())
        .width(110.0)
        .show_ui(ui, |ui| {
            for b in options {
                if ui.selectable_label(value == b, b.label()).clicked() {
                    *value = *b;
                }
            }
        })
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label));
}

/// A number field named `label`.
fn number(
    ui: &mut egui::Ui,
    numbers: &craft_ui::localized::LocaleNumbers,
    salt: &str,
    label: &str,
    value: &mut f64,
    width: f32,
) -> egui::Response {
    let r = ui.add(
        NumberField::new(value, numbers)
            .id_salt(salt)
            .decimals(0..=6)
            .desired_width(width),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
    r
}

fn row_label(ui: &mut egui::Ui, text: &str) {
    crate::form::label_cell(ui, text, 26.0);
}

/// A percentage, as the results write shares.
fn percent(p: f64) -> String {
    format!("{}%", grouped((p * 1000.0).round() / 10.0))
}

/// "mean ± half width".
fn mean_pm(v: &Value) -> String {
    let mean = v["mean"].as_f64().map_or_else(|| "–".into(), grouped);
    match v["half_width"].as_f64() {
        Some(h) if h.is_finite() => format!("{mean} ± {}", grouped(h)),
        _ => mean,
    }
}

/// Units by resource: "Desk 2, Oven 6".
fn units_text(model: &Value, units: &Value) -> String {
    units
        .as_object()
        .into_iter()
        .flatten()
        .map(|(id, n)| {
            format!(
                "{} {}",
                name_of(model, id),
                n.as_f64().map_or_else(String::new, grouped)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn grid(ui: &mut egui::Ui, id: &str, columns: usize, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(columns)
        .striped(true)
        .spacing(vec2(18.0, 6.0))
        .show(ui, add);
}

impl App {
    // -----------------------------------------------------------------------------------------
    // The model overview: an event that ends each replication early.

    /// The experiment's stop condition: never (the run lasts its duration), when enough entities
    /// have left a block, or when a task empties.
    pub(crate) fn stop_row(&mut self, ui: &mut egui::Ui, model: &Value) {
        let stop = model["experiment"]["stop"].clone();
        let when = stop["when"].as_str().unwrap_or("");
        let kinds = [
            ("", t("Never: it runs for its duration")),
            ("count", t("When enough have left a block")),
            ("empty", t("When a task empties")),
        ];
        let mut chosen = when.to_owned();
        ui.horizontal(|ui| {
            row_label(ui, t("Ends early"));
            craft_ui::ComboBox::from_id_salt("x/stop")
                .selected_text(kinds.iter().find(|k| k.0 == when).map_or("", |k| k.1))
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (k, l) in kinds {
                        if ui.selectable_label(when == k, l).clicked() {
                            chosen = k.to_owned();
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Ends early"))
                });
        });
        let blocks: Vec<(String, String)> = model["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| match chosen.as_str() {
                "empty" => matches!(e["kind"].as_str(), Some("server" | "shared_task" | "delay")),
                _ => !matches!(e["kind"].as_str(), Some("source")),
            })
            .filter_map(|e| e["id"].as_str())
            .map(|id| (id.to_owned(), name_of(model, id)))
            .collect();
        if chosen != when {
            let first = |kind: &str| {
                model["elements"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|e| e["kind"] == kind)
                    .and_then(|e| e["id"].as_str())
                    .map(str::to_owned)
            };
            let element = stop["element"]
                .as_str()
                .filter(|e| blocks.iter().any(|(id, _)| id == e))
                .map(str::to_owned)
                .or_else(|| first(if chosen == "empty" { "server" } else { "sink" }))
                .or_else(|| blocks.first().map(|b| b.0.clone()));
            let spec = match (chosen.as_str(), element) {
                ("count", Some(e)) => json!({ "when": "count", "element": e, "count": 100 }),
                ("empty", Some(e)) => json!({ "when": "empty", "element": e }),
                _ => Value::Null,
            };
            self.execute("experiment.set", json!({ "stop": spec }));
            return;
        }
        if when.is_empty() {
            return;
        }
        let mut element = stop["element"].as_str().map(str::to_owned);
        let mut count = stop["count"].as_f64().unwrap_or(0.0);
        let before = (element.clone(), count);
        ui.horizontal(|ui| {
            row_label(ui, "");
            if when == "count" {
                number(
                    ui,
                    &self.numbers,
                    "x/stop/count",
                    t("How many"),
                    &mut count,
                    70.0,
                );
                ui.label(theme::weak(ui, t("have left")));
            }
            choose(
                ui,
                "x/stop/element",
                t("Block"),
                &blocks,
                &mut element,
                170.0,
            );
        });
        if (element.clone(), count) != before
            && let Some(e) = element
        {
            let spec = if when == "count" {
                json!({ "when": "count", "element": e, "count": count.round().max(1.0) as u64 })
            } else {
                json!({ "when": "empty", "element": e })
            };
            self.execute("experiment.set", json!({ "stop": spec }));
        }
    }

    // -----------------------------------------------------------------------------------------
    // A timed block's inspector: its time from a three-point estimate.

    /// The block's estimate, if it has one, and a form to fit its time to a new one.
    pub(crate) fn estimate_section(&mut self, ui: &mut egui::Ui, el: &Value) {
        let id = el["id"].as_str().unwrap_or("").to_owned();
        section(ui, t("Three-point estimate"));
        let est = &el["estimate"];
        if est.is_object() {
            let fam = family_label(est["family"].as_str().unwrap_or(""));
            let num = |k: &str| est[k].as_f64().map(grouped);
            let upper = match (num("max"), num("p95")) {
                (Some(m), _) => tf("maximum {value}", &[("value", &m)]),
                (None, Some(p)) => tf("95th percentile {value}", &[("value", &p)]),
                _ => String::new(),
            };
            ui.label(tf(
                "{family}: minimum {min}, most likely {mode}, {upper}",
                &[
                    ("family", fam),
                    ("min", &num("min").unwrap_or_default()),
                    ("mode", &num("mode").unwrap_or_default()),
                    ("upper", &upper),
                ],
            ));
            if let Some(s) = est["source"].as_str().filter(|s| !s.is_empty()) {
                ui.label(theme::weak(ui, tf("Source: {source}", &[("source", s)])));
            }
        } else {
            ui.label(theme::weak(
                ui,
                t("Fit this block's time to a minimum, a most likely value and a maximum or 95th percentile."),
            ));
        }
        // The form, for this block.
        let fresh = || EstimateForm {
            id: id.clone(),
            family: FAMILIES
                .into_iter()
                .find(|f| est["family"] == *f)
                .unwrap_or("pert"),
            min: est["min"].as_f64().unwrap_or(0.0),
            mode: est["mode"].as_f64().unwrap_or(1.0),
            upper: est["max"]
                .as_f64()
                .or_else(|| est["p95"].as_f64())
                .unwrap_or(3.0),
            p95: est["max"].is_null() && est["p95"].is_number(),
            source: est["source"].as_str().unwrap_or("").to_owned(),
        };
        // The form stays closed until asked for, so the inspector stays short.
        let mut form = match self.decide.estimate.take() {
            Some(f) if f.id == id => f,
            _ => {
                let mut forget = false;
                ui.horizontal(|ui| {
                    let open = if est.is_object() {
                        t("Change the estimate")
                    } else {
                        t("Estimate from three values…")
                    };
                    if ui.add(theme::secondary(ui, open)).clicked() {
                        self.decide.estimate = Some(fresh());
                    }
                    if est.is_object() {
                        forget = ui
                            .add(theme::secondary(ui, t("Forget the estimate")))
                            .on_hover_text(t("Keep the time, without the estimate it came from"))
                            .clicked();
                    }
                });
                if forget {
                    self.execute("estimate.remove", json!({ "id": id }));
                }
                return;
            }
        };
        ui.horizontal(|ui| {
            row_label(ui, t("Family"));
            craft_ui::ComboBox::from_id_salt(("estimate/family", &id))
                .selected_text(family_label(form.family))
                .width(140.0)
                .show_ui(ui, |ui| {
                    for f in FAMILIES {
                        if ui
                            .selectable_label(form.family == f, family_label(f))
                            .clicked()
                        {
                            form.family = f;
                        }
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Family")));
        });
        ui.horizontal(|ui| {
            row_label(ui, t("Minimum"));
            number(
                ui,
                &self.numbers,
                &format!("{id}/est/min"),
                t("Minimum"),
                &mut form.min,
                110.0,
            );
        });
        ui.horizontal(|ui| {
            row_label(ui, t("Most likely"));
            number(
                ui,
                &self.numbers,
                &format!("{id}/est/mode"),
                t("Most likely"),
                &mut form.mode,
                110.0,
            );
        });
        // Log-normal and Weibull have no maximum: they are fitted to a 95th percentile.
        let open_ended = matches!(form.family, "lognormal" | "weibull");
        if open_ended {
            form.p95 = true;
        }
        let upper = if form.p95 {
            t("95th percentile")
        } else {
            t("Maximum")
        };
        ui.horizontal(|ui| {
            row_label(ui, t("Upper value"));
            number(
                ui,
                &self.numbers,
                &format!("{id}/est/upper"),
                upper,
                &mut form.upper,
                110.0,
            );
            if open_ended {
                ui.label(theme::weak(ui, upper));
            } else {
                craft_ui::ComboBox::from_id_salt(("estimate/upper", &id))
                    .selected_text(upper)
                    .width(140.0)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(!form.p95, t("Maximum")).clicked() {
                            form.p95 = false;
                        }
                        if ui
                            .selectable_label(form.p95, t("95th percentile"))
                            .clicked()
                        {
                            form.p95 = true;
                        }
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Upper value"))
                    });
            }
        });
        ui.horizontal(|ui| {
            row_label(ui, t("Source"));
            let r = ui.add(
                egui::TextEdit::singleline(&mut form.source)
                    .desired_width(220.0)
                    .hint_text(t("who gave it, or where it comes from")),
            );
            crate::form::field_outline(ui, &r);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, t("Source")));
        });
        let mut fit = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            fit = ui.add(theme::primary(ui, t("Fit the time"))).clicked();
            cancel = ui.add(theme::secondary(ui, t("Cancel"))).clicked();
        });
        if fit {
            let mut params = json!({
                "id": id,
                "family": form.family,
                "min": form.min,
                "mode": form.mode,
                "source": form.source.trim(),
            });
            params[if form.p95 { "p95" } else { "max" }] = json!(form.upper);
            if self.execute("estimate.set", params).is_some() {
                self.info(tf("Fitted the time of {id}", &[("id", &id)]));
                return;
            }
        }
        if !cancel {
            self.decide.estimate = Some(form);
        }
    }

    // -----------------------------------------------------------------------------------------
    // The results: the stop, and one result's distribution.

    /// When the replications met the experiment's stop condition, if it has one.
    pub(crate) fn stop_line(&self, ui: &mut egui::Ui, results: &Value) {
        let stop = &results["stop"];
        if !stop.is_object() {
            return;
        }
        let n = results["replications"].as_u64().unwrap_or(0);
        let met = stop["met"].as_u64().unwrap_or(0);
        let unit = unit_label(results["record"]["time_unit"].as_str().unwrap_or(""));
        let text = if met == 0 {
            t("No replication met the stop condition: each ran to its time limit.").to_owned()
        } else {
            tnf(
                n,
                "The stop condition was met in {met} of {n} replication, after {time} {unit} on average.",
                "The stop condition was met in {met} of {n} replications, after {time} {unit} on average.",
                &[
                    ("met", &met.to_string()),
                    ("time", &mean_pm(&stop["time"])),
                    ("unit", unit),
                ],
            )
        };
        ui.label(text);
    }

    /// One result's distribution across the replications, with a target's probability.
    pub(crate) fn distribution_section(&mut self, ui: &mut egui::Ui, results: &Value) {
        let model = stochcraft_engine::wire::model(self.engine.model());
        let mut options = outputs(&model, Some(results));
        if results["stop"].is_object() {
            options.insert(
                0,
                (
                    "/stop_time".to_owned(),
                    t("Time to the stop condition").to_owned(),
                ),
            );
        }
        if options.is_empty() {
            return;
        }
        section(ui, t("Spread across replications"));
        let mut chosen = self
            .decide
            .dist
            .as_ref()
            .map(|(e, m)| format!("{e}/{m}"))
            .filter(|k| options.iter().any(|(o, _)| o == k))
            .or_else(|| {
                options
                    .iter()
                    .find(|(k, _)| k.ends_with("/mean_wait") || k.ends_with("/mean_time_in_system"))
                    .or(options.first())
                    .map(|(k, _)| k.clone())
            });
        ui.horizontal_wrapped(|ui| {
            choose(ui, "dist/result", t("Result"), &options, &mut chosen, 260.0);
            bound_choice(
                ui,
                "dist/bound",
                t("Target"),
                &mut self.decide.dist_bound,
                true,
            );
            if self.decide.dist_bound != Bound::None {
                number(
                    ui,
                    &self.numbers,
                    "dist/target",
                    t("Target value"),
                    &mut self.decide.dist_target,
                    90.0,
                );
            }
        });
        let Some(key) = chosen else {
            return;
        };
        let (element, metric) = split(&key);
        self.decide.dist = Some((element.clone(), metric.clone()));
        let mut request = json!({ "metric": metric });
        if !element.is_empty() {
            request["element"] = json!(element);
        }
        if let Some(b) = self.decide.dist_bound.key() {
            request[b] = json!(self.decide.dist_target);
        }
        // Asked again only when the question or the results change.
        let digest = &results["record"]["model_digest"];
        let asked = json!({ "request": request, "digest": digest, "n": results["replications"] });
        if self
            .decide
            .dist_answer
            .as_ref()
            .is_none_or(|(a, _)| *a != asked)
        {
            let answer = self.engine.execute("results.distribution", request).ok();
            self.decide.dist_answer = Some((asked, answer));
        }
        let Some((_, Some(d))) = &self.decide.dist_answer else {
            ui.label(theme::weak(
                ui,
                t("Run the experiment again to see this result's spread."),
            ));
            return;
        };
        let d = d.clone();
        // Times read in the model's time unit; counts and shares as they are.
        let unit = match metric.as_str() {
            "stop_time" | "mean_wait" | "mean_time_in_system" | "mean_blocked_time" => {
                unit_label(results["record"]["time_unit"].as_str().unwrap_or("")).to_owned()
            }
            _ => String::new(),
        };
        ui.label(theme::weak(
            ui,
            tnf(
                d["n"].as_u64().unwrap_or(0),
                "{n} replication · mean {mean} · from {min} to {max}",
                "{n} replications · mean {mean} · from {min} to {max}",
                &[
                    ("mean", &mean_pm(&d)),
                    ("min", &d["min"].as_f64().map(grouped).unwrap_or_default()),
                    ("max", &d["max"].as_f64().map(grouped).unwrap_or_default()),
                ],
            ),
        ));
        // Percentiles, in order.
        let mut ps: Vec<(f64, f64)> = d["percentiles"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.parse().ok()?, v.as_f64()?)))
            .collect();
        ps.sort_by(|a, b| a.0.total_cmp(&b.0));
        grid(ui, "dist/percentiles", ps.len() + 1, |ui| {
            ui.label(theme::weak(ui, t("Percentile")));
            for (p, _) in &ps {
                ui.label(theme::weak(ui, grouped(*p)));
            }
            ui.end_row();
            ui.label(theme::weak(ui, t("Value")));
            for (_, v) in &ps {
                ui.label(grouped(*v));
            }
            ui.end_row();
        });
        ui.add_space(8.0);
        ecdf_chart(ui, &d, &unit);
        if let Some(target) = d.get("target").filter(|t| t.is_object()) {
            let interval = &target["interval"];
            let lo = interval[0].as_f64().unwrap_or(0.0);
            let hi = interval[1].as_f64().unwrap_or(0.0);
            ui.add_space(6.0);
            ui.label(theme::strong(tf(
                "{met} of {of} replications met the target: {p} (95% interval {lo} to {hi})",
                &[
                    ("met", &target["met"].to_string()),
                    ("of", &target["of"].to_string()),
                    ("p", &percent(target["probability"].as_f64().unwrap_or(0.0))),
                    ("lo", &percent(lo)),
                    ("hi", &percent(hi)),
                ],
            )));
        }
        if d["censored"].as_u64().is_some_and(|c| c > 0) {
            ui.label(theme::weak(
                ui,
                t("Replications that never met the stop condition count as later than their time limit."),
            ));
        }
    }

    // -----------------------------------------------------------------------------------------
    // The Analysis view.

    /// Sweeps, sensitivity and sizing, each with the last answer for this model.
    pub(crate) fn analysis_view(&mut self, ui: &mut egui::Ui) {
        let model = stochcraft_engine::wire::model(self.engine.model());
        let digest = stochcraft_engine::model_digest(self.engine.model());
        let results = self.results.clone();
        let outs = outputs(&model, results.as_ref());
        let params = parameters(&model);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(760.0));
                ui.heading(t("Analysis"));
                ui.label(theme::weak(
                    ui,
                    t("Each analysis runs the experiment several times with the same seed, so the differences come from the change, not from chance. They can take a while."),
                ));
                self.sweep_section(ui, &model, &digest, &outs, &params);
                self.sensitivity_section(ui, &model, &digest, &outs, &params);
                self.sizing_section(ui, &model, &digest, &outs);
            });
    }

    fn sweep_section(
        &mut self,
        ui: &mut egui::Ui,
        model: &Value,
        digest: &str,
        outs: &[(String, String)],
        params: &[(String, String)],
    ) {
        section(ui, t("Sweep a parameter"));
        ui.label(theme::weak(
            ui,
            t("Run the experiment at each value of one parameter, and compare a result."),
        ));
        let before = self.decide.sweep_param.clone();
        ui.horizontal(|ui| {
            row_label(ui, t("Parameter"));
            choose(
                ui,
                "sweep/param",
                t("Parameter"),
                params,
                &mut self.decide.sweep_param,
                260.0,
            );
        });
        ui.horizontal(|ui| {
            row_label(ui, t("Result"));
            choose(
                ui,
                "sweep/result",
                t("Result to compare"),
                outs,
                &mut self.decide.sweep_out,
                260.0,
            );
        });
        // Values to start from: the parameter's own and two more.
        if self.decide.sweep_param != before
            && let Some((id, field)) = self.decide.sweep_param.as_deref().map(split)
        {
            let current = current_value(model, &id, &field);
            self.decide.sweep_values = match (field.as_str(), current) {
                ("mean", Some(m)) => [0.8, 1.0, 1.2]
                    .iter()
                    .map(|k| grouped(m * k))
                    .collect::<Vec<_>>()
                    .join("; "),
                (_, Some(c)) => (0..3)
                    .map(|k| grouped(c + f64::from(k)))
                    .collect::<Vec<_>>()
                    .join("; "),
                _ => "1; 2; 3".into(),
            };
        }
        let mut go = false;
        ui.horizontal(|ui| {
            row_label(ui, t("Values"));
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.decide.sweep_values)
                    .desired_width(220.0)
                    .hint_text("1; 2; 3"),
            );
            crate::form::field_outline(ui, &r);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, t("Values")));
            let ready = self.decide.sweep_param.is_some() && self.decide.sweep_out.is_some();
            go = ui
                .add_enabled(ready, theme::primary(ui, t("Run the sweep")))
                .clicked();
        });
        ui.label(theme::weak(
            ui,
            t("Separate the values with semicolons or spaces."),
        ));
        if go {
            let values: Result<Vec<f64>, _> = self
                .decide
                .sweep_values
                .split([';', ' '])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| self.numbers.parse(s))
                .collect();
            match values {
                Ok(values) if !values.is_empty() => {
                    let (id, field) = split(self.decide.sweep_param.as_deref().unwrap_or(""));
                    let (element, metric) = split(self.decide.sweep_out.as_deref().unwrap_or(""));
                    let request = json!({
                        "parameter": { "id": id, "field": field },
                        "values": values,
                        "results": [{ "element": element, "metric": metric }],
                    });
                    if let Some(result) = self.execute("experiment.sweep", request.clone()) {
                        self.decide.sweep_answer = Some(Answer {
                            digest: digest.to_owned(),
                            request,
                            result,
                        });
                    }
                }
                _ => self.report_error(t(
                    "The values are numbers, separated by semicolons or spaces",
                )),
            }
        }
        let Some(a) = self
            .decide
            .sweep_answer
            .as_ref()
            .filter(|a| a.digest == digest)
        else {
            return;
        };
        let r = &a.result;
        let label = params
            .iter()
            .find(|(k, _)| {
                *k == format!(
                    "{}/{}",
                    a.request["parameter"]["id"].as_str().unwrap_or(""),
                    a.request["parameter"]["field"].as_str().unwrap_or("")
                )
            })
            .map_or_else(String::new, |(_, l)| l.clone());
        let out = outs
            .iter()
            .find(|(k, _)| {
                *k == format!(
                    "{}/{}",
                    a.request["results"][0]["element"].as_str().unwrap_or(""),
                    a.request["results"][0]["metric"].as_str().unwrap_or("")
                )
            })
            .map_or_else(String::new, |(_, l)| l.clone());
        ui.add_space(6.0);
        grid(ui, "sweep/table", 2, |ui| {
            ui.label(theme::weak(ui, label));
            ui.label(theme::weak(ui, out));
            ui.end_row();
            for p in r["points"].as_array().into_iter().flatten() {
                ui.label(p["value"].as_f64().map(grouped).unwrap_or_default());
                ui.label(mean_pm(&p["results"][0]));
                ui.end_row();
            }
        });
        for e in r["effects"].as_array().into_iter().flatten() {
            let change = e["change"].as_f64().map(grouped).unwrap_or_default();
            ui.label(if e["significant"] == true {
                tf(
                    "Across the range it changed by {change}: more than chance would explain.",
                    &[("change", &change)],
                )
            } else {
                tf(
                    "Across the range it changed by {change}: within what chance alone could do.",
                    &[("change", &change)],
                )
            });
        }
        ui.label(theme::weak(ui, replications_line(r)));
    }

    fn sensitivity_section(
        &mut self,
        ui: &mut egui::Ui,
        model: &Value,
        digest: &str,
        outs: &[(String, String)],
        params: &[(String, String)],
    ) {
        section(ui, t("Which parameters matter"));
        ui.label(theme::weak(
            ui,
            t("Move each parameter down and up in turn, and rank them by how much the result moves."),
        ));
        if self.decide.sens_change == 0.0 {
            self.decide.sens_change = 10.0;
        }
        let mut go = false;
        ui.horizontal(|ui| {
            row_label(ui, t("Result"));
            choose(
                ui,
                "sens/result",
                t("Result to explain"),
                outs,
                &mut self.decide.sens_out,
                260.0,
            );
        });
        ui.horizontal(|ui| {
            row_label(ui, t("Change"));
            number(
                ui,
                &self.numbers,
                "sens/change",
                t("Change"),
                &mut self.decide.sens_change,
                50.0,
            );
            ui.label(theme::weak(ui, t("% down and up")));
            let ready = self.decide.sens_out.is_some() && !params.is_empty();
            go = ui
                .add_enabled(ready, theme::primary(ui, t("Rank the parameters")))
                .clicked();
        });
        if go {
            let (element, metric) = split(self.decide.sens_out.as_deref().unwrap_or(""));
            // Every parameter but waiting rooms, which are a design choice rather than an input.
            let moved: Vec<Value> = params
                .iter()
                .map(|(k, _)| split(k))
                .filter(|(_, f)| f != "queue_capacity")
                .filter(|(id, f)| current_value(model, id, f).is_some())
                .map(|(id, field)| json!({ "id": id, "field": field }))
                .collect();
            let request = json!({
                "parameters": moved,
                "results": [{ "element": element, "metric": metric }],
                "change": (self.decide.sens_change / 100.0).clamp(0.001, 0.9),
            });
            if let Some(result) = self.execute("experiment.sensitivity", request.clone()) {
                self.decide.sens_answer = Some(Answer {
                    digest: digest.to_owned(),
                    request,
                    result,
                });
            }
        }
        let Some(a) = self
            .decide
            .sens_answer
            .as_ref()
            .filter(|a| a.digest == digest)
        else {
            return;
        };
        let mut effects: Vec<Value> = a.result["effects"].as_array().cloned().unwrap_or_default();
        effects.sort_by(|x, y| {
            let e = |v: &Value| v["elasticity"].as_f64().unwrap_or(0.0).abs();
            e(y).total_cmp(&e(x))
        });
        ui.add_space(6.0);
        grid(ui, "sens/table", 4, |ui| {
            for h in [
                t("Parameter"),
                t("Elasticity"),
                t("Result, down and up"),
                "",
            ] {
                ui.label(theme::weak(ui, h));
            }
            ui.end_row();
            for e in &effects {
                let key = format!(
                    "{}/{}",
                    e["parameter"]["id"].as_str().unwrap_or(""),
                    e["parameter"]["field"].as_str().unwrap_or("")
                );
                ui.label(
                    params
                        .iter()
                        .find(|(k, _)| *k == key)
                        .map_or(key.clone(), |(_, l)| l.clone()),
                );
                ui.label(e["elasticity"].as_f64().map(grouped).unwrap_or_default());
                let num = |k: &str| e[k].as_f64().map(grouped).unwrap_or_default();
                ui.label(format!("{} → {}", num("at_down"), num("at_up")));
                ui.label(if e["significant"] == true {
                    theme::strong(t("matters"))
                } else {
                    theme::weak(ui, t("within chance"))
                });
                ui.end_row();
            }
        });
        ui.label(theme::weak(ui, replications_line(&a.result)));
    }

    fn sizing_section(
        &mut self,
        ui: &mut egui::Ui,
        model: &Value,
        digest: &str,
        outs: &[(String, String)],
    ) {
        section(ui, t("Size the resources"));
        ui.label(theme::weak(
            ui,
            t("Add a unit at the busiest task or pool until enough replications meet a target. The answer with every time at its mean is shown too; it is usually too small."),
        ));
        if self.decide.size_bound == Bound::None {
            self.decide.size_bound = Bound::AtMost;
            self.decide.size_confidence = 70.0;
            self.decide.size_target = 1.0;
        }
        let mut go = false;
        ui.horizontal(|ui| {
            row_label(ui, t("Result"));
            choose(
                ui,
                "size/result",
                t("Result with a target"),
                outs,
                &mut self.decide.size_out,
                260.0,
            );
        });
        ui.horizontal(|ui| {
            row_label(ui, t("Target"));
            bound_choice(
                ui,
                "size/bound",
                t("Target"),
                &mut self.decide.size_bound,
                false,
            );
            number(
                ui,
                &self.numbers,
                "size/target",
                t("Target value"),
                &mut self.decide.size_target,
                80.0,
            );
        });
        ui.horizontal(|ui| {
            row_label(ui, t("Met in at least"));
            number(
                ui,
                &self.numbers,
                "size/confidence",
                t("Share of replications"),
                &mut self.decide.size_confidence,
                50.0,
            );
            ui.label(t("% of replications"));
            let ready = self.decide.size_out.is_some();
            go = ui
                .add_enabled(ready, theme::primary(ui, t("Size")))
                .clicked();
        });
        let request = || {
            let (element, metric) = split(self.decide.size_out.as_deref().unwrap_or(""));
            let mut target = json!({ "element": element, "metric": metric });
            if let Some(b) = self.decide.size_bound.key() {
                target[b] = json!(self.decide.size_target);
            }
            json!({
                "target": target,
                "confidence": (self.decide.size_confidence / 100.0).clamp(0.01, 0.999),
            })
        };
        if go {
            let request = request();
            if let Some(result) = self.execute("experiment.size", request.clone()) {
                self.decide.size_answer = Some(Answer {
                    digest: digest.to_owned(),
                    request,
                    result,
                });
            }
        }
        let Some(a) = self
            .decide
            .size_answer
            .as_ref()
            .filter(|a| a.digest == digest)
        else {
            return;
        };
        let r = a.result.clone();
        let applied_request = a.request.clone();
        let st = &r["stochastic"];
        ui.add_space(6.0);
        if st["reached"] == true {
            ui.label(theme::strong(tf(
                "Enough: {units}, meeting the target in {p} of replications",
                &[
                    ("units", &units_text(model, &st["units"])),
                    ("p", &percent(st["probability"].as_f64().unwrap_or(0.0))),
                ],
            )));
        } else {
            ui.label(theme::strong(tf(
                "Not reached, even with {units}",
                &[("units", &units_text(model, &st["units"]))],
            )));
        }
        let det = &r["deterministic"];
        ui.label(tf(
            "With every time at its mean: {units}",
            &[("units", &units_text(model, &det["units"]))],
        ));
        grid(ui, "size/steps", 3, |ui| {
            for h in [
                t("Units"),
                t("Replications meeting the target"),
                t("Busiest"),
            ] {
                ui.label(theme::weak(ui, h));
            }
            ui.end_row();
            for s in st["steps"].as_array().into_iter().flatten() {
                ui.label(units_text(model, &s["units"]));
                let lo = s["interval"][0].as_f64().unwrap_or(0.0);
                let hi = s["interval"][1].as_f64().unwrap_or(0.0);
                ui.label(format!(
                    "{} ({}–{})",
                    percent(s["probability"].as_f64().unwrap_or(0.0)),
                    percent(lo),
                    percent(hi)
                ));
                ui.label(
                    s["bottleneck"]["id"]
                        .as_str()
                        .map_or_else(String::new, |id| name_of(model, id)),
                );
                ui.end_row();
            }
        });
        ui.label(theme::weak(ui, replications_line(&r)));
        if st["reached"] == true
            && ui
                .add(theme::secondary(ui, t("Use these units in the model")))
                .on_hover_text(t("One change, which Undo takes back"))
                .clicked()
        {
            let mut request = applied_request;
            request["apply"] = json!(true);
            if self.execute("experiment.size", request).is_some() {
                self.info(t("The model now has the units sizing found"));
            }
        }
    }
}

/// A parameter's value in the model, if it has one (a time distribution's mean is not read here,
/// only whether there is one).
fn current_value(model: &Value, id: &str, field: &str) -> Option<f64> {
    let el = model["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(model["resources"].as_array().into_iter().flatten())
        .find(|e| e["id"] == id)?;
    match field {
        "capacity" => el["capacity"].as_f64().or(Some(1.0)),
        "queue_capacity" => el["queue_capacity"].as_f64(),
        "quantity" => el["quantity"].as_f64(),
        "mean" => ["interarrival", "service", "duration"]
            .iter()
            .find_map(|k| el[*k].as_object())
            .map(|d| d.get("mean").and_then(Value::as_f64).unwrap_or(1.0)),
        _ => None,
    }
}

fn replications_line(r: &Value) -> String {
    tnf(
        r["replications"].as_u64().unwrap_or(0),
        "{n} replication at each step · seed {seed}",
        "{n} replications at each step · seed {seed}",
        &[("seed", &r["seed"].to_string())],
    )
}

/// The empirical cumulative distribution as a step line, with its confidence band shaded.
fn ecdf_chart(ui: &mut egui::Ui, d: &Value, unit: &str) {
    let points: Vec<(f64, f64)> = d["ecdf"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p[0].as_f64()?, p[1].as_f64()?)))
        .collect();
    if points.is_empty() {
        return;
    }
    let band = d["band"].as_f64().unwrap_or(0.0);
    let p = theme::palette(ui.ctx());
    let width = ui.available_width().min(520.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 150.0), egui::Sense::hover());
    let plot = Rect::from_min_max(
        pos2(rect.left() + 34.0, rect.top() + 4.0),
        pos2(rect.right() - 8.0, rect.bottom() - 20.0),
    );
    let lo = points.first().map_or(0.0, |p| p.0);
    let hi = points.last().map_or(1.0, |p| p.0);
    let span = if hi > lo { hi - lo } else { 1.0 };
    let (lo, hi) = (lo - span * 0.05, hi + span * 0.05);
    let x = |v: f64| plot.left() + ((v - lo) / (hi - lo)) as f32 * plot.width();
    let y = |f: f64| plot.bottom() - f.clamp(0.0, 1.0) as f32 * plot.height();
    let painter = ui.painter_at(rect);
    let grid = Stroke::new(1.0, p.border);
    for f in [0.0, 0.5, 1.0] {
        painter.line_segment([pos2(plot.left(), y(f)), pos2(plot.right(), y(f))], grid);
        painter.text(
            pos2(plot.left() - 4.0, y(f)),
            Align2::RIGHT_CENTER,
            percent(f),
            FontId::proportional(10.0),
            p.text_weak,
        );
    }
    let fill = p.accent.gamma_multiply(0.18);
    let line = Stroke::new(2.0, p.accent);
    // Each step holds from its point to the next (and to the right edge after the last).
    let mut prev_f = 0.0;
    let mut prev_x = lo;
    for (i, &(v, f)) in points.iter().enumerate() {
        let r = Rect::from_x_y_ranges(
            x(prev_x)..=x(v),
            y((prev_f + band).min(1.0))..=y((prev_f - band).max(0.0)),
        );
        painter.rect_filled(r, 0.0, fill);
        painter.line_segment([pos2(x(prev_x), y(prev_f)), pos2(x(v), y(prev_f))], line);
        painter.line_segment([pos2(x(v), y(prev_f)), pos2(x(v), y(f))], line);
        prev_f = f;
        prev_x = v;
        if i + 1 == points.len() {
            let r = Rect::from_x_y_ranges(
                x(v)..=x(hi),
                y((f + band).min(1.0))..=y((f - band).max(0.0)),
            );
            painter.rect_filled(r, 0.0, fill);
            painter.line_segment([pos2(x(v), y(f)), pos2(x(hi), y(f))], line);
        }
    }
    let first = points.first().map_or(0.0, |p| p.0);
    let last = points.last().map_or(0.0, |p| p.0);
    for (v, align) in [(first, Align2::LEFT_TOP), (last, Align2::RIGHT_TOP)] {
        let text = if unit.is_empty() {
            grouped(v)
        } else {
            format!("{} {unit}", grouped(v))
        };
        painter.text(
            pos2(x(v), plot.bottom() + 4.0),
            align,
            text,
            FontId::proportional(10.0),
            p.text_weak,
        );
    }
    ui.label(theme::weak(
        ui,
        tf(
            "The share of replications at or below each value. The band holds the true distribution with {confidence} confidence.",
            &[("confidence", &percent(d["confidence"].as_f64().unwrap_or(0.95)))],
        ),
    ));
}
