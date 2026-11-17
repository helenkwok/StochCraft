//! Help as a side panel beside the work (#149): search, Getting started as a checklist that
//! follows the model, a glossary of the window's words, the main keyboard shortcuts, and links
//! to the user guide and to report a problem. F1 or Help ▸ Help opens and closes it; it never
//! covers the model.

use serde_json::Value;

use crate::i18n::{t, tf};
use crate::theme;

/// Where the user guide is read: the repository's copy, which matches the released app.
pub const USER_GUIDE: &str = "https://github.com/helenkwok/StochCraft/blob/main/docs/user-guide.md";
/// The user guide's section on reading the results.
pub const RESULTS_GUIDE: &str =
    "https://github.com/helenkwok/StochCraft/blob/main/docs/user-guide.md#read-the-results";
/// Where problems are reported.
pub const REPORT: &str = "https://github.com/helenkwok/StochCraft/issues/new";

/// A section of the panel that a menu item can bring into view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    GettingStarted,
    Glossary,
    Shortcuts,
}

/// The Help panel's state.
#[derive(Default)]
pub struct Help {
    /// The panel is shown.
    pub open: bool,
    /// What is typed in its search field.
    pub search: String,
    /// The section to bring into view on the next frame.
    pub scroll_to: Option<Section>,
    /// The results of a run have been shown since the model was started, for the last step of
    /// Getting started.
    pub read_results: bool,
}

/// The glossary: each term of the window with what it means, in the window's words. The terms
/// are the labels the window uses; the definitions are StochCraft's own.
pub const GLOSSARY: &[(&str, &str)] = &[
    (
        "Block",
        "One step of the process on the canvas: where entities arrive, wait, are worked on, are routed or leave.",
    ),
    (
        "Entity",
        "What flows through the blocks: a part, a patient, a call, an order.",
    ),
    (
        "Arrivals",
        "The block where entities enter the model, with the time between one arrival and the next.",
    ),
    (
        "Task",
        "A step that takes time, with no limit on how many entities it works on at once.",
    ),
    (
        "Task with a resource",
        "A step done by a limited number of units, such as machines or clerks. Entities wait in its waiting room until a unit is free.",
    ),
    (
        "Task with shared resources",
        "A step that borrows units from a pool that several tasks share, such as a team of nurses.",
    ),
    (
        "Departures",
        "The block where entities leave the model; it counts them and their time in the system.",
    ),
    (
        "Decision",
        "Sends each entity down one of its ways out, by chance or by a condition.",
    ),
    (
        "Units",
        "How many entities a task with a resource can work on at the same time: its machines, clerks or beds.",
    ),
    (
        "Waiting room",
        "Where entities wait for a free unit (a queue). It can have a limit; an entity that finds it full is lost or waits upstream.",
    ),
    (
        "Service time",
        "How long a unit takes with one entity, drawn from a distribution each time.",
    ),
    (
        "Distribution",
        "How a time or a quantity varies from one entity to the next: its shape and its mean.",
    ),
    (
        "Run",
        "One pass of the model from start to end with its own random numbers. Textbooks call it a replication.",
    ),
    (
        "Experiment",
        "All the runs together, with the same settings: how many runs, the seed, the warm-up and the time measured.",
    ),
    (
        "Seed",
        "The starting point of the random numbers. The same model with the same seed always gives the same results.",
    ),
    (
        "Warm-up",
        "Time at the start of each run that is simulated but not measured, so an empty model filling up does not pull the results.",
    ),
    (
        "Utilisation",
        "The share of the time a task's units are busy, counted over the time they are on shift.",
    ),
    (
        "Mean wait",
        "The average time an entity spends in a waiting room before a unit takes it.",
    ),
    (
        "Mean queue length",
        "The average number of entities in a waiting room, over the time measured.",
    ),
    (
        "Time in system",
        "How long an entity spends from its arrival to its departure, waiting and being worked on.",
    ),
    (
        "Blocked",
        "A unit has finished but cannot pass the entity on, because the next waiting room is full.",
    ),
    ("Starved", "A unit is free but has no entity to work on."),
    (
        "95% confidence interval",
        "Written ± after a mean: how far the true mean could plausibly be from the one shown. More runs narrow it.",
    ),
    (
        "Settled",
        "A result known well enough to act on: to ± 5% of its mean, or ± 1 percentage point for a share.",
    ),
    (
        "Baseline",
        "Results kept to compare a later run with, to see what a change to the model did.",
    ),
    (
        "Could be chance",
        "A difference whose 95% interval takes in zero: the runs cannot tell it from random variation.",
    ),
];

