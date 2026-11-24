//! The physical layout (ROADMAP.md, M5.5): where things are, apart from the flow diagram.
//!
//! A model's [`Site`] holds each conveyor's [`Track`] (its [`Path`] and the belt's width), each
//! block's [`Place`] (a frame, or a distance along a conveyor's path), and look-only
//! [`Scenery`]. Distances are in the experiment's length unit, like a conveyor's `length`, so a
//! conveyor's length is its path's length with no conversion; headings are in degrees.
//!
//! **Units.** Every distance in a site (positions, path lengths, radii, rises, a belt's width,
//! scenery sizes) is in the experiment's [`LengthUnit`]. The two fixed amounts the layout uses
//! are stated in metres and converted to that unit: a belt's default width
//! ([`DEFAULT_WIDTH_M`], [`default_width`]) and the step a path's length is taken to
//! ([`LENGTH_STEP_M`], [`length_step`]).
//!
//! **Length from the path (D23).** A conveyor with a track runs for its path's length, taken to
//! 0.1 mm (the precision a file keeps). A typed length that differs by more is an override: the
//! run uses it, and validation warns, naming both ([`crate::Problem::ConveyorLengthMismatch`]).
//!
//! **Sim and look (D28).** Every field is tagged ([`FIELDS`]): the run reads the *sim* ones, which
//! enter the results baseline ([`Site::for_results`]); a *look* field never marks results out of
//! date.
//!
//! A block with no place is unplaced: it runs, but the Layout and 3D views list it rather than
//! guess where it is.
//!
//! **Points along a section, not cuts (L5).** A divert gateway placed at `s` along a conveyor
//! that sends into it is a [`Point`] of that conveyor ([`points`]): items keep one position
//! along the whole section and meet the divert as their front reaches `s`. Two conveyors joined
//! end to start by a flow may carry a [`Junction`] on that flow, with its kind and transfer time.

use std::collections::BTreeMap;

use crate::{Element, ElementId, ElementKind, Frame, LengthUnit, Model, Path, Routing};

/// The width a belt is drawn with when its track does not say: 0.6 m ([`default_width`] in a
/// model's unit).
pub const DEFAULT_WIDTH_M: f64 = 0.6;

/// The step a path's length is taken to: 0.1 mm, the precision a file keeps, so a loop drawn
/// with arcs is exactly as long as its straight sum says and a typed length that agrees with it
/// is no override ([`length_step`] in a model's unit).
pub const LENGTH_STEP_M: f64 = 1e-4;

/// Metres in one of `unit`.
pub fn metres(unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::Millimetres => 0.001,
        LengthUnit::Centimetres => 0.01,
        LengthUnit::Metres => 1.0,
        LengthUnit::Kilometres => 1000.0,
        LengthUnit::Inches => 0.0254,
        LengthUnit::Feet => 0.3048,
    }
}

/// How many [`LENGTH_STEP_M`] steps (0.1 mm) one of `unit` holds, written out so that each is
/// exact.
fn steps_per_unit(unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::Millimetres => 10.0,
        LengthUnit::Centimetres => 100.0,
        LengthUnit::Metres => 10_000.0,
        LengthUnit::Kilometres => 10_000_000.0,
        LengthUnit::Inches => 254.0,
        LengthUnit::Feet => 3048.0,
    }
}

/// 0.1 mm in `unit`: the step a path's length is taken to, and the most a typed length may differ
/// from it without being an override.
pub fn length_step(unit: LengthUnit) -> f64 {
    1.0 / steps_per_unit(unit)
}

/// The default belt width, 0.6 m, in `unit`.
pub fn default_width(unit: LengthUnit) -> f64 {
    match unit {
        LengthUnit::Millimetres => 600.0,
        LengthUnit::Centimetres => 60.0,
        LengthUnit::Metres => DEFAULT_WIDTH_M,
        LengthUnit::Kilometres => 0.0006,
        other => DEFAULT_WIDTH_M / metres(other),
    }
}

/// `x` (in `unit`) to the nearest 0.1 mm.
pub fn to_step(x: f64, unit: LengthUnit) -> f64 {
    let k = steps_per_unit(unit);
    (x * k).round() / k
}

/// Whether a field is read by the run or only drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    /// The run reads it, so changing it marks results out of date.
    Sim,
    /// Only the picture uses it.
    Look,
}

