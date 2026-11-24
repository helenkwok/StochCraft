//! Parametric low-poly mesh generators.
//!
//! Conventions: units are metres, `z` is up, `x` runs along an item's length, front faces wind
//! counter-clockwise, and vertices are split across sharp edges so normals are flat per face.

use core::f32::consts::{FRAC_PI_2, TAU};

#[cfg(test)]
mod tests;

pub mod kit;

/// A flat-shaded triangle-list mesh in metres with `z` up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// An empty mesh.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the mesh contains no triangles.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// Number of triangles in the index buffer.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Appends another mesh's vertices and re-indexed triangles into `self`.
    pub fn merge(&mut self, other: &Mesh) {
        let Ok(base) = u32::try_from(self.positions.len()) else {
            return;
        };
        if u32::try_from(self.positions.len() + other.positions.len()).is_err() {
            return;
        }
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.indices.extend(other.indices.iter().map(|i| base + *i));
    }

    /// Returns a copy translated by `offset`.
    pub fn translated(&self, offset: [f32; 3]) -> Self {
        Self {
            positions: self
                .positions
                .iter()
                .map(|p| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]])
                .collect(),
            normals: self.normals.clone(),
            indices: self.indices.clone(),
        }
    }

    /// Returns a copy rotated counter-clockwise around the `z` axis by `angle` radians.
    pub fn rotated_z(&self, angle: f32) -> Self {
        let s = libm::sinf(angle);
        let c = libm::cosf(angle);
        let rot = |v: &[f32; 3]| [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]];
        Self {
            positions: self.positions.iter().map(rot).collect(),
            normals: self.normals.iter().map(rot).collect(),
            indices: self.indices.clone(),
        }
    }

    /// Axis-aligned bounding box `(min, max)`. Returns zeroes when the mesh has no vertices.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let Some(&first) = self.positions.first() else {
            return ([0.0; 3], [0.0; 3]);
        };
        let mut min = first;
        let mut max = first;
        for &p in &self.positions[1..] {
            for axis in 0..3 {
                min[axis] = min[axis].min(p[axis]);
                max[axis] = max[axis].max(p[axis]);
            }
        }
        (min, max)
    }

    fn push_triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
        let n = cross(sub(b, a), sub(c, a));
        let len = libm::sqrtf(dot(n, n));
        if len <= 1e-12 {
            return;
        }
        let Ok(base) = u32::try_from(self.positions.len()) else {
            return;
        };
        let normal = [n[0] / len, n[1] / len, n[2] / len];
        self.positions.extend_from_slice(&[a, b, c]);
        self.normals.extend_from_slice(&[normal, normal, normal]);
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    /// Appends a planar counter-clockwise quad as two triangles sharing one flat normal.
    fn push_quad(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) {
        // Newell's cross product handles quads whose opposite corners form clean diagonals.
        let n = cross(sub(c, a), sub(d, b));
        let len = libm::sqrtf(dot(n, n));
        if len <= 1e-12 {
            return;
        }
        let Ok(base) = u32::try_from(self.positions.len()) else {
            return;
        };
        let normal = [n[0] / len, n[1] / len, n[2] / len];
        self.positions.extend_from_slice(&[a, b, c, d]);
        self.normals
            .extend_from_slice(&[normal, normal, normal, normal]);
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Appends a quad split into two independently shaded triangles (for warped helicoid faces).
    fn push_split_quad(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) {
        self.push_triangle(a, b, c);
        self.push_triangle(a, c, d);
    }

    fn push_strip_quad(
        &mut self,
        a: [f32; 3],
        b: [f32; 3],
        c: [f32; 3],
        d: [f32; 3],
        planar: bool,
    ) {
        if planar {
            self.push_quad(a, b, c, d);
        } else {
            self.push_split_quad(a, b, c, d);
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn valid_positive(values: &[f32]) -> bool {
    values.iter().all(|v| v.is_finite() && *v > 0.0)
}

fn valid_non_negative(values: &[f32]) -> bool {
    values.iter().all(|v| v.is_finite() && *v >= 0.0)
}

/// Side guard thickness scaled sensibly with conveyor width so small conveyors stay well-formed.
fn guard_thickness(width: f32) -> f32 {
    (width * 0.05).clamp(0.015, 0.04).min(width * 0.25)
}

/// Generates a closed, flat-shaded box centred at the origin with dimensions `size`.
pub fn box_mesh(size: [f32; 3]) -> Mesh {
    if !valid_positive(&size) {
        return Mesh::new();
    }
    let [hx, hy, hz] = [size[0] * 0.5, size[1] * 0.5, size[2] * 0.5];
    let p000 = [-hx, -hy, -hz];
    let p100 = [hx, -hy, -hz];
    let p110 = [hx, hy, -hz];
    let p010 = [-hx, hy, -hz];
    let p001 = [-hx, -hy, hz];
    let p101 = [hx, -hy, hz];
    let p111 = [hx, hy, hz];
    let p011 = [-hx, hy, hz];

    let mut mesh = Mesh::new();
    mesh.push_quad(p100, p110, p111, p101); // +X
    mesh.push_quad(p010, p000, p001, p011); // -X
    mesh.push_quad(p110, p010, p011, p111); // +Y
    mesh.push_quad(p000, p100, p101, p001); // -Y
    mesh.push_quad(p001, p101, p111, p011); // +Z
    mesh.push_quad(p010, p110, p100, p000); // -Z
    mesh
}

/// Closed `sides`-sided prism along `z in [z0, z1]` centred at `(0, 0)` in XY.
fn prism_z(sides: u32, radius: f32, z0: f32, z1: f32) -> Mesh {
    if sides < 3 || !valid_positive(&[radius]) || !z0.is_finite() || !z1.is_finite() || z1 <= z0 {
        return Mesh::new();
    }
    let mut ring = Vec::with_capacity(sides as usize);
    for k in 0..sides {
        let a = (k as f32) * TAU / (sides as f32);
        ring.push([radius * libm::cosf(a), radius * libm::sinf(a)]);
    }

    let mut mesh = Mesh::new();
    for k in 0..sides as usize {
        let k1 = (k + 1) % (sides as usize);
        let [x0, y0] = ring[k];
        let [x1, y1] = ring[k1];
        mesh.push_quad([x0, y0, z0], [x1, y1, z0], [x1, y1, z1], [x0, y0, z1]);
    }
    let [ax, ay] = ring[0];
    for k in 1..(sides as usize - 1) {
        let [bx, by] = ring[k];
        let [cx, cy] = ring[k + 1];
        mesh.push_triangle([ax, ay, z1], [bx, by, z1], [cx, cy, z1]);
        mesh.push_triangle([ax, ay, z0], [cx, cy, z0], [bx, by, z0]);
    }
    mesh
}

/// Cross-section corners at one station along a swept conveyor slab or guard.
struct Section {
    rt: [f32; 3],
    lt: [f32; 3],
    lb: [f32; 3],
    rb: [f32; 3],
}

fn emit_swept_strip(mesh: &mut Mesh, sections: &[Section], planar_sides: bool) {
    if sections.len() < 2 {
        return;
    }
    for pair in sections.windows(2) {
        let s0 = &pair[0];
        let s1 = &pair[1];
        mesh.push_strip_quad(s0.rt, s1.rt, s1.lt, s0.lt, planar_sides);
        mesh.push_strip_quad(s0.lb, s1.lb, s1.rb, s0.rb, planar_sides);
        mesh.push_strip_quad(s0.lt, s1.lt, s1.lb, s0.lb, planar_sides);
        mesh.push_strip_quad(s1.rt, s0.rt, s0.rb, s1.rb, planar_sides);
    }
    let first = &sections[0];
    mesh.push_quad(first.lt, first.lb, first.rb, first.rt);
    if let Some(last) = sections.last() {
        mesh.push_quad(last.rt, last.rb, last.lb, last.lt);
    }
}

/// Leg centre positions along a span of length `span`, keeping legs inside `[0, span]`.
fn leg_positions(span: f32, leg_size: f32, leg_spacing: f32) -> Vec<f32> {
    if span <= leg_size {
        return vec![span * 0.5];
    }
    let n_spans = libm::ceilf(span / leg_spacing).max(1.0) as u32;
    let start = leg_size * 0.5;
    let end = span - leg_size * 0.5;
    (0..=n_spans)
        .map(|i| start + (end - start) * (i as f32 / n_spans as f32))
        .collect()
}

/// Transport surface style for a straight conveyor.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ConveyorKind {
    #[default]
    Belt,
    Rollers {
        pitch: f32,
        diameter: f32,
    },
}

/// Parameters for a straight belt or roller conveyor running along `+x` from `x = 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StraightConveyor {
    pub length: f32,
    pub width: f32,
    pub height: f32,
    pub belt_thickness: f32,
    pub leg_spacing: f32,
    pub leg_size: f32,
    pub guard_height: f32,
    pub kind: ConveyorKind,
}

impl Default for StraightConveyor {
    fn default() -> Self {
        Self {
            length: 3.0,
            width: 0.6,
            height: 0.85,
            belt_thickness: 0.08,
            leg_spacing: 2.0,
            leg_size: 0.05,
            guard_height: 0.06,
            kind: ConveyorKind::Belt,
        }
    }
}

impl StraightConveyor {
    /// Total travel distance along the conveyor centre line.
    pub fn path_length(&self) -> f32 {
        if self.length.is_finite() && self.length > 0.0 {
            self.length
        } else {
            0.0
        }
    }

    /// Position on the transport surface centre line at `distance` metres from the start.
    pub fn point_at(&self, distance: f32) -> [f32; 3] {
        let len = self.path_length();
        let d = if distance.is_finite() {
            distance.clamp(0.0, len)
        } else {
            0.0
        };
        [d, 0.0, self.height]
    }
}

/// Generates a straight conveyor with belt or hexagonal rollers, side guards and legs.
pub fn straight_conveyor(params: StraightConveyor) -> Mesh {
    if !valid_positive(&[
        params.length,
        params.width,
        params.height,
        params.belt_thickness,
        params.leg_spacing,
        params.leg_size,
    ]) || !valid_non_negative(&[params.guard_height])
        || params.belt_thickness > params.height
    {
        return Mesh::new();
    }

    let mut mesh = Mesh::new();
    match params.kind {
        ConveyorKind::Belt => {
            let slab = box_mesh([params.length, params.width, params.belt_thickness]).translated([
                params.length * 0.5,
                0.0,
                params.height - params.belt_thickness * 0.5,
            ]);
            mesh.merge(&slab);
        }
        ConveyorKind::Rollers { pitch, diameter } => {
            if !valid_positive(&[pitch, diameter])
                || diameter > params.length
                || diameter > params.height
            {
                return Mesh::new();
            }
            let r = diameter * 0.5;
            let count = libm::floorf((params.length - diameter) / pitch) as u32 + 1;
            let span = (count - 1) as f32 * pitch;
            let x0 = r + 0.5 * (params.length - diameter - span);
            let hex = prism_z(6, r, -params.width * 0.5, params.width * 0.5);
            // Proper rotation (x, y, z) -> (y, z, x) maps prism Z to Y and puts vertex 0 at +Z.
            let roller = Mesh {
                positions: hex.positions.iter().map(|p| [p[1], p[2], p[0]]).collect(),
                normals: hex.normals.iter().map(|n| [n[1], n[2], n[0]]).collect(),
                indices: hex.indices,
            };
            let z_center = params.height - r;
            for i in 0..count {
                let x = x0 + i as f32 * pitch;
                mesh.merge(&roller.translated([x, 0.0, z_center]));
            }
        }
    }

    let emit_guards =
        params.guard_height > 0.0 || matches!(params.kind, ConveyorKind::Rollers { .. });
    if emit_guards {
        let guard_t = guard_thickness(params.width);
        let guard_h = params.belt_thickness + params.guard_height;
        let guard_z = params.height - params.belt_thickness + guard_h * 0.5;
        let guard_y = params.width * 0.5 + guard_t * 0.5;
        let guard = box_mesh([params.length, guard_t, guard_h]);
        mesh.merge(&guard.translated([params.length * 0.5, guard_y, guard_z]));
        mesh.merge(&guard.translated([params.length * 0.5, -guard_y, guard_z]));
    }

    let leg_h = params.height - params.belt_thickness;
    if leg_h > 0.0 {
        let leg = box_mesh([params.leg_size, params.leg_size, leg_h]);
        let y_off = (params.width - params.leg_size).max(0.0) * 0.5;
        for x in leg_positions(params.length, params.leg_size, params.leg_spacing) {
            mesh.merge(&leg.translated([x, y_off, leg_h * 0.5]));
            mesh.merge(&leg.translated([x, -y_off, leg_h * 0.5]));
        }
    }

    mesh
}

/// Parameters for a curved conveyor starting at `(0, 0, height)` heading along `+x`.
///
/// Positive `angle` turns left (`+y`), negative turns right (`-y`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvedConveyor {
    pub radius: f32,
    pub angle: f32,
    pub width: f32,
    pub height: f32,
    pub belt_thickness: f32,
    pub leg_spacing: f32,
    pub leg_size: f32,
    pub guard_height: f32,
    pub segments_per_quarter: u32,
}

