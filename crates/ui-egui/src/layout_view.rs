//! The Layout view (M5.5, L1): the model's plan in metres, to scale, read-only. A grid, a scale
//! bar, every conveyor as its [`Path`] with its width, every other block as its footprint with
//! its name, and the list of blocks that are not placed.
//!
//! There are no layout files yet, so every model is "without a layout": positions come from the
//! flow diagram, as in the 3D view (block centres on the canvas, [`view3d::centre_metres`]), and
//! the view says so. A conveyor starts at the edge of the block before it and runs along its own
//! path, in the direction of the block after it, for its own length.
//!
//! Conventions: metres, `x` right and `y` up, like the 3D view's floor. Drag to pan, scroll to
//! zoom about the pointer, Fit to see everything; the arrow keys and plus and minus work while
//! the drawing has focus.

use egui::{
    Align2, FontId, Key, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2,
};
use stochcraft_model::{ElementId, ElementKind, Frame, Model, Segment};
use stochcraft_render::StraightConveyor;

use crate::App;
use crate::i18n::{t, tf};
use crate::results::number;
use crate::theme;
use crate::view3d::{Outline, block_outline, centre_metres, neighbour, outline_of, unit_metres};

/// The least and most pixels a metre may take.
const MIN_SCALE: f32 = 0.05;
const MAX_SCALE: f32 = 2000.0;
/// The closest the grid lines may be, in pixels; the grid coarsens beyond it.
const MIN_GRID_PX: f32 = 12.0;
/// About how long the scale bar is, in pixels, at least.
const BAR_PX: f32 = 80.0;
/// The most grid lines drawn each way.
const MOST_LINES: usize = 400;
/// The space round the drawing when it is fitted, in pixels.
const FIT_PAD: f32 = 40.0;
/// How far a curved path is cut for drawing: about this many metres between points (the
/// geometry itself stays exact, the points only draw it).
const DRAW_STEP: f64 = 0.1;

/// The distances a grid or a scale bar may take, in metres.
const NICE: [f64; 18] = [
    0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0,
    2000.0, 5000.0,
];

/// The first of [`NICE`] that takes at least `least_px` pixels at `scale` pixels a metre.
fn nice_step(scale: f32, least_px: f32) -> f64 {
    NICE.into_iter()
        .find(|s| s * f64::from(scale) >= f64::from(least_px))
        .unwrap_or(10_000.0)
}

/// A block (not a conveyor) as it is drawn: its name, its centre and the outline of its footprint.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Place {
    name: String,
    centre: [f32; 2],
    outline: Outline,
}

/// A conveyor as it is drawn: its name, its path cut into points (in metres), and the point
/// half way along, where its name goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Belt {
    name: String,
    points: Vec<[f32; 2]>,
    middle: [f32; 2],
}

/// What the Layout view draws, worked out from the model once per revision.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Plan {
    places: Vec<Place>,
    belts: Vec<Belt>,
    /// The names of the blocks that have no position: listed, not drawn.
    unplaced: Vec<String>,
    /// The corners of the box round everything drawn.
    low: [f32; 2],
    high: [f32; 2],
}

/// What a block is called on the drawing: its name, or its id when it has none.
fn label_of(name: &str, id: &ElementId) -> String {
    if name.is_empty() {
        id.as_str().to_owned()
    } else {
        name.to_owned()
    }
}

