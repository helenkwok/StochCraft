//! The physical layout in a file (ROADMAP.md, M5.5; D24): StochCraft's vendor extensions
//! `stochcraft:path` (a conveyor's track) and `stochcraft:place` (a block's place) on the
//! elements' `ElementParameters`, and `stochcraft:scene` (look-only scenery) on the scenario.
//!
//! ```xml
//! <bpsim:VendorExtension name="stochcraft:path" xmlns:stochcraft="…">
//!   <stochcraft:path x="0" y="0" heading="0" width="0.8">
//!     <stochcraft:line length="10"/>
//!     <stochcraft:arc radius="3.18" angle="180"/>
//!     <stochcraft:helix radius="1.5" rise="3" turns="2"/>
//!   </stochcraft:path>
//! </bpsim:VendorExtension>
//! <bpsim:VendorExtension name="stochcraft:place" xmlns:stochcraft="…">
//!   <stochcraft:place x="2" y="-5" heading="-90"/>   or   <stochcraft:place path="loop" s="0"/>
//! </bpsim:VendorExtension>
//! <bpsim:VendorExtension name="stochcraft:scene" xmlns:stochcraft="…">
//!   <stochcraft:scene version="1">
//!     <stochcraft:object id="floor" kind="floor" x="5" y="-2" width="24" depth="16"/>
//!   </stochcraft:scene>
//! </bpsim:VendorExtension>
//! ```
//!
//! Distances are in the scenario's length unit, headings and angles in degrees; `z`, `heading`,
//! `rise`, `width` (0.6 m for a track) and a size of 0 are left out when they are the default.
//! Numbers are written in full, so a file reads back as exactly the layout that was saved.
//! Scenery objects are written in id order.

use stochcraft_model::site::{DEFAULT_WIDTH, LENGTH_STEP};
use stochcraft_model::{
    Element as ModelElement, ElementId, ElementKind, Frame, Path, Place, Scenery, Segment, Site,
    Track,
};

use crate::Note;
use crate::bpsim::{self, STOCHCRAFT_NS, Scenario};
use crate::duration::trim_float;
use crate::xml::Element;

/// The version of the scene's vocabulary this reader knows.
pub const SCENE_VERSION: &str = "1";

fn num(v: f64) -> String {
    trim_float(v)
}

fn st(local: &str) -> Element {
    Element::new(&format!("stochcraft:{local}"), Some(STOCHCRAFT_NS))
}

/// A vendor extension called `name` holding `body`.
fn extension(name: &str, body: Element) -> Element {
    Element::new("bpsim:VendorExtension", Some(bpsim::NS2))
        .with_attr("name", name)
        .with_attr("xmlns:stochcraft", STOCHCRAFT_NS)
        .with_child(body)
}

/// `e` with attribute `name` set to `v`, unless `v` is the default `default`.
fn with_num(e: Element, name: &str, v: f64, default: f64) -> Element {
    if v == default {
        e
    } else {
        e.with_attr(name, &num(v))
    }
}

/// `e` with the frame's attributes.
fn with_frame(e: Element, f: &Frame) -> Element {
    let e = e.with_attr("x", &num(f.x)).with_attr("y", &num(f.y));
    let e = with_num(e, "z", f.z, 0.0);
    with_num(e, "heading", f.heading_deg, 0.0)
}

/// A conveyor's track as its `stochcraft:path` vendor extension.
pub fn path_element(t: &Track) -> Element {
    let mut body = with_num(
        with_frame(st("path"), &t.path.start),
        "width",
        t.width,
        DEFAULT_WIDTH,
    );
    for seg in &t.path.segments {
        let child = match *seg {
            Segment::Line { length, rise } => with_num(
                st("line").with_attr("length", &num(length)),
                "rise",
                rise,
                0.0,
            ),
            Segment::Arc {
                radius,
                angle_deg,
                rise,
            } => with_num(
                st("arc")
                    .with_attr("radius", &num(radius))
                    .with_attr("angle", &num(angle_deg)),
                "rise",
                rise,
                0.0,
            ),
            Segment::Helix {
                radius,
                rise,
                turns,
            } => st("helix")
                .with_attr("radius", &num(radius))
                .with_attr("rise", &num(rise))
                .with_attr("turns", &num(turns)),
        };
        body.push(child);
    }
    extension(bpsim::PATH, body)
}

/// A block's place as its `stochcraft:place` vendor extension.
pub fn place_element(p: &Place) -> Element {
    let body = match p {
        Place::At(f) => with_frame(st("place"), f),
        Place::Along { path, s } => st("place")
            .with_attr("path", path.as_str())
            .with_attr("s", &num(*s)),
    };
    extension(bpsim::PLACE, body)
}

