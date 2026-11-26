//! The 3D view (M5.3, fed from the layout since M5.5's L3): the model's physical layout as
//! low-poly shapes built by code, drawn without any GPU code. Each frame the triangles are
//! projected on the CPU (an orbit camera with a perspective projection), back faces are dropped,
//! the rest are shaded flat by a key light and a sky fill, sorted far to near (the painter's
//! algorithm) and handed to egui as meshes, so the same code draws in the desktop window and in
//! the browser.
//!
//! The scene is built once per model revision from the model's [`stochcraft_model::Site`]: each
//! conveyor is a belt with side rails and legs swept along its track's path (a rail leaves a gap
//! where another belt leaves it or joins it), each placed block is its kind's shape at its place
//! ([`look_of`]), and the scenery is drawn as its kind says. A block with no place is unplaced:
//! it is listed, never drawn, and nothing is guessed from the flow diagram. While a run is
//! watched, each item on a conveyor is a parcel at `point_at(s)` on its path, coloured by its
//! product; at the other blocks, parcels stand for the number of entities inside.
//!
//! Conventions: metres, `z` up, `x` and `y` as the layout's.

use core::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use egui::{
    Align2, Color32, FontId, Key, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2,
};
use stochcraft_model::{
    Element, ElementId, ElementKind, Frame, LengthUnit, Model, Path, Place, Segment as PathSegment,
};
use stochcraft_random::math::{cos, sin};
use stochcraft_render::kit::{self, Station};
use stochcraft_render::{Mesh, box_mesh};

use crate::i18n::t;
use crate::{App, chart_palette, theme};

/// The vertical field of view.
const FOV: f32 = FRAC_PI_4;
/// Triangles with a corner nearer than this to the camera (along its view direction) are dropped.
const NEAR: f32 = 0.1;
/// Triangles with an edge longer than this are split, so the painter's sort puts a long belt
/// and the parcels on it in the right order.
const MAX_EDGE: f32 = 1.5;
/// The height of a belt's top above the floor (plus its path's own height).
pub(crate) const BELT_HEIGHT: f32 = 0.85;
/// The side rails' width, which a belt's outline includes.
pub(crate) const RAIL: f32 = 0.07;
/// The longest step between two cross-sections of a belt, in metres.
const STEP: f64 = 0.25;
/// The most cross-sections [`stations`] makes of a path, past its start and one for each
/// segment: a longer path has its steps stretched, so one hundreds of kilometres long is drawn
/// at no more cost.
const MOST_STATIONS: f64 = 4096.0;
/// The most parcels drawn at a block; more are counted by a number.
const MOST_AT_A_BLOCK: usize = 3;
/// The most parcels drawn on one conveyor.
const MOST_ON_A_BELT: usize = 100;
/// Triangles to an egui mesh, so a mesh never has more vertices than 16-bit indices reach.
const CHUNK: usize = 18_000;
/// Parcel sizes (length along the belt, width, height), chosen by an item's number.
const PARCELS: [[f32; 3]; 3] = [[0.36, 0.3, 0.22], [0.38, 0.34, 0.28], [0.32, 0.24, 0.16]];

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
    /// The floor slab.
    Floor,
    /// A soft shadow on the floor under an object.
    Shadow,
    /// Safety markings on the floor and accent stripes.
    Marking,
    /// A belt's moving surface.
    Belt,
    /// Conveyor frames and rails, arches.
    Frame,
    /// A lane's rails, or a station's band, in the colour its name says.
    Lane([u8; 3]),
    /// Legs, posts and other bare steel.
    Steel,
    /// Work tops and pallets.
    Wood,
    /// Cartons.
    Cardboard,
    /// Parcel tape.
    Tape,
    /// Screens' and cabinets' housings.
    Housing,
    /// A screen's face, a scanner's window.
    Glow,
    /// A person's hi-vis vest.
    Person,
    /// A person's face.
    Skin,
    /// A stack light's lamp: green, amber, red.
    Lamp(u8),
    /// A parcel, by the order of the block it is at.
    Tote(usize),
    /// A parcel of a product named after a colour ("red", "Blue parcel"), in that colour.
    Named([u8; 3]),
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
    /// At a block: the point on its top where parcels stand.
    Block { centre: [f32; 2], top: f32 },
    /// Along a conveyor: its path in metres.
    Path { path: Path },
}

/// A block of the model that can hold entities, in the scene.
#[derive(Debug, Clone)]
pub(crate) struct Anchor {
    pub id: String,
    /// The block's place in the model, for its parcels' colour.
    pub order: usize,
    pub spot: Spot,
}

/// The model in world space.
#[derive(Debug, Clone, Default)]
pub(crate) struct Scene {
    /// The floor, its markings and the shadows on it, drawn first in this order.
    pub floor: Vec<Tri>,
    pub tris: Vec<Tri>,
    /// The floor grid.
    pub grid: Vec<Segment>,
    pub anchors: Vec<Anchor>,
    /// The corners of the box round everything standing on the floor, which Fit frames; the
    /// floor's own box when nothing stands on it.
    pub bounds: (Vec3, Vec3),
    /// For each product key, the colour its name says, if it names one.
    pub product_colours: Vec<Option<[u8; 3]>>,
    /// The names of the blocks with no place, which are not drawn.
    pub unplaced: Vec<String>,
}

impl Scene {
    /// Whether there is nothing to draw: no block, belt or scenery, not even a floor.
    pub(crate) fn is_empty(&self) -> bool {
        self.tris.is_empty() && self.floor.is_empty()
    }
}

/// The colour a name says, as the first colour word in it: "red", "Lane blue".
pub(crate) fn named_colour(name: &str) -> Option<[u8; 3]> {
    const NAMED: [(&str, [u8; 3]); 11] = [
        ("red", [214, 48, 49]),
        ("blue", [41, 109, 204]),
        ("green", [46, 158, 76]),
        ("yellow", [242, 196, 33]),
        ("orange", [240, 135, 30]),
        ("purple", [136, 84, 208]),
        ("pink", [232, 106, 166]),
        ("brown", [140, 90, 50]),
        ("black", [45, 45, 48]),
        ("white", [236, 236, 232]),
        ("grey", [140, 140, 145]),
    ];
    let lower = name.to_lowercase().replace("gray", "grey");
    lower
        .split(|c: char| !c.is_alphanumeric())
        .find_map(|w| NAMED.iter().find(|(n, _)| *n == w).map(|(_, c)| *c))
}