impl Default for CurvedConveyor {
    fn default() -> Self {
        Self {
            radius: 1.2,
            angle: FRAC_PI_2,
            width: 0.6,
            height: 0.85,
            belt_thickness: 0.08,
            leg_spacing: 2.0,
            leg_size: 0.05,
            guard_height: 0.06,
            segments_per_quarter: 8,
        }
    }
}

impl CurvedConveyor {
    /// Total centre-line arc length (`radius * |angle|`).
    pub fn path_length(&self) -> f32 {
        if !valid_positive(&[self.radius]) || !self.angle.is_finite() || self.angle == 0.0 {
            0.0
        } else {
            self.radius * self.angle.abs()
        }
    }

    /// Position on the transport surface centre line at `distance` metres from the start.
    pub fn point_at(&self, distance: f32) -> [f32; 3] {
        let len = self.path_length();
        if len <= 0.0 {
            return [0.0, 0.0, self.height];
        }
        let d = if distance.is_finite() {
            distance.clamp(0.0, len)
        } else {
            0.0
        };
        let u = d / self.radius;
        let sign = if self.angle >= 0.0 { 1.0 } else { -1.0 };
        [
            self.radius * libm::sinf(u),
            sign * self.radius * (1.0 - libm::cosf(u)),
            self.height,
        ]
    }
}

