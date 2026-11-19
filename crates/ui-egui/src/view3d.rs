//! The 3D view (M5.3): the model's blocks at their layout positions, as the low-poly shapes of
//! `stochcraft-render`, drawn without any GPU code. Each frame the triangles are projected on the
//! CPU (an orbit camera with a perspective projection), back faces are dropped, the rest are
//! shaded flat by one directional light, sorted far to near (the painter's algorithm) and handed
//! to egui as meshes, so the same code draws in the desktop window and in the browser.
//!
//! The scene (every block's mesh in world space) is built once per model revision. While a run is
//! watched, each item on a conveyor is a tote where the run has carried it, coloured by its
//! product; at the other blocks, totes stand for the number of entities inside.
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
/// The least length of belt between two blocks; closer than that, the belt runs between their
/// centres instead.
const LEAST_BELT: f32 = 0.2;
/// How far apart lanes that leave (or enter) one block are, sideways.
const LANE_PITCH: f32 = 0.4;
/// The narrowest and widest a loop of belt is (the gap between its two arms' centre lines).
const LOOP_WIDTH: (f32, f32) = (0.8, 1.2);
/// How much a loop's corners are cut off, along each arm.
const LOOP_CUT: f32 = 0.3;
/// The shortest and longest a loop of belt is drawn, in metres.
const LOOP_LENGTH: (f32, f32) = (3.0, 60.0);
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
#[derive(Debug, Clone)]
pub(crate) enum Spot {
    /// Beside a block: its centre on the floor and half its footprint.
    Block { centre: [f32; 2], half: f32 },
    /// Along a conveyor: the corners of its centre line on the floor (two for a straight belt,
    /// more for a loop) and the length of that line in metres.
    Path { points: Vec<[f32; 2]>, length: f32 },
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
    for tri in mesh.indices.as_chunks::<3>().0 {
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

/// The block a conveyor takes from (`upstream`) or hands to: the other end of its first
/// connection in that direction.
fn neighbour<'m>(model: &'m Model, conveyor: &ElementId, upstream: bool) -> Option<&'m ElementId> {
    model.connections.iter().find_map(|k| {
        if upstream {
            (k.to == *conveyor).then_some(&k.from)
        } else {
            (k.from == *conveyor).then_some(&k.to)
        }
    })
}

/// The distance between two points on the floor.
fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    libm::hypotf(b[0] - a[0], b[1] - a[1])
}

/// The length of the polyline through `points`.
fn path_length(points: &[[f32; 2]]) -> f32 {
    points
        .windows(2)
        .map(|w| match w {
            [a, b] => dist(*a, *b),
            _ => 0.0,
        })
        .sum()
}

/// Where `distance` metres along `points` is (clamped to the ends), and the heading of the
/// segment there.
fn along_path(points: &[[f32; 2]], distance: f32) -> ([f32; 2], f32) {
    let mut left = distance.max(0.0);
    let mut at = points.first().copied().unwrap_or([0.0, 0.0]);
    let mut angle = 0.0;
    for w in points.windows(2) {
        let &[a, b] = w else { continue };
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let seg = libm::hypotf(dx, dy);
        if seg <= 1e-6 {
            continue;
        }
        angle = libm::atan2f(dy, dx);
        if left <= seg {
            let k = left / seg;
            return ([a[0] + dx * k, a[1] + dy * k], angle);
        }
        left -= seg;
        at = b;
    }
    (at, angle)
}

/// Adds a belt along each segment of `points` to `out`.
fn push_path(out: &mut Vec<Tri>, points: &[[f32; 2]]) {
    for w in points.windows(2) {
        let &[a, b] = w else { continue };
        let seg = dist(a, b);
        if seg <= 1e-3 {
            continue;
        }
        let mesh = straight_conveyor(StraightConveyor {
            length: seg,
            ..StraightConveyor::default()
        });
        let angle = libm::atan2f(b[1] - a[1], b[0] - a[0]);
        let placed = mesh.rotated_z(angle).translated([a[0], a[1], 0.0]);
        push_mesh(out, &placed, Paint::Belt);
    }
}