/// The scenery as the scenario's `stochcraft:scene` vendor extension, objects in id order;
/// `None` for none.
pub fn scene_element(scene: &[Scenery]) -> Option<Element> {
    if scene.is_empty() {
        return None;
    }
    let mut sorted: Vec<&Scenery> = scene.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let mut body = st("scene").with_attr("version", SCENE_VERSION);
    for o in sorted {
        let e = st("object")
            .with_attr("id", &o.id)
            .with_attr("kind", &o.kind);
        let e = with_frame(e, &o.at);
        let e = with_num(e, "width", o.size[0], 0.0);
        let e = with_num(e, "depth", o.size[1], 0.0);
        body.push(with_num(e, "height", o.size[2], 0.0));
    }
    Some(extension(bpsim::SCENE, body))
}

/// A number attribute: `default` when it is missing, `Err` naming it when it is not a finite
/// number.
fn number(e: &Element, name: &str, default: Option<f64>) -> Result<f64, String> {
    match e.attr(name) {
        None => default.ok_or_else(|| format!("needs `{name}`")),
        Some(raw) => match raw.trim().parse::<f64>() {
            Ok(x) if x.is_finite() => Ok(x),
            _ => Err(format!("has `{name}` \"{raw}\", which is not a number")),
        },
    }
}

fn read_frame(e: &Element) -> Result<Frame, String> {
    Ok(Frame::new(
        number(e, "x", None)?,
        number(e, "y", None)?,
        number(e, "z", Some(0.0))?,
        number(e, "heading", Some(0.0))?,
    ))
}

/// The body of vendor extension `ext` called `local`.
fn body<'a>(ext: &'a Element, local: &str) -> Option<&'a Element> {
    ext.elements().find(|e| e.local() == local)
}

/// A track from a `stochcraft:path` vendor extension.
pub fn read_track(ext: &Element) -> Result<Track, String> {
    let b = body(ext, "path").ok_or("has no `path` element")?;
    let start = read_frame(b)?;
    let width = number(b, "width", Some(DEFAULT_WIDTH))?;
    if width <= 0.0 {
        return Err("has a `width` that is not above 0".to_owned());
    }
    let mut segments = Vec::new();
    for s in b.elements() {
        let rise = number(s, "rise", Some(0.0))?;
        let seg = match s.local() {
            "line" => Segment::Line {
                length: number(s, "length", None)?,
                rise,
            },
            "arc" => Segment::Arc {
                radius: number(s, "radius", None)?,
                angle_deg: number(s, "angle", None)?,
                rise,
            },
            "helix" => Segment::Helix {
                radius: number(s, "radius", None)?,
                rise: number(s, "rise", None)?,
                turns: number(s, "turns", None)?,
            },
            other => return Err(format!("has a segment `{other}`, not line, arc or helix")),
        };
        segments.push(seg);
    }
    if segments.is_empty() {
        return Err("has no segments".to_owned());
    }
    Ok(Track {
        path: Path::new(start, segments),
        width,
    })
}

/// A place from a `stochcraft:place` vendor extension.
pub fn read_place(ext: &Element) -> Result<Place, String> {
    let b = body(ext, "place").ok_or("has no `place` element")?;
    match b.attr("path") {
        Some(path) => Ok(Place::Along {
            path: ElementId::unchecked(path.trim()),
            s: number(b, "s", Some(0.0))?,
        }),
        None => Ok(Place::At(read_frame(b)?)),
    }
}

