//! The Layout view (M5.5, L1; drawn from the layout since L3, edited since L4): the model's plan
//! in metres, to scale. A grid, a scale bar, the floor and markings, every conveyor as its belt
//! along its track's path with its width and rails, every placed block as its footprint with its
//! name, and the list of blocks that are not placed.
//!
//! Positions come only from the model's layout ([`stochcraft_model::Site`]); a block with no
//! place is listed as unplaced and drawn nowhere, as in the 3D view. The plan and the 3D view
//! use the same paths and frames ([`crate::view3d::track_metres`],
//! [`crate::view3d::frame_metres`]), so every item is at the same `point_at(s)` in both.
//!
//! **Editing paths (L4, part 1).** As established layout editors do: **Draw path** for the
//! selected conveyor, then click to add points, double-click or Enter to finish, Backspace to take
//! back the last point, Esc to cancel; each corner bends with a radius ([`Outline`]). A click on a
//! belt or a block selects it, shared with the flowchart and the properties panel. The selected
//! belt shows its points: drag one to move it, drag the belt to move the whole path, double-click
//! the belt to add a point there, Delete to remove the selected point. Each point is a control
//! the keyboard reaches with Tab: the arrow keys move it by 0.1 m (1 m with Shift). Every gesture
//! is one `track.set`, one undo step. The properties panel lists the points as fields, the way
//! without a pointer.
//!
//! Conventions: metres, `x` right and `y` up, like the 3D view's floor. Drag to pan, scroll to
//! zoom about the pointer, Fit to see everything; the arrow keys and plus and minus work while
//! the drawing has focus.

use egui::{
    Align2, FontId, Key, Modifiers, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Vec2,
    pos2, vec2,
};
use serde_json::{Value, json};
use stochcraft_model::site::{default_bend, to_step};
use stochcraft_model::{Element, ElementId, ElementKind, Model, Outline, Path, Segment};

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
    id: String,
    name: String,
    footprint: Footprint,
}

