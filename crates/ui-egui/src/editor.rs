//! Editing the model beside the canvas: the model overview (name, experiment, connections,
//! problems) and the inspector of the selected block (name, parameters, flows, its problems).
//! Every change is one engine command, so each is one undo step.

use serde_json::{Value, json};
use stochcraft_engine::COMMANDS;

use crate::form::{self, Schema};
use crate::i18n::{t, tf};
use crate::{App, theme};

/// The kinds of block that can be added, with their parameters when added.
pub const KINDS: &[(&str, &str)] = &[
    ("source", "Arrivals"),
    ("server", "Task with a resource"),
    ("shared_task", "Task with shared resources"),
    ("delay", "Task"),
    ("exclusive_gateway", "Decision (one way)"),
    ("parallel_gateway", "Split or join (all ways)"),
    ("batch", "Batch (collect, then send together)"),
    ("unbatch", "Split batches"),
    ("sink", "Departures"),
];

/// A new block's parameters: one arrival a time unit; a mean service of 0.8 time units, so arrivals
/// into a new task with a resource keep it busy 80% of the time (at 1, as many as it can serve,
/// its queue would grow without limit and the model would be warned about, #68); delay of one
/// time unit.
pub fn new_block(kind: &str) -> Value {
    match kind {
        "source" => json!({"kind": "source", "interarrival": {"type": "exponential", "mean": 1.0}}),
        "server" => {
            json!({"kind": "server", "capacity": 1, "service": {"type": "exponential", "mean": 0.8}})
        }
        "shared_task" => {
            json!({"kind": "shared_task", "service": {"type": "exponential", "mean": 1.0}, "needs": []})
        }
        "delay" => json!({"kind": "delay", "duration": {"type": "constant", "value": 1.0}}),
        "exclusive_gateway" => json!({"kind": "exclusive_gateway"}),
        "parallel_gateway" => json!({"kind": "parallel_gateway"}),
        "batch" => json!({"kind": "batch", "size": 2}),
        "unbatch" => json!({"kind": "unbatch"}),
        _ => json!({"kind": "sink"}),
    }
}

/// The parameters each kind of block has, in the order the inspector shows them.
fn parameters(kind: &str) -> &'static [&'static str] {
    match kind {
        "source" => &["interarrival", "limit"],
        "server" => &[
            "capacity",
            "shifts",
            "service",
            "processing_by",
            "queue_capacity",
            "transfer",
            "transfer_holds_sender",
            "setup",
            "planned_run_time",
        ],
        // Its needs are edited with M2.5's window work; service and priority here.
        "shared_task" => &["service", "processing_by", "priority"],
        "delay" => &["duration", "processing_by"],
        "batch" => &["size"],
        _ => &[],
    }
}

/// The parameters that change with the calendars or over time (breakdowns), shown in a section
/// of their own that starts open when any is set.
fn timed_parameters(kind: &str) -> &'static [&'static str] {
    match kind {
        "source" => &["timed", "idle_outside"],
        "server" => &["breakdown", "available", "availability"],
        _ => &[],
    }
}

/// Whether a block's parameter differs from what a block without it has (left out of the file).
fn is_set(el: &Value, f: &str) -> bool {
    match (f, &el[f]) {
        (_, Value::Null) => false,
        ("available", v) => *v == json!(false),
        ("idle_outside", v) => *v == json!(true),
        (_, Value::Array(a)) => !a.is_empty(),
        _ => true,
    }
}

/// The parameters of a command, from the engine's catalogue.
pub(crate) fn params_schema(command: &str) -> Value {
    COMMANDS
        .iter()
        .find(|c| c.name == command)
        .map(|c| c.params_schema())
        .unwrap_or(Value::Null)
}

/// A problem found by validation, for the place it concerns.
#[derive(Debug, Clone)]
pub struct Problem {
    pub error: bool,
    pub at: Option<String>,
    pub message: String,
}

impl App {
    /// The model's problems (validation is a query: nothing is journalled). Validated once per
    /// change of the model, however many views ask in a frame.
    pub(crate) fn problems(&mut self) -> Vec<Problem> {
        if let Some((model, problems)) = &self.problems_of
            && model == self.engine.model()
        {
            return problems.clone();
        }
        let problems = self.validate();
        self.problems_of = Some((self.engine.model().clone(), problems.clone()));
        problems
    }