/// The steps of Getting started, and whether each is done for `model` (its wire form):
/// arrivals, a task connected to something, departures that something connects to, a run, and
/// reading what it found.
pub fn steps(model: &Value, ran: bool, read: bool) -> [(&'static str, bool); 5] {
    let elements = model["elements"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let connections = model["connections"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let fed = |id: &Value| connections.iter().any(|c| &c["to"] == id);
    let kind_is = |e: &Value, kinds: &[&str]| kinds.contains(&e["kind"].as_str().unwrap_or(""));
    let arrivals = elements.iter().any(|e| kind_is(e, &["source"]));
    let task = elements
        .iter()
        .any(|e| kind_is(e, &["server", "delay", "shared_task"]) && fed(&e["id"]));
    let departures = elements
        .iter()
        .any(|e| kind_is(e, &["sink"]) && fed(&e["id"]));
    [
        ("Add Arrivals", arrivals),
        ("Add a task and connect Arrivals to it", task),
        ("Add Departures and connect the task to it", departures),
        ("Run the model: press F5", ran),
        ("Read what the run found", read),
    ]
}

/// What the panel asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Close,
    /// Open this address in the browser.
    Open(&'static str),
    /// Open the keyboard shortcuts dialog to change keys.
    ChangeKeys,
}

/// Whether `query` (already lower-cased) is found in `term` or `meaning`, in the interface
/// language.
fn matches(query: &str, term: &str, meaning: &str) -> bool {
    query.is_empty()
        || term.to_lowercase().contains(query)
        || meaning.to_lowercase().contains(query)
}

/// Draws the panel. `steps` is Getting started's checklist ([`steps`]); `keys` the shortcuts
/// to list, each with what it does.
pub fn show(
    ui: &mut egui::Ui,
    help: &mut Help,
    steps: &[(&'static str, bool)],
    keys: &[(String, &'static str)],
) -> Option<Action> {
    let p = theme::palette(ui.ctx());
    let mut action = None;
    ui.horizontal(|ui| {
        ui.heading(t("Help"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new("\u{00d7}").frame(false))
                .on_hover_text(t("Close Help (F1)"))
                .clicked()
            {
                action = Some(Action::Close);
            }
        });
    });
    ui.add_space(6.0);
    let search = ui.add(
        egui::TextEdit::singleline(&mut help.search)
            .hint_text(t("Search help and terms"))
            .desired_width(f32::INFINITY),
    );
    if search.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        help.search.clear();
    }
    let query = help.search.trim().to_lowercase();
    ui.add_space(8.0);
    let scroll_to = help.scroll_to.take();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let heading = |ui: &mut egui::Ui, text: &str, section: Section| {
                ui.add_space(6.0);
                let r = ui.label(theme::strong(text).size(15.0));
                if scroll_to == Some(section) {
                    r.scroll_to_me(Some(egui::Align::TOP));
                }
            };
            // Getting started, unless a search narrows the panel to terms.
            if query.is_empty() {
                let done = steps.iter().filter(|(_, d)| *d).count();
                ui.horizontal(|ui| {
                    heading(ui, t("Getting started"), Section::GettingStarted);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(theme::weak(
                            ui,
                            tf(
                                "{k} of {n} done",
                                &[("k", &done.to_string()), ("n", &steps.len().to_string())],
                            ),
                        ));
                    });
                });
                let next = steps.iter().position(|(_, d)| !d);
                for (i, (step, done)) in steps.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
                        if *done {
                            theme::paint_check_mark(ui.painter(), rect.center(), 5.5, &p);
                        } else if next == Some(i) {
                            ui.painter().circle_filled(rect.center(), 4.5, p.accent);
                        } else {
                            ui.painter().circle_stroke(
                                rect.center(),
                                4.5,
                                egui::Stroke::new(1.2, p.text_weak),
                            );
                        }
                        let text = egui::RichText::new(t(step));
                        let text = if next == Some(i) {
                            theme::strong(t(step))
                        } else {
                            text
                        };
                        ui.label(text);
                    });
                }
                ui.add_space(10.0);
            }
            heading(ui, t("Glossary"), Section::Glossary);
            let mut shown = 0;
            for (term, meaning) in GLOSSARY {
                let (term, meaning) = (t(term), t(meaning));
                if !matches(&query, term, meaning) {
                    continue;
                }
                shown += 1;
                ui.add_space(4.0);
                ui.label(theme::strong(term));
                ui.add(egui::Label::new(theme::weak(ui, meaning)).wrap());
            }
            if shown == 0 {
                ui.label(theme::weak(
                    ui,
                    tf(
                        "No term matches \u{201c}{text}\u{201d}.",
                        &[("text", help.search.trim())],
                    ),
                ));
            }
            if query.is_empty() {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    heading(ui, t("Keyboard shortcuts"), Section::Shortcuts);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.link(t("Change keys")).clicked() {
                            action = Some(Action::ChangeKeys);
                        }
                    });
                });
                egui::Grid::new(egui::Id::unique("help-keys"))
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for (key, what) in keys {
                            ui.label(egui::RichText::new(key.as_str()).monospace());
                            ui.label(t(what));
                            ui.end_row();
                        }
                    });
                ui.add_space(14.0);
                ui.label(theme::strong(t("More help")).size(15.0));
                ui.add_space(4.0);
                for (label, url) in [
                    (t("User guide"), USER_GUIDE),
                    (t("What the results mean"), RESULTS_GUIDE),
                    (t("Report a problem"), REPORT),
                ] {
                    if ui.link(label).on_hover_text(url).clicked() {
                        action = Some(Action::Open(url));
                    }
                }
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(theme::weak(ui, t("These open in your web browser."))).wrap(),
                );
            }
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn getting_started_follows_the_model() {
        let none = steps(&json!({ "elements": [], "connections": [] }), false, false);
        assert!(none.iter().all(|(_, d)| !d));
        let model = json!({
            "elements": [
                { "id": "a", "kind": "source" },
                { "id": "t", "kind": "server" },
                { "id": "d", "kind": "sink" }
            ],
            "connections": [{ "id": "c1", "from": "a", "to": "t" }]
        });
        let s = steps(&model, false, false);
        assert_eq!(
            s.map(|(_, d)| d),
            [true, true, false, false, false],
            "departures not connected yet"
        );
        let mut model = model;
        if let Some(c) = model["connections"].as_array_mut() {
            c.push(json!({ "id": "c2", "from": "t", "to": "d" }));
        }
        let s = steps(&model, true, true);
        assert!(s.iter().all(|(_, d)| *d));
    }

    #[test]
    fn search_finds_terms_and_meanings() {
        assert!(matches("", "Seed", "x"));
        assert!(matches("seed", "Seed", "x"));
        assert!(matches(
            "random",
            "Seed",
            "The starting point of the random numbers."
        ));
        assert!(!matches("queue", "Seed", "The starting point."));
    }

    #[test]
    fn the_glossary_has_no_term_twice() {
        let mut terms: Vec<&str> = GLOSSARY.iter().map(|(t, _)| *t).collect();
        terms.sort_unstable();
        let n = terms.len();
        terms.dedup();
        assert_eq!(terms.len(), n);
    }
}
