//! The opened file's scenarios: a choice of which one to open, shown where the experiment is
//! (the model page and the sidebar) when the file has more than one. Choosing another reopens
//! the file in it, as File ▸ Open would, so its results go with the model they were of; unsaved
//! changes are asked about first, beside the choice.

use serde_json::Value;

use crate::i18n::{t, tf};
use crate::{App, Replace, theme};

/// One of the opened file's scenarios.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Scenario {
    pub id: String,
    /// Its name, or its id when it has none.
    pub label: String,
    pub description: Option<String>,
}

/// A scenario chosen while the model had unsaved changes, waiting on the question beside the
/// choice it was made in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ask {
    pub id: String,
    /// The choice it was made in ("model" or "sidebar"), where the question is shown.
    pub place: &'static str,
    /// The document it was asked for (`doc_gen`): an answer is not applied to another one.
    pub for_doc: u64,
}

/// An answer to the question before unsaved changes are dropped.
#[derive(Debug, Clone, Copy)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

/// The scenarios of `listing` (`file.scenarios`), and the id of the open one.
fn scenarios_of(listing: &Value) -> (Vec<Scenario>, Option<String>) {
    let list = listing["scenarios"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s["id"].as_str()?.to_owned();
            let label = s["name"]
                .as_str()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(&id)
                .to_owned();
            Some(Scenario {
                id,
                label,
                description: s["description"]
                    .as_str()
                    .filter(|d| !d.trim().is_empty())
                    .map(str::to_owned),
            })
        })
        .collect();
    (list, listing["open"].as_str().map(str::to_owned))
}

impl App {
    /// The opened file's scenarios and the open one's id: none without a file.
    pub(crate) fn scenarios(&self) -> (Vec<Scenario>, Option<String>) {
        scenarios_of(&self.engine.scenarios())
    }

    /// The name of the open scenario, when the file has more than one to choose from.
    pub(crate) fn open_scenario_label(&self) -> Option<String> {
        let (list, open) = self.scenarios();
        if list.len() < 2 {
            return None;
        }
        let open = open?;
        list.into_iter().find(|s| s.id == open).map(|s| s.label)
    }

