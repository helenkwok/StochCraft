//! The 3D view (M5.3): the model's blocks at their layout positions, as the low-poly shapes of
//! `stochcraft-render`, drawn without any GPU code. Each frame the triangles are projected on the
//! CPU (an orbit camera with a perspective projection), back faces are dropped, the rest are
//! shaded flat by one directional light, sorted far to near (the painter's algorithm) and handed
//! to egui as meshes, so the same code draws in the desktop window and in the browser.
//!
//! The scene (every block's mesh in world space) is built once per model revision. While a run is
//! watched, totes are placed by the number of entities inside each block; exact positions of
//! single entities come later.
//!
//! Conventions: metres, `z` up. A shape's centre on the canvas `(x, y)` (y grows down) is
//! `(x, -y)` times [`METRES_PER_UNIT`], so the layout keeps its handedness seen from above.

use core::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use egui::{
    Align2, Color32, FontId, Key, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2,
};
use stochcraft_model::{ElementId, ElementKind, LengthUnit, Model};
use stochcraft_render::{
    Machine, Mesh, StraightConveyor, Tote, box_mesh, machine, straight_conveyor, tote,
};

use crate::i18n::t;
use crate::{App, chart_palette, theme};

/// Metres per drawing unit of the canvas: a 100-unit-wide shape is 2 m wide.
const METRES_PER_UNIT: f32 = 0.02;
/// The vertical field of view.
const FOV: f32 = FRAC_PI_4;
/// Triangles with a corner nearer than this to the camera (along its view direction) are dropped.
const NEAR: f32 = 0.1;
/// Triangles with an edge longer than this are split, so the painter's sort puts a long belt
/// and the totes on it in the right order.
const MAX_EDGE: f32 = 1.5;
/// The height of a conveyor's transport surface, as `StraightConveyor`'s default.
const BELT_HEIGHT: f32 = 0.85;
/// A tote's size.
const TOTE_SIZE: [f32; 3] = [0.5, 0.4, 0.3];
/// The most totes drawn beside a block; more are counted by a number.
const MOST_AT_A_BLOCK: usize = 10;
/// The most totes drawn on one conveyor.
const MOST_ON_A_BELT: usize = 100;
/// Triangles to an egui mesh, so a mesh never has more vertices than 16-bit indices reach.
const CHUNK: usize = 18_000;

type Vec3 = [f32; 3];
type Segment = (Vec3, Vec3);

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Vec3, k: f32) -> Vec3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: Vec3) -> f32 {
    libm::sqrtf(dot(a, a))
}

fn normalized(a: Vec3) -> Vec3 {
    let l = length(a);
    if l > 1e-12 { scale(a, 1.0 / l) } else { a }
}

fn midpoint(a: Vec3, b: Vec3) -> Vec3 {
    scale(add(a, b), 0.5)
}

/// What a triangle is part of, which decides its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Paint {
    Source,
    Sink,
    Machine,
    Delay,
    Gateway,
    Batch,
    Belt,
    /// A tote, by the order of the block it is at.
    Tote(usize),
}

/// A triangle in world space with the normal of its face.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tri {
    pub v: [Vec3; 3],
    pub normal: Vec3,
    pub paint: Paint,
}

/// Where a block's entities are drawn.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Spot {
    /// Beside a block: its centre on the floor and half its footprint.
    Block { centre: [f32; 2], half: f32 },
    /// Along a conveyor: where it starts on the floor, its heading and its length.
    Belt {
        start: [f32; 2],
        angle: f32,
        length: f32,
    },
}

/// A block of the model that can hold entities, in the scene.
#[derive(Debug, Clone)]
pub(crate) struct Anchor {
    pub id: String,
    /// The block's place in the model, for its totes' colour.
    pub order: usize,
    pub spot: Spot,
}

/// The model in world space.
#[derive(Debug, Clone, Default)]
pub(crate) struct Scene {
    pub tris: Vec<Tri>,
    /// Lines on the floor between shapes that no conveyor joins.
    pub links: Vec<Segment>,
    /// The floor grid.
    pub grid: Vec<Segment>,
    pub anchors: Vec<Anchor>,
    /// The corners of the box around everything.
    pub bounds: (Vec3, Vec3),
}