/// Generates a curved conveyor swept along its arc with side guards and legs.
///
/// Returns an empty mesh if `angle == 0` or parameters are non-positive or self-intersecting.
pub fn curved_conveyor(params: CurvedConveyor) -> Mesh {
    if !valid_positive(&[
        params.radius,
        params.width,
        params.height,
        params.belt_thickness,
        params.leg_spacing,
        params.leg_size,
    ]) || !valid_non_negative(&[params.guard_height])
        || !params.angle.is_finite()
        || params.angle == 0.0
        || params.belt_thickness > params.height
        || params.segments_per_quarter == 0
    {
        return Mesh::new();
    }

    let guard_t = guard_thickness(params.width);
    if params.radius <= params.width * 0.5 + guard_t {
        return Mesh::new();
    }

    let abs_angle = params.angle.abs();
    let sign = if params.angle >= 0.0 { 1.0 } else { -1.0 };
    let n_segs = libm::ceilf((abs_angle / FRAC_PI_2) * (params.segments_per_quarter as f32) - 1e-5)
        .max(1.0) as u32;

    let build_strip = |w_left: f32, w_right: f32, z_bot: f32, z_top: f32| -> Vec<Section> {
        (0..=n_segs)
            .map(|i| {
                let u = abs_angle * (i as f32 / n_segs as f32);
                let theta = sign * u;
                let cx = params.radius * libm::sinf(u);
                let cy = sign * params.radius * (1.0 - libm::cosf(u));
                let lx = -libm::sinf(theta);
                let ly = libm::cosf(theta);
                Section {
                    rt: [cx + w_right * lx, cy + w_right * ly, z_top],
                    lt: [cx + w_left * lx, cy + w_left * ly, z_top],
                    lb: [cx + w_left * lx, cy + w_left * ly, z_bot],
                    rb: [cx + w_right * lx, cy + w_right * ly, z_bot],
                }
            })
            .collect()
    };

    let mut mesh = Mesh::new();
    let hw = params.width * 0.5;
    let z_bot = params.height - params.belt_thickness;
    let slab_sections = build_strip(hw, -hw, z_bot, params.height);
    emit_swept_strip(&mut mesh, &slab_sections, true);

    if params.guard_height > 0.0 {
        let g_top = params.height + params.guard_height;
        let left_guard = build_strip(hw + guard_t, hw, z_bot, g_top);
        let right_guard = build_strip(-hw, -(hw + guard_t), z_bot, g_top);
        emit_swept_strip(&mut mesh, &left_guard, true);
        emit_swept_strip(&mut mesh, &right_guard, true);
    }

    let leg_h = params.height - params.belt_thickness;
    if leg_h > 0.0 {
        let arc_len = params.radius * abs_angle;
        let y_off = (params.width - params.leg_size).max(0.0) * 0.5;
        let leg = box_mesh([params.leg_size, params.leg_size, leg_h]);
        let pair = {
            let mut m = Mesh::new();
            m.merge(&leg.translated([0.0, y_off, leg_h * 0.5]));
            m.merge(&leg.translated([0.0, -y_off, leg_h * 0.5]));
            m
        };
        for d in leg_positions(arc_len, params.leg_size, params.leg_spacing) {
            let u = d / params.radius;
            let theta = sign * u;
            let cx = params.radius * libm::sinf(u);
            let cy = sign * params.radius * (1.0 - libm::cosf(u));
            mesh.merge(&pair.rotated_z(theta).translated([cx, cy, 0.0]));
        }
    }

    mesh
}

