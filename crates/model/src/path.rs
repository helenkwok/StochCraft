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

use stochcraft_random::math::{atan2, cos, sin, tan};

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

    /// The path as a plan editor shows it ([`Outline`]): the points it turns at, each corner
    /// with the radius of its bend. `None` for a path that climbs, has a helix, or turns through
    /// a whole circle or more in one arc: its points cannot be moved on a plan without losing
    /// that. Also `None` for one that turns sharply (an arc of radius 0) where it starts or ends:
    /// an outline's ends take their headings from the straights next to them, so that turn would
    /// be lost. Each arc becomes one corner, or for one of more than 90 degrees, several of equal
    /// angle, so the corner points stay near the path. An arc of no angle is a point on a
    /// straight, with its bend kept for when the point is moved ([`Outline::to_path`]).
    pub fn outline(&self) -> Option<Outline> {
        let mut points = vec![[self.start.x, self.start.y]];
        let mut bends = Vec::new();
        let mut frame = self.start;
        // Whether the last point put in is a sharp turn's corner, which is where the turn is.
        let mut last_sharp = false;
        for seg in &self.segments {
            match *seg {
                Segment::Line { length, rise } => {
                    if rise != 0.0 || !length.is_finite() {
                        return None;
                    }
                    frame = seg.frame_at(frame, 1.0);
                }
                Segment::Arc {
                    radius,
                    angle_deg,
                    rise,
                } => {
                    if rise != 0.0
                        || !radius.is_finite()
                        || !angle_deg.is_finite()
                        || !within_a_circle(angle_deg)
                    {
                        return None;
                    }
                    let r = radius.abs();
                    if angle_deg.abs() <= ANGLE_EPS {
                        // A point on a straight is an arc of no length. A wide arc that turns too
                        // little to tell from none but travels is a straight of its length.
                        if seg.length() > LENGTH_EPS {
                            frame = seg.frame_at(frame, 1.0);
                            continue;
                        }
                        let at = [frame.x, frame.y];
                        if points.last() != Some(&at) {
                            points.push(at);
                            bends.push(r);
                            last_sharp = false;
                        }
                        continue;
                    }
                    // A sharp turn of less than a half turn is one corner, so that removing its
                    // point removes the turn; a rounded one is split to stay near the path.
                    let sharp = r == 0.0;
                    let n = if sharp && angle_deg.abs() < SHARPEST {
                        1
                    } else {
                        pieces(angle_deg)
                    };
                    let step = angle_deg / f64::from(n);
                    let t = r * tan((step.abs() / 2.0).to_radians());
                    if sharp && points.len() == 1 && points[0] == [frame.x, frame.y] {
                        return None;
                    }
                    let piece = Segment::Arc {
                        radius: r,
                        angle_deg: step,
                        rise: 0.0,
                    };
                    for _ in 0..n {
                        let h = frame.heading_deg.to_radians();
                        points.push([frame.x + t * cos(h), frame.y + t * sin(h)]);
                        bends.push(r);
                        last_sharp = sharp;
                        frame = piece.frame_at(frame, 1.0);
                    }
                }
                Segment::Helix { .. } => return None,
            }
        }
        // A point of no turn where the path ends is its end; a sharp turn there has no outline.
        let end = [frame.x, frame.y];
        if points.len() > 1 && points.last() == Some(&end) {
            if last_sharp {
                return None;
            }
            points.pop();
            bends.pop();
        }
        points.push(end);
        Some(Outline {
            z: self.start.z,
            points,
            bends,
        })
    }
}

/// Angles closer than this, in degrees, are taken as equal (no turn).
const ANGLE_EPS: f64 = 1e-9;
/// The sharpest turn, in degrees, one corner of an [`Outline`] makes: short of a half turn,
/// which would leave the way back undecided.
const SHARPEST: f64 = 180.0 - 1e-6;
/// Distances shorter than this, relative to the larger of 1 and the distances compared, are
/// taken as none.
const LENGTH_EPS: f64 = 1e-9;

/// Whether an arc turning `angle_deg` has an [`Outline`]: less than a whole circle, so its
/// corner points do not come round on themselves. [`Outline::to_path`] joins bends only while
/// the arc they make stays so.
pub fn within_a_circle(angle_deg: f64) -> bool {
    angle_deg.abs() < 360.0 - ANGLE_EPS
}