/// Metres in one of `unit`.
fn unit_metres(unit: LengthUnit) -> f32 {
    match unit {
        LengthUnit::Millimetres => 0.001,
        LengthUnit::Centimetres => 0.01,
        LengthUnit::Metres => 1.0,
        LengthUnit::Kilometres => 1000.0,
        LengthUnit::Inches => 0.0254,
        LengthUnit::Feet => 0.3048,
    }
}

/// Adds `v` to `out`, in halves while an edge is longer than [`MAX_EDGE`].
fn push_split(out: &mut Vec<Tri>, v: [Vec3; 3], normal: Vec3, paint: Paint, depth: u8) {
    let lens = [
        dot(sub(v[1], v[0]), sub(v[1], v[0])),
        dot(sub(v[2], v[1]), sub(v[2], v[1])),
        dot(sub(v[0], v[2]), sub(v[0], v[2])),
    ];
    let mut longest = 0;
    for (i, l) in lens.iter().enumerate() {
        if *l > lens[longest] {
            longest = i;
        }
    }
    if depth >= 6 || lens[longest] <= MAX_EDGE * MAX_EDGE {
        out.push(Tri { v, normal, paint });
        return;
    }
    let a = v[longest];
    let b = v[(longest + 1) % 3];
    let c = v[(longest + 2) % 3];
    let m = midpoint(a, b);
    push_split(out, [a, m, c], normal, paint, depth + 1);
    push_split(out, [m, b, c], normal, paint, depth + 1);
}

/// Adds a mesh's triangles to `out`.
fn push_mesh(out: &mut Vec<Tri>, mesh: &Mesh, paint: Paint) {
    for tri in mesh.indices.chunks_exact(3) {
        let at = |k: usize| mesh.positions.get(tri[k] as usize).copied();
        let (Some(a), Some(b), Some(c)) = (at(0), at(1), at(2)) else {
            continue;
        };
        let normal = mesh
            .normals
            .get(tri[0] as usize)
            .copied()
            .unwrap_or([0.0, 0.0, 1.0]);
        push_split(out, [a, b, c], normal, paint, 0);
    }
}

/// A block's shape, its paint, and half its footprint.
fn block_shape(kind: &ElementKind) -> (Mesh, Paint, f32) {
    match kind {
        ElementKind::Source { .. } => (
            box_mesh([0.9, 0.9, 0.6]).translated([0.0, 0.0, 0.3]),
            Paint::Source,
            0.45,
        ),
        ElementKind::Sink { .. } => (
            box_mesh([0.9, 0.9, 0.6]).translated([0.0, 0.0, 0.3]),
            Paint::Sink,
            0.45,
        ),
        ElementKind::Server { .. } | ElementKind::SharedTask { .. } => {
            (machine(Machine::default()), Paint::Machine, 0.8)
        }
        ElementKind::Delay { .. } => (
            box_mesh([1.2, 1.2, 0.08]).translated([0.0, 0.0, 0.04]),
            Paint::Delay,
            0.6,
        ),
        ElementKind::ExclusiveGateway { .. } | ElementKind::ParallelGateway => (
            box_mesh([0.7, 0.7, 0.7])
                .rotated_z(FRAC_PI_4)
                .translated([0.0, 0.0, 0.35]),
            Paint::Gateway,
            0.5,
        ),
        _ => (
            box_mesh([0.9, 0.9, 0.7]).translated([0.0, 0.0, 0.35]),
            Paint::Batch,
            0.45,
        ),
    }
}

/// The corners of the box around the triangles and links; a small box over the origin when there
/// are none.
fn bounds_of(tris: &[Tri], links: &[Segment]) -> (Vec3, Vec3) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    let points = tris
        .iter()
        .flat_map(|t| t.v)
        .chain(links.iter().flat_map(|s| [s.0, s.1]));
    for p in points {
        if p.iter().all(|c| c.is_finite()) {
            for ((l, h), c) in lo.iter_mut().zip(hi.iter_mut()).zip(p) {
                *l = l.min(c);
                *h = h.max(c);
            }
        }
    }
    if lo[0] > hi[0] {
        ([-2.0, -2.0, 0.0], [2.0, 2.0, 1.5])
    } else {
        (lo, hi)
    }
}

