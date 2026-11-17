//! The guided first model (#149): "Show me" in Getting started rings the control the next step
//! uses (a block in the palette, a block's connect dot, Run, the Results tab) with a one-line
//! callout. The person makes each change; the guide follows the model to the next step, and Esc
//! or × stops it.

use egui::{Align2, Pos2, Rect, Stroke, StrokeKind};
use serde_json::Value;

use crate::help;
use crate::i18n::{t, tf};
use crate::theme;

/// What a step's callout points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A kind in the canvas's palette ("source", "server", "sink").
    Chip(&'static str),
    /// The connect dot of this block (its id), which shows while it is selected.
    Handle(String),
    /// The Run button in the header.
    Run,
    /// The Results tab.
    ResultsTab,
}

/// The next step's callout: the step (0-based), what it points at and what it says (in the
/// interface language).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub step: usize,
    pub target: Target,
    pub text: &'static str,
}

/// The guide's state, kept by the app.
#[derive(Default)]
pub struct Guide {
    /// The guide is showing.
    pub on: bool,
    /// The target shown last frame, to notice a new one.
    pub last: Option<Target>,
    /// The target changed this frame: its control is scrolled into view.
    pub fresh: bool,
    /// Where each control a step can point at was drawn this frame.
    pub targets: Targets,
}

/// Where the controls were drawn this frame (screen coordinates).
#[derive(Default)]
pub struct Targets {
    pub chips: Vec<(&'static str, Rect)>,
    /// A block's connect dot, by block id.
    pub handle: Option<(String, Rect)>,
    /// Each block's shape on the canvas, for when its dot isn't drawn.
    pub blocks: Vec<(String, Rect)>,
    pub run: Option<Rect>,
    pub results_tab: Option<Rect>,
}

impl Targets {
    fn rect(&self, target: &Target) -> Option<Rect> {
        match target {
            Target::Chip(kind) => self.chips.iter().find(|(k, _)| k == kind).map(|c| c.1),
            Target::Handle(id) => self
                .handle
                .as_ref()
                .filter(|(h, _)| h == id)
                .or_else(|| self.blocks.iter().find(|(b, _)| b == id))
                .map(|h| h.1),
            Target::Run => self.run,
            Target::ResultsTab => self.results_tab,
        }
    }
}

/// The next step's callout for `model` (its wire form), or `None` once every step is done.
/// Step 2 points at the task with a resource, whose queue gives the first run something to show.
pub fn hint(model: &Value, ran: bool, read: bool) -> Option<Hint> {
    let w = help::walk(model);
    let (step, target, text) = if w.arrivals.is_none() {
        (
            0,
            Target::Chip("source"),
            t("Drag Arrivals onto the canvas, or click it."),
        )
    } else if w.task.is_none() {
        if w.any_task {
            (
                1,
                Target::Handle(w.arrivals.unwrap_or_default()),
                t(
                    "Drag from the dot on Arrivals to the task to connect them, or choose the task under \u{201c}Add a flow to\u{2026}\u{201d} in the inspector.",
                ),
            )
        } else {
            (
                1,
                Target::Chip("server"),
                t("Now add a task: drag \u{201c}Task with a resource\u{201d} onto the canvas."),
            )
        }
    } else if !w.departures {
        if w.any_sink {
            (
                2,
                Target::Handle(w.task.unwrap_or_default()),
                t(
                    "Drag from the dot on the task to Departures, or choose Departures under \u{201c}Add a flow to\u{2026}\u{201d} in the inspector.",
                ),
            )
        } else {
            (
                2,
                Target::Chip("sink"),
                t("Add Departures, where entities leave: drag it onto the canvas."),
            )
        }
    } else if !ran {
        (3, Target::Run, t("Press Run to run the model."))
    } else if !read {
        (
            4,
            Target::ResultsTab,
            t("Open Results to read what the run found."),
        )
    } else {
        return None;
    };
    Some(Hint { step, target, text })
}

/// What the callout asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Stop,
}