/// The number of corners of equal turn an arc turning `angle_deg` is on a plan
/// ([`Path::outline`]): one up to a quarter turn, and one more for each quarter turn beyond.
fn pieces(angle_deg: f64) -> u8 {
    let a = angle_deg.abs() - ANGLE_EPS;
    if a <= 90.0 {
        1
    } else if a <= 180.0 {
        2
    } else if a <= 270.0 {
        3
    } else {
        4
    }
}

/// Whether corners that turn `angle_deg` in all, then one more that turns `turn`, `n` in all,
/// come back as the same corners once they are one arc: [`Path::outline`] makes an arc
/// [`pieces`] corners of equal turn, so they must be that many and each turn the same.
fn joins_evenly(angle_deg: f64, turn: f64, n: usize) -> bool {
    let total = angle_deg + turn;
    let p = pieces(total);
    usize::from(p) == n && (turn - total / f64::from(p)).abs() <= ANGLE_EPS
}

/// `a` in degrees brought into (−180, 180].
fn half_turn(a: f64) -> f64 {
    let mut a = a % 360.0;
    if a > 180.0 {
        a -= 360.0;
    } else if a <= -180.0 {
        a += 360.0;
    }
    a
}

/// A flat path as a plan editor shows it: the points it passes, in order (its start, each
/// corner, its end), straight between them, and at each corner a bend of a radius, a circular
/// arc that meets both straights. A bend of radius 0 is a sharp corner.
///
/// This is how established layout editors draw a conveyor or a walkway: click the points, and
/// the corners round themselves. [`Outline::to_path`] makes the [`Path`]: a line along each
/// straight, shortened at each end by the bend's reach `r · tan(turn / 2)`, and an arc at each
/// corner. Where two bends would overlap on a short straight, both are tightened to fit, so the
/// path always passes the points' straights in order.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    /// The height of the whole path.
    pub z: f64,
    /// The points, at least two for a path with a length.
    pub points: Vec<[f64; 2]>,
    /// The radius of the bend at each corner, the points between the first and the last
    /// (`points.len() − 2` of them); 0 or more.
    pub bends: Vec<f64>,
}

impl Outline {
    /// The bend at point `i`: `None` at either end.
    pub fn bend(&self, i: usize) -> Option<f64> {
        if i == 0 || i + 1 >= self.points.len() {
            return None;
        }
        self.bends.get(i - 1).copied()
    }

    /// The same outline with point `i` at `to`.
    pub fn moved(&self, i: usize, to: [f64; 2]) -> Outline {
        let mut out = self.clone();
        if let Some(p) = out.points.get_mut(i) {
            *p = to;
        }
        out
    }

    /// The same outline moved by `by` as a whole.
    pub fn shifted(&self, by: [f64; 2]) -> Outline {
        let mut out = self.clone();
        for p in &mut out.points {
            p[0] += by[0];
            p[1] += by[1];
        }
        out
    }

    /// The same outline with a point at `at` put in at index `i` (0 before the start, the
    /// number of points after the end), with a bend of `bend` where it, or the end it displaces,
    /// becomes a corner.
    pub fn inserted(&self, i: usize, at: [f64; 2], bend: f64) -> Outline {
        let mut out = self.clone();
        let n = out.points.len();
        let i = i.min(n);
        out.points.insert(i, at);
        if n >= 2 {
            if i == 0 {
                out.bends.insert(0, bend);
            } else if i >= n {
                out.bends.push(bend);
            } else {
                out.bends.insert((i - 1).min(out.bends.len()), bend);
            }
        }
        out.bends.truncate(out.points.len().saturating_sub(2));
        out
    }

    /// The same outline without point `i`; `None` if it would have fewer than two.
    pub fn removed(&self, i: usize) -> Option<Outline> {
        let n = self.points.len();
        if n <= 2 || i >= n {
            return None;
        }
        let mut out = self.clone();
        out.points.remove(i);
        if !out.bends.is_empty() {
            if i == 0 {
                out.bends.remove(0);
            } else if i + 1 >= n {
                out.bends.pop();
            } else {
                out.bends.remove(i - 1);
            }
        }
        Some(out)
    }