/// How far a ray from `p` along `d` goes before it leaves the square of half-side `half` around
/// `centre` (0 when `p` is outside it on that side).
fn leave_square(centre: [f32; 2], half: f32, p: [f32; 2], d: [f32; 2]) -> f32 {
    let axis = |c: f32, p: f32, d: f32| {
        if d.abs() > 1e-9 {
            ((c + half * d.signum() - p) / d).max(0.0)
        } else {
            f32::INFINITY
        }
    };
    let t = axis(centre[0], p[0], d[0]).min(axis(centre[1], p[1], d[1]));
    if t.is_finite() { t } else { 0.0 }
}

/// The two ends of a straight conveyor from the block at `a` (half-side `half_a`) to the one at
/// `b`: where the line between the centres, shifted sideways by `side_a` at `a` and `side_b` at
/// `b`, leaves the first footprint and enters the second. When the footprints leave less than
/// [`LEAST_BELT`] between them, the centres themselves.
fn edge_to_edge(
    (a, half_a, side_a): ([f32; 2], f32, f32),
    (b, half_b, side_b): ([f32; 2], f32, f32),
) -> [[f32; 2]; 2] {
    let apart = dist(a, b);
    if apart <= 1e-6 {
        return [a, b];
    }
    let d = [(b[0] - a[0]) / apart, (b[1] - a[1]) / apart];
    let n = [-d[1], d[0]];
    let pa = [a[0] + n[0] * side_a, a[1] + n[1] * side_a];
    let pb = [b[0] + n[0] * side_b, b[1] + n[1] * side_b];
    let span = dist(pa, pb);
    let dir = if span > 1e-6 {
        [(pb[0] - pa[0]) / span, (pb[1] - pa[1]) / span]
    } else {
        d
    };
    let out = leave_square(a, half_a, pa, dir);
    let back = leave_square(b, half_b, pb, [-dir[0], -dir[1]]);
    if span - out - back < LEAST_BELT {
        return [a, b];
    }
    [
        [pa[0] + dir[0] * out, pa[1] + dir[1] * out],
        [pb[0] - dir[0] * back, pb[1] - dir[1] * back],
    ]
}

/// A loop of belt of `total` metres that leaves the block at `centre` (half-side `half`) on the
/// side that `toward` lies on, goes out, round two chamfered corners and back to the same face a
/// belt's width or so along: five straight pieces beside the block, which closes the ring.
fn loop_path(centre: [f32; 2], half: f32, toward: [f32; 2], total: f32) -> Vec<[f32; 2]> {
    let (dx, dy) = (toward[0] - centre[0], toward[1] - centre[1]);
    let out = if dx.abs() >= dy.abs() {
        [if dx >= 0.0 { 1.0 } else { -1.0 }, 0.0]
    } else {
        [0.0, if dy >= 0.0 { 1.0 } else { -1.0 }]
    };
    let side = [-out[1], out[0]];
    let width = (2.0 * half * 0.9).clamp(LOOP_WIDTH.0, LOOP_WIDTH.1);
    let cut = LOOP_CUT;
    // Two sides of `depth` and the width between them, less 2 * (2 - sqrt 2) * cut for the corners.
    let depth = (total - width + 2.0 * (2.0 - core::f32::consts::SQRT_2) * cut) / 2.0;
    let at = |across: f32, up: f32| {
        [
            centre[0] + out[0] * (half + up) + side[0] * across,
            centre[1] + out[1] * (half + up) + side[1] * across,
        ]
    };
    let h = width / 2.0;
    vec![
        at(-h, 0.0),
        at(-h, depth - cut),
        at(-h + cut, depth),
        at(h - cut, depth),
        at(h, depth - cut),
        at(h, 0.0),
    ]
}

