//! Parametric pieces for a parcel sorter, in a chunky low-poly style: belt sections with side
//! rails, legs and dashed accent stripes swept along any centre line, a scanner arch, a packing
//! station, a pallet of cartons, a roll cage, a control cabinet with a stack light, and parcels.
//!
//! Every generator works in metres with `z` up and returns its parts as separate meshes (the
//! belt's surface, the frame, the legs, the accents…), so a viewer colours each part from its own
//! palette. The shapes are drawn by code from their parameters; none is read from a file.

use super::{Mesh, Section, box_mesh, emit_swept_strip, valid_positive};

/// A point on a conveyor's centre line, on the floor, with the height its path has climbed and
/// the direction of travel in radians (0 along `+x`, positive turning left).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Station {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub heading: f32,
}

/// A belt section's look: its width, the height of the belt's top, and the frame round it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Belt {
    /// The belt's width, edge to edge.
    pub width: f32,
    /// The height of the belt's top above the floor (plus the path's own climb).
    pub height: f32,
    /// How thick the belt is drawn.
    pub belt_thickness: f32,
    /// How deep the side frame is, below the belt's top.
    pub frame_depth: f32,
    /// How far the side rails stand above the belt.
    pub rail_height: f32,
    /// The side frame's and rails' width.
    pub rail_width: f32,
    /// The legs' square section.
    pub leg_size: f32,
    /// The distance between pairs of legs along the belt.
    pub leg_spacing: f32,
    /// The length of one dash and gap of the accent stripe along the frame.
    pub stripe_pitch: f32,
}

impl Default for Belt {
    fn default() -> Self {
        Self {
            width: 0.6,
            height: 0.85,
            belt_thickness: 0.04,
            frame_depth: 0.16,
            rail_height: 0.06,
            rail_width: 0.07,
            leg_size: 0.07,
            leg_spacing: 1.8,
            stripe_pitch: 0.5,
        }
    }
}

/// A belt section's parts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BeltParts {
    /// The moving surface.
    pub belt: Mesh,
    /// The side frames and rails.
    pub frame: Mesh,
    /// Legs, feet and cross bars.
    pub legs: Mesh,
    /// The dashed stripe along the outside of the frame.
    pub accent: Mesh,
}

/// A point `along` metres to the left of `s` and `up` metres above the floor.
fn beside(s: &Station, along: f32, up: f32) -> [f32; 3] {
    let (sn, cs) = (libm::sinf(s.heading), libm::cosf(s.heading));
    [s.x - sn * along, s.y + cs * along, s.z + up]
}

/// A cross-section at `s` from `left` to `right` (offsets to the left, `left > right`), between
/// heights `bottom` and `top`.
fn section(s: &Station, left: f32, right: f32, bottom: f32, top: f32) -> Section {
    Section {
        rt: beside(s, right, top),
        lt: beside(s, left, top),
        lb: beside(s, left, bottom),
        rb: beside(s, right, bottom),
    }
}

/// The distance along `stations` to each of them.
fn distances(stations: &[Station]) -> Vec<f32> {
    let mut out = Vec::with_capacity(stations.len());
    let mut d = 0.0;
    for (i, s) in stations.iter().enumerate() {
        if i > 0 {
            let p = &stations[i - 1];
            d += libm::hypotf(s.x - p.x, s.y - p.y);
        }
        out.push(d);
    }
    out
}

/// The station `d` metres along `stations` (`at` from [`distances`]), by straight interpolation.
fn station_at(stations: &[Station], at: &[f32], d: f32) -> Station {
    for i in 1..stations.len() {
        if d <= at[i] {
            let span = (at[i] - at[i - 1]).max(1e-6);
            let k = ((d - at[i - 1]) / span).clamp(0.0, 1.0);
            let (a, b) = (stations[i - 1], stations[i]);
            return Station {
                x: a.x + (b.x - a.x) * k,
                y: a.y + (b.y - a.y) * k,
                z: a.z + (b.z - a.z) * k,
                heading: a.heading + (b.heading - a.heading) * k,
            };
        }
    }
    stations.last().copied().unwrap_or_default()
}