/// Each layout field and its tag. A place is *sim* for a block a transport serves (its pickup,
/// drop-off or home), where distances are measured between places, and for a divert gateway
/// placed along a conveyor, whose `s` is where items meet it; elsewhere it is *look*. A
/// junction's kind and transfer time are *sim*.
pub const FIELDS: &[(&str, Tag)] = &[
    ("track.path", Tag::Sim),
    ("track.width", Tag::Look),
    ("place (served by a transport)", Tag::Sim),
    ("place (a divert along a conveyor)", Tag::Sim),
    ("place (otherwise)", Tag::Look),
    ("junction", Tag::Sim),
    ("scenery", Tag::Look),
];

/// A conveyor's run on the floor: its path and the belt's width.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Where the belt runs, in the length unit; its length is the conveyor's (D23). Sim.
    pub path: Path,
    /// The belt's width in the length unit; above 0. Look.
    pub width: f64,
}

impl Track {
    /// A track along `path` (in `unit`) of the default width, 0.6 m.
    pub fn new(path: Path, unit: LengthUnit) -> Self {
        Self {
            path,
            width: default_width(unit),
        }
    }

    /// The path's length in `unit`, the model's, taken to 0.1 mm.
    pub fn length(&self, unit: LengthUnit) -> f64 {
        to_step(self.path.length(), unit)
    }
}

/// Where a block is on the floor.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    /// At a frame: a point, a height and the way it faces.
    At(Frame),
    /// At distance `s` along conveyor `path`'s track, facing along it: a divert on a sorter
    /// loop, a station on a line (D25).
    Along { path: ElementId, s: f64 },
}

/// A look-only object of the scene: a floor, a wall, a pallet, a column. The run never reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenery {
    /// A stable id, unique in the scene.
    pub id: String,
    /// What it is (`floor`, `wall`, `pallet`, `column`, `marking`…); the views draw the kinds they
    /// know and skip the rest, which a file keeps.
    pub kind: String,
    /// Its centre on the floor and the way it faces.
    pub at: Frame,
    /// Its width (across), depth (along its heading) and height, in the length unit.
    pub size: [f64; 3],
}

/// How two sections meet at a [`Junction`] (ROADMAP.md, M5.5). Each joins the end of one
/// section to the start of the next; a section joining another part-way along (a merge at `s`)
/// is not one of these yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JunctionKind {
    /// End to start, the same heading: the item keeps its orientation.
    #[default]
    Inline,
    /// At an angle: the item turns, so the next section's item length (its width here) is the
    /// one that counts there.
    Side,
    /// A turntable between them, whose turn is the transfer time.
    Turntable,
}

impl JunctionKind {
    /// The kind's name in a file.
    pub fn name(self) -> &'static str {
        match self {
            JunctionKind::Inline => "inline",
            JunctionKind::Side => "side",
            JunctionKind::Turntable => "turntable",
        }
    }

    /// The kind a file names, if it is one.
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "inline" => Some(JunctionKind::Inline),
            "side" => Some(JunctionKind::Side),
            "turntable" => Some(JunctionKind::Turntable),
            _ => None,
        }
    }
}

/// Where a flow from one conveyor to another joins their sections: the kind of junction and the
/// time an item takes across it, from leaving the end of the one to getting on the start of
/// the other (0 by default, as M5.1's hand-over). The item takes its room on the next section
/// before it sets off, so a full section holds it at the end of the one before. Sim.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Junction {
    pub kind: JunctionKind,
    /// In the experiment's time unit; 0 or more.
    pub transfer: f64,
}

/// A divert point (D25): a divert gateway placed at `s` along a conveyor's track, which items on
/// that conveyor meet as their front reaches `s` (see [`points`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    /// The divert gateway.
    pub gateway: ElementId,
    /// Its distance along the track, in the length unit.
    pub s: f64,
}

/// The physical layout of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Site {
    /// Each conveyor's track, by the conveyor's id.
    pub paths: BTreeMap<ElementId, Track>,
    /// Each placed block's place, by the block's id.
    pub places: BTreeMap<ElementId, Place>,
    /// The junctions between conveyors, by the id of the flow that joins them.
    pub junctions: BTreeMap<ElementId, Junction>,
    /// Scenery, in id order.
    pub scene: Vec<Scenery>,
}