/// Parameters for a helical spiral conveyor centred on the `z` axis.
///
/// The helix starts at `(radius, 0, height)` and winds counter-clockwise around the `z` axis for
/// `turns` revolutions, climbing by `rise` metres per turn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpiralConveyor {
    pub radius: f32,
    pub rise: f32,
    pub turns: f32,
    pub width: f32,
    pub height: f32,
    pub belt_thickness: f32,
    pub guard_height: f32,
    pub column_radius: f32,
    pub segments_per_turn: u32,
}

impl Default for SpiralConveyor {
    fn default() -> Self {
        Self {
            radius: 1.2,
            rise: 1.5,
            turns: 2.0,
            width: 0.6,
            height: 0.5,
            belt_thickness: 0.08,
            guard_height: 0.06,
            column_radius: 0.25,
            segments_per_turn: 24,
        }
    }
}

impl SpiralConveyor {
    /// Total helical centre-line path length: `turns * sqrt((2 * pi * radius)^2 + rise^2)`.
    pub fn path_length(&self) -> f32 {
        if !valid_positive(&[self.radius, self.turns]) || !self.rise.is_finite() {
            return 0.0;
        }
        let circumference = TAU * self.radius;
        self.turns * libm::sqrtf(circumference * circumference + self.rise * self.rise)
    }