/// Whether distance `d` falls in one of `gaps` (from, to).
fn in_gap(gaps: &[(f32, f32)], d: f32) -> bool {
    gaps.iter().any(|(a, b)| d > *a && d < *b)
}

/// Strips of cross-sections, one per run of station pairs outside `gaps`.
fn runs(
    stations: &[Station],
    at: &[f32],
    gaps: &[(f32, f32)],
    make: &dyn Fn(&Station) -> Section,
) -> Vec<Vec<Section>> {
    let mut out: Vec<Vec<Section>> = Vec::new();
    let mut current: Vec<Section> = Vec::new();
    for i in 1..stations.len() {
        let mid = (at[i - 1] + at[i]) / 2.0;
        if in_gap(gaps, mid) {
            if current.len() >= 2 {
                out.push(std::mem::take(&mut current));
            }
            current.clear();
            continue;
        }
        if current.is_empty() {
            current.push(make(&stations[i - 1]));
        }
        current.push(make(&stations[i]));
    }
    if current.len() >= 2 {
        out.push(current);
    }
    out
}

/// A belt section swept along `stations` (at least two, in order of travel): the belt, a side
/// frame each side with a rail above the belt, legs in pairs every `leg_spacing`, and a dashed
/// accent stripe along the outside of each frame. The rail on the left leaves out the stretches
/// in `gaps_left` (from, to, in metres along the stations) and the right one those in
/// `gaps_right`, where another belt joins or leaves. Empty for fewer than two stations or
/// parameters that are not positive.
pub fn belt_along(
    stations: &[Station],
    p: Belt,
    gaps_left: &[(f32, f32)],
    gaps_right: &[(f32, f32)],
) -> BeltParts {
    let mut parts = BeltParts::default();
    if stations.len() < 2
        || !valid_positive(&[
            p.width,
            p.height,
            p.belt_thickness,
            p.frame_depth,
            p.rail_width,
            p.leg_size,
            p.leg_spacing,
            p.stripe_pitch,
        ])
    {
        return parts;
    }
    let at = distances(stations);
    let total = at.last().copied().unwrap_or(0.0);
    let flat = stations.iter().all(|s| (s.z - stations[0].z).abs() < 1e-6);
    let hw = p.width / 2.0;
    let out = hw + p.rail_width;
    let top = p.height;
    let under = p.height - p.frame_depth;

    // The belt, whole.
    let belt: Vec<Section> = stations
        .iter()
        .map(|s| section(s, hw, -hw, top - p.belt_thickness, top))
        .collect();
    emit_swept_strip(&mut parts.belt, &belt, flat);

    // Each side: the frame below the belt's top, whole; the rail above it, with its gaps.
    for (left, gaps) in [(true, gaps_left), (false, gaps_right)] {
        let (a, b) = if left { (out, hw) } else { (-hw, -out) };
        let frame: Vec<Section> = stations
            .iter()
            .map(|s| section(s, a, b, under, top))
            .collect();
        emit_swept_strip(&mut parts.frame, &frame, flat);
        if p.rail_height > 0.0 {
            for run in runs(stations, &at, gaps, &|s| {
                section(s, a, b, top, top + p.rail_height)
            }) {
                emit_swept_strip(&mut parts.frame, &run, flat);
            }
        }
        // The dashed stripe on the frame's outer face, a little proud of it.
        let face = if left { out + 0.004 } else { -out - 0.004 };
        let (lo, hi) = (under + p.frame_depth * 0.3, under + p.frame_depth * 0.7);
        for i in 1..stations.len() {
            let mid = (at[i - 1] + at[i]) / 2.0;
            if libm::fmodf(mid, p.stripe_pitch) >= p.stripe_pitch / 2.0 {
                continue;
            }
            let (s0, s1) = (&stations[i - 1], &stations[i]);
            let (a0, b0) = (beside(s0, face, lo), beside(s0, face, hi));
            let (a1, b1) = (beside(s1, face, lo), beside(s1, face, hi));
            if left {
                parts.accent.push_quad(a0, b0, b1, a1);
            } else {
                parts.accent.push_quad(a0, a1, b1, b0);
            }
        }
    }

    // Legs in pairs, with feet and a bar between them.
    let leg_h = under;
    if leg_h > 0.0 {
        let count = (libm::ceilf(total / p.leg_spacing) as u32).max(1);
        let first = p.leg_size.min(total / 2.0);
        let last = (total - p.leg_size).max(first);
        let leg = box_mesh([p.leg_size, p.leg_size, leg_h]);
        let foot = box_mesh([p.leg_size * 2.0, p.leg_size * 2.0, 0.02]);
        let bar = box_mesh([
            p.leg_size * 0.7,
            2.0 * (hw + p.rail_width / 2.0),
            p.leg_size * 0.7,
        ]);
        for k in 0..=count {
            let d = first + (last - first) * (k as f32 / count as f32);
            let s = station_at(stations, &at, d);
            let mut pair = Mesh::new();
            for side in [1.0_f32, -1.0] {
                let y = side * (hw + p.rail_width / 2.0);
                pair.merge(&leg.translated([0.0, y, s.z + leg_h / 2.0]));
                pair.merge(&foot.translated([0.0, y, 0.01]));
            }
            pair.merge(&bar.translated([0.0, 0.0, s.z + leg_h * 0.3]));
            parts
                .legs
                .merge(&pair.rotated_z(s.heading).translated([s.x, s.y, 0.0]));
        }
    }
    parts
}

