//! Geometry with exact lengths (ROADMAP.md, M5.5): a [`Path`] is a start [`Frame`] and a list of
//! [`Segment`]s, each continuing from the end frame of the one before.
//!
//! Lengths and positions are worked out per segment from its own formula, never from a flattened
//! polyline, so the run's travel distance, the mesh's sweep and the view's positions all agree.
//!
//! Conventions: metres and degrees at every public surface, with one conversion to radians
//! inside. The plane is `x` right and `y` up, `z` up from the floor; a heading of 0 points along
//! `+x` and a positive heading turns left (towards `+y`). A path does not say what unit its
//! numbers are in; [`Path::scaled`] converts between units.

use std::f64::consts::TAU;

use stochcraft_random::math::{cos, sin};

/// `√(a² + b²)`: exactly rounded on every target, as `hypot` need not be.
fn norm(a: f64, b: f64) -> f64 {
    (a * a + b * b).sqrt()
}

/// A place, a height and the direction one faces there.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// The direction of travel in degrees: 0 along `+x`, positive turning left. It adds up along
    /// a path, so a path that turns left through two full circles ends at 720.
    pub heading_deg: f64,
}

impl Frame {
    /// A frame at `(x, y, z)` facing `heading_deg`.
    pub fn new(x: f64, y: f64, z: f64, heading_deg: f64) -> Self {
        Self {
            x,
            y,
            z,
            heading_deg,
        }
    }
}

/// One run of a [`Path`]. `rise` is always the segment's total rise, positive going up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// A straight run of `length` along the heading (the horizontal distance), climbing `rise`,
    /// so a slope of `rise` over `length`.
    Line { length: f64, rise: f64 },
    /// A circular arc of `radius` that turns through `angle_deg` (positive turns left, negative
    /// right), climbing `rise` in all.
    Arc {
        radius: f64,
        angle_deg: f64,
        rise: f64,
    },
    /// A helix of `radius` that turns left through `turns` full turns (a negative count turns
    /// right), climbing `rise` over all of them.
    Helix { radius: f64, rise: f64, turns: f64 },
}

/// The frame after the fraction `fraction` (0 to 1) of an arc that starts at `from`, turns
/// through `angle_deg` on a circle of `radius` and climbs `rise`.
fn arc_frame(from: Frame, radius: f64, angle_deg: f64, rise: f64, fraction: f64) -> Frame {
    let r = radius.abs();
    let sign = if angle_deg < 0.0 { -1.0 } else { 1.0 };
    let h0 = from.heading_deg.to_radians();
    let heading_deg = from.heading_deg + angle_deg * fraction;
    let h = heading_deg.to_radians();
    // The circle's centre is `r` to the left of the start (to the right when turning right).
    let (cx, cy) = (from.x - sign * r * sin(h0), from.y + sign * r * cos(h0));
    Frame {
        x: cx + sign * r * sin(h),
        y: cy - sign * r * cos(h),
        z: from.z + rise * fraction,
        heading_deg,
    }
}

impl Segment {
    /// The distance along the segment from its start to its end, climb included:
    /// a line is `√(length² + rise²)`, an arc `√((radius · angle)² + rise²)` with the angle in
    /// radians, a helix `√((2π · radius · turns)² + rise²)`.
    pub fn length(&self) -> f64 {
        match *self {
            Segment::Line { length, rise } => norm(length, rise),
            Segment::Arc {
                radius,
                angle_deg,
                rise,
            } => norm(radius.abs() * angle_deg.to_radians().abs(), rise),
            Segment::Helix {
                radius,
                rise,
                turns,
            } => norm(TAU * radius.abs() * turns.abs(), rise),
        }
    }

    /// The frame after the fraction `fraction` of the segment's length (0 to 1, clamped) when it
    /// starts at `from`. Rise and turning both grow in step with the distance along it.
    pub fn frame_at(&self, from: Frame, fraction: f64) -> Frame {
        let f = fraction.clamp(0.0, 1.0);
        match *self {
            Segment::Line { length, rise } => {
                let h = from.heading_deg.to_radians();
                Frame {
                    x: from.x + length * f * cos(h),
                    y: from.y + length * f * sin(h),
                    z: from.z + rise * f,
                    heading_deg: from.heading_deg,
                }
            }
            Segment::Arc {
                radius,
                angle_deg,
                rise,
            } => arc_frame(from, radius, angle_deg, rise, f),
            Segment::Helix {
                radius,
                rise,
                turns,
            } => arc_frame(from, radius, 360.0 * turns, rise, f),
        }
    }
}

/// A start frame and the segments that follow it, each from the end of the one before.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Path {
    pub start: Frame,
    pub segments: Vec<Segment>,
}