    /// Position on the transport surface centre line at `distance` metres from the start.
    pub fn point_at(&self, distance: f32) -> [f32; 3] {
        let len = self.path_length();
        if len <= 0.0 {
            return [self.radius, 0.0, self.height];
        }
        let d = if distance.is_finite() {
            distance.clamp(0.0, len)
        } else {
            0.0
        };
        let t = d / len;
        let theta = TAU * self.turns * t;
        [
            self.radius * libm::cosf(theta),
            self.radius * libm::sinf(theta),
            self.height + self.rise * self.turns * t,
        ]
    }
}

/// Generates a helical spiral conveyor with ramp slab, inner/outer guards and central column.
///
/// Returns an empty mesh if `turns <= 0` or parameters are invalid.
pub fn spiral_conveyor(params: SpiralConveyor) -> Mesh {
    if !valid_positive(&[
        params.radius,
        params.turns,
        params.width,
        params.belt_thickness,
        params.column_radius,
    ]) || !valid_non_negative(&[params.guard_height])
        || !params.rise.is_finite()
        || !params.height.is_finite()
        || params.height < params.belt_thickness
        || params.height + params.rise * params.turns < params.belt_thickness
        || params.segments_per_turn == 0
    {
        return Mesh::new();
    }

    let guard_t = guard_thickness(params.width);
    let inner_clear = params.radius - params.width * 0.5 - guard_t;
    if inner_clear <= 0.0 {
        return Mesh::new();
    }

    let n_segs =
        libm::ceilf(params.turns * (params.segments_per_turn as f32) - 1e-5).max(1.0) as u32;

    let build_helix = |r_in: f32, r_out: f32, dz_bot: f32, dz_top: f32| -> Vec<Section> {
        (0..=n_segs)
            .map(|i| {
                let t = i as f32 / n_segs as f32;
                let theta = TAU * params.turns * t;
                let cos_t = libm::cosf(theta);
                let sin_t = libm::sinf(theta);
                let z = params.height + params.rise * params.turns * t;
                Section {
                    rt: [r_out * cos_t, r_out * sin_t, z + dz_top],
                    lt: [r_in * cos_t, r_in * sin_t, z + dz_top],
                    lb: [r_in * cos_t, r_in * sin_t, z + dz_bot],
                    rb: [r_out * cos_t, r_out * sin_t, z + dz_bot],
                }
            })
            .collect()
    };

    let mut mesh = Mesh::new();
    let r_in = params.radius - params.width * 0.5;
    let r_out = params.radius + params.width * 0.5;
    let slab = build_helix(r_in, r_out, -params.belt_thickness, 0.0);
    emit_swept_strip(&mut mesh, &slab, false);

    if params.guard_height > 0.0 {
        let inner_guard = build_helix(
            r_in - guard_t,
            r_in,
            -params.belt_thickness,
            params.guard_height,
        );
        let outer_guard = build_helix(
            r_out,
            r_out + guard_t,
            -params.belt_thickness,
            params.guard_height,
        );
        emit_swept_strip(&mut mesh, &inner_guard, false);
        emit_swept_strip(&mut mesh, &outer_guard, false);
    }

    let col_r = params.column_radius.min(inner_clear * 0.8);
    let max_z = params
        .height
        .max(params.height + params.rise * params.turns)
        + params.guard_height;
    mesh.merge(&prism_z(8, col_r, 0.0, max_z));

    mesh
}