/// A box of `size` standing on `z = 0`, centred in `x` and `y`, its top edges bevelled by `bevel`
/// (at most a third of its smallest side).
pub fn bevel_box(size: [f32; 3], bevel: f32) -> Mesh {
    if !valid_positive(&size) {
        return Mesh::new();
    }
    let b = bevel.clamp(0.0, size[0].min(size[1]).min(size[2]) / 3.0);
    let (hx, hy, h) = (size[0] / 2.0, size[1] / 2.0, size[2]);
    let ring = |r: f32, z: f32| {
        [
            [-(hx - r), -(hy - r), z],
            [hx - r, -(hy - r), z],
            [hx - r, hy - r, z],
            [-(hx - r), hy - r, z],
        ]
    };
    let low = ring(0.0, 0.0);
    let mid = ring(0.0, h - b);
    let top = ring(b, h);
    let mut m = Mesh::new();
    for i in 0..4 {
        let j = (i + 1) % 4;
        m.push_quad(low[i], low[j], mid[j], mid[i]);
        if b > 0.0 {
            m.push_quad(mid[i], mid[j], top[j], top[i]);
        }
    }
    m.push_quad(top[0], top[1], top[2], top[3]);
    m.push_quad(low[3], low[2], low[1], low[0]);
    m
}

/// A scanner arch over a belt: two posts and a beam across, the belt passing along `x`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScannerArch {
    /// The clear width between the posts.
    pub span: f32,
    /// The clear height under the beam.
    pub height: f32,
    /// The posts' and beam's square section.
    pub post: f32,
}

impl Default for ScannerArch {
    fn default() -> Self {
        Self {
            span: 1.1,
            height: 1.45,
            post: 0.16,
        }
    }
}

/// A scanner arch's parts: its frame, and its accents (the reading window under the beam and a
/// band round each post).
pub fn scanner_arch(p: ScannerArch) -> (Mesh, Mesh) {
    let mut frame = Mesh::new();
    let mut accent = Mesh::new();
    if !valid_positive(&[p.span, p.height, p.post]) {
        return (frame, accent);
    }
    let y = p.span / 2.0 + p.post / 2.0;
    let depth = p.post * 1.8;
    for side in [1.0_f32, -1.0] {
        frame.merge(
            &bevel_box([depth, p.post, p.height + p.post], 0.02).translated([0.0, side * y, 0.0]),
        );
        frame.merge(&box_mesh([depth * 1.4, p.post * 1.6, 0.05]).translated([
            0.0,
            side * y,
            0.025,
        ]));
        accent.merge(&box_mesh([depth + 0.01, p.post + 0.01, 0.08]).translated([
            0.0,
            side * y,
            0.45,
        ]));
    }
    frame.merge(
        &bevel_box([depth, p.span + 2.0 * p.post, p.post], 0.02).translated([0.0, 0.0, p.height]),
    );
    accent.merge(&box_mesh([depth * 0.6, p.span * 0.85, 0.04]).translated([
        0.0,
        0.0,
        p.height - 0.02,
    ]));
    (frame, accent)
}

