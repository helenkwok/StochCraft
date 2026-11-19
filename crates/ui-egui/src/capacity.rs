//! The Capacity view: what each station can carry, worked out from the model before any run
//! (`model.capacity`), with the simulated values beside it once there are results of this model.
//! It shows what `stochcraft-cli explain` prints, so the two read the same.

use std::rc::Rc;

use egui::accesskit::Role;
use serde_json::Value;

use crate::i18n::{t, tf};
use crate::{App, results, theme};

/// The Capacity view's target, as typed, and the table worked out for it.
pub(crate) struct State {
    /// The target utilisation in percent, as typed.
    target: String,
    /// The table with the engine's revision and the target (as bits) it was worked out for, so
    /// it is worked out when the model or the target changes, not every frame.
    table: Option<(u64, u64, Rc<Value>)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            target: "85".into(),
            table: None,
        }
    }
}

/// A cell of the table: its text, and what a pointer or screen reader gets on top of it.
struct Cell {
    text: String,
    hover: Option<String>,
}

impl Cell {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            hover: None,
        }
    }
}

/// A number as the table writes it: `?` (the reason on hover) where it is not known, ≤ or ≥
/// before a bound, and a percentage to one decimal.
fn bounded(v: &Value, percent: bool) -> Cell {
    let Some(x) = v["value"].as_f64() else {
        return Cell {
            text: "?".into(),
            hover: v["unknown"]["message"].as_str().map(str::to_owned),
        };
    };
    let shown = if percent {
        format!("{}%", results::grouped((x * 1000.0).round() / 10.0))
    } else {
        results::grouped(x)
    };
    Cell::plain(match v["bound"].as_str() {
        Some("at_most") => format!("≤ {shown}"),
        Some("at_least") => format!("≥ {shown}"),
        _ => shown,
    })
}

/// A simulated value of a station from the results, as "mean ± half-width" rounded as the
/// Results view rounds it; `scale` 100 and `percent` for a share. "–" where there is none.
fn simulated(results: Option<&Value>, id: &str, metric: &str, percent: bool) -> String {
    let m = results
        .and_then(|r| r["elements"].as_array())
        .and_then(|es| es.iter().find(|e| e["id"] == id))
        .map(|e| &e["metrics"][metric]);
    let scale = if percent { 100.0 } else { 1.0 };
    let Some(mean) = m.and_then(|m| m["mean"].as_f64()) else {
        return "–".into();
    };
    let unit = if percent { "%" } else { "" };
    match m.and_then(|m| m["half_width"].as_f64()) {
        Some(hw) => {
            let (mean, hw) = results::rounded(mean * scale, hw * scale);
            format!("{mean}{unit} ± {hw}{unit}")
        }
        None => format!("{}{unit}", results::grouped(mean * scale)),
    }
}

impl App {
    /// The table for `target` (a share), worked out again only when the model or the target
    /// has changed.
    fn capacity_table(&mut self, target: f64) -> Rc<Value> {
        let key = (self.engine.revision(), target.to_bits());
        if let Some((r, b, table)) = &self.capacity.table
            && (*r, *b) == key
        {
            return table.clone();
        }
        let model = self.engine.model();
        let table = Rc::new(stochcraft_engine::wire::capacity(
            model,
            &model.capacity(target),
        ));
        self.capacity.table = Some((key.0, key.1, table.clone()));
        table
    }

    /// The target as typed, in percent, if it is a number above 0 and below 100.
    fn capacity_target(&self) -> Option<f64> {
        let typed = self.capacity.target.trim().trim_end_matches('%').trim();
        let x = self.numbers.parse(typed).ok()?;
        (x.is_finite() && x > 0.0 && x < 100.0).then_some(x)
    }