/// Parameters for a selective pallet rack centred in XY on `z = 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rack {
    pub bays: u32,
    pub levels: u32,
    pub bay_width: f32,
    pub depth: f32,
    pub level_height: f32,
    pub beam_size: f32,
    pub upright_size: f32,
}

impl Default for Rack {
    fn default() -> Self {
        Self {
            bays: 3,
            levels: 3,
            bay_width: 2.7,
            depth: 1.1,
            level_height: 1.5,
            beam_size: 0.1,
            upright_size: 0.08,
        }
    }
}

/// Generates a pallet rack from uprights and horizontal front/back beams.
pub fn rack(params: Rack) -> Mesh {
    if params.bays == 0
        || params.levels == 0
        || !valid_positive(&[
            params.bay_width,
            params.depth,
            params.level_height,
            params.beam_size,
            params.upright_size,
        ])
        || params.bay_width <= params.upright_size
        || params.depth <= params.upright_size
        || params.depth <= params.beam_size
        || params.level_height <= params.beam_size
    {
        return Mesh::new();
    }

    let mut mesh = Mesh::new();
    let total_width = params.bays as f32 * params.bay_width;
    let total_height = params.levels as f32 * params.level_height;
    let x0 = -total_width * 0.5;

    let upright = box_mesh([params.upright_size, params.upright_size, total_height]);
    let upright_y = (params.depth - params.upright_size) * 0.5;
    for b in 0..=params.bays {
        let x = x0 + b as f32 * params.bay_width;
        mesh.merge(&upright.translated([x, upright_y, total_height * 0.5]));
        mesh.merge(&upright.translated([x, -upright_y, total_height * 0.5]));
    }

    let beam_len = params.bay_width - params.upright_size;
    let beam = box_mesh([beam_len, params.beam_size, params.beam_size]);
    let beam_y = (params.depth - params.beam_size) * 0.5;
    for b in 0..params.bays {
        let x = x0 + (b as f32 + 0.5) * params.bay_width;
        for l in 1..=params.levels {
            let z = l as f32 * params.level_height - params.beam_size * 0.5;
            mesh.merge(&beam.translated([x, beam_y, z]));
            mesh.merge(&beam.translated([x, -beam_y, z]));
        }
    }

    mesh
}

/// Parameters for a machine workstation centred in XY on `z = 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Machine {
    pub size: [f32; 3],
}

impl Default for Machine {
    fn default() -> Self {
        Self {
            size: [1.6, 1.2, 1.4],
        }
    }
}

/// Generates a machine housing with a smaller control-panel box on top.
pub fn machine(params: Machine) -> Mesh {
    if !valid_positive(&params.size) {
        return Mesh::new();
    }
    let [sx, sy, sz] = params.size;
    let body_h = sz * 0.8;
    let panel_h = sz - body_h;
    let panel_x = sx * 0.35;
    let panel_y = sy * 0.4;

    let mut mesh = box_mesh([sx, sy, body_h]).translated([0.0, 0.0, body_h * 0.5]);
    let panel = box_mesh([panel_x, panel_y, panel_h]).translated([
        sx * 0.5 - panel_x * 0.6,
        -(sy * 0.5 - panel_y * 0.6),
        body_h + panel_h * 0.5,
    ]);
    mesh.merge(&panel);
    mesh
}

/// Parameters for an open-top tote or container centred in XY on `z = 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tote {
    pub size: [f32; 3],
}

impl Default for Tote {
    fn default() -> Self {
        Self {
            size: [0.6, 0.4, 0.3],
        }
    }
}