/// A packing station: a bench with a lower shelf, a screen on a post at one end and a stack of
/// flat cartons at the other. Its front (where work arrives) faces `-x`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PackStation {
    /// Across the bench, along `y`.
    pub width: f32,
    /// Front to back, along `x`.
    pub depth: f32,
    /// The height of the work top.
    pub height: f32,
}

impl Default for PackStation {
    fn default() -> Self {
        Self {
            width: 1.4,
            depth: 0.8,
            height: 0.9,
        }
    }
}

/// A packing station's parts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PackParts {
    /// Legs, shelf and the screen's post.
    pub frame: Mesh,
    /// The work top.
    pub top: Mesh,
    /// The screen's housing.
    pub screen: Mesh,
    /// The screen's face.
    pub glow: Mesh,
    /// The cartons.
    pub cartons: Mesh,
}

/// A packing station.
pub fn pack_station(p: PackStation) -> PackParts {
    let mut parts = PackParts::default();
    if !valid_positive(&[p.width, p.depth, p.height]) || p.height <= 0.1 {
        return parts;
    }
    let (hx, hy) = (p.depth / 2.0, p.width / 2.0);
    let slab = 0.05;
    parts.top = bevel_box([p.depth, p.width, slab], 0.015).translated([0.0, 0.0, p.height - slab]);
    let leg = box_mesh([0.06, 0.06, p.height - slab]);
    for (x, y) in [(hx, hy), (hx, -hy), (-hx, hy), (-hx, -hy)] {
        parts.frame.merge(&leg.translated([
            x - x.signum() * 0.05,
            y - y.signum() * 0.05,
            (p.height - slab) / 2.0,
        ]));
    }
    parts
        .frame
        .merge(&box_mesh([p.depth - 0.1, p.width - 0.1, 0.03]).translated([
            0.0,
            0.0,
            p.height * 0.3,
        ]));
    // The screen at the `+y` end, facing the back (`+x`), where the packer stands.
    let post_h = 0.45;
    parts
        .frame
        .merge(&box_mesh([0.05, 0.05, post_h]).translated([
            -hx * 0.4,
            hy - 0.2,
            p.height + post_h / 2.0,
        ]));
    let screen = [0.05, 0.42, 0.28];
    let at = [-hx * 0.4 + 0.04, hy - 0.2, p.height + post_h];
    parts.screen = box_mesh(screen).translated(at);
    parts.glow = box_mesh([0.01, screen[1] - 0.06, screen[2] - 0.06]).translated([
        at[0] + screen[0] / 2.0 + 0.004,
        at[1],
        at[2],
    ]);
    // Flat cartons stacked at the `-y` end.
    for k in 0..4 {
        parts.cartons.merge(
            &box_mesh([0.42, 0.34, 0.035])
                .rotated_z(0.08 * k as f32)
                .translated([0.0, -hy + 0.25, p.height + 0.02 + 0.036 * k as f32]),
        );
    }
    parts
}

/// A pallet with cartons on it, its long side along `x`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pallet {
    /// Length (`x`) and width (`y`) of the pallet.
    pub size: [f32; 2],
    /// How many layers of cartons it carries.
    pub layers: u32,
}

impl Default for Pallet {
    fn default() -> Self {
        Self {
            size: [1.2, 0.8],
            layers: 2,
        }
    }
}

