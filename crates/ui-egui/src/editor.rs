//! Editing the model beside the canvas: the properties of the selection. With nothing selected,
//! the model overview (name, experiment, connections, problems); with a block, its inspector
//! (name, parameters, flows, its problems); with a flow, how entities take it; with several
//! blocks, the properties they share. A value that differs from its default, or from the
//! scenario the opened one inherits from, is in bold with a reset beside it (see `props`).
//! Every change is one engine command (a batch for several blocks), so each is one undo step.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use stochcraft_engine::COMMANDS;
use stochcraft_model::Model;

use crate::form::{self, Mark, Marked, Schema};
use crate::i18n::{t, tc, tf};
use crate::props::{self, Against};
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
    ("conveyor", "Conveyor"),
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
        // Ten long, one length unit a time unit, items one long with no gap.
        "conveyor" => json!({
            "kind": "conveyor",
            "length": 10.0,
            "speed": 1.0,
            "accumulating": true,
            "item_length": 1.0,
            "gap": 0.0
        }),
        _ => json!({"kind": "sink"}),
    }
}

/// A name for a new block of `kind`: the kind's short label, as the results name it, with a
/// number ("Arrivals 1"), the lowest
/// number no block of that kind already has. `pending` holds the names of blocks about to be
/// added in the same step. Blocks show words, not the engine's ids ("source-1").
pub(crate) fn default_block_name(
    model: &Model,
    pending: &[(String, String)],
    kind: &str,
) -> String {
    // The results' label is translated with each language's own capitals (no rule for the
    // first letter, which would be wrong in Turkish or German).
    let base = crate::results::kind_label(kind);
    let taken = |name: &str| {
        model
            .elements
            .iter()
            .any(|e| e.kind.name() == kind && e.name == name)
            || pending.iter().any(|(k, n)| k == kind && n == name)
    };
    let mut n: u32 = 1;
    loop {
        let name = format!("{base} {n}");
        if !taken(&name) {
            return name;
        }
        n += 1;
    }
}

/// The basic parameters of each kind of block, first in the inspector in this order: its
/// capacity and main time.
fn parameters(kind: &str) -> &'static [&'static str] {
    match kind {
        "source" => &["interarrival"],
        "server" => &["capacity", "shifts", "service", "queue_capacity"],
        // Its needs are edited by `needs_section` in pools.rs; service and priority here.
        "shared_task" => &["service", "priority"],
        "delay" => &["duration"],
        "batch" => &["size"],
        "conveyor" => &["length", "speed", "accumulating", "item_length", "gap"],
        _ => &[],
    }
}

/// The rest of a kind's parameters, folded under More settings, in this order.
fn more_parameters(kind: &str) -> &'static [&'static str] {
    match kind {
        "source" => &["limit"],
        "server" => &[
            "transfer",
            "transfer_holds_sender",
            "setup",
            "processing_by",
            "planned_run_time",
            "starting",
        ],
        "shared_task" | "delay" => &["processing_by", "starting"],
        "batch" => &["starting"],
        _ => &[],
    }
}

