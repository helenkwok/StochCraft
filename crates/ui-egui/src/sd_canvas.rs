//! The diagram of a system dynamics model on a canvas (ROADMAP M4.2, "the canvas"): the Diagram
//! tab of the system dynamics screens.
//!
//! It draws the model's layout as `sd.get` gives it, in the plain stock-and-flow notation of the
//! textbooks (a stock is a rectangle, an auxiliary a circle, a flow a pipe of two parallel lines
//! with a valve on it and a cloud where it has no stock, an information link a thin curved arrow)
//! and edits the model through the engine's `sd.variable.*` and `sd.layout.set` commands, so every
//! change is one command, one undo step and journalled like an agent's.
//!
//! - Click a variable to select it, or Tab to it (the variables are in name order); the side panel
//!   shows its kind, equation and unit, which can be edited there: Enter or leaving the field
//!   commits it, Escape gives it up, and the engine's refusal (an unknown name, an algebraic loop)
//!   is shown under the field.
//! - Drag a stock, an auxiliary or a valve to move it (one command on release; the pipes at a
//!   stock follow it), and the background to pan. Scroll pans, Ctrl+scroll or pinch zooms.
//! - The tools: Stock and Auxiliary place a variable where the diagram is clicked (or at its middle
//!   on Enter); Flow is drawn by dragging from a stock, or empty space, to a stock or empty space
//!   (empty space is a cloud); Link by dragging from a variable to the one that uses it.
//! - Delete removes the selected variable; the engine refuses while another equation uses it.
//!
//! The diagram fits the model when it is opened and again when the view is resized, until the
//! user pans or zooms.

use std::collections::HashMap;

use egui::{Color32, FontId, Modifiers, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2};
use serde_json::{Value, json};

use crate::i18n::{t, tf};
use crate::sd::kind_label;
use crate::{App, theme};

/// Positions snap to this many diagram units.
const GRID: f64 = 10.0;
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 4.0;
/// The width of the side panel.
const PANEL_WIDTH: f32 = 300.0;
/// The side of a flow's valve, in diagram units (as the layout draws it).
const VALVE: f32 = 20.0;
/// Room round the model when it is fitted to the view, for the names under the shapes.
const MARGIN: f32 = 50.0;

/// What a click or drag on the diagram does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Tool {
    /// Select, move and pan.
    #[default]
    Select,
    Stock,
    Flow,
    Aux,
    Link,
}

/// What a drag on the diagram is doing.
#[derive(Debug, Clone, Default)]
enum Drag {
    #[default]
    None,
    /// Moving a variable by `by` (diagram units).
    Move { name: String, by: Vec2 },
    /// Drawing a flow from a stock (`from`), or from the point `start` (screen), to the pointer.
    Flow {
        from: Option<String>,
        start: Pos2,
        to: Pos2,
    },
    /// Drawing a link from a variable to the pointer.
    Link { from: String, to: Pos2 },
}

/// What the diagram keeps between frames.
#[derive(Debug, Clone)]
pub(crate) struct SdCanvas {
    /// The work area shows the Diagram tab (rather than Model or Results).
    pub(crate) on: bool,
    /// Diagram to screen: `screen = rect.min + offset + diagram * zoom`.
    offset: Vec2,
    zoom: f32,
    /// Fit the model to the view on the next frame (a model was opened).
    fit: bool,
    /// Until the user pans or zooms, the view fits again whenever it changes size; the size it was
    /// last fitted to.
    auto_fit: bool,
    fitted: Option<Vec2>,
    /// The selected variable, by name.
    selected: Option<String>,
    tool: Tool,
    drag: Drag,
    /// The text typed into a field of the side panel and not committed yet, by field.
    drafts: HashMap<&'static str, String>,
    /// The field the engine refused a change to (or `delete`), with its message.
    refused: Option<(&'static str, String)>,
    /// The selection and the engine's revision the drafts are of.
    drafts_of: (Option<String>, u64),
}

impl Default for SdCanvas {
    fn default() -> Self {
        Self {
            on: false,
            offset: Vec2::new(24.0, 24.0),
            zoom: 1.0,
            fit: true,
            auto_fit: true,
            fitted: None,
            selected: None,
            tool: Tool::Select,
            drag: Drag::None,
            drafts: HashMap::new(),
            refused: None,
            drafts_of: (None, 0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Stock,
    Flow,
    Aux,
}

impl Kind {
    /// The kind as `sd.get` names it.
    fn key(self) -> &'static str {
        match self {
            Kind::Stock => "stock",
            Kind::Flow => "flow",
            Kind::Aux => "aux",
        }
    }
}

/// A variable and its shape, as `sd.get` describes them.
#[derive(Debug, Clone)]
struct Var {
    name: String,
    kind: Kind,
    equation: String,
    units: String,
    /// The centre of the shape (a flow's valve) and its size.
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// A flow's pipe, from its source end to its sink end.
    pipe: Vec<(f64, f64)>,
    source: Option<String>,
    sink: Option<String>,
    inflows: Vec<String>,
    outflows: Vec<String>,
    /// It has a shape to draw.
    placed: bool,
}

/// The variables of `info` (`sd.get`'s description of the model).
fn read(info: &Value) -> Vec<Var> {
    info["variables"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|v| {
            let kind = match v["kind"].as_str() {
                Some("stock") => Kind::Stock,
                Some("flow") => Kind::Flow,
                _ => Kind::Aux,
            };
            let names = |key: &str| -> Vec<String> {
                v[key]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            };
            let pipe = v["pipe"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|q| Some((q[0].as_f64()?, q[1].as_f64()?)))
                .collect();
            Var {
                name: v["name"].as_str().unwrap_or("").to_owned(),
                kind,
                equation: v["equation"].as_str().unwrap_or("").to_owned(),
                units: v["units"].as_str().unwrap_or("").to_owned(),
                x: v["x"].as_f64().unwrap_or(0.0),
                y: v["y"].as_f64().unwrap_or(0.0),
                w: v["width"].as_f64().unwrap_or(20.0),
                h: v["height"].as_f64().unwrap_or(20.0),
                pipe,
                source: v["source"].as_str().map(str::to_owned),
                sink: v["sink"].as_str().map(str::to_owned),
                inflows: names("inflows"),
                outflows: names("outflows"),
                placed: v["x"].is_number() && v["y"].is_number(),
            }
        })
        .collect()
}

/// The information links of `info`: from, to and the angle if the link is bent.
fn read_links(info: &Value) -> Vec<(String, String, Option<f64>)> {
    info["connectors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some((
                c["from"].as_str()?.to_owned(),
                c["to"].as_str()?.to_owned(),
                c["angle"].as_f64(),
            ))
        })
        .collect()
}