    /// The same outline with the bend at corner `i` (a point that is neither end) of radius
    /// `radius`.
    pub fn with_bend(&self, i: usize, radius: f64) -> Outline {
        let mut out = self.clone();
        if i > 0
            && let Some(b) = out.bends.get_mut(i - 1)
        {
            *b = radius;
        }
        out
    }

    /// The radius each corner's bend has in the path once tightened to fit its straights (the
    /// bend asked for, where it fits), by point: `None` at either end.
    pub fn fitted_bends(&self) -> Vec<Option<f64>> {
        let (_, _, turns, reach) = self.layout();
        (0..self.points.len())
            .map(|i| {
                let b = self.bend(i)?;
                let t = turns.get(i).copied().unwrap_or(0.0);
                if t.abs() <= ANGLE_EPS {
                    return Some(b);
                }
                let full = b * tan((t.abs() / 2.0).to_radians());
                let got = reach.get(i).copied().unwrap_or(0.0);
                Some(if full > 0.0 { b * got / full } else { b })
            })
            .collect()
    }

    /// Each straight's length and heading (degrees), each point's turn (degrees, 0 at either
    /// end) and each point's reach along its straights once fitted.
    fn layout(&self) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
        let n = self.points.len();
        let legs = n.saturating_sub(1);
        let mut lengths = Vec::with_capacity(legs);
        let mut headings: Vec<Option<f64>> = Vec::with_capacity(legs);
        for w in self.points.windows(2) {
            let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let len = norm(dx, dy);
            lengths.push(len);
            // The same tolerance as a line's in `to_path`, wherever on the floor the straight is.
            headings.push((len > LENGTH_EPS).then(|| atan2(dy, dx).to_degrees()));
        }
        // A straight of no length takes the heading of the one before it, or for those at the
        // start, of the first that has one.
        let first = headings.iter().flatten().next().copied().unwrap_or(0.0);
        let mut last = first;
        let headings: Vec<f64> = headings
            .into_iter()
            .map(|h| {
                if let Some(h) = h {
                    last = h;
                }
                last
            })
            .collect();
        let mut turns = vec![0.0; n];
        let mut reach = vec![0.0; n];
        for i in 1..n.saturating_sub(1) {
            let turn = half_turn(headings[i] - headings[i - 1]);
            turns[i] = turn;
            let r = self.bends.get(i - 1).copied().unwrap_or(0.0).max(0.0);
            if turn.abs() > ANGLE_EPS && turn.abs() < SHARPEST && r.is_finite() {
                reach[i] = r * tan((turn.abs() / 2.0).to_radians());
            }
        }
        // Two bends that would overlap on a straight are tightened together to meet.
        let mut scale = vec![1.0_f64; n];
        for (i, len) in lengths.iter().enumerate() {
            let need = reach[i] + reach[i + 1];
            if need > len * (1.0 + LENGTH_EPS) + LENGTH_EPS {
                let k = if need > 0.0 { len / need } else { 1.0 };
                scale[i] = scale[i].min(k);
                scale[i + 1] = scale[i + 1].min(k);
            }
        }
        for (r, k) in reach.iter_mut().zip(&scale) {
            *r *= k;
        }
        (lengths, headings, turns, reach)
    }

    /// The path: a line along each straight, shortened by the bends at its ends, and an arc at
    /// each corner. Arcs of one radius that turn the same way with no line between are one arc
    /// when its outline has the same corners (as two quarter bends make a half circle) and it
    /// stays less than a whole circle. A corner of no turn (a point on a straight) is an arc of no
    /// angle, so the point and its bend are kept.
    pub fn to_path(&self) -> Path {
        let start = self.points.first().copied().unwrap_or([0.0, 0.0]);
        let (lengths, headings, turns, reach) = self.layout();
        let heading = headings.first().copied().unwrap_or(0.0);
        let mut segments: Vec<Segment> = Vec::new();
        // The corners joined into the last segment, when it is an arc.
        let mut joined = 0_usize;
        let fitted = self.fitted_bends();
        for (i, len) in lengths.iter().enumerate() {
            let line = len - reach[i] - reach[i + 1];
            if line > LENGTH_EPS * len.max(1.0) {
                segments.push(Segment::Line {
                    length: line,
                    rise: 0.0,
                });
            }
            let corner = i + 1;
            let turn = turns.get(corner).copied().unwrap_or(0.0);
            if corner + 1 < self.points.len() {
                let turn = if turn.abs() > ANGLE_EPS { turn } else { 0.0 };
                let radius = fitted[corner].unwrap_or(0.0).max(0.0);
                let same = |r: f64| (r - radius).abs() <= LENGTH_EPS * radius.max(1.0);
                match segments.last_mut() {
                    Some(Segment::Arc {
                        radius: r,
                        angle_deg,
                        ..
                    }) if turn != 0.0
                        && *angle_deg != 0.0
                        && same(*r)
                        && (*angle_deg > 0.0) == (turn > 0.0)
                        && within_a_circle(*angle_deg + turn)
                        && joins_evenly(*angle_deg, turn, joined + 1) =>
                    {
                        *angle_deg += turn;
                        joined += 1;
                    }
                    _ => {
                        segments.push(Segment::Arc {
                            radius,
                            angle_deg: turn,
                            rise: 0.0,
                        });
                        joined = 1;
                    }
                }
            }
        }
        if segments.is_empty() {
            segments.push(Segment::Line {
                length: 0.0,
                rise: 0.0,
            });
        }
        Path::new(Frame::new(start[0], start[1], self.z, heading), segments)
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

    /// The same path in every way that matters: length, and the frame every metre along.
    fn same_path(a: &Path, b: &Path) {
        near(a.length(), b.length(), 1e-9);
        let n = a.length().ceil() as usize;
        for k in 0..=n {
            let s = (k as f64).min(a.length());
            let (p, q) = (a.point_at(s), b.point_at(s));
            at(q, p.x, p.y, p.z, p.heading_deg);
        }
    }

    fn line(length: f64) -> Segment {
        Segment::Line { length, rise: 0.0 }
    }

    fn arc(radius: f64, angle_deg: f64) -> Segment {
        Segment::Arc {
            radius,
            angle_deg,
            rise: 0.0,
        }
    }

    #[test]
    fn a_racetrack_loop_has_four_corners_and_comes_back_the_same() {
        // The sorter's loop: two 10.5 m straights and two half circles of 30 m in all.
        let r = 4.5 / std::f64::consts::PI;
        let path = Path::new(
            Frame::default(),
            vec![line(10.5), arc(r, 180.0), line(10.5), arc(r, 180.0)],
        );
        let o = path.outline().unwrap();
        // Each half circle is two quarter bends at the corners of the box round it.
        assert_eq!(o.points.len(), 6);
        assert_eq!(o.bends.len(), 4);
        assert!(o.bends.iter().all(|b| (b - r).abs() < 1e-12));
        let corner = o.points[1];
        near(corner[0], 10.5 + r, 1e-12);
        near(corner[1], 0.0, 1e-12);
        let back = o.to_path();
        same_path(&path, &back);
        near(back.length(), 30.0, 1e-9);
        // The quarter bends join again into the half circles: four segments, as drawn.
        assert_eq!(back.segments.len(), 4, "{:?}", back.segments);
    }

    #[test]
    fn a_line_a_bend_and_a_line_is_three_points() {
        // 10 m, a left quarter turn of radius 5 m, 4 m on: the corner where the straights meet
        // is (15, 0), and the end (15, 9).
        let path = Path::new(
            Frame::new(1.0, 2.0, 0.5, 0.0),
            vec![line(10.0), arc(5.0, 90.0), line(4.0)],
        );
        let o = path.outline().unwrap();
        assert_eq!(o.points.len(), 3);
        for (p, q) in o.points.iter().zip([[1.0, 2.0], [16.0, 2.0], [16.0, 11.0]]) {
            near(p[0], q[0], 1e-12);
            near(p[1], q[1], 1e-12);
        }
        assert_eq!(o.bends, vec![5.0]);
        near(o.z, 0.5, 0.0);
        same_path(&path, &o.to_path());
    }

    #[test]
    fn clicked_points_round_their_corners() {
        // An L of 10 m and 6 m with a 1 m bend: the bend reaches 1 m along each straight (a
        // quarter turn), so 9 m, a quarter circle of π/2 m and 5 m: 14 + π/2.
        let o = Outline {
            z: 0.0,
            points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 6.0]],
            bends: vec![1.0],
        };
        let p = o.to_path();
        near(p.length(), 14.0 + std::f64::consts::FRAC_PI_2, 1e-12);
        at(p.end(), 10.0, 6.0, 0.0, 90.0);
        // A sharp corner (radius 0) is the two straights end to end.
        let sharp = o.with_bend(1, 0.0).to_path();
        near(sharp.length(), 16.0, 1e-12);
        at(sharp.end(), 10.0, 6.0, 0.0, 90.0);
        // Turning right mirrors it.
        let right = Outline {
            z: 0.0,
            points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, -6.0]],
            bends: vec![1.0],
        }
        .to_path();
        at(right.end(), 10.0, -6.0, 0.0, -90.0);
    }

    #[test]
    fn bends_too_big_for_their_straights_are_tightened_to_fit() {
        // A U of 2 m sides and a 1 m bottom with bends of 5 m: each quarter bend could reach
        // only half the bottom, 0.5 m, so both are 0.5 m in radius.
        let o = Outline {
            z: 0.0,
            points: vec![[0.0, 0.0], [2.0, 0.0], [2.0, 1.0], [0.0, 1.0]],
            bends: vec![5.0, 5.0],
        };
        let fitted = o.fitted_bends();
        assert_eq!(fitted[0], None);
        near(fitted[1].unwrap(), 0.5, 1e-12);
        near(fitted[2].unwrap(), 0.5, 1e-12);
        let p = o.to_path();
        // 1.5 + π/2 · 0.5 · 2 + 1.5: a half circle of radius 0.5 between two 1.5 m straights.
        near(p.length(), 3.0 + std::f64::consts::FRAC_PI_2, 1e-12);
        at(p.end(), 0.0, 1.0, 0.0, 180.0);
    }

    #[test]
    fn points_are_moved_added_and_removed() {
        let o = Outline {
            z: 0.0,
            points: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 6.0]],
            bends: vec![1.0],
        };
        // Moving the corner keeps its bend.
        let m = o.moved(1, [10.0, -2.0]);
        assert_eq!(m.points[1], [10.0, -2.0]);
        assert_eq!(m.bends, vec![1.0]);
        // A point after the end makes the old end a corner.
        let a = o.inserted(3, [0.0, 6.0], 2.0);
        assert_eq!(a.points.len(), 4);
        assert_eq!(a.bends, vec![1.0, 2.0]);
        // One before the start, likewise.
        let b = o.inserted(0, [0.0, -5.0], 2.0);
        assert_eq!(b.points[0], [0.0, -5.0]);
        assert_eq!(b.bends, vec![2.0, 1.0]);
        // One on a straight is a corner of no turn: the path is as long as before.
        let c = o.inserted(1, [4.0, 0.0], 3.0);
        assert_eq!(c.bends, vec![3.0, 1.0]);
        near(c.to_path().length(), o.to_path().length(), 1e-12);
        // Removing the corner leaves the diagonal from (0, 0) to (10, 6).
        let d = o.removed(1).unwrap();
        assert!(d.bends.is_empty());
        near(d.to_path().length(), 136.0_f64.sqrt(), 1e-12);
        // Removing an end makes the corner next to it the end.
        let e = o.removed(2).unwrap();
        assert_eq!(e.points, vec![[0.0, 0.0], [10.0, 0.0]]);
        assert!(e.bends.is_empty());
        // Two points are the least a path has.
        assert!(e.removed(0).is_none());
        // Moving the whole outline moves every point.
        assert_eq!(o.shifted([1.0, 1.0]).points[2], [11.0, 7.0]);
    }

    #[test]
    fn a_point_put_on_a_straight_stays_with_its_bend() {
        // A 10 m straight with a point put in at 5 m: the path is as long, and its outline has
        // the point and the 3 m bend it was given.
        let o = Outline {
            z: 0.0,
            points: vec![[0.0, 0.0], [10.0, 0.0]],
            bends: Vec::new(),
        }
        .inserted(1, [5.0, 0.0], 3.0);
        let p = o.to_path();
        near(p.length(), 10.0, 1e-12);
        let back = p.outline().unwrap();
        assert_eq!(back.points.len(), 3);
        for (q, r) in back
            .points
            .iter()
            .zip([[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]])
        {
            near(q[0], r[0], 1e-12);
            near(q[1], r[1], 1e-12);
        }
        assert_eq!(back.bends, vec![3.0]);
        same_path(&p, &back.to_path());
        // Moved off the line, the point is a corner with that bend, not a sharp one.
        let bent = back.moved(1, [5.0, 5.0]).to_path();
        let turned: Vec<f64> = bent
            .segments
            .iter()
            .filter_map(|s| match *s {
                Segment::Arc {
                    radius, angle_deg, ..
                } if angle_deg != 0.0 => Some(radius),
                _ => None,
            })
            .collect();
        assert_eq!(turned.len(), 1, "{:?}", bent.segments);
        near(turned[0], 3.0, 1e-12);
        at(bent.end(), 10.0, 0.0, 0.0, -45.0);
    }

    #[test]
    fn a_short_straight_keeps_its_heading_far_from_the_origin() {
        // 5 mm north, then 10 m east, 10 000 km out along x: the path ends where the points say.
        let x = 1.0e7;
        let o = Outline {
            z: 0.0,
            points: vec![[x, 0.0], [x, 0.005], [x + 10.0, 0.005]],
            bends: vec![0.0],
        };
        let p = o.to_path();
        near(p.start.heading_deg, 90.0, 1e-9);
        let end = p.end();
        near(end.x, x + 10.0, 1e-6);
        near(end.y, 0.005, 1e-9);
        near(p.length(), 10.005, 1e-6);
    }

    #[test]
    fn a_loop_of_four_bends_keeps_its_points() {
        // A 2 m square drawn with 1 m bends and no straight left between them: the bends join
        // only while less than a whole circle, so the loop keeps its four corners.
        let points = vec![
            [1.0, 0.0],
            [2.0, 0.0],
            [2.0, 2.0],
            [0.0, 2.0],
            [0.0, 0.0],
            [1.0, 0.0],
        ];
        let o = Outline {
            z: 0.0,
            points: points.clone(),
            bends: vec![1.0; 4],
        };
        let p = o.to_path();
        near(p.length(), 2.0 * PI, 1e-9);
        for s in &p.segments {
            if let Segment::Arc { angle_deg, .. } = *s {
                assert!(within_a_circle(angle_deg), "{:?}", p.segments);
            }
        }
        let back = p.outline().unwrap();
        assert_eq!(back.points.len(), points.len());
        for (q, r) in back.points.iter().zip(&points) {
            near(q[0], r[0], 1e-9);
            near(q[1], r[1], 1e-9);
        }
        // A half circle still joins into one arc.
        let half = Outline {
            z: 0.0,
            points: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]],
            bends: vec![1.0, 1.0],
        }
        .to_path();
        assert_eq!(half.segments.len(), 1, "{:?}", half.segments);
        near(half.length(), PI, 1e-9);
    }

    #[test]
    fn a_path_that_climbs_or_coils_has_no_outline() {
        let climb = Path::new(
            Frame::default(),
            vec![Segment::Line {
                length: 4.0,
                rise: 1.0,
            }],
        );
        assert!(climb.outline().is_none());
        let coil = Path::new(
            Frame::default(),
            vec![Segment::Helix {
                radius: 1.0,
                rise: 0.0,
                turns: 1.0,
            }],
        );
        assert!(coil.outline().is_none());
        assert!(
            Path::new(Frame::default(), vec![arc(1.0, 360.0)])
                .outline()
                .is_none()
        );
        // A sharp corner in a file (an arc of radius 0) is a corner with a bend of 0.
        let sharp = Path::new(Frame::default(), vec![line(3.0), arc(0.0, 90.0), line(2.0)]);
        let o = sharp.outline().unwrap();
        assert_eq!(o.points.len(), 3);
        for (p, q) in o.points.iter().zip([[0.0, 0.0], [3.0, 0.0], [3.0, 2.0]]) {
            near(p[0], q[0], 1e-12);
            near(p[1], q[1], 1e-12);
        }
        assert_eq!(o.bends, vec![0.0]);
        same_path(&sharp, &o.to_path());
    }

    #[test]
    fn a_sharp_turn_at_either_end_has_no_outline() {
        // 3 m east, then a sharp quarter turn left where it ends: it ends facing north, which an
        // outline (its last straight east) cannot say.
        let end = Path::new(Frame::default(), vec![line(3.0), arc(0.0, 90.0)]);
        at(end.point_at(end.length()), 3.0, 0.0, 0.0, 90.0);
        assert!(end.outline().is_none());
        assert!(
            Path::new(Frame::default(), vec![line(3.0), arc(0.0, 180.0)])
                .outline()
                .is_none()
        );
        // Likewise a sharp turn where it starts, before its first straight.
        assert!(
            Path::new(Frame::default(), vec![arc(0.0, 90.0), line(3.0)])
                .outline()
                .is_none()
        );
        // A sharp corner between two straights, and a round bend where the path ends, still
        // have outlines that come back the same.
        let inside = Path::new(Frame::default(), vec![line(3.0), arc(0.0, 90.0), line(2.0)]);
        same_path(&inside, &inside.outline().unwrap().to_path());
        let round = Path::new(Frame::default(), vec![line(3.0), arc(1.0, 90.0)]);
        let back = round.outline().unwrap().to_path();
        at(back.end(), 4.0, 1.0, 0.0, 90.0);
        same_path(&round, &back);
    }

    #[test]
    fn a_sharp_turn_inside_is_one_point_and_a_shallow_wide_arc_a_straight() {
        // A sharp 135 degree corner is one point: removing it takes the turn away.
        let sharp = Path::new(
            Frame::default(),
            vec![line(3.0), arc(0.0, 135.0), line(2.0)],
        );
        let o = sharp.outline().unwrap();
        assert_eq!(o.points.len(), 3, "{:?}", o.points);
        same_path(&sharp, &o.to_path());
        let less = o.removed(1).unwrap().to_path();
        assert_eq!(less.outline().unwrap().points.len(), 2);
        // An arc 8.7 m long that turns too little to tell from none is a straight of its
        // length, not a point that would come back with no length.
        let shallow = Path::new(Frame::default(), vec![line(1.0), arc(1.0e12, 5.0e-10)]);
        let long = 1.0 + 1.0e12 * (5.0e-10_f64).to_radians();
        near(shallow.length(), long, 1e-6);
        let o = shallow.outline().unwrap();
        assert_eq!(o.points.len(), 2, "{:?}", o.points);
        near(o.to_path().length(), long, 1e-6);
        // A point on a straight, an arc of no angle, still is one.
        let marked = Path::new(Frame::default(), vec![line(1.0), arc(3.0, 0.0), line(1.0)]);
        assert_eq!(marked.outline().unwrap().points.len(), 3);
    }

    #[test]
    fn tangent_bends_join_only_when_their_outline_keeps_the_corners() {
        // A 45 degree bend and a 90 degree one of 1 m with no straight between: as one arc of
        // 135 degrees, the outline would be two corners of 67.5, both moved.
        let r = 1.0;
        let first = r * tan((22.5_f64).to_radians());
        let p1 = [3.0, 0.0];
        let between = first + r;
        let d = 45.0_f64.to_radians();
        let p2 = [p1[0] + between * cos(d), p1[1] + between * sin(d)];
        let d = 135.0_f64.to_radians();
        let p3 = [p2[0] + 2.0 * cos(d), p2[1] + 2.0 * sin(d)];
        let o = Outline {
            z: 0.0,
            points: vec![[0.0, 0.0], p1, p2, p3],
            bends: vec![r, r],
        };
        let p = o.to_path();
        let turns: Vec<f64> = p
            .segments
            .iter()
            .filter_map(|s| match *s {
                Segment::Arc { angle_deg, .. } => Some(angle_deg),
                _ => None,
            })
            .collect();
        assert_eq!(turns.len(), 2, "{:?}", p.segments);
        near(turns[0], 45.0, 1e-9);
        near(turns[1], 90.0, 1e-9);
        let back = p.outline().unwrap();
        assert_eq!(back.points.len(), o.points.len());
        for (q, w) in back.points.iter().zip(&o.points) {
            near(q[0], w[0], 1e-9);
            near(q[1], w[1], 1e-9);
        }
        same_path(&p, &back.to_path());
    }
}