/// A pallet's parts: the wood, and the cartons.
pub fn pallet(p: Pallet) -> (Mesh, Mesh) {
    let mut wood = Mesh::new();
    let mut cartons = Mesh::new();
    if !valid_positive(&p.size) {
        return (wood, cartons);
    }
    let [lx, ly] = p.size;
    for k in 0..3 {
        let y = (k as f32 - 1.0) * (ly / 2.0 - 0.05);
        wood.merge(&box_mesh([lx, 0.1, 0.1]).translated([0.0, y, 0.05]));
    }
    for k in 0..5 {
        let x = (k as f32 - 2.0) * (lx / 2.0 - 0.05) / 2.0;
        wood.merge(&box_mesh([0.1, ly, 0.022]).translated([x, 0.0, 0.111]));
    }
    let base = 0.122;
    let (cx, cy, ch) = (lx / 3.0 - 0.02, ly / 2.0 - 0.02, 0.28);
    for layer in 0..p.layers {
        for i in 0..3 {
            for j in 0..2 {
                let x = (i as f32 - 1.0) * (lx / 3.0);
                let y = (j as f32 - 0.5) * (ly / 2.0);
                cartons.merge(&bevel_box([cx, cy, ch], 0.02).translated([
                    x,
                    y,
                    base + layer as f32 * (ch + 0.005),
                ]));
            }
        }
    }
    (wood, cartons)
}

/// A roll cage for outgoing parcels: a base on castors, corner posts and rails on three sides
/// (open towards `-x`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RollCage {
    pub size: [f32; 3],
}

impl Default for RollCage {
    fn default() -> Self {
        Self {
            size: [0.8, 0.7, 1.7],
        }
    }
}

/// A roll cage's parts: the frame, and the base.
pub fn roll_cage(p: RollCage) -> (Mesh, Mesh) {
    let mut frame = Mesh::new();
    let mut base = Mesh::new();
    if !valid_positive(&p.size) {
        return (frame, base);
    }
    let [lx, ly, h] = p.size;
    let (hx, hy) = (lx / 2.0, ly / 2.0);
    let floor = 0.16;
    base.merge(&box_mesh([lx, ly, 0.04]).translated([0.0, 0.0, floor]));
    for (x, y) in [(hx, hy), (hx, -hy), (-hx, hy), (-hx, -hy)] {
        let (x, y) = (x - x.signum() * 0.05, y - y.signum() * 0.05);
        base.merge(&box_mesh([0.07, 0.07, floor - 0.02]).translated([x, y, (floor - 0.02) / 2.0]));
        frame.merge(&box_mesh([0.035, 0.035, h - floor]).translated([
            x,
            y,
            floor + (h - floor) / 2.0,
        ]));
    }
    for k in 1..5 {
        let z = floor + (h - floor) * k as f32 / 5.0;
        frame.merge(&box_mesh([0.02, ly - 0.1, 0.02]).translated([hx - 0.05, 0.0, z]));
        for side in [1.0_f32, -1.0] {
            frame.merge(&box_mesh([lx - 0.1, 0.02, 0.02]).translated([0.0, side * (hy - 0.05), z]));
        }
    }
    (frame, base)
}

/// A control cabinet with a stack light on top.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cabinet {
    pub size: [f32; 3],
}

impl Default for Cabinet {
    fn default() -> Self {
        Self {
            size: [0.4, 0.6, 1.3],
        }
    }
}

/// A control cabinet's parts: its body, its door panel, and the stack light's three lamps
/// (green, amber, red from the bottom).
pub fn cabinet(p: Cabinet) -> (Mesh, Mesh, [Mesh; 3]) {
    let mut lamps = [Mesh::new(), Mesh::new(), Mesh::new()];
    if !valid_positive(&p.size) {
        return (Mesh::new(), Mesh::new(), lamps);
    }
    let [lx, ly, h] = p.size;
    let mut body = bevel_box([lx, ly, h], 0.03);
    let door = box_mesh([0.01, ly * 0.8, h * 0.75]).translated([lx / 2.0 + 0.005, 0.0, h * 0.45]);
    body.merge(&box_mesh([0.04, 0.04, 0.12]).translated([0.0, 0.0, h + 0.06]));
    for (k, lamp) in lamps.iter_mut().enumerate() {
        *lamp = box_mesh([0.09, 0.09, 0.08]).translated([0.0, 0.0, h + 0.16 + 0.085 * k as f32]);
    }
    (body, door, lamps)
}

/// A parcel: a carton of `size` (length along `x`, width, height) with tape along its top.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Parcel {
    pub size: [f32; 3],
}

