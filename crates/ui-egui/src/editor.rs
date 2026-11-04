//! Editing the model without a canvas: the model overview (name, experiment, connections,
//! problems) and the inspector of the selected block (name, parameters, flows, its problems).
//! Every change is one engine command, so each is one undo step.

use serde_json::{Value, json};
use stochcraft_engine::COMMANDS;

use crate::form::{self, Schema};
use crate::{App, theme};

/// The kinds of block that can be added, with their parameters when added.
pub const KINDS: &[(&str, &str)] = &[
    ("source", "Arrivals"),
    ("server", "Task with a resource"),
    ("delay", "Task"),
    ("sink", "Departures"),
];

/// A new block's parameters: one arrival a time unit, service and delay of one time unit.
pub fn new_block(kind: &str) -> Value {
    match kind {
        "source" => json!({"kind": "source", "interarrival": {"type": "exponential", "mean": 1.0}}),
        "server" => {
            json!({"kind": "server", "capacity": 1, "service": {"type": "exponential", "mean": 1.0}})
        }
        "delay" => json!({"kind": "delay", "duration": {"type": "constant", "value": 1.0}}),
        _ => json!({"kind": "sink"}),
    }
}

/// The parameters each kind of block has, in the order the inspector shows them.
fn parameters(kind: &str) -> &'static [&'static str] {
    match kind {
        "source" => &["interarrival", "limit"],
        "server" => &["capacity", "service"],
        "delay" => &["duration"],
        _ => &[],
    }
}

/// The parameters of a command, from the engine's catalogue.
fn params_schema(command: &str) -> Value {
    COMMANDS
        .iter()
        .find(|c| c.name == command)
        .map(|c| c.params_schema())
        .unwrap_or(Value::Null)
}

/// A problem found by validation, for the place it concerns.
pub struct Problem {
    pub error: bool,
    pub at: Option<String>,
    pub message: String,
}

impl App {
    /// The model's problems (validation is a query: nothing is journalled).
    pub(crate) fn problems(&mut self) -> Vec<Problem> {
        let v = self
            .engine
            .execute("model.validate", json!({}))
            .unwrap_or_default();
        v["diagnostics"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| Problem {
                error: d["severity"] == "error",
                at: d["at"].as_str().map(str::to_owned),
                message: d["message"].as_str().unwrap_or("").to_owned(),
            })
            .collect()
    }

    /// Adds a block of `kind` after the selection, joins the selection to it when the selected
    /// block sends entities on, and selects it.
    pub(crate) fn add_block(&mut self, kind: &str) {
        let mut params = new_block(kind);
        let index = self.selected_index().map(|i| i + 1);
        if let Some(i) = index {
            params["index"] = json!(i);
        }
        if let Some(r) = self.execute("block.add", params)
            && let Some(id) = r["id"].as_str()
        {
            let label = KINDS.iter().find(|k| k.0 == kind).map_or(kind, |k| k.1);
            self.info(format!("Added {} ({id})", label.to_lowercase()));
            self.selected = Some(id.to_owned());
        }
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        let id = self.selected.as_deref()?;
        self.engine
            .model()
            .elements
            .iter()
            .position(|e| e.id.as_str() == id)
    }

    /// Removes the selected block (and its flows) and selects its neighbour.
    pub(crate) fn remove_selected(&mut self) {
        let Some(i) = self.selected_index() else {
            return;
        };
        let Some(id) = self.selected.clone() else {
            return;
        };
        if self.execute("block.remove", json!({ "id": id })).is_some() {
            self.info(format!("Removed {id}"));
            let els = &self.engine.model().elements;
            self.selected = els
                .get(i.min(els.len().saturating_sub(1)))
                .map(|e| e.id.as_str().to_owned());
        }
    }