/// Generates an open-top tote from a base slab and four walls.
pub fn tote(params: Tote) -> Mesh {
    if !valid_positive(&params.size) {
        return Mesh::new();
    }
    let [sx, sy, sz] = params.size;
    let t = (sx.min(sy) * 0.08)
        .clamp(0.01, 0.025)
        .min(sx.min(sy) * 0.25)
        .min(sz * 0.25);
    let wall_h = sz - t;

    let mut mesh = box_mesh([sx, sy, t]).translated([0.0, 0.0, t * 0.5]);
    let fb_wall = box_mesh([t, sy, wall_h]);
    let x_off = sx * 0.5 - t * 0.5;
    let wall_z = t + wall_h * 0.5;
    mesh.merge(&fb_wall.translated([x_off, 0.0, wall_z]));
    mesh.merge(&fb_wall.translated([-x_off, 0.0, wall_z]));

    let lr_len = sx - 2.0 * t;
    if lr_len > 0.0 {
        let lr_wall = box_mesh([lr_len, t, wall_h]);
        let y_off = sy * 0.5 - t * 0.5;
        mesh.merge(&lr_wall.translated([0.0, y_off, wall_z]));
        mesh.merge(&lr_wall.translated([0.0, -y_off, wall_z]));
    }
    mesh
}

/// Closed `sides`-sided capsule (or sphere when `cyl_height == 0`) sitting on `z = 0`.
fn capsule_z(radius: f32, cyl_height: f32, sides: u32, cap_rings: u32) -> Mesh {
    if sides < 3
        || cap_rings == 0
        || !valid_positive(&[radius])
        || !valid_non_negative(&[cyl_height])
    {
        return Mesh::new();
    }
    let mut ring_specs: Vec<(f32, f32)> = Vec::new();
    let z_bot = radius;
    let z_top = radius + cyl_height;

    for r in 1..cap_rings {
        let phi = -FRAC_PI_2 + FRAC_PI_2 * (r as f32 / cap_rings as f32);
        ring_specs.push((z_bot + radius * libm::sinf(phi), radius * libm::cosf(phi)));
    }
    ring_specs.push((z_bot, radius));
    if cyl_height > 0.0 {
        ring_specs.push((z_top, radius));
    }
    for r in 1..cap_rings {
        let phi = FRAC_PI_2 * (r as f32 / cap_rings as f32);
        ring_specs.push((z_top + radius * libm::sinf(phi), radius * libm::cosf(phi)));
    }

    let unit_circle: Vec<[f32; 2]> = (0..sides)
        .map(|k| {
            let a = k as f32 * TAU / sides as f32;
            [libm::cosf(a), libm::sinf(a)]
        })
        .collect();
    let rings: Vec<Vec<[f32; 3]>> = ring_specs
        .iter()
        .map(|&(z, rad)| {
            unit_circle
                .iter()
                .map(|&[ux, uy]| [rad * ux, rad * uy, z])
                .collect()
        })
        .collect();

    let mut mesh = Mesh::new();
    let south = [0.0, 0.0, 0.0];
    let north = [0.0, 0.0, 2.0 * radius + cyl_height];
    let n = sides as usize;

    if let Some(first_ring) = rings.first() {
        for s in 0..n {
            let s1 = (s + 1) % n;
            mesh.push_triangle(south, first_ring[s1], first_ring[s]);
        }
    }
    for pair in rings.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        for s in 0..n {
            let s1 = (s + 1) % n;
            mesh.push_quad(a[s], a[s1], b[s1], b[s]);
        }
    }
    if let Some(last_ring) = rings.last() {
        for s in 0..n {
            let s1 = (s + 1) % n;
            mesh.push_triangle(last_ring[s], last_ring[s1], north);
        }
    }
    mesh
}

/// Parameters for a standing human operator centred in XY on `z = 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Operator {
    pub height: f32,
}

impl Default for Operator {
    fn default() -> Self {
        Self { height: 1.75 }
    }
}

/// Generates a low-poly operator with an 8-sided capsule body and a low-poly head.
pub fn operator(params: Operator) -> Mesh {
    if !valid_positive(&[params.height]) {
        return Mesh::new();
    }
    let h = params.height;
    let body_r = h * 0.12;
    let head_r = h * 0.07;
    let body_total = h * 0.84;
    let cyl_h = (body_total - 2.0 * body_r).max(0.0);

    let mut mesh = capsule_z(body_r, cyl_h, 8, 2);
    let head = capsule_z(head_r, 0.0, 8, 2).translated([0.0, 0.0, h - 2.0 * head_r]);
    mesh.merge(&head);
    mesh
}