/// A conveyor as it is drawn: its name, its path cut into points (in metres), its width, and
/// the point half way along, where its name goes.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Belt {
    id: String,
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
    /// The blocks that have no place, listed, not drawn: each one's id, name, and whether it is
    /// a conveyor (whose path can be drawn).
    unplaced: Vec<(String, String, bool)>,
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
        let id = e.id.as_str().to_owned();
        if let ElementKind::Conveyor { length, .. } = &e.kind {
            let Some((path, width)) = track_metres(model, &e.id) else {
                unplaced.push((id, name, true));
                continue;
            };
            let points: Vec<[f32; 2]> = stations(&path).iter().map(|s| [s.x, s.y]).collect();
            let mid = path.point_at(path.length() / 2.0);
            let own = number(*length * unit_metres(model.experiment.length_unit));
            belts.push(Belt {
                id,
                name: format!("{name} · {}", tf("{length} m", &[("length", &own)])),
                points,
                width,
                middle: [mid.x as f32, mid.y as f32],
            });
            continue;
        }
        let Some(f) = frame_metres(model, e) else {
            unplaced.push((id, name, false));
            continue;
        };
        let look = look_of(e);
        places.push(Place {
            id,
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
    /// A path being drawn.
    drawing: Option<Drawing>,
    /// The point picked on the selected conveyor's path: the conveyor and the point's index.
    point: Option<(String, usize)>,
    /// A drag under way on the plan.
    drag: Option<Drag>,
}

impl LayoutView {
    /// Whether the view takes Delete, Backspace, Esc and the arrows this frame (while it is
    /// shown): it is drawing, dragging, or has a point picked, so they act on that and not on
    /// the selected block.
    pub(crate) fn wants_keys(&self) -> bool {
        self.drawing.is_some() || self.point.is_some() || self.drag.is_some()
    }
}

/// A path being drawn: the conveyor's id and the points clicked so far, in the model's length
/// unit.
#[derive(Debug, Clone, PartialEq)]
struct Drawing {
    conveyor: String,
    points: Vec<[f64; 2]>,
}

/// A drag on the plan: of a point of the selected path or of the whole path (to and by, in the
/// model's length unit), or of the view. A drag of a path is of conveyor `conveyor` as the model
/// was at `revision`: if the model changes under it (an agent's edit, an undo), the drag ends
/// without writing over the change.
#[derive(Debug, Clone, PartialEq)]
enum Drag {
    Point {
        conveyor: String,
        revision: u64,
        index: usize,
        to: [f64; 2],
    },
    /// `at` is where the pointer was when `by` was last worked out.
    Whole {
        conveyor: String,
        revision: u64,
        by: [f64; 2],
        at: Pos2,
    },
    Pan,
}

/// The outline of points `points` (in the model's unit) with the default bend at each corner;
/// consecutive points that coincide (to 0.1 mm) count once.
fn drawn_outline(points: &[[f64; 2]], bend: f64) -> Outline {
    let mut kept: Vec<[f64; 2]> = Vec::with_capacity(points.len());
    for p in points {
        if kept.last() != Some(p) {
            kept.push(*p);
        }
    }
    let bends = vec![bend; kept.len().saturating_sub(2)];
    Outline {
        z: 0.0,
        points: kept,
        bends,
    }
}

/// The distance from `q` to the segment from `a` to `b`, and the point of it nearest `q`.
fn to_segment(a: [f64; 2], b: [f64; 2], q: [f64; 2]) -> (f64, [f64; 2]) {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let k = if len2 > 0.0 {
        (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let at = [a[0] + k * dx, a[1] + k * dy];
    let (ex, ey) = (q[0] - at[0], q[1] - at[1]);
    ((ex * ex + ey * ey).sqrt(), at)
}

/// Where a point added near `q` goes in `outline`: the index it takes and the point on the
/// nearest straight between two of its points.
fn insertion(outline: &Outline, q: [f64; 2]) -> Option<(usize, [f64; 2])> {
    outline
        .points
        .windows(2)
        .enumerate()
        .map(|(i, w)| {
            let (d, at) = to_segment(w[0], w[1], q);
            (d, i + 1, at)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, i, at)| (i, at))
}

/// The distance from `q` to the polyline through `points`.
fn to_polyline(points: &[[f32; 2]], q: [f32; 2]) -> f32 {
    let q = [f64::from(q[0]), f64::from(q[1])];
    points
        .windows(2)
        .map(|w| {
            let a = [f64::from(w[0][0]), f64::from(w[0][1])];
            let b = [f64::from(w[1][0]), f64::from(w[1][1])];
            to_segment(a, b, q).0
        })
        .fold(f64::INFINITY, f64::min) as f32
}

/// The belt under `q` (metres), the nearest whose band and rails reach within `slack` metres of
/// it.
fn belt_at(plan: &Plan, q: [f32; 2], slack: f32) -> Option<&Belt> {
    plan.belts
        .iter()
        .map(|b| (to_polyline(&b.points, q) - b.width / 2.0 - RAIL, b))
        .filter(|(d, _)| *d <= slack)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, b)| b)
}

/// The block whose footprint holds `q` (metres), the last drawn first.
fn place_at(plan: &Plan, q: [f32; 2]) -> Option<&Place> {
    plan.places.iter().rev().find(|pl| {
        let fp = &pl.footprint;
        let (s, c) = (libm::sinf(fp.heading), libm::cosf(fp.heading));
        let (dx, dy) = (q[0] - fp.centre[0], q[1] - fp.centre[1]);
        let (along, across) = (c * dx + s * dy, -s * dx + c * dy);
        along.abs() <= fp.along / 2.0 && across.abs() <= fp.across / 2.0
    })
}

/// A path as `track.set` takes it, in the model's unit; the width is left to the conveyor's.
fn track_json(path: &Path) -> Value {
    let doc = stochcraft_engine::site::TrackDoc {
        start: path.start.into(),
        segments: path.segments.iter().map(|s| (*s).into()).collect(),
        width: None,
    };
    serde_json::to_value(doc).unwrap_or(Value::Null)
}

/// `path` moved by `by` as a whole.
fn shifted(path: &Path, by: [f64; 2]) -> Path {
    let mut out = path.clone();
    out.start.x += by[0];
    out.start.y += by[1];
    out
}

/// Points along `path` (in the model's unit) as metres on the plan, for drawing it.
fn plan_points(path: &Path, m: f64) -> Vec<[f32; 2]> {
    stations(&path.scaled(m))
        .iter()
        .map(|s| [s.x, s.y])
        .collect()
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

/// The selected conveyor as the Layout view edits it: its id and name, its path in the model's
/// unit (none while it has no track), and the path's outline where its points can be edited.
struct Chosen {
    id: String,
    name: String,
    path: Option<Path>,
    outline: Option<Outline>,
}

/// How near a belt a click must be, in pixels, to pick it.
const PICK_PX: f32 = 6.0;
/// The side of a point's handle, in pixels: the least target WCAG 2.2 asks for.
const HANDLE_PX: f32 = 24.0;

impl App {
    /// Starts drawing conveyor `id`'s path in the Layout view, and shows the view.
    pub(crate) fn draw_path_of(&mut self, id: &str) {
        self.select(std::iter::once(id.to_owned()).collect());
        self.show_results = false;
        self.show_analysis = false;
        self.show_capacity = false;
        self.show_3d = true;
        self.layout_view.on = true;
        self.layout_view.point = None;
        self.layout_view.drag = None;
        self.layout_view.drawing = Some(Drawing {
            conveyor: id.to_owned(),
            points: Vec::new(),
        });
    }

    /// Sets conveyor `id`'s path to `path` (in the model's unit), in one undo step.
    pub(crate) fn set_path(&mut self, id: &str, path: &Path) -> bool {
        self.execute("track.set", json!({ "id": id, "track": track_json(path) }))
            .is_some()
    }

    /// Removes conveyor `id`'s path, which leaves it unplaced.
    pub(crate) fn remove_path(&mut self, id: &str) {
        if self
            .execute("track.set", json!({ "id": id, "track": null }))
            .is_some()
        {
            self.layout_view.point = None;
        }
    }

    /// Ends the drawing: its points become the conveyor's path, if there are two.
    fn finish_drawing(&mut self) {
        let Some(d) = self.layout_view.drawing.take() else {
            return;
        };
        let unit = self.engine.model().experiment.length_unit;
        let outline = drawn_outline(&d.points, default_bend(unit));
        if outline.points.len() < 2 {
            self.layout_view.drawing = Some(d);
            self.info(t("A path needs at least two points."));
            return;
        }
        if self.set_path(&d.conveyor, &outline.to_path()) {
            self.select(std::iter::once(d.conveyor).collect());
        }
    }

    /// The selected block, if it is a conveyor.
    fn chosen_conveyor(&self) -> Option<Chosen> {
        let model = self.engine.model();
        let id = self.selected.as_ref()?;
        let eid = ElementId::unchecked(id);
        let e = model.element(&eid)?;
        if !matches!(e.kind, ElementKind::Conveyor { .. }) {
            return None;
        }
        let path = model.site.paths.get(&eid).map(|t| t.path.clone());
        let outline = path.as_ref().and_then(Path::outline);
        Some(Chosen {
            id: id.clone(),
            name: label_of(e),
            path,
            outline,
        })
    }

    /// The Layout view, filling the work area.
    pub(crate) fn layout_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let revision = self.engine.revision();
        if !matches!(&self.layout_view.plan, Some((r, _)) if *r == revision) {
            self.layout_view.plan = Some((revision, build_plan(self.engine.model())));
        }
        let Some((revision, plan)) = self.layout_view.plan.take() else {
            return;
        };
        let unit = self.engine.model().experiment.length_unit;
        let m = unit_metres(unit);
        let chosen = self.chosen_conveyor();
        // What the view was doing, checked against the model as it is now (an undo, or another
        // file, may have taken the conveyor or the point away).
        let mut drawing = self.layout_view.drawing.take().filter(|d| {
            self.engine
                .model()
                .element(&ElementId::unchecked(&d.conveyor))
                .is_some_and(|e| matches!(e.kind, ElementKind::Conveyor { .. }))
        });
        let mut point = self.layout_view.point.take().filter(|(c, i)| {
            chosen.as_ref().is_some_and(|ch| {
                &ch.id == c && ch.outline.as_ref().is_some_and(|o| *i < o.points.len())
            })
        });
        let mut drag = self.layout_view.drag.take().filter(|d| match d {
            Drag::Pan => true,
            Drag::Point {
                conveyor,
                revision: r,
                ..
            }
            | Drag::Whole {
                conveyor,
                revision: r,
                ..
            } => {
                *r == revision
                    && chosen
                        .as_ref()
                        .is_some_and(|c| &c.id == conveyor && c.path.is_some())
            }
        });
        let drawing_name = drawing.as_ref().map(|d| {
            self.engine
                .model()
                .element(&ElementId::unchecked(&d.conveyor))
                .map_or_else(|| d.conveyor.clone(), label_of)
        });

        // The toolbar: Fit, then Draw path and Remove path, or while drawing, Finish and Cancel
        // with what to do.
        let (mut fit, mut start, mut remove, mut finish, mut cancel) =
            (false, false, false, false, false);
        ui.horizontal_wrapped(|ui| {
            fit = ui.button(t("Fit")).clicked();
            if let Some(name) = &drawing_name {
                finish = ui.button(t("Finish")).clicked();
                cancel = ui.button(t("Cancel")).clicked();
                ui.label(
                    RichText::new(tf(
                        "Drawing the path of {name}: click to add points, double-click or Enter to finish, Backspace to take back a point, Esc to cancel.",
                        &[("name", name)],
                    ))
                    .color(p.text_weak),
                );
            } else {
                start = ui
                    .add_enabled(chosen.is_some(), egui::Button::new(t("Draw path")))
                    .on_disabled_hover_text(t("Select a conveyor to draw its path"))
                    .clicked();
                let has_path = chosen.as_ref().is_some_and(|c| c.path.is_some());
                remove = ui
                    .add_enabled(has_path, egui::Button::new(t("Remove path")))
                    .clicked();
                ui.label(RichText::new(t("Positions from the layout")).color(p.text_weak));
            }
        });
        ui.add_space(4.0);

        // The rows under the drawing: the scale and the pointer, then the unplaced list, each a
        // single line so the drawing can take all the rest.
        let row = ui
            .text_style_height(&egui::TextStyle::Body)
            .max(ui.spacing().interact_size.y);
        let reserve = 5.0 * row + 6.0 * ui.spacing().item_spacing.y + 4.0;
        let size = (ui.available_size() - vec2(0.0, reserve)).max(vec2(120.0, 120.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Canvas, true, t("Layout of the model, to scale"))
        });
        let hint = if drawing.is_some() {
            t("Click to add a point · Double-click or Enter to finish · Esc to cancel")
        } else {
            t(
                "Click a belt to select it · Drag its points or the belt · Double-click it to add a point · Drag the floor to pan · Scroll to zoom",
            )
        };
        ui.ctx()
            .accesskit_node_builder(resp.id, |n| n.set_description(hint));

        let mut vw = self
            .layout_view
            .view
            .unwrap_or_else(|| Vw::fitted(plan.low, plan.high, rect.size()));
        if fit {
            vw = Vw::fitted(plan.low, plan.high, rect.size());
        }
        // A point on the screen in the model's unit, to 0.1 mm, and back.
        let to_model = |vw: &Vw, at: Pos2| -> [f64; 2] {
            let w = vw.in_metres(rect, at);
            [
                to_step(f64::from(w[0]) / m, unit),
                to_step(f64::from(w[1]) / m, unit),
            ]
        };
        let to_screen =
            |vw: &Vw, q: [f64; 2]| vw.on_screen(rect, [(q[0] * m) as f32, (q[1] * m) as f32]);
        let in_metres = |vw: &Vw, at: Pos2| vw.in_metres(rect, at);

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

        // What this frame changes in the model, applied once the view is drawn.
        let mut commits: Vec<(String, Path)> = Vec::new();
        let mut pick: Option<Option<String>> = None;
        let mut too_few = false;

        // The selected path's points, each a control of its own: picked by a click or Tab,
        // dragged to move, moved by the arrow keys.
        let mut handle_ids: Vec<egui::Id> = Vec::new();
        let mut handles: Vec<(Pos2, bool, bool)> = Vec::new();
        if drawing.is_none()
            && let Some(ch) = &chosen
            && let Some(outline) = &ch.outline
        {
            for (i, q) in outline.points.iter().enumerate() {
                let shown_at = match &drag {
                    Some(Drag::Point { index, to, .. }) if *index == i => *to,
                    _ => *q,
                };
                let pos = to_screen(&vw, shown_at);
                let hid = egui::Id::unique(("layout point", ch.id.as_str(), i));
                handle_ids.push(hid);
                let r = ui.interact(
                    Rect::from_center_size(pos, Vec2::splat(HANDLE_PX)),
                    hid,
                    Sense::click_and_drag(),
                );
                let n = (i + 1).to_string();
                let (x, y) = (format!("{:.2}", q[0] * m), format!("{:.2}", q[1] * m));
                let label = tf(
                    "Point {n} of {name}, x {x} m, y {y} m",
                    &[("n", &n), ("name", &ch.name), ("x", &x), ("y", &y)],
                );
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
                if r.clicked() || r.drag_started() || r.gained_focus() {
                    point = Some((ch.id.clone(), i));
                }
                if r.clicked() {
                    r.request_focus();
                }
                if r.drag_started() {
                    drag = Some(Drag::Point {
                        conveyor: ch.id.clone(),
                        revision,
                        index: i,
                        to: *q,
                    });
                }
                // The frame that lets go has the pointer where it was let go, even if it moved
                // in that frame (egui says `drag_stopped` and not `dragged` then).
                if (r.dragged() || r.drag_stopped())
                    && let Some(Drag::Point { index, to, .. }) = &mut drag
                    && *index == i
                    && let Some(at) = r.interact_pointer_pos()
                {
                    *to = to_model(&vw, at);
                }
                if r.drag_stopped() {
                    if let Some(Drag::Point { index, to, .. }) = drag
                        && index == i
                        && to != *q
                    {
                        commits.push((ch.id.clone(), outline.moved(i, to).to_path()));
                    }
                    drag = None;
                }
                if r.has_focus() {
                    let by = ui.input_mut(|k| {
                        let mut by = [0.0, 0.0];
                        // Shift first: a plain arrow's match also takes a shifted press.
                        for (mods, metres) in [(Modifiers::SHIFT, 1.0), (Modifiers::NONE, 0.1)] {
                            let d = metres / m;
                            if k.consume_key(mods, Key::ArrowLeft) {
                                by[0] -= d;
                            }
                            if k.consume_key(mods, Key::ArrowRight) {
                                by[0] += d;
                            }
                            if k.consume_key(mods, Key::ArrowUp) {
                                by[1] += d;
                            }
                            if k.consume_key(mods, Key::ArrowDown) {
                                by[1] -= d;
                            }
                        }
                        by
                    });
                    if by != [0.0, 0.0] {
                        let to = [to_step(q[0] + by[0], unit), to_step(q[1] + by[1], unit)];
                        commits.push((ch.id.clone(), outline.moved(i, to).to_path()));
                    }
                }
                let picked = point.as_ref().is_some_and(|(c, j)| c == &ch.id && *j == i);
                handles.push((pos, picked, r.has_focus()));
            }
        }

        // The drawing itself: a drag on the selected belt moves the whole path, anywhere else
        // pans; a click adds a point while drawing, else picks what is under it.
        if resp.drag_started() && drag.is_none() {
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .or(resp.interact_pointer_pos())
                .unwrap_or_else(|| rect.center());
            let slack = PICK_PX / vw.scale;
            let on_chosen = drawing.is_none()
                && chosen.as_ref().is_some_and(|c| {
                    c.path.is_some()
                        && belt_at(&plan, in_metres(&vw, origin), slack)
                            .is_some_and(|b| b.id == c.id)
                });
            drag = Some(match chosen.as_ref().filter(|_| on_chosen) {
                Some(c) => Drag::Whole {
                    conveyor: c.id.clone(),
                    revision,
                    by: [0.0, 0.0],
                    at: origin,
                },
                None => Drag::Pan,
            });
        }
        // The belt follows the pointer from where it was pressed to where it is let go,
        // including the movement before egui calls it a drag and in the frame that lets go.
        match &mut drag {
            Some(Drag::Whole { by, at, .. }) => {
                if (resp.dragged() || resp.drag_stopped())
                    && let Some(now) = resp.interact_pointer_pos()
                {
                    let d = now - *at;
                    by[0] += f64::from(d.x / vw.scale) / m;
                    by[1] -= f64::from(d.y / vw.scale) / m;
                    *at = now;
                }
            }
            _ if resp.dragged() => vw.pan(resp.drag_delta()),
            _ => {}
        }
        if resp.drag_stopped() {
            if let (Some(Drag::Whole { by, .. }), Some(ch)) = (&drag, &chosen)
                && let Some(path) = &ch.path
            {
                let by = [to_step(by[0], unit), to_step(by[1], unit)];
                if by != [0.0, 0.0] {
                    commits.push((ch.id.clone(), shifted(path, by)));
                }
            }
            drag = None;
        }
        if resp.clicked()
            && let Some(at) = resp.interact_pointer_pos()
        {
            let q = in_metres(&vw, at);
            if let Some(d) = &mut drawing {
                if resp.double_clicked() {
                    finish = true;
                } else {
                    d.points.push(to_model(&vw, at));
                }
            } else if resp.double_clicked() {
                if let Some(ch) = &chosen
                    && let Some(outline) = &ch.outline
                    && belt_at(&plan, q, PICK_PX / vw.scale).is_some_and(|b| b.id == ch.id)
                {
                    let here = to_model(&vw, at);
                    if let Some((i, on)) = insertion(outline, here) {
                        let on = [to_step(on[0], unit), to_step(on[1], unit)];
                        commits.push((
                            ch.id.clone(),
                            outline.inserted(i, on, default_bend(unit)).to_path(),
                        ));
                        point = Some((ch.id.clone(), i));
                    }
                }
            } else {
                let hit = belt_at(&plan, q, PICK_PX / vw.scale)
                    .map(|b| b.id.clone())
                    .or_else(|| place_at(&plan, q).map(|pl| pl.id.clone()));
                point = None;
                pick = Some(hit);
            }
        }

        // The keys, while no other control has the keyboard: Enter, Esc and Backspace while
        // drawing; Esc to drop a drag; Delete for the picked point, Esc to let it go.
        let focused = ui.ctx().memory(|mem| mem.focused());
        let free = focused.is_none_or(|f| f == resp.id || handle_ids.contains(&f));
        if free {
            let key =
                |ui: &mut egui::Ui, k: Key| ui.input_mut(|i| i.consume_key(Modifiers::NONE, k));
            if let Some(d) = &mut drawing {
                if key(ui, Key::Enter) {
                    finish = true;
                }
                if key(ui, Key::Escape) {
                    cancel = true;
                }
                if key(ui, Key::Backspace) || key(ui, Key::Delete) {
                    d.points.pop();
                }
            } else if matches!(drag, Some(Drag::Point { .. } | Drag::Whole { .. })) {
                if key(ui, Key::Escape) {
                    drag = None;
                }
            } else if let Some((c, i)) = point.clone() {
                if key(ui, Key::Delete) || key(ui, Key::Backspace) {
                    match chosen.as_ref().and_then(|ch| ch.outline.as_ref()) {
                        Some(o) => match o.removed(i) {
                            Some(less) => {
                                commits.push((c, less.to_path()));
                                point = None;
                            }
                            None => too_few = true,
                        },
                        None => point = None,
                    }
                } else if key(ui, Key::Escape) {
                    point = None;
                }
            }
        }
        if cancel {
            drawing = None;
        }

        // Painting: the floor, the grid, the markings, the belts and the blocks, then the
        // selected path, its points and what is being drawn or dragged.
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
        let outline_stroke = Stroke::new(1.5, p.control_border);
        for pl in &plan.places {
            let picked = self.selected.as_deref() == Some(pl.id.as_str());
            let stroke = if picked {
                Stroke::new(2.5, p.accent)
            } else {
                outline_stroke
            };
            painter.add(Shape::convex_polygon(
                poly(&pl.footprint),
                p.accent_soft,
                stroke,
            ));
        }
        // The selected path, as it is or as the drag would leave it, and its length then.
        let mut note: Option<(Pos2, String)> = None;
        if let Some(ch) = &chosen
            && let Some(path) = &ch.path
        {
            let shown = match (&drag, &ch.outline) {
                (Some(Drag::Point { index, to, .. }), Some(o)) => o.moved(*index, *to).to_path(),
                (Some(Drag::Whole { by, .. }), _) => shifted(path, *by),
                _ => path.clone(),
            };
            let pts: Vec<Pos2> = plan_points(&shown, m)
                .iter()
                .map(|q| vw.on_screen(rect, *q))
                .collect();
            painter.add(Shape::line(pts, Stroke::new(2.0, p.accent)));
            if matches!(drag, Some(Drag::Point { .. } | Drag::Whole { .. }))
                && let Some(at) = ui.input(|i| i.pointer.hover_pos())
            {
                let len = number(shown.length() * m);
                note = Some((at, tf("{length} m", &[("length", &len)])));
            }
        }
        for (pos, picked, focus) in &handles {
            let fill = if *picked { p.accent } else { p.canvas };
            painter.circle(*pos, 5.0, fill, Stroke::new(2.0, p.accent));
            if *focus {
                painter.circle_stroke(*pos, 9.0, Stroke::new(2.0, p.accent));
            }
        }
        // The path being drawn: its points so far and on to the pointer.
        if let Some(d) = &drawing {
            let mut pts = d.points.clone();
            if let Some(at) = resp.hover_pos() {
                pts.push(to_model(&vw, at));
            }
            let outline = drawn_outline(&pts, default_bend(unit));
            if outline.points.len() >= 2 {
                let path = outline.to_path();
                let line: Vec<Pos2> = plan_points(&path, m)
                    .iter()
                    .map(|q| vw.on_screen(rect, *q))
                    .collect();
                painter.add(Shape::line(line, Stroke::new(2.0, p.accent)));
                if let Some(at) = resp.hover_pos() {
                    let len = number(path.length() * m);
                    note = Some((at, tf("{length} m", &[("length", &len)])));
                }
            }
            for q in &d.points {
                painter.circle(to_screen(&vw, *q), 4.0, p.accent, Stroke::NONE);
            }
        }
        if let Some((at, text)) = &note {
            painter.text(
                *at + vec2(12.0, -12.0),
                Align2::LEFT_BOTTOM,
                text,
                FontId::proportional(12.0),
                p.text,
            );
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
        let mut draw_unplaced: Option<String> = None;
        if plan.unplaced.is_empty() {
            ui.label(RichText::new(t("Every block is placed.")).color(p.text_weak));
        } else {
            if plan.unplaced.iter().any(|(_, _, conveyor)| *conveyor) {
                ui.label(
                    RichText::new(t("Click a conveyor here to draw its path.")).color(p.text_weak),
                );
            }
            egui::ScrollArea::horizontal()
                .id_salt("unplaced")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (id, name, conveyor) in &plan.unplaced {
                            if *conveyor {
                                if ui.link(name).clicked() {
                                    draw_unplaced = Some(id.clone());
                                }
                            } else {
                                ui.label(RichText::new(name).color(p.text_weak));
                            }
                        }
                    });
                });
        }

        // The view's state back, then the changes.
        self.layout_view.plan = Some((revision, plan));
        self.layout_view.view = Some(vw);
        self.layout_view.drawing = drawing;
        self.layout_view.point = point;
        self.layout_view.drag = drag;
        if let Some(hit) = pick {
            self.select(hit.into_iter().collect());
        }
        for (id, path) in commits {
            self.set_path(&id, &path);
        }
        if too_few {
            self.info(t("A path needs at least two points."));
        }
        if finish {
            self.finish_drawing();
        }
        if remove && let Some(ch) = &chosen {
            self.remove_path(&ch.id);
        }
        if start && let Some(ch) = &chosen {
            self.draw_path_of(&ch.id);
        }
        if let Some(id) = draw_unplaced {
            self.draw_path_of(&id);
        }
    }

    /// The selected conveyor's path in the properties panel: its drawn length, and each point's
    /// x and y and each corner's bend as fields, with a point added at the end or taken away,
    /// so a path can be drawn and edited without a pointer.
    pub(crate) fn path_section(&mut self, ui: &mut egui::Ui, id: &str) {
        crate::editor::section(ui, t("Path"));
        let model = self.engine.model();
        let unit = model.experiment.length_unit;
        let unit_label = self.length_unit_label();
        let eid = ElementId::unchecked(id);
        let Some(track) = model.site.paths.get(&eid) else {
            let typed = match model.element(&eid).map(|e| &e.kind) {
                Some(ElementKind::Conveyor { length, .. }) => *length,
                _ => 0.0,
            };
            ui.label(theme::weak(ui, t("Not drawn in the layout yet.")));
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("Draw in the Layout view")).clicked() {
                    self.draw_path_of(id);
                }
                if ui.button(t("Add a straight path")).clicked() {
                    let straight = Path::new(
                        stochcraft_model::Frame::default(),
                        vec![stochcraft_model::Segment::Line {
                            length: typed,
                            rise: 0.0,
                        }],
                    );
                    self.set_path(id, &straight);
                }
            });
            return;
        };
        let path = track.path.clone();
        let drawn = track.length(unit);
        let shown = self.numbers.format(drawn, 0..=4);
        ui.label(tf(
            "Drawn length: {length} {unit}",
            &[("length", &shown), ("unit", &unit_label)],
        ));
        let Some(outline) = path.outline() else {
            // Flat and still without an outline, it turns a whole circle or more in one arc.
            let climbs = path.segments.iter().any(|s| match *s {
                Segment::Line { rise, .. } | Segment::Arc { rise, .. } => rise != 0.0,
                Segment::Helix { .. } => true,
            });
            let why = if climbs {
                t("This path climbs or has a helix: its points cannot be edited here.")
            } else {
                t(
                    "This path turns a whole circle or more in one arc: its points cannot be edited here.",
                )
            };
            ui.label(theme::weak(ui, why));
            if ui.button(t("Remove path")).clicked() {
                self.remove_path(id);
            }
            return;
        };
        let fitted = outline.fitted_bends();
        let mut edit: Option<Outline> = None;
        let mut refused = false;
        let n_points = outline.points.len();
        for (i, q) in outline.points.iter().enumerate() {
            let n = (i + 1).to_string();
            ui.horizontal_wrapped(|ui| {
                ui.label(tf("Point {n}", &[("n", &n)]));
                for (axis, value) in [("x", q[0]), ("y", q[1])] {
                    let prefix = if axis == "x" { t("x") } else { t("y") };
                    let current = self.numbers.format(value, 0..=4);
                    let name = if axis == "x" {
                        tf("x of point {n}", &[("n", &n)])
                    } else {
                        tf("y of point {n}", &[("n", &n)])
                    };
                    let key = format!("{id}/path/{i}/{axis}");
                    if let Some(typed) = self.small_field(ui, &key, &name, prefix, &current, 64.0) {
                        match self.numbers.parse(&typed) {
                            Ok(v) if v.is_finite() => {
                                let v = to_step(v, unit);
                                let to = if axis == "x" { [v, q[1]] } else { [q[0], v] };
                                edit = Some(outline.moved(i, to));
                            }
                            _ => refused = true,
                        }
                    }
                }
                if let Some(bend) = outline.bend(i) {
                    let current = self.numbers.format(bend, 0..=4);
                    let name = tf("Bend radius at point {n}", &[("n", &n)]);
                    let key = format!("{id}/path/{i}/bend");
                    if let Some(typed) =
                        self.small_field(ui, &key, &name, t("bend"), &current, 56.0)
                    {
                        match self.numbers.parse(&typed) {
                            Ok(v) if v.is_finite() && v >= 0.0 => {
                                edit = Some(outline.with_bend(i, to_step(v, unit)));
                            }
                            _ => refused = true,
                        }
                    }
                    ui.label(theme::weak(ui, &unit_label));
                    if let Some(Some(got)) = fitted.get(i)
                        && (got - bend).abs() > 1e-9 * bend.max(1.0)
                    {
                        let got = self.numbers.format(*got, 0..=3);
                        ui.label(theme::weak(
                            ui,
                            tf("tightened to {radius} to fit", &[("radius", &got)]),
                        ));
                    }
                }
                let remove = ui
                    .add_enabled(n_points > 2, egui::Button::new(t("Remove")))
                    .on_hover_text(tf("Remove point {n}", &[("n", &n)]));
                if remove.clicked() {
                    edit = outline.removed(i);
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(t("Add a point at the end")).clicked() {
                // A metre on from the end, the way the last straight goes.
                let k = n_points;
                let (a, b) = (outline.points[k - 2], outline.points[k - 1]);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                let len = (dx * dx + dy * dy).sqrt();
                let step = 1.0 / unit_metres(unit);
                let (ux, uy) = if len > 0.0 {
                    (dx / len, dy / len)
                } else {
                    (1.0, 0.0)
                };
                let at = [
                    to_step(b[0] + ux * step, unit),
                    to_step(b[1] + uy * step, unit),
                ];
                edit = Some(outline.inserted(k, at, default_bend(unit)));
            }
            if ui.button(t("Draw in the Layout view")).clicked() {
                self.draw_path_of(id);
            }
            if ui.button(t("Remove path")).clicked() {
                self.remove_path(id);
                // A field the click took the keyboard from commits its text in this frame; the
                // path is gone, so that edit is dropped rather than drawing the path again.
                edit = None;
            }
        });
        if refused {
            self.report_error(t("Not a number"));
        }
        if let Some(o) = edit {
            self.set_path(id, &o.to_path());
        }
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

    #[test]
    fn a_click_picks_the_belt_under_it_or_the_block() {
        let p = build_plan(&sorter());
        // The red lane runs from (2.25, -0.45) 4 m down; a point beside it, within its band.
        let lane = belt_at(&p, [2.4, -2.0], 0.05).unwrap();
        assert_eq!(lane.id, "lane_red");
        // Far from every belt, nothing.
        assert!(belt_at(&p, [2.4, -30.0], 0.05).is_none());
        // The red packing station's footprint, centred at y = -4.95 below the lane.
        let pack = p.places.iter().find(|x| x.name == "Pack red").unwrap();
        let hit = place_at(&p, pack.footprint.centre).unwrap();
        assert_eq!(hit.id, pack.id);
    }

    #[test]
    fn drawn_points_make_an_outline_and_a_point_added_goes_on_the_nearest_straight() {
        // A repeated click counts once; corners get the bend.
        let o = drawn_outline(&[[0.0, 0.0], [10.0, 0.0], [10.0, 0.0], [10.0, 6.0]], 1.0);
        assert_eq!(o.points, vec![[0.0, 0.0], [10.0, 0.0], [10.0, 6.0]]);
        assert_eq!(o.bends, vec![1.0]);
        // Near the second straight, half way up: it goes in at index 2, on the straight.
        let (i, at) = insertion(&o, [10.4, 3.0]).unwrap();
        assert_eq!(i, 2);
        assert_eq!(at, [10.0, 3.0]);
        // Near the first: index 1.
        assert_eq!(insertion(&o, [4.0, -1.0]).unwrap(), (1, [4.0, 0.0]));
    }

    #[test]
    fn a_path_goes_to_track_set_as_a_file_holds_it() {
        let o = Outline {
            z: 0.0,
            points: vec![[1.0, 1.0], [4.0, 1.0], [4.0, 3.0]],
            bends: vec![1.0],
        };
        let v = track_json(&o.to_path());
        assert_eq!(v["start"]["x"], 1.0);
        assert_eq!(v["segments"][0]["type"], "line");
        assert_eq!(v["segments"][1]["type"], "arc");
        assert!((v["segments"][1]["angle"].as_f64().unwrap() - 90.0).abs() < 1e-9);
        assert!(v.get("width").is_none(), "{v}");
        // Moving the whole path moves only its start.
        let moved = shifted(&o.to_path(), [2.0, -1.0]);
        assert_eq!((moved.start.x, moved.start.y), (3.0, 0.0));
    }
}