impl Site {
    /// Whether the model has no layout at all.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
            && self.places.is_empty()
            && self.junctions.is_empty()
            && self.scene.is_empty()
    }

    /// The frame of block `id`: its place's, or for a conveyor, the start of its track. `None`
    /// for a block with neither, or one placed along a conveyor that has no track.
    pub fn frame_of(&self, id: &ElementId) -> Option<Frame> {
        match self.places.get(id) {
            Some(Place::At(f)) => Some(*f),
            Some(Place::Along { path, s }) => Some(self.paths.get(path)?.path.point_at(*s)),
            None => self.paths.get(id).map(|t| t.path.start),
        }
    }

    /// The entries `model` can hold, which are what a file keeps: a track only on a conveyor, a
    /// place only on an element of the model, a place along a conveyor only while that
    /// conveyor has a track, and a junction only on a flow from a conveyor to a conveyor. An
    /// edit that removes a block, or makes a conveyor something else, leaves the rest out.
    pub fn kept(&self, model: &Model) -> Site {
        let is_conveyor = |id: &ElementId| {
            model
                .element(id)
                .is_some_and(|e| matches!(e.kind, ElementKind::Conveyor { .. }))
        };
        let junctions = self
            .junctions
            .iter()
            .filter(|(id, _)| {
                model
                    .connections
                    .iter()
                    .any(|c| &c.id == *id && is_conveyor(&c.from) && is_conveyor(&c.to))
            })
            .map(|(id, j)| (id.clone(), *j))
            .collect();
        let paths: BTreeMap<ElementId, Track> = self
            .paths
            .iter()
            .filter(|(id, _)| {
                model
                    .element(id)
                    .is_some_and(|e| matches!(e.kind, ElementKind::Conveyor { .. }))
            })
            .map(|(id, t)| (id.clone(), t.clone()))
            .collect();
        let places = self
            .places
            .iter()
            .filter(|(id, p)| {
                model.element(id).is_some()
                    && match p {
                        Place::Along { path, .. } => paths.contains_key(path),
                        Place::At(_) => true,
                    }
            })
            .map(|(id, p)| (id.clone(), p.clone()))
            .collect();
        Site {
            paths,
            places,
            junctions,
            scene: self.scene.clone(),
        }
    }

    /// Only the sim fields (D28), for the results baseline: each track's path at the default
    /// width, the places of blocks a transport serves and of diverts along a conveyor, the
    /// junctions, and no scenery.
    pub fn for_results(&self, model: &Model) -> Site {
        Site {
            paths: self
                .paths
                .iter()
                .map(|(id, t)| {
                    (
                        id.clone(),
                        Track::new(t.path.clone(), model.experiment.length_unit),
                    )
                })
                .collect(),
            places: self
                .places
                .iter()
                .filter(|(id, p)| served_by_transport(model, id) || is_divert_along(model, id, p))
                .map(|(id, p)| (id.clone(), p.clone()))
                .collect(),
            junctions: self.junctions.clone(),
            scene: Vec::new(),
        }
    }
}

/// Whether block `id`, at `place`, is a divert gateway placed along a conveyor.
fn is_divert_along(model: &Model, id: &ElementId, place: &Place) -> bool {
    matches!(place, Place::Along { .. })
        && model.element(id).is_some_and(|e| {
            matches!(
                e.kind,
                ElementKind::ExclusiveGateway {
                    routing: Routing::Divert,
                    ..
                }
            )
        })
}

/// The divert points along `conveyor`, in the order its items meet them (D25).
///
/// They are found by following the flows: the divert gateway the conveyor's one flow leads to,
/// if it is placed along this conveyor short of the end of its track; then the gateway that
/// one's default flow leads to, if it is placed along this conveyor too; and so on. An item on
/// the conveyor meets each in turn as its front reaches its `s`. A non-default way of a point
/// leaves the section there; the default flow of the last point is where items go at the end.
///
/// A divert placed at the end of the track (within 0.1 mm) is not a point: the conveyor sends
/// into it at its end, as in M5.1. Nor is a divert placed along a conveyor that does not send
/// into it this way: it diverts at an instant, wherever its senders are, and its place is only
/// drawn. Empty for an element that is not a conveyor with a track.
pub fn points(model: &Model, conveyor: &ElementId) -> Vec<Point> {
    chain(model, conveyor, false)
}

/// The diverts placed along `conveyor` in the order its items reach them by the flows, as
/// [`points`] finds them, but going on past one placed at the end of the track: the order of
/// the flows and the order along the track are checked over all of them.
pub fn placed_in_turn(model: &Model, conveyor: &ElementId) -> Vec<Point> {
    chain(model, conveyor, true)
}