/// What the "?" beside a basic parameter's label explains.
fn explanation(f: &str) -> Option<&'static str> {
    match f {
        "capacity" => Some(t(
            "How many units serve here at the same time, such as machines or staff, each one entity at a time.",
        )),
        "service" => Some(t(
            "How long one unit takes to serve one entity, drawn anew for each entity.",
        )),
        "queue_capacity" => Some(t(
            "How many entities may wait here for a free unit, besides those being served; unticked, any number may.",
        )),
        "starting" => Some(t(
            "Entities already inside when each run starts, instead of an empty start: as many as there are free units start being served at once, the rest wait.",
        )),
        _ => None,
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

/// A block's parameter `f` as its field shows it: a setting the file leaves out when it has
/// its default, with that default.
fn shown_value(f: &str, el: &Value) -> Value {
    match (f, &el[f]) {
        ("available" | "transfer_holds_sender", Value::Null) => json!(true),
        ("idle_outside", Value::Null) => json!(false),
        (_, v) => v.clone(),
    }
}

/// Whether a block's parameter is a time, or a conveyor's length or speed: a number with a unit
/// after it.
fn is_timed(f: &str) -> bool {
    matches!(
        f,
        "interarrival"
            | "service"
            | "duration"
            | "transfer"
            | "planned_run_time"
            | "length"
            | "speed"
            | "item_length"
            | "gap"
    )
}

/// A box for a setting that is on or off (a flow that is rework, entities giving up), its label
/// in bold with a reset after it while the setting differs from its reference. Whether it was
/// toggled, and whether the reset was clicked.
fn marked_checkbox(
    ui: &mut egui::Ui,
    on: &mut bool,
    label: &str,
    hover: &str,
    mark: Mark,
) -> (bool, bool) {
    ui.horizontal(|ui| {
        let r = ui
            .checkbox(on, form::marked_text(label, Some(mark)))
            .on_hover_text(hover);
        form::describe_mark(ui, &r, Some(mark), Some(hover));
        let reset = form::reset_button(ui, Some(mark), label);
        (r.changed(), reset)
    })
    .inner
}

/// The line above a choice of settings ("Job under way on a server going off shift:"), in bold
/// with a reset after it while the choice differs from its reference; the reset is named after
/// the line, without its colon. Whether the reset was clicked.
fn marked_heading(ui: &mut egui::Ui, text: &str, mark: Mark) -> bool {
    ui.horizontal(|ui| {
        let r = ui.label(form::marked_text(text, Some(mark)));
        form::describe_mark(ui, &r, Some(mark), None);
        let name = text.trim_end().trim_end_matches([':', '：']).trim_end();
        form::reset_button(ui, Some(mark), name)
    })
    .inner
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
    /// The engine's stable code for it (`zero_capacity`, `bad_distribution`…).
    pub code: String,
    /// The diagnostic as the engine sent it: what the message is built from in the interface
    /// language ([`crate::i18n::engine_message`]).
    diagnostic: Value,
}

impl Problem {
    /// The message in the interface language, without the word "Error" or "Warning", which is a
    /// label of its own. It names the block and says what to do; the place is the block shown.
    pub fn text(&self) -> String {
        crate::i18n::engine_message(&self.diagnostic)
    }

    /// The field it concerns, as the inspector keys it under its block (`capacity`,
    /// `service`, `breakdown/repair`) or the model page keys it (`x/replications`), when known.
    pub fn field(&self) -> Option<String> {
        if let Some(f) = self.diagnostic["field"].as_str() {
            return Some(f.to_owned());
        }
        let f = match self.code.as_str() {
            "zero_capacity" => "capacity",
            "zero_batch_size" => "size",
            "no_time_between_arrivals" => "interarrival",
            "planned_run_time_not_positive" => "planned_run_time",
            "patience_negative" => "patience",
            "health_check_interval_not_positive" => "health_check/check_interval",
            "health_check_limit_not_valid" => "health_check/check_limit",
            _ => return None,
        };
        Some(f.to_owned())
    }
}

impl App {
    /// The model's problems (validation is a query: nothing is journalled). Validated once per
    /// change of the model, however many views ask in a frame.
    pub(crate) fn problems(&mut self) -> Vec<Problem> {
        if let Some((revision, problems)) = &self.problems_of
            && *revision == self.engine.revision()
        {
            return problems.clone();
        }
        let problems = self.validate();
        self.problems_of = Some((self.engine.revision(), problems.clone()));
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
                code: d["code"].as_str().unwrap_or("").to_owned(),
                diagnostic: d.clone(),
            })
            .collect()
    }

    /// The block a problem at `at` is shown on: the block itself, or the one a flow leaves (a
    /// flow's problems are listed in its blocks' inspectors); `None` for the model page.
    pub(crate) fn block_of(&self, at: Option<&str>) -> Option<String> {
        let at = at?;
        let m = self.engine.model();
        if m.elements.iter().any(|e| e.id.as_str() == at) {
            return Some(at.to_owned());
        }
        m.connections
            .iter()
            .find(|c| c.id.as_str() == at)
            .map(|c| c.from.as_str().to_owned())
            .filter(|from| m.elements.iter().any(|e| e.id.as_str() == from))
    }

    /// Shows the model's next error: the first one past those of the selected block, or the
    /// first. Its block is selected (or the model page, for the experiment's), its field focused
    /// when known, and the status bar says what is wrong. False when there is none.
    pub(crate) fn go_to_problem(&mut self) -> bool {
        let errors: Vec<Problem> = self.problems().into_iter().filter(|p| p.error).collect();
        let blocks: Vec<Option<String>> = errors
            .iter()
            .map(|p| self.block_of(p.at.as_deref()))
            .collect();
        // Past the error shown last, while its page is still the one shown; otherwise past the
        // selected block's last; otherwise the first.
        let here = self
            .problem_shown
            .filter(|&i| i < errors.len() && blocks[i] == self.selected)
            .or_else(|| {
                self.selected.as_ref()?;
                blocks.iter().rposition(|b| *b == self.selected)
            });
        let i = here.map_or(0, |i| (i + 1) % errors.len().max(1));
        self.problem_shown = Some(i);
        let Some(pr) = errors.get(i) else {
            return false;
        };
        let block = blocks[i].clone();
        self.show_block(block.clone());
        form::focus(pr.field().map(|f| match &block {
            Some(b) if !f.starts_with("x/") => format!("{b}/{f}"),
            _ => f,
        }));
        self.report_error(pr.text());
        self.status_at = block;
        true
    }

    /// After a reset of the field under `key`: what was typed in it is dropped (a refused number
    /// would otherwise stay in it), and on the next frame the keyboard goes to the field's first
    /// number, since the reset it was on is gone with the change.
    fn after_reset(&mut self, key: &str) {
        let under = format!("{key}/");
        let refused = format!("{key}!");
        self.drafts
            .retain(|k, _| k != key && k != &refused && !k.starts_with(&under));
        self.refocus = Some(key.to_owned());
    }

    /// Which page of the properties panel the selection shows: a block's, several blocks', a
    /// flow's or the model's.
    fn page(&self) -> String {
        match (&self.selected, &self.canvas.flow) {
            (Some(id), _) => format!("block {id}"),
            (None, _) if self.canvas.multi.len() > 1 => {
                let ids: Vec<&str> = self.canvas.multi.iter().map(String::as_str).collect();
                format!("blocks {}", ids.join(" "))
            }
            (None, Some(flow)) => format!("flow {flow}"),
            (None, None) => "model".to_owned(),
        }
    }

    /// Selects `block` (`None`: the model page) in the model view.
    pub(crate) fn show_block(&mut self, block: Option<String>) {
        self.show_results = false;
        self.show_analysis = false;
        self.show_capacity = false;
        self.show_3d = false;
        self.canvas.flow = None;
        self.canvas.multi.clear();
        self.selected = block;
    }

    /// Adds a block of `kind` after the selection, joins the selection to it when the selected
    /// block sends entities on, and selects it.
    pub(crate) fn add_block(&mut self, kind: &str) {
        let mut params = new_block(kind);
        params["name"] = json!(default_block_name(self.engine.model(), &[], kind));
        let index = self.selected_index().map(|i| i + 1);
        if let Some(i) = index {
            params["index"] = json!(i);
        }
        let added = if kind == "shared_task" {
            self.add_shared_task(params)
        } else {
            self.execute("block.add", params)
        };
        if let Some(r) = added
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
        let model = self.wire_model();
        let ctx = ui.ctx().clone();
        if let Some(key) = self.refocus.take() {
            form::focus(Some(key));
        }
        self.inherited = self
            .engine
            .inherited_model()
            .map(stochcraft_engine::wire::model);
        // Text typed in a field and not yet entered belongs to the block it was typed for. When
        // another is selected, that block's page is drawn once more with the focus gone from
        // the field, so the field commits as Enter would; the new selection shows next frame.
        // Esc drops what was typed instead, and so does a change to the model meanwhile (another
        // file opened, Undo), after which the block may not be there or not be the same.
        if self.drafts_of == self.selected {
            self.drafts_held = false;
        } else {
            let typing = self.inspector_focus.take();
            if let Some((field, revision)) = typing
                && revision == self.engine.revision()
                && !self.drafts_held
                && !ctx.input(|i| i.key_pressed(egui::Key::Escape))
            {
                if ctx.memory(|m| m.focused()) == Some(field) {
                    ctx.memory_mut(|m| m.surrender_focus(field));
                }
                self.drafts_held = true;
                ctx.request_repaint();
            } else {
                // A refused number belongs to the block it was typed for: showing another drops
                // it (Esc in the field drops it too, and also goes back to the model).
                self.drafts.clear();
                self.drafts_of = self.selected.clone();
                self.drafts_held = false;
            }
        }
        // The model page, a flow's and several blocks' all have no block selected: text typed
        // on one of them is dropped when another is shown.
        if !self.drafts_held {
            let page = self.page();
            if self.drafts_page != page {
                self.drafts.clear();
                self.drafts_page = page;
            }
        }
        let selected = self.selected.clone();
        if self.drafts_held {
            self.selected = self.drafts_of.clone();
        }
        let shown = self.selected.clone();
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
                    None if self.canvas.multi.len() > 1 && !self.drafts_held => {
                        self.multi_inspector(ui)
                    }
                    None => match self.canvas.flow.clone() {
                        Some(flow) if !self.drafts_held => {
                            self.flow_inspector(ui, &model, &flow, &problems);
                        }
                        _ => self.overview(ui, p, &model, &problems),
                    },
                }
            });
        if self.drafts_held {
            // The selection made meanwhile, unless drawing the held page changed it.
            if self.selected == shown {
                self.selected = selected;
            }
        } else {
            // A field of this page that has the keyboard, so a new selection commits it first,
            // with the model it was typed for.
            let area = ui.max_rect();
            let revision = self.engine.revision();
            self.inspector_focus = ctx
                .memory(|m| m.focused())
                .filter(|id| {
                    ctx.read_response(*id)
                        .is_some_and(|r| area.intersects(r.rect))
                })
                .map(|id| (id, revision));
        }
        // A field to focus on the page just drawn was taken, or is not there: forgotten. (One on
        // a page chosen meanwhile, by a Show link, waits for that page.)
        let page = match &shown {
            Some(id) => format!("{id}/"),
            None => "x/".to_owned(),
        };
        if form::focus_key().is_some_and(|k| k.starts_with(&page) || k.starts_with("multi/")) {
            form::focus(None);
        }
    }

    /// What is set of a block's parameters `fields`, in a line: each with its main number
    /// ("Transfer time Constant 2 min, Setup Exponential 1 min"), or None.
    fn summary(&self, el: &Value, fields: &[&str]) -> String {
        let unit = self.time_unit_label();
        let dist = |d: &Value| -> String {
            let kind = form::choice_label(d["type"].as_str().unwrap_or(""));
            match ["mean", "value", "mode"]
                .iter()
                .find_map(|k| d[*k].as_f64())
            {
                Some(x) => format!("{kind} {} {unit}", self.numbers.format(x, 0..=3)),
                None => kind,
            }
        };
        let parts: Vec<String> = fields
            .iter()
            .filter(|f| is_set(el, f) && **f != "transfer_holds_sender")
            .map(|f| {
                let v = &el[*f];
                let label = form::label(f);
                match *f {
                    "breakdown" => format!(
                        "{label}: {}, {}",
                        dist(&v["time_to_failure"]),
                        dist(&v["repair"])
                    ),
                    "patience" => dist(&v["time"]),
                    "health_check" => {
                        format!("{} {}", form::label("check_interval"), dist(&v["interval"]))
                    }
                    "setup" => format!("{label} {}", dist(&v["time"])),
                    // Set only when false: no units outside the calendars.
                    "available" => format!("{label}: {}", t("None")),
                    _ if v["type"].is_string() => format!("{label} {}", dist(v)),
                    _ => match v.as_f64() {
                        Some(x) => format!("{label} {}", self.numbers.format(x, 0..=3)),
                        None => label,
                    },
                }
            })
            .collect();
        if parts.is_empty() {
            t("None").to_owned()
        } else {
            parts.join(", ")
        }
    }

    pub(crate) fn time_unit_label(&self) -> String {
        crate::time_unit(&self.engine)
    }

    /// The model's length unit as written after a number ("m", "ft").
    pub(crate) fn length_unit_label(&self) -> String {
        crate::length_unit(&self.engine)
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
        if let Some(n) = self.text_field(ui, "model/name", t("Name"), &name, true) {
            self.execute("model.rename", json!({ "name": n }));
        }

        section(ui, t("Experiment"));
        let mut schema = params_schema("experiment.set");
        // The limits the model checks the experiment against, so its fields refuse the rest.
        schema["properties"]["replications"]["minimum"] = json!(1);
        schema["properties"]["warmup"]["minimum"] = json!(0.0);
        schema["properties"]["duration"]["exclusiveMinimum"] = json!(0.0);
        let unit = self.time_unit_label();
        let x = model["experiment"].clone();
        for (f, time) in [
            ("replications", false),
            ("seed", false),
            ("warmup", true),
            ("duration", true),
            ("time_unit", false),
            ("length_unit", false),
        ] {
            let mut cx = form::Cx {
                time_unit: &unit,
                numbers: &self.numbers,
                drafts: &mut self.drafts,
                calendars: &[],
            };
            // The file leaves the length unit out when it is metres.
            let current = if f == "length_unit" && x[f].is_null() {
                json!("m")
            } else {
                x[f].clone()
            };
            if let Some(v) = form::field(
                ui,
                &mut cx,
                Schema::new(&schema),
                &format!("x/{f}"),
                f,
                &current,
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
        self.sequential_row(ui, model);
        self.confidence_row(ui, model);
        self.observed_section(ui, model);

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

    pub(crate) fn problem_row(
        &mut self,
        ui: &mut egui::Ui,
        p: &theme::Palette,
        pr: &Problem,
        link: bool,
    ) {
        ui.horizontal_wrapped(|ui| {
            let (word, color) = if pr.error {
                (t("Error"), p.error)
            } else {
                (t("Warning"), p.warning)
            };
            ui.label(theme::strong(word).color(color));
            // The word is the label and the place is the inspector or the Show link, so only
            // the rest of the engine's message is shown.
            ui.label(pr.text());
            if link
                && let Some(at) = &pr.at
                && ui.link(tf("Show {id}", &[("id", at)])).clicked()
            {
                // A flow's problem is shown on the block it leaves.
                let block = self.block_of(Some(at));
                self.show_block(block);
                form::focus(
                    pr.field()
                        .and_then(|f| Some(format!("{}/{f}", self.selected.as_ref()?))),
                );
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

    /// `block.set`'s schema with the limits the model checks, so the fields refuse the rest: a
    /// server has a unit (or, with units by calendar, `by_calendar`, may have none outside
    /// them), a batch at least one entity; a conveyor's lengths and speed are above 0 and its
    /// gap 0 or more.
    fn block_schema(by_calendar: bool) -> Value {
        let mut schema = params_schema("block.set");
        if !by_calendar {
            schema["properties"]["capacity"]["minimum"] = json!(1);
        }
        schema["properties"]["size"]["minimum"] = json!(1);
        // The accumulating box has the words below; the others are plain from their labels
        // and units.
        for f in ["length", "speed", "item_length"] {
            schema["properties"][f]["exclusiveMinimum"] = json!(0.0);
            schema["properties"][f]["description"] = Value::Null;
        }
        schema["properties"]["gap"]["minimum"] = json!(0.0);
        schema["properties"]["gap"]["description"] = Value::Null;
        schema["properties"]["accumulating"]["description"] = json!(t(
            "Ticked, entities queue up at the end while the rest keep moving. Unticked, the whole belt stops when the entity at the end cannot leave."
        ));
        schema
    }

    /// The unit block field `f`'s numbers are shown in: a conveyor's lengths and speed in their
    /// own, the rest in the time unit (the form's "time" fields are those with a unit after
    /// them).
    fn field_unit(&self, f: &str) -> String {
        let unit = self.time_unit_label();
        match f {
            "length" | "item_length" | "gap" => self.length_unit_label(),
            "speed" => format!("{}/{unit}", self.length_unit_label()),
            _ => unit,
        }
    }

    /// Block `id` in the scenario the opened one inherits from, if it inherits from one and the
    /// block is there.
    fn inherited_block(&self, id: &str) -> Option<&Value> {
        self.inherited.as_ref()?["elements"]
            .as_array()?
            .iter()
            .find(|e| e["id"] == id)
    }

    /// Flow `id` in the scenario the opened one inherits from, if it is there.
    fn inherited_flow(&self, id: &str) -> Option<&Value> {
        self.inherited.as_ref()?["connections"]
            .as_array()?
            .iter()
            .find(|c| c["id"] == id)
    }

    /// How block `el`'s field `f` compares with what it is measured against (the inherited
    /// scenario's value, or a new block's), and that reference value.
    fn block_mark(&self, el: &Value, f: &str) -> (Mark, Value) {
        let id = el["id"].as_str().unwrap_or("");
        let kind = el["kind"].as_str().unwrap_or("");
        let (reference, against) = props::block_reference(kind, f, self.inherited_block(id));
        let changed = props::differs(f, &el[f], &reference);
        (Mark { changed, against }, reference)
    }

    /// How flow `c`'s field `f` compares with what it is measured against, and that value.
    fn flow_mark(&self, c: &Value, f: &str) -> (Mark, Value) {
        let id = c["id"].as_str().unwrap_or("");
        let (reference, against) = props::flow_reference(f, self.inherited_flow(id));
        let changed = props::differs(f, &c[f], &reference);
        (Mark { changed, against }, reference)
    }

    /// A block's parameters `fields`, each drawn from `block.set`'s schema, in bold with a
    /// reset while it differs from its default (or inherited) value; an edit or a reset is one
    /// `block.set`.
    fn block_fields(
        &mut self,
        ui: &mut egui::Ui,
        model: &Value,
        el: &Value,
        id: &str,
        fields: &[&str],
    ) {
        let schema = Self::block_schema(is_set(el, "shifts"));
        let calendars = form::calendars(model);
        for f in fields {
            // Whether the sender waits applies only to a transfer.
            if *f == "transfer_holds_sender" && el["transfer"].is_null() {
                continue;
            }
            let current = shown_value(f, el);
            let shown_unit = self.field_unit(f);
            let (mark, reference) = self.block_mark(el, f);
            let mut cx = form::Cx {
                time_unit: &shown_unit,
                numbers: &self.numbers,
                drafts: &mut self.drafts,
                calendars: &calendars,
            };
            let key = format!("{id}/{f}");
            // The basics' "?", on the task's own fields (not a time by value, say).
            // The words for starting contents speak of units serving, so only on a task with
            // units.
            let shown = match *f {
                "starting" => el["kind"] == "server" || el["kind"] == "shared_task",
                _ => el["kind"] != "delay",
            };
            form::explain(&key, explanation(f).filter(|_| shown));
            let edit = form::field_marked(
                ui,
                &mut cx,
                Schema::new(&schema),
                &key,
                f,
                &current,
                is_timed(f),
                mark,
            );
            form::explain(&key, None);
            match edit {
                Marked::Set(v) => {
                    self.execute("block.set", json!({ "id": id, *f: v }));
                }
                Marked::Reset => {
                    self.after_reset(&key);
                    let v = props::reset_value(&el[*f], &reference);
                    self.execute("block.set", json!({ "id": id, *f: v }));
                }
                Marked::Unchanged => {}
            }
        }
    }

    /// The properties the selected blocks share, each in bold with a reset while any of them
    /// differs from its default (or inherited) value: the fields `block.set` takes for every
    /// selected kind. Where their values differ the field shows that they do, and an edit sets
    /// only what was edited, so typing a mean keeps each distribution's other parameters. An
    /// edit or a reset is one batch, so one undo step for them all.
    pub(crate) fn multi_properties(&mut self, ui: &mut egui::Ui) {
        let model = self.wire_model();
        let els: Vec<Value> = model["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| {
                e["id"]
                    .as_str()
                    .is_some_and(|id| self.canvas.multi.contains(id))
            })
            .cloned()
            .collect();
        let Some(first) = els.first() else {
            return;
        };
        let calendars = form::calendars(&model);
        let all_of = |kind: &str| -> Vec<&'static str> {
            parameters(kind)
                .iter()
                .chain(more_parameters(kind))
                .chain(timed_parameters(kind))
                .copied()
                .collect()
        };
        let by_calendar = !calendars.is_empty() || els.iter().any(|e| is_set(e, "shifts"));
        let fields: Vec<&str> = all_of(first["kind"].as_str().unwrap_or(""))
            .into_iter()
            .filter(|f| {
                els.iter()
                    .all(|e| all_of(e["kind"].as_str().unwrap_or("")).contains(f))
            })
            .filter(|f| *f != "shifts" || by_calendar)
            .filter(|f| {
                *f != "transfer_holds_sender" || els.iter().all(|e| !e["transfer"].is_null())
            })
            .collect();
        section(ui, t("Shared properties"));
        if fields.is_empty() {
            ui.label(theme::weak(ui, t("These blocks share no properties.")));
            return;
        }
        let schema = Self::block_schema(els.iter().any(|e| is_set(e, "shifts")));
        for f in fields {
            let currents: Vec<Value> = els.iter().map(|e| shown_value(f, e)).collect();
            let marks: Vec<(Mark, Value)> = els.iter().map(|e| self.block_mark(e, f)).collect();
            let mark = Mark {
                changed: marks.iter().any(|(m, _)| m.changed),
                against: if marks.iter().any(|(m, _)| m.against == Against::Inherited) {
                    Against::Inherited
                } else {
                    Against::Default
                },
            };
            let key = format!("multi/{f}");
            let mut mixed = BTreeSet::new();
            let shown = props::merge(&currents, &key, &mut mixed);
            let shown_unit = self.field_unit(f);
            let mut cx = form::Cx {
                time_unit: &shown_unit,
                numbers: &self.numbers,
                drafts: &mut self.drafts,
                calendars: &calendars,
            };
            // Mixed as a whole (some with a transfer time, some without): the field showed none
            // of their values, so what is chosen there is set on each as it is.
            let whole = mixed.contains(&key);
            form::set_mixed(mixed);
            let edit = form::field_marked(
                ui,
                &mut cx,
                Schema::new(&schema),
                &key,
                f,
                &shown,
                is_timed(f),
                mark,
            );
            form::set_mixed(BTreeSet::new());
            let reset = edit == Marked::Reset;
            let steps: Vec<Value> = match edit {
                Marked::Set(v) => els
                    .iter()
                    .zip(&currents)
                    .filter_map(|(e, now)| {
                        let next = if whole {
                            v.clone()
                        } else {
                            props::patch(now, &shown, &v)
                        };
                        (!props::same(&next, now)).then(|| {
                            json!({"command": "block.set", "params": { "id": e["id"], f: next }})
                        })
                    })
                    .collect(),
                Marked::Reset => els
                    .iter()
                    .zip(&marks)
                    .filter(|(_, (m, _))| m.changed)
                    .map(|(e, (_, reference))| {
                        let v = props::reset_value(&e[f], reference);
                        json!({"command": "block.set", "params": { "id": e["id"], f: v }})
                    })
                    .collect(),
                Marked::Unchanged => Vec::new(),
            };
            if reset {
                self.after_reset(&key);
            }
            if !steps.is_empty() {
                self.execute("batch", json!({ "commands": steps }));
            }
        }
    }

    /// The properties of the flow selected on the canvas: the blocks it joins, and how entities
    /// take it (a decision's probability and condition, and whether it is rework or the way for
    /// those who give up, find no room or balk), each in bold with a reset while it differs
    /// from a new flow's (or the inherited scenario's) value. Each edit is one `flow.set`.
    fn flow_inspector(
        &mut self,
        ui: &mut egui::Ui,
        model: &Value,
        cid: &str,
        problems: &[Problem],
    ) {
        let c = connection(model, cid);
        if c.is_null() {
            self.canvas.flow = None;
            return;
        }
        let from = c["from"].as_str().unwrap_or("").to_owned();
        let to = c["to"].as_str().unwrap_or("").to_owned();
        let names = element_names(model);
        let name_of = |x: &str| {
            names
                .iter()
                .find(|(i, _)| i == x)
                .map_or_else(|| x.to_owned(), |(_, n)| n.clone())
        };
        ui.horizontal(|ui| {
            if ui
                .link(t("Model"))
                .on_hover_text(t("Back to the model (Esc)"))
                .clicked()
            {
                self.canvas.flow = None;
            }
            ui.label(theme::weak(ui, "›"));
            ui.label(theme::weak(ui, tc("connection", "Flow")));
        });
        ui.add_space(4.0);
        // From and to, each a link to its block.
        let mut go = None;
        let mut remove = false;
        ui.horizontal(|ui| {
            if ui
                .link(egui::RichText::new(name_of(&from)).heading())
                .clicked()
            {
                go = Some(from.clone());
            }
            ui.label(theme::weak(ui, "→"));
            if ui
                .link(egui::RichText::new(name_of(&to)).heading())
                .clicked()
            {
                go = Some(to.clone());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                remove = ui
                    .add(theme::secondary(ui, t("Remove flow")))
                    .on_hover_text(tf("Remove the flow {id}", &[("id", cid)]))
                    .clicked();
            });
        });
        ui.label(theme::weak(ui, tf("id {id}", &[("id", cid)])).monospace());
        ui.add_space(8.0);
        let p = theme::palette(ui.ctx());
        for pr in problems.iter().filter(|pr| pr.at.as_deref() == Some(cid)) {
            self.problem_row(ui, &p, pr, false);
        }
        let from_kind = kind_of(model, &from);
        let from_el = model["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|e| e["id"] == from.as_str())
            .cloned()
            .unwrap_or_default();
        if from_kind == "exclusive_gateway" {
            self.flow_probability_row(ui, &c, cid);
            self.flow_condition_row(ui, &c, cid);
        }
        let server = from_kind == "server";
        let boxes = [
            (
                "rework",
                from_kind != "source",
                t("Rework"),
                t(
                    "Parts taking this flow go back to be worked on again; they count as rework against the last task with a resource they finished at.",
                ),
            ),
            (
                "abandoned",
                server,
                t("Gave up"),
                t(
                    "Entities that give up waiting take this flow (to try again later, say); the others take the task's other way out.",
                ),
            ),
            (
                "turned_away",
                server && from_el["queue_capacity"].is_number(),
                t("Turned away"),
                t(
                    "Entities finding the waiting room full take this flow at once (to try again later, say), instead of being lost or waiting before the task; they count as lost here.",
                ),
            ),
            (
                "balked",
                server && from_el["balking"].is_object(),
                t("Balked"),
                t(
                    "Entities that do not join the waiting room take this flow at once (to try again later, say), instead of being lost; they count as balked here.",
                ),
            ),
        ];
        for (f, applies, label, hover) in boxes {
            // A mark that no longer applies still shows, so it can be taken off.
            if !applies && c[f].as_bool() != Some(true) {
                continue;
            }
            let (mark, reference) = self.flow_mark(&c, f);
            let mut on = c[f].as_bool().unwrap_or(false);
            let (toggled, reset) = marked_checkbox(ui, &mut on, label, hover, mark);
            if toggled {
                self.execute("flow.set", json!({ "id": cid, f: on }));
            } else if reset {
                self.execute("flow.set", json!({ "id": cid, f: reference }));
            }
        }
        if remove {
            self.execute("disconnect", json!({ "id": cid }));
            self.canvas.flow = None;
        } else if let Some(b) = go {
            self.select(BTreeSet::from([b]));
        }
    }

    /// A decision's way out: its probability, on a row of its own in the flow's page, blank when
    /// unset, in bold with a reset while set (or while it differs from the inherited one).
    fn flow_probability_row(&mut self, ui: &mut egui::Ui, c: &Value, cid: &str) {
        let (mark, reference) = self.flow_mark(c, "probability");
        let label = t("Probability");
        let key = format!("{cid}/p");
        let p_now = c["probability"]
            .as_f64()
            .map(crate::results::grouped)
            .unwrap_or_default();
        let mut reset = false;
        ui.vertical(|ui| {
            let typed = ui
                .horizontal(|ui| {
                    let l = form::label_cell(ui, form::marked_text(label, Some(mark)), 26.0);
                    form::describe_mark(ui, &l, Some(mark), None);
                    let typed = self.small_field(ui, &key, label, "", &p_now, 80.0);
                    reset = form::reset_button(ui, Some(mark), label);
                    typed
                })
                .inner;
            if let Some(typed) = typed {
                self.probability_typed(cid, &typed);
            }
            if let Some(why) = self.probability_refused(cid, &p_now) {
                theme::error_message(ui, why);
            }
        });
        if reset {
            self.drafts.remove(&key);
            self.execute("flow.set", json!({ "id": cid, "probability": reference }));
        }
    }

    /// A decision's way out: its condition, on a row of its own in the flow's page.
    fn flow_condition_row(&mut self, ui: &mut egui::Ui, c: &Value, cid: &str) {
        let (mark, reference) = self.flow_mark(c, "condition");
        let label = t("Condition");
        let key = format!("{cid}/c");
        let now = c["condition"].as_str().unwrap_or("").to_owned();
        let mut reset = false;
        let typed = ui
            .horizontal(|ui| {
                let l = form::label_cell(ui, form::marked_text(label, Some(mark)), 26.0);
                form::describe_mark(ui, &l, Some(mark), None);
                let typed = self.small_field(ui, &key, label, "", &now, 240.0);
                reset = form::reset_button(ui, Some(mark), label);
                typed
            })
            .inner;
        if reset {
            self.drafts.remove(&key);
            self.execute("flow.set", json!({ "id": cid, "condition": reference }));
        } else if let Some(typed) = typed {
            let typed = typed.trim();
            let v = if typed.is_empty() {
                Value::Null
            } else {
                json!(typed)
            };
            self.execute("flow.set", json!({ "id": cid, "condition": v }));
        }
    }

    /// A probability typed for flow `cid`: blank removes it, a number from 0 to 1 is one
    /// `flow.set`, and anything else stays in the field (under `{cid}/p`, marked refused) until
    /// it is fixed or Esc drops it.
    fn probability_typed(&mut self, cid: &str, typed: &str) {
        let key = format!("{cid}/p");
        let mark = format!("{key}!");
        let typed = typed.trim();
        self.drafts.remove(&mark);
        if typed.is_empty() {
            self.execute("flow.set", json!({ "id": cid, "probability": null }));
            return;
        }
        // As the user's region writes it ("0,5" in German), or with a point.
        let why = match self.numbers.parse(typed) {
            Ok(v) if (0.0..=1.0).contains(&v) => {
                self.execute("flow.set", json!({ "id": cid, "probability": v }));
                None
            }
            Ok(_) => Some(tf(
                "Must be from {min} to {max}",
                &[("min", "0"), ("max", "1")],
            )),
            Err(_) => Some(t("Not a number").to_owned()),
        };
        if let Some(why) = why {
            self.drafts.insert(key, typed.to_owned());
            self.drafts.insert(mark, why);
        }
    }

    /// Why the probability typed for flow `cid` was refused, while it stays in its field, with
    /// the value the flow keeps (`p_now`, blank when unset).
    fn probability_refused(&mut self, cid: &str, p_now: &str) -> Option<String> {
        let key = format!("{cid}/p");
        let mark = format!("{key}!");
        // Esc, or a fix, took the text away.
        if !self.drafts.contains_key(&key) {
            self.drafts.remove(&mark);
        }
        self.drafts.get(&mark).map(|why| {
            if p_now.is_empty() {
                why.clone()
            } else {
                tf(
                    "{reason}. {field} is still {value}.",
                    &[
                        ("reason", why),
                        ("field", t("Probability")),
                        ("value", p_now),
                    ],
                )
            }
        })
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
        self.decision_link(ui, &id);

        // The block's own problems, and those of the flows into and out of it, in a place of
        // fixed height (two lines, scrolling beyond), so the form under it doesn't move when one
        // appears or goes.
        let flows = flows_of(model, &id);
        let mine: Vec<&Problem> = problems
            .iter()
            .filter(|pr| {
                pr.at
                    .as_deref()
                    .is_some_and(|at| at == id || flows.iter().any(|f| f == at))
            })
            .collect();
        egui::ScrollArea::vertical()
            .id_salt(("block problems", id.as_str()))
            .max_height(PROBLEMS_HEIGHT)
            .min_scrolled_height(PROBLEMS_HEIGHT)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for pr in mine {
                    self.problem_row(ui, p, pr, false);
                }
            });

        if let Some(n) = self.text_field(ui, &format!("{id}/name"), t("Name"), &name, true) {
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
        // A conveyor's path in the layout, point by point (L4).
        if kind == "conveyor" {
            self.path_section(ui, &id);
        }
        // The rest, folded with a summary of what is set; open when anything is.
        let more = more_parameters(&kind);
        if !more.is_empty() {
            ui.add_space(6.0);
            let summary = self.summary(el, more);
            fold(
                ui,
                &format!("{id}/more"),
                t("More settings"),
                &summary,
                more.iter().any(|f| is_set(el, f)),
                more.iter().any(|f| form::focus_waits(&format!("{id}/{f}"))),
                |ui| self.block_fields(ui, model, el, &id, more),
            );
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
            let summary = self.summary(el, more);
            fold(
                ui,
                &format!("{id}/timed"),
                title,
                &summary,
                more.iter().any(|f| is_set(el, f)),
                // Open for a problem's field in it.
                more.iter().any(|f| form::focus_waits(&format!("{id}/{f}"))),
                |ui| self.block_fields(ui, model, el, &id, more),
            );
        }

        if kind == "shared_task" {
            self.needs_section(ui, model, el);
            self.by_resource_section(ui, model, el);
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
            let summary = self.summary(el, &["patience"]);
            fold(
                ui,
                &format!("{id}/patience"),
                t("Patience"),
                &summary,
                el["patience"].is_object(),
                form::focus_waits(&format!("{id}/patience")),
                |ui| self.patience_section(ui, model, el, &id),
            );
            // Balking, from block.set's schema: a table or 1/(n + 1), next to the flows those
            // that balk take; closed on a server without it.
            ui.add_space(6.0);
            let summary = self.summary(el, &["balking"]);
            fold(
                ui,
                &format!("{id}/balking"),
                t("Balking"),
                &summary,
                el["balking"].is_object(),
                form::focus_waits(&format!("{id}/balking")),
                |ui| self.block_fields(ui, model, el, &id, &["balking"]),
            );
            ui.add_space(6.0);
            let summary = self.summary(el, &["health_check"]);
            fold(
                ui,
                &format!("{id}/health_check"),
                t("Health check"),
                &summary,
                el["health_check"].is_object(),
                form::focus_waits(&format!("{id}/health_check")),
                |ui| self.health_check_section(ui, model, el, &id),
            );
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
                t(
                    "An entity takes the first flow whose condition holds; with no conditions, a \
                   flow drawn by the probabilities (blank ones share what the others leave).",
                ),
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
                                // One column whether or not a refusal shows under the fields, so
                                // the line appearing doesn't give the widgets after it new ids
                                // (taking the keyboard from a field below, and its node from a
                                // screen reader).
                                ui.vertical(|ui| {
                                    let refused = ui
                                        .horizontal(|ui| {
                                            ui.add_space(12.0);
                                            self.flow_fields(ui, cid, &c)
                                        })
                                        .inner;
                                    if let Some(why) = refused {
                                        ui.horizontal(|ui| {
                                            ui.add_space(12.0);
                                            theme::error_message(ui, why);
                                        });
                                    }
                                });
                            }
                            ui.horizontal(|ui| {
                                ui.add_space(12.0);
                                self.rework_field(ui, cid, &c);
                                // Only a task with a resource of its own has entities giving up.
                                if kind == "server" {
                                    self.abandoned_field(ui, cid, &c);
                                }
                                // Only one with a finite waiting room turns entities away;
                                // a way still marked shows, so the mark can be taken off.
                                if (kind == "server" && el["queue_capacity"].is_number())
                                    || c["turned_away"].as_bool() == Some(true)
                                {
                                    self.turned_away_field(ui, cid, &c);
                                }
                                // Only one with balking has entities not joining.
                                if (kind == "server" && el["balking"].is_object())
                                    || c["balked"].as_bool() == Some(true)
                                {
                                    self.balked_field(ui, cid, &c);
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
        let (mark, reference) = self.block_mark(el, "demand");
        let (toggled, reset) = marked_checkbox(
            ui,
            &mut on,
            t("Customer demand"),
            t(
                "How many parts customers want from these departures; the results then show takt (the time available over the demand) and each task's cycle time.",
            ),
            mark,
        );
        if reset {
            self.after_reset(&format!("{id}/demand"));
            self.execute("block.set", json!({ "id": id, "demand": reference }));
            return;
        }
        if toggled {
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
        if let Some(text) = self.text_field(
            ui,
            &format!("{id}/demand"),
            t("Parts wanted"),
            &q_now,
            false,
        ) {
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
        let (mark, reference) = self.block_mark(el, "scrap");
        let (toggled, reset) = marked_checkbox(
            ui,
            &mut on,
            t("Scrap"),
            t(
                "What reaches these departures is scrap, counted in the quality ratio of the task it counts against.",
            ),
            mark,
        );
        if reset {
            self.after_reset(&format!("{id}/scrap"));
            self.execute("block.set", json!({ "id": id, "scrap": reference }));
            return;
        }
        if toggled {
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
        let (mark, reference) = self.block_mark(el, "default");
        let mut reset = false;
        ui.horizontal(|ui| {
            let label = t("Default flow");
            let l = form::label_cell(ui, form::marked_text(label, Some(mark)), 26.0);
            form::describe_mark(ui, &l, Some(mark), None);
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
            reset = form::reset_button(ui, Some(mark), label);
        });
        if reset {
            self.execute("block.set", json!({ "id": id, "default": reference }));
        } else if let Some(d) = chosen.filter(|d| *d != now) {
            self.execute("block.set", json!({ "id": id, "default": d }));
        }
    }

    /// A decision's way out: its probability and condition, each blank when unset. Each commits
    /// on Enter or leaving the field, as one `flow.set`. A probability that isn't a number from
    /// 0 to 1 stays in its field, outlined, until it is fixed or Esc drops it; the reason is
    /// returned, for the line under the row.
    fn flow_fields(&mut self, ui: &mut egui::Ui, cid: &str, c: &Value) -> Option<String> {
        let p_now = c["probability"]
            .as_f64()
            .map(crate::results::grouped)
            .unwrap_or_default();
        let key = format!("{cid}/p");
        if let Some(typed) = self.small_field(
            ui,
            &key,
            &tf("Probability of {id}", &[("id", cid)]),
            "p",
            &p_now,
            56.0,
        ) {
            self.probability_typed(cid, &typed);
        }
        let refused = self.probability_refused(cid, &p_now);
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
        refused
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
        let r = ui.checkbox(&mut abandoned, t("Gave up")).on_hover_text(t(
            "Entities that give up waiting take this flow (to try again later, say); the others take the task's other way out.",
        ));
        let (ticked, name) = (abandoned, tf("Gave up on {id}", &[("id", cid)]));
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

    /// Whether this way out is the one entities not joining the task's queue take, besides the
    /// task's ordinary way out.
    fn balked_field(&mut self, ui: &mut egui::Ui, cid: &str, c: &Value) {
        let mut balked = c["balked"].as_bool().unwrap_or(false);
        let r = ui.checkbox(&mut balked, t("Balked")).on_hover_text(t(
            "Entities that do not join the waiting room take this flow at once (to try again later, say), instead of being lost; they count as balked here.",
        ));
        let (ticked, name) = (balked, tf("Balked on {id}", &[("id", cid)]));
        r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, ticked, &name));
        if r.changed() {
            self.execute("flow.set", json!({ "id": cid, "balked": balked }));
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
        // Measured against the choice of the inherited scenario's block, or a new block's
        // (fixed); the calendars' numbers have their own mark below.
        let (per_mark, per_ref) = self.block_mark(el, "queue_per_working_server");
        let (_, shifts_ref) = self.block_mark(el, "queue_shifts");
        let reference = if per_ref == json!(true) {
            "per_working_server"
        } else if shifts_ref.is_array() {
            "by_calendar"
        } else {
            "fixed"
        };
        let mark = Mark {
            changed: reference != current,
            against: per_mark.against,
        };
        if marked_heading(ui, t("Size of the waiting room:"), mark) {
            let v = match reference {
                "per_working_server" => json!({ "id": id, "queue_per_working_server": true }),
                "by_calendar" => json!({ "id": id, "queue_shifts": shifts_ref }),
                _ => json!({ "id": id, "queue_per_working_server": false, "queue_shifts": [] }),
            };
            self.execute("block.set", v);
            return;
        }
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
        let (mark, reference) = self.block_mark(el, "patience");
        let (toggled, reset) = marked_checkbox(
            ui,
            &mut on,
            t("Entities give up waiting"),
            t(
                "Each entity waits here at most a time drawn for it as it arrives, then gives up: it takes the flow ticked “Gave up”, or leaves the model.",
            ),
            mark,
        );
        if reset {
            self.after_reset(&format!("{id}/patience"));
            self.execute("block.set", json!({ "id": id, "patience": reference }));
            return;
        }
        if toggled {
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
        let (mark, reference) = self.block_mark(el, "on_shift_end");
        if marked_heading(ui, t("Job under way on a server going off shift:"), mark) {
            self.execute("block.set", json!({ "id": id, "on_shift_end": reference }));
            return;
        }
        ui.radio_value(&mut chosen, "finish".to_owned(), t("Finish, then go off"))
            .on_hover_text(t(
                "The server finishes the job it has, then goes off; until then it counts as on shift.",
            ));
        ui.radio_value(&mut chosen, "retry".to_owned(), t("Retry elsewhere"))
            .on_hover_text(t(
                "The job stops at once: the entity goes back to the front of the waiting room for any server on shift, and the work done on it is wasted.",
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
        let (mark, reference) = self.block_mark(el, "health_check");
        let (toggled, reset) = marked_checkbox(
            ui,
            &mut on,
            t("Take servers out when overloaded"),
            t(
                "Each server (unit) is checked every so often, as a load balancer probes the servers behind it. One that fails, with too many waiting, is taken out of service for a restart, then comes back.",
            ),
            mark,
        );
        if reset {
            self.after_reset(&format!("{id}/health_check"));
            self.execute("block.set", json!({ "id": id, "health_check": reference }));
            return;
        }
        if toggled {
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
                "The connection drops and the client tries again: the entity goes back to the front of the waiting room at once, and the work done on it is wasted.",
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
    pub(crate) fn small_field(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        name: &str,
        prefix: &str,
        current: &str,
        width: f32,
    ) -> Option<String> {
        if !prefix.is_empty() {
            ui.label(theme::weak(ui, prefix));
        }
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
        // Text its caller refused (marked `key!`), outlined as an error until it is fixed.
        if self.drafts.contains_key(&format!("{key}!")) {
            ui.painter().rect_stroke(
                r.rect,
                4.0,
                egui::Stroke::new(1.5, theme::palette(ui.ctx()).error),
                egui::StrokeKind::Inside,
            );
        }
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

    /// A text field that commits on Enter or losing focus; Esc restores it. Empty text commits
    /// only where `empty_ok`: a name (which falls back to the id) or a list of roles, where
    /// empty is a real value. A required value (a number) is refused, so the caller can say why.
    pub(crate) fn text_field(
        &mut self,
        ui: &mut egui::Ui,
        key: &str,
        label: &str,
        current: &str,
        empty_ok: bool,
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
                if text.trim() != current && (empty_ok || !text.trim().is_empty()) {
                    out = Some(text.trim().to_owned());
                }
            } else if r.has_focus() || r.changed() {
                self.drafts.insert(key.to_owned(), text);
            }
        });
        out
    }
}

/// The height of the inspector's place for its block's problems: two lines.
const PROBLEMS_HEIGHT: f32 = 44.0;

/// A folded part of the inspector: `title`, and beside it, while closed, `summary` (what is set
/// in it, or None), which a screen reader hears as the title's description. It starts open
/// when `set`, and opens for a problem's field in it (`wanted`). Its id is `salt`'s.
fn fold(
    ui: &mut egui::Ui,
    salt: &str,
    title: &str,
    summary: &str,
    set: bool,
    wanted: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    let r = egui::CollapsingHeader::new(theme::strong(title))
        .id_salt(salt)
        .default_open(set)
        .open(wanted.then_some(true))
        .show(ui, body);
    let header = r.header_response;
    ui.ctx()
        .accesskit_node_builder(header.id, |n| n.set_description(summary));
    if r.openness > 0.0 {
        return;
    }
    // On the header's line, after its title, cut to the room there is.
    let p = theme::palette(ui.ctx());
    let left = header.rect.right() + 10.0;
    let room = ui.max_rect().right() - left;
    if room < 24.0 {
        return;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(
        summary.to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        p.text_weak,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(room);
    let galley = ui.painter().layout_job(job);
    let top = header.rect.center().y - galley.size().y / 2.0;
    ui.painter()
        .galley(egui::pos2(left, top), galley, p.text_weak);
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