fn snap(v: f64) -> f64 {
    (v / GRID).round() * GRID
}

/// A name for a new variable that no other has: `Stock 1`, `Stock 2`, … Model names, so English
/// whatever the interface language (an equation names a variable with them).
fn fresh_name(base: &str, vars: &[Var]) -> String {
    let same = |a: &str, b: &str| {
        a.trim().to_lowercase().replace(' ', "_") == b.to_lowercase().replace(' ', "_")
    };
    (1u32..)
        .map(|n| format!("{base} {n}"))
        .find(|name| !vars.iter().any(|v| same(&v.name, name)))
        .unwrap_or_else(|| base.to_owned())
}

/// How the diagram maps its units to the screen.
#[derive(Clone, Copy)]
struct View {
    origin: Pos2,
    zoom: f32,
}

impl View {
    fn pos(&self, x: f64, y: f64) -> Pos2 {
        self.origin + Vec2::new(x as f32, y as f32) * self.zoom
    }

    /// A screen point in diagram units.
    fn diagram(&self, p: Pos2) -> (f64, f64) {
        let v = (p - self.origin) / self.zoom;
        (f64::from(v.x), f64::from(v.y))
    }
}

/// The point on the outline of a shape of `kind` and screen `size`, centred on `c`, that is on
/// the way to `toward`.
fn edge(kind: Kind, size: Vec2, c: Pos2, toward: Pos2) -> Pos2 {
    let d = toward - c;
    let len = d.length();
    if len < 0.5 || !len.is_finite() {
        return c;
    }
    let u = d / len;
    let reach = match kind {
        Kind::Stock => {
            let tx = if u.x.abs() > 1e-6 {
                size.x / 2.0 / u.x.abs()
            } else {
                f32::INFINITY
            };
            let ty = if u.y.abs() > 1e-6 {
                size.y / 2.0 / u.y.abs()
            } else {
                f32::INFINITY
            };
            tx.min(ty)
        }
        _ => size.x / 2.0,
    };
    c + u * reach.min(len)
}

/// A point of the quadratic curve from `a` to `b` bent towards `c`.
fn curve(a: Pos2, c: Pos2, b: Pos2, t: f32) -> Pos2 {
    let u = 1.0 - t;
    Pos2::new(
        u * u * a.x + 2.0 * u * t * c.x + t * t * b.x,
        u * u * a.y + 2.0 * u * t * c.y + t * t * b.y,
    )
}

/// An arrowhead with its point at `tip`, along `dir`.
fn arrowhead(painter: &egui::Painter, tip: Pos2, dir: Vec2, colour: Color32, size: f32) {
    let len = dir.length();
    if len < 1e-3 || !len.is_finite() {
        return;
    }
    let d = dir / len;
    let back = tip - d * size;
    let side = Vec2::new(-d.y, d.x) * size * 0.45;
    painter.add(Shape::convex_polygon(
        vec![tip, back + side, back - side],
        colour,
        Stroke::NONE,
    ));
}