/// Metres in one of `unit`.
pub(crate) fn unit_metres(unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::Millimetres => 0.001,
        LengthUnit::Centimetres => 0.01,
        LengthUnit::Metres => 1.0,
        LengthUnit::Kilometres => 1000.0,
        LengthUnit::Inches => 0.0254,
        LengthUnit::Feet => 0.3048,
    }
}

/// Conveyor `id`'s track in metres: its path, and the belt's width. `None` for an element with
/// no track.
pub(crate) fn track_metres(model: &Model, id: &ElementId) -> Option<(Path, f32)> {
    let m = unit_metres(model.experiment.length_unit);
    let t = model.site.paths.get(id)?;
    Some((t.path.scaled(m), (t.width * m) as f32))
}

/// Where block `e` stands, in metres: its place's frame. A block placed along a conveyor (a
/// divert) stands beside the belt, on its left, rather than on it. A conveyor's frame is the
/// start of its track. `None` for a block with no place.
pub(crate) fn frame_metres(model: &Model, e: &Element) -> Option<Frame> {
    let m = unit_metres(model.experiment.length_unit);
    let f = model.site.frame_of(&e.id)?;
    let mut f = Frame::new(f.x * m, f.y * m, f.z * m, f.heading_deg);
    if let Some(Place::Along { path, .. }) = model.site.places.get(&e.id) {
        let width = track_metres(model, path).map_or(0.6, |(_, w)| f64::from(w));
        let side = width / 2.0 + f64::from(RAIL) + 0.45;
        let h = f.heading_deg.to_radians();
        f.x -= sin(h) * side;
        f.y += cos(h) * side;
    }
    Some(f)
}

/// For block `e` placed along a conveyor, the spot on that belt's top at its point, in metres;
/// `None` for a block placed otherwise.
fn on_belt_at(model: &Model, e: &Element) -> Option<Spot> {
    if !matches!(model.site.places.get(&e.id), Some(Place::Along { .. })) {
        return None;
    }
    let m = unit_metres(model.experiment.length_unit);
    let f = model.site.frame_of(&e.id)?;
    Some(Spot::Block {
        centre: [(f.x * m) as f32, (f.y * m) as f32],
        top: (f.z * m) as f32 + BELT_HEIGHT,
    })
}

/// A block's look: meshes in its own frame (standing on the floor at the origin, facing `+x`),
/// each with its paint, the size of its footprint (along `x`, along `y`), and the height its
/// entities are drawn at.
pub(crate) struct Look {
    pub parts: Vec<(Mesh, Paint)>,
    pub footprint: [f32; 2],
    pub top: f32,
}