/// A floor grid under `bounds`, a metre apart or wider when the model is large.
fn floor_grid(bounds: (Vec3, Vec3)) -> Vec<Segment> {
    let (lo, hi) = bounds;
    let span = (hi[0] - lo[0]).max(hi[1] - lo[1]) + 4.0;
    let step = [1.0_f32, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0]
        .into_iter()
        .find(|s| span / s <= 50.0)
        .unwrap_or(1000.0);
    let x0 = libm::floorf((lo[0] - 2.0) / step) * step;
    let x1 = libm::ceilf((hi[0] + 2.0) / step) * step;
    let y0 = libm::floorf((lo[1] - 2.0) / step) * step;
    let y1 = libm::ceilf((hi[1] + 2.0) / step) * step;
    let count = |a: f32, b: f32| (((b - a) / step) as usize).min(100);
    let mut out = Vec::new();
    for i in 0..=count(x0, x1) {
        let x = x0 + i as f32 * step;
        out.push(([x, y0, 0.0], [x, y1, 0.0]));
    }
    for i in 0..=count(y0, y1) {
        let y = y0 + i as f32 * step;
        out.push(([x0, y, 0.0], [x1, y, 0.0]));
    }
    out
}

/// The model in world space: each block's shape at its layout position (a conveyor as a belt
/// from the middle of the shape before it to the middle of the shape after it), and a line on
/// the floor for each connection that no conveyor carries. The length of a conveyor with a
/// neighbour on only one side, or none, is the model's own, in metres, along `+x` from its shape.
pub(crate) fn scene(model: &Model) -> Scene {
    let placed = model.placed();
    let centre_of = |id: &ElementId| {
        placed.get(id).map(|b| {
            let (x, y) = b.centre();
            [x as f32 * METRES_PER_UNIT, -(y as f32) * METRES_PER_UNIT]
        })
    };
    let metres = unit_metres(model.experiment.length_unit);
    let is_conveyor = |id: &ElementId| {
        model
            .element(id)
            .is_some_and(|e| matches!(e.kind, ElementKind::Conveyor { .. }))
    };
    let mut tris = Vec::new();
    let mut anchors = Vec::new();
    for (order, e) in model.elements.iter().enumerate() {
        let Some(c) = centre_of(&e.id) else { continue };
        let id = e.id.as_str().to_owned();
        if let ElementKind::Conveyor { length: len, .. } = &e.kind {
            let before = model
                .connections
                .iter()
                .find(|k| k.to == e.id)
                .and_then(|k| centre_of(&k.from));
            let after = model
                .connections
                .iter()
                .find(|k| k.from == e.id)
                .and_then(|k| centre_of(&k.to));
            let own_end = [c[0] + *len as f32 * metres, c[1]];
            let (start, end) = match (before, after) {
                (Some(a), Some(b)) if (b[0] - a[0]).abs() + (b[1] - a[1]).abs() > 0.05 => (a, b),
                _ => (c, own_end),
            };
            let (dx, dy) = (end[0] - start[0], end[1] - start[1]);
            let belt_length = libm::sqrtf(dx * dx + dy * dy);
            let angle = libm::atan2f(dy, dx);
            let mesh = straight_conveyor(StraightConveyor {
                length: belt_length,
                ..StraightConveyor::default()
            });
            let placed_mesh = mesh.rotated_z(angle).translated([start[0], start[1], 0.0]);
            push_mesh(&mut tris, &placed_mesh, Paint::Belt);
            anchors.push(Anchor {
                id,
                order,
                spot: Spot::Belt {
                    start,
                    angle,
                    length: belt_length,
                },
            });
        } else {
            let (mesh, paint, half) = block_shape(&e.kind);
            push_mesh(&mut tris, &mesh.translated([c[0], c[1], 0.0]), paint);
            // Entities do not pile up at where they come from or go to.
            if !matches!(paint, Paint::Source | Paint::Sink) {
                anchors.push(Anchor {
                    id,
                    order,
                    spot: Spot::Block { centre: c, half },
                });
            }
        }
    }
    let links: Vec<Segment> = model
        .connections
        .iter()
        .filter(|k| !is_conveyor(&k.from) && !is_conveyor(&k.to))
        .filter_map(|k| {
            let (a, b) = (centre_of(&k.from)?, centre_of(&k.to)?);
            Some(([a[0], a[1], 0.01], [b[0], b[1], 0.01]))
        })
        .collect();
    let bounds = bounds_of(&tris, &links);
    Scene {
        grid: floor_grid(bounds),
        tris,
        links,
        anchors,
        bounds,
    }
}