    /// The Capacity view.
    pub(crate) fn capacity_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let unit = crate::time_unit(&self.engine);
        let mut show: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading(t("Capacity"));
                ui.label(theme::weak(
                    ui,
                    tf("Rates and times are per {unit}.", &[("unit", &unit)]),
                ));
                ui.add_space(8.0);
                // The target: a field of its own, with its error in words under it.
                let valid = self.capacity_target();
                ui.horizontal(|ui| {
                    ui.label(t("Target utilisation"));
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.capacity.target).desired_width(60.0),
                    );
                    crate::form::field_outline(ui, &r);
                    if valid.is_none() {
                        ui.painter().rect_stroke(
                            r.rect,
                            4.0,
                            egui::Stroke::new(1.5, p.error),
                            egui::StrokeKind::Inside,
                        );
                    }
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(Role::TextInput, true, t("Target utilisation"))
                    });
                    r.on_hover_text(t(
                        "The utilisation above which a station counts as congested",
                    ));
                    ui.label("%");
                });
                // An invalid target leaves the table as it was for the last good one.
                let used = valid.unwrap_or(85.0);
                let table = self.capacity_table(used / 100.0);
                if valid.is_none() {
                    theme::error_message(
                        ui,
                        tf(
                            "{reason}. {field} is still {value}.",
                            &[
                                ("reason", t("Must be above 0% and below 100%")),
                                ("field", t("Target utilisation")),
                                ("value", &format!("{}%", results::grouped(used))),
                            ],
                        ),
                    );
                }
                ui.add_space(8.0);
                let stations = table["stations"].as_array().map_or(&[][..], Vec::as_slice);
                if stations.is_empty() {
                    ui.label(t("No servers, shared tasks, delays or pools."));
                    return;
                }
                let sim = self.results();
                let target_label = results::grouped(used);
                self.capacity_grid(ui, &p, stations, sim, &target_label, &mut show);
                self.capacity_notes(ui, &table, stations);
            });
        if let Some(id) = show {
            self.show_block(Some(id));
        }
    }

    /// The table: a row per station, scrolling sideways where the panel is narrow.
    fn capacity_grid(
        &self,
        ui: &mut egui::Ui,
        p: &theme::Palette,
        stations: &[Value],
        sim: Option<&Value>,
        target: &str,
        show: &mut Option<String>,
    ) {
        let headers: [(String, Option<&str>); 13] = [
            (t("Station").into(), None),
            (t("Kind").into(), None),
            (t("Arrivals").into(), None),
            (t("Service").into(), None),
            (t("Units").into(), None),
            (t("Utilisation").into(), None),
            (
                t("Simulated utilisation").into(),
                Some(t(
                    "From the last run; – when there are none, or they are out of date.",
                )),
            ),
            (t("Load").into(), None),
            (t("Status").into(), None),
            (tf("Units for {target}%", &[("target", target)]), None),
            (t("Formula").into(), None),
            (t("Mean wait").into(), None),
            (
                t("Simulated mean wait").into(),
                Some(t(
                    "From the last run; – when there are none, or they are out of date.",
                )),
            ),
        ];
        egui::ScrollArea::horizontal()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                egui::Grid::new("capacity-table")
                    .striped(true)
                    .num_columns(headers.len())
                    .spacing(egui::vec2(18.0, 6.0))
                    .show(ui, |ui| {
                        for (h, hover) in &headers {
                            let r = ui.label(theme::strong(h).size(12.0).color(p.text_weak));
                            if let Some(hover) = hover {
                                r.on_hover_text(*hover);
                            }
                        }
                        ui.end_row();
                        for st in stations {
                            self.capacity_row(ui, p, st, sim, show);
                            ui.end_row();
                        }
                    });
            });
    }

    /// One station's row.
    fn capacity_row(
        &self,
        ui: &mut egui::Ui,
        p: &theme::Palette,
        st: &Value,
        sim: Option<&Value>,
        show: &mut Option<String>,
    ) {
        let kind = st["kind"].as_str().unwrap_or("");
        let (pool, delay) = (kind == "pool", kind == "delay");
        let id = st["id"].as_str().unwrap_or("");
        let name = st["name"].as_str().unwrap_or(id);
        let units = match st["units"].as_u64() {
            None => "-".to_owned(),
            Some(u) if st["units_vary"] == true => tf("up to {n}", &[("n", &u.to_string())]),
            Some(u) => u.to_string(),
        };
        let dash = || Cell::plain("-");
        let numbers = |key: &str| {
            if pool {
                dash()
            } else {
                bounded(&st[key], false)
            }
        };
        let share = |key: &str| {
            if delay {
                dash()
            } else {
                bounded(&st[key], true)
            }
        };
        let simulated_cell = |metric: &str, percent: bool| {
            Cell::plain(if pool || delay {
                "–".to_owned()
            } else {
                simulated(sim, id, metric, percent)
            })
        };
        let wait = if delay || pool {
            dash()
        } else {
            st["formula"]["mean_wait"]
                .as_f64()
                .map_or_else(dash, |w| Cell::plain(results::grouped(w)))
        };
        let can_show = self
            .engine
            .model()
            .elements
            .iter()
            .any(|e| e.id.as_str() == id);
        if can_show {
            if ui.link(name).on_hover_text(t("Show this block")).clicked() {
                *show = Some(id.to_owned());
            }
        } else {
            ui.label(name);
        }
        ui.label(results::kind_label(if pool { "resource" } else { kind }));
        cell(ui, numbers("arrival_rate"));
        cell(ui, numbers("mean_service"));
        cell(ui, Cell::plain(units));
        cell(ui, share("utilisation"));
        cell(ui, simulated_cell("utilisation", true));
        cell(ui, share("load"));
        status_cell(ui, p, st["status"].as_str().unwrap_or("unknown"));
        cell(
            ui,
            Cell::plain(
                st["units_for_target"]
                    .as_u64()
                    .map_or_else(|| "-".into(), |u| u.to_string()),
            ),
        );
        ui.label(st["formula"]["name"].as_str().unwrap_or(""));
        cell(ui, wait);
        cell(ui, simulated_cell("mean_wait", false));
    }

    /// Under the table: the bottleneck, what is not known before a run, why no formula holds
    /// where none does, and how utilisation is counted.
    fn capacity_notes(&self, ui: &mut egui::Ui, table: &Value, stations: &[Value]) {
        ui.add_space(12.0);
        let bottleneck = table["bottleneck"]
            .as_str()
            .and_then(|b| stations.iter().find(|st| st["id"] == b));
        if let Some(st) = bottleneck {
            ui.label(tf(
                "Bottleneck: {name} (load {load}).",
                &[
                    ("name", st["name"].as_str().unwrap_or("")),
                    ("load", &bounded(&st["load"], true).text),
                ],
            ));
        } else if stations
            .iter()
            .any(|st| st["kind"] == "server" || st["kind"] == "pool")
        {
            ui.label(t(
                "Bottleneck: not known before a run (a load not known, or only bounded, could be the highest).",
            ));
        }
        let mut unknown = Vec::new();
        let mut no_formula = Vec::new();
        for st in stations {
            let name = st["name"].as_str().unwrap_or("");
            let pool = st["kind"] == "pool";
            for (what, key) in [
                (t("arrival rate"), "arrival_rate"),
                (t("service time"), "mean_service"),
                (t("utilisation"), "utilisation"),
            ] {
                if pool && key != "utilisation" {
                    continue;
                }
                if let Some(m) = st[key]["unknown"]["message"].as_str() {
                    unknown.push(tf(
                        "{station}: {what}: {message}.",
                        &[("station", name), ("what", what), ("message", m)],
                    ));
                    break;
                }
            }
            if !pool && let Some(m) = st["no_formula"]["message"].as_str() {
                no_formula.push(tf(
                    "{station}: {message}.",
                    &[("station", name), ("message", m)],
                ));
            }
        }
        if !unknown.is_empty() {
            ui.add_space(8.0);
            ui.label(theme::strong(t("Not known before a run:")));
            for l in &unknown {
                ui.label(l);
            }
        }
        if !no_formula.is_empty() {
            ui.add_space(8.0);
            ui.label(theme::strong(t("No exact formula for the wait:")));
            for l in &no_formula {
                ui.label(l);
            }
        }
        ui.add_space(8.0);
        ui.label(theme::weak(
            ui,
            t(
                "Utilisation is arrivals × service ÷ units, as the results count it; the load is that over the units' availability. A run adds what variability and blocking cost.",
            ),
        ));
    }
}

/// A numeric cell, right-aligned, with its reason on hover where it is unknown.
fn cell(ui: &mut egui::Ui, c: Cell) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let r = ui.label(egui::RichText::new(c.text).monospace());
        if let Some(h) = c.hover {
            r.on_hover_text(h);
        }
    });
}

/// A status as a mark and a word, so it is not told by colour alone: a tick for fine, a
/// triangle for congested, a cross for unstable, an open circle for unknown.
fn status_cell(ui: &mut egui::Ui, p: &theme::Palette, status: &str) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        let (centre, painter) = (rect.center(), ui.painter());
        let word = match status {
            "fine" => {
                theme::paint_check_mark(painter, centre, 6.0, p);
                t("Fine")
            }
            "congested" => {
                theme::paint_warning_mark(painter, centre, 6.0, p);
                t("Congested")
            }
            "unstable" => {
                theme::paint_error_mark(painter, centre, 6.5, p);
                t("Unstable")
            }
            _ => {
                painter.circle_stroke(centre, 5.5, egui::Stroke::new(1.5, p.text_weak));
                t("Unknown")
            }
        };
        ui.label(word);
    });
}