/// The distance from `p` to the segment `a`–`b`.
fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len = ab.length_sq();
    if len <= f32::EPSILON {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// A cloud (the end of a pipe that has no stock) at `at`.
fn cloud(painter: &egui::Painter, at: Pos2, zoom: f32, p: &theme::Palette) {
    let r = (7.0 * zoom).clamp(4.0, 14.0);
    let blobs = [
        Vec2::new(-r * 0.9, r * 0.3),
        Vec2::new(0.0, -r * 0.5),
        Vec2::new(r * 0.9, r * 0.3),
        Vec2::new(0.0, r * 0.5),
    ];
    for b in blobs {
        painter.circle_filled(at + b, r + 1.5, p.text_weak);
    }
    for b in blobs {
        painter.circle_filled(at + b, r, p.canvas);
    }
}

impl App {
    /// The Diagram tab is shown (an agent's command or a click on the tab).
    pub(crate) fn sd_diagram_on(&self) -> bool {
        self.sd_canvas.on
    }

    /// The variable selected on the diagram, by name.
    pub fn sd_selected(&self) -> Option<&str> {
        self.sd_canvas.selected.as_deref()
    }

    /// Keeps the selection and the typed-but-uncommitted text in step with the model: a variable
    /// that is gone is not selected, and a change of the model (an undo, an agent's command) or of
    /// the selection drops what was typed.
    fn sd_canvas_sync(&mut self, vars: &[Var]) {
        if let Some(s) = &self.sd_canvas.selected
            && !vars.iter().any(|v| &v.name == s)
        {
            self.sd_canvas.selected = None;
        }
        let now = (self.sd_canvas.selected.clone(), self.engine.revision());
        if self.sd_canvas.drafts_of != now {
            self.sd_canvas.drafts.clear();
            self.sd_canvas.refused = None;
            self.sd_canvas.drafts_of = now;
        }
    }

    /// The tool buttons, and a line saying what the tool in hand does.
    fn sd_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let tools = [
                (Tool::Select, t("Select")),
                (Tool::Stock, t("Stock")),
                (Tool::Flow, t("Flow")),
                (Tool::Aux, t("Auxiliary")),
                (Tool::Link, t("Link")),
            ];
            for (tool, label) in tools {
                let on = self.sd_canvas.tool == tool;
                let r = ui.add(egui::Button::selectable(on, label));
                r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
                if r.clicked() {
                    self.sd_canvas.tool = tool;
                    self.sd_canvas.drag = Drag::None;
                }
            }
            let hint = match self.sd_canvas.tool {
                Tool::Select => None,
                Tool::Stock => Some(t("Click the diagram to place a stock.")),
                Tool::Aux => Some(t("Click the diagram to place an auxiliary.")),
                Tool::Flow => Some(t(
                    "Drag from a stock, or from empty space, to another stock or empty space to draw a flow.",
                )),
                Tool::Link => Some(t(
                    "Drag from one variable to another to show that the second uses the first.",
                )),
            };
            if let Some(hint) = hint {
                ui.label(theme::weak(ui, hint));
            }
        });
    }

    /// Zoom out, actual size, zoom in and fit, in the diagram's bottom-right corner.
    fn sd_zoom_controls(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let percent = format!("{:.0}%", self.sd_canvas.zoom * 100.0);
        let buttons: [(String, &str, f32); 4] = [
            ("−".to_owned(), t("Zoom out"), 28.0),
            (percent, t("Actual size"), 60.0),
            ("+".to_owned(), t("Zoom in"), 28.0),
            (t("Fit").to_owned(), t("Fit the diagram to the view"), 44.0),
        ];
        let total: f32 = buttons.iter().map(|b| b.2 + 4.0).sum();
        let mut x = rect.right() - 8.0 - total;
        let y = rect.bottom() - 8.0 - 24.0;
        for (i, (label, hint, w)) in buttons.into_iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 24.0));
            x += w + 4.0;
            if ui
                .put(
                    r,
                    egui::Button::new(egui::RichText::new(label).size(12.0))
                        .wrap_mode(egui::TextWrapMode::Extend),
                )
                .on_hover_text(hint)
                .clicked()
            {
                let about = rect.center() - rect.min;
                match i {
                    0 => self.sd_zoom_at(1.0 / 1.25, about),
                    1 => self.sd_zoom_at(1.0 / self.sd_canvas.zoom, about),
                    2 => self.sd_zoom_at(1.25, about),
                    _ => self.sd_canvas.fit = true,
                }
            }
        }
    }

    /// Zooms keeping the point `at` (relative to the diagram's corner) where it is.
    fn sd_zoom_at(&mut self, factor: f32, at: Vec2) {
        let c = &mut self.sd_canvas;
        c.auto_fit = false;
        let z = (c.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let f = z / c.zoom;
        c.offset = at - (at - c.offset) * f;
        c.zoom = z;
    }

    /// Fits the model in `rect`, at most at its actual size.
    fn sd_fit(&mut self, rect: Rect, vars: &[Var]) {
        let mut bounds: Option<Rect> = None;
        let mut grow = |r: Rect| {
            bounds = Some(bounds.map_or(r, |b| b.union(r)));
        };
        for v in vars.iter().filter(|v| v.placed) {
            let (x, y) = (v.x as f32, v.y as f32);
            let half = match v.kind {
                Kind::Flow => Vec2::splat(VALVE / 2.0),
                _ => Vec2::new(v.w as f32, v.h as f32) / 2.0,
            };
            grow(Rect::from_center_size(Pos2::new(x, y), half * 2.0));
            for q in &v.pipe {
                grow(Rect::from_center_size(
                    Pos2::new(q.0 as f32, q.1 as f32),
                    Vec2::splat(16.0),
                ));
            }
        }
        let Some(b) = bounds else {
            return;
        };
        let w = b.width() + 2.0 * MARGIN;
        let h = b.height() + 2.0 * MARGIN;
        let zoom = (rect.width() / w)
            .min(rect.height() / h)
            .clamp(MIN_ZOOM, 1.0);
        let c = &mut self.sd_canvas;
        c.zoom = zoom;
        c.offset = rect.size() / 2.0 - b.center().to_vec2() * zoom;
        c.fit = false;
    }

    /// Runs a command that edits the model, saying what it did in the status bar, or why it could
    /// not. Whether it worked.
    fn sd_apply(&mut self, command: &str, params: Value, done: String) -> bool {
        match self.engine.execute(command, params) {
            Ok(_) => {
                self.info(done);
                true
            }
            Err(e) => {
                self.report_error(e.message);
                false
            }
        }
    }

    /// Moves the shape of `name` so its centre is at (x, y): one command, one undo step. A stock
    /// takes the pipes at it along.
    fn sd_move(&mut self, name: &str, x: f64, y: f64) {
        let done = tf("Moved {name}", &[("name", name)]);
        self.sd_apply(
            "sd.layout.set",
            json!({ "shapes": [{ "name": name, "x": x, "y": y }] }),
            done,
        );
    }

    /// Adds a stock or an auxiliary with its shape centred on `at` (diagram units), and selects it.
    fn sd_add(&mut self, kind: Kind, at: (f64, f64), vars: &[Var]) {
        let base = if kind == Kind::Stock {
            "Stock"
        } else {
            "Auxiliary"
        };
        let name = fresh_name(base, vars);
        let params = json!({
            "kind": kind.key(),
            "name": name,
            "equation": "0",
            "position": { "x": snap(at.0), "y": snap(at.1) },
        });
        let done = tf(
            "Added {kind}: {name}",
            &[("kind", kind_label(kind.key())), ("name", &name)],
        );
        if self.sd_apply("sd.variable.add", params, done) {
            self.sd_canvas.selected = Some(name);
            self.sd_canvas.tool = Tool::Select;
        }
    }

    /// Adds a flow from the stock `from` (or from the point `start`, a cloud) to the stock `to`
    /// (or to the point `end`), and selects it. The valve is put on the way between them.
    fn sd_add_flow(
        &mut self,
        from: Option<&Var>,
        start: (f64, f64),
        to: Option<&Var>,
        end: (f64, f64),
        vars: &[Var],
    ) {
        if let (Some(a), Some(b)) = (from, to)
            && a.name == b.name
        {
            return;
        }
        let s = from.map_or((snap(start.0), snap(start.1)), |v| (v.x, v.y));
        let e = to.map_or((snap(end.0), snap(end.1)), |v| (v.x, v.y));
        // A pipe with a cloud at one end keeps the cloud where the drag was: the engine puts it
        // 50 units from the valve, along the pipe.
        let valve = match (from.is_some(), to.is_some()) {
            (false, true) => (s.0 + 50.0, s.1),
            (true, false) => (e.0 - 50.0, e.1),
            _ => (snap((s.0 + e.0) / 2.0), snap((s.1 + e.1) / 2.0)),
        };
        let name = fresh_name("Flow", vars);
        let mut params = json!({
            "kind": "flow",
            "name": name,
            "equation": "0",
            "position": { "x": valve.0, "y": valve.1 },
        });
        if let Some(v) = from {
            params["from_stock"] = json!(v.name);
        }
        if let Some(v) = to {
            params["into_stock"] = json!(v.name);
        }
        let done = tf(
            "Added {kind}: {name}",
            &[("kind", kind_label("flow")), ("name", &name)],
        );
        if self.sd_apply("sd.variable.add", params, done) {
            self.sd_canvas.selected = Some(name);
            self.sd_canvas.tool = Tool::Select;
        }
    }

    /// Adds an information link from `from` to `to`. (The equation of `to` is the user's to edit:
    /// a link is kept by an equation that uses the variable.)
    fn sd_link(&mut self, from: &str, to: &str) {
        let done = tf("Linked {from} to {to}", &[("from", from), ("to", to)]);
        self.sd_apply(
            "sd.layout.set",
            json!({ "add_connectors": [{ "from": from, "to": to }] }),
            done,
        );
    }

    /// Removes the variable `name`; if the engine refuses (another equation uses it) the status
    /// bar and the side panel say why.
    fn sd_delete(&mut self, name: &str) {
        match self
            .engine
            .execute("sd.variable.remove", json!({ "name": name }))
        {
            Ok(_) => {
                self.sd_canvas.selected = None;
                self.sd_canvas.refused = None;
                self.info(tf("Removed {name}", &[("name", name)]));
            }
            Err(e) => {
                self.sd_canvas.refused = Some(("delete", e.message.clone()));
                self.report_error(e.message);
            }
        }
    }

    /// Commits a field of the side panel: `value` for the field `key` of the variable `name`. A
    /// refusal keeps the text in the field and says why under it.
    fn sd_commit(&mut self, name: &str, key: &'static str, value: String) {
        let params = match key {
            "name" => json!({ "name": name, "rename": value }),
            "equation" => json!({ "name": name, "equation": value }),
            _ => json!({ "name": name, "units": value }),
        };
        match self.engine.execute("sd.variable.set", params) {
            Ok(_) => {
                let shown = if key == "name" {
                    value.clone()
                } else {
                    name.to_owned()
                };
                if key == "name" {
                    self.sd_canvas.selected = Some(value);
                }
                self.sd_canvas.refused = None;
                self.info(tf("Edited {name}", &[("name", &shown)]));
            }
            Err(e) => {
                self.sd_canvas.drafts.insert(key, value);
                self.sd_canvas.refused = Some((key, e.message));
            }
        }
    }

    /// A text field of the side panel, labelled `label`: what it commits (on Enter or when it
    /// loses the keyboard, if it changed), and the refusal of the last commit under it.
    fn sd_field(
        &mut self,
        ui: &mut egui::Ui,
        key: &'static str,
        label: &str,
        current: &str,
    ) -> Option<String> {
        let p = theme::palette(ui.ctx());
        let mut out = None;
        ui.label(label);
        let mut text = self
            .sd_canvas
            .drafts
            .get(key)
            .cloned()
            .unwrap_or_else(|| current.to_owned());
        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
        crate::form::field_outline(ui, &r);
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
        if r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.sd_canvas.drafts.remove(key);
            self.sd_canvas.refused = None;
            r.surrender_focus();
        } else if r.lost_focus() {
            self.sd_canvas.drafts.remove(key);
            if text.trim() != current {
                out = Some(text.trim().to_owned());
            }
        } else if r.has_focus() || r.changed() {
            if r.changed() {
                self.sd_canvas.refused = None;
            }
            self.sd_canvas.drafts.insert(key, text);
        }
        if let Some((k, message)) = &self.sd_canvas.refused
            && *k == key
        {
            ui.add(egui::Label::new(egui::RichText::new(message).color(p.error)).wrap());
        }
        ui.add_space(6.0);
        out
    }

    /// The side panel: the selected variable's kind and name, its equation (a stock's initial
    /// value) and unit to edit, its flows, and Delete; or what to do to select one.
    fn sd_panel(&mut self, ui: &mut egui::Ui, v: Option<&Var>) {
        let Some(v) = v else {
            ui.add(
                egui::Label::new(theme::weak(
                    ui,
                    t("Click a variable to select it and edit it here. Drag a variable to move it."),
                ))
                .wrap(),
            );
            return;
        };
        let p = theme::palette(ui.ctx());
        let mut commits: Vec<(&'static str, String)> = Vec::new();
        let mut delete = false;
        egui::ScrollArea::vertical()
            .id_salt("sd/panel")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        theme::strong(tf(
                            "{kind} {name}",
                            &[("kind", kind_label(v.kind.key())), ("name", &v.name)],
                        ))
                        .size(15.0),
                    )
                    .wrap(),
                );
                ui.add_space(8.0);
                if let Some(x) = self.sd_field(ui, "name", t("Name"), &v.name) {
                    commits.push(("name", x));
                }
                let equation = if v.kind == Kind::Stock {
                    t("Initial value")
                } else {
                    t("Equation")
                };
                if let Some(x) = self.sd_field(ui, "equation", equation, &v.equation) {
                    commits.push(("equation", x));
                }
                if let Some(x) = self.sd_field(ui, "units", t("Unit"), &v.units) {
                    commits.push(("units", x));
                }
                if v.kind == Kind::Stock {
                    if !v.inflows.is_empty() {
                        let flows = v.inflows.join(", ");
                        ui.add(egui::Label::new(tf("In: {flows}", &[("flows", &flows)])).wrap());
                    }
                    if !v.outflows.is_empty() {
                        let flows = v.outflows.join(", ");
                        ui.add(egui::Label::new(tf("Out: {flows}", &[("flows", &flows)])).wrap());
                    }
                }
                ui.add_space(10.0);
                delete = ui.add(theme::secondary(ui, t("Delete"))).clicked();
                if let Some((k, message)) = &self.sd_canvas.refused
                    && *k == "delete"
                {
                    ui.add(egui::Label::new(egui::RichText::new(message).color(p.error)).wrap());
                }
            });
        for (key, value) in commits {
            self.sd_commit(&v.name, key, value);
        }
        if delete {
            self.sd_delete(&v.name);
        }
    }

    /// The Diagram tab: the tools, the diagram, and the side panel.
    pub(crate) fn sd_diagram_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let info = self.sd_info();
        let vars = read(&info);
        let links = read_links(&info);
        self.sd_canvas_sync(&vars);
        self.sd_toolbar(ui);
        ui.add_space(4.0);

        let avail = ui.available_rect_before_wrap();
        if avail.width() < 160.0 || avail.height() < 120.0 {
            return;
        }
        let panel_w = PANEL_WIDTH.min(avail.width() * 0.4);
        let rect = Rect::from_min_max(
            avail.min,
            Pos2::new(avail.max.x - panel_w - 8.0, avail.max.y),
        );
        let panel = Rect::from_min_max(Pos2::new(avail.max.x - panel_w, avail.min.y), avail.max);
        let at: HashMap<&str, usize> = vars
            .iter()
            .enumerate()
            .map(|(i, v)| (v.name.as_str(), i))
            .collect();
        let selected: Option<usize> = self
            .sd_canvas
            .selected
            .as_deref()
            .and_then(|s| at.get(s).copied());

        let bg = ui.interact(rect, egui::Id::new("sd-diagram"), Sense::click_and_drag());
        bg.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, t("Diagram")));
        let resized = self.sd_canvas.auto_fit && self.sd_canvas.fitted != Some(rect.size());
        if (self.sd_canvas.fit || resized) && rect.width() > 1.0 {
            self.sd_fit(rect, &vars);
            self.sd_canvas.fitted = Some(rect.size());
        }

        // Pan and zoom.
        if ui.rect_contains_pointer(rect) {
            let (scroll, zoom, ptr, ctrl) = ui.input(|i| {
                (
                    i.smooth_scroll_delta,
                    i.zoom_delta(),
                    i.pointer.hover_pos(),
                    i.modifiers.command,
                )
            });
            if (zoom - 1.0).abs() > f32::EPSILON {
                let about = ptr.unwrap_or(rect.center()) - rect.min;
                self.sd_zoom_at(zoom, about);
            } else if !ctrl && scroll != Vec2::ZERO {
                self.sd_canvas.offset += scroll;
                self.sd_canvas.auto_fit = false;
            }
        }
        let panning = bg.dragged_by(egui::PointerButton::Middle)
            || bg.dragged_by(egui::PointerButton::Secondary)
            || (bg.dragged_by(egui::PointerButton::Primary)
                && self.sd_canvas.tool == Tool::Select
                && matches!(self.sd_canvas.drag, Drag::None));
        if panning {
            self.sd_canvas.offset += bg.drag_delta();
            self.sd_canvas.auto_fit = false;
        }

        let view = View {
            origin: rect.min + self.sd_canvas.offset,
            zoom: self.sd_canvas.zoom,
        };
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, p.canvas);
        painter.rect_stroke(rect, 6.0, Stroke::new(1.0, p.border), StrokeKind::Inside);
        let step = 50.0 * view.zoom;
        if step >= 14.0 {
            let (sx, sy) = (
                rect.left() + (view.origin.x - rect.left()).rem_euclid(step),
                rect.top() + (view.origin.y - rect.top()).rem_euclid(step),
            );
            let mut y = sy;
            while y < rect.bottom() {
                let mut x = sx;
                while x < rect.right() {
                    painter.circle_filled(Pos2::new(x, y), 1.0, p.border);
                    x += step;
                }
                y += step;
            }
        }

        // What the drag in hand moves, in diagram units.
        let moving: Option<(String, Vec2)> = match &self.sd_canvas.drag {
            Drag::Move { name, by } => Some((name.clone(), *by)),
            _ => None,
        };
        let centre = |v: &Var| {
            let by = moving
                .as_ref()
                .filter(|(name, _)| *name == v.name)
                .map_or(Vec2::ZERO, |(_, by)| *by);
            view.pos(v.x + f64::from(by.x), v.y + f64::from(by.y))
        };
        let size = |v: &Var| match v.kind {
            Kind::Flow => Vec2::splat(VALVE * view.zoom),
            _ => Vec2::new(v.w as f32, v.h as f32) * view.zoom,
        };
        let on = |v: &Var| selected.is_some_and(|i| vars[i].name == v.name);

        // Information links, under everything.
        let link_stroke = Stroke::new(1.3, p.text_weak);
        for (from, to, angle) in &links {
            let (Some(&a), Some(&b)) = (at.get(from.as_str()), at.get(to.as_str())) else {
                continue;
            };
            let (va, vb) = (&vars[a], &vars[b]);
            if !va.placed || !vb.placed {
                continue;
            }
            let (ca, cb) = (centre(va), centre(vb));
            let dist = (cb - ca).length();
            if dist < 1.0 {
                continue;
            }
            let ctrl = match angle {
                Some(deg) => {
                    let rad = deg.to_radians();
                    let dir = Vec2::new(
                        stochcraft_random::math::cos(rad) as f32,
                        -stochcraft_random::math::sin(rad) as f32,
                    );
                    ca + dir * dist * 0.5
                }
                None => {
                    let u = (cb - ca) / dist;
                    ca + (cb - ca) * 0.5 + Vec2::new(-u.y, u.x) * dist * 0.2
                }
            };
            let start = edge(va.kind, size(va), ca, ctrl);
            let end = edge(vb.kind, size(vb), cb, ctrl);
            let points: Vec<Pos2> = (0..=16)
                .map(|k| curve(start, ctrl, end, k as f32 / 16.0))
                .collect();
            if let [.., before, last] = points.as_slice() {
                painter.add(Shape::line(points.clone(), link_stroke));
                arrowhead(
                    &painter,
                    *last,
                    *last - *before,
                    link_stroke.color,
                    8.0 * view.zoom.clamp(0.7, 1.4),
                );
            }
        }

        // Pipes, as two parallel lines, with an arrowhead where they enter a stock or a cloud.
        for v in vars.iter().filter(|v| v.kind == Kind::Flow && v.placed) {
            let pts: Vec<Pos2> = v.pipe.iter().map(|q| view.pos(q.0, q.1)).collect();
            let (Some(&first), Some(&last)) = (pts.first(), pts.last()) else {
                continue;
            };
            if pts.len() < 2 {
                continue;
            }
            let colour = if on(v) { p.accent } else { p.text };
            let width = (7.0 * view.zoom).clamp(4.0, 12.0);
            painter.add(Shape::line(pts.clone(), Stroke::new(width, colour)));
            painter.add(Shape::line(pts.clone(), Stroke::new(width - 3.0, p.canvas)));
            if let [.., before, end] = pts.as_slice() {
                arrowhead(&painter, *end, *end - *before, colour, width * 1.6);
            }
            if v.source.is_none() {
                cloud(&painter, first, view.zoom, &p);
            }
            if v.sink.is_none() {
                cloud(&painter, last, view.zoom, &p);
            }
        }

        // The shapes, each its own widget, in name order (the keyboard's Tab order).
        let font = FontId::proportional((13.0 * view.zoom).clamp(8.0, 22.0));
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let tool = self.sd_canvas.tool;
        let mut order: Vec<usize> = (0..vars.len()).collect();
        order.sort_by_key(|&i| vars[i].name.to_lowercase());
        let mut hits: Vec<(usize, Rect)> = Vec::new();
        let mut clicked: Option<usize> = None;
        let mut focused: Option<usize> = None;
        let mut started: Option<(usize, Pos2)> = None;
        let mut dragging = Vec2::ZERO;
        for &i in &order {
            let v = &vars[i];
            if !v.placed {
                continue;
            }
            let c = centre(v);
            let s = size(v);
            let selected_now = on(v);
            let ink = if selected_now { p.accent } else { p.text };
            let stroke = Stroke::new(if selected_now { 2.5 } else { 1.8 }, ink);
            let fill = if selected_now {
                p.accent_soft
            } else {
                p.raised
            };
            let body = Rect::from_center_size(c, s);
            // Where the name goes: under the shape.
            let name_at = match v.kind {
                Kind::Flow => c.y + 12.0 * view.zoom,
                _ => body.bottom() + 4.0 * view.zoom,
            };
            match v.kind {
                Kind::Stock => {
                    painter.rect_filled(body, 2.0, fill);
                    painter.rect_stroke(body, 2.0, stroke, StrokeKind::Inside);
                }
                Kind::Aux => {
                    painter.circle(c, s.x / 2.0, fill, stroke);
                }
                Kind::Flow => {
                    // A bow-tie across the pipe: two triangles meeting at the valve's centre.
                    let horizontal = v
                        .pipe
                        .windows(2)
                        .map(|w| {
                            let (a, b) = (view.pos(w[0].0, w[0].1), view.pos(w[1].0, w[1].1));
                            (
                                distance_to_segment(c, a, b),
                                (b.y - a.y).abs() <= (b.x - a.x).abs(),
                            )
                        })
                        .min_by(|a, b| a.0.total_cmp(&b.0))
                        .is_none_or(|(_, h)| h);
                    let r = s.x / 2.0 * 0.8;
                    let tris = if horizontal {
                        [
                            vec![c + Vec2::new(-r, -r), c + Vec2::new(-r, r), c],
                            vec![c + Vec2::new(r, -r), c + Vec2::new(r, r), c],
                        ]
                    } else {
                        [
                            vec![c + Vec2::new(-r, -r), c + Vec2::new(r, -r), c],
                            vec![c + Vec2::new(-r, r), c + Vec2::new(r, r), c],
                        ]
                    };
                    for tri in tris {
                        painter.add(Shape::convex_polygon(tri, fill, stroke));
                    }
                }
            }
            let g = painter.layout(
                v.name.clone(),
                font.clone(),
                p.text,
                (110.0 * view.zoom).max(40.0),
            );
            painter.galley(Pos2::new(c.x - g.size().x / 2.0, name_at), g, p.text);

            // Only the part inside the diagram takes the pointer, and at least a finger's width.
            let hit = Rect::from_center_size(c, s.max(Vec2::splat(16.0))).intersect(rect);
            let resp = ui.interact(
                hit,
                egui::Id::new(("sd-shape", &v.name)),
                if hit.is_positive() {
                    Sense::click_and_drag()
                } else {
                    Sense::hover()
                },
            );
            let label = tf(
                "{kind} {name}",
                &[("kind", kind_label(v.kind.key())), ("name", &v.name)],
            );
            resp.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, true, selected_now, &label)
            });
            // The keyboard's place: a ring outside the shape, apart from the selection's outline
            // and fill (WCAG 2.4.7).
            if resp.has_focus() {
                let ring = Stroke::new(2.0, p.accent);
                match v.kind {
                    Kind::Aux => {
                        painter.circle_stroke(c, s.x / 2.0 + 4.0, ring);
                    }
                    _ => {
                        painter.rect_stroke(body.expand(4.0), 4.0, ring, StrokeKind::Outside);
                    }
                }
            }
            if resp.gained_focus() {
                focused = Some(i);
            }
            if resp.clicked() {
                clicked = Some(i);
            }
            if resp.drag_started_by(egui::PointerButton::Primary) {
                started = Some((i, c));
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                dragging += resp.drag_delta();
            }
            hits.push((i, hit));
        }
        if bg.has_focus() {
            painter.rect_stroke(
                rect.shrink(1.0),
                4.0,
                Stroke::new(2.0, p.accent),
                StrokeKind::Inside,
            );
        }

        // Clicks, focus and the start of a drag on a shape.
        let mut pending: Option<Option<String>> =
            focused.or(clicked).map(|i| Some(vars[i].name.clone()));
        if let Some((i, c)) = started
            && matches!(self.sd_canvas.drag, Drag::None)
        {
            let v = &vars[i];
            let to = pointer.unwrap_or(c);
            match tool {
                Tool::Select => {
                    pending = Some(Some(v.name.clone()));
                    self.sd_canvas.drag = Drag::Move {
                        name: v.name.clone(),
                        by: Vec2::ZERO,
                    };
                }
                Tool::Flow if v.kind == Kind::Stock => {
                    self.sd_canvas.drag = Drag::Flow {
                        from: Some(v.name.clone()),
                        start: c,
                        to,
                    };
                }
                Tool::Link => {
                    self.sd_canvas.drag = Drag::Link {
                        from: v.name.clone(),
                        to,
                    };
                }
                _ => {}
            }
        }
        if bg.drag_started_by(egui::PointerButton::Primary)
            && tool == Tool::Flow
            && matches!(self.sd_canvas.drag, Drag::None)
        {
            let start = ui
                .input(|i| i.pointer.press_origin())
                .or(pointer)
                .unwrap_or(rect.center());
            self.sd_canvas.drag = Drag::Flow {
                from: None,
                start,
                to: pointer.unwrap_or(start),
            };
        }
        match &mut self.sd_canvas.drag {
            Drag::Move { by, .. } => *by += dragging / view.zoom,
            Drag::Flow { to, .. } | Drag::Link { to, .. } => {
                if let Some(pt) = pointer {
                    *to = pt;
                }
            }
            Drag::None => {}
        }

        // The drag in hand, drawn.
        match &self.sd_canvas.drag {
            Drag::Flow { from, start, to } => {
                let a = from
                    .as_deref()
                    .and_then(|n| at.get(n))
                    .map_or(*start, |&k| centre(&vars[k]));
                painter.line_segment([a, *to], Stroke::new(3.0, p.accent));
                arrowhead(&painter, *to, *to - a, p.accent, 9.0);
            }
            Drag::Link { from, to } => {
                if let Some(&k) = at.get(from.as_str()) {
                    let a = centre(&vars[k]);
                    painter.line_segment([a, *to], Stroke::new(1.5, p.accent));
                    arrowhead(&painter, *to, *to - a, p.accent, 9.0);
                }
            }
            _ => {}
        }

        // The drag ends where the button is let go.
        let released = !ui.input(|i| i.pointer.primary_down());
        if released && !matches!(self.sd_canvas.drag, Drag::None) {
            match std::mem::take(&mut self.sd_canvas.drag) {
                Drag::Move { name, by } => {
                    if by.length() * view.zoom >= 1.0
                        && let Some(&k) = at.get(name.as_str())
                    {
                        let v = &vars[k];
                        self.sd_move(
                            &name,
                            snap(v.x + f64::from(by.x)),
                            snap(v.y + f64::from(by.y)),
                        );
                    }
                }
                Drag::Link { from, to } => {
                    let target = hits
                        .iter()
                        .rev()
                        .find(|(k, r)| vars[*k].name != from && r.expand(4.0).contains(to))
                        .map(|(k, _)| vars[*k].name.clone());
                    if let Some(target) = target {
                        self.sd_link(&from, &target);
                    }
                }
                Drag::Flow { from, start, to } => {
                    let target = hits
                        .iter()
                        .rev()
                        .find(|(k, r)| vars[*k].kind == Kind::Stock && r.contains(to))
                        .map(|(k, _)| *k);
                    if rect.contains(to) {
                        let source = from.as_deref().and_then(|n| at.get(n)).map(|&k| &vars[k]);
                        self.sd_add_flow(
                            source,
                            view.diagram(start),
                            target.map(|k| &vars[k]),
                            view.diagram(to),
                            &vars,
                        );
                    }
                }
                Drag::None => {}
            }
        }

        // The background: a click clears the selection, or picks the pipe it is on; with the
        // Stock or Auxiliary tool it places one there.
        if bg.clicked()
            && let Some(pt) = bg.interact_pointer_pos()
        {
            match tool {
                Tool::Stock => self.sd_add(Kind::Stock, view.diagram(pt), &vars),
                Tool::Aux => self.sd_add(Kind::Aux, view.diagram(pt), &vars),
                Tool::Select => {
                    let pipe = vars
                        .iter()
                        .filter(|v| v.kind == Kind::Flow && v.pipe.len() > 1)
                        .filter(|v| {
                            v.pipe.windows(2).any(|w| {
                                distance_to_segment(
                                    pt,
                                    view.pos(w[0].0, w[0].1),
                                    view.pos(w[1].0, w[1].1),
                                ) <= 6.0
                            })
                        })
                        .map(|v| v.name.clone())
                        .next();
                    pending = Some(pipe);
                }
                _ => {}
            }
        }
        // The keyboard's way to place one: Enter on the diagram puts it in the middle.
        if bg.has_focus()
            && matches!(tool, Tool::Stock | Tool::Aux)
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            let kind = if tool == Tool::Stock {
                Kind::Stock
            } else {
                Kind::Aux
            };
            self.sd_add(kind, view.diagram(rect.center()), &vars);
        }

        self.sd_zoom_controls(ui, rect);

        // Delete removes the selection, Escape lets go of the tool or the selection; not while a
        // field is being typed in.
        if !ui.ctx().egui_wants_keyboard_input() {
            if let Some(i) = selected
                && ui.input_mut(|k| {
                    k.consume_key(Modifiers::NONE, egui::Key::Delete)
                        || k.consume_key(Modifiers::NONE, egui::Key::Backspace)
                })
            {
                self.sd_delete(&vars[i].name);
            } else if ui.input_mut(|k| k.consume_key(Modifiers::NONE, egui::Key::Escape)) {
                if self.sd_canvas.tool == Tool::Select {
                    self.sd_canvas.selected = None;
                } else {
                    self.sd_canvas.tool = Tool::Select;
                    self.sd_canvas.drag = Drag::None;
                }
            }
        }

        // The side panel, in the space beside the diagram.
        let selected_var = self
            .sd_canvas
            .selected
            .as_deref()
            .and_then(|s| at.get(s))
            .map(|&k| &vars[k]);
        ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
            self.sd_panel(ui, selected_var);
        });
        // A new selection takes effect after the panel was drawn for the old one, so a field that
        // lost the keyboard to the click that selects commits to the variable it was typed in.
        if let Some(next) = pending
            && next != self.sd_canvas.selected
        {
            self.sd_canvas.selected = next;
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn var(name: &str) -> Var {
        Var {
            name: name.to_owned(),
            kind: Kind::Aux,
            equation: String::new(),
            units: String::new(),
            x: 0.0,
            y: 0.0,
            w: 20.0,
            h: 20.0,
            pipe: Vec::new(),
            source: None,
            sink: None,
            inflows: Vec::new(),
            outflows: Vec::new(),
            placed: true,
        }
    }

    #[test]
    fn a_new_name_is_one_no_variable_has() {
        assert_eq!(fresh_name("Stock", &[]), "Stock 1");
        let taken = [var("Stock 1"), var("stock_2")];
        assert_eq!(fresh_name("Stock", &taken), "Stock 3");
        assert_eq!(fresh_name("Flow", &taken), "Flow 1");
    }

    #[test]
    fn positions_snap_to_the_grid() {
        assert_eq!(snap(14.0), 10.0);
        assert_eq!(snap(15.0), 20.0);
        assert_eq!(snap(-14.0), -10.0);
    }

    #[test]
    fn a_link_leaves_a_stock_on_its_edge() {
        let c = Pos2::new(100.0, 100.0);
        let size = Vec2::new(60.0, 40.0);
        let p = edge(Kind::Stock, size, c, Pos2::new(300.0, 100.0));
        assert!(
            (p.x - 130.0).abs() < 1e-3 && (p.y - 100.0).abs() < 1e-3,
            "{p:?}"
        );
        let p = edge(Kind::Stock, size, c, Pos2::new(100.0, 0.0));
        assert!(
            (p.x - 100.0).abs() < 1e-3 && (p.y - 80.0).abs() < 1e-3,
            "{p:?}"
        );
    }
}
