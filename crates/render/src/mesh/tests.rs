use core::f32::consts::{FRAC_PI_2, TAU};
use std::collections::BTreeMap;

use super::*;

fn close(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

fn close3(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
    close(a[0], b[0], eps) && close(a[1], b[1], eps) && close(a[2], b[2], eps)
}

fn assert_valid_mesh(mesh: &Mesh) {
    assert!(!mesh.is_empty(), "expected non-empty mesh");
    assert_eq!(mesh.indices.len() % 3, 0, "indices must form triangles");
    assert_eq!(
        mesh.positions.len(),
        mesh.normals.len(),
        "positions and normals must have equal length"
    );
    let vert_count = mesh.positions.len();
    for tri in mesh.indices.chunks_exact(3) {
        let [i0, i1, i2] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        assert!(i0 < vert_count && i1 < vert_count && i2 < vert_count);
        let p0 = mesh.positions[i0];
        let p1 = mesh.positions[i1];
        let p2 = mesh.positions[i2];
        let geom = cross(sub(p1, p0), sub(p2, p0));
        for idx in [i0, i1, i2] {
            let n = mesh.normals[idx];
            let len = libm::sqrtf(dot(n, n));
            assert!(close(len, 1.0, 1e-5), "normal {n:?} length {len} != 1");
            assert!(
                dot(n, geom) > 0.0,
                "normal {n:?} disagrees with CCW winding {geom:?}"
            );
        }
    }
}

fn signed_volume(mesh: &Mesh) -> f32 {
    let mut sum = 0.0_f32;
    for tri in mesh.indices.chunks_exact(3) {
        let a = mesh.positions[tri[0] as usize];
        let b = mesh.positions[tri[1] as usize];
        let c = mesh.positions[tri[2] as usize];
        sum += dot(a, cross(b, c));
    }
    sum / 6.0
}

fn quantize(p: [f32; 3]) -> [i64; 3] {
    [
        libm::roundf(p[0] * 1e5) as i64,
        libm::roundf(p[1] * 1e5) as i64,
        libm::roundf(p[2] * 1e5) as i64,
    ]
}

fn assert_watertight(mesh: &Mesh) {
    let mut edges: BTreeMap<([i64; 3], [i64; 3]), u32> = BTreeMap::new();
    for tri in mesh.indices.chunks_exact(3) {
        let pts = [
            quantize(mesh.positions[tri[0] as usize]),
            quantize(mesh.positions[tri[1] as usize]),
            quantize(mesh.positions[tri[2] as usize]),
        ];
        for (u, v) in [(pts[0], pts[1]), (pts[1], pts[2]), (pts[2], pts[0])] {
            let key = if u <= v { (u, v) } else { (v, u) };
            *edges.entry(key).or_insert(0) += 1;
        }
    }
    assert!(!edges.is_empty());
    for (edge, count) in edges {
        assert_eq!(count, 2, "edge {edge:?} shared by {count} triangles");
    }
}

#[test]
fn box_mesh_is_closed_and_outward() {
    let m = box_mesh([2.0, 3.0, 4.0]);
    assert_valid_mesh(&m);
    assert_watertight(&m);
    // Formula: 6 faces * 2 triangles = 12 triangles.
    assert_eq!(m.triangle_count(), 12);

    let (min, max) = m.bounds();
    assert!(close3(min, [-1.0, -1.5, -2.0], 1e-6));
    assert!(close3(max, [1.0, 1.5, 2.0], 1e-6));

    let centre = [0.0, 0.0, 0.0];
    for tri in m.indices.chunks_exact(3) {
        let a = m.positions[tri[0] as usize];
        let b = m.positions[tri[1] as usize];
        let c = m.positions[tri[2] as usize];
        let centroid = [
            (a[0] + b[0] + c[0]) / 3.0,
            (a[1] + b[1] + c[1]) / 3.0,
            (a[2] + b[2] + c[2]) / 3.0,
        ];
        let n = m.normals[tri[0] as usize];
        assert!(dot(n, sub(centroid, centre)) > 0.0);
    }
    assert!(close(signed_volume(&m), 24.0, 1e-4));
}

#[test]
fn mesh_helpers_transform_and_merge() {
    let a = box_mesh([1.0, 2.0, 3.0]).translated([5.0, 0.0, 1.5]);
    let (min, max) = a.bounds();
    assert!(close3(min, [4.5, -1.0, 0.0], 1e-6));
    assert!(close3(max, [5.5, 1.0, 3.0], 1e-6));

    let rot = box_mesh([2.0, 1.0, 1.0]).rotated_z(FRAC_PI_2);
    assert_valid_mesh(&rot);
    let (rmin, rmax) = rot.bounds();
    assert!(close3(rmin, [-0.5, -1.0, -0.5], 1e-5));
    assert!(close3(rmax, [0.5, 1.0, 0.5], 1e-5));

    let mut combined = a;
    combined.merge(&rot);
    assert_valid_mesh(&combined);
    assert_eq!(combined.triangle_count(), 24);
    assert_eq!(Mesh::new().bounds(), ([0.0; 3], [0.0; 3]));
}

#[test]
fn straight_belt_conveyor_bounds_counts_and_watertight_slab() {
    let params = StraightConveyor {
        length: 5.0,
        width: 0.8,
        height: 0.9,
        leg_spacing: 2.0,
        guard_height: 0.08,
        ..Default::default()
    };
    let m = straight_conveyor(params);
    assert_valid_mesh(&m);
    assert!(signed_volume(&m) > 0.0);

    // Formula:
    //   slab: 12
    //   2 guards: 2 * 12 = 24
    //   leg pairs = ceil(5.0 / 2.0) + 1 = 4 pairs = 8 legs * 12 = 96
    //   total = 12 + 24 + 96 = 132
    assert_eq!(m.triangle_count(), 132);

    let (min, max) = m.bounds();
    assert!(close(min[0], 0.0, 1e-6));
    assert!(close(max[0], 5.0, 1e-6));
    assert!(max[1] - min[1] >= 0.8 - 1e-6);
    assert!(close(min[2], 0.0, 1e-6));
    assert!(close(max[2], params.height + params.guard_height, 1e-6));

    // Slab alone (guard_height = 0, height = belt_thickness so no legs) is watertight.
    let slab_only = straight_conveyor(StraightConveyor {
        height: params.belt_thickness,
        guard_height: 0.0,
        ..params
    });
    assert_valid_mesh(&slab_only);
    assert_watertight(&slab_only);
    assert_eq!(slab_only.triangle_count(), 12);

    // Default count: length 3.0, leg_spacing 2.0 -> ceil(3/2)+1 = 3 pairs = 6 legs = 72,
    // plus 12 slab + 24 guards = 108.
    assert_eq!(
        straight_conveyor(StraightConveyor::default()).triangle_count(),
        108
    );
}

#[test]
fn straight_roller_conveyor_counts_and_bounds() {
    let params = StraightConveyor {
        length: 3.0,
        width: 0.6,
        height: 0.85,
        kind: ConveyorKind::Rollers {
            pitch: 0.15,
            diameter: 0.05,
        },
        ..Default::default()
    };
    let m = straight_conveyor(params);
    assert_valid_mesh(&m);
    assert!(signed_volume(&m) > 0.0);

    // Formula:
    //   n_rollers = floor((3.0 - 0.05) / 0.15) + 1 = 20
    //   6-sided prism = 6*2 + 2*(6-2) = 20 triangles each -> 20 * 20 = 400
    //   2 guards = 24
    //   3 leg pairs = 6 * 12 = 72
    //   total = 496
    assert_eq!(m.triangle_count(), 496);

    let (min, max) = m.bounds();
    assert!(min[0] >= -1e-6 && max[0] <= params.length + 1e-6);
    assert!(max[1] - min[1] >= params.width - 1e-6);
    assert!(close(min[2], 0.0, 1e-6));
    assert!(close(max[2], params.height + params.guard_height, 1e-6));
}

#[test]
fn curved_conveyor_counts_bounds_and_watertight_slab() {
    let params = CurvedConveyor::default();
    let m = curved_conveyor(params);
    assert_valid_mesh(&m);
    assert!(signed_volume(&m) > 0.0);

    // Formula for default (90 deg, segments_per_quarter = 8 -> n_segs = 8, arc_len ~ 1.885 <= 2.0):
    //   slab: 8 * n_segs + 4 = 68
    //   2 guards: 2 * 68 = 136
    //   2 leg pairs = 4 legs * 12 = 48
    //   total = 252
    assert_eq!(m.triangle_count(), 252);

    let (min, max) = m.bounds();
    assert!(close(min[2], 0.0, 1e-6));
    assert!(close(max[2], params.height + params.guard_height, 1e-6));

    // Right turn (-angle) mirrors Y bounds cleanly.
    let right = curved_conveyor(CurvedConveyor {
        angle: -FRAC_PI_2,
        ..params
    });
    assert_valid_mesh(&right);
    assert_eq!(right.triangle_count(), 252);
    let (rmin, rmax) = right.bounds();
    assert!(close(rmin[1], -max[1], 1e-5));
    assert!(close(rmax[1], -min[1], 1e-5));

    // Slab alone is watertight.
    let slab_only = curved_conveyor(CurvedConveyor {
        height: params.belt_thickness,
        guard_height: 0.0,
        ..params
    });
    assert_valid_mesh(&slab_only);
    assert_watertight(&slab_only);
    assert_eq!(slab_only.triangle_count(), 8 * 8 + 4);
}

#[test]
fn spiral_conveyor_counts_bounds_and_watertight_slab() {
    let params = SpiralConveyor::default();
    let m = spiral_conveyor(params);
    assert_valid_mesh(&m);
    assert!(signed_volume(&m) > 0.0);

    // Formula for default (turns = 2, segments_per_turn = 24 -> n_segs = 48):
    //   ramp slab: 8 * 48 + 4 = 388
    //   2 guards: 2 * 388 = 776
    //   8-sided column: 8 * 2 + 2 * (8 - 2) = 28
    //   total = 1192
    assert_eq!(m.triangle_count(), 1192);

    let (min, max) = m.bounds();
    let outer = params.radius + params.width * 0.5 + guard_thickness(params.width);
    assert!(close(min[0], -outer, 1e-4));
    assert!(close(max[0], outer, 1e-4));
    assert!(close(min[1], -outer, 1e-4));
    assert!(close(max[1], outer, 1e-4));
    assert!(close(min[2], 0.0, 1e-6));
    assert!(close(
        max[2],
        params.height + params.rise * params.turns + params.guard_height,
        1e-5
    ));

    // Slab + disjoint column without guards are watertight solids.
    let no_guards = spiral_conveyor(SpiralConveyor {
        guard_height: 0.0,
        ..params
    });
    assert_valid_mesh(&no_guards);
    assert_watertight(&no_guards);
    assert_eq!(no_guards.triangle_count(), 388 + 28);
}

#[test]
fn rack_machine_tote_operator_forklift_agv() {
    // Rack
    let r = Rack::default();
    let rm = rack(r);
    assert_valid_mesh(&rm);
    assert_watertight(&rm);
    // Formula: (2 * (bays + 1) uprights + 2 * bays * levels beams) * 12 = (8 + 18) * 12 = 312
    assert_eq!(rm.triangle_count(), 312);
    let (rmin, rmax) = rm.bounds();
    assert!(close(
        rmax[0] - rmin[0],
        r.bays as f32 * r.bay_width + r.upright_size,
        1e-5
    ));
    assert!(close(rmax[1] - rmin[1], r.depth, 1e-5));
    assert!(close(rmin[2], 0.0, 1e-6));
    assert!(close(rmax[2], r.levels as f32 * r.level_height, 1e-5));

    // Machine
    let mc = Machine::default();
    let mm = machine(mc);
    assert_valid_mesh(&mm);
    // Formula: body box (12) + control panel box (12) = 24
    assert_eq!(mm.triangle_count(), 24);
    let (mmin, mmax) = mm.bounds();
    assert!(close3(
        [mmax[0] - mmin[0], mmax[1] - mmin[1], mmax[2] - mmin[2]],
        mc.size,
        1e-5
    ));
    assert!(close(mmin[2], 0.0, 1e-6));

    // Tote
    let t = Tote::default();
    let tm = tote(t);
    assert_valid_mesh(&tm);
    // Formula: 1 base + 4 walls = 5 boxes * 12 = 60
    assert_eq!(tm.triangle_count(), 60);
    let (tmin, tmax) = tm.bounds();
    assert!(close3(
        [tmax[0] - tmin[0], tmax[1] - tmin[1], tmax[2] - tmin[2]],
        t.size,
        1e-5
    ));
    assert!(close(tmin[2], 0.0, 1e-6));

    // Operator
    let op = Operator { height: 1.8 };
    let om = operator(op);
    assert_valid_mesh(&om);
    assert_watertight(&om);
    // Formula: 8-sided capsule (8 + 16 + 16 + 16 + 8 = 64) + 8-sided head sphere (8 + 16 + 16 + 8 = 48) = 112
    assert_eq!(om.triangle_count(), 112);
    let (omin, omax) = om.bounds();
    assert!(close(omin[2], 0.0, 1e-6));
    assert!(close(omax[2], 1.8, 1e-5));

    // Forklift
    let fk = Forklift::default();
    let fm = forklift(fk);
    assert_valid_mesh(&fm);
    // Formula: 1 body + 1 cab + 2 masts + 1 carriage + 2 forks + 4 wheels = 11 boxes * 12 = 132
    assert_eq!(fm.triangle_count(), 132);
    let (fmin, fmax) = fm.bounds();
    assert!(close3(
        [fmax[0] - fmin[0], fmax[1] - fmin[1], fmax[2] - fmin[2]],
        fk.size,
        1e-5
    ));
    assert!(close(fmin[2], 0.0, 1e-6));

    // AGV
    let ag = Agv::default();
    let am = agv(ag);
    assert_valid_mesh(&am);
    // Formula: body (12) + deck (12) + sensor bar (12) = 36
    assert_eq!(am.triangle_count(), 36);
    let (amin, amax) = am.bounds();
    assert!(close3(
        [amax[0] - amin[0], amax[1] - amin[1], amax[2] - amin[2]],
        ag.size,
        1e-5
    ));
    assert!(close(amin[2], 0.0, 1e-6));
}

#[test]
fn path_helpers_match_geometry() {
    let sc = StraightConveyor {
        length: 5.0,
        height: 0.9,
        ..Default::default()
    };
    assert!(close(sc.path_length(), 5.0, 1e-6));
    assert!(close3(sc.point_at(0.0), [0.0, 0.0, 0.9], 1e-6));
    assert!(close3(sc.point_at(2.5), [2.5, 0.0, 0.9], 1e-6));
    assert!(close3(sc.point_at(sc.path_length()), [5.0, 0.0, 0.9], 1e-6));
    assert!(close3(sc.point_at(-1.0), [0.0, 0.0, 0.9], 1e-6));
    assert!(close3(sc.point_at(99.0), [5.0, 0.0, 0.9], 1e-6));

    let r = 1.5_f32;
    let a = FRAC_PI_2;
    let cc = CurvedConveyor {
        radius: r,
        angle: a,
        height: 0.85,
        ..Default::default()
    };
    assert!(close(cc.path_length(), r * a, 1e-6));
    assert!(close3(cc.point_at(0.0), [0.0, 0.0, 0.85], 1e-6));
    assert!(close3(cc.point_at(cc.path_length()), [r, r, 0.85], 1e-5));

    let cc_right = CurvedConveyor { angle: -a, ..cc };
    assert!(close(cc_right.path_length(), r * a, 1e-6));
    assert!(close3(
        cc_right.point_at(cc_right.path_length()),
        [r, -r, 0.85],
        1e-5
    ));

    let sp = SpiralConveyor {
        radius: 1.2,
        rise: 1.5,
        turns: 2.5,
        height: 0.5,
        ..Default::default()
    };
    let expected_len = 2.5 * libm::sqrtf((TAU * 1.2) * (TAU * 1.2) + 1.5 * 1.5);
    assert!(close(sp.path_length(), expected_len, 1e-5));
    let start = sp.point_at(0.0);
    let end = sp.point_at(sp.path_length());
    assert!(close3(start, [1.2, 0.0, 0.5], 1e-6));
    assert!(close(end[2] - start[2], sp.rise * sp.turns, 1e-5));
    assert!(close3(end, [-1.2, 0.0, 0.5 + 1.5 * 2.5], 1e-4));
}

#[test]
fn zero_angle_curve_and_zero_turn_spiral_return_empty() {
    let empty_curve = curved_conveyor(CurvedConveyor {
        angle: 0.0,
        ..Default::default()
    });
    assert!(empty_curve.is_empty());
    assert_eq!(empty_curve.triangle_count(), 0);

    let empty_spiral = spiral_conveyor(SpiralConveyor {
        turns: 0.0,
        ..Default::default()
    });
    assert!(empty_spiral.is_empty());
    assert_eq!(empty_spiral.triangle_count(), 0);

    assert!(box_mesh([0.0, 1.0, 1.0]).is_empty());
    assert!(operator(Operator { height: 0.0 }).is_empty());
}
