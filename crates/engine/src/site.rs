//! The physical layout as JSON ([`stochcraft_model::Site`]): conveyors' tracks, blocks' places
//! and look-only scenery, in the model's length unit and degrees (ROADMAP.md, M5.5).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use stochcraft_model::site::DEFAULT_WIDTH;
use stochcraft_model::{ElementId, Frame, Path, Place, Scenery, Segment, Site, Track};

/// A point on the floor, its height and the way it faces.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FrameDoc {
    pub x: f64,
    pub y: f64,
    /// Height above the floor; 0 when left out.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub z: f64,
    /// Degrees: 0 along +x, positive turning left; 0 when left out.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub heading: f64,
}

fn is_zero(x: &f64) -> bool {
    *x == 0.0
}

fn default_width() -> f64 {
    DEFAULT_WIDTH
}

fn is_default_width(x: &f64) -> bool {
    *x == DEFAULT_WIDTH
}

impl From<Frame> for FrameDoc {
    fn from(f: Frame) -> Self {
        Self {
            x: f.x,
            y: f.y,
            z: f.z,
            heading: f.heading_deg,
        }
    }
}

impl From<FrameDoc> for Frame {
    fn from(f: FrameDoc) -> Self {
        Frame::new(f.x, f.y, f.z, f.heading)
    }
}

/// One run of a track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SegmentDoc {
    /// Straight on for `length` (along the floor), climbing `rise`.
    Line {
        length: f64,
        #[serde(default, skip_serializing_if = "is_zero")]
        rise: f64,
    },
    /// A circular arc of `radius` turning `angle` degrees (positive left), climbing `rise`.
    Arc {
        radius: f64,
        angle: f64,
        #[serde(default, skip_serializing_if = "is_zero")]
        rise: f64,
    },
    /// A helix of `radius` through `turns` turns, climbing `rise` over all of them.
    Helix { radius: f64, rise: f64, turns: f64 },
}

impl From<Segment> for SegmentDoc {
    fn from(s: Segment) -> Self {
        match s {
            Segment::Line { length, rise } => SegmentDoc::Line { length, rise },
            Segment::Arc {
                radius,
                angle_deg,
                rise,
            } => SegmentDoc::Arc {
                radius,
                angle: angle_deg,
                rise,
            },
            Segment::Helix {
                radius,
                rise,
                turns,
            } => SegmentDoc::Helix {
                radius,
                rise,
                turns,
            },
        }
    }
}

impl From<SegmentDoc> for Segment {
    fn from(s: SegmentDoc) -> Self {
        match s {
            SegmentDoc::Line { length, rise } => Segment::Line { length, rise },
            SegmentDoc::Arc {
                radius,
                angle,
                rise,
            } => Segment::Arc {
                radius,
                angle_deg: angle,
                rise,
            },
            SegmentDoc::Helix {
                radius,
                rise,
                turns,
            } => Segment::Helix {
                radius,
                rise,
                turns,
            },
        }
    }
}

/// A conveyor's track: where its path starts, its segments, and the belt's width. The
/// conveyor's length is the path's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackDoc {
    pub start: FrameDoc,
    pub segments: Vec<SegmentDoc>,
    /// The belt's width, drawn only; 0.6 when left out.
    #[serde(default = "default_width", skip_serializing_if = "is_default_width")]
    pub width: f64,
}

/// Where a block is: along a conveyor's track (`path`, at `s`), or at a frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum PlaceDoc {
    Along { path: String, s: f64 },
    At(FrameDoc),
}

/// A look-only object of the scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneryDoc {
    pub id: String,
    pub kind: String,
    pub at: FrameDoc,
    /// Width, depth and height.
    pub size: [f64; 3],
}

/// The physical layout: tracks and places by element id, and scenery.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SiteDoc {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub paths: BTreeMap<String, TrackDoc>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub places: BTreeMap<String, PlaceDoc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene: Vec<SceneryDoc>,
}

impl SiteDoc {
    /// Whether there is no layout (a model without one leaves `site` out).
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.places.is_empty() && self.scene.is_empty()
    }
}

impl From<&Site> for SiteDoc {
    fn from(s: &Site) -> Self {
        Self {
            paths: s
                .paths
                .iter()
                .map(|(id, t)| {
                    (
                        id.to_string(),
                        TrackDoc {
                            start: t.path.start.into(),
                            segments: t.path.segments.iter().map(|x| (*x).into()).collect(),
                            width: t.width,
                        },
                    )
                })
                .collect(),
            places: s
                .places
                .iter()
                .map(|(id, p)| {
                    let doc = match p {
                        Place::At(f) => PlaceDoc::At((*f).into()),
                        Place::Along { path, s } => PlaceDoc::Along {
                            path: path.to_string(),
                            s: *s,
                        },
                    };
                    (id.to_string(), doc)
                })
                .collect(),
            scene: s
                .scene
                .iter()
                .map(|o| SceneryDoc {
                    id: o.id.clone(),
                    kind: o.kind.clone(),
                    at: o.at.into(),
                    size: o.size,
                })
                .collect(),
        }
    }
}

impl From<SiteDoc> for Site {
    fn from(s: SiteDoc) -> Self {
        let mut scene: Vec<Scenery> = s
            .scene
            .into_iter()
            .map(|o| Scenery {
                id: o.id,
                kind: o.kind,
                at: o.at.into(),
                size: o.size,
            })
            .collect();
        scene.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            paths: s
                .paths
                .into_iter()
                .map(|(id, t)| {
                    let path = Path::new(
                        t.start.into(),
                        t.segments.into_iter().map(Into::into).collect(),
                    );
                    (
                        ElementId::unchecked(&id),
                        Track {
                            path,
                            width: t.width,
                        },
                    )
                })
                .collect(),
            places: s
                .places
                .into_iter()
                .map(|(id, p)| {
                    let place = match p {
                        PlaceDoc::At(f) => Place::At(f.into()),
                        PlaceDoc::Along { path, s } => Place::Along {
                            path: ElementId::unchecked(&path),
                            s,
                        },
                    };
                    (ElementId::unchecked(&id), place)
                })
                .collect(),
            scene,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_site_goes_to_json_and_back_unchanged() {
        let mut site = Site::default();
        site.paths.insert(
            ElementId::unchecked("loop"),
            Track {
                path: Path::new(
                    Frame::new(0.0, 0.0, 0.0, 0.0),
                    vec![
                        Segment::Line {
                            length: 10.0,
                            rise: 0.0,
                        },
                        Segment::Arc {
                            radius: 1.5,
                            angle_deg: 180.0,
                            rise: 0.0,
                        },
                    ],
                ),
                width: 0.8,
            },
        );
        site.places.insert(
            ElementId::unchecked("divert"),
            Place::Along {
                path: ElementId::unchecked("loop"),
                s: 2.0,
            },
        );
        site.places.insert(
            ElementId::unchecked("pack"),
            Place::At(Frame::new(2.0, -5.0, 0.0, -90.0)),
        );
        site.scene.push(Scenery {
            id: "floor".to_owned(),
            kind: "floor".to_owned(),
            at: Frame::new(5.0, -2.0, 0.0, 0.0),
            size: [24.0, 16.0, 0.0],
        });
        let json = serde_json::to_value(SiteDoc::from(&site)).unwrap();
        let back: SiteDoc = serde_json::from_value(json).unwrap();
        assert_eq!(Site::from(back), site);
        assert!(SiteDoc::from(&Site::default()).is_empty());
    }
}