/// The plan of `model`. Positions are the diagram's ([`Model::placed`]); a block that has none is
/// unplaced.
fn build_plan(model: &Model) -> Plan {
    let placed = model.placed();
    let metres = unit_metres(model.experiment.length_unit);
    let centre_of = |id: &ElementId| placed.get(id).map(centre_metres);
    let mut places = Vec::new();
    let mut belts = Vec::new();
    let mut unplaced = Vec::new();
    for e in &model.elements {
        let name = label_of(&e.name, &e.id);
        let Some(c) = centre_of(&e.id) else {
            unplaced.push(name);
            continue;
        };
        let ElementKind::Conveyor { length, shape, .. } = &e.kind else {
            places.push(Place {
                name,
                centre: c,
                outline: block_outline(&e.kind),
            });
            continue;
        };
        // From the edge of the block before, towards the block after; with a neighbour on one
        // side only, or none, from the conveyor's own centre along `+x`.
        let before = neighbour(model, &e.id, true).and_then(|f| Some((f, centre_of(f)?)));
        let after = neighbour(model, &e.id, false).and_then(centre_of);
        let (start, heading) = match (before, after) {
            (Some((from, a)), Some(b)) if libm::hypotf(b[0] - a[0], b[1] - a[1]) > 1e-6 => {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let d = libm::hypotf(dx, dy);
                let unit = [dx / d, dy / d];
                let out = outline_of(model, from).leave(a, a, unit);
                (
                    [a[0] + unit[0] * out, a[1] + unit[1] * out],
                    libm::atan2f(dy, dx).to_degrees(),
                )
            }
            _ => (c, 0.0),
        };
        let mut path = shape.path(*length).scaled(f64::from(metres));
        path.start = Frame::new(
            f64::from(start[0]),
            f64::from(start[1]),
            0.0,
            f64::from(heading),
        );
        let total = path.length();
        let curved = path
            .segments
            .iter()
            .any(|s| !matches!(s, Segment::Line { .. }));
        let n = if curved && total.is_finite() && total > 0.0 {
            ((total / DRAW_STEP).ceil() as usize).clamp(2, 400)
        } else {
            2
        };
        let at = |s: f64| {
            let f = path.point_at(s);
            [f.x as f32, f.y as f32]
        };
        let points = (0..=n).map(|i| at(total * i as f64 / n as f64)).collect();
        belts.push(Belt {
            name,
            points,
            middle: at(total / 2.0),
        });
    }
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    let half_belt = belt_width() / 2.0;
    let mut grow = |p: [f32; 2], by: f32| {
        if p.iter().all(|c| c.is_finite()) {
            low = [low[0].min(p[0] - by), low[1].min(p[1] - by)];
            high = [high[0].max(p[0] + by), high[1].max(p[1] + by)];
        }
    };
    for p in &places {
        grow(p.centre, p.outline.reach());
    }
    for b in &belts {
        for q in &b.points {
            grow(*q, half_belt);
        }
    }
    if low[0] > high[0] {
        (low, high) = ([-2.0, -2.0], [2.0, 2.0]);
    }
    Plan {
        places,
        belts,
        unplaced,
        low,
        high,
    }
}

/// How wide a belt is drawn, in metres: the 3D view's.
fn belt_width() -> f32 {
    StraightConveyor::default().width
}

/// Where the drawing is looking: the point in metres at the middle of the canvas and how many
/// pixels a metre takes.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Vw {
    centre: [f32; 2],
    scale: f32,
}

impl Vw {
    /// The view that shows the box from `low` to `high` whole in a canvas of `size`.
    fn fitted(low: [f32; 2], high: [f32; 2], size: Vec2) -> Self {
        let span = [(high[0] - low[0]).max(1.0), (high[1] - low[1]).max(1.0)];
        let room = [
            (size.x - 2.0 * FIT_PAD).max(1.0),
            (size.y - 2.0 * FIT_PAD).max(1.0),
        ];
        Vw {
            centre: [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0],
            scale: (room[0] / span[0])
                .min(room[1] / span[1])
                .clamp(MIN_SCALE, MAX_SCALE),
        }
    }

    /// Where the point `w` (metres) falls on the canvas `rect`; `y` grows up in metres and down
    /// on the screen.
    fn on_screen(&self, rect: Rect, w: [f32; 2]) -> Pos2 {
        let c = rect.center();
        pos2(
            c.x + (w[0] - self.centre[0]) * self.scale,
            c.y - (w[1] - self.centre[1]) * self.scale,
        )
    }

    /// The point in metres under the screen position `p`.
    fn in_metres(&self, rect: Rect, p: Pos2) -> [f32; 2] {
        let c = rect.center();
        [
            self.centre[0] + (p.x - c.x) / self.scale,
            self.centre[1] - (p.y - c.y) / self.scale,
        ]
    }