/// Parameters for a counterbalance forklift facing `+x`, centred in XY on `z = 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forklift {
    pub size: [f32; 3],
}

impl Default for Forklift {
    fn default() -> Self {
        Self {
            size: [2.2, 1.1, 2.1],
        }
    }
}

/// Generates a forklift from body, cab, mast uprights, fork carriage, forks and wheels.
pub fn forklift(params: Forklift) -> Mesh {
    if !valid_positive(&params.size) {
        return Mesh::new();
    }
    let [sx, sy, sz] = params.size;
    let mut mesh = Mesh::new();

    // Counterweight chassis spanning the rear 60% of length.
    let body_len = sx * 0.60;
    let body_h = sz * 0.40;
    mesh.merge(&box_mesh([body_len, sy, body_h]).translated([
        -sx * 0.5 + body_len * 0.5,
        0.0,
        sz * 0.05 + body_h * 0.5,
    ]));

    // Overhead cab guard.
    let cab_len = sx * 0.40;
    let cab_h = sz * 0.47;
    mesh.merge(&box_mesh([cab_len, sy * 0.80, cab_h]).translated([
        -sx * 0.15,
        0.0,
        sz * 0.45 + cab_h * 0.5,
    ]));

    // Twin vertical mast channels reaching total height `sz`.
    let mast_x = sx * 0.14;
    let mast_thick = sx * 0.08;
    let mast_w = sy * 0.12;
    let mast_h = sz * 0.95;
    let mast_y = sy * 0.28;
    let mast = box_mesh([mast_thick, mast_w, mast_h]);
    mesh.merge(&mast.translated([mast_x, mast_y, sz * 0.05 + mast_h * 0.5]));
    mesh.merge(&mast.translated([mast_x, -mast_y, sz * 0.05 + mast_h * 0.5]));

    // Fork carriage crossbeam.
    mesh.merge(&box_mesh([sx * 0.05, sy * 0.68, sz * 0.14]).translated([
        sx * 0.175,
        0.0,
        sz * 0.15,
    ]));

    // Two forks reaching `+sx * 0.5`.
    let fork_len = sx * 0.30;
    let fork = box_mesh([fork_len, sy * 0.12, sz * 0.05]);
    let fork_x = sx * 0.5 - fork_len * 0.5;
    let fork_y = sy * 0.21;
    let fork_z = sz * 0.045;
    mesh.merge(&fork.translated([fork_x, fork_y, fork_z]));
    mesh.merge(&fork.translated([fork_x, -fork_y, fork_z]));

    // Four low-poly wheel blocks resting on `z = 0`.
    let front_wheel = box_mesh([sx * 0.14, sy * 0.14, sz * 0.18]);
    let fw_y = sy * 0.41;
    mesh.merge(&front_wheel.translated([sx * 0.05, fw_y, sz * 0.09]));
    mesh.merge(&front_wheel.translated([sx * 0.05, -fw_y, sz * 0.09]));

    let rear_wheel = box_mesh([sx * 0.14, sy * 0.14, sz * 0.16]);
    let rw_y = sy * 0.37;
    mesh.merge(&rear_wheel.translated([-sx * 0.37, rw_y, sz * 0.08]));
    mesh.merge(&rear_wheel.translated([-sx * 0.37, -rw_y, sz * 0.08]));

    mesh
}

/// Parameters for an automated guided vehicle (AGV) facing `+x`, centred in XY on `z = 0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Agv {
    pub size: [f32; 3],
}

impl Default for Agv {
    fn default() -> Self {
        Self {
            size: [1.2, 0.8, 0.35],
        }
    }
}

/// Generates a flat AGV with a drive chassis, raised load deck and front sensor bar.
pub fn agv(params: Agv) -> Mesh {
    if !valid_positive(&params.size) {
        return Mesh::new();
    }
    let [sx, sy, sz] = params.size;
    let body_h = sz * 0.65;
    let deck_h = sz - body_h;

    let mut mesh = box_mesh([sx, sy, body_h]).translated([0.0, 0.0, body_h * 0.5]);
    mesh.merge(&box_mesh([sx * 0.84, sy * 0.84, deck_h]).translated([
        0.0,
        0.0,
        body_h + deck_h * 0.5,
    ]));
    mesh.merge(&box_mesh([sx * 0.06, sy * 0.40, deck_h * 0.6]).translated([
        sx * 0.46,
        0.0,
        body_h + deck_h * 0.3,
    ]));
    mesh
}