    fn validate(&mut self) -> Vec<Problem> {
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
        if kind == "shared_task" {
            let Some(pool) = self.first_pool() else {
                return;
            };
            params["needs"] = json!([{ "resource": pool, "quantity": 1 }]);
        }
        let index = self.selected_index().map(|i| i + 1);
        if let Some(i) = index {
            params["index"] = json!(i);
        }
        if let Some(r) = self.execute("block.add", params)
            && let Some(id) = r["id"].as_str()
        {
            self.info(tf(
                "Added {block} ({id})",
                &[("block", crate::kind_text(kind)), ("id", id)],
            ));
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
            self.info(tf("Removed {id}", &[("id", &id)]));
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
                ui.set_max_width(ui.available_width().min(640.0));
                if self.is_watching() {
                    self.live_section(ui, p);
                    ui.add_space(12.0);
                }
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
                    None if self.canvas.multi.len() > 1 => self.multi_inspector(ui),
                    None => self.overview(ui, p, &model, &problems),
                }
            });
    }

    pub(crate) fn time_unit_label(&self) -> String {
        crate::time_unit(&self.engine)
    }

    fn overview(
        &mut self,
        ui: &mut egui::Ui,
        p: &theme::Palette,
        model: &Value,
        problems: &[Problem],
    ) {
        ui.heading(t("Model"));
        ui.add_space(8.0);
        // The name.
        let name = model["name"].as_str().unwrap_or("").to_owned();
        if let Some(n) = self.text_field(ui, "model/name", t("Name"), &name) {
            self.execute("model.rename", json!({ "name": n }));
        }

        section(ui, t("Experiment"));
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
                &x[f],
                time,
            ) {
                self.execute("experiment.set", json!({ f: v }));
            }
        }
        ui.label(theme::weak(
            ui,
            t("Changing the time unit does not convert the times already in the model."),
        ));
        self.stop_row(ui, model);

        section(ui, t("Connections"));
        self.connections_table(ui, p, model);

        self.pools_section(ui, model);
        self.calendars_section(ui, model);

        section(ui, t("Problems"));
        if problems.is_empty() {
            ui.label(theme::weak(ui, t("None found. The model is ready to run.")));
        }
        for pr in problems {
            self.problem_row(ui, p, pr, true);
        }
    }

    fn problem_row(&mut self, ui: &mut egui::Ui, p: &theme::Palette, pr: &Problem, link: bool) {
        ui.horizontal_wrapped(|ui| {
            let (word, color) = if pr.error {
                (t("Error"), p.error)
            } else {
                (t("Warning"), p.warning)
            };
            ui.label(theme::strong(word).color(color));
            // The engine's message starts "error at `id`: " or "error: "; the word is the label
            // and the place is the inspector or the Show link, so only the rest is shown.
            let message = ["error at `", "warning at `"]
                .iter()
                .find_map(|head| {
                    pr.message
                        .strip_prefix(head)
                        .and_then(|rest| rest.split_once("`: "))
                        .map(|(_, m)| m)
                })
                .or_else(|| pr.message.strip_prefix("error: "))
                .or_else(|| pr.message.strip_prefix("warning: "))
                .unwrap_or(&pr.message);
            ui.label(message);
            if link
                && let Some(at) = &pr.at
                && ui.link(tf("Show {id}", &[("id", at)])).clicked()
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
            ui.label(theme::weak(ui, t("No connections yet: join blocks below.")));
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
                        .add(theme::secondary(ui, t("Remove")))
                        .on_hover_text(tf("Remove the flow {id}", &[("id", id)]))
                        .clicked()
                    {
                        self.execute("disconnect", json!({ "id": id }));
                    }
                    ui.end_row();
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            pick(
                ui,
                "connect/from",
                t("From"),
                &names,
                &mut self.connect_from,
            );
            ui.label(theme::weak(ui, "→"));
            pick(ui, "connect/to", t("To"), &names, &mut self.connect_to);
            let ready = self.connect_from.is_some() && self.connect_to.is_some();
            if ui
                .add_enabled(ready, theme::secondary(ui, t("Connect")))
                .clicked()
                && let (Some(f), Some(t)) = (self.connect_from.take(), self.connect_to.take())
            {
                self.execute("connect", json!({ "from": f, "to": t }));
            }
        });
        let _ = p;
    }

    /// A block's parameters `fields`, each drawn from `block.set`'s schema; an edit is one
    /// `block.set`.
    fn block_fields(
        &mut self,
        ui: &mut egui::Ui,
        model: &Value,
        el: &Value,
        id: &str,
        fields: &[&str],
    ) {
        let schema = params_schema("block.set");
        let unit = self.time_unit_label();
        let calendars = form::calendars(model);
        for f in fields {
            // Whether the sender waits applies only to a transfer.
            if *f == "transfer_holds_sender" && el["transfer"].is_null() {
                continue;
            }
            // Settings the file leaves out when they have their default.
            let current = match (*f, &el[*f]) {
                ("available" | "transfer_holds_sender", Value::Null) => json!(true),
                ("idle_outside", Value::Null) => json!(false),
                (_, v) => v.clone(),
            };
            let mut cx = form::Cx {
                time_unit: &unit,
                numbers: &self.numbers,
                drafts: &mut self.drafts,
                calendars: &calendars,
            };
            let key = format!("{id}/{f}");
            let time = matches!(
                *f,
                "interarrival" | "service" | "duration" | "transfer" | "planned_run_time"
            );
            if let Some(v) = form::field(ui, &mut cx, Schema::new(&schema), &key, f, &current, time)
            {
                self.execute("block.set", json!({ "id": id, *f: v }));
            }
        }
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
            .map_or(kind.as_str(), |k| t(k.1));
        ui.horizontal(|ui| {
            if ui
                .link(t("Model"))
                .on_hover_text(t("Back to the model (Esc)"))
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
                    .add(theme::secondary(ui, t("Remove block")))
                    .on_hover_text(t("Remove this block and its flows (Delete)"))
                    .clicked()
                {
                    self.remove_selected();
                }
            });
        });
        ui.label(theme::weak(ui, tf("id {id}", &[("id", &id)])).monospace());
        ui.add_space(8.0);

        // The block's own problems, and those of the flows into and out of it.
        let flows = flows_of(model, &id);
        for pr in problems.iter().filter(|pr| {
            pr.at
                .as_deref()
                .is_some_and(|at| at == id || flows.iter().any(|f| f == at))
        }) {
            self.problem_row(ui, p, pr, false);
        }

        if let Some(n) = self.text_field(ui, &format!("{id}/name"), t("Name"), &name) {
            self.execute("block.rename", json!({ "id": id, "name": n }));
        }

        // Units by calendar need a calendar: shown once the model has one (or the server has
        // some already).
        let by_calendar = !form::calendars(model).is_empty() || is_set(el, "shifts");
        let fields: Vec<&str> = parameters(&kind)
            .iter()
            .copied()
            .filter(|f| *f != "shifts" || by_calendar)
            .collect();
        if !fields.is_empty() {
            section(ui, t("Parameters"));
            self.block_fields(ui, model, el, &id, &fields);
        }
        // How a finite waiting room's size is set, once there is one.
        if kind == "server" && el["queue_capacity"].is_number() {
            self.waiting_room_choice(ui, model, el, &id);
        }
        // What a job under way does when its server goes off shift, once servers can.
        if kind == "server"
            && ["shifts", "available", "availability"]
                .iter()
                .any(|f| is_set(el, f))
        {
            self.shift_end_choice(ui, el, &id);
        }
        let more = timed_parameters(&kind);
        if !more.is_empty() {
            let title = if kind == "server" {
                t("Breakdowns and availability")
            } else {
                t("Arrivals by calendar")
            };
            ui.add_space(6.0);
            egui::CollapsingHeader::new(theme::strong(title))
                .id_salt(format!("{id}/timed"))
                .default_open(more.iter().any(|f| is_set(el, f)))
                .show(ui, |ui| self.block_fields(ui, model, el, &id, more));
        }

        if kind == "shared_task" {
            self.needs_section(ui, model, el);
        }
        if kind == "sink" {
            self.demand_section(ui, model, el, &id);
            self.scrap_section(ui, model, el, &id);
        }

        if matches!(kind.as_str(), "source" | "server" | "shared_task" | "delay") {
            self.estimate_section(ui, el);
        }

        self.properties_section(ui, p, el);
        // Patience, next to the flows entities that give up take; closed on a server without one.
        if kind == "server" {
            ui.add_space(6.0);
            egui::CollapsingHeader::new(theme::strong(t("Patience")))
                .id_salt(format!("{id}/patience"))
                .default_open(el["patience"].is_object())
                .show(ui, |ui| self.patience_section(ui, model, el, &id));
            ui.add_space(6.0);
            egui::CollapsingHeader::new(theme::strong(t("Health check")))
                .id_salt(format!("{id}/health_check"))
                .default_open(el["health_check"].is_object())
                .show(ui, |ui| self.health_check_section(ui, model, el, &id));
        }

        section(ui, t("Flows"));
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
        let decision = kind == "exclusive_gateway";
        if decision {
            ui.label(theme::weak(
                ui,
                "An entity takes the first flow whose condition holds; with no conditions, a flow \
                 drawn by the probabilities (blank ones share what the others leave).",
            ));
            ui.add_space(4.0);
        }
        for (heading, incoming) in [(t("Comes from"), true), (t("Goes to"), false)] {
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
                        ui.label(theme::weak(ui, t("nothing yet")));
                    }
                    for (cid, f, to) in mine {
                        let other = if incoming { f } else { to };
                        ui.horizontal(|ui| {
                            if ui.link(name_of(other)).clicked() {
                                self.selected = Some(other.clone());
                            }
                            if ui
                                .add(
                                    egui::Button::new(egui::RichText::new(t("Remove")).size(12.0))
                                        .stroke(egui::Stroke::new(1.0, p.border)),
                                )
                                .on_hover_text(tf("Remove the flow {id}", &[("id", cid)]))
                                .clicked()
                            {
                                self.execute("disconnect", json!({ "id": cid }));
                            }
                        });
                        // A way out: a decision's probability and condition, and for any whether
                        // it is rework, on lines of their own so the row fits the inspector.
                        if !incoming && kind != "source" {
                            let c = connection(model, cid);
                            if decision {
                                ui.horizontal(|ui| {
                                    ui.add_space(12.0);
                                    self.flow_fields(ui, cid, &c);
                                });
                            }
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                self.rework_field(ui, cid, &c);
                                // Only a task with a resource of its own has entities giving up.
                                if kind == "server" {
                                    self.abandoned_field(ui, cid, &c);
                                }
                                // Only one with a finite waiting room turns entities away.
                                if kind == "server" && el["queue_capacity"].is_number() {
                                    self.turned_away_field(ui, cid, &c);
                                }
                            });
                        }
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
                        t("Add a flow from…")
                    } else {
                        t("Add a flow to…")
                    };
                    craft_ui::ComboBox::from_id_salt((id.as_str(), heading))
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
        if decision {
            self.default_flow(ui, &id, el, &conns, &names);
        }
    }

    /// The customer demand a sink serves, for takt: how many parts, per run, day, week or
    /// shift, and whether the time available is the whole clock or a calendar's active time.
    fn demand_section(&mut self, ui: &mut egui::Ui, model: &Value, el: &Value, id: &str) {
        section(ui, t("Takt"));
        let demand = el["demand"].clone();
        let mut on = demand.is_object();
        if ui
            .checkbox(&mut on, t("Customer demand"))
            .on_hover_text(t(
                "How many parts customers want from these departures; the results then show takt (the time available over the demand) and each task's cycle time.",
            ))
            .changed()
        {
            let v = if on {
                json!({"quantity": 100.0, "per": "day"})
            } else {
                Value::Null
            };
            self.execute("block.set", json!({ "id": id, "demand": v }));
        }
        if !on {
            return;
        }
        let set = |d: &Value, key: &str, v: Value| {
            let mut d = d.clone();
            if v.is_null() {
                if let Some(o) = d.as_object_mut() {
                    o.remove(key);
                }
            } else {
                d[key] = v;
            }
            d
        };
        let q_now = demand["quantity"]
            .as_f64()
            .map(crate::results::grouped)
            .unwrap_or_default();
        if let Some(text) = self.text_field(ui, &format!("{id}/demand"), t("Parts wanted"), &q_now)
        {
            match self.numbers.parse(&text) {
                Ok(q) if q.is_finite() && q > 0.0 => {
                    let d = set(&demand, "quantity", json!(q));
                    self.execute("block.set", json!({ "id": id, "demand": d }));
                }
                _ => self.report_error(t("Parts wanted is a number above 0")),
            }
        }
        let pers = [
            ("run", t("for the whole run")),
            ("day", t("per day")),
            ("week", t("per week")),
            ("shift", t("per shift")),
        ];
        let per_now = demand["per"].as_str().unwrap_or("day").to_owned();
        let mut chosen_per: Option<&str> = None;
        ui.horizontal(|ui| {
            form::label_cell(ui, t("Per"), 26.0);
            let shown = pers
                .iter()
                .find(|(k, _)| *k == per_now)
                .map_or(per_now.as_str(), |(_, l)| l);
            craft_ui::ComboBox::from_id_salt((id, "per"))
                .selected_text(shown)
                .width(200.0)
                .show_ui(ui, |ui| {
                    for (k, l) in &pers {
                        if ui.selectable_label(per_now == *k, *l).clicked() {
                            chosen_per = Some(k);
                        }
                    }
                })
                .response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Per")));
        });
        if let Some(k) = chosen_per.filter(|k| *k != per_now) {
            let d = set(&demand, "per", json!(k));
            self.execute("block.set", json!({ "id": id, "demand": d }));
        }
        let calendars: Vec<(String, String)> = model["calendars"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                let i = c["id"].as_str().unwrap_or("").to_owned();
                let n = c["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&i)
                    .to_owned();
                (i, n)
            })
            .collect();
        let cal_now = demand["calendar"].as_str().map(str::to_owned);
        let clock = t("The whole clock");
        let shown = cal_now
            .as_ref()
            .map(|c| {
                calendars
                    .iter()
                    .find(|(i, _)| i == c)
                    .map_or_else(|| c.clone(), |(_, n)| n.clone())
            })
            .unwrap_or_else(|| clock.to_owned());
        let mut chosen_cal: Option<Option<String>> = None;
        ui.horizontal(|ui| {
            form::label_cell(ui, t("Time available"), 26.0);
            craft_ui::ComboBox::from_id_salt((id, "available"))
                .selected_text(shown)
                .width(200.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(cal_now.is_none(), clock).clicked() {
                        chosen_cal = Some(None);
                    }
                    for (i, n) in &calendars {
                        if ui
                            .selectable_label(cal_now.as_deref() == Some(i), n)
                            .clicked()
                        {
                            chosen_cal = Some(Some(i.clone()));
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Time available"))
                });
        });
        if let Some(c) = chosen_cal.filter(|c| *c != cal_now) {
            let d = set(&demand, "calendar", c.map_or(Value::Null, |c| json!(c)));
            self.execute("block.set", json!({ "id": id, "demand": d }));
        }
        if per_now == "shift" && cal_now.is_none() {
            ui.label(theme::weak(
                ui,
                t("Per shift needs a calendar: each period it is active is a shift."),
            ));
        }
    }

    /// Whether what reaches a sink is scrap, for the OEE quality results, and which server it
    /// counts against: the last one each part finished at, or one named (an inspection that
    /// rejects work done upstream names the machine that did it).
    fn scrap_section(&mut self, ui: &mut egui::Ui, model: &Value, el: &Value, id: &str) {
        section(ui, t("Quality"));
        let scrap = &el["scrap"];
        let mut on = scrap.is_object();
        if ui
            .checkbox(&mut on, t("Scrap"))
            .on_hover_text(t(
                "What reaches these departures is scrap, counted in the quality ratio of the task it counts against.",
            ))
            .changed()
        {
            let v = if on { json!({}) } else { Value::Null };
            self.execute("block.set", json!({ "id": id, "scrap": v }));
        }
        if !on {
            return;
        }
        let servers: Vec<(String, String)> = model["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["kind"] == "server")
            .map(|e| {
                let i = e["id"].as_str().unwrap_or("").to_owned();
                let n = e["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&i)
                    .to_owned();
                (i, n)
            })
            .collect();
        let now = scrap["charged_to"].as_str().map(str::to_owned);
        let last = t("The last task with a resource it finished at");
        let shown = now
            .as_ref()
            .map(|c| {
                servers
                    .iter()
                    .find(|(i, _)| i == c)
                    .map_or_else(|| c.clone(), |(_, n)| n.clone())
            })
            .unwrap_or_else(|| last.to_owned());
        let mut chosen: Option<Option<String>> = None;
        ui.horizontal(|ui| {
            form::label_cell(ui, t("Counts against"), 26.0);
            craft_ui::ComboBox::from_id_salt((id, "charged_to"))
                .selected_text(shown)
                .width(220.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(now.is_none(), last).clicked() {
                        chosen = Some(None);
                    }
                    for (i, n) in &servers {
                        if ui.selectable_label(now.as_deref() == Some(i), n).clicked() {
                            chosen = Some(Some(i.clone()));
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Counts against"))
                });
        });
        if let Some(c) = chosen.filter(|c| *c != now) {
            let v = match c {
                Some(c) => json!({ "charged_to": c }),
                None => json!({}),
            };
            self.execute("block.set", json!({ "id": id, "scrap": v }));
        }
    }

    /// A decision's default flow: taken when no condition holds. Without one, such an entity
    /// leaves the model at the gateway (with a warning).
    fn default_flow(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        el: &Value,
        conns: &[(String, String, String)],
        names: &[(String, String)],
    ) {
        let outs: Vec<(String, String)> = conns
            .iter()
            .filter(|(_, f, _)| f == id)
            .map(|(cid, _, t)| {
                let to = names
                    .iter()
                    .find(|(i, _)| i == t)
                    .map_or_else(|| t.clone(), |(_, n)| n.clone());
                (cid.clone(), tf("to {block}", &[("block", &to)]))
            })
            .collect();
        let now = el["default"].as_str().map(str::to_owned);
        let shown = now
            .as_ref()
            .and_then(|d| outs.iter().find(|(c, _)| c == d))
            .map_or_else(|| t("None").to_owned(), |(_, n)| n.clone());
        let mut chosen: Option<Option<String>> = None;
        ui.horizontal(|ui| {
            form::label_cell(ui, t("Default flow"), 26.0);
            craft_ui::ComboBox::from_id_salt((id, "default"))
                .selected_text(shown)
                .width(200.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(now.is_none(), t("None")).clicked() {
                        chosen = Some(None);
                    }
                    for (c, n) in &outs {
                        if ui.selectable_label(now.as_deref() == Some(c), n).clicked() {
                            chosen = Some(Some(c.clone()));
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Default flow"))
                });
        });
        if let Some(d) = chosen.filter(|d| *d != now) {
            self.execute("block.set", json!({ "id": id, "default": d }));
        }
    }

    /// A decision's way out: its probability and condition, each blank when unset. Each commits
    /// on Enter or leaving the field, as one `flow.set`.
    fn flow_fields(&mut self, ui: &mut egui::Ui, cid: &str, c: &Value) {
        let p_now = c["probability"]
            .as_f64()
            .map(crate::results::grouped)
            .unwrap_or_default();
        if let Some(t) = self.small_field(
            ui,
            &format!("{cid}/p"),
            &tf("Probability of {id}", &[("id", cid)]),
            "p",
            &p_now,
            56.0,
        ) {
            let t = t.trim();
            if t.is_empty() {
                self.execute("flow.set", json!({ "id": cid, "probability": null }));
            } else {
                // As the user's region writes it ("0,5" in German), or with a point.
                match self.numbers.parse(t) {
                    Ok(v) if (0.0..=1.0).contains(&v) => {
                        self.execute("flow.set", json!({ "id": cid, "probability": v }));
                    }
                    _ => self.report_error(tf(
                        "{id}: a probability is a number from 0 to 1",
                        &[("id", cid)],
                    )),
                }
            }
        }
        let c_now = c["condition"].as_str().unwrap_or("").to_owned();
        if let Some(t) = self.small_field(
            ui,
            &format!("{cid}/c"),
            &tf("Condition of {id}", &[("id", cid)]),
            "if",
            &c_now,
            200.0,
        ) {
            let t = t.trim();
            let v = if t.is_empty() { Value::Null } else { json!(t) };
            self.execute("flow.set", json!({ "id": cid, "condition": v }));
        }
    }

    /// Whether parts sent this way go back to be worked on again: rework, for the quality
    /// results.
    fn rework_field(&mut self, ui: &mut egui::Ui, cid: &str, c: &Value) {
        let mut rework = c["rework"].as_bool().unwrap_or(false);
        let r = ui.checkbox(&mut rework, t("Rework")).on_hover_text(t(
            "Parts taking this flow go back to be worked on again; they count as rework against the last task with a resource they finished at.",
        ));
        let (ticked, name) = (rework, tf("Rework on {id}", &[("id", cid)]));
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, ticked, &name));
        if r.changed() {
            self.execute("flow.set", json!({ "id": cid, "rework": rework }));
        }
    }

    /// Whether this way out is the one entities that give up take, besides the task's ordinary
    /// way out.
    fn abandoned_field(&mut self, ui: &mut egui::Ui, cid: &str, c: &Value) {
        let mut abandoned = c["abandoned"].as_bool().unwrap_or(false);
        let r = ui.checkbox(&mut abandoned, t("Abandoned")).on_hover_text(t(
            "Entities that give up waiting take this flow (to try again later, say); the others take the task's other way out.",
        ));
        let (ticked, name) = (abandoned, tf("Abandoned on {id}", &[("id", cid)]));
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, ticked, &name));
        if r.changed() {
            self.execute("flow.set", json!({ "id": cid, "abandoned": abandoned }));
        }
    }

    /// Whether this way out is the one entities finding no room in the task's waiting room take,
    /// besides the task's ordinary way out.
    fn turned_away_field(&mut self, ui: &mut egui::Ui, cid: &str, c: &Value) {
        let mut turned = c["turned_away"].as_bool().unwrap_or(false);
        let r = ui.checkbox(&mut turned, t("Turned away")).on_hover_text(t(
            "Entities finding the waiting room full take this flow at once (to try again later, say), instead of being lost or waiting before the task; they count as lost here.",
        ));
        let (ticked, name) = (turned, tf("Turned away on {id}", &[("id", cid)]));
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, ticked, &name));
        if r.changed() {
            self.execute("flow.set", json!({ "id": cid, "turned_away": turned }));
        }
    }

    /// How a server's finite waiting room is set: a fixed number, a number by calendar (with
    /// the table of them), or a number per working server.
    fn waiting_room_choice(&mut self, ui: &mut egui::Ui, model: &Value, el: &Value, id: &str) {
        let by_calendar = el["queue_shifts"].as_array().is_some_and(|a| !a.is_empty());
        let current = if el["queue_per_working_server"] == json!(true) {
            "per_working_server"
        } else if by_calendar {
            "by_calendar"
        } else {
            "fixed"
        };
        let mut chosen = current.to_owned();
        ui.label(t("Size of the waiting room:"));
        ui.radio_value(&mut chosen, "fixed".to_owned(), t("Fixed"))
            .on_hover_text(t(
                "This many may wait, besides a place for each server; a failed server keeps its place.",
            ));
        // A timetable needs a calendar: offered once the model has one.
        let calendars = form::calendars(model);
        if !calendars.is_empty() || by_calendar {
            ui.radio_value(&mut chosen, "by_calendar".to_owned(), t("By calendar"))
                .on_hover_text(t(
                    "This many may wait outside the calendars, and each calendar's number while it is active. When the number drops, those already waiting stay.",
                ));
        }
        ui.radio_value(
            &mut chosen,
            "per_working_server".to_owned(),
            t("Per working server"),
        )
        .on_hover_text(t(
            "This many may wait for each server working now (not failed, not taken out, on shift), so the waiting room follows them. When it shrinks, those already waiting stay.",
        ));
        if chosen != current {
            let v = match chosen.as_str() {
                "per_working_server" => json!({ "id": id, "queue_per_working_server": true }),
                "by_calendar" => {
                    // A first row: the first calendar, with the number outside.
                    let first = calendars
                        .first()
                        .map(|(c, _)| c.clone())
                        .unwrap_or_default();
                    let places = el["queue_capacity"].as_u64().unwrap_or(0);
                    json!({ "id": id, "queue_shifts": [{ "calendar": first, "places": places }] })
                }
                _ => json!({ "id": id, "queue_per_working_server": false, "queue_shifts": [] }),
            };
            self.execute("block.set", v);
        }
        if current == "by_calendar" {
            self.block_fields(ui, model, el, id, &["queue_shifts"]);
        }
    }

    /// How long entities wait at a task with a resource before they give up, and whether they
    /// give up only before their service starts or also before it is done.
    fn patience_section(&mut self, ui: &mut egui::Ui, model: &Value, el: &Value, id: &str) {
        let patience = el["patience"].clone();
        let mut on = patience.is_object();
        let r = ui.checkbox(&mut on, t("Entities give up waiting")).on_hover_text(t(
            "Each entity waits here at most a time drawn for it as it arrives, then gives up: it takes the way out marked Abandoned, or leaves the model.",
        ));
        if r.changed() {
            let v = if on {
                json!({"time": {"type": "exponential", "mean": 1.0}})
            } else {
                Value::Null
            };
            self.execute("block.set", json!({ "id": id, "patience": v }));
        }
        if !on {
            return;
        }
        // The time's editor, from the schema of block.set's patience.
        let schema = params_schema("block.set");
        let resolve = |n: &Value| -> Value {
            let n = n["anyOf"]
                .as_array()
                .and_then(|a| a.iter().find(|x| x["type"] != "null"))
                .unwrap_or(n);
            match n["$ref"].as_str().and_then(|r| r.strip_prefix("#/$defs/")) {
                Some(name) => schema["$defs"][name].clone(),
                None => n.clone(),
            }
        };
        let spec = resolve(&schema["properties"]["patience"]);
        let root = json!({
            "properties": { "patience": spec["properties"]["time"] },
            "$defs": schema["$defs"],
        });
        let unit = self.time_unit_label();
        let calendars = form::calendars(model);
        let mut cx = form::Cx {
            time_unit: &unit,
            numbers: &self.numbers,
            drafts: &mut self.drafts,
            calendars: &calendars,
        };
        let key = format!("{id}/patience");
        let time = &patience["time"];
        if let Some(v) = form::field(
            ui,
            &mut cx,
            Schema::new(&root),
            &key,
            "patience",
            time,
            true,
        ) {
            let mut p = patience.clone();
            p["time"] = v;
            self.execute("block.set", json!({ "id": id, "patience": p }));
        }
        let until = patience["until"].as_str().unwrap_or("start").to_owned();
        let mut chosen = until.clone();
        ui.radio_value(
            &mut chosen,
            "start".to_owned(),
            t("Gives up before service starts"),
        )
        .on_hover_text(t(
            "A caller on hold: once a unit takes it, the entity stays.",
        ));
        ui.radio_value(&mut chosen, "done".to_owned(), t("Gives up before it is done"))
            .on_hover_text(t(
                "A client that times out: if its time runs out during service, it gives up then, and the unit finishes the job anyway (wasted work).",
            ));
        if chosen != until {
            let mut p = patience.clone();
            if chosen == "start" {
                if let Some(o) = p.as_object_mut() {
                    o.remove("until");
                }
            } else {
                p["until"] = json!(chosen);
            }
            self.execute("block.set", json!({ "id": id, "patience": p }));
        }
    }

    /// What happens to a job under way on a server that goes off shift (fewer on shift, or the
    /// availability ending): it finishes first, is retried elsewhere, or resumes when the
    /// server is back.
    fn shift_end_choice(&mut self, ui: &mut egui::Ui, el: &Value, id: &str) {
        let current = el["on_shift_end"].as_str().unwrap_or("finish").to_owned();
        let mut chosen = current.clone();
        ui.label(t("Job under way on a server going off shift:"));
        ui.radio_value(&mut chosen, "finish".to_owned(), t("Finish, then go off"))
            .on_hover_text(t(
                "The server finishes the job it has, then goes off; until then it counts as on shift.",
            ));
        ui.radio_value(&mut chosen, "retry".to_owned(), t("Retry elsewhere"))
            .on_hover_text(t(
                "The job stops at once: the entity goes back to the head of the queue for any server on shift, and the work done on it is wasted.",
            ));
        ui.radio_value(&mut chosen, "resume".to_owned(), t("Resume when back on shift"))
            .on_hover_text(t(
                "The job stops at once; the entity waits on its server and the job carries on with what was left once that server is on shift again.",
            ));
        if chosen != current {
            self.execute("block.set", json!({ "id": id, "on_shift_end": chosen }));
        }
    }

    /// Whether a task's units are checked and taken out for a restart when too many entities
    /// wait (a load balancer's health check), and how.
    fn health_check_section(&mut self, ui: &mut egui::Ui, model: &Value, el: &Value, id: &str) {
        let check = el["health_check"].clone();
        let mut on = check.is_object();
        let r = ui
            .checkbox(&mut on, t("Take servers out when overloaded"))
            .on_hover_text(t(
                "Each server (unit) is checked every so often, as a load balancer probes the servers behind it. One that fails, with too many waiting, is taken out of service for a restart, then comes back.",
            ));
        if r.changed() {
            let v = if on {
                json!({
                    "interval": {"type": "exponential", "mean": 1.0},
                    "limit": 2.0,
                    "restart": {"type": "constant", "value": 1.0},
                })
            } else {
                Value::Null
            };
            self.execute("block.set", json!({ "id": id, "health_check": v }));
        }
        if !on {
            return;
        }
        // The times' editors, from the schema of block.set's health check; the counts are
        // plain numbers (a keep of 0 is no floor).
        let schema = params_schema("block.set");
        let resolve = |n: &Value| -> Value {
            let n = n["anyOf"]
                .as_array()
                .and_then(|a| a.iter().find(|x| x["type"] != "null"))
                .unwrap_or(n);
            match n["$ref"].as_str().and_then(|r| r.strip_prefix("#/$defs/")) {
                Some(name) => schema["$defs"][name].clone(),
                None => n.clone(),
            }
        };
        let spec = resolve(&schema["properties"]["health_check"]);
        let root = json!({
            "properties": {
                "check_interval": spec["properties"]["interval"],
                "check_limit": {
                    "type": "number",
                    "minimum": 0.0,
                    "description": "A server fails its check when more entities wait than this per server working (a probe's timeout times one server's service rate).",
                },
                "restart": spec["properties"]["restart"],
                "keep_in_service": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "A failed check takes no server out when that would leave fewer than this working; 0: no floor.",
                },
            },
            "$defs": schema["$defs"],
        });
        let unit = self.time_unit_label();
        let calendars = form::calendars(model);
        let mut changed = None;
        for (name, wire, time) in [
            ("check_interval", "interval", true),
            ("check_limit", "limit", false),
            ("restart", "restart", true),
            ("keep_in_service", "keep", false),
        ] {
            let current = match wire {
                "keep" if check[wire].is_null() => json!(0),
                _ => check[wire].clone(),
            };
            let mut cx = form::Cx {
                time_unit: &unit,
                numbers: &self.numbers,
                drafts: &mut self.drafts,
                calendars: &calendars,
            };
            let key = format!("{id}/health_check/{name}");
            if let Some(v) =
                form::field(ui, &mut cx, Schema::new(&root), &key, name, &current, time)
            {
                changed = Some((wire, v));
            }
        }
        let on_takeout = check["on_takeout"].as_str().unwrap_or("retry").to_owned();
        let mut chosen = on_takeout.clone();
        ui.label(t("Job under way on a server taken out:"));
        ui.radio_value(&mut chosen, "retry".to_owned(), t("Retry elsewhere"))
            .on_hover_text(t(
                "The connection drops and the client tries again: the entity goes back to the head of the queue at once, and the work done on it is wasted.",
            ));
        ui.radio_value(&mut chosen, "resume".to_owned(), t("Resume after restart"))
            .on_hover_text(t(
                "The entity waits on its server and the job carries on with what was left once the server is back.",
            ));
        if chosen != on_takeout {
            changed = Some((
                "on_takeout",
                if chosen == "retry" {
                    Value::Null
                } else {
                    json!(chosen)
                },
            ));
        }
        // Only a fixed finite buffer has places to keep.
        if !el["queue_capacity"].is_null()
            && el["queue_per_working_server"] != json!(true)
            && el["queue_shifts"].as_array().is_none_or(|a| a.is_empty())
        {
            let place = check["place_while_out"]
                .as_str()
                .unwrap_or("reserved")
                .to_owned();
            let mut chosen = place.clone();
            ui.label(t("Place in the waiting room of a server taken out:"));
            ui.radio_value(&mut chosen, "reserved".to_owned(), t("Kept while it restarts"))
                .on_hover_text(t(
                    "As a failed server's: it keeps a place free of entities until it is back, so the waiting room holds fewer while servers are out.",
                ));
            ui.radio_value(&mut chosen, "open".to_owned(), t("Open to entities"))
                .on_hover_text(t(
                    "As a server off shift's: entities may take its place while it is out.",
                ));
            if chosen != place {
                changed = Some((
                    "place_while_out",
                    if chosen == "reserved" {
                        Value::Null
                    } else {
                        json!(chosen)
                    },
                ));
            }
        }
        if let Some((wire, v)) = changed {
            let mut c = check.clone();
            if let Some(o) = c.as_object_mut() {
                // No floor, retrying, and a kept place, are left out.
                if v.is_null() || (wire == "keep" && v == json!(0)) {
                    o.remove(wire);
                } else {
                    o.insert(wire.to_owned(), v);
                }
            }
            self.execute("block.set", json!({ "id": id, "health_check": c }));
        }
    }

    /// A small labelled text field ("p", "if") that commits on Enter or leaving it, when changed;
    /// Esc restores it.
    fn small_field(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        name: &str,
        prefix: &str,
        current: &str,
        width: f32,
    ) -> Option<String> {
        ui.label(theme::weak(ui, prefix));
        let mut text = self
            .drafts
            .get(key)
            .cloned()
            .unwrap_or_else(|| current.to_owned());
        let r = ui.add(
            egui::TextEdit::singleline(&mut text)
                .desired_width(width)
                .font(egui::TextStyle::Monospace),
        );
        form::field_outline(ui, &r);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, name));
        if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.drafts.remove(key);
            r.surrender_focus();
            None
        } else if r.lost_focus() {
            self.drafts.remove(key);
            (text.trim() != current.trim()).then_some(text)
        } else {
            if r.has_focus() || r.changed() {
                self.drafts.insert(key.to_owned(), text);
            }
            None
        }
    }

    /// The properties this block sets on every entity entering it: name, type and value, each
    /// edited in place; a row to add one.
    fn properties_section(&mut self, ui: &mut egui::Ui, p: &theme::Palette, el: &Value) {
        let id = el["id"].as_str().unwrap_or("").to_owned();
        section(ui, t("Properties"));
        ui.label(theme::weak(
            ui,
            t(
                "Set on every entity as it enters this block. Expressions read the values it \
               arrived with, such as getProperty('issues') - 1.",
            ),
        ));
        ui.add_space(6.0);
        let schema = params_schema("property.set");
        let unit = self.time_unit_label();
        let props: Vec<Value> = el["properties"].as_array().cloned().unwrap_or_default();
        for prop in &props {
            let name = prop["name"].as_str().unwrap_or("").to_owned();
            ui.horizontal(|ui| {
                ui.label(theme::strong(&name).monospace());
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new(t("Remove")).size(12.0))
                            .stroke(egui::Stroke::new(1.0, p.border)),
                    )
                    .on_hover_text(tf("Stop setting {name}", &[("name", &name)]))
                    .clicked()
                {
                    self.execute("property.remove", json!({ "element": id, "name": name }));
                }
            });
            ui.indent(("prop", &id, &name), |ui| {
                let mut cx = form::Cx {
                    time_unit: &unit,
                    numbers: &self.numbers,
                    drafts: &mut self.drafts,
                    calendars: &[],
                };
                let base = format!("{id}/prop/{name}");
                let sch = Schema::new(&schema);
                if let Some(t) = form::field(
                    ui,
                    &mut cx,
                    sch,
                    &format!("{base}/type"),
                    "type",
                    &prop["type"],
                    false,
                ) {
                    self.execute(
                        "property.set",
                        json!({ "element": id, "name": name, "type": t, "value": prop["value"] }),
                    );
                    return;
                }
                let mut cx = form::Cx {
                    time_unit: &unit,
                    numbers: &self.numbers,
                    drafts: &mut self.drafts,
                    calendars: &[],
                };
                if let Some(v) = form::field(
                    ui,
                    &mut cx,
                    sch,
                    &format!("{base}/value"),
                    "value",
                    &prop["value"],
                    false,
                ) {
                    self.execute(
                        "property.set",
                        json!({ "element": id, "name": name, "value": v }),
                    );
                }
            });
            ui.add_space(4.0);
        }
        // A new property: its name, then Add (it starts as the number 0).
        let key = format!("{id}/prop/new");
        let mut cx = form::Cx {
            time_unit: &unit,
            numbers: &self.numbers,
            drafts: &mut self.drafts,
            calendars: &[],
        };
        let mut add = None;
        ui.horizontal(|ui| {
            form::label_cell(ui, theme::weak(ui, t("New property")), 26.0);
            let committed =
                form::text(ui, &mut cx, &key, t("New property name"), &json!(""), 180.0);
            let typed = cx.drafts.get(&key).cloned().unwrap_or_default();
            let name = committed
                .as_ref()
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or(typed);
            let enter = committed.is_some() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui
                .add_enabled(
                    !name.trim().is_empty(),
                    theme::secondary(ui, t("Add property")),
                )
                .clicked()
                || (enter && !name.trim().is_empty())
            {
                add = Some(name.trim().to_owned());
            } else if let Some(n) = committed.and_then(|v| v.as_str().map(str::to_owned)) {
                // Leaving the field keeps the name for the Add button.
                cx.drafts.insert(key.clone(), n);
            }
        });
        if let Some(name) = add {
            self.drafts.remove(&key);
            self.execute(
                "property.set",
                json!({ "element": id, "name": name, "value": {"number": 0.0} }),
            );
        }
    }

    /// A text field that commits on Enter or losing focus; Esc restores it.
    pub(crate) fn text_field(
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
pub(crate) fn section(ui: &mut egui::Ui, title: &str) {
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

/// The ids of the connections into and out of element `id`.
pub(crate) fn flows_of(model: &Value, id: &str) -> Vec<String> {
    model["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["from"] == id || c["to"] == id)
        .filter_map(|c| c["id"].as_str().map(str::to_owned))
        .collect()
}

/// A connection's JSON by id.
fn connection(model: &Value, id: &str) -> Value {
    model["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|c| c["id"] == id)
        .cloned()
        .unwrap_or_default()
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
    craft_ui::ComboBox::from_id_salt(salt)
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
