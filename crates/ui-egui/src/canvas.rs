//! The flowchart canvas (M3.4): the model drawn as BPMN shapes, where the diagram puts them.
//!
//! - Drag a shape to move it (with the rest of the selection); drag from a shape's handle to
//!   another shape to connect them; drag a kind from the palette to add a block there.
//! - Click to select, Shift- or Ctrl-click to add to the selection, drag on the background to
//!   select with a rectangle; Select All, Copy, Cut, Paste, Duplicate and Delete work on the
//!   selection, and several selected blocks can be aligned and spread out.
//! - Scroll to pan (or drag with the middle or right button), Ctrl+scroll or pinch to zoom.
//!
//! Every change is one engine command (a `batch` where it takes several), so each undoes in
//! one step. Positions are BPMN DI, saved in the model file.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Align2, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2};
use serde_json::{Value, json};
use stochcraft_model::{Bounds, ElementId, ElementKind, Model};

use crate::editor::KINDS;
use crate::i18n::{t, tf, tnf};
use crate::{App, theme};

/// Positions snap to this many diagram units.
const GRID: f64 = 10.0;
/// A paste lands this far down and to the right of what was copied (or the last paste).
const PASTE_STEP: f64 = 30.0;
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 4.0;

/// What a drag on the canvas is doing.
#[derive(Debug, Clone, Default)]
enum Drag {
    #[default]
    None,
    /// Moving these blocks by `by` (diagram units).
    Move { ids: Vec<String>, by: Vec2 },
    /// Selecting with a rectangle (screen points); `add` keeps the selection there was.
    Marquee { from: Pos2, to: Pos2, add: bool },
    /// Drawing a connection from a block to the pointer.
    Connect { from: String, to: Pos2 },
}

/// The canvas's view and selection, kept between frames.
#[derive(Debug, Clone)]
pub(crate) struct Canvas {
    /// Diagram to screen: `screen = rect.min + offset + diagram * zoom`.
    offset: Vec2,
    zoom: f32,
    /// Fit the drawing to the view on the next frame (a model was opened).
    pub(crate) fit: bool,
    /// Until the user pans or zooms, the view fits again whenever the canvas changes size; the
    /// size it was last fitted to.
    auto_fit: bool,
    fitted: Option<Vec2>,
    /// The selection when it is more than one block (`App::selected` is then `None`).
    pub(crate) multi: BTreeSet<String>,
    /// A selected connection (Delete removes it).
    pub(crate) flow: Option<String>,
    drag: Drag,
    /// What Copy or Cut took, as the clipboard text; and how often it has been pasted.
    clipboard: Option<Value>,
    pastes: u32,
    /// Copied text for the system's clipboard, handed over on the next frame.
    to_system: Option<String>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            offset: Vec2::new(24.0, 24.0),
            zoom: 1.0,
            fit: true,
            auto_fit: true,
            fitted: None,
            multi: BTreeSet::new(),
            flow: None,
            drag: Drag::None,
            clipboard: None,
            pastes: 0,
            to_system: None,
        }
    }
}

/// How the canvas maps diagram units to the screen.
#[derive(Clone, Copy)]
struct View {
    origin: Pos2,
    zoom: f32,
}

impl View {
    fn pos(&self, x: f64, y: f64) -> Pos2 {
        self.origin + Vec2::new(x as f32, y as f32) * self.zoom
    }

    fn rect(&self, b: &Bounds) -> Rect {
        Rect::from_min_max(self.pos(b.x, b.y), self.pos(b.x + b.width, b.y + b.height))
    }

    /// A screen point in diagram units.
    fn diagram(&self, p: Pos2) -> (f64, f64) {
        let v = (p - self.origin) / self.zoom;
        (f64::from(v.x), f64::from(v.y))
    }
}

fn snap(v: f64) -> f64 {
    (v / GRID).round() * GRID
}

/// The block kind's key, as the engine names it.
fn kind_key(kind: &ElementKind) -> &'static str {
    kind.name()
}

/// Ends of an edge between two shapes: from the middle of a side of the first to the facing
/// side of the second (as `stochcraft-io` writes new edges).
fn edge_ends(a: &Rect, b: &Rect) -> (Pos2, Pos2) {
    if b.left() >= a.right() {
        (a.right_center(), b.left_center())
    } else if b.right() <= a.left() {
        (a.left_center(), b.right_center())
    } else if b.center().y >= a.center().y {
        (a.center_bottom(), b.center_top())
    } else {
        (a.center_top(), b.center_bottom())
    }
}

