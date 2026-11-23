//! The Layout view (M5.5, L1; drawn from the layout since L3): the model's plan in metres, to
//! scale, read-only. A grid, a scale bar, the floor and markings, every conveyor as its belt along
//! its track's path with its width and rails, every placed block as its footprint with its name,
//! and the list of blocks that are not placed.
//!
//! Positions come only from the model's layout ([`stochcraft_model::Site`]); a block with no
//! place is listed as unplaced and drawn nowhere, as in the 3D view. The plan and the 3D view
//! use the same paths and frames ([`crate::view3d::track_metres`],
//! [`crate::view3d::frame_metres`]), so every item is at the same `point_at(s)` in both.
//!
//! Conventions: metres, `x` right and `y` up, like the 3D view's floor. Drag to pan, scroll to
//! zoom about the pointer, Fit to see everything; the arrow keys and plus and minus work while
//! the drawing has focus.

use egui::{
    Align2, FontId, Key, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2,
};
use stochcraft_model::{Element, ElementKind, Model};

use crate::App;
use crate::i18n::{t, tf};
use crate::results::number;
use crate::theme;
use crate::view3d::{RAIL, frame_metres, look_of, stations, track_metres, unit_metres};

/// The least and most pixels a metre may take.
const MIN_SCALE: f32 = 0.05;
const MAX_SCALE: f32 = 2000.0;
/// The closest the grid lines may be, in pixels; the grid coarsens beyond it.
const MIN_GRID_PX: f32 = 12.0;
/// The finest the grid may be, in metres.
const MIN_GRID_M: f64 = 1.0;
/// About how long the scale bar is, in pixels, at least.
const BAR_PX: f32 = 80.0;
/// The most grid lines drawn each way.
const MOST_LINES: usize = 400;
/// The space round the drawing when it is fitted, in pixels.
const FIT_PAD: f32 = 40.0;

/// The distances a grid or a scale bar may take, in metres.
const NICE: [f64; 18] = [
    0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0,
    2000.0, 5000.0,
];

/// The first of [`NICE`] of at least `least_m` metres that takes at least `least_px` pixels at
/// `scale` pixels a metre.
fn nice_step(scale: f32, least_px: f32, least_m: f64) -> f64 {
    NICE.into_iter()
        .find(|s| *s >= least_m && s * f64::from(scale) >= f64::from(least_px))
        .unwrap_or(10_000.0)
}

/// A rectangle on the plan: its centre, its size along and across its heading, and the heading
/// in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Footprint {
    pub centre: [f32; 2],
    pub along: f32,
    pub across: f32,
    pub heading: f32,
}

impl Footprint {
    /// Its four corners, anticlockwise.
    fn corners(&self) -> [[f32; 2]; 4] {
        let (s, c) = (libm::sinf(self.heading), libm::cosf(self.heading));
        let (a, b) = (self.along / 2.0, self.across / 2.0);
        [(-a, -b), (a, -b), (a, b), (-a, b)].map(|(x, y)| {
            [
                self.centre[0] + c * x - s * y,
                self.centre[1] + s * x + c * y,
            ]
        })
    }
}

/// A block (not a conveyor) as it is drawn: its name and its footprint.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Place {
    name: String,
    footprint: Footprint,
}

/// A conveyor as it is drawn: its name, its path cut into points (in metres), its width, and
/// the point half way along, where its name goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Belt {
    name: String,
    points: Vec<[f32; 2]>,
    width: f32,
    middle: [f32; 2],
}

/// What the Layout view draws, worked out from the model once per revision.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Plan {
    /// Floor areas and markings from the scenery.
    floors: Vec<Footprint>,
    markings: Vec<Footprint>,
    places: Vec<Place>,
    belts: Vec<Belt>,
    /// The names of the blocks that have no place: listed, not drawn.
    unplaced: Vec<String>,
    /// The corners of the box round everything drawn.
    low: [f32; 2],
    high: [f32; 2],
}

/// What a block is called on the drawing: its name, or its id when it has none.
pub(crate) fn label_of(e: &Element) -> String {
    if e.name.is_empty() {
        e.id.as_str().to_owned()
    } else {
        e.name.clone()
    }
}