    /// The Scenario choice, drawn in `place` ("model": a row of the model page's experiment;
    /// "sidebar": the sidebar's experiment summary), with the question about unsaved changes
    /// under it when a choice made here is waiting on one. Nothing for a file with one
    /// scenario or none.
    pub(crate) fn scenario_choice(&mut self, ui: &mut egui::Ui, place: &'static str) {
        let (list, open) = self.scenarios();
        if list.len() < 2 {
            return;
        }
        let current = list.iter().find(|s| Some(&s.id) == open.as_ref());
        let shown = current.map_or_else(String::new, |s| s.label.clone());
        let mut picked = None;
        let mut combo = |ui: &mut egui::Ui, width: f32| {
            craft_ui::ComboBox::from_id_salt(("scenario", place))
                .selected_text(shown.as_str())
                .width(width)
                .show_ui(ui, |ui| {
                    for s in &list {
                        let r = ui.selectable_label(Some(&s.id) == open.as_ref(), &s.label);
                        let r = match &s.description {
                            Some(d) => r.on_hover_text(d),
                            None => r,
                        };
                        if r.clicked() && Some(&s.id) != open.as_ref() {
                            picked = Some(s.id.clone());
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Scenario"))
                });
        };
        if place == "model" {
            ui.horizontal(|ui| {
                crate::decide::row_label(ui, t("Scenario"));
                combo(ui, 240.0);
            });
            if let Some(d) = current.and_then(|s| s.description.as_deref()) {
                ui.add(egui::Label::new(theme::weak(ui, d)).wrap());
            }
            self.scenario_question(ui, place, &list);
        } else {
            // The sidebar's summary is laid out from the bottom: the question goes first, to
            // show under the choice.
            self.scenario_question(ui, place, &list);
            ui.horizontal(|ui| {
                ui.label(theme::weak(ui, t("Scenario")));
                let width = ui.available_width().max(80.0);
                combo(ui, width);
            });
        }
        if let Some(id) = picked {
            self.choose_scenario(id, place);
        }
    }

    /// Opens scenario `id` of the file, first asking (beside the choice in `place`) about
    /// unsaved changes, which opening it drops.
    fn choose_scenario(&mut self, id: String, place: &'static str) {
        if self.unsaved_changes() {
            self.scenario_ask = Some(Ask {
                id,
                place,
                for_doc: self.doc_gen,
            });
        } else {
            self.scenario_ask = None;
            self.open_other_scenario(&id);
        }
    }

    /// The question about unsaved changes before another scenario opens, under the choice it
    /// was asked from, in the warning colour: save first, drop them, or keep the scenario that is
    /// open.
    fn scenario_question(&mut self, ui: &mut egui::Ui, place: &'static str, list: &[Scenario]) {
        let Some(ask) = self.scenario_ask.clone() else {
            return;
        };
        // Another document since (or nothing left to lose): the question is not for it.
        if ask.for_doc != self.doc_gen || !self.unsaved_changes() {
            self.scenario_ask = None;
            return;
        }
        if ask.place != place {
            return;
        }
        let label = list
            .iter()
            .find(|s| s.id == ask.id)
            .map_or(ask.id.as_str(), |s| s.label.as_str());
        let question = tf(
            "Opening {scenario} drops the unsaved changes to {name}. Save them first?",
            &[("scenario", label), ("name", &self.document_name())],
        );
        let warning = theme::palette(ui.ctx()).warning;
        let question = egui::Label::new(egui::RichText::new(question).color(warning)).wrap();
        if place == "model" {
            ui.add(question);
            ui.horizontal_wrapped(|ui| {
                for answer in [Answer::Save, Answer::Discard, Answer::Cancel] {
                    self.scenario_answer(ui, answer, &ask.id);
                }
            });
        } else {
            // From the bottom up, one button a row so none can wrap over the summary below:
            // the question, then Save, Don't Save and Cancel under it.
            for answer in [Answer::Cancel, Answer::Discard, Answer::Save] {
                self.scenario_answer(ui, answer, &ask.id);
            }
            ui.add(question);
        }
    }

    /// One of the question's answers: save (or download) and then open scenario `id`, open it
    /// without saving, or cancel.
    fn scenario_answer(&mut self, ui: &mut egui::Ui, answer: Answer, id: &str) {
        match answer {
            Answer::Save => {
                let save = if self.platform.file_system {
                    t("Save")
                } else {
                    t("Download")
                };
                if ui.add(theme::primary(ui, save)).clicked() {
                    self.scenario_ask = None;
                    self.scenario_next = Some(id.to_owned());
                    self.after_save = Some(Replace::Scenario);
                    self.save();
                }
            }
            Answer::Discard => {
                if ui.add(theme::secondary(ui, t("Don't Save"))).clicked() {
                    self.scenario_ask = None;
                    self.open_other_scenario(id);
                }
            }
            Answer::Cancel => {
                if ui.add(theme::secondary(ui, t("Cancel"))).clicked() {
                    self.scenario_ask = None;
                }
            }
        }
    }

    /// Reopens the file in scenario `id`, dropping any unsaved changes (they were asked about):
    /// a new document, as opening the file is, so the results, selection and history of the
    /// last one go. As when the app opens a file, nothing beside it is read.
    pub(crate) fn open_other_scenario(&mut self, id: &str) {
        let name = self.engine.path().unwrap_or_default().to_owned();
        match self.engine.reopen_scenario(id, true, &mut |_| {
            Err("the app opens a model from its file alone".to_owned())
        }) {
            Ok(opened) => self.opened(&name, &opened),
            Err(e) => self.command_failed(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_scenario_without_a_name_is_shown_by_its_id() {
        let (list, open) = scenarios_of(&json!({
            "open": "b",
            "scenarios": [
                { "id": "a", "name": "Base", "description": "The first", "inherits": null, "open": false },
                { "id": "b", "name": null, "description": " ", "inherits": "a", "open": true },
            ],
        }));
        assert_eq!(open.as_deref(), Some("b"));
        assert_eq!(list[0].label, "Base");
        assert_eq!(list[0].description.as_deref(), Some("The first"));
        assert_eq!(list[1].label, "b");
        assert_eq!(list[1].description, None);
    }
}