/// The scenery of a `stochcraft:scene` vendor extension. An object that cannot be read is
/// noted and left out, as is a second object with an id already used.
pub fn read_scene(ext: &Element, notes: &mut Vec<Note>) -> Vec<Scenery> {
    let mut out: Vec<Scenery> = Vec::new();
    let Some(b) = body(ext, "scene") else {
        return out;
    };
    if let Some(v) = b.attr("version").filter(|v| v.trim() != SCENE_VERSION) {
        notes.push(Note::warning(
            None,
            format!("the scene is in version {v}, which this version of StochCraft may read only in part"),
        ));
    }
    for o in b.elements().filter(|e| e.local() == "object") {
        let read = || -> Result<Scenery, String> {
            let id = o.attr("id").ok_or("needs `id`")?.trim().to_owned();
            let kind = o.attr("kind").ok_or("needs `kind`")?.trim().to_owned();
            Ok(Scenery {
                id,
                kind,
                at: read_frame(o)?,
                size: [
                    number(o, "width", Some(0.0))?,
                    number(o, "depth", Some(0.0))?,
                    number(o, "height", Some(0.0))?,
                ],
            })
        };
        match read() {
            Ok(x) if out.iter().any(|y| y.id == x.id) => notes.push(Note::warning(
                None,
                format!("the scene repeats the id `{}`; the second is ignored", x.id),
            )),
            Ok(x) => out.push(x),
            Err(why) => notes.push(Note::warning(
                None,
                format!("a scene object {why} and is ignored"),
            )),
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// The layout the scenario gives `elements`: a track for each conveyor with a `stochcraft:path`,
/// a place for each block with a `stochcraft:place`, and the scenario's scenery. One that cannot
/// be read, a track on a block that is not a conveyor, or a place along a conveyor that has no
/// track, is noted and left out.
pub fn site_of(
    scenario: Option<&Scenario>,
    elements: &[ModelElement],
    notes: &mut Vec<Note>,
) -> Site {
    let mut site = Site::default();
    let Some(s) = scenario else {
        return site;
    };
    site.scene = s.settings.scene.clone();
    for e in elements {
        let id = e.id.as_str();
        if let Some(ext) = s.param(id, &format!("VendorExtension/{}", bpsim::PATH)) {
            if !matches!(e.kind, ElementKind::Conveyor { .. }) {
                notes.push(Note::warning(
                    Some(id),
                    "a path (stochcraft:path) belongs to a conveyor; it is ignored here",
                ));
            } else {
                match read_track(ext) {
                    Ok(t) => {
                        site.paths.insert(e.id.clone(), t);
                    }
                    Err(why) => notes.push(Note::warning(
                        Some(id),
                        format!("the path (stochcraft:path) {why} and is ignored"),
                    )),
                }
            }
        }
        if let Some(ext) = s.param(id, &format!("VendorExtension/{}", bpsim::PLACE)) {
            match read_place(ext) {
                Ok(p) => {
                    site.places.insert(e.id.clone(), p);
                }
                Err(why) => notes.push(Note::warning(
                    Some(id),
                    format!("the place (stochcraft:place) {why} and is ignored"),
                )),
            }
        }
    }
    let tracks: Vec<ElementId> = site.paths.keys().cloned().collect();
    site.places.retain(|id, p| match p {
        Place::Along { path, .. } if !tracks.contains(path) => {
            notes.push(Note::warning(
                Some(id.as_str()),
                format!("the place (stochcraft:place) is along `{path}`, which has no path; it is ignored"),
            ));
            false
        }
        _ => true,
    });
    site
}

/// Gives each conveyor with a track its path's length (D23) where its typed length agrees with
/// it to within [`LENGTH_STEP`]; a length that differs more stays, as an override.
pub fn lengths_from_paths(site: &Site, elements: &mut [ModelElement]) {
    for e in elements {
        if let (Some(t), ElementKind::Conveyor { length, .. }) =
            (site.paths.get(&e.id), &mut e.kind)
        {
            let drawn = t.length();
            if (drawn - *length).abs() <= LENGTH_STEP {
                *length = drawn;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic, clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_track_a_place_and_the_scene_read_back_as_written() {
        let track = Track {
            path: Path::new(
                Frame::new(1.5, -2.0, 0.25, 30.0),
                vec![
                    Segment::Line {
                        length: 10.0,
                        rise: 0.0,
                    },
                    Segment::Arc {
                        radius: 10.0 / std::f64::consts::PI,
                        angle_deg: 180.0,
                        rise: 0.5,
                    },
                    Segment::Helix {
                        radius: 1.5,
                        rise: 3.0,
                        turns: 2.0,
                    },
                ],
            ),
            width: 0.8,
        };
        assert_eq!(read_track(&path_element(&track)), Ok(track));
        for place in [
            Place::At(Frame::new(2.0, -5.2, 0.0, -90.0)),
            Place::Along {
                path: ElementId::unchecked("loop"),
                s: 3.5,
            },
        ] {
            assert_eq!(read_place(&place_element(&place)), Ok(place));
        }
        let scene = vec![
            Scenery {
                id: "wall".to_owned(),
                kind: "wall".to_owned(),
                at: Frame::new(0.0, 4.0, 0.0, 90.0),
                size: [0.2, 20.0, 3.0],
            },
            Scenery {
                id: "floor".to_owned(),
                kind: "floor".to_owned(),
                at: Frame::new(5.0, -2.0, 0.0, 0.0),
                size: [24.0, 16.0, 0.0],
            },
        ];
        let mut notes = Vec::new();
        let ext = scene_element(&scene).unwrap();
        let back = read_scene(&ext, &mut notes);
        assert!(notes.is_empty(), "{notes:?}");
        // Written and read in id order.
        assert_eq!(back, vec![scene[1].clone(), scene[0].clone()]);
        assert!(scene_element(&[]).is_none());
    }

    #[test]
    fn a_path_without_segments_or_with_a_bad_number_is_refused() {
        let empty = extension(
            bpsim::PATH,
            st("path").with_attr("x", "0").with_attr("y", "0"),
        );
        assert!(read_track(&empty).is_err());
        let bad = extension(
            bpsim::PATH,
            st("path")
                .with_attr("x", "0")
                .with_attr("y", "zero")
                .with_child(st("line").with_attr("length", "1")),
        );
        assert!(read_track(&bad).unwrap_err().contains("`y`"));
    }
}