/// The plan of `model`, from its layout; a block with no place is unplaced.
fn build_plan(model: &Model) -> Plan {
    let mut places = Vec::new();
    let mut belts = Vec::new();
    let mut unplaced = Vec::new();
    for e in &model.elements {
        let name = label_of(e);
        if let ElementKind::Conveyor { length, .. } = &e.kind {
            let Some((path, width)) = track_metres(model, &e.id) else {
                unplaced.push(name);
                continue;
            };
            let points: Vec<[f32; 2]> = stations(&path).iter().map(|s| [s.x, s.y]).collect();
            let mid = path.point_at(path.length() / 2.0);
            let own = number(*length * unit_metres(model.experiment.length_unit));
            belts.push(Belt {
                name: format!("{name} · {}", tf("{length} m", &[("length", &own)])),
                points,
                width,
                middle: [mid.x as f32, mid.y as f32],
            });
            continue;
        }
        let Some(f) = frame_metres(model, e) else {
            unplaced.push(name);
            continue;
        };
        let look = look_of(e);
        places.push(Place {
            name,
            footprint: Footprint {
                centre: [f.x as f32, f.y as f32],
                along: look.footprint[0],
                across: look.footprint[1],
                heading: f.heading_deg.to_radians() as f32,
            },
        });
    }
    let m = unit_metres(model.experiment.length_unit);
    let mut floors = Vec::new();
    let mut markings = Vec::new();
    for o in &model.site.scene {
        let fp = Footprint {
            centre: [(o.at.x * m) as f32, (o.at.y * m) as f32],
            along: (o.size[1] * m) as f32,
            across: (o.size[0] * m) as f32,
            heading: o.at.heading_deg.to_radians() as f32,
        };
        match o.kind.as_str() {
            "floor" => floors.push(fp),
            "marking" => markings.push(fp),
            _ => {}
        }
    }
    // What is drawn, each point with how far round it its drawing reaches; the floor only when
    // nothing else is.
    let mut reach: Vec<([f32; 2], f32)> = places
        .iter()
        .flat_map(|p| p.footprint.corners().map(|c| (c, 0.0)))
        .chain(
            belts
                .iter()
                .flat_map(|b| b.points.iter().map(|q| (*q, b.width / 2.0 + RAIL))),
        )
        .collect();
    if reach.is_empty() {
        reach = floors
            .iter()
            .flat_map(|f| f.corners().map(|c| (c, 0.0)))
            .collect();
    }
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    for (p, by) in reach {
        if p.iter().all(|c| c.is_finite()) {
            low = [low[0].min(p[0] - by), low[1].min(p[1] - by)];
            high = [high[0].max(p[0] + by), high[1].max(p[1] + by)];
        }
    }
    if low[0] > high[0] {
        (low, high) = ([-2.0, -2.0], [2.0, 2.0]);
    }
    Plan {
        floors,
        markings,
        places,
        belts,
        unplaced,
        low,
        high,
    }
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

/// Grid lines across one axis: each line's screen position, and whether it is a strong one.
type Lines = Vec<(f32, bool)>;

/// The screen `x` of every vertical grid line and the screen `y` of every horizontal one in
/// `rect`, a metre apart or wider when that would crowd them.
fn grid_lines(vw: &Vw, rect: Rect) -> (Lines, Lines) {
    let step = nice_step(vw.scale, MIN_GRID_PX, MIN_GRID_M);
    // A stronger line every 5 steps (every 2 for 5, 50 …, which makes 10, 100 …).
    let every: i64 = if [5.0, 50.0, 500.0, 5000.0].contains(&step) {
        2
    } else {
        5
    };
    let low = vw.in_metres(rect, rect.left_bottom());
    let high = vw.in_metres(rect, rect.right_top());
    let lines = |a: f32, b: f32, at: &dyn Fn(f32) -> f32| -> Vec<(f32, bool)> {
        let first = (f64::from(a) / step).ceil() as i64;
        let last = (f64::from(b) / step).floor() as i64;
        (first..=last)
            .take(MOST_LINES)
            .map(|k| (at((k as f64 * step) as f32), k.rem_euclid(every) == 0))
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
fn name_label(
    ui: &mut egui::Ui,
    canvas: Rect,
    at: Pos2,
    text: &str,
    p: &theme::Palette,
    drawn: &mut Vec<Rect>,
) {
    let font = FontId::proportional(12.0);
    let size = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font.clone(), p.text)
        .size();
    let r = Rect::from_center_size(at, size);
    // A name that would sit on another, or off the drawing, is not painted; it stays in the
    // accessibility tree, as transparent text.
    let shown = canvas.contains_rect(r) && !drawn.iter().any(|d| d.intersects(r.expand(2.0)));
    let colour = if shown {
        let back = p.canvas.gamma_multiply(0.75);
        ui.painter().rect_filled(r.expand(2.0), 2.0, back);
        drawn.push(r);
        p.text
    } else {
        egui::Color32::TRANSPARENT
    };
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
    child.add(
        egui::Label::new(RichText::new(text).font(font).color(colour))
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
            ui.label(RichText::new(t("Positions from the layout")).color(p.text_weak));
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
        let poly = |fp: &Footprint| -> Vec<Pos2> {
            fp.corners()
                .iter()
                .map(|c| vw.on_screen(rect, *c))
                .collect()
        };
        for fl in &plan.floors {
            painter.add(Shape::convex_polygon(poly(fl), p.stripe, Stroke::NONE));
        }
        let (xs, ys) = grid_lines(&vw, rect);
        let minor = Stroke::new(1.0, p.border);
        let major = Stroke::new(1.5, p.text_weak.gamma_multiply(0.45));
        for (x, strong) in xs {
            let stroke = if strong { major } else { minor };
            painter.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], stroke);
        }
        for (y, strong) in ys {
            let stroke = if strong { major } else { minor };
            painter.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
        }
        for mk in &plan.markings {
            painter.add(Shape::convex_polygon(poly(mk), p.warning, Stroke::NONE));
        }
        // Each belt: its rails as a wider band, the belt inside them.
        for b in &plan.belts {
            let points: Vec<Pos2> = b.points.iter().map(|q| vw.on_screen(rect, *q)).collect();
            let rails = Stroke::new(
                ((b.width + 2.0 * RAIL) * vw.scale).max(3.0),
                p.control_border,
            );
            let belt = Stroke::new((b.width * vw.scale).max(2.0), p.chart);
            painter.add(Shape::line(points.clone(), rails));
            painter.add(Shape::line(points, belt));
        }
        let outline = Stroke::new(1.5, p.control_border);
        for pl in &plan.places {
            painter.add(Shape::convex_polygon(
                poly(&pl.footprint),
                p.accent_soft,
                outline,
            ));
        }
        // The scale bar, bottom left: its length in metres written over it.
        let bar = nice_step(vw.scale, BAR_PX, 0.01);
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
        // The names, as labels over the drawing: blocks first, then conveyors, none on another.
        let mut drawn = Vec::new();
        for pl in &plan.places {
            name_label(
                ui,
                rect,
                vw.on_screen(rect, pl.footprint.centre),
                &pl.name,
                &p,
                &mut drawn,
            );
        }
        for b in &plan.belts {
            name_label(
                ui,
                rect,
                vw.on_screen(rect, b.middle),
                &b.name,
                &p,
                &mut drawn,
            );
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
            t("Every block is placed.").to_owned()
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

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0))
    }

    fn sorter() -> Model {
        let mut e = stochcraft_engine::Engine::new();
        e.open_bytes(
            "sorter.bpmn",
            include_bytes!("../../../tests/models/sorter.bpmn"),
            None,
        )
        .unwrap();
        e.model().clone()
    }

    #[test]
    fn the_sorter_is_drawn_from_its_layout() {
        let p = build_plan(&sorter());
        assert!(p.unplaced.is_empty(), "{:?}", p.unplaced);
        assert_eq!((p.places.len(), p.belts.len()), (7, 6));
        assert_eq!(p.floors.len(), 1);
        let belt = |name: &str| p.belts.iter().find(|b| b.name.starts_with(name)).unwrap();
        // The loop runs its 30 m round and closes; its label gives its own length.
        let lp = belt("Loop");
        assert_eq!(lp.name, "Loop · 30 m");
        let (first, last) = (lp.points[0], lp.points[lp.points.len() - 1]);
        assert!(first[0].abs() < 1e-4 && last[0].abs() < 1e-4 && last[1].abs() < 1e-4);
        let drawn: f32 = lp
            .points
            .windows(2)
            .map(|w| libm::hypotf(w[1][0] - w[0][0], w[1][1] - w[0][1]))
            .sum();
        assert!((drawn - 30.0).abs() < 0.01, "{drawn}");
        // A lane leaves the sort straight at right angles, 4 m down to its station.
        let red = belt("Lane red");
        assert_eq!(red.points[0], [2.25, -0.45]);
        let end = red.points[red.points.len() - 1];
        assert!((end[0] - 2.25).abs() < 1e-4 && (end[1] + 4.45).abs() < 1e-4);
        let pack = p.places.iter().find(|x| x.name == "Pack red").unwrap();
        assert!((pack.footprint.centre[1] + 4.95).abs() < 1e-4);
        // The divert stands beside the loop's start, inside it.
        let divert = p
            .places
            .iter()
            .find(|x| x.name == "Divert by product")
            .unwrap();
        assert!(divert.footprint.centre[1] > 0.4);
    }

    #[test]
    fn a_model_without_a_layout_draws_nothing_and_lists_every_block() {
        let mut m = sorter();
        m.site = Default::default();
        let p = build_plan(&m);
        assert!(p.places.is_empty() && p.belts.is_empty());
        assert_eq!(p.unplaced.len(), m.elements.len());
        assert_eq!((p.low, p.high), ([-2.0, -2.0], [2.0, 2.0]));
    }

    #[test]
    fn a_footprint_turns_with_its_heading() {
        let fp = Footprint {
            centre: [1.0, 1.0],
            along: 2.0,
            across: 1.0,
            heading: core::f32::consts::FRAC_PI_2,
        };
        // Turned a quarter: 2 m along y, 1 m along x.
        let c = fp.corners();
        let xs = c.iter().map(|p| p[0]);
        let span_x = xs.clone().fold(f32::MIN, f32::max) - xs.fold(f32::MAX, f32::min);
        assert!((span_x - 1.0).abs() < 1e-5);
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
    fn the_grid_is_a_metre_apart_at_the_finest_and_coarsens_when_zoomed_out() {
        let r = rect();
        // 20 px a metre: every metre, and every fifth line stronger.
        let near = Vw {
            centre: [0.0, 0.0],
            scale: 20.0,
        };
        let (xs, _) = grid_lines(&near, r);
        assert!((xs[1].0 - xs[0].0 - 20.0).abs() < 1e-3, "{xs:?}");
        let strong: Vec<usize> = (0..xs.len()).filter(|i| xs[*i].1).collect();
        assert!(strong.windows(2).all(|w| w[1] - w[0] == 5), "{strong:?}");
        assert!(!strong.is_empty());
        // 200 px a metre: still a metre (0.1 m would be 20 px), 200 px apart.
        let close = Vw {
            centre: [0.0, 0.0],
            scale: 200.0,
        };
        let (xs, _) = grid_lines(&close, r);
        assert!((xs[1].0 - xs[0].0 - 200.0).abs() < 1e-3, "{xs:?}");
        // 1 px a metre: a line every 20 m (the first of the steps that is 12 px or more).
        let far = Vw {
            centre: [0.0, 0.0],
            scale: 1.0,
        };
        let (xs, ys) = grid_lines(&far, r);
        assert!((xs[1].0 - xs[0].0 - 20.0).abs() < 1e-3, "{xs:?}");
        assert!(!ys.is_empty() && xs.len() <= MOST_LINES);
    }

    #[test]
    fn the_scale_bar_is_a_round_number_of_metres_of_at_least_80_pixels() {
        // 20 px a metre: 5 m is 100 px (2 m would be 40).
        assert_eq!(nice_step(20.0, BAR_PX, 0.01), 5.0);
        // 1 px a metre: 100 m.
        assert_eq!(nice_step(1.0, BAR_PX, 0.01), 100.0);
        // 400 px a metre: 0.2 m is 80 px.
        assert_eq!(nice_step(400.0, BAR_PX, 0.01), 0.2);
    }
}
