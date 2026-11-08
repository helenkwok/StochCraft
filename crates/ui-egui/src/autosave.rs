//! Autosave: a recovery copy of a model with unsaved changes, and the offer to restore it.
//!
//! The app decides what to keep and when there is nothing to keep; the shell decides where it
//! lives (the platform's local data folder on the desktop, the browser's storage on the web) and
//! asks every half minute or so ([`App::autosave`]). The copy is the model in the engine's own
//! JSON form, so a model built in the window is kept as well as one opened from a file; restored,
//! it is an untitled model with unsaved changes, and Save asks where it goes.

use serde_json::{Value, json};

use crate::i18n::{t, tf};
use crate::{App, theme};

/// What the shell does with the recovery copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Autosave {
    /// Keep this copy (replacing the last).
    Write(String),
    /// Nothing is unsaved any more: remove the copy.
    Clear,
    /// Nothing new since the last copy.
    Keep,
}

impl App {
    /// The recovery copy to keep now: the model while it has unsaved changes (once per change),
    /// [`Autosave::Clear`] once it has none or the user chose to discard it.
    pub fn autosave(&mut self) -> Autosave {
        if self.recovery.is_some() {
            // Waiting for the user's answer: leave the copy alone.
            return Autosave::Keep;
        }
        if !self.engine.is_dirty() || self.quitting {
            if self.autosaved.take().is_some() || self.autosave_clear_pending {
                self.autosave_clear_pending = false;
                return Autosave::Clear;
            }
            return Autosave::Keep;
        }
        let digest = stochcraft_engine::model_digest(self.engine.model());
        if self.autosaved.as_deref() == Some(digest.as_str()) {
            return Autosave::Keep;
        }
        self.autosaved = Some(digest);
        let copy = json!({
            "stochcraft_recovery": 1,
            "name": self.document_name(),
            "model": stochcraft_engine::wire::model(self.engine.model()),
        });
        Autosave::Write(copy.to_string())
    }

    /// A recovery copy found at start: the app offers to restore it.
    pub fn offer_recovery(&mut self, copy: &str) {
        match serde_json::from_str::<Value>(copy) {
            Ok(v) if v["stochcraft_recovery"] == 1 && v["model"].is_object() => {
                self.recovery = Some(v);
            }
            // Not one of ours (or damaged): nothing to offer, and it goes.
            _ => self.autosave_clear_pending = true,
        }
    }

    /// Restores the recovery copy as an untitled model with unsaved changes.
    fn recover(&mut self, copy: Value) {
        let name = copy["name"].as_str().unwrap_or("").to_owned();
        if self.execute("model.new", json!({})).is_none() {
            return;
        }
        if self
            .execute("model.set", json!({ "model": copy["model"] }))
            .is_some()
        {
            self.results = None;
            self.selected = None;
            self.show_results = false;
            self.show_analysis = false;
            self.info(tf(
                "Recovered {name}; save it to keep it",
                &[("name", &name)],
            ));
        }
    }

    pub(crate) fn recovery_dialog(&mut self, ctx: &egui::Context) {
        let Some(copy) = self.recovery.clone() else {
            return;
        };
        let name = copy["name"].as_str().unwrap_or(t("a model")).to_owned();
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("recovery")).show(ctx, |ui| {
            ui.set_max_width(400.0);
            ui.heading(t("Recover unsaved changes?"));
            ui.add_space(4.0);
            ui.label(theme::weak(
                ui,
                tf(
                    "StochCraft closed with unsaved changes to {name}. Recover them as an \
                     untitled model, or discard them.",
                    &[("name", &name)],
                ),
            ));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary(ui, t("Recover"))).clicked() {
                    choice = Some(true);
                }
                if ui.add(theme::secondary(ui, t("Discard"))).clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(true) => {
                self.recovery = None;
                self.recover(copy);
            }
            Some(false) => {
                self.recovery = None;
                self.autosave_clear_pending = true;
                self.info(t("Discarded the recovered changes"));
            }
            None => {}
        }
    }
}