    /// Moves the selection up or down the block list.
    pub(crate) fn move_selection(&mut self, down: bool) {
        let n = self.engine.model().elements.len();
        if n == 0 {
            return;
        }
        let i = match (self.selected_index(), down) {
            (None, true) => 0,
            (None, false) => n - 1,
            (Some(i), true) => (i + 1).min(n - 1),
            (Some(i), false) => i.saturating_sub(1),
        };
        self.selected = self
            .engine
            .model()
            .elements
            .get(i)
            .map(|e| e.id.as_str().to_owned());
    }

    /// Model overview or block inspector, by the selection.
    pub(crate) fn editor(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let problems = self.problems();
        let model = stochcraft_engine::wire::model(self.engine.model());
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(640.0);
                match self.selected.clone() {
                    Some(id) => {
                        let el = model["elements"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|e| e["id"] == id.as_str())
                            .cloned();
                        match el {
                            Some(el) => self.inspector(ui, p, &model, &el, &problems),
                            None => self.selected = None,
                        }
                    }
                    None => self.overview(ui, p, &model, &problems),
                }
            });
    }

    fn time_unit_label(&self) -> String {
        crate::time_unit(&self.engine)
    }

    fn overview(
        &mut self,
        ui: &mut egui::Ui,
        p: &theme::Palette,
        model: &Value,
        problems: &[Problem],
    ) {
        ui.heading("Model");
        ui.add_space(8.0);
        // The name.
        let name = model["name"].as_str().unwrap_or("").to_owned();
        if let Some(n) = self.text_field(ui, "model/name", "Name", &name) {
            self.execute("model.rename", json!({ "name": n }));
        }

        section(ui, "Experiment");
        let schema = params_schema("experiment.set");
        let unit = self.time_unit_label();
        let x = model["experiment"].clone();
        for (f, time) in [
            ("replications", false),
            ("seed", false),
            ("warmup", true),
            ("duration", true),
            ("time_unit", false),
        ] {
            let mut cx = form::Cx {
                time_unit: &unit,
                drafts: &mut self.drafts,
            };
            if let Some(v) = form::field(
                ui,
                &mut cx,
                Schema::new(&schema),
                &format!("x/{f}"),
                f,
                &x[f],
                time,
            ) {
                self.execute("experiment.set", json!({ f: v }));
            }
        }
        ui.label(theme::weak(
            ui,
            "Changing the time unit does not convert the times already in the model.",
        ));

        section(ui, "Connections");
        self.connections_table(ui, p, model);

        section(ui, "Problems");
        if problems.is_empty() {
            ui.label(theme::weak(ui, "None found. The model is ready to run."));
        }
        for pr in problems {
            self.problem_row(ui, p, pr, true);
        }
    }

    fn problem_row(&mut self, ui: &mut egui::Ui, p: &theme::Palette, pr: &Problem, link: bool) {
        ui.horizontal_wrapped(|ui| {
            let (word, color) = if pr.error {
                ("Error", p.error)
            } else {
                ("Warning", p.warning)
            };
            ui.label(theme::strong(word).color(color));
            ui.label(&pr.message);
            if link
                && let Some(at) = &pr.at
                && ui.link(format!("Show {at}")).clicked()
            {
                self.selected = Some(at.clone());
            }
        });
    }

    fn connections_table(&mut self, ui: &mut egui::Ui, p: &theme::Palette, model: &Value) {
        let names = element_names(model);
        let name_of = |id: &str| {
            names
                .iter()
                .find(|(i, _)| i == id)
                .map_or_else(|| id.to_owned(), |(_, n)| n.clone())
        };
        let conns: Vec<(String, String, String)> = model["connections"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                (
                    c["id"].as_str().unwrap_or("").to_owned(),
                    c["from"].as_str().unwrap_or("").to_owned(),
                    c["to"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect();
        if conns.is_empty() {
            ui.label(theme::weak(ui, "No connections yet: join blocks below."));
        }
        egui::Grid::new("connections")
            .num_columns(4)
            .striped(true)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                for (id, from, to) in &conns {
                    ui.label(name_of(from));
                    ui.label(theme::weak(ui, "→"));
                    ui.label(name_of(to));
                    if ui
                        .add(theme::secondary(ui, "Remove"))
                        .on_hover_text(format!("Remove the flow {id}"))
                        .clicked()
                    {
                        self.execute("disconnect", json!({ "id": id }));
                    }
                    ui.end_row();
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            pick(ui, "connect/from", "From", &names, &mut self.connect_from);
            ui.label(theme::weak(ui, "→"));
            pick(ui, "connect/to", "To", &names, &mut self.connect_to);
            let ready = self.connect_from.is_some() && self.connect_to.is_some();
            if ui
                .add_enabled(ready, theme::secondary(ui, "Connect"))
                .clicked()
                && let (Some(f), Some(t)) = (self.connect_from.take(), self.connect_to.take())
            {
                self.execute("connect", json!({ "from": f, "to": t }));
            }
        });
        let _ = p;
    }

    fn inspector(
        &mut self,
        ui: &mut egui::Ui,
        p: &theme::Palette,
        model: &Value,
        el: &Value,
        problems: &[Problem],
    ) {
        let id = el["id"].as_str().unwrap_or("").to_owned();
        let kind = el["kind"].as_str().unwrap_or("").to_owned();
        let kind_label = KINDS
            .iter()
            .find(|k| k.0 == kind)
            .map_or(kind.as_str(), |k| k.1);
        ui.horizontal(|ui| {
            if ui
                .link("Model")
                .on_hover_text("Back to the model (Esc)")
                .clicked()
            {
                self.selected = None;
            }
            ui.label(theme::weak(ui, "›"));
            ui.label(theme::weak(ui, kind_label));
        });
        ui.add_space(4.0);
        let name = el["name"].as_str().unwrap_or("").to_owned();
        ui.horizontal(|ui| {
            ui.heading(if name.is_empty() {
                id.as_str()
            } else {
                name.as_str()
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(theme::secondary(ui, "Remove block"))
                    .on_hover_text("Remove this block and its flows (Delete)")
                    .clicked()
                {
                    self.remove_selected();
                }
            });
        });
        ui.label(theme::weak(ui, format!("id {id}")).monospace());
        ui.add_space(8.0);

        for pr in problems
            .iter()
            .filter(|pr| pr.at.as_deref() == Some(id.as_str()))
        {
            self.problem_row(ui, p, pr, false);
        }

        if let Some(n) = self.text_field(ui, &format!("{id}/name"), "Name", &name) {
            self.execute("block.rename", json!({ "id": id, "name": n }));
        }

        let fields = parameters(&kind);
        if !fields.is_empty() {
            section(ui, "Parameters");
            let schema = params_schema("block.set");
            let unit = self.time_unit_label();
            for f in fields {
                let mut cx = form::Cx {
                    time_unit: &unit,
                    drafts: &mut self.drafts,
                };
                let key = format!("{id}/{f}");
                let time = matches!(*f, "interarrival" | "service" | "duration");
                if let Some(v) =
                    form::field(ui, &mut cx, Schema::new(&schema), &key, f, &el[*f], time)
                {
                    self.execute("block.set", json!({ "id": id, *f: v }));
                }
            }
        }

        section(ui, "Flows");
        let names = element_names(model);
        let name_of = |x: &str| {
            names
                .iter()
                .find(|(i, _)| i == x)
                .map_or_else(|| x.to_owned(), |(_, n)| n.clone())
        };
        let conns: Vec<(String, String, String)> = model["connections"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                (
                    c["id"].as_str().unwrap_or("").to_owned(),
                    c["from"].as_str().unwrap_or("").to_owned(),
                    c["to"].as_str().unwrap_or("").to_owned(),
                )
            })
            .collect();
        for (heading, incoming) in [("Comes from", true), ("Goes to", false)] {
            if (incoming && kind == "source") || (!incoming && kind == "sink") {
                continue;
            }
            ui.horizontal(|ui| {
                form::label_cell(ui, theme::weak(ui, heading), 22.0);
                ui.vertical(|ui| {
                    let mine: Vec<_> = conns
                        .iter()
                        .filter(|(_, f, t)| if incoming { t == &id } else { f == &id })
                        .collect();
                    if mine.is_empty() {
                        ui.label(theme::weak(ui, "nothing yet"));
                    }
                    for (cid, f, t) in mine {
                        let other = if incoming { f } else { t };
                        ui.horizontal(|ui| {
                            if ui.link(name_of(other)).clicked() {
                                self.selected = Some(other.clone());
                            }
                            if ui
                                .add(
                                    egui::Button::new(egui::RichText::new("Remove").size(12.0))
                                        .stroke(egui::Stroke::new(1.0, p.border)),
                                )
                                .on_hover_text(format!("Remove the flow {cid}"))
                                .clicked()
                            {
                                self.execute("disconnect", json!({ "id": cid }));
                            }
                        });
                    }
                    let others: Vec<(String, String)> = names
                        .iter()
                        .filter(|(i, _)| i != &id)
                        .filter(|(i, _)| {
                            let k = kind_of(model, i);
                            if incoming { k != "sink" } else { k != "source" }
                        })
                        .cloned()
                        .collect();
                    let mut chosen: Option<String> = None;
                    let label = if incoming {
                        "Add a flow from…"
                    } else {
                        "Add a flow to…"
                    };
                    egui::ComboBox::from_id_salt((id.as_str(), heading))
                        .selected_text(theme::weak(ui, label))
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for (i, n) in &others {
                                if ui.selectable_label(false, n).clicked() {
                                    chosen = Some(i.clone());
                                }
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label)
                        });
                    if let Some(o) = chosen {
                        let (f, t) = if incoming {
                            (o, id.clone())
                        } else {
                            (id.clone(), o)
                        };
                        self.execute("connect", json!({ "from": f, "to": t }));
                    }
                });
            });
            ui.add_space(4.0);
        }
    }

    /// A text field that commits on Enter or losing focus; Esc restores it.
    fn text_field(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        label: &str,
        current: &str,
    ) -> Option<String> {
        let mut out = None;
        ui.horizontal(|ui| {
            form::label_cell(ui, label, 26.0);
            let mut text = self
                .drafts
                .get(key)
                .cloned()
                .unwrap_or_else(|| current.to_owned());
            let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(260.0));
            form::field_outline(ui, &r);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
            if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.drafts.remove(key);
                r.surrender_focus();
            } else if r.lost_focus() {
                self.drafts.remove(key);
                if text.trim() != current && !text.trim().is_empty() {
                    out = Some(text.trim().to_owned());
                }
            } else if r.has_focus() || r.changed() {
                self.drafts.insert(key.to_owned(), text);
            }
        });
        out
    }
}