/// The totes of a watched run: for each block, `inside(id)` entities, on a conveyor spread
/// evenly along it, at another block stacked beside it (at most [`MOST_AT_A_BLOCK`]). With them,
/// the numbers of those too many to draw, and where to write them.
pub(crate) fn totes(
    scene: &Scene,
    inside: &dyn Fn(&str) -> u64,
) -> (Vec<Tri>, Vec<(Vec3, String)>) {
    let base = tote(Tote { size: TOTE_SIZE });
    let mut tris = Vec::new();
    let mut labels = Vec::new();
    for a in &scene.anchors {
        let n = inside(&a.id);
        if n == 0 {
            continue;
        }
        let n = usize::try_from(n).unwrap_or(usize::MAX);
        let paint = Paint::Tote(a.order);
        match a.spot {
            Spot::Belt {
                start,
                angle,
                length,
            } => {
                let room = if length.is_finite() && length > 0.0 {
                    (libm::floorf(length / 0.6) as usize).max(1)
                } else {
                    0
                };
                let shown = n.min(room).min(MOST_ON_A_BELT);
                let turned = base.rotated_z(angle);
                let (s, c) = (libm::sinf(angle), libm::cosf(angle));
                for i in 0..shown {
                    let along = (i as f32 + 0.5) / shown as f32 * length;
                    let at = [start[0] + c * along, start[1] + s * along, BELT_HEIGHT];
                    push_mesh(&mut tris, &turned.translated(at), paint);
                }
                if n > shown && shown > 0 {
                    let mid = length / 2.0;
                    labels.push((
                        [start[0] + c * mid, start[1] + s * mid, BELT_HEIGHT + 0.6],
                        format!("+{}", n - shown),
                    ));
                }
            }
            Spot::Block { centre, half } => {
                let shown = n.min(MOST_AT_A_BLOCK);
                for i in 0..shown {
                    let (column, level) = (i / 5, i % 5);
                    let at = [
                        centre[0] + half + 0.45 + column as f32 * 0.65,
                        centre[1],
                        level as f32 * TOTE_SIZE[2],
                    ];
                    push_mesh(&mut tris, &base.translated(at), paint);
                }
                if n > shown {
                    labels.push((
                        [centre[0] + half + 0.45, centre[1], 5.0 * TOTE_SIZE[2] + 0.3],
                        format!("+{}", n - shown),
                    ));
                }
            }
        }
    }
    (tris, labels)
}

/// An orbit camera looking at `target` from `distance` metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Camera {
    pub target: Vec3,
    /// Turn around the vertical axis, in radians; -pi/2 looks along +y.
    pub yaw: f32,
    /// Height above the floor plane, in radians.
    pub pitch: f32,
    pub distance: f32,
}

impl Camera {
    /// A camera that sees all of `bounds`.
    pub(crate) fn fitted(bounds: (Vec3, Vec3)) -> Self {
        let (lo, hi) = bounds;
        let radius = (0.5 * length(sub(hi, lo))).max(1.0);
        Self {
            target: midpoint(lo, hi),
            yaw: -FRAC_PI_2 - 0.5,
            pitch: 0.6,
            distance: radius / libm::sinf(FOV / 2.0) * 1.1,
        }
    }

    /// Turns the camera around its target.
    pub(crate) fn orbit(&mut self, yaw: f32, pitch: f32) {
        self.yaw += yaw;
        self.pitch = (self.pitch + pitch).clamp(0.05, 1.5);
    }