impl Path {
    /// A path from `start` through `segments`.
    pub fn new(start: Frame, segments: Vec<Segment>) -> Self {
        Self { start, segments }
    }

    /// The distance along the whole path: the sum of its segments' lengths.
    pub fn length(&self) -> f64 {
        self.segments.iter().map(Segment::length).sum()
    }

    /// The frame at the end of the path (the start frame when there are no segments).
    pub fn end(&self) -> Frame {
        self.segments
            .iter()
            .fold(self.start, |frame, s| s.frame_at(frame, 1.0))
    }

    /// The position and heading at distance `s` from the start, along the path. `s` below 0 gives
    /// the start and `s` beyond [`Path::length`] the end.
    pub fn point_at(&self, s: f64) -> Frame {
        let s = s.max(0.0);
        if s >= self.length() {
            return self.end();
        }
        let mut frame = self.start;
        let mut done = 0.0;
        for seg in &self.segments {
            let len = seg.length();
            if s <= done + len {
                let fraction = if len > 0.0 { (s - done) / len } else { 1.0 };
                return seg.frame_at(frame, fraction);
            }
            done += len;
            frame = seg.frame_at(frame, 1.0);
        }
        frame
    }

    /// The same path with every distance (positions, lengths, radii, rises) times `k`, to change
    /// its unit; angles and turns stay.
    pub fn scaled(&self, k: f64) -> Path {
        let segments = self
            .segments
            .iter()
            .map(|seg| match *seg {
                Segment::Line { length, rise } => Segment::Line {
                    length: length * k,
                    rise: rise * k,
                },
                Segment::Arc {
                    radius,
                    angle_deg,
                    rise,
                } => Segment::Arc {
                    radius: radius * k,
                    angle_deg,
                    rise: rise * k,
                },
                Segment::Helix {
                    radius,
                    rise,
                    turns,
                } => Segment::Helix {
                    radius: radius * k,
                    rise: rise * k,
                    turns,
                },
            })
            .collect();
        Path {
            start: Frame {
                x: self.start.x * k,
                y: self.start.y * k,
                z: self.start.z * k,
                heading_deg: self.start.heading_deg,
            },
            segments,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

    use std::f64::consts::PI;

    use super::*;

    fn near(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol, "{a} is not within {tol} of {b}");
    }

    fn at(f: Frame, x: f64, y: f64, z: f64, heading: f64) {
        near(f.x, x, 1e-9);
        near(f.y, y, 1e-9);
        near(f.z, z, 1e-9);
        near(f.heading_deg, heading, 1e-9);
    }

    fn one(segment: Segment) -> Path {
        Path::new(Frame::default(), vec![segment])
    }

    #[test]
    fn a_line_with_a_rise_is_the_hypotenuse() {
        // 10 m along and 3 m up: √(100 + 9) = √109.
        let line = one(Segment::Line {
            length: 10.0,
            rise: 3.0,
        });
        near(line.length(), 109.0_f64.sqrt(), 1e-12);
        near(line.length(), 10.440_306_508_910_55, 1e-9);
        // Half way along the slope: 5 m along, 1.5 m up.
        at(line.point_at(line.length() / 2.0), 5.0, 0.0, 1.5, 0.0);
    }

    #[test]
    fn a_flat_quarter_arc_is_a_quarter_of_the_circumference() {
        // r = 10 m through 90 degrees: 10 · π/2 = 15.708 m.
        let arc = one(Segment::Arc {
            radius: 10.0,
            angle_deg: 90.0,
            rise: 0.0,
        });
        near(arc.length(), 5.0 * PI, 1e-12);
        near(arc.length(), 15.708, 1e-3);
        // It ends 10 m ahead and 10 m to the left, facing left.
        at(arc.end(), 10.0, 10.0, 0.0, 90.0);
    }

    #[test]
    fn an_arc_with_a_rise_is_longer() {
        // √(15.708² + 10²) = √346.74 = 18.62 m.
        let arc = one(Segment::Arc {
            radius: 10.0,
            angle_deg: 90.0,
            rise: 10.0,
        });
        near(arc.length(), (5.0 * PI).hypot(10.0), 1e-12);
        near(arc.length(), 18.621, 1e-3);
        at(arc.end(), 10.0, 10.0, 10.0, 90.0);
    }

    #[test]
    fn a_helix_of_two_turns_climbing_three_metres() {
        // r = 1 m: 2π · 1 · 2 = 12.566 m round, 3 m up: √(157.91 + 9) = 12.92 m.
        let helix = one(Segment::Helix {
            radius: 1.0,
            rise: 3.0,
            turns: 2.0,
        });
        near(helix.length(), (4.0 * PI).hypot(3.0), 1e-12);
        near(helix.length(), 12.9195, 1e-3);
        // Two whole turns bring it back over the start, 3 m up, facing round by 720 degrees.
        at(helix.end(), 0.0, 0.0, 3.0, 720.0);
        // Half way is one turn: over the start again, half way up.
        at(helix.point_at(helix.length() / 2.0), 0.0, 0.0, 1.5, 360.0);
        // An eighth of the way is a quarter turn of the first circle, which is centred 1 m to
        // the left of the start: a metre ahead, a metre left, facing left, 3/8 m up.
        at(helix.point_at(helix.length() / 8.0), 1.0, 1.0, 0.375, 90.0);
    }

    #[test]
    fn the_point_at_the_length_is_the_end() {
        let path = Path::new(
            Frame::new(1.0, 2.0, 0.5, 30.0),
            vec![
                Segment::Line {
                    length: 4.0,
                    rise: 1.0,
                },
                Segment::Arc {
                    radius: 3.0,
                    angle_deg: -120.0,
                    rise: 0.0,
                },
                Segment::Helix {
                    radius: 2.0,
                    rise: 2.0,
                    turns: 1.5,
                },
            ],
        );
        let end = path.end();
        let there = path.point_at(path.length());
        at(there, end.x, end.y, end.z, end.heading_deg);
        // Beyond either end it stays at the end.
        let past = path.point_at(path.length() + 100.0);
        at(past, end.x, end.y, end.z, end.heading_deg);
        at(path.point_at(-5.0), 1.0, 2.0, 0.5, 30.0);
        at(path.point_at(0.0), 1.0, 2.0, 0.5, 30.0);
        // A path with no segments is its start.
        let empty = Path::new(Frame::new(3.0, 4.0, 0.0, 10.0), Vec::new());
        near(empty.length(), 0.0, 0.0);
        at(empty.point_at(7.0), 3.0, 4.0, 0.0, 10.0);
    }

    #[test]
    fn half_way_round_a_quarter_arc_is_eight_of_a_circle() {
        // The centre is at (0, 10); 45 degrees round, the point is r(sin 45, -cos 45) from it.
        let arc = one(Segment::Arc {
            radius: 10.0,
            angle_deg: 90.0,
            rise: 0.0,
        });
        let half = 10.0 * sin(PI / 4.0);
        at(
            arc.point_at(arc.length() / 2.0),
            half,
            10.0 - half,
            0.0,
            45.0,
        );
        // Turning right mirrors it.
        let right = one(Segment::Arc {
            radius: 10.0,
            angle_deg: -90.0,
            rise: 0.0,
        });
        at(right.end(), 10.0, -10.0, 0.0, -90.0);
        at(
            right.point_at(right.length() / 2.0),
            half,
            -(10.0 - half),
            0.0,
            -45.0,
        );
    }

    #[test]
    fn segments_continue_from_the_frame_before() {
        // 10 m along +x, a left quarter turn of radius 5 m, then 4 m on: the arc's centre is at
        // (10, 5), it ends at (15, 5) facing +y, and the last line ends at (15, 9).
        let path = Path::new(
            Frame::default(),
            vec![
                Segment::Line {
                    length: 10.0,
                    rise: 0.0,
                },
                Segment::Arc {
                    radius: 5.0,
                    angle_deg: 90.0,
                    rise: 0.0,
                },
                Segment::Line {
                    length: 4.0,
                    rise: 0.0,
                },
            ],
        );
        near(path.length(), 14.0 + 2.5 * PI, 1e-12);
        at(path.end(), 15.0, 9.0, 0.0, 90.0);
        // Where the line ends and the arc begins.
        at(path.point_at(10.0), 10.0, 0.0, 0.0, 0.0);
        // Two metres into the last line.
        at(path.point_at(10.0 + 2.5 * PI + 2.0), 15.0, 7.0, 0.0, 90.0);
    }

    #[test]
    fn scaling_changes_distances_and_not_angles() {
        let path = Path::new(
            Frame::new(1.0, 2.0, 3.0, 45.0),
            vec![
                Segment::Arc {
                    radius: 2.0,
                    angle_deg: 90.0,
                    rise: 1.0,
                },
                Segment::Helix {
                    radius: 1.0,
                    rise: 3.0,
                    turns: 2.0,
                },
            ],
        );
        let big = path.scaled(1000.0);
        near(big.length(), path.length() * 1000.0, 1e-6);
        let (a, b) = (path.end(), big.end());
        at(b, a.x * 1000.0, a.y * 1000.0, a.z * 1000.0, a.heading_deg);
    }
}