/// The model in world space: each block's shape at its layout position, a conveyor as a belt
/// from the edge of the shape before it to the edge of the shape after it (lanes that leave or
/// enter one block side by side start at different points of its edge), and a line on the floor
/// for each connection that no conveyor carries. A conveyor that returns to the block it leaves
/// is a loop beside that block, as long as the conveyor's own length (within limits). The
/// length of a conveyor with a neighbour on only one side, or none, is the model's own, in
/// metres, along `+x` from its centre.
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
            let before = neighbour(model, &e.id, true).and_then(|f| Some((f, centre_of(f)?)));
            let after = neighbour(model, &e.id, false).and_then(|t| Some((t, centre_of(t)?)));
            // Half the footprint of a block; a neighbouring conveyor has none, the belts meet
            // at its middle.
            let half_of = |id: &ElementId| {
                model.element(id).map_or(0.0, |n| {
                    if matches!(n.kind, ElementKind::Conveyor { .. }) {
                        0.0
                    } else {
                        block_shape(&n.kind).2
                    }
                })
            };
            // The sideways shift of this lane among the conveyors that leave `block` (or, when
            // `upstream` is false, enter it).
            let lane = |block: &ElementId, upstream: bool| {
                let mates: Vec<&ElementId> = model
                    .elements
                    .iter()
                    .filter(|m| matches!(m.kind, ElementKind::Conveyor { .. }))
                    .filter(|m| neighbour(model, &m.id, upstream) == Some(block))
                    .map(|m| &m.id)
                    .collect();
                let at = mates.iter().position(|m| **m == e.id).unwrap_or(0);
                (at as f32 - (mates.len() as f32 - 1.0) / 2.0) * LANE_PITCH
            };
            let own_end = [c[0] + *len as f32 * metres, c[1]];
            let points: Vec<[f32; 2]> = match (before, after) {
                (Some((f, a)), Some((t, b))) if f == t || dist(a, b) <= 0.05 => {
                    let want = *len as f32 * metres;
                    let total = if want.is_finite() {
                        want.clamp(LOOP_LENGTH.0, LOOP_LENGTH.1)
                    } else {
                        LOOP_LENGTH.0
                    };
                    loop_path(a, half_of(f), c, total)
                }
                (Some((f, a)), Some((t, b))) => {
                    let (ha, hb) = (half_of(f), half_of(t));
                    let side = |h: f32, s: f32| s.clamp(-0.6 * h, 0.6 * h);
                    edge_to_edge(
                        (a, ha, side(ha, lane(f, true))),
                        (b, hb, side(hb, lane(t, false))),
                    )
                    .to_vec()
                }
                _ => vec![c, own_end],
            };
            let belt_length = path_length(&points);
            push_path(&mut tris, &points);
            anchors.push(Anchor {
                id,
                order,
                spot: Spot::Path {
                    points,
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

/// The items on a conveyor: how far along each is (0 to 1) and its product key.
pub(crate) type BeltItems = Vec<(f32, u32)>;

/// The totes of a watched run. On a conveyor, one per item of `on_belt(id)` at its place along
/// the belt and in its product's colour; when that is `None`, `inside(id)` totes spread evenly.
/// At another block, `inside(id)` totes stacked beside it (at most [`MOST_AT_A_BLOCK`]). With
/// them, the numbers of those too many to draw, and where to write them.
pub(crate) fn totes(
    scene: &Scene,
    inside: &dyn Fn(&str) -> u64,
    on_belt: &dyn Fn(&str) -> Option<BeltItems>,
) -> (Vec<Tri>, Vec<(Vec3, String)>) {
    let base = tote(Tote { size: TOTE_SIZE });
    let mut tris = Vec::new();
    let mut labels = Vec::new();
    for a in &scene.anchors {
        if let Spot::Path { points, length } = &a.spot
            && let Some(items) = on_belt(&a.id)
        {
            for &(fraction, product) in items.iter().take(MOST_ON_A_BELT) {
                let (at, angle) = along_path(points, fraction.clamp(0.0, 1.0) * length);
                let paint = Paint::Tote(usize::try_from(product).unwrap_or(0));
                let placed = base
                    .rotated_z(angle)
                    .translated([at[0], at[1], BELT_HEIGHT]);
                push_mesh(&mut tris, &placed, paint);
            }
            if items.len() > MOST_ON_A_BELT {
                let (mid, _) = along_path(points, length / 2.0);
                labels.push((
                    [mid[0], mid[1], BELT_HEIGHT + 0.6],
                    format!("+{}", items.len() - MOST_ON_A_BELT),
                ));
            }
            continue;
        }
        let n = inside(&a.id);
        if n == 0 {
            continue;
        }
        let n = usize::try_from(n).unwrap_or(usize::MAX);
        let paint = Paint::Tote(a.order);
        match &a.spot {
            Spot::Path { points, length } => {
                let room = if length.is_finite() && *length > 0.0 {
                    (libm::floorf(length / 0.6) as usize).max(1)
                } else {
                    0
                };
                let shown = n.min(room).min(MOST_ON_A_BELT);
                for i in 0..shown {
                    let along = (i as f32 + 0.5) / shown as f32 * length;
                    let (at, angle) = along_path(points, along);
                    let placed = base
                        .rotated_z(angle)
                        .translated([at[0], at[1], BELT_HEIGHT]);
                    push_mesh(&mut tris, &placed, paint);
                }
                if n > shown && shown > 0 {
                    let (mid, _) = along_path(points, length / 2.0);
                    labels.push((
                        [mid[0], mid[1], BELT_HEIGHT + 0.6],
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
    fn in_camera(&self, p: Vec3) -> Vec3 {
        let v = sub(p, self.basis.eye);
        [
            dot(v, self.basis.right),
            dot(v, self.basis.up),
            dot(v, self.basis.forward),
        ]
    }

    fn on_screen(&self, v: Vec3) -> Pos2 {
        pos2(
            self.centre.x + v[0] / v[2] * self.focal,
            self.centre.y - v[1] / v[2] * self.focal,
        )
    }

    /// Where the point is on screen; none when it is behind the near plane.
    pub(crate) fn project(&self, p: Vec3) -> Option<Pos2> {
        let v = self.in_camera(p);
        (v[2] >= NEAR).then(|| self.on_screen(v))
    }

    /// The part of the segment in front of the near plane, on screen.
    pub(crate) fn segment(&self, a: Vec3, b: Vec3) -> Option<[Pos2; 2]> {
        let (mut va, mut vb) = (self.in_camera(a), self.in_camera(b));
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
        Some([self.on_screen(va), self.on_screen(vb)])
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
            let corners = tri.v.map(|p| self.in_camera(p));
            if corners.iter().any(|c| c[2] < NEAR) {
                continue;
            }
            out.push(Face {
                pts: corners.map(|c| self.on_screen(c)),
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
            Some(w) => totes(
                scene,
                &|id| {
                    w.state(id).map_or(0, |s| {
                        s.entered.saturating_sub(s.left).saturating_sub(s.lost)
                    })
                },
                &|id| {
                    Some(
                        w.conveyor_items(id)
                            .into_iter()
                            .map(|(fraction, _, product)| (fraction, product))
                            .collect(),
                    )
                },
            ),
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
    use stochcraft_model::{Bounds, Connection, Dist, Element, Routing};

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

    fn conveyor(name: &str, length: f64) -> Element {
        Element::new(
            id(name),
            name,
            ElementKind::Conveyor {
                length,
                speed: 1.0,
                accumulating: false,
                item_length: 0.5,
                gap: 0.1,
            },
        )
    }

    fn sink(name: &str) -> Element {
        Element::new(
            id(name),
            name,
            ElementKind::Sink {
                scrap: None,
                demand: None,
            },
        )
    }

    fn gateway(name: &str) -> Element {
        Element::new(
            id(name),
            name,
            ElementKind::ExclusiveGateway {
                default: None,
                routing: Routing::Standard,
            },
        )
    }

    fn place(m: &mut Model, name: &str, (x, y, width, height): (f64, f64, f64, f64)) {
        m.layout.insert(
            id(name),
            Bounds {
                x,
                y,
                width,
                height,
            },
        );
    }

    fn connect(m: &mut Model, from: &str, to: &str) {
        m.connections.push(Connection::new(
            id(&format!("{from}-{to}")),
            id(from),
            id(to),
        ));
    }

    /// Source, conveyor and sink side by side, 400 drawing units (8 m) from the source's centre
    /// to the sink's, the sink `drop` drawing units lower.
    fn two_blocks(drop: f64) -> Model {
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
        m.elements.push(conveyor("belt", 10.0));
        m.elements.push(sink("out"));
        connect(&mut m, "in", "belt");
        connect(&mut m, "belt", "out");
        place(&mut m, "in", (0.0, 0.0, 100.0, 80.0));
        place(&mut m, "belt", (200.0, 0.0, 100.0, 80.0));
        place(&mut m, "out", (400.0, drop, 100.0, 80.0));
        m
    }

    fn line_model() -> Model {
        two_blocks(0.0)
    }

    /// A gateway whose conveyor leaves it and comes back, `length` metres long, with its own
    /// canvas position to the right of the gateway.
    fn loop_model(length: f64) -> Model {
        let mut m = Model::default();
        m.experiment.length_unit = LengthUnit::Metres;
        m.elements.push(gateway("g"));
        m.elements.push(conveyor("loop", length));
        connect(&mut m, "g", "loop");
        connect(&mut m, "loop", "g");
        place(&mut m, "g", (0.0, 0.0, 50.0, 50.0));
        place(&mut m, "loop", (200.0, 0.0, 100.0, 80.0));
        m
    }

    /// A conveyor's path and length in the scene.
    fn path_of(s: &Scene, name: &str) -> (Vec<[f32; 2]>, f32) {
        match s
            .anchors
            .iter()
            .find(|a| a.id == name)
            .map(|a| a.spot.clone())
        {
            Some(Spot::Path { points, length }) => (points, length),
            other => panic!("{name} has no path: {other:?}"),
        }
    }

    /// How far `p` is from `centre` in the largest coordinate: `half` on the edge of a square
    /// footprint of half-side `half`.
    fn reach(p: [f32; 2], centre: [f32; 2]) -> f32 {
        (p[0] - centre[0]).abs().max((p[1] - centre[1]).abs())
    }

    #[test]
    fn a_conveyor_runs_from_the_edge_of_the_shape_before_to_the_edge_of_the_one_after() {
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
        // The centres are at x = 50 and 450 drawing units: 1 m and 9 m. The source and the sink
        // are 0.9 m across, so the belt runs from 1 + 0.45 = 1.45 m to 9 - 0.45 = 8.55 m.
        assert!((lo - 1.45).abs() < 1e-3, "{lo}");
        assert!((hi - 8.55).abs() < 1e-3, "{hi}");
        // Both ends of the model are drawn too, and no floor line joins what a belt joins.
        assert!(s.tris.iter().any(|t| t.paint == Paint::Source));
        assert!(s.tris.iter().any(|t| t.paint == Paint::Sink));
        assert!(s.links.is_empty());
    }

    #[test]
    fn a_slanting_conveyor_starts_and_ends_on_the_edges_of_both_footprints() {
        // The sink is 200 units (4 m) lower: the centres are (1, -0.8) and (7, -4.8) m, 6 m
        // across and 4 m down. A ray along that line leaves the source's 0.45 m half-side at
        // 0.45 / 6 = 0.075 of the way: (1.45, -1.10); it enters the sink's at the same fraction
        // from the other end: (6.55, -4.50).
        let s = scene(&two_blocks(200.0));
        let (points, length) = path_of(&s, "belt");
        let (start, end) = (points[0], points[points.len() - 1]);
        let (from, to) = ([1.0, -0.8], [7.0, -4.8]);
        assert!(
            reach(start, from) >= 0.45 - 1e-4,
            "{start:?} is inside the source"
        );
        assert!(reach(end, to) >= 0.45 - 1e-4, "{end:?} is inside the sink");
        assert!(
            reach(start, from) <= 0.45 + 1e-4,
            "{start:?} is off the source"
        );
        assert!(reach(end, to) <= 0.45 + 1e-4, "{end:?} is off the sink");
        assert!((start[0] - 1.45).abs() < 1e-3 && (start[1] + 1.10).abs() < 1e-3);
        assert!((end[0] - 6.55).abs() < 1e-3 && (end[1] + 4.50).abs() < 1e-3);
        // 0.85 of the 2 * sqrt(13) = 7.211 m between the centres: 6.130 m.
        assert!((length - 0.85 * 52.0_f32.sqrt()).abs() < 1e-3, "{length}");
    }

    #[test]
    fn lanes_that_leave_one_shape_start_at_different_points_of_its_edge() {
        // A gateway (half-side 0.5 m) at (0.5, -0.5) and a sink (half-side 0.45 m) at (8, -0.5),
        // joined by two conveyors. They are shifted 0.2 m either side of the line between the
        // centres, so they start at (1.0, -0.7) and (1.0, -0.3) and end at (7.55, -0.7) and
        // (7.55, -0.3): 0.4 m apart at both ends.
        let mut m = Model::default();
        m.experiment.length_unit = LengthUnit::Metres;
        m.elements.push(gateway("g"));
        m.elements.push(conveyor("a", 5.0));
        m.elements.push(conveyor("b", 5.0));
        m.elements.push(sink("s"));
        for lane in ["a", "b"] {
            connect(&mut m, "g", lane);
            connect(&mut m, lane, "s");
            place(&mut m, lane, (200.0, 100.0, 100.0, 80.0));
        }
        place(&mut m, "g", (0.0, 0.0, 50.0, 50.0));
        place(&mut m, "s", (375.0, 0.0, 50.0, 50.0));
        let s = scene(&m);
        let (a, _) = path_of(&s, "a");
        let (b, _) = path_of(&s, "b");
        let first = |p: &[[f32; 2]]| p[0];
        let last = |p: &[[f32; 2]]| p[p.len() - 1];
        assert!((first(&a)[0] - 1.0).abs() < 1e-3 && (first(&b)[0] - 1.0).abs() < 1e-3);
        assert!((first(&a)[1] + 0.7).abs() < 1e-3, "{:?}", first(&a));
        assert!((first(&b)[1] + 0.3).abs() < 1e-3, "{:?}", first(&b));
        assert!((last(&a)[0] - 7.55).abs() < 1e-3 && (last(&b)[0] - 7.55).abs() < 1e-3);
        assert!((last(&a)[1] + 0.7).abs() < 1e-3, "{:?}", last(&a));
        assert!((last(&b)[1] + 0.3).abs() < 1e-3, "{:?}", last(&b));
    }

    #[test]
    fn a_conveyor_back_to_the_block_it_leaves_is_a_loop_beside_it() {
        // The gateway's centre is (0.5, -0.5) m with half-side 0.5; the conveyor's own centre
        // (5, -0.8) is to its right, so the loop leaves the right face (x = 1.0). The arms are
        // 2 * 0.5 * 0.9 = 0.9 m apart, so it starts at (1.0, -0.95) and ends at (1.0, -0.05).
        // With the 0.3 m corner cuts the arms are (8 - 0.9 + 2 * (2 - sqrt 2) * 0.3) / 2 = 3.7257
        // m deep.
        let s = scene(&loop_model(8.0));
        let (points, length) = path_of(&s, "loop");
        let centre = [0.5, -0.5];
        assert!(points.len() >= 4, "{points:?}");
        let (start, end) = (points[0], points[points.len() - 1]);
        assert!((reach(start, centre) - 0.5).abs() < 1e-4, "{start:?}");
        assert!((reach(end, centre) - 0.5).abs() < 1e-4, "{end:?}");
        assert!((start[1] + 0.95).abs() < 1e-3 && (end[1] + 0.05).abs() < 1e-3);
        // 2 * 3.4257 along the arms, 2 * 0.3 * sqrt 2 round the corners and 0.3 across: 8 m.
        assert!((length - 8.0).abs() < 1e-3, "{length}");
        assert!((path_length(&points) - 8.0).abs() < 1e-3);
        // All of it lies to the right of the gateway's face, out to 1.0 + 3.7257.
        let far = points.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
        assert!(points.iter().all(|p| p[0] >= 1.0 - 1e-4));
        assert!((far - 4.7257).abs() < 1e-3, "{far}");
        assert!(s.tris.iter().any(|t| t.paint == Paint::Belt));
    }

    #[test]
    fn a_loop_is_kept_between_three_and_sixty_metres() {
        // 0.5 m is drawn as 3 m and 1000 m as 60 m.
        assert!((path_of(&scene(&loop_model(0.5)), "loop").1 - 3.0).abs() < 1e-3);
        assert!((path_of(&scene(&loop_model(1000.0)), "loop").1 - 60.0).abs() < 1e-3);
    }

    #[test]
    fn totes_are_spread_along_the_conveyor() {
        let s = scene(&line_model());
        let (tris, labels) = totes(&s, &|id| u64::from(id == "belt") * 3, &|_| None);
        assert!(labels.is_empty());
        assert!(!tris.is_empty());
        assert!(tris.iter().all(|t| matches!(t.paint, Paint::Tote(_))));
        // The belt is 1.45 m to 8.55 m long, and a tote's centre is at least half its 0.5 m
        // length from the ends.
        assert!(
            tris.iter()
                .flat_map(|t| t.v)
                .all(|v| v[0] > 1.45 && v[0] < 8.55)
        );
    }

    /// The middle of the bounding box of the totes painted `paint`.
    fn tote_centre(tris: &[Tri], paint: Paint) -> [f32; 2] {
        let vs: Vec<Vec3> = tris
            .iter()
            .filter(|t| t.paint == paint)
            .flat_map(|t| t.v)
            .collect();
        assert!(!vs.is_empty(), "no tote painted {paint:?}");
        let mid = |axis: usize| {
            (vs.iter().map(|v| v[axis]).fold(f32::MAX, f32::min)
                + vs.iter().map(|v| v[axis]).fold(f32::MIN, f32::max))
                / 2.0
        };
        [mid(0), mid(1)]
    }

    #[test]
    fn items_on_a_belt_are_where_the_run_has_carried_them_in_their_products_colour() {
        let s = scene(&line_model());
        let (points, length) = path_of(&s, "belt");
        // The belt is 7.1 m long from x = 1.45 m.
        assert!((length - 7.1).abs() < 1e-3, "{length}");
        // One item at the start of the belt (product 0) and one three quarters along (product 2).
        let items = vec![(0.0, 0), (0.75, 2)];
        let (tris, labels) = totes(&s, &|_| 0, &|id| (id == "belt").then(|| items.clone()));
        assert!(labels.is_empty());
        // 1.45 + 0.75 * 7.1 = 6.775 m.
        assert!((tote_centre(&tris, Paint::Tote(0))[0] - points[0][0]).abs() < 1e-3);
        assert!((tote_centre(&tris, Paint::Tote(2))[0] - 6.775).abs() < 1e-3);
    }

    #[test]
    fn an_item_half_way_round_a_loop_is_at_the_half_way_point_of_its_path() {
        // The loop is 8 m long: 4 m in is 3.4257 m along the first arm and 0.4243 m round the
        // first corner (3.85 m), then 0.15 m into the 0.3 m across: (4.7257, -0.5), the point
        // furthest from the gateway's centre, which is (0.5, -0.5).
        let s = scene(&loop_model(8.0));
        let items = vec![(0.5, 1)];
        let (tris, _) = totes(&s, &|_| 0, &|id| (id == "loop").then(|| items.clone()));
        let at = tote_centre(&tris, Paint::Tote(1));
        assert!(
            (at[0] - 4.7257).abs() < 1e-3 && (at[1] + 0.5).abs() < 1e-3,
            "{at:?}"
        );
    }

    #[test]
    fn totes_without_positions_spread_evenly_round_a_loop() {
        // Two totes, at a quarter (2 m: 2 m along the first arm, at (3.0, -0.95)) and three
        // quarters (6 m: 1.4257 m along the way back, at (3.0, -0.05)) of the 8 m. A tote is
        // 0.5 m long and 0.4 m across and lies along the arms.
        let s = scene(&loop_model(8.0));
        let (tris, labels) = totes(&s, &|id| u64::from(id == "loop") * 2, &|_| None);
        assert!(labels.is_empty());
        let vs: Vec<Vec3> = tris.iter().flat_map(|t| t.v).collect();
        let range = |axis: usize| {
            (
                vs.iter().map(|v| v[axis]).fold(f32::MAX, f32::min),
                vs.iter().map(|v| v[axis]).fold(f32::MIN, f32::max),
            )
        };
        let (x, y) = (range(0), range(1));
        assert!(
            (x.0 - 2.75).abs() < 1e-3 && (x.1 - 3.25).abs() < 1e-3,
            "{x:?}"
        );
        assert!(
            (y.0 + 1.15).abs() < 1e-3 && (y.1 + 0.25).abs() < 1e-3,
            "{y:?}"
        );
    }
}
