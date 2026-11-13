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
            "service",
            "queue_capacity",
            "transfer",
            "transfer_holds_sender",
            "setup",
            "planned_run_time",
        ],
        // Its needs are edited with M2.5's window work; service and priority here.
        "shared_task" => &["service", "priority"],
        "delay" => &["duration"],
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

        let fields = parameters(&kind);
        if !fields.is_empty() {
            section(ui, t("Parameters"));
            self.block_fields(ui, model, el, &id, fields);
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
            self.scrap_section(ui, model, el, &id);
        }

        if matches!(kind.as_str(), "source" | "server" | "shared_task" | "delay") {
            self.estimate_section(ui, el);
        }

        self.properties_section(ui, p, el);

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
                        // A decision's way out: its probability, condition and whether it is
                        // rework, on a line of their own so the row fits the inspector.
                        if decision && !incoming {
                            let c = connection(model, cid);
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                self.flow_fields(ui, cid, &c);
                            });
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                self.rework_field(ui, cid, &c);
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