    /// Moves the camera nearer (`factor` below 1) or further.
    pub(crate) fn zoom(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.distance = (self.distance * factor).clamp(0.5, 5000.0);
        }
    }

    /// Slides the target over the floor as the scene follows a drag of `dx`, `dy` points in a
    /// view `height` points tall.
    pub(crate) fn pan(&mut self, dx: f32, dy: f32, height: f32) {
        let per_point = 2.0 * self.distance * libm::tanf(FOV / 2.0) / height.max(1.0);
        let basis = self.basis();
        let ahead = normalized([basis.forward[0], basis.forward[1], 0.0]);
        let shift = add(scale(basis.right, -dx), scale(ahead, dy));
        self.target = add(self.target, scale(shift, per_point));
    }

    fn basis(&self) -> Basis {
        let (sp, cp) = (libm::sinf(self.pitch), libm::cosf(self.pitch));
        let (sy, cy) = (libm::sinf(self.yaw), libm::cosf(self.yaw));
        let offset = [
            self.distance * cp * cy,
            self.distance * cp * sy,
            self.distance * sp,
        ];
        let forward = normalized(scale(offset, -1.0));
        let right = normalized(cross(forward, [0.0, 0.0, 1.0]));
        Basis {
            eye: add(self.target, offset),
            right,
            up: cross(right, forward),
            forward,
        }
    }

    /// The camera's projection onto `rect`.
    pub(crate) fn view(&self, rect: Rect) -> View {
        View {
            basis: self.basis(),
            centre: rect.center(),
            focal: rect.height() / 2.0 / libm::tanf(FOV / 2.0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Basis {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    forward: Vec3,
}

/// A triangle on screen, to be painted.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Face {
    pub pts: [Pos2; 3],
    /// Distance from the camera along its view direction, the mean of the corners'.
    pub depth: f32,
    /// How much of its colour the light leaves, 0.35 to 1.
    pub shade: f32,
    pub paint: Paint,
}

/// A camera's projection onto a rectangle of the screen.
#[derive(Debug, Clone, Copy)]
pub(crate) struct View {
    basis: Basis,
    centre: Pos2,
    focal: f32,
}

impl View {
    /// `p` in the camera's frame: right, up and distance along the view direction.
    fn to_view(&self, p: Vec3) -> Vec3 {
        let v = sub(p, self.basis.eye);
        [
            dot(v, self.basis.right),
            dot(v, self.basis.up),
            dot(v, self.basis.forward),
        ]
    }

    fn to_screen(&self, v: Vec3) -> Pos2 {
        pos2(
            self.centre.x + v[0] / v[2] * self.focal,
            self.centre.y - v[1] / v[2] * self.focal,
        )
    }

    /// Where the point is on screen; none when it is behind the near plane.
    pub(crate) fn project(&self, p: Vec3) -> Option<Pos2> {
        let v = self.to_view(p);
        (v[2] >= NEAR).then(|| self.to_screen(v))
    }

    /// The part of the segment in front of the near plane, on screen.
    pub(crate) fn segment(&self, a: Vec3, b: Vec3) -> Option<[Pos2; 2]> {
        let (mut va, mut vb) = (self.to_view(a), self.to_view(b));
        if va[2] < NEAR && vb[2] < NEAR {
            return None;
        }
        let cut = |near: Vec3, far: Vec3| {
            let k = (NEAR - near[2]) / (far[2] - near[2]);
            [
                near[0] + (far[0] - near[0]) * k,
                near[1] + (far[1] - near[1]) * k,
                NEAR,
            ]
        };
        if va[2] < NEAR {
            va = cut(va, vb);
        } else if vb[2] < NEAR {
            vb = cut(vb, va);
        }
        Some([self.to_screen(va), self.to_screen(vb)])
    }

    /// The triangles of `parts` that face the camera and lie in front of it, shaded, far to
    /// near.
    pub(crate) fn faces(&self, parts: &[&[Tri]]) -> Vec<Face> {
        let light = normalized([0.35, -0.45, 0.82]);
        let mut out = Vec::new();
        for tri in parts.iter().flat_map(|p| p.iter()) {
            if dot(tri.normal, sub(self.basis.eye, tri.v[0])) <= 0.0 {
                continue;
            }
            let corners = tri.v.map(|p| self.to_view(p));
            if corners.iter().any(|c| c[2] < NEAR) {
                continue;
            }
            out.push(Face {
                pts: corners.map(|c| self.to_screen(c)),
                depth: (corners[0][2] + corners[1][2] + corners[2][2]) / 3.0,
                shade: 0.35 + 0.65 * dot(tri.normal, light).max(0.0),
                paint: tri.paint,
            });
        }
        out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        out
    }
}

/// `colour` lit by `shade`.
fn shaded(colour: Color32, shade: f32) -> Color32 {
    let k = |c: u8| (f32::from(c) * shade).clamp(0.0, 255.0) as u8;
    Color32::from_rgb(k(colour.r()), k(colour.g()), k(colour.b()))
}

/// The 3D view's state: the camera (none until the first frame fits it) and the scene of the
/// model revision it was built for.
#[derive(Default)]
pub(crate) struct View3d {
    camera: Option<Camera>,
    scene: Option<(u64, Scene)>,
}

fn paint_colour(ctx: &egui::Context, p: &theme::Palette, paint: Paint) -> Color32 {
    match paint {
        Paint::Source => p.accent,
        Paint::Sink => p.chart,
        Paint::Machine => p.control_border,
        Paint::Delay => p.border,
        Paint::Gateway => p.warning,
        Paint::Batch => p.chart_hover,
        Paint::Belt => p.text_weak,
        Paint::Tote(i) => chart_palette::series_color(ctx, i),
    }
}

impl App {
    /// The 3D view, filling the work area.
    pub(crate) fn view3d_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let revision = self.engine.revision();
        if !matches!(&self.view3d.scene, Some((r, _)) if *r == revision) {
            self.view3d.scene = Some((revision, scene(self.engine.model())));
        }
        let watching = self.watching.as_ref();
        let View3d { camera, scene } = &mut self.view3d;
        let Some((_, scene)) = scene else { return };

        let mut fit = false;
        ui.horizontal(|ui| {
            fit = ui.button(t("Fit")).clicked();
        });
        ui.add_space(4.0);
        let size = ui.available_size().max(vec2(120.0, 120.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Canvas, true, t("3D view of the model"))
        });
        let hint = t("Drag to orbit · Scroll to zoom · Shift-drag to pan");
        ui.ctx()
            .accesskit_node_builder(resp.id, |n| n.set_description(hint));

        let cam = camera.get_or_insert_with(|| Camera::fitted(scene.bounds));
        if fit {
            *cam = Camera::fitted(scene.bounds);
        }
        if resp.dragged() {
            let d = resp.drag_delta();
            let pan = resp.dragged_by(egui::PointerButton::Secondary)
                || resp.dragged_by(egui::PointerButton::Middle)
                || ui.input(|i| i.modifiers.shift);
            if pan {
                cam.pan(d.x, d.y, rect.height());
            } else {
                cam.orbit(-d.x * 0.01, d.y * 0.01);
            }
        }
        if resp.hovered() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if scroll != 0.0 {
                cam.zoom(libm::expf(-scroll * 0.0015));
            }
            if pinch != 1.0 {
                cam.zoom(1.0 / pinch);
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
            let turn = |on: bool, by: f32| if on { by } else { 0.0 };
            cam.orbit(
                turn(left, 0.08) - turn(right, 0.08),
                turn(up, 0.08) - turn(down, 0.08),
            );
            if closer {
                cam.zoom(0.9);
            }
            if further {
                cam.zoom(1.1);
            }
        }

        // The run being watched: its totes.
        let (live_tris, labels) = match watching {
            Some(w) => totes(scene, &|id| {
                w.state(id).map_or(0, |s| {
                    s.entered.saturating_sub(s.left).saturating_sub(s.lost)
                })
            }),
            None => (Vec::new(), Vec::new()),
        };

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, p.canvas);
        let view = cam.view(rect);
        let grid = Stroke::new(1.0, p.border);
        for (a, b) in &scene.grid {
            if let Some(line) = view.segment(*a, *b) {
                painter.line_segment(line, grid);
            }
        }
        let link = Stroke::new(2.0, p.text_weak);
        for (a, b) in &scene.links {
            if let Some(line) = view.segment(*a, *b) {
                painter.line_segment(line, link);
            }
        }
        let faces = view.faces(&[&scene.tris, &live_tris]);
        for chunk in faces.chunks(CHUNK) {
            let mut mesh = egui::Mesh::default();
            for f in chunk {
                let colour = shaded(paint_colour(ui.ctx(), &p, f.paint), f.shade);
                let first = mesh.vertices.len() as u32;
                for pt in f.pts {
                    mesh.colored_vertex(pt, colour);
                }
                mesh.add_triangle(first, first + 1, first + 2);
            }
            painter.add(Shape::mesh(mesh));
        }
        for (at, text) in labels {
            if let Some(pos) = view.project(at) {
                painter.text(
                    pos,
                    Align2::CENTER_BOTTOM,
                    text,
                    FontId::proportional(12.0),
                    p.text,
                );
            }
        }
        painter.text(
            rect.left_bottom() + vec2(8.0, -8.0),
            Align2::LEFT_BOTTOM,
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
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use stochcraft_model::{Bounds, Connection, Dist, Element};

    fn id(s: &str) -> ElementId {
        ElementId::unchecked(s)
    }

    /// A camera 10 m from the origin, in front of it and above.
    fn camera() -> Camera {
        Camera {
            target: [0.0, 0.0, 0.0],
            yaw: -FRAC_PI_2,
            pitch: 0.5,
            distance: 10.0,
        }
    }

    fn rect() -> Rect {
        Rect::from_min_size(pos2(10.0, 20.0), vec2(400.0, 300.0))
    }

    fn triangle(y: f32, normal: Vec3) -> Tri {
        Tri {
            v: [
                [-1.0, y - 1.0, 0.0],
                [1.0, y - 1.0, 0.0],
                [0.0, y + 1.0, 0.0],
            ],
            normal,
            paint: Paint::Belt,
        }
    }

    #[test]
    fn the_target_projects_to_the_middle_of_the_rect() {
        let cam = Camera {
            target: [1.0, 2.0, 0.5],
            ..camera()
        };
        let at = cam
            .view(rect())
            .project([1.0, 2.0, 0.5])
            .expect("in front of the camera");
        assert!((at - rect().center()).length() < 1e-3, "{at:?}");
    }

    #[test]
    fn a_point_behind_the_camera_does_not_project() {
        let cam = camera();
        let behind = add(cam.basis().eye, scale(cam.basis().forward, -1.0));
        assert!(cam.view(rect()).project(behind).is_none());
    }

    #[test]
    fn a_triangle_facing_away_is_culled() {
        let view = camera().view(rect());
        let facing = triangle(0.0, [0.0, 0.0, 1.0]);
        let away = triangle(0.0, [0.0, 0.0, -1.0]);
        assert_eq!(view.faces(&[&[facing]]).len(), 1);
        assert!(view.faces(&[&[away]]).is_empty());
    }

    #[test]
    fn the_farther_triangle_is_painted_first() {
        let view = camera().view(rect());
        // The camera is on the -y side, so y = 5 is far and y = -5 near; near is listed first.
        let near = triangle(-5.0, [0.0, 0.0, 1.0]);
        let far = triangle(5.0, [0.0, 0.0, 1.0]);
        let faces = view.faces(&[&[near, far]]);
        assert_eq!(faces.len(), 2);
        assert!(faces[0].depth > faces[1].depth, "{faces:?}");
    }

    /// Source, conveyor and sink side by side, 400 drawing units (8 m) from the source's centre
    /// to the sink's.
    fn line_model() -> Model {
        let mut m = Model::default();
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
                length: 10.0,
                speed: 1.0,
                accumulating: false,
                item_length: 0.5,
                gap: 0.1,
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
        m.connections
            .push(Connection::new(id("c1"), id("in"), id("belt")));
        m.connections
            .push(Connection::new(id("c2"), id("belt"), id("out")));
        for (name, x) in [("in", 0.0), ("belt", 200.0), ("out", 400.0)] {
            m.layout.insert(
                id(name),
                Bounds {
                    x,
                    y: 0.0,
                    width: 100.0,
                    height: 80.0,
                },
            );
        }
        m
    }

    #[test]
    fn a_conveyor_spans_the_centres_of_its_neighbours() {
        let s = scene(&line_model());
        let xs: Vec<f32> = s
            .tris
            .iter()
            .filter(|t| t.paint == Paint::Belt)
            .flat_map(|t| t.v)
            .map(|v| v[0])
            .collect();
        assert!(!xs.is_empty());
        let lo = xs.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        // The centres are at x = 50 and 450 drawing units: 1 m and 9 m.
        assert!((lo - 1.0).abs() < 1e-3, "{lo}");
        assert!((hi - lo - 8.0).abs() < 1e-3, "{}", hi - lo);
        // Both ends of the model are drawn too, and no floor line joins what a belt joins.
        assert!(s.tris.iter().any(|t| t.paint == Paint::Source));
        assert!(s.tris.iter().any(|t| t.paint == Paint::Sink));
        assert!(s.links.is_empty());
    }

    #[test]
    fn totes_are_spread_along_the_conveyor() {
        let s = scene(&line_model());
        let (tris, labels) = totes(&s, &|id| u64::from(id == "belt") * 3);
        assert!(labels.is_empty());
        assert!(!tris.is_empty());
        assert!(tris.iter().all(|t| matches!(t.paint, Paint::Tote(_))));
        assert!(
            tris.iter()
                .flat_map(|t| t.v)
                .all(|v| v[0] > 1.0 - 0.5 && v[0] < 9.0 + 0.5)
        );
    }
}