    /// Zooms by `factor` (above 1 to see more closely) keeping the point under `at` where it is.
    fn zoom_at(&mut self, rect: Rect, at: Pos2, factor: f32) {
        let w = self.in_metres(rect, at);
        self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let c = rect.center();
        self.centre = [
            w[0] - (at.x - c.x) / self.scale,
            w[1] + (at.y - c.y) / self.scale,
        ];
    }

    /// Moves the drawing by `d` pixels.
    fn pan(&mut self, d: Vec2) {
        self.centre[0] -= d.x / self.scale;
        self.centre[1] += d.y / self.scale;
    }
}

/// The screen `x` of every vertical grid line and the screen `y` of every horizontal one in
/// `rect`, a metre apart or wider when that would crowd them.
fn grid_lines(vw: &Vw, rect: Rect) -> (Vec<f32>, Vec<f32>) {
    let step = nice_step(vw.scale, MIN_GRID_PX);
    let low = vw.in_metres(rect, rect.left_bottom());
    let high = vw.in_metres(rect, rect.right_top());
    let lines = |a: f32, b: f32, at: &dyn Fn(f32) -> f32| -> Vec<f32> {
        let first = (f64::from(a) / step).ceil() as i64;
        let last = (f64::from(b) / step).floor() as i64;
        (first..=last)
            .take(MOST_LINES)
            .map(|k| at((k as f64 * step) as f32))
            .collect()
    };
    (
        lines(low[0], high[0], &|x| vw.on_screen(rect, [x, 0.0]).x),
        lines(low[1], high[1], &|y| vw.on_screen(rect, [0.0, y]).y),
    )
}

/// The Layout view's state: the view (none until the first frame fits it), the plan of the
/// model revision it was made for, and whether the work area shows it. It shares the 3D tab's
/// flag (`show_3d`), so every place that goes back to the model view clears both.
#[derive(Default)]
pub(crate) struct LayoutView {
    /// The Layout tab is the one showing, not the 3D one.
    pub(crate) on: bool,
    plan: Option<(u64, Plan)>,
    view: Option<Vw>,
}

/// Writes `text` centred at `at` over the drawing as a label (so a screen reader reads it), on a
/// faint patch of the canvas colour, if it fits inside `canvas`.
fn name_label(ui: &mut egui::Ui, canvas: Rect, at: Pos2, text: &str, p: &theme::Palette) {
    let font = FontId::proportional(12.0);
    let size = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), p.text)
        .size();
    let r = Rect::from_center_size(at, size);
    if !canvas.contains_rect(r) {
        return;
    }
    let back = p.canvas.gamma_multiply(0.75);
    ui.painter().rect_filled(r.expand(2.0), 2.0, back);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
    child.add(
        egui::Label::new(RichText::new(text).font(font).color(p.text))
            .extend()
            .selectable(false),
    );
}

impl App {
    /// The Layout view, filling the work area.
    pub(crate) fn layout_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let revision = self.engine.revision();
        if !matches!(&self.layout_view.plan, Some((r, _)) if *r == revision) {
            self.layout_view.plan = Some((revision, build_plan(self.engine.model())));
        }
        let LayoutView { plan, view, .. } = &mut self.layout_view;
        let Some((_, plan)) = plan else { return };
        let plan: &Plan = plan;

        let mut fit = false;
        ui.horizontal(|ui| {
            fit = ui.button(t("Fit")).clicked();
            ui.label(RichText::new(t("Positions from the diagram")).color(p.text_weak));
        });
        ui.add_space(4.0);

