//! The physical layout (ROADMAP.md, M5.5): where things are, apart from the flow diagram.
//!
//! A model's [`Site`] holds each conveyor's [`Track`] (its [`Path`] and the belt's width), each
//! block's [`Place`] (a frame, or a distance along a conveyor's path), and look-only
//! [`Scenery`]. Distances are in the experiment's length unit, like a conveyor's `length`, so a
//! conveyor's length is its path's length with no conversion; headings are in degrees.
//!
//! **Length from the path (D23).** A conveyor with a track runs for its path's length, taken to
//! [`LENGTH_STEP`] (the 0.1 mm a file keeps). A typed length that differs is an override: the run
//! uses it, and validation warns, naming both ([`crate::Problem::ConveyorLengthMismatch`]).
//!
//! **Sim and look (D28).** Every field is tagged ([`FIELDS`]): the run reads the *sim* ones, which
//! enter the results baseline ([`Site::for_results`]); a *look* field never marks results out of
//! date.
//!
//! A block with no place is unplaced: it runs, but the Layout and 3D views list it rather than
//! guess where it is.

use std::collections::BTreeMap;

use crate::{ElementId, ElementKind, Frame, Model, Path};

/// The width a belt is drawn with when its track does not say, in metres.
pub const DEFAULT_WIDTH: f64 = 0.6;

/// The step a path's length is taken to: a ten-thousandth of the length unit (0.1 mm in
/// metres), the precision a file keeps, so a loop drawn with arcs is exactly as long as its
/// straight sum says and a typed length that agrees with it is no override.
pub const LENGTH_STEP: f64 = 1e-4;

/// Whether a field is read by the run or only drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    /// The run reads it, so changing it marks results out of date.
    Sim,
    /// Only the picture uses it.
    Look,
}

/// Each layout field and its tag. A place is *sim* only for a block a transport serves (its
/// pickup, drop-off or home), where distances are measured between places; elsewhere it is
/// *look*.
pub const FIELDS: &[(&str, Tag)] = &[
    ("track.path", Tag::Sim),
    ("track.width", Tag::Look),
    ("place (served by a transport)", Tag::Sim),
    ("place (otherwise)", Tag::Look),
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
    /// A track along `path` of the default width (in metres).
    pub fn new(path: Path) -> Self {
        Self {
            path,
            width: DEFAULT_WIDTH,
        }
    }

    /// The path's length, taken to [`LENGTH_STEP`].
    pub fn length(&self) -> f64 {
        to_step(self.path.length())
    }
}

/// `x` to the nearest [`LENGTH_STEP`].
pub fn to_step(x: f64) -> f64 {
    let steps = 1.0 / LENGTH_STEP;
    (x * steps).round() / steps
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

/// The physical layout of a model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Site {
    /// Each conveyor's track, by the conveyor's id.
    pub paths: BTreeMap<ElementId, Track>,
    /// Each placed block's place, by the block's id.
    pub places: BTreeMap<ElementId, Place>,
    /// Scenery, in id order.
    pub scene: Vec<Scenery>,
}

impl Site {
    /// Whether the model has no layout at all.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.places.is_empty() && self.scene.is_empty()
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
    /// place only on an element of the model, and a place along a conveyor only while that
    /// conveyor has a track. An edit that removes a block, or makes a conveyor something else,
    /// leaves the rest out.
    pub fn kept(&self, model: &Model) -> Site {
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
            scene: self.scene.clone(),
        }
    }

    /// Only the sim fields (D28), for the results baseline: each track's path at the default
    /// width, the places of blocks a transport serves, and no scenery.
    pub fn for_results(&self, model: &Model) -> Site {
        Site {
            paths: self
                .paths
                .iter()
                .map(|(id, t)| (id.clone(), Track::new(t.path.clone())))
                .collect(),
            places: self
                .places
                .iter()
                .filter(|(id, _)| served_by_transport(model, id))
                .map(|(id, p)| (id.clone(), p.clone()))
                .collect(),
            scene: Vec::new(),
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
        let t = Track::new(racetrack());
        // The arcs make the raw sum differ from 30 in the last bits; the length is 30 exactly,
        // so 30 m of 0.6 m slots is 50 of them, not 49.
        assert_eq!(t.length(), 30.0);
        assert_eq!((t.length() / 0.6).floor(), 50.0);
        assert_eq!(to_step(1.23456), 1.2346);
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
            m.site.paths.insert(id("loop"), Track::new(racetrack()));
            m.validate()
                .into_iter()
                .filter(|d| matches!(d.problem, crate::Problem::ConveyorLengthMismatch { .. }))
                .count()
        };
        // 30 m agrees with the racetrack; 32 m overrides it, and says so.
        assert_eq!(belt(30.0), 0);
        assert_eq!(belt(32.0), 1);
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
        m.site.paths.insert(id("loop"), Track::new(racetrack()));
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

    #[test]
    fn a_place_along_a_track_is_on_its_path_and_a_conveyor_starts_where_its_track_does() {
        let mut site = Site::default();
        site.paths.insert(id("loop"), Track::new(racetrack()));
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
        let mut wide = Track::new(racetrack());
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
        assert_eq!(sim.paths[&id("loop")].width, DEFAULT_WIDTH);
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
        moved.paths.insert(id("loop"), Track::new(Path::default()));
        assert_ne!(moved.for_results(&m), sim);
    }
}