/// What block `e` looks like. This is the one place a block's shape comes from: today a
/// generator for its kind, built by code from its parameters.
pub(crate) fn look_of(e: &Element) -> Look {
    match &e.kind {
        ElementKind::Source { .. } => {
            let (wood, cartons) = kit::pallet(kit::Pallet::default());
            Look {
                parts: vec![(wood, Paint::Wood), (cartons, Paint::Cardboard)],
                footprint: [1.2, 0.8],
                top: 0.7,
            }
        }
        ElementKind::Sink { .. } => {
            let (frame, base) = kit::roll_cage(kit::RollCage::default());
            Look {
                parts: vec![(frame, Paint::Steel), (base, Paint::Frame)],
                footprint: [0.8, 0.7],
                top: 0.2,
            }
        }
        ElementKind::Server { .. } | ElementKind::SharedTask { .. } => {
            let s = kit::pack_station(kit::PackStation::default());
            let mut parts = vec![
                (s.frame, Paint::Steel),
                (s.top, Paint::Wood),
                (s.screen, Paint::Housing),
                (s.glow, Paint::Glow),
                (s.cartons, Paint::Cardboard),
            ];
            // The packer, behind the bench and facing it.
            let (torso, limbs, head) = kit::person(kit::Person::default());
            for (mesh, paint) in [
                (torso, Paint::Person),
                (limbs, Paint::Housing),
                (head, Paint::Skin),
            ] {
                parts.push((mesh.translated([0.75, 0.15, 0.0]), paint));
            }
            // A band along the bench's front in the colour its name says ("Pack red").
            if let Some(c) = named_colour(&e.name) {
                parts.push((
                    box_mesh([0.02, 1.3, 0.08]).translated([-0.41, 0.0, 0.8]),
                    Paint::Lane(c),
                ));
            }
            Look {
                parts,
                footprint: [0.8, 1.4],
                top: 0.9,
            }
        }
        ElementKind::ExclusiveGateway { .. } | ElementKind::ParallelGateway => {
            let (body, door, lamps) = kit::cabinet(kit::Cabinet::default());
            let mut parts = vec![(body, Paint::Housing), (door, Paint::Frame)];
            for (k, lamp) in lamps.into_iter().enumerate() {
                parts.push((lamp, Paint::Lamp(k as u8)));
            }
            Look {
                parts,
                footprint: [0.4, 0.6],
                top: 1.3,
            }
        }
        ElementKind::Delay { .. } => Look {
            parts: vec![(kit::bevel_box([1.2, 1.2, 0.08], 0.02), Paint::Frame)],
            footprint: [1.2, 1.2],
            top: 0.08,
        },
        _ => Look {
            parts: vec![(kit::bevel_box([0.9, 0.9, 0.7], 0.04), Paint::Housing)],
            footprint: [0.9, 0.9],
            top: 0.7,
        },
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

/// A flat rectangle on the floor at height `z`: centre `c`, `along` by `across` metres, its long
/// side at `heading` radians.
fn floor_rect(c: [f32; 2], along: f32, across: f32, heading: f32, z: f32) -> Mesh {
    box_mesh([along, across, 0.001])
        .rotated_z(heading)
        .translated([c[0], c[1], z])
}

/// The cross-sections of a belt along `path` (in metres): at most [`STEP`] apart, closer on a
/// tight curve, and at every segment's ends; on a path long enough to need more than
/// [`MOST_STATIONS`], the steps are stretched alike to fit.
pub(crate) fn stations(path: &Path) -> Vec<Station> {
    stations_with(path, &[])
}

/// [`stations`], and where its steps are stretched, one more at each distance along the path in
/// `cuts` (the ends of the rails' gaps), so a gap shorter than a step still has stations at its
/// ends and its rail left out.
pub(crate) fn stations_with(path: &Path, cuts: &[f64]) -> Vec<Station> {
    let step_of = |seg: &PathSegment| match *seg {
        PathSegment::Line { .. } => STEP,
        PathSegment::Arc { radius, .. } | PathSegment::Helix { radius, .. } => {
            (radius.abs() * 6.0_f64.to_radians()).clamp(0.05, STEP)
        }
    };
    let wanted: f64 = path.segments.iter().map(|s| s.length() / step_of(s)).sum();
    let stretch = (wanted / MOST_STATIONS).max(1.0);
    let mut at: Vec<f64> = Vec::new();
    let mut done = 0.0;
    for seg in &path.segments {
        let len = seg.length();
        let step = step_of(seg) * stretch;
        let n = (len / step).ceil().max(1.0) as u32;
        let first = u32::from(!at.is_empty());
        for k in first..=n {
            at.push(done + len * f64::from(k) / f64::from(n));
        }
        done += len;
    }
    if stretch > 1.0 {
        at.extend(cuts.iter().copied().filter(|c| *c > 0.0 && *c < done));
        at.sort_by(f64::total_cmp);
        at.dedup();
    }
    at.into_iter()
        .map(|s| {
            let f = path.point_at(s);
            Station {
                x: f.x as f32,
                y: f.y as f32,
                z: f.z as f32,
                heading: f.heading_deg.to_radians() as f32,
            }
        })
        .collect()
}

/// The distance along `path` of the point nearest `q`, how far `q` is from it, and on which side
/// (1 left of the direction of travel, -1 right), to within 5 cm along the path. A path of up to
/// [`MOST_STATIONS`] steps of 5 cm is looked at every 5 cm. A longer one is looked at in that many
/// steps, then each step that could hold a nearer point is looked at more closely, until the
/// steps are 5 cm: no point of a step is nearer than its ends' distances allow, since a point
/// moves no further than it goes along the path. At most [`NEAREST_LOOKS`] points in all.
fn nearest(path: &Path, q: [f64; 2]) -> (f64, f64, f64) {
    let total = path.length();
    let look = |s: f64| {
        let f = path.point_at(s);
        let (dx, dy) = (q[0] - f.x, q[1] - f.y);
        let h = f.heading_deg.to_radians();
        let side = cos(h) * dy - sin(h) * dx;
        (
            s,
            (dx * dx + dy * dy).sqrt(),
            if side >= 0.0 { 1.0 } else { -1.0 },
        )
    };
    let n = ((total / 0.05).ceil().clamp(1.0, MOST_STATIONS) as u32).max(1);
    let first: Vec<(f64, f64, f64)> = (0..=n)
        .map(|k| look(total * f64::from(k) / f64::from(n)))
        .collect();
    let mut best = (0.0, f64::INFINITY, 1.0);
    for p in &first {
        if p.1 < best.1 {
            best = *p;
        }
    }
    if !total.is_finite() || total / f64::from(n) <= 0.05 {
        return best;
    }
    // The steps that may hold a nearer point, the most promising first.
    let mut open = std::collections::BinaryHeap::new();
    for w in first.windows(2) {
        open.push(Step::new(w[0], w[1]));
    }
    let mut looks = NEAREST_LOOKS;
    while let Some(step) = open.pop() {
        if step.bound >= best.1 || looks == 0 {
            break;
        }
        let (a, b) = (step.a, step.b);
        let len = b.0 - a.0;
        if len <= 0.05 {
            continue;
        }
        let m = (len / 0.05).ceil().min(16.0) as u32;
        let mut prev = a;
        for k in 1..=m {
            let here = if k == m {
                b
            } else {
                looks = looks.saturating_sub(1);
                look(a.0 + len * f64::from(k) / f64::from(m))
            };
            if here.1 < best.1 {
                best = here;
            }
            let part = Step::new(prev, here);
            if part.bound < best.1 {
                open.push(part);
            }
            prev = here;
        }
    }
    best
}

/// The most points [`nearest`] looks at past its first [`MOST_STATIONS`].
const NEAREST_LOOKS: u32 = 65_536;

/// A step along a path between two points [`nearest`] looked at (distance along, distance from
/// the point sought, side), with the least distance any point between them can have.
struct Step {
    a: (f64, f64, f64),
    b: (f64, f64, f64),
    bound: f64,
}

impl Step {
    fn new(a: (f64, f64, f64), b: (f64, f64, f64)) -> Self {
        Self {
            a,
            b,
            bound: (a.1 + b.1 - (b.0 - a.0)) / 2.0,
        }
    }
}

// Ordered so that a `BinaryHeap` gives the step of the least bound first.
impl PartialEq for Step {
    fn eq(&self, other: &Self) -> bool {
        self.bound.total_cmp(&other.bound).is_eq()
    }
}
impl Eq for Step {}
impl PartialOrd for Step {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Step {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.bound.total_cmp(&self.bound)
    }
}

/// A belt's rail gaps on its left and right (from, to, in metres along it), and the transfer
/// plates to add between it and the belts that leave it.
type Gaps = (Vec<(f32, f32)>, Vec<(f32, f32)>, Vec<Mesh>);

/// Where other belts (`others`: path and width) leave the belt along `path` from its side (a
/// lane at a divert) or join it with their end (an infeed merging): the rail on that side opens
/// there, and a plate bridges to a lane that starts a little way off.
fn gaps_of(path: &Path, width: f32, others: &[(&Path, f32)]) -> Gaps {
    let (mut left, mut right, mut plates) = (Vec::new(), Vec::new(), Vec::new());
    let total = path.length();
    let closed = {
        let (a, b) = (path.start, path.end());
        ((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y)).sqrt() < 0.05
    };
    let half = f64::from(width / 2.0);
    let reach = half + f64::from(RAIL) + 0.3;
    for (other, w) in others {
        let w = f64::from(*w);
        // Another belt that starts beside this one: a lane leaving it at a divert.
        let start = other.start;
        let (s, d, side) = nearest(path, [start.x, start.y]);
        if d <= reach && d > half - 0.05 {
            let gap = ((s - w / 2.0 - 0.06) as f32, (s + w / 2.0 + 0.06) as f32);
            (if side > 0.0 { &mut left } else { &mut right }).push(gap);
            let f = path.point_at(s);
            let h = f.heading_deg.to_radians();
            let (nx, ny) = (-sin(h) * side, cos(h) * side);
            let edge = half - 0.02;
            let span = d - edge;
            if span > 0.01 {
                let c = [
                    (f.x + nx * (edge + span / 2.0)) as f32,
                    (f.y + ny * (edge + span / 2.0)) as f32,
                ];
                let angle = libm::atan2f(ny as f32, nx as f32);
                plates.push(
                    box_mesh([span as f32 + 0.04, w as f32, 0.03])
                        .rotated_z(angle)
                        .translated([c[0], c[1], f.z as f32 + BELT_HEIGHT - 0.03]),
                );
            }
        }
        // Another belt that ends on this one: an infeed merging onto it, from one side.
        let end = other.end();
        let (s, d, _) = nearest(path, [end.x, end.y]);
        if d <= reach {
            let back = other.point_at(other.length() - 1.5);
            let (_, _, side) = nearest(path, [back.x, back.y]);
            let target = if side > 0.0 { &mut left } else { &mut right };
            target.push(((s - 1.6) as f32, (s + 0.4) as f32));
            if closed {
                target.push(((s + total - 1.6) as f32, (s + total + 0.4) as f32));
            }
        }
    }
    (left, right, plates)
}

/// `mesh` placed at frame `f` (metres): turned by its heading, then moved.
fn placed(mesh: &Mesh, f: &Frame) -> Mesh {
    mesh.rotated_z(f.heading_deg.to_radians() as f32)
        .translated([f.x as f32, f.y as f32, f.z as f32])
}

/// The model in world space, from its layout.
pub(crate) fn scene(model: &Model) -> Scene {
    let mut tris = Vec::new();
    let mut floor = Vec::new();
    let mut shadows = Vec::new();
    let mut anchors = Vec::new();
    let mut unplaced = Vec::new();
    let tracks: Vec<(ElementId, Path, f32)> = model
        .elements
        .iter()
        .filter(|e| matches!(e.kind, ElementKind::Conveyor { .. }))
        .filter_map(|e| track_metres(model, &e.id).map(|(p, w)| (e.id.clone(), p, w)))
        .collect();
    for (order, e) in model.elements.iter().enumerate() {
        let id = e.id.as_str().to_owned();
        if matches!(e.kind, ElementKind::Conveyor { .. }) {
            let Some((_, path, width)) = tracks.iter().find(|(t, _, _)| *t == e.id) else {
                unplaced.push(crate::layout_view::label_of(e));
                continue;
            };
            let others: Vec<(&Path, f32)> = tracks
                .iter()
                .filter(|(t, _, _)| *t != e.id)
                .map(|(_, p, w)| (p, *w))
                .collect();
            let (left, right, plates) = gaps_of(path, *width, &others);
            let cuts: Vec<f64> = left
                .iter()
                .chain(&right)
                .flat_map(|(a, b)| [f64::from(*a), f64::from(*b)])
                .collect();
            let st = stations_with(path, &cuts);
            let belt = kit::Belt {
                width: *width,
                height: BELT_HEIGHT,
                ..kit::Belt::default()
            };
            let parts = kit::belt_along(&st, belt, &left, &right);
            let rails = named_colour(&e.name).map_or(Paint::Frame, Paint::Lane);
            push_mesh(&mut tris, &parts.belt, Paint::Belt);
            push_mesh(&mut tris, &parts.frame, rails);
            push_mesh(&mut tris, &parts.legs, Paint::Steel);
            push_mesh(&mut tris, &parts.accent, Paint::Marking);
            for p in &plates {
                push_mesh(&mut tris, p, Paint::Steel);
            }
            for w in st.windows(2) {
                let (a, b) = (w[0], w[1]);
                let along = libm::hypotf(b.x - a.x, b.y - a.y);
                let c = [(a.x + b.x) / 2.0, (a.y + b.y) / 2.0];
                let h = libm::atan2f(b.y - a.y, b.x - a.x);
                shadows.push(floor_rect(c, along + 0.02, width + 0.35, h, 0.002));
            }
            anchors.push(Anchor {
                id,
                order,
                spot: Spot::Path { path: path.clone() },
            });
            continue;
        }
        let Some(f) = frame_metres(model, e) else {
            unplaced.push(crate::layout_view::label_of(e));
            continue;
        };
        let look = look_of(e);
        for (mesh, paint) in &look.parts {
            push_mesh(&mut tris, &placed(mesh, &f), *paint);
        }
        shadows.push(floor_rect(
            [f.x as f32, f.y as f32],
            look.footprint[0] + 0.3,
            look.footprint[1] + 0.3,
            f.heading_deg.to_radians() as f32,
            0.002,
        ));
        // A gateway holds entities while a full buffer after it keeps them from leaving: a divert
        // holds them on the belt at its point, a gateway standing on its own on its cabinet.
        let on_top = Spot::Block {
            centre: [f.x as f32, f.y as f32],
            top: f.z as f32 + look.top,
        };
        let spot = match e.kind {
            ElementKind::Source { .. } | ElementKind::Sink { .. } => None,
            ElementKind::ExclusiveGateway { .. } | ElementKind::ParallelGateway => {
                Some(on_belt_at(model, e).unwrap_or(on_top))
            }
            _ => Some(on_top),
        };
        if let Some(spot) = spot {
            anchors.push(Anchor { id, order, spot });
        }
    }
    // The scenery, look only.
    let m = unit_metres(model.experiment.length_unit);
    let mut floors: Vec<(Vec3, Vec3)> = Vec::new();
    let mut markings = Vec::new();
    for o in &model.site.scene {
        let f = Frame::new(o.at.x * m, o.at.y * m, o.at.z * m, o.at.heading_deg);
        let [across, along, high] = o.size.map(|x| (x * m) as f32);
        let c = [f.x as f32, f.y as f32];
        let h = f.heading_deg.to_radians() as f32;
        match o.kind.as_str() {
            "floor" if along > 0.0 && across > 0.0 => {
                let slab = floor_rect(c, along, across, h, 0.0);
                floors.push(slab.bounds());
                push_mesh(&mut floor, &slab, Paint::Floor);
            }
            "marking" if along > 0.0 && across > 0.0 => {
                markings.push(floor_rect(c, along, across, h, 0.004));
            }
            "scanner" => {
                let arch = kit::ScannerArch {
                    span: if across > 0.0 { across } else { 1.1 },
                    height: if high > 0.0 { high } else { 1.45 },
                    ..kit::ScannerArch::default()
                };
                let (frame, accent) = kit::scanner_arch(arch);
                push_mesh(&mut tris, &placed(&frame, &f), Paint::Frame);
                push_mesh(&mut tris, &placed(&accent, &f), Paint::Glow);
            }
            "column" | "wall" if along > 0.0 && across > 0.0 && high > 0.0 => {
                let b = kit::bevel_box([along, across, high], 0.03);
                push_mesh(&mut tris, &placed(&b, &f), Paint::Housing);
            }
            _ => {}
        }
    }
    for mk in &markings {
        push_mesh(&mut floor, mk, Paint::Marking);
    }
    for sh in &shadows {
        push_mesh(&mut floor, sh, Paint::Shadow);
    }
    // Fit frames what stands on the floor (the equipment, and scenery such as a scanner arch or
    // a column), as the plan's Fit frames the placed blocks and belts: a floor is usually far
    // larger than the equipment on it, and fitting it would shrink the equipment to a corner of
    // the view. The floor and its markings are framed only when nothing stands on them.
    let bounds = bounds_of(if tris.is_empty() { &floor } else { &tris });
    let grid_box = floors.into_iter().reduce(|a, b| {
        (
            [a.0[0].min(b.0[0]), a.0[1].min(b.0[1]), 0.0],
            [a.1[0].max(b.1[0]), a.1[1].max(b.1[1]), 0.0],
        )
    });
    let product_colours = model
        .product_values()
        .map(|(_, values)| values.iter().map(|v| named_colour(v)).collect())
        .unwrap_or_default();
    Scene {
        grid: floor_grid(grid_box.unwrap_or(bounds), grid_box.is_some()),
        floor,
        tris,
        anchors,
        bounds,
        product_colours,
        unplaced,
    }
}

/// The corners of the box around the triangles; a small box over the origin when there are none.
fn bounds_of(tris: &[Tri]) -> (Vec3, Vec3) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in tris.iter().flat_map(|t| t.v) {
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

/// A floor grid a metre apart (wider for a large model) over `area`; when `exact` is false, a
/// little beyond it.
fn floor_grid(area: (Vec3, Vec3), exact: bool) -> Vec<Segment> {
    let (lo, hi) = area;
    let pad = if exact { 0.0 } else { 2.0 };
    let span = (hi[0] - lo[0]).max(hi[1] - lo[1]) + 2.0 * pad;
    let step = [1.0_f32, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0]
        .into_iter()
        .find(|s| span / s <= 60.0)
        .unwrap_or(1000.0);
    let (x0, x1) = (lo[0] - pad, hi[0] + pad);
    let (y0, y1) = (lo[1] - pad, hi[1] + pad);
    let mut out = Vec::new();
    let mut x = libm::ceilf(x0 / step) * step;
    while x <= x1 && out.len() < 400 {
        out.push(([x, y0, 0.006], [x, y1, 0.006]));
        x += step;
    }
    let mut y = libm::ceilf(y0 / step) * step;
    while y <= y1 && out.len() < 800 {
        out.push(([x0, y, 0.006], [x1, y, 0.006]));
        y += step;
    }
    out
}

/// The items on a conveyor: how far along each is (0 to 1), its number and its product key.
pub(crate) type BeltItems = Vec<(f32, u64, u32)>;

/// Where to write a number of parcels not drawn.
type Labels = Vec<(Vec3, String)>;

/// The parcels of a watched run. On a conveyor, one per item of `on_belt(id)` at its place along
/// the path, in its product's colour and a size by its number; when that is `None`, `inside(id)`
/// parcels spread evenly. At another block, `inside(id)` parcels on its top (at most
/// [`MOST_AT_A_BLOCK`]). With them, the numbers of those too many to draw, and where to write
/// them.
pub(crate) fn parcels(
    scene: &Scene,
    inside: &dyn Fn(&str) -> u64,
    on_belt: &dyn Fn(&str) -> Option<BeltItems>,
) -> (Vec<Tri>, Labels) {
    let shapes: Vec<(Mesh, Mesh)> = PARCELS
        .iter()
        .map(|size| kit::parcel(kit::Parcel { size: *size }))
        .collect();
    let colour = |key: usize| {
        scene
            .product_colours
            .get(key)
            .copied()
            .flatten()
            .map_or(Paint::Tote(key), Paint::Named)
    };
    let put = |tris: &mut Vec<Tri>, which: usize, f: &Frame, paint: Paint| {
        if let Some((carton, tape)) = shapes.get(which % shapes.len()) {
            push_mesh(tris, &placed(carton, f), paint);
            push_mesh(tris, &placed(tape, f), Paint::Tape);
        }
    };
    let mut tris = Vec::new();
    let mut labels = Vec::new();
    for a in &scene.anchors {
        match &a.spot {
            Spot::Path { path } => {
                let total = path.length();
                let on_top = |s: f64| {
                    let f = path.point_at(s);
                    Frame::new(f.x, f.y, f.z + f64::from(BELT_HEIGHT), f.heading_deg)
                };
                if let Some(items) = on_belt(&a.id) {
                    for &(fraction, number, product) in items.iter().take(MOST_ON_A_BELT) {
                        let s = f64::from(fraction.clamp(0.0, 1.0)) * total;
                        let key = usize::try_from(product).unwrap_or(0);
                        let which = usize::try_from(number % 3).unwrap_or(0);
                        put(&mut tris, which, &on_top(s), colour(key));
                    }
                    if items.len() > MOST_ON_A_BELT {
                        let f = on_top(total / 2.0);
                        labels.push((
                            [f.x as f32, f.y as f32, f.z as f32 + 0.6],
                            format!("+{}", items.len() - MOST_ON_A_BELT),
                        ));
                    }
                    continue;
                }
                let n = usize::try_from(inside(&a.id)).unwrap_or(usize::MAX);
                let room = (total / 0.5).floor().max(0.0) as usize;
                let shown = n.min(room).min(MOST_ON_A_BELT);
                for i in 0..shown {
                    let s = (i as f64 + 0.5) / shown as f64 * total;
                    put(&mut tris, i, &on_top(s), Paint::Tote(a.order));
                }
                if n > shown && shown > 0 {
                    let f = on_top(total / 2.0);
                    labels.push((
                        [f.x as f32, f.y as f32, f.z as f32 + 0.6],
                        format!("+{}", n - shown),
                    ));
                }
            }
            Spot::Block { centre, top } => {
                let n = usize::try_from(inside(&a.id)).unwrap_or(usize::MAX);
                let shown = n.min(MOST_AT_A_BLOCK);
                let mut z = *top;
                for i in 0..shown {
                    let f = Frame::new(
                        f64::from(centre[0]),
                        f64::from(centre[1]),
                        f64::from(z),
                        30.0 * i as f64,
                    );
                    put(&mut tris, i, &f, Paint::Tote(a.order));
                    z += PARCELS[i % PARCELS.len()][2] + 0.01;
                }
                if n > shown {
                    labels.push(([centre[0], centre[1], z + 0.3], format!("+{}", n - shown)));
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
    /// A camera at a three-quarter view from the front, a little to the left (from `-y`, a little
    /// towards `-x`, so that a belt and an arch across a 30-degree infeed both show their faces),
    /// as near as it can be while every corner of `bounds` shows in a view `aspect` (width over
    /// height) wide, with a margin, and with the box centred on screen.
    pub(crate) fn fitted(bounds: (Vec3, Vec3), aspect: f32) -> Self {
        let (lo, hi) = bounds;
        let mut cam = Self {
            target: midpoint(lo, hi),
            yaw: -FRAC_PI_2 - 0.25,
            pitch: 0.7,
            distance: 1.0,
        };
        let corners: Vec<Vec3> = (0..8)
            .map(|k| {
                [
                    if k & 1 == 0 { lo[0] } else { hi[0] },
                    if k & 2 == 0 { lo[1] } else { hi[1] },
                    if k & 4 == 0 { lo[2] } else { hi[2] },
                ]
            })
            .collect();
        let ty = libm::tanf(FOV / 2.0) * 0.92;
        let tx = ty * aspect.clamp(0.2, 5.0);
        for _ in 0..6 {
            // Far enough that each corner is inside the view.
            let b = cam.basis();
            let mut d: f32 = 1.0;
            for c in &corners {
                let v = sub(*c, cam.target);
                let (x, y, z) = (dot(v, b.right), dot(v, b.up), dot(v, b.forward));
                d = d.max(x.abs() / tx - z).max(y.abs() / ty - z);
            }
            cam.distance = d.min(5000.0);
            // Then centre the box: move the target by the middle of the corners' extent on screen.
            let b = cam.basis();
            let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
            for c in &corners {
                let v = sub(*c, b.eye);
                let depth = dot(v, b.forward).max(NEAR);
                let (x, y) = (dot(v, b.right) / depth, dot(v, b.up) / depth);
                (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
            }
            let shift = add(
                scale(b.right, (x0 + x1) / 2.0 * cam.distance),
                scale(b.up, (y0 + y1) / 2.0 * cam.distance),
            );
            cam.target = add(cam.target, shift);
        }
        cam
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
    /// How much of its colour the light leaves.
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

    /// The triangles of `parts` that face the camera and lie in front of it, shaded: far to near
    /// when `sort`, otherwise in the order given.
    pub(crate) fn faces(&self, parts: &[&[Tri]], sort: bool) -> Vec<Face> {
        // A key light from the front left and above, with a sky fill so no face goes black.
        let light = normalized([0.35, -0.55, 0.76]);
        let mut out = Vec::new();
        for tri in parts.iter().flat_map(|p| p.iter()) {
            if dot(tri.normal, sub(self.basis.eye, tri.v[0])) <= 0.0 {
                continue;
            }
            let corners = tri.v.map(|p| self.in_camera(p));
            if corners.iter().any(|c| c[2] < NEAR) {
                continue;
            }
            let sky = 0.5 + 0.5 * tri.normal[2].max(0.0);
            out.push(Face {
                pts: corners.map(|c| self.on_screen(c)),
                depth: (corners[0][2] + corners[1][2] + corners[2][2]) / 3.0,
                shade: (0.45 * sky + 0.6 * dot(tri.normal, light).max(0.0)).min(1.0),
                paint: tri.paint,
            });
        }
        if sort {
            out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        }
        out
    }
}

/// `colour` lit by `shade`, keeping its alpha.
fn shaded(colour: Color32, shade: f32) -> Color32 {
    let k = |c: u8| (f32::from(c) * shade).clamp(0.0, 255.0) as u8;
    Color32::from_rgba_premultiplied(k(colour.r()), k(colour.g()), k(colour.b()), colour.a())
}

/// The 3D view's state: the camera (none until the first frame fits it) and the scene of the
/// model revision it was built for.
#[derive(Default)]
pub(crate) struct View3d {
    camera: Option<Camera>,
    scene: Option<(u64, Scene)>,
}

/// The colour of `paint` in the current theme (light or dark).
fn paint_colour(ctx: &egui::Context, paint: Paint) -> Color32 {
    let dark = ctx.theme() == egui::Theme::Dark;
    let pick = |light: [u8; 3], night: [u8; 3]| {
        let [r, g, b] = if dark { night } else { light };
        Color32::from_rgb(r, g, b)
    };
    match paint {
        Paint::Floor => pick([214, 218, 224], [52, 57, 66]),
        Paint::Shadow => Color32::from_black_alpha(if dark { 70 } else { 40 }),
        Paint::Marking => pick([236, 176, 28], [230, 172, 30]),
        Paint::Belt => pick([46, 50, 58], [24, 26, 31]),
        Paint::Frame => pick([62, 96, 140], [92, 132, 186]),
        Paint::Lane([r, g, b]) | Paint::Named([r, g, b]) => Color32::from_rgb(r, g, b),
        Paint::Steel => pick([120, 128, 140], [140, 148, 160]),
        Paint::Wood => pick([196, 160, 108], [176, 142, 96]),
        Paint::Cardboard => pick([200, 160, 104], [184, 146, 94]),
        Paint::Tape => pick([236, 222, 196], [224, 210, 186]),
        Paint::Housing => pick([78, 86, 100], [104, 114, 132]),
        Paint::Glow => pick([56, 189, 248], [96, 205, 250]),
        Paint::Person => pick([242, 140, 40], [246, 152, 56]),
        Paint::Skin => pick([214, 166, 126], [206, 160, 122]),
        Paint::Lamp(0) => Color32::from_rgb(46, 190, 90),
        Paint::Lamp(1) => Color32::from_rgb(246, 176, 34),
        Paint::Lamp(_) => Color32::from_rgb(226, 62, 56),
        Paint::Tote(i) => chart_palette::series_color(ctx, i),
    }
}

/// Paints `faces` onto `painter` in chunks small enough for 16-bit indices.
fn paint_faces(painter: &egui::Painter, ctx: &egui::Context, faces: &[Face]) {
    for chunk in faces.chunks(CHUNK) {
        let mut mesh = egui::Mesh::default();
        for f in chunk {
            let colour = match f.paint {
                Paint::Shadow | Paint::Floor => paint_colour(ctx, f.paint),
                _ => shaded(paint_colour(ctx, f.paint), f.shade),
            };
            let first = mesh.vertices.len() as u32;
            for pt in f.pts {
                mesh.colored_vertex(pt, colour);
            }
            mesh.add_triangle(first, first + 1, first + 2);
        }
        painter.add(Shape::mesh(mesh));
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

        let aspect = rect.width() / rect.height().max(1.0);
        let cam = camera.get_or_insert_with(|| Camera::fitted(scene.bounds, aspect));
        if fit {
            *cam = Camera::fitted(scene.bounds, aspect);
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

        // The run being watched: its parcels.
        let (live_tris, labels) = match watching {
            Some(w) => parcels(
                scene,
                &|id| {
                    w.state(id).map_or(0, |s| {
                        s.entered.saturating_sub(s.left).saturating_sub(s.lost)
                    })
                },
                &|id| Some(w.conveyor_items(id)),
            ),
            None => (Vec::new(), Vec::new()),
        };

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, p.canvas);
        let view = cam.view(rect);
        paint_faces(&painter, ui.ctx(), &view.faces(&[&scene.floor], false));
        let grid = Stroke::new(1.0, p.border);
        for (a, b) in &scene.grid {
            if let Some(line) = view.segment(*a, *b) {
                painter.line_segment(line, grid);
            }
        }
        paint_faces(
            &painter,
            ui.ctx(),
            &view.faces(&[&scene.tris, &live_tris], true),
        );
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
        if !scene.is_empty() && !scene.unplaced.is_empty() {
            // Blocks the layout leaves out are named, never drawn somewhere guessed.
            painter.text(
                rect.left_top() + vec2(8.0, 8.0),
                Align2::LEFT_TOP,
                format!("{}: {}", t("Unplaced"), scene.unplaced.join(", ")),
                FontId::proportional(12.0),
                p.text_weak,
            );
        }
        if scene.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                t("Nothing to draw: this model has no layout yet. The Layout tab lists its blocks as unplaced."),
                FontId::proportional(14.0),
                p.text_weak,
            );
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

    /// The parcel sorter example, as the engine opens it.
    pub(crate) fn sorter() -> Model {
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
    fn a_belt_hundreds_of_kilometres_long_has_a_bounded_number_of_stations() {
        let straight = |length: f64| {
            Path::new(
                Frame::default(),
                vec![PathSegment::Line { length, rise: 0.0 }],
            )
        };
        // 500 km would be two million quarter-metre steps.
        let long = stations(&straight(500_000.0));
        // At most the budget, past the start and one for its segment.
        assert!(long.len() <= MOST_STATIONS as usize + 2, "{}", long.len());
        let last = long.last().unwrap();
        assert!((last.x - 500_000.0).abs() < 1.0, "{}", last.x);
        // An ordinary length keeps its quarter-metre steps: 10 m is 41 stations.
        assert_eq!(stations(&straight(10.0)).len(), 41);
    }

    #[test]
    fn the_nearest_point_on_a_very_long_belt_is_found_to_5_cm() {
        // A million kilometres: 5 cm steps all along would be twenty billion points.
        let path = Path::new(
            Frame::default(),
            vec![PathSegment::Line {
                length: 1.0e9,
                rise: 0.0,
            }],
        );
        let (s, d, side) = nearest(&path, [654_321_987.6, -3.0]);
        assert!((s - 654_321_987.6).abs() <= 0.05, "{s}");
        assert!((d - 3.0).abs() < 1e-3, "{d}");
        assert_eq!(side, -1.0);
        // On an ordinary belt, the 5 cm steps as before.
        let short = Path::new(
            Frame::default(),
            vec![PathSegment::Line {
                length: 10.0,
                rise: 0.0,
            }],
        );
        let (s, d, side) = nearest(&short, [4.0, 1.0]);
        assert!((s - 4.0).abs() < 1e-9 && (d - 1.0).abs() < 1e-9, "{s} {d}");
        assert_eq!(side, 1.0);
    }

    #[test]
    fn a_rail_gap_on_a_very_long_belt_is_left_out() {
        // 10 km: the stations are about 2.4 m apart, wider than a gap of 0.72 m.
        let path = Path::new(
            Frame::default(),
            vec![PathSegment::Line {
                length: 10_000.0,
                rise: 0.0,
            }],
        );
        let gap = (9.64_f32, 10.36_f32);
        let st = stations_with(&path, &[f64::from(gap.0), f64::from(gap.1)]);
        let belt = kit::Belt::default();
        let gapped = kit::belt_along(&st, belt, &[gap], &[]);
        let whole = kit::belt_along(&st, belt, &[], &[]);
        assert!(gapped.frame.triangle_count() < whole.frame.triangle_count());
    }

    #[test]
    fn the_nearest_point_is_found_on_the_nearer_of_two_close_runs() {
        // A long U: 2047.25 m east, a sharp left turn, 1 m north, another, 2047.75 m west. The
        // point is 0.45 m from the first run and 0.55 m from the way back.
        let path = Path::new(
            Frame::default(),
            vec![
                PathSegment::Line {
                    length: 2047.25,
                    rise: 0.0,
                },
                PathSegment::Arc {
                    radius: 0.0,
                    angle_deg: 90.0,
                    rise: 0.0,
                },
                PathSegment::Line {
                    length: 1.0,
                    rise: 0.0,
                },
                PathSegment::Arc {
                    radius: 0.0,
                    angle_deg: 90.0,
                    rise: 0.0,
                },
                PathSegment::Line {
                    length: 2047.75,
                    rise: 0.0,
                },
            ],
        );
        let (s, d, side) = nearest(&path, [100.5, 0.45]);
        assert!((s - 100.5).abs() <= 0.05, "{s}");
        assert!((d - 0.45).abs() < 1e-3, "{d}");
        assert_eq!(side, 1.0);
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
        assert_eq!(view.faces(&[&[facing]], true).len(), 1);
        assert!(view.faces(&[&[away]], true).is_empty());
    }

    #[test]
    fn the_farther_triangle_is_painted_first() {
        let view = camera().view(rect());
        // The camera is on the -y side, so y = 5 is far and y = -5 near; near is listed first.
        let near = triangle(-5.0, [0.0, 0.0, 1.0]);
        let far = triangle(5.0, [0.0, 0.0, 1.0]);
        let faces = view.faces(&[&[near, far]], true);
        assert_eq!(faces.len(), 2);
        assert!(faces[0].depth > faces[1].depth, "{faces:?}");
        // The floor keeps its order.
        let floor = view.faces(&[&[near, far]], false);
        assert!(floor[0].depth < floor[1].depth);
    }

    #[test]
    fn fit_frames_the_whole_box_and_fills_the_view() {
        let bounds = ([-12.0, -8.0, 0.0], [13.0, 3.5, 1.8]);
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(1200.0, 700.0));
        let cam = Camera::fitted(bounds, r.width() / r.height());
        let view = cam.view(r);
        let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for k in 0..8 {
            let c = [
                if k & 1 == 0 { bounds.0[0] } else { bounds.1[0] },
                if k & 2 == 0 { bounds.0[1] } else { bounds.1[1] },
                if k & 4 == 0 { bounds.0[2] } else { bounds.1[2] },
            ];
            let p = view.project(c).expect("in front");
            assert!(r.expand(2.0).contains(p), "{p:?} is off screen");
            (x0, x1, y0, y1) = (x0.min(p.x), x1.max(p.x), y0.min(p.y), y1.max(p.y));
        }
        // It fills most of the view one way or the other, and is centred.
        let fill = ((x1 - x0) / r.width()).max((y1 - y0) / r.height());
        assert!(fill > 0.8, "fills {fill}");
        assert!(((x0 + x1) / 2.0 - r.center().x).abs() < 0.05 * r.width());
        assert!(((y0 + y1) / 2.0 - r.center().y).abs() < 0.05 * r.height());
    }

    #[test]
    fn the_sorter_is_drawn_from_its_layout_with_every_block_placed() {
        let m = sorter();
        let s = scene(&m);
        assert!(s.unplaced.is_empty(), "{:?}", s.unplaced);
        // It stands on the floor, inside the layout's box (the loop's far arc reaches about
        // 12.4 m, the infeed's pallet about -11.8 m).
        let (lo, hi) = s.bounds;
        assert!(lo[2] > -0.01 && hi[2] > BELT_HEIGHT);
        assert!(hi[0] > 11.5 && hi[0] < 13.5, "{hi:?}");
        assert!(lo[0] < -10.0 && lo[0] > -13.0, "{lo:?}");
        assert!(!s.floor.is_empty() && !s.grid.is_empty());
        // Every conveyor has its path, and every pack station and the divert is a spot for
        // parcels.
        let paths = s
            .anchors
            .iter()
            .filter(|a| matches!(a.spot, Spot::Path { .. }))
            .count();
        assert_eq!(paths, 6);
        assert_eq!(s.anchors.len(), 11);
    }

    #[test]
    fn parcels_held_at_the_divert_stand_on_the_belt_at_its_point() {
        let m = sorter();
        let s = scene(&m);
        let a = s.anchors.iter().find(|a| a.id == "sorter").expect("a spot");
        let Spot::Block { centre, top } = a.spot else {
            panic!("{:?}", a.spot);
        };
        let (loop_path, _) = track_metres(&m, &ElementId::unchecked("loop")).unwrap();
        let at = loop_path.point_at(0.0);
        assert!(
            (f64::from(centre[0]) - at.x).abs() < 1e-3,
            "{centre:?} {at:?}"
        );
        assert!(
            (f64::from(centre[1]) - at.y).abs() < 1e-3,
            "{centre:?} {at:?}"
        );
        assert!((top - (at.z as f32 + BELT_HEIGHT)).abs() < 1e-4);
        let (tris, _) = parcels(&s, &|id| u64::from(id == "sorter") * 2, &|_| None);
        assert!(!tris.is_empty());
    }

    #[test]
    fn a_layout_with_only_a_floor_draws_and_frames_the_floor() {
        let mut m = sorter();
        m.site.paths.clear();
        m.site.places.clear();
        m.site.scene.retain(|o| o.kind == "floor");
        let s = scene(&m);
        assert!(s.tris.is_empty() && !s.is_empty());
        // The floor is 26.4 m along x by 12.8 m across, centred at (0.3, -2.4).
        let (lo, hi) = s.bounds;
        assert!(
            (lo[0] + 12.9).abs() < 0.01 && (hi[0] - 13.5).abs() < 0.01,
            "{lo:?} {hi:?}"
        );
        assert!(
            (lo[1] + 8.8).abs() < 0.01 && (hi[1] - 4.0).abs() < 0.01,
            "{lo:?} {hi:?}"
        );
    }

    #[test]
    fn a_lane_leaving_the_loop_cuts_a_gap_in_its_rail_on_that_side() {
        let m = sorter();
        let (loop_path, w) = track_metres(&m, &ElementId::unchecked("loop")).unwrap();
        let (lane, lw) = track_metres(&m, &ElementId::unchecked("lane_red")).unwrap();
        let (left, right, plates) = gaps_of(&loop_path, w, &[(&lane, lw)]);
        // The lane leaves the sort straight on the right (outside the loop), 2.25 m along it.
        assert!(left.is_empty());
        assert_eq!(right.len(), 1);
        assert!(right[0].0 < 2.25 && right[0].1 > 2.25, "{right:?}");
        assert_eq!(plates.len(), 1);
        // The infeed joins at the loop's start (and so at its end, the loop being closed), from
        // the right.
        let (infeed, iw) = track_metres(&m, &ElementId::unchecked("infeed")).unwrap();
        let (left, right, _) = gaps_of(&loop_path, w, &[(&infeed, iw)]);
        assert!(left.is_empty());
        assert_eq!(right.len(), 2, "{right:?}");
    }

    #[test]
    fn an_item_on_a_belt_is_drawn_where_its_path_puts_it() {
        let m = sorter();
        let s = scene(&m);
        let (tris, _) = parcels(&s, &|_| 0, &|id| (id == "loop").then(|| vec![(0.5, 7, 0)]));
        assert!(!tris.is_empty());
        // Half way round the 30 m loop is 15 m along: on the far straight.
        let (loop_path, _) = track_metres(&m, &ElementId::unchecked("loop")).unwrap();
        let want = loop_path.point_at(15.0);
        let n = tris.len() as f32;
        let sum = tris.iter().fold([0.0_f32; 2], |acc, tri| {
            let c = tri
                .v
                .iter()
                .fold([0.0_f32; 2], |a, v| [a[0] + v[0], a[1] + v[1]]);
            [acc[0] + c[0] / 3.0, acc[1] + c[1] / 3.0]
        });
        let c = [sum[0] / n, sum[1] / n];
        assert!((f64::from(c[0]) - want.x).abs() < 0.1, "{c:?} {want:?}");
        assert!((f64::from(c[1]) - want.y).abs() < 0.1, "{c:?} {want:?}");
        // In its product's colour.
        assert!(tris.iter().any(|t| matches!(t.paint, Paint::Named(_))));
    }

    #[test]
    fn a_model_without_a_layout_draws_nothing_and_lists_every_block() {
        let mut m = sorter();
        m.site = Default::default();
        let s = scene(&m);
        assert!(s.is_empty() && s.anchors.is_empty());
        assert_eq!(s.unplaced.len(), m.elements.len());
    }

    #[test]
    fn a_name_that_says_a_colour_is_drawn_in_it() {
        assert_eq!(named_colour("Blue parcel"), Some([41, 109, 204]));
        assert_eq!(named_colour("gray"), Some([140, 140, 145]));
        assert_eq!(named_colour("parcel"), None);
    }
}