impl Default for Parcel {
    fn default() -> Self {
        Self {
            size: [0.38, 0.3, 0.22],
        }
    }
}

/// A parcel's parts: the carton, and the tape.
pub fn parcel(p: Parcel) -> (Mesh, Mesh) {
    if !valid_positive(&p.size) {
        return (Mesh::new(), Mesh::new());
    }
    let [lx, ly, h] = p.size;
    let carton = bevel_box(p.size, 0.012);
    let tape =
        box_mesh([lx * 0.98, (ly * 0.22).min(0.06), 0.004]).translated([0.0, 0.0, h + 0.002]);
    (carton, tape)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    fn straight(length: f32) -> Vec<Station> {
        (0..=10)
            .map(|i| Station {
                x: length * i as f32 / 10.0,
                ..Station::default()
            })
            .collect()
    }

    #[test]
    fn a_belt_stands_on_legs_with_its_top_at_its_height_and_its_rails_beside_it() {
        let p = Belt::default();
        let parts = belt_along(&straight(5.0), p, &[], &[]);
        let (lo, hi) = parts.belt.bounds();
        assert!((hi[2] - p.height).abs() < 1e-5);
        assert!((hi[1] - p.width / 2.0).abs() < 1e-5 && (lo[0]).abs() < 1e-5);
        assert!((hi[0] - 5.0).abs() < 1e-5);
        let (flo, fhi) = parts.frame.bounds();
        assert!((fhi[2] - (p.height + p.rail_height)).abs() < 1e-5);
        assert!((fhi[1] - (p.width / 2.0 + p.rail_width)).abs() < 1e-5);
        assert!((flo[1] + p.width / 2.0 + p.rail_width).abs() < 1e-5);
        let (llo, _) = parts.legs.bounds();
        assert!(llo[2].abs() < 1e-5, "legs reach the floor");
        assert!(!parts.accent.is_empty());
    }

    #[test]
    fn a_gap_leaves_the_rail_out_on_that_side_only() {
        let p = Belt::default();
        let whole = belt_along(&straight(5.0), p, &[], &[]);
        let gapped = belt_along(&straight(5.0), p, &[(2.0, 3.0)], &[]);
        assert!(gapped.frame.triangle_count() < whole.frame.triangle_count());
        // No rail top over the gap on the left: nothing above the belt there.
        let over_gap = gapped
            .frame
            .positions
            .iter()
            .any(|v| v[0] > 2.1 && v[0] < 2.9 && v[1] > 0.0 && v[2] > p.height + 1e-4);
        assert!(!over_gap);
    }

    #[test]
    fn a_bevelled_box_keeps_its_size() {
        let m = bevel_box([1.0, 0.6, 0.4], 0.05);
        let (lo, hi) = m.bounds();
        assert_eq!(lo, [-0.5, -0.3, 0.0]);
        assert_eq!(hi, [0.5, 0.3, 0.4]);
        assert_eq!(m.triangle_count(), 2 * (4 + 4 + 2));
    }

    #[test]
    fn the_other_pieces_have_their_sizes() {
        let (frame, accent) = scanner_arch(ScannerArch::default());
        let (_, hi) = frame.bounds();
        assert!(hi[2] > 1.45 && !accent.is_empty());
        let st = pack_station(PackStation::default());
        assert!((st.top.bounds().1[2] - 0.9).abs() < 1e-5);
        assert!(!st.glow.is_empty() && !st.cartons.is_empty());
        let (wood, cartons) = pallet(Pallet::default());
        assert!(!wood.is_empty() && !cartons.is_empty());
        let (cage, base) = roll_cage(RollCage::default());
        assert!((cage.bounds().1[2] - 1.7).abs() < 1e-4 && !base.is_empty());
        let (body, door, lamps) = cabinet(Cabinet::default());
        assert!(!body.is_empty() && !door.is_empty() && lamps.iter().all(|l| !l.is_empty()));
        let (carton, tape) = parcel(Parcel::default());
        assert!((carton.bounds().1[2] - 0.22).abs() < 1e-5 && !tape.is_empty());
    }
}