fn arrow(painter: &egui::Painter, from: Pos2, to: Pos2, stroke: Stroke, zoom: f32) {
    painter.line_segment([from, to], stroke);
    let dir = (to - from).normalized();
    if !dir.x.is_finite() || !dir.y.is_finite() {
        return;
    }
    let size = 9.0 * zoom.clamp(0.6, 1.6);
    let back = to - dir * size;
    let side = Vec2::new(-dir.y, dir.x) * size * 0.45;
    painter.add(Shape::convex_polygon(
        vec![to, back + side, back - side],
        stroke.color,
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

/// An id like `base` that is not in `taken`: `base-copy`, `base-copy-2`, …
fn fresh_id(base: &str, taken: &BTreeSet<String>) -> String {
    let stem = format!("{base}-copy");
    if !taken.contains(&stem) {
        return stem;
    }
    (2u32..)
        .map(|n| format!("{stem}-{n}"))
        .find(|id| !taken.contains(id))
        .unwrap_or(stem)
}

impl App {
    /// The blocks selected now: several, one, or none.
    pub(crate) fn selection(&self) -> BTreeSet<String> {
        if self.canvas.multi.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            self.canvas.multi.clone()
        }
    }

    /// Selects `ids`: one shows its inspector, several the selection's tools.
    pub(crate) fn select(&mut self, ids: BTreeSet<String>) {
        self.canvas.flow = None;
        if ids.len() == 1 {
            self.selected = ids.into_iter().next();
            self.canvas.multi.clear();
        } else {
            self.selected = None;
            self.canvas.multi = ids;
        }
    }

    /// Keeps the selection in step with the model and with the block list's own selection.
    fn reconcile_selection(&mut self) {
        if self.selected.is_some() {
            self.canvas.multi.clear();
        }
        let model = self.engine.model();
        let exists = |id: &str| model.elements.iter().any(|e| e.id.as_str() == id);
        let kept: BTreeSet<String> = self
            .canvas
            .multi
            .iter()
            .filter(|id| exists(id))
            .cloned()
            .collect();
        let flow_gone = self
            .canvas
            .flow
            .as_ref()
            .is_some_and(|f| !model.connections.iter().any(|c| c.id.as_str() == f));
        if kept.len() != self.canvas.multi.len() {
            self.select(kept);
        }
        if flow_gone {
            self.canvas.flow = None;
        }
    }

    /// Moves shapes in one step. A model whose diagram is incomplete has every shape fixed where
    /// it is drawn now, so placing one does not move the others.
    fn place(&mut self, moved: &BTreeMap<String, (f64, f64)>) {
        let model = self.engine.model();
        let placed = model.placed();
        let mut shapes = Vec::new();
        for e in &model.elements {
            let id = e.id.as_str();
            match moved.get(id) {
                Some((x, y)) => shapes.push(json!({ "id": id, "x": x, "y": y })),
                None if !model.layout.contains_key(&e.id) && placed.contains_key(&e.id) => {
                    shapes.push(json!({ "id": id }));
                }
                None => {}
            }
        }
        if !shapes.is_empty() {
            self.execute("layout.set", json!({ "shapes": shapes }));
        }
    }

    /// The palette of kinds to drag onto the canvas.
    fn canvas_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::horizontal()
            .id_salt("palette")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::weak(ui, t("Drag to add:")));
                    for (kind, label) in KINDS {
                        let id = egui::Id::unique(("palette", *kind));
                        let r = ui
                            .dnd_drag_source(id, (*kind).to_owned(), |ui| {
                                ui.add(
                                    egui::Button::new(
                                        egui::RichText::new(crate::kind_text(kind)).size(12.0),
                                    )
                                    .sense(Sense::click_and_drag()),
                                )
                            })
                            .response;
                        let r = r.on_hover_text(crate::i18n::td(label));
                        if r.clicked() {
                            // Clicked rather than dragged: added after the selection, as from the list.
                            self.add_block(kind);
                        }
                    }
                })
            });
    }

    /// Zoom out, actual size, zoom in and fit, in the canvas's bottom-right corner.
    fn zoom_controls(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let percent = format!("{:.0}%", self.canvas.zoom * 100.0);
        let buttons: [(String, &str, f32); 4] = [
            ("−".to_owned(), t("Zoom out"), 28.0),
            (percent, t("Actual size"), 52.0),
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
                .put(r, egui::Button::new(egui::RichText::new(label).size(12.0)))
                .on_hover_text(hint)
                .clicked()
            {
                match i {
                    0 => self.zoom_by(1.0 / 1.25, Some(rect)),
                    1 => self.zoom_by(1.0 / self.canvas.zoom, Some(rect)),
                    2 => self.zoom_by(1.25, Some(rect)),
                    _ => self.canvas.fit = true,
                }
            }
        }
    }

    /// Zooms about the middle of the view.
    fn zoom_by(&mut self, factor: f32, view_rect: Option<Rect>) {
        let Some(rect) = view_rect else {
            return;
        };
        self.zoom_at(factor, rect.center() - rect.min);
    }

    /// Zooms keeping the point `at` (relative to the canvas's corner) where it is.
    fn zoom_at(&mut self, factor: f32, at: Vec2) {
        let c = &mut self.canvas;
        c.auto_fit = false;
        let z = (c.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let f = z / c.zoom;
        c.offset = at - (at - c.offset) * f;
        c.zoom = z;
    }

    /// Fits the drawing in `rect`, at most at its actual size.
    fn fit(&mut self, rect: Rect, placed: &BTreeMap<ElementId, Bounds>) {
        let mut it = placed.values();
        let Some(first) = it.next() else {
            return;
        };
        let (mut x0, mut y0) = (first.x, first.y);
        let (mut x1, mut y1) = (first.x + first.width, first.y + first.height);
        for b in it {
            x0 = x0.min(b.x);
            y0 = y0.min(b.y);
            x1 = x1.max(b.x + b.width);
            y1 = y1.max(b.y + b.height);
        }
        // Room for names under events and gateways.
        let margin = 40.0;
        let w = (x1 - x0) as f32 + 2.0 * margin;
        let h = (y1 - y0) as f32 + 2.0 * margin;
        let zoom = (rect.width() / w)
            .min(rect.height() / h)
            .clamp(MIN_ZOOM, 1.0);
        let size = Vec2::new((x1 - x0) as f32, (y1 - y0) as f32) * zoom;
        let c = &mut self.canvas;
        c.zoom = zoom;
        c.offset = (rect.size() - size) / 2.0 - Vec2::new(x0 as f32, y0 as f32) * zoom;
        c.fit = false;
    }

    /// The canvas: the toolbar, then the diagram filling the rest of `ui`.
    pub(crate) fn canvas(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        self.reconcile_selection();
        if let Some(text) = self.canvas.to_system.take() {
            ui.ctx().copy_text(text);
        }
        self.canvas_toolbar(ui);
        ui.add_space(4.0);

        let size = ui.available_size().max(Vec2::new(120.0, 120.0));
        let (rect, bg) = ui.allocate_exact_size(size, Sense::click_and_drag());
        bg.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, t("Diagram")));
        let model: Model = self.engine.model().clone();
        let placed = model.placed();
        let resized = self.canvas.auto_fit && self.canvas.fitted != Some(rect.size());
        if (self.canvas.fit || resized) && rect.width() > 1.0 {
            self.fit(rect, &placed);
            self.canvas.fitted = Some(rect.size());
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
                let at = ptr.unwrap_or(rect.center()) - rect.min;
                self.zoom_at(zoom, at);
            } else if !ctrl && scroll != Vec2::ZERO {
                self.canvas.offset += scroll;
                self.canvas.auto_fit = false;
            }
        }
        if bg.dragged_by(egui::PointerButton::Middle)
            || bg.dragged_by(egui::PointerButton::Secondary)
        {
            self.canvas.offset += bg.drag_delta();
            self.canvas.auto_fit = false;
        }

        let view = View {
            origin: rect.min + self.canvas.offset,
            zoom: self.canvas.zoom,
        };
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, p.canvas);
        painter.rect_stroke(rect, 6.0, Stroke::new(1.0, p.border), StrokeKind::Inside);
        // A dot every 50 units, when they are far enough apart to help.
        let step = 50.0 * view.zoom;
        if step >= 14.0 {
            let (sx, sy) = (
                rect.left() + (view.origin.x - rect.left()).rem_euclid(step),
                rect.top() + (view.origin.y - rect.top()).rem_euclid(step),
            );
            let dot = p.border;
            let mut y = sy;
            while y < rect.bottom() {
                let mut x = sx;
                while x < rect.right() {
                    painter.circle_filled(Pos2::new(x, y), 1.0, dot);
                    x += step;
                }
                y += step;
            }
        }

        let problems = self.problems();
        let errors: BTreeSet<String> = problems
            .iter()
            .filter(|pr| pr.error)
            .filter_map(|pr| pr.at.clone())
            .collect();
        let warnings: BTreeSet<String> = problems
            .iter()
            .filter(|pr| !pr.error)
            .filter_map(|pr| pr.at.clone())
            .collect();
        let selection = self.selection();
        let moving: (BTreeSet<String>, Vec2) = match &self.canvas.drag {
            Drag::Move { ids, by } => (ids.iter().cloned().collect(), *by),
            _ => (BTreeSet::new(), Vec2::ZERO),
        };
        let rect_of = |id: &ElementId| -> Option<Rect> {
            let b = placed.get(id)?;
            let r = view.rect(b);
            Some(if moving.0.contains(id.as_str()) {
                r.translate(moving.1 * view.zoom)
            } else {
                r
            })
        };

        // Connections under the shapes.
        let mut edges: Vec<(String, Pos2, Pos2)> = Vec::new();
        for c in &model.connections {
            let (Some(a), Some(b)) = (rect_of(&c.from), rect_of(&c.to)) else {
                continue;
            };
            let (from, to) = edge_ends(&a, &b);
            let on = self.canvas.flow.as_deref() == Some(c.id.as_str());
            let stroke = if on {
                Stroke::new(2.5, p.accent)
            } else {
                Stroke::new(1.4, p.text_weak)
            };
            arrow(&painter, from, to, stroke, view.zoom);
            // How a decision splits: the chance, or that it has a condition.
            let note = match (&c.probability, &c.condition) {
                (Some(pr), _) => Some(format!("{:.0}%", pr * 100.0)),
                (None, Some(_)) => Some("?".to_owned()),
                _ => None,
            };
            if let Some(n) = note {
                let mid = from + (to - from) * 0.5;
                painter.text(
                    mid + Vec2::new(4.0, -4.0),
                    Align2::LEFT_BOTTOM,
                    n,
                    FontId::proportional((11.0 * view.zoom).clamp(8.0, 16.0)),
                    p.text_weak,
                );
            }
            edges.push((c.id.as_str().to_owned(), from, to));
        }

        // The shapes, each its own widget.
        let font = FontId::proportional((13.0 * view.zoom).clamp(7.0, 22.0));
        let small = FontId::proportional((10.5 * view.zoom).clamp(6.0, 18.0));
        let pointer = ui.input(|i| i.pointer.hover_pos());
        let mut shape_rects: Vec<(String, Rect)> = Vec::new();
        let mut clicked: Option<(String, bool)> = None;
        let mut drag_start: Option<String> = None;
        let mut dragging = Vec2::ZERO;
        let mut drag_end = false;
        for e in &model.elements {
            let Some(r) = rect_of(&e.id) else { continue };
            let id = e.id.as_str().to_owned();
            let name = if e.name.is_empty() { &id } else { &e.name };
            let on = selection.contains(&id);
            let colour = if errors.contains(&id) {
                p.error
            } else if warnings.contains(&id) {
                p.warning
            } else {
                p.text
            };
            let stroke = Stroke::new(
                if on { 2.5 } else { 1.5 },
                if on { p.accent } else { colour },
            );
            let fill = if on { p.accent_soft } else { p.raised };
            let label_below = |painter: &egui::Painter| {
                let g = painter.layout(
                    name.clone(),
                    font.clone(),
                    p.text,
                    (110.0 * view.zoom).max(40.0),
                );
                let at = Pos2::new(
                    r.center().x - g.size().x / 2.0,
                    r.bottom() + 4.0 * view.zoom,
                );
                painter.galley(at, g, p.text);
            };
            match &e.kind {
                ElementKind::Source { .. } | ElementKind::Sink => {
                    let radius = r.width().min(r.height()) / 2.0;
                    let width = if matches!(e.kind, ElementKind::Sink) {
                        // An end event's thick ring.
                        stroke.width + 2.0
                    } else {
                        stroke.width
                    };
                    painter.circle(r.center(), radius, fill, Stroke::new(width, stroke.color));
                    label_below(&painter);
                }
                ElementKind::ExclusiveGateway { .. } | ElementKind::ParallelGateway => {
                    let c = r.center();
                    let pts = vec![
                        Pos2::new(c.x, r.top()),
                        Pos2::new(r.right(), c.y),
                        Pos2::new(c.x, r.bottom()),
                        Pos2::new(r.left(), c.y),
                    ];
                    painter.add(Shape::convex_polygon(pts, fill, stroke));
                    let mark = r.width().min(r.height()) * 0.2;
                    let s = Stroke::new(2.0 * view.zoom.clamp(0.5, 1.5), stroke.color);
                    if matches!(e.kind, ElementKind::ParallelGateway) {
                        painter.line_segment([c - Vec2::X * mark, c + Vec2::X * mark], s);
                        painter.line_segment([c - Vec2::Y * mark, c + Vec2::Y * mark], s);
                    } else {
                        let d = Vec2::splat(mark * 0.75);
                        painter.line_segment([c - d, c + d], s);
                        painter
                            .line_segment([c + Vec2::new(-d.x, d.y), c + Vec2::new(d.x, -d.y)], s);
                    }
                    label_below(&painter);
                }
                _ => {
                    let corner = 10.0 * view.zoom;
                    painter.rect_filled(r, corner, fill);
                    painter.rect_stroke(r, corner, stroke, StrokeKind::Inside);
                    let g = painter.layout(
                        name.clone(),
                        font.clone(),
                        p.text,
                        (r.width() - 10.0 * view.zoom).max(10.0),
                    );
                    let at = Pos2::new(
                        r.center().x - g.size().x / 2.0,
                        r.center().y - g.size().y / 2.0 - 4.0 * view.zoom,
                    );
                    painter.galley(at, g, p.text);
                    // The kind, small, along the bottom.
                    let kind = match &e.kind {
                        ElementKind::Server { capacity, .. } if *capacity > 1 => {
                            format!("{} ×{capacity}", crate::kind_text(kind_key(&e.kind)))
                        }
                        k => crate::kind_text(kind_key(k)).to_owned(),
                    };
                    painter.text(
                        Pos2::new(r.center().x, r.bottom() - 5.0 * view.zoom),
                        Align2::CENTER_BOTTOM,
                        kind,
                        small.clone(),
                        p.text_weak,
                    );
                }
            }
            // Only the part inside the canvas takes the pointer, not one under the inspector.
            let hit = r.intersect(rect);
            let resp = ui.interact(
                hit,
                ui.scope_id().with(("shape", &id)),
                if hit.is_positive() {
                    Sense::click_and_drag()
                } else {
                    Sense::hover()
                },
            );
            let label = tf(
                "{kind} {name}",
                &[
                    ("kind", crate::kind_text(kind_key(&e.kind))),
                    ("name", name),
                ],
            );
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &label));
            if resp.clicked() {
                let add = ui.input(|i| i.modifiers.shift || i.modifiers.command);
                clicked = Some((id.clone(), add));
            }
            if resp.drag_started_by(egui::PointerButton::Primary) {
                drag_start = Some(id.clone());
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                dragging += resp.drag_delta();
            }
            if resp.drag_stopped() {
                drag_end = true;
            }
            shape_rects.push((id, r));
        }

        // The connect handle: on the right of the block under the pointer (or the selected one),
        // except where nothing can leave (departures).
        let near = pointer.and_then(|pt| {
            shape_rects
                .iter()
                .rev()
                .find(|(_, r)| r.expand(14.0).contains(pt))
                .cloned()
        });
        let connecting = match &self.canvas.drag {
            Drag::Connect { from, .. } => shape_rects.iter().find(|(id, _)| id == from).cloned(),
            _ => None,
        };
        let handle_for = connecting.or(near).or_else(|| {
            self.selected
                .as_ref()
                .and_then(|s| shape_rects.iter().find(|(id, _)| id == s).cloned())
        });
        let mut connect_end: Option<Pos2> = None;
        if let Some((hid, hr)) = handle_for
            && model
                .elements
                .iter()
                .any(|e| e.id.as_str() == hid && !matches!(e.kind, ElementKind::Sink))
            && matches!(self.canvas.drag, Drag::None | Drag::Connect { .. })
        {
            let at = hr.right_center() + Vec2::new(10.0, 0.0);
            let handle = Rect::from_center_size(at, Vec2::splat(14.0));
            let hit = handle.intersect(rect);
            let hresp = ui.interact(
                hit,
                ui.scope_id().with(("handle", &hid)),
                if hit.is_positive() {
                    Sense::drag()
                } else {
                    Sense::hover()
                },
            );
            hresp.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    true,
                    tf("Connect from {name}", &[("name", &hid)]),
                )
            });
            let hot = hresp.hovered() || hresp.dragged();
            painter.circle(
                at,
                if hot { 6.5 } else { 5.0 },
                if hot { p.accent } else { p.raised },
                Stroke::new(1.5, p.accent),
            );
            if hresp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }
            if hresp.drag_started() {
                self.canvas.drag = Drag::Connect {
                    from: hid.clone(),
                    to: at,
                };
            }
            if hresp.dragged()
                && let Drag::Connect { to, .. } = &mut self.canvas.drag
                && let Some(pt) = pointer
            {
                *to = pt;
            }
            if hresp.drag_stopped() {
                connect_end = pointer.or(match &self.canvas.drag {
                    Drag::Connect { to, .. } => Some(*to),
                    _ => None,
                });
            }
        }
        if let Drag::Connect { from, to } = &self.canvas.drag
            && let Some((_, fr)) = shape_rects.iter().find(|(id, _)| id == from)
        {
            arrow(
                &painter,
                fr.right_center(),
                *to,
                Stroke::new(1.5, p.accent),
                view.zoom,
            );
        }
        if let Some(end) = connect_end {
            if let Drag::Connect { from, .. } = std::mem::take(&mut self.canvas.drag)
                && let Some((target, _)) = shape_rects
                    .iter()
                    .rev()
                    .find(|(id, r)| *id != from && r.expand(4.0).contains(end))
                && let Some(r) = self.execute("connect", json!({ "from": from, "to": target }))
            {
                let flow = r["id"].as_str().unwrap_or("").to_owned();
                self.info(tf(
                    "Connected {from} to {to}",
                    &[("from", &from), ("to", target)],
                ));
                self.canvas.flow = Some(flow);
            }
            self.canvas.drag = Drag::None;
        }

        // Clicks and drags on shapes.
        if let Some((id, add)) = clicked {
            let mut sel = if add {
                self.selection()
            } else {
                BTreeSet::new()
            };
            if add && sel.contains(&id) {
                sel.remove(&id);
            } else {
                sel.insert(id);
            }
            self.select(sel);
        }
        if let Some(id) = drag_start {
            let mut sel = self.selection();
            if !sel.contains(&id) {
                sel = BTreeSet::from([id]);
                self.select(sel.clone());
            }
            self.canvas.drag = Drag::Move {
                ids: sel.into_iter().collect(),
                by: Vec2::ZERO,
            };
        }
        if let Drag::Move { by, .. } = &mut self.canvas.drag {
            *by += dragging / view.zoom;
        }
        if drag_end
            && let Drag::Move { ids, by } = std::mem::take(&mut self.canvas.drag)
            && by.length() * view.zoom >= 1.0
        {
            let moved: BTreeMap<String, (f64, f64)> = ids
                .iter()
                .filter_map(|id| {
                    let b = placed.get(&ElementId::unchecked(id))?;
                    Some((
                        id.clone(),
                        (snap(b.x + f64::from(by.x)), snap(b.y + f64::from(by.y))),
                    ))
                })
                .collect();
            self.place(&moved);
        }

        // The background: click clears the selection (or picks a connection), drag selects.
        let shift = ui.input(|i| i.modifiers.shift || i.modifiers.command);
        if bg.clicked() {
            let at = bg.interact_pointer_pos();
            let flow = at.and_then(|pt| {
                edges
                    .iter()
                    .map(|(id, a, b)| (id, distance_to_segment(pt, *a, *b)))
                    .filter(|(_, d)| *d <= 6.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(id, _)| id.clone())
            });
            if !shift {
                self.select(BTreeSet::new());
            }
            self.canvas.flow = flow;
        }
        if bg.drag_started_by(egui::PointerButton::Primary)
            && let Some(from) = bg.interact_pointer_pos()
        {
            self.canvas.drag = Drag::Marquee {
                from,
                to: from,
                add: shift,
            };
        }
        if let Drag::Marquee { to, .. } = &mut self.canvas.drag
            && let Some(pt) = pointer
        {
            *to = pt;
        }
        if let Drag::Marquee { from, to, .. } = &self.canvas.drag {
            let r = Rect::from_two_pos(*from, *to);
            painter.rect_filled(r, 2.0, p.accent_soft.gamma_multiply(0.5));
            painter.rect_stroke(r, 2.0, Stroke::new(1.0, p.accent), StrokeKind::Inside);
        }
        if bg.drag_stopped()
            && let Drag::Marquee { from, to, add } = std::mem::take(&mut self.canvas.drag)
        {
            let r = Rect::from_two_pos(from, to);
            let mut sel = if add {
                self.selection()
            } else {
                BTreeSet::new()
            };
            sel.extend(
                shape_rects
                    .iter()
                    .filter(|(_, s)| r.intersects(*s))
                    .map(|(id, _)| id.clone()),
            );
            self.select(sel);
        }
        // A drag that ended outside any widget.
        if !ui.input(|i| i.pointer.any_down())
            && !matches!(self.canvas.drag, Drag::None)
            && !(drag_end || bg.drag_stopped())
            && connect_end.is_none()
        {
            self.canvas.drag = Drag::None;
        }

        // A kind dropped from the palette: a new block there, centred on the drop.
        if let Some(kind) = bg.dnd_release_payload::<String>()
            && let Some(pt) = pointer.or(bg.interact_pointer_pos())
        {
            self.add_block_at(&kind, view.diagram(pt));
        }

        self.zoom_controls(ui, rect);

        // Clipboard events (the system's copy, cut and paste shortcuts arrive as these).
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing && ui.rect_contains_pointer(rect) || !typing && !self.selection().is_empty() {
            let events = ui.input(|i| i.events.clone());
            for ev in events {
                match ev {
                    egui::Event::Copy => self.copy(),
                    egui::Event::Cut => self.cut(),
                    egui::Event::Paste(text) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&text)
                            && v.get("stochcraft").is_some()
                        {
                            self.canvas.clipboard = Some(v);
                        }
                        self.paste();
                    }
                    _ => {}
                }
            }
        }
    }

    /// Adds a block of `kind` with its shape centred on `at` (diagram units).
    pub(crate) fn add_block_at(&mut self, kind: &str, at: (f64, f64)) {
        let mut params = crate::editor::new_block(kind);
        if kind == "shared_task" {
            let Some(pool) = self.first_pool() else {
                return;
            };
            params["needs"] = json!([{ "resource": pool, "quantity": 1 }]);
        }
        let (w, h) = match kind {
            "source" | "sink" => (36.0, 36.0),
            "exclusive_gateway" | "parallel_gateway" => (50.0, 50.0),
            _ => (100.0, 80.0),
        };
        // Fix the shapes drawn automatically first, so adding one does not move them.
        self.place(&BTreeMap::new());
        params["x"] = json!(snap(at.0 - w / 2.0));
        params["y"] = json!(snap(at.1 - h / 2.0));
        if let Some(r) = self.execute("block.add", params)
            && let Some(id) = r["id"].as_str()
        {
            self.info(tf(
                "Added {block} ({id})",
                &[("block", crate::kind_text(kind)), ("id", id)],
            ));
            self.select(BTreeSet::from([id.to_owned()]));
        }
    }

    /// The selected blocks and the connections between them, as the clipboard holds them.
    fn copied(&self) -> Option<Value> {
        let sel = self.selection();
        if sel.is_empty() {
            return None;
        }
        let model = self.engine.model();
        let placed = model.placed();
        let doc = stochcraft_engine::wire::model(model);
        let elements: Vec<Value> = doc["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["id"].as_str().is_some_and(|id| sel.contains(id)))
            .map(|e| {
                let mut e = e.clone();
                let id = ElementId::unchecked(e["id"].as_str().unwrap_or(""));
                if let Some(b) = placed.get(&id) {
                    e["bounds"] = json!({"x": b.x, "y": b.y, "width": b.width, "height": b.height});
                }
                e
            })
            .collect();
        let connections: Vec<Value> = doc["connections"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| {
                let inside = |k: &str| c[k].as_str().is_some_and(|id| sel.contains(id));
                inside("from") && inside("to")
            })
            .cloned()
            .collect();
        Some(json!({ "stochcraft": 1, "elements": elements, "connections": connections }))
    }

    /// Copies the selection, to the app's clipboard and as text to the system's.
    pub(crate) fn copy(&mut self) {
        if let Some(v) = self.copied() {
            self.canvas.to_system = Some(v.to_string());
            let n = v["elements"].as_array().map_or(0, Vec::len);
            self.canvas.clipboard = Some(v);
            self.canvas.pastes = 0;
            self.info(tnf(n as u64, "Copied {n} block", "Copied {n} blocks", &[]));
        }
    }

    pub(crate) fn cut(&mut self) {
        self.copy();
        if self.canvas.clipboard.is_some() {
            self.delete_selection();
            // A paste after a cut goes where the blocks were.
            self.canvas.pastes = u32::MAX;
        }
    }

    pub(crate) fn can_paste(&self) -> bool {
        self.canvas.clipboard.is_some()
    }

    pub(crate) fn paste(&mut self) {
        let Some(clip) = self.canvas.clipboard.clone() else {
            return;
        };
        let step = if self.canvas.pastes == u32::MAX {
            self.canvas.pastes = 0;
            0
        } else {
            self.canvas.pastes = self.canvas.pastes.saturating_add(1);
            self.canvas.pastes
        };
        self.insert(&clip, PASTE_STEP * f64::from(step), "paste");
    }

    pub(crate) fn duplicate(&mut self) {
        if let Some(v) = self.copied() {
            self.insert(&v, PASTE_STEP, "duplicate");
        }
    }

    /// Adds clipboard blocks (and their connections) under new ids, moved by `by`, in one step,
    /// and selects them.
    fn insert(&mut self, clip: &Value, by: f64, label: &str) {
        let model = self.engine.model();
        let mut taken: BTreeSet<String> = model
            .elements
            .iter()
            .map(|e| e.id.as_str().to_owned())
            .chain(model.connections.iter().map(|c| c.id.as_str().to_owned()))
            .collect();
        let incomplete = model
            .elements
            .iter()
            .any(|e| !model.layout.contains_key(&e.id));
        let mut commands = Vec::new();
        if incomplete {
            // Fix the shapes drawn automatically, so the new ones do not move them.
            let shapes: Vec<Value> = model
                .elements
                .iter()
                .filter(|e| !model.layout.contains_key(&e.id))
                .map(|e| json!({ "id": e.id.as_str() }))
                .collect();
            if !shapes.is_empty() {
                commands.push(json!({ "command": "layout.set", "params": { "shapes": shapes } }));
            }
        }
        let mut ids: BTreeMap<String, String> = BTreeMap::new();
        let mut defaults: Vec<(String, String)> = Vec::new();
        for e in clip["elements"].as_array().into_iter().flatten() {
            let Some(old) = e["id"].as_str() else {
                continue;
            };
            let new = fresh_id(old, &taken);
            taken.insert(new.clone());
            ids.insert(old.to_owned(), new.clone());
            let mut params = e.clone();
            params["id"] = json!(new);
            if let Some(obj) = params.as_object_mut() {
                obj.remove("bounds");
                if let Some(d) = obj
                    .remove("default")
                    .and_then(|d| d.as_str().map(str::to_owned))
                {
                    defaults.push((new.clone(), d));
                }
            }
            if let Some(b) = e.get("bounds") {
                params["x"] = json!(b["x"].as_f64().unwrap_or(0.0) + by);
                params["y"] = json!(b["y"].as_f64().unwrap_or(0.0) + by);
            }
            commands.push(json!({ "command": "block.add", "params": params }));
        }
        let mut flows: BTreeMap<String, String> = BTreeMap::new();
        for c in clip["connections"].as_array().into_iter().flatten() {
            let (Some(from), Some(to), Some(old)) = (
                c["from"].as_str().and_then(|f| ids.get(f)),
                c["to"].as_str().and_then(|t| ids.get(t)),
                c["id"].as_str(),
            ) else {
                continue;
            };
            let new = fresh_id(old, &taken);
            taken.insert(new.clone());
            flows.insert(old.to_owned(), new.clone());
            let mut params = json!({ "id": new, "from": from, "to": to });
            for k in ["probability", "condition"] {
                if !c[k].is_null() {
                    params[k] = c[k].clone();
                }
            }
            commands.push(json!({ "command": "connect", "params": params }));
        }
        for (gateway, d) in defaults {
            if let Some(f) = flows.get(&d) {
                commands.push(json!({
                    "command": "block.set",
                    "params": { "id": gateway, "kind": "exclusive_gateway", "default": f }
                }));
            }
        }
        if ids.is_empty() {
            return;
        }
        if self
            .execute("batch", json!({ "label": label, "commands": commands }))
            .is_some()
        {
            let n = ids.len();
            self.select(ids.into_values().collect());
            self.info(tnf(n as u64, "Added {n} block", "Added {n} blocks", &[]));
        }
    }

    /// Removes the selected blocks (with their connections), or the selected connection.
    pub(crate) fn delete_selection(&mut self) {
        if let Some(f) = self.canvas.flow.take() {
            if self.execute("disconnect", json!({ "id": f })).is_some() {
                self.info(tf("Removed {id}", &[("id", &f)]));
            }
            return;
        }
        let sel = self.selection();
        match sel.len() {
            0 => {}
            1 => self.remove_selected(),
            n => {
                let commands: Vec<Value> = sel
                    .iter()
                    .map(|id| json!({ "command": "block.remove", "params": { "id": id } }))
                    .collect();
                if self
                    .execute("batch", json!({ "label": "delete", "commands": commands }))
                    .is_some()
                {
                    self.select(BTreeSet::new());
                    self.info(tnf(
                        n as u64,
                        "Removed {n} block",
                        "Removed {n} blocks",
                        &[],
                    ));
                }
            }
        }
    }

    pub(crate) fn select_all(&mut self) {
        let all = self
            .engine
            .model()
            .elements
            .iter()
            .map(|e| e.id.as_str().to_owned())
            .collect();
        self.select(all);
    }

    /// Lines up or spreads out the selected blocks, in one step.
    pub(crate) fn align(&mut self, how: Align) {
        let sel = self.selection();
        let placed = self.engine.model().placed();
        let shapes: Vec<(String, Bounds)> = sel
            .iter()
            .filter_map(|id| Some((id.clone(), *placed.get(&ElementId::unchecked(id))?)))
            .collect();
        if shapes.len() < 2 {
            return;
        }
        let fold = |f: fn(&Bounds) -> f64, min: bool| {
            shapes.iter().map(|(_, b)| f(b)).fold(
                if min {
                    f64::INFINITY
                } else {
                    f64::NEG_INFINITY
                },
                if min { f64::min } else { f64::max },
            )
        };
        let left = fold(|b| b.x, true);
        let right = fold(|b| b.x + b.width, false);
        let top = fold(|b| b.y, true);
        let bottom = fold(|b| b.y + b.height, false);
        let mut moved: BTreeMap<String, (f64, f64)> = BTreeMap::new();
        match how {
            Align::Left
            | Align::Centre
            | Align::Right
            | Align::Top
            | Align::Middle
            | Align::Bottom => {
                for (id, b) in &shapes {
                    let (x, y) = match how {
                        Align::Left => (left, b.y),
                        Align::Right => (right - b.width, b.y),
                        Align::Centre => ((left + right) / 2.0 - b.width / 2.0, b.y),
                        Align::Top => (b.x, top),
                        Align::Bottom => (b.x, bottom - b.height),
                        _ => (b.x, (top + bottom) / 2.0 - b.height / 2.0),
                    };
                    moved.insert(id.clone(), (x, y));
                }
            }
            Align::SpreadAcross | Align::SpreadDown => {
                let across = how == Align::SpreadAcross;
                let mut order = shapes.clone();
                order.sort_by(|a, b| {
                    let k = |s: &Bounds| if across { s.centre().0 } else { s.centre().1 };
                    k(&a.1).total_cmp(&k(&b.1))
                });
                let (first, last) = match (order.first(), order.last()) {
                    (Some(f), Some(l)) => (f.1.centre(), l.1.centre()),
                    _ => return,
                };
                let n = (order.len() - 1) as f64;
                for (i, (id, b)) in order.iter().enumerate() {
                    let f = i as f64 / n;
                    let (cx, cy) = if across {
                        (first.0 + (last.0 - first.0) * f, b.centre().1)
                    } else {
                        (b.centre().0, first.1 + (last.1 - first.1) * f)
                    };
                    moved.insert(id.clone(), (cx - b.width / 2.0, cy - b.height / 2.0));
                }
            }
        }
        let shapes: Vec<Value> = moved
            .iter()
            .map(|(id, (x, y))| json!({ "id": id, "x": x, "y": y }))
            .collect();
        self.execute(
            "batch",
            json!({ "label": "align", "commands": [
                { "command": "layout.set", "params": { "shapes": shapes } }
            ]}),
        );
    }

    /// The inspector when several blocks are selected: how many, and what can be done to them.
    pub(crate) fn multi_inspector(&mut self, ui: &mut egui::Ui) {
        let n = self.canvas.multi.len();
        ui.heading(tnf(
            n as u64,
            "{n} block selected",
            "{n} blocks selected",
            &[],
        ));
        ui.add_space(8.0);
        crate::editor::section(ui, t("Align"));
        let mut pick = None;
        ui.horizontal_wrapped(|ui| {
            for (how, label) in [
                (Align::Left, "Left edges"),
                (Align::Centre, "Centres"),
                (Align::Right, "Right edges"),
                (Align::Top, "Tops"),
                (Align::Middle, "Middles"),
                (Align::Bottom, "Bottoms"),
            ] {
                if ui.add(theme::secondary(ui, t(label))).clicked() {
                    pick = Some(how);
                }
            }
        });
        crate::editor::section(ui, t("Spread out"));
        ui.horizontal_wrapped(|ui| {
            for (how, label) in [
                (Align::SpreadAcross, "Evenly across"),
                (Align::SpreadDown, "Evenly down"),
            ] {
                if ui.add(theme::secondary(ui, t(label))).clicked() {
                    pick = Some(how);
                }
            }
        });
        if let Some(how) = pick {
            self.align(how);
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add(theme::secondary(ui, t("Duplicate"))).clicked() {
                self.duplicate();
            }
            if ui.add(theme::secondary(ui, t("Delete"))).clicked() {
                self.delete_selection();
            }
        });
    }
}

/// How to line up the selected blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Centre,
    Right,
    Top,
    Middle,
    Bottom,
    /// Centres evenly spaced from left to right (the outermost stay).
    SpreadAcross,
    /// Centres evenly spaced from top to bottom.
    SpreadDown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_get_ids_of_their_own() {
        let taken: BTreeSet<String> = ["desk", "desk-copy"].map(str::to_owned).into();
        assert_eq!(fresh_id("desk", &taken), "desk-copy-2");
        assert_eq!(fresh_id("exit", &taken), "exit-copy");
    }

    #[test]
    fn edges_leave_the_facing_side() {
        let a = Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0));
        let right = a.translate(Vec2::new(30.0, 5.0));
        assert_eq!(
            edge_ends(&a, &right),
            (a.right_center(), right.left_center())
        );
        let below = a.translate(Vec2::new(2.0, 30.0));
        assert_eq!(
            edge_ends(&a, &below),
            (a.center_bottom(), below.center_top())
        );
        assert!(distance_to_segment(Pos2::new(5.0, 3.0), Pos2::ZERO, Pos2::new(10.0, 0.0)) == 3.0);
    }
}