/// The chain of diverts placed along `conveyor` that its items reach in turn by the flows;
/// with `past_end`, one at the end of the track does not end it.
fn chain(model: &Model, conveyor: &ElementId, past_end: bool) -> Vec<Point> {
    let unit = model.experiment.length_unit;
    let Some(track) = model.site.paths.get(conveyor) else {
        return Vec::new();
    };
    if !model
        .element(conveyor)
        .is_some_and(|e| matches!(e.kind, ElementKind::Conveyor { .. }))
    {
        return Vec::new();
    }
    let end = track.length(unit) - length_step(unit);
    let mut outs = model.connections.iter().filter(|c| &c.from == conveyor);
    let (Some(first), None) = (outs.next(), outs.next()) else {
        return Vec::new();
    };
    let mut out: Vec<Point> = Vec::new();
    let mut next = Some(first.to.clone());
    while let Some(g) = next.take() {
        if out.iter().any(|p| p.gateway == g) {
            break;
        }
        let Some(ElementKind::ExclusiveGateway {
            default,
            routing: Routing::Divert,
        }) = model.element(&g).map(|e| &e.kind)
        else {
            break;
        };
        let Some(Place::Along { path, s }) = model.site.places.get(&g) else {
            break;
        };
        if path != conveyor || s.is_nan() || (*s >= end && !past_end) {
            break;
        }
        out.push(Point {
            gateway: g.clone(),
            s: *s,
        });
        next = default
            .as_ref()
            .and_then(|d| model.connections.iter().find(|c| &c.id == d))
            .map(|c| c.to.clone());
    }
    out
}

/// Gives each conveyor in `elements` with a track in `site` its path's length (D23) where its
/// typed length agrees with it to within 0.1 mm (in `unit`, the model's); a length that differs
/// more stays, as an override. Reading a file and reading JSON both do this, so a model reads the
/// same either way.
pub fn lengths_from_paths(site: &Site, elements: &mut [Element], unit: LengthUnit) {
    for e in elements {
        if let (Some(t), ElementKind::Conveyor { length, .. }) =
            (site.paths.get(&e.id), &mut e.kind)
        {
            let drawn = t.length(unit);
            if (drawn - *length).abs() <= length_step(unit) {
                *length = drawn;
            }
        }
    }
}