/// A section heading in a form: more space above than below.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(18.0);
    ui.label(theme::strong(title).size(15.0));
    ui.add_space(4.0);
}

fn element_names(model: &Value) -> Vec<(String, String)> {
    model["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let id = e["id"].as_str().unwrap_or("").to_owned();
            let name = e["name"]
                .as_str()
                .filter(|n| !n.is_empty())
                .unwrap_or(&id)
                .to_owned();
            (id, name)
        })
        .collect()
}

fn kind_of(model: &Value, id: &str) -> String {
    model["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|e| e["id"] == id)
        .and_then(|e| e["kind"].as_str())
        .unwrap_or("")
        .to_owned()
}

/// A choice of block by name.
fn pick(
    ui: &mut egui::Ui,
    salt: &str,
    hint: &str,
    names: &[(String, String)],
    value: &mut Option<String>,
) {
    let shown = value
        .as_ref()
        .and_then(|v| names.iter().find(|(i, _)| i == v))
        .map_or_else(|| hint.to_owned(), |(_, n)| n.clone());
    egui::ComboBox::from_id_salt(salt)
        .selected_text(shown)
        .width(170.0)
        .show_ui(ui, |ui| {
            for (i, n) in names {
                if ui
                    .selectable_label(value.as_deref() == Some(i), n)
                    .clicked()
                {
                    *value = Some(i.clone());
                }
            }
        })
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, hint));
}