        // The rows under the drawing: the scale and the pointer, then the unplaced list, each a
        // single line so the drawing can take all the rest.
        let row = ui
            .text_style_height(&egui::TextStyle::Body)
            .max(ui.spacing().interact_size.y);
        let reserve = 4.0 * row + 5.0 * ui.spacing().item_spacing.y + 4.0;
        let size = (ui.available_size() - vec2(0.0, reserve)).max(vec2(120.0, 120.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Canvas, true, t("Layout of the model, to scale"))
        });
        let hint = t("Drag to pan · Scroll to zoom");
        ui.ctx()
            .accesskit_node_builder(resp.id, |n| n.set_description(hint));

        let vw = view.get_or_insert_with(|| Vw::fitted(plan.low, plan.high, rect.size()));
        if fit {
            *vw = Vw::fitted(plan.low, plan.high, rect.size());
        }
        if resp.dragged() {
            vw.pan(resp.drag_delta());
        }
        if resp.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let at = resp.hover_pos().unwrap_or_else(|| rect.center());
            if scroll != 0.0 {
                vw.zoom_at(rect, at, libm::expf(scroll * 0.0015));
            }
            if pinch != 1.0 {
                vw.zoom_at(rect, at, pinch);
            }
        }
        if resp.has_focus() {
            let (left, right, up, down, closer, further) = ui.input(|i| {
                (
                    i.key_pressed(Key::ArrowLeft),
                    i.key_pressed(Key::ArrowRight),
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals),
                    i.key_pressed(Key::Minus),
                )
            });
            let step = |on: bool| if on { 40.0 } else { 0.0 };
            vw.pan(vec2(step(left) - step(right), step(up) - step(down)));
            if closer {
                vw.zoom_at(rect, rect.center(), 1.1);
            }
            if further {
                vw.zoom_at(rect, rect.center(), 1.0 / 1.1);
            }
        }
        let vw = *vw;

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, p.canvas);
        let (xs, ys) = grid_lines(&vw, rect);
        let grid = Stroke::new(1.0, p.border);
        for x in xs {
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], grid);
        }
        for y in ys {
            painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], grid);
        }
        let belt = Stroke::new((belt_width() * vw.scale).max(2.0), p.chart);
        for b in &plan.belts {
            let points = b.points.iter().map(|q| vw.on_screen(rect, *q)).collect();
            painter.add(Shape::line(points, belt));
        }
        let outline = Stroke::new(1.5, p.control_border);
        for pl in &plan.places {
            let c = vw.on_screen(rect, pl.centre);
            let reach = pl.outline.reach() * vw.scale;
            match pl.outline {
                Outline::Square(_) => {
                    let r = Rect::from_center_size(c, vec2(2.0 * reach, 2.0 * reach));
                    painter.rect_filled(r, 2.0, p.accent_soft);
                    painter.rect_stroke(r, 2.0, outline, StrokeKind::Inside);
                }
                Outline::Diamond(_) => {
                    let corners = vec![
                        pos2(c.x + reach, c.y),
                        pos2(c.x, c.y - reach),
                        pos2(c.x - reach, c.y),
                        pos2(c.x, c.y + reach),
                    ];
                    painter.add(Shape::convex_polygon(corners, p.accent_soft, outline));
                }
            }
        }
        // The scale bar, bottom left: its length in metres written over it.
        let bar = nice_step(vw.scale, BAR_PX);
        let bar_px = (bar * f64::from(vw.scale)) as f32;
        let from = rect.left_bottom() + vec2(16.0, -16.0);
        let to = from + vec2(bar_px, 0.0);
        let bar_stroke = Stroke::new(2.0, p.text);
        painter.line_segment([from, to], bar_stroke);
        for end in [from, to] {
            painter.line_segment([end - vec2(0.0, 5.0), end + vec2(0.0, 5.0)], bar_stroke);
        }
        let bar_metres = number(bar);
        painter.text(
            from + vec2(0.0, -8.0),
            Align2::LEFT_BOTTOM,
            tf("{length} m", &[("length", &bar_metres)]),
            FontId::proportional(12.0),
            p.text,
        );
        painter.text(
            rect.left_top() + vec2(8.0, 8.0),
            Align2::LEFT_TOP,
            hint,
            FontId::proportional(12.0),
            p.text_weak,
        );
        if resp.has_focus() {
            painter.rect_stroke(
                rect.shrink(1.0),
                4.0,
                Stroke::new(2.0, p.accent),
                StrokeKind::Inside,
            );
        }
        // The names, as labels over the drawing.
        for pl in &plan.places {
            name_label(ui, rect, vw.on_screen(rect, pl.centre), &pl.name, &p);
        }
        for b in &plan.belts {
            name_label(ui, rect, vw.on_screen(rect, b.middle), &b.name, &p);
        }

        ui.horizontal(|ui| {
            ui.label(tf("Scale bar: {length} m", &[("length", &bar_metres)]));
            if let Some(at) = resp.hover_pos() {
                let w = vw.in_metres(rect, at);
                let (x, y) = (format!("{:.2}", w[0]), format!("{:.2}", w[1]));
                ui.label(tf("x {x} m, y {y} m", &[("x", &x), ("y", &y)]));
            }
        });
        ui.label(RichText::new(t("Unplaced")).strong());
        let note = if plan.unplaced.is_empty() {
            t("Every block is placed from the diagram for now. A block the diagram leaves out is listed here and is not drawn.").to_owned()
        } else {
            plan.unplaced.join(", ")
        };
        ui.add(egui::Label::new(RichText::new(note).color(p.text_weak)).truncate());
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use stochcraft_model::{Bounds, Connection, ConveyorShape, Dist, Element, LengthUnit};

    fn id(s: &str) -> ElementId {
        ElementId::unchecked(s)
    }

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0))
    }

    fn put(m: &mut Model, name: &str, x: f64, y: f64) {
        m.layout.insert(
            id(name),
            Bounds {
                x,
                y,
                width: 100.0,
                height: 80.0,
            },
        );
    }

    /// A source and a sink 8 m apart on the canvas (400 drawing units), a conveyor of `shape` and
    /// `length` metres between them.
    fn line_with(shape: ConveyorShape, length: f64) -> Model {
        let mut m = Model::default();
        m.experiment.length_unit = LengthUnit::Metres;
        m.elements.push(Element::new(
            id("in"),
            "In",
            ElementKind::Source {
                interarrival: Dist::Constant(1.0),
                limit: None,
                timed: Vec::new(),
                idle_outside: false,
            },
        ));
        m.elements.push(Element::new(
            id("belt"),
            "Belt",
            ElementKind::Conveyor {
                length,
                speed: 1.0,
                accumulating: false,
                item_length: 0.5,
                gap: 0.1,
                shape,
            },
        ));
        m.elements.push(Element::new(
            id("out"),
            "Out",
            ElementKind::Sink {
                scrap: None,
                demand: None,
            },
        ));
        for (a, b) in [("in", "belt"), ("belt", "out")] {
            m.connections
                .push(Connection::new(id(&format!("{a}-{b}")), id(a), id(b)));
        }
        put(&mut m, "in", 0.0, 0.0);
        put(&mut m, "belt", 200.0, 0.0);
        put(&mut m, "out", 400.0, 0.0);
        m
    }

    fn polyline_length(points: &[[f32; 2]]) -> f32 {
        points
            .windows(2)
            .map(|w| libm::hypotf(w[1][0] - w[0][0], w[1][1] - w[0][1]))
            .sum()
    }

    #[test]
    fn a_straight_conveyor_runs_its_own_length_from_the_edge_of_the_block_before() {
        // The source is at (1, -0.8) m with a half-side of 0.45 m, so the belt starts at 1.45 m
        // and, 10 m long, ends at 11.45 m, though the sink is nearer: the length is the model's.
        let p = build_plan(&line_with(ConveyorShape::Straight, 10.0));
        let belt = &p.belts[0];
        let (first, last) = (belt.points[0], belt.points[belt.points.len() - 1]);
        assert!((first[0] - 1.45).abs() < 1e-3 && (first[1] + 0.8).abs() < 1e-3);
        assert!((last[0] - 11.45).abs() < 1e-3 && (last[1] + 0.8).abs() < 1e-3);
        assert!((polyline_length(&belt.points) - 10.0).abs() < 1e-3);
        assert!((belt.middle[0] - 6.45).abs() < 1e-3);
        assert_eq!(p.places.len(), 2);
        assert!(p.unplaced.is_empty());
    }

    #[test]
    fn a_curved_conveyor_is_drawn_to_its_arc_length() {
        // A left quarter turn of radius 4 m: 2 pi = 6.283 m. It leaves along +x and ends 4 m on
        // and 4 m to the left, that is up on the floor.
        let shape = ConveyorShape::Curve {
            radius: 4.0,
            angle: 90.0,
        };
        let p = build_plan(&line_with(shape, 1.0));
        let belt = &p.belts[0];
        assert!((polyline_length(&belt.points) - 2.0 * std::f32::consts::PI).abs() < 0.01);
        let last = belt.points[belt.points.len() - 1];
        assert!(
            (last[0] - 5.45).abs() < 1e-3 && (last[1] - 3.2).abs() < 1e-3,
            "{last:?}"
        );
    }

    #[test]
    fn a_conveyor_in_another_unit_is_drawn_in_metres() {
        let mut m = line_with(ConveyorShape::Straight, 1000.0);
        m.experiment.length_unit = LengthUnit::Millimetres;
        let p = build_plan(&m);
        assert!((polyline_length(&p.belts[0].points) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_model_with_no_diagram_places_every_block() {
        let mut m = line_with(ConveyorShape::Straight, 10.0);
        m.layout.clear();
        let p = build_plan(&m);
        assert!(p.unplaced.is_empty());
        assert_eq!(p.places.len() + p.belts.len(), 3);
    }

    #[test]
    fn an_empty_model_has_a_small_box_to_fit() {
        let p = build_plan(&Model::default());
        assert_eq!((p.low, p.high), ([-2.0, -2.0], [2.0, 2.0]));
    }

    #[test]
    fn fitting_shows_the_whole_box_and_zooming_keeps_the_point_under_the_pointer() {
        let r = rect();
        let vw = Vw::fitted([0.0, 0.0], [20.0, 10.0], r.size());
        // 400 - 80 = 320 px for 20 m is 16 px a metre; 300 - 80 = 220 px for 10 m is 22.
        assert!((vw.scale - 16.0).abs() < 1e-4);
        assert_eq!(vw.centre, [10.0, 5.0]);
        let corner = vw.on_screen(r, [0.0, 0.0]);
        assert!((corner.x - (r.center().x - 160.0)).abs() < 1e-3);
        assert!(
            corner.y > r.center().y,
            "y grows up in metres, down on screen"
        );
        let at = pos2(r.left() + 100.0, r.top() + 50.0);
        let before = vw.in_metres(r, at);
        let mut zoomed = vw;
        zoomed.zoom_at(r, at, 2.0);
        assert!((zoomed.scale - 32.0).abs() < 1e-4);
        let after = zoomed.in_metres(r, at);
        assert!((before[0] - after[0]).abs() < 1e-4 && (before[1] - after[1]).abs() < 1e-4);
        // Panning by a drag moves the drawing with the pointer.
        let mut panned = vw;
        let was = panned.on_screen(r, [3.0, 4.0]);
        panned.pan(vec2(25.0, -10.0));
        let now = panned.on_screen(r, [3.0, 4.0]);
        assert!((now.x - was.x - 25.0).abs() < 1e-3 && (now.y - was.y + 10.0).abs() < 1e-3);
    }

    #[test]
    fn the_grid_is_a_metre_apart_and_coarsens_when_zoomed_out() {
        let r = rect();
        // 20 px a metre: every metre.
        let near = Vw {
            centre: [0.0, 0.0],
            scale: 20.0,
        };
        let (xs, _) = grid_lines(&near, r);
        assert!((xs[1] - xs[0] - 20.0).abs() < 1e-3, "{xs:?}");
        // 1 px a metre: a line every 20 m (the first of the steps that is 12 px or more).
        let far = Vw {
            centre: [0.0, 0.0],
            scale: 1.0,
        };
        let (xs, ys) = grid_lines(&far, r);
        assert!((xs[1] - xs[0] - 20.0).abs() < 1e-3, "{xs:?}");
        assert!(!ys.is_empty() && xs.len() <= MOST_LINES);
    }

    #[test]
    fn the_scale_bar_is_a_round_number_of_metres_of_at_least_80_pixels() {
        // 20 px a metre: 5 m is 100 px (2 m would be 40).
        assert_eq!(nice_step(20.0, BAR_PX), 5.0);
        // 1 px a metre: 100 m.
        assert_eq!(nice_step(1.0, BAR_PX), 100.0);
        // 400 px a metre: 0.2 m is 80 px.
        assert_eq!(nice_step(400.0, BAR_PX), 0.2);
    }
}