/// Draws the ring around the hint's control and the callout beside it, above everything else;
/// with no hint (every step done), a last callout by the Results tab. With the control not
/// drawn this frame (another view is open), the callout sits at the bottom of the window.
pub fn show(
    ctx: &egui::Context,
    guide: &Guide,
    hint: Option<&Hint>,
    steps: usize,
) -> Option<Action> {
    let p = theme::palette(ctx);
    let target = hint.map_or(Target::ResultsTab, |h| h.target.clone());
    let rect = guide.targets.rect(&target);
    if let Some(r) = rect {
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("guide-ring"),
        ))
        .rect_stroke(
            r.expand(4.0),
            8.0,
            Stroke::new(2.5, p.accent),
            StrokeKind::Outside,
        );
    }
    let screen = ctx.content_rect();
    // Below the control, or above it near the bottom of the window.
    let (pos, pivot) = match rect {
        Some(r) if screen.bottom() - r.bottom() > 160.0 => {
            (Pos2::new(r.left(), r.bottom() + 12.0), Align2::LEFT_TOP)
        }
        Some(r) => (Pos2::new(r.left(), r.top() - 12.0), Align2::LEFT_BOTTOM),
        None => (
            Pos2::new(screen.center().x, screen.bottom() - 48.0),
            Align2::CENTER_BOTTOM,
        ),
    };
    let mut action = None;
    egui::Area::new(egui::Id::unique("guide-callout"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .pivot(pivot)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .stroke(Stroke::new(1.5, p.accent))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_max_width(300.0);
                    ui.horizontal(|ui| {
                        let title = match hint {
                            Some(h) => tf(
                                "Step {k} of {n}",
                                &[("k", &(h.step + 1).to_string()), ("n", &steps.to_string())],
                            ),
                            None => t("Done").to_owned(),
                        };
                        ui.label(theme::weak(ui, title));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(egui::Button::new("\u{00d7}").frame(false))
                                .on_hover_text(t("Stop the guide (Esc)"))
                                .clicked()
                            {
                                action = Some(Action::Stop);
                            }
                        });
                    });
                    // Once every step is done, what the run shows and where it is explained.
                    let text = hint.map_or_else(
                        || t("That is your first model. The first lines of Results say what the run found; Help \u{25b8} What the Results Mean explains each number."),
                        |h| h.text,
                    );
                    ui.add(egui::Label::new(text).wrap().selectable(false));
                });
        });
    action
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // A missing hint fails the test.
    use super::*;
    use serde_json::json;

    fn model(elements: Value, connections: Value) -> Value {
        json!({ "elements": elements, "connections": connections })
    }

    #[test]
    fn each_step_points_at_the_control_it_needs() {
        let empty = model(json!([]), json!([]));
        assert_eq!(
            hint(&empty, false, false).unwrap().target,
            Target::Chip("source")
        );
        let a = json!({"id": "a", "kind": "source"});
        let s = json!({"id": "s", "kind": "server"});
        let d = json!({"id": "d", "kind": "sink"});
        let one = model(json!([a]), json!([]));
        let h = hint(&one, false, false).unwrap();
        assert_eq!((h.step, h.target), (1, Target::Chip("server")));
        // A task not yet reached: connect Arrivals to it.
        let two = model(json!([a, s]), json!([]));
        assert_eq!(
            hint(&two, false, false).unwrap().target,
            Target::Handle("a".into())
        );
        let joined = model(json!([a, s]), json!([{"from": "a", "to": "s"}]));
        let h = hint(&joined, false, false).unwrap();
        assert_eq!((h.step, h.target), (2, Target::Chip("sink")));
        let three = model(json!([a, s, d]), json!([{"from": "a", "to": "s"}]));
        assert_eq!(
            hint(&three, false, false).unwrap().target,
            Target::Handle("s".into())
        );
        let whole = model(
            json!([a, s, d]),
            json!([{"from": "a", "to": "s"}, {"from": "s", "to": "d"}]),
        );
        assert_eq!(hint(&whole, false, false).unwrap().target, Target::Run);
        assert_eq!(
            hint(&whole, true, false).unwrap().target,
            Target::ResultsTab
        );
        assert_eq!(hint(&whole, true, true), None);
    }

    #[test]
    fn the_hints_agree_with_the_checklist() {
        // The guide's step is always Getting started's first unticked one.
        let a = json!({"id": "a", "kind": "source"});
        let s = json!({"id": "s", "kind": "delay"});
        let d = json!({"id": "d", "kind": "sink"});
        for (elements, connections) in [
            (json!([]), json!([])),
            (json!([a]), json!([])),
            (json!([a, s]), json!([])),
            (json!([a, s, d]), json!([{"from": "a", "to": "s"}])),
            // Departures straight from Arrivals is not through a task.
            (
                json!([a, s, d]),
                json!([{"from": "a", "to": "s"}, {"from": "a", "to": "d"}]),
            ),
        ] {
            let m = model(elements, connections);
            let next = help::steps(&m, false, false)
                .iter()
                .position(|(_, done)| !done);
            assert_eq!(hint(&m, false, false).map(|h| h.step), next);
        }
    }
}