/// Whether a transport carries to or from block `id`, or a mobile pool's units start there.
pub fn served_by_transport(model: &Model, id: &ElementId) -> bool {
    let is_transport = |e: &ElementId| {
        model
            .element(e)
            .is_some_and(|x| matches!(x.kind, ElementKind::Transport { .. }))
    };
    model
        .connections
        .iter()
        .any(|c| (&c.to == id && is_transport(&c.from)) || (&c.from == id && is_transport(&c.to)))
        || model
            .resources
            .iter()
            .any(|r| r.mobility.as_ref().is_some_and(|m| &m.home == id))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    /// The unit of the models in these tests unless one says otherwise.
    const M: LengthUnit = LengthUnit::Metres;
    use crate::{Connection, Dist, Element, Segment};

    fn id(s: &str) -> ElementId {
        ElementId::unchecked(s)
    }

    /// A racetrack loop: two 10 m straights and two half circles of radius 5/π (5 m each), 30 m round.
    fn racetrack() -> Path {
        let r = 5.0 / std::f64::consts::PI;
        let half = Segment::Arc {
            radius: r,
            angle_deg: 180.0,
            rise: 0.0,
        };
        let straight = Segment::Line {
            length: 10.0,
            rise: 0.0,
        };
        Path::new(Frame::default(), vec![straight, half, straight, half])
    }

    #[test]
    fn a_track_is_as_long_as_its_path_to_a_tenth_of_a_millimetre() {
        let t = Track::new(racetrack(), M);
        // The arcs make the raw sum differ from 30 in the last bits; the length is 30 exactly,
        // so 30 m of 0.6 m slots is 50 of them, not 49.
        assert_eq!(t.length(M), 30.0);
        assert_eq!((t.length(M) / 0.6).floor(), 50.0);
        assert_eq!(to_step(1.23456, M), 1.2346);
    }

    /// The override's message gives both lengths to the step a path is taken to: in kilometres
    /// 0.012 and 0.012001 differ by 1 m and must read differently; in metres the numbers stay
    /// short.
    #[test]
    fn the_override_message_tells_the_two_lengths_apart_in_any_unit() {
        let message = |unit: LengthUnit, typed: f64, drawn: f64| {
            let mut m = Model::default();
            m.experiment.length_unit = unit;
            m.elements.push(Element::new(
                id("belt"),
                "Belt",
                ElementKind::Conveyor {
                    length: typed,
                    speed: 1.0,
                    accumulating: true,
                    item_length: typed / 100.0,
                    gap: 0.0,
                    shape: crate::ConveyorShape::Straight,
                },
            ));
            let line = Path::new(
                Frame::default(),
                vec![Segment::Line {
                    length: drawn,
                    rise: 0.0,
                }],
            );
            m.site.paths.insert(id("belt"), Track::new(line, unit));
            m.validate()
                .iter()
                .find(|d| matches!(d.problem, crate::Problem::ConveyorLengthMismatch { .. }))
                .map(|d| d.describe(&m).english())
                .unwrap_or_default()
        };
        let km = message(LengthUnit::Kilometres, 0.012, 0.012001);
        assert!(
            km.contains("is 0.012001 long") && km.contains("is 0.012,"),
            "{km}"
        );
        let m = message(LengthUnit::Metres, 32.0, 30.0);
        assert!(m.contains("is 30 long") && m.contains("is 32,"), "{m}");
    }

    #[test]
    fn a_typed_length_that_differs_from_the_track_is_an_override_and_is_flagged() {
        let belt = |length: f64| {
            let mut m = Model::default();
            m.elements.push(Element::new(
                id("loop"),
                "Loop",
                ElementKind::Conveyor {
                    length,
                    speed: 0.5,
                    accumulating: false,
                    item_length: 0.4,
                    gap: 0.2,
                    shape: crate::ConveyorShape::Straight,
                },
            ));
            m.site.paths.insert(id("loop"), Track::new(racetrack(), M));
            m.validate()
                .into_iter()
                .filter(|d| matches!(d.problem, crate::Problem::ConveyorLengthMismatch { .. }))
                .count()
        };
        // 30 m agrees with the racetrack; 32 m overrides it, and says so.
        assert_eq!(belt(30.0), 0);
        assert_eq!(belt(32.0), 1);
    }

    /// The same 30 m racetrack drawn in millimetres or centimetres: 0.1 mm and 0.6 m are the
    /// same distances whatever the unit.
    #[test]
    fn the_step_and_the_default_width_are_the_same_distances_in_every_unit() {
        for (unit, k) in [
            (LengthUnit::Millimetres, 1000.0),
            (LengthUnit::Centimetres, 100.0),
            (LengthUnit::Metres, 1.0),
        ] {
            let t = Track::new(racetrack().scaled(k), unit);
            assert_eq!(t.length(unit), 30.0 * k, "{unit:?}");
            assert!((t.width * metres(unit) - 0.6).abs() < 1e-12, "{unit:?}");
            assert!(
                (length_step(unit) * metres(unit) - 1e-4).abs() < 1e-15,
                "{unit:?}"
            );
        }
        // 0.05 mm is no override in a millimetre model; 0.2 mm is. In centimetres, 0.005 cm
        // (0.05 mm) is none either.
        let flagged = |unit: LengthUnit, length: f64, track: f64| {
            let mut m = Model::default();
            m.experiment.length_unit = unit;
            m.elements.push(Element::new(
                id("belt"),
                "Belt",
                ElementKind::Conveyor {
                    length,
                    speed: 1.0,
                    accumulating: false,
                    item_length: 1.0,
                    gap: 0.0,
                    shape: crate::ConveyorShape::Straight,
                },
            ));
            let line = Path::new(
                Frame::default(),
                vec![Segment::Line {
                    length: track,
                    rise: 0.0,
                }],
            );
            m.site.paths.insert(id("belt"), Track::new(line, unit));
            m.validate()
                .iter()
                .any(|d| matches!(d.problem, crate::Problem::ConveyorLengthMismatch { .. }))
        };
        assert!(!flagged(LengthUnit::Millimetres, 12_000.05, 12_000.0));
        assert!(flagged(LengthUnit::Millimetres, 12_000.2, 12_000.0));
        assert!(!flagged(LengthUnit::Centimetres, 1_200.005, 1_200.0));
        assert!(flagged(LengthUnit::Centimetres, 1_200.05, 1_200.0));
        assert_eq!(to_step(12.34567, LengthUnit::Millimetres), 12.3);
    }

    #[test]
    fn what_a_model_can_hold_is_kept_and_the_rest_left_out() {
        let mut m = Model::default();
        m.elements.push(Element::new(
            id("loop"),
            "Loop",
            ElementKind::Conveyor {
                length: 30.0,
                speed: 0.5,
                accumulating: false,
                item_length: 0.4,
                gap: 0.2,
                shape: crate::ConveyorShape::Straight,
            },
        ));
        m.elements.push(Element::new(
            id("divert"),
            "Divert",
            ElementKind::ExclusiveGateway {
                default: None,
                routing: crate::Routing::Standard,
            },
        ));
        m.site.paths.insert(id("loop"), Track::new(racetrack(), M));
        m.site.places.insert(
            id("divert"),
            Place::Along {
                path: id("loop"),
                s: 0.0,
            },
        );
        m.site
            .places
            .insert(id("gone"), Place::At(Frame::default()));
        let kept = m.site.kept(&m);
        assert_eq!(kept.paths.len(), 1);
        assert_eq!(kept.places.len(), 1, "a place on no element goes");
        // The loop made a delay: its track goes, and so does the divert placed along it.
        m.elements[0].kind = ElementKind::Delay {
            duration: Dist::Constant(1.0),
            processing_by: None,
        };
        let kept = m.site.kept(&m);
        assert!(kept.paths.is_empty() && kept.places.is_empty());
    }

    /// A 10 m belt sending into divert `d1`, whose default leads to divert `d2`, whose default
    /// leads to `out`; each divert's other way is a lane.
    fn two_diverts(s1: f64, s2: f64) -> Model {
        let mut m = Model::default();
        let divert = |name: &str, default: &str| {
            Element::new(
                id(name),
                name,
                ElementKind::ExclusiveGateway {
                    default: Some(id(default)),
                    routing: Routing::Divert,
                },
            )
        };
        m.elements.push(Element::new(
            id("belt"),
            "Belt",
            ElementKind::Conveyor {
                length: 10.0,
                speed: 1.0,
                accumulating: true,
                item_length: 0.5,
                gap: 0.5,
                shape: crate::ConveyorShape::Straight,
            },
        ));
        m.elements.push(divert("d1", "d1-d2"));
        m.elements.push(divert("d2", "d2-out"));
        for (c, from, to) in [
            ("belt-d1", "belt", "d1"),
            ("d1-d2", "d1", "d2"),
            ("d1-lane", "d1", "lane1"),
            ("d2-out", "d2", "out"),
            ("d2-lane", "d2", "lane2"),
        ] {
            m.connections.push(Connection::new(id(c), id(from), id(to)));
        }
        let line = Path::new(
            Frame::default(),
            vec![Segment::Line {
                length: 10.0,
                rise: 0.0,
            }],
        );
        m.site.paths.insert(id("belt"), Track::new(line, M));
        for (g, s) in [("d1", s1), ("d2", s2)] {
            m.site.places.insert(
                id(g),
                Place::Along {
                    path: id("belt"),
                    s,
                },
            );
        }
        m
    }

    #[test]
    fn the_points_of_a_belt_follow_its_flow_and_the_diverts_defaults() {
        let at = |m: &Model| -> Vec<(String, f64)> {
            points(m, &id("belt"))
                .into_iter()
                .map(|p| (p.gateway.to_string(), p.s))
                .collect()
        };
        let m = two_diverts(3.0, 6.0);
        assert_eq!(at(&m), [("d1".to_owned(), 3.0), ("d2".to_owned(), 6.0)]);
        // A divert at the end of the track is the belt's end, as in M5.1: the chain stops there.
        assert_eq!(at(&two_diverts(3.0, 10.0)), [("d1".to_owned(), 3.0)]);
        assert!(at(&two_diverts(10.0, 6.0)).is_empty());
        // Validation follows the flows past one at the end, so that order is checked too.
        let past = two_diverts(10.0, 6.0);
        assert_eq!(placed_in_turn(&past, &id("belt")).len(), 2);
        assert!(
            past.validate()
                .iter()
                .any(|d| matches!(d.problem, crate::Problem::PointsOutOfOrder { .. }))
        );
        // Placed along another conveyor, it is not one of this belt's points.
        let mut other = two_diverts(3.0, 6.0);
        other.site.places.insert(
            id("d1"),
            Place::Along {
                path: id("elsewhere"),
                s: 3.0,
            },
        );
        assert!(at(&other).is_empty());
        // A divert's s is read by the run, so it is in the results baseline; a junction too.
        let sim = m.site.for_results(&m);
        assert!(sim.places.contains_key(&id("d1")) && sim.places.contains_key(&id("d2")));
    }

    #[test]
    fn a_junction_is_kept_only_between_two_conveyors() {
        let mut m = two_diverts(3.0, 6.0);
        m.site.junctions.insert(id("belt-d1"), Junction::default());
        assert!(m.site.kept(&m).junctions.is_empty());
        m.elements.push(Element::new(
            id("next"),
            "Next",
            ElementKind::Conveyor {
                length: 5.0,
                speed: 1.0,
                accumulating: true,
                item_length: 0.5,
                gap: 0.5,
                shape: crate::ConveyorShape::Straight,
            },
        ));
        m.connections
            .push(Connection::new(id("belt-next"), id("belt"), id("next")));
        m.site.junctions.insert(
            id("belt-next"),
            Junction {
                kind: JunctionKind::Side,
                transfer: 1.5,
            },
        );
        let kept = m.site.kept(&m);
        assert_eq!(kept.junctions.len(), 1);
        assert_eq!(kept.junctions[&id("belt-next")].transfer, 1.5);
        assert_eq!(m.site.for_results(&m).junctions.len(), 2);
        for k in [
            JunctionKind::Inline,
            JunctionKind::Side,
            JunctionKind::Turntable,
        ] {
            assert_eq!(JunctionKind::from_name(k.name()), Some(k));
        }
    }

    #[test]
    fn a_place_along_a_track_is_on_its_path_and_a_conveyor_starts_where_its_track_does() {
        let mut site = Site::default();
        site.paths.insert(id("loop"), Track::new(racetrack(), M));
        site.places.insert(
            id("divert"),
            Place::Along {
                path: id("loop"),
                s: 4.0,
            },
        );
        site.places
            .insert(id("pack"), Place::At(Frame::new(2.0, -5.0, 0.0, -90.0)));
        let d = site.frame_of(&id("divert")).unwrap();
        assert!((d.x - 4.0).abs() < 1e-12 && d.y.abs() < 1e-12);
        assert_eq!(site.frame_of(&id("pack")).unwrap().y, -5.0);
        assert_eq!(site.frame_of(&id("loop")), Some(Frame::default()));
        assert_eq!(site.frame_of(&id("nowhere")), None);
    }

    #[test]
    fn only_sim_fields_enter_the_results_baseline() {
        let mut m = Model::default();
        m.elements.push(Element::new(
            id("cart"),
            "Cart",
            ElementKind::Transport {
                pool: id("carts"),
                load: Dist::Constant(1.0),
                unload: Dist::Constant(1.0),
            },
        ));
        m.connections
            .push(Connection::new(id("a-cart"), id("a"), id("cart")));
        let mut site = Site::default();
        let mut wide = Track::new(racetrack(), M);
        wide.width = 1.0;
        site.paths.insert(id("loop"), wide);
        site.places.insert(id("a"), Place::At(Frame::default()));
        site.places.insert(id("b"), Place::At(Frame::default()));
        site.scene.push(Scenery {
            id: "floor".to_owned(),
            kind: "floor".to_owned(),
            at: Frame::default(),
            size: [20.0, 10.0, 0.0],
        });
        let sim = site.for_results(&m);
        assert_eq!(sim.paths[&id("loop")].width, DEFAULT_WIDTH_M);
        assert_eq!(sim.paths[&id("loop")].path, racetrack());
        // `a` is the transport's pickup; `b` is only drawn.
        assert!(sim.places.contains_key(&id("a")) && !sim.places.contains_key(&id("b")));
        assert!(sim.scene.is_empty());
        // A look-only change leaves the baseline as it was.
        let mut moved = site.clone();
        moved.scene[0].size[0] = 30.0;
        moved
            .places
            .insert(id("b"), Place::At(Frame::new(1.0, 1.0, 0.0, 0.0)));
        if let Some(t) = moved.paths.get_mut(&id("loop")) {
            t.width = 0.8;
        }
        assert_eq!(moved.for_results(&m), sim);
        // A sim change does not.
        moved
            .paths
            .insert(id("loop"), Track::new(Path::default(), M));
        assert_ne!(moved.for_results(&m), sim);
    }
}
