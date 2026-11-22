//! Autosave: a recovery copy of a model with unsaved changes, and the offer to restore it.
//!
//! The app decides what to keep and when there is nothing to keep; the shell decides where it
//! lives (the platform's local data folder on the desktop, the browser's storage on the web) and
//! asks every half minute or so ([`App::autosave`]). The copy is the model in the engine's own
//! JSON form, so a model built in the window is kept as well as one opened from a file; restored,
//! it is an untitled model with unsaved changes, and Save asks where it goes.
//!
//! Every running session (a window, a tab) keeps its own copy in a slot of its own, so two
//! sessions never replace or remove each other's. At start the shell lists the slots left by
//! sessions that are gone and hands each to [`App::offer_recovery`]; a slot the app is finished
//! with (recovered and kept by this session, discarded, or damaged) comes back from
//! [`App::take_dropped_copies`] for the shell to remove. A shell that can't write or remove
//! something says so ([`App::autosave_write_failed`], [`App::autosave_clear_failed`]), so the
//! app asks again at the next tick instead of believing the copy is kept.

use serde_json::{Value, json};

use crate::i18n::{t, tf};
use crate::{App, theme};

/// What the shell does with this session's recovery copy.
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
    /// A recovery copy is on offer and the question is showing. While the model has unsaved
    /// changes (one copy was recovered already) the other offers wait, so recovering another
    /// does not replace what was just recovered.
    pub(crate) fn recovery_waiting(&self) -> bool {
        !self.recovery.is_empty() && !self.unsaved_changes()
    }

    /// The recovery copy of this session to keep now: the model while it has unsaved changes
    /// (once per change), [`Autosave::Clear`] once it has none or the user chose to discard it.
    ///
    /// The shell calls [`App::autosave_write_failed`] or [`App::autosave_clear_failed`] if it
    /// could not do what it was told, then [`App::take_dropped_copies`].
    pub fn autosave(&mut self) -> Autosave {
        self.recovery_released.clear();
        if self.recovery_waiting() {
            // Waiting for the user's answer: leave the copies alone.
            return Autosave::Keep;
        }
        // No recovery copy of a system dynamics model: the copy is the process model's JSON, and
        // recovery copies are process models only for now. Its unsaved changes are still asked
        // about on closing.
        if !self.unsaved_changes() || self.is_sd() || self.quitting {
            // The recovered model is saved, or was given up: its old copy is not needed.
            self.recovery_drops.append(&mut self.recovery_held);
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
        // This session's own copy takes over from the one it recovered, once it is written.
        self.recovery_released = std::mem::take(&mut self.recovery_held);
        self.recovery_drops
            .extend(self.recovery_released.iter().cloned());
        let copy = json!({
            "stochcraft_recovery": 1,
            "name": self.document_name(),
            "model": stochcraft_engine::wire::model(self.engine.model()),
        });
        Autosave::Write(copy.to_string())
    }

    /// The shell could not write the copy [`App::autosave`] handed out: it is handed out again
    /// at the next call (an unchanged model included), and the copies this one was to replace
    /// stay.
    pub fn autosave_write_failed(&mut self) {
        // An older copy may still be there: it goes if nothing is unsaved by the next call.
        self.autosaved = None;
        self.autosave_clear_pending = true;
        let back = std::mem::take(&mut self.recovery_released);
        self.recovery_drops.retain(|slot| !back.contains(slot));
        self.recovery_held.extend(back);
    }

    /// The shell could not remove the copy [`App::autosave`] told it to: it is told again at the
    /// next call.
    pub fn autosave_clear_failed(&mut self) {
        self.autosave_clear_pending = true;
    }

    /// The slots of other sessions' recovery copies the shell can remove now. Each is returned
    /// once.
    pub fn take_dropped_copies(&mut self) -> Vec<String> {
        std::mem::take(&mut self.recovery_drops)
    }

    /// A recovery copy found at start in `slot` (the shell's name for where it was kept, left
    /// by a session that is gone): the app offers to restore it.
    pub fn offer_recovery(&mut self, slot: &str, copy: &str) {
        match serde_json::from_str::<Value>(copy) {
            Ok(v) if v["stochcraft_recovery"] == 1 && v["model"].is_object() => {
                self.recovery.push_back((slot.to_owned(), v));
            }
            // Not one of ours (or damaged): nothing to offer, and it goes.
            _ => self.recovery_drops.push(slot.to_owned()),
        }
    }

    /// Restores the recovery copy kept in `slot` as an untitled model with unsaved changes.
    fn recover(&mut self, slot: String, copy: Value) {
        let name = copy["name"].as_str().unwrap_or("").to_owned();
        if self.execute("model.new", json!({})).is_none() {
            return;
        }
        // Another document from here (even if the copy can't be set): a Save dialog's answer
        // asked for the last one is not applied to it.
        self.doc_gen += 1;
        if self
            .execute("model.set", json!({ "model": copy["model"] }))
            .is_some()
        {
            self.clear_results();
            self.selected = None;
            self.show_results = false;
            self.show_analysis = false;
            self.show_capacity = false;
            self.show_3d = false;
            // This session keeps the model now: its own copy is written at the next call of
            // `autosave` (whatever the last one was), and then the old copy goes. If the model
            // is saved first, `autosave` lets the old copy go then.
            // A copy of the model shown before stays until then, and goes if nothing is
            // unsaved by then.
            if self.autosaved.take().is_some() {
                self.autosave_clear_pending = true;
            }
            self.recovery_held.push(slot);
            self.info(tf(
                "Recovered {name}; save it to keep it",
                &[("name", &name)],
            ));
        }
    }

    pub(crate) fn recovery_dialog(&mut self, ctx: &egui::Context) {
        if !self.recovery_waiting() {
            return;
        }
        let Some((slot, copy)) = self.recovery.front().cloned() else {
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
                self.recovery.pop_front();
                self.recover(slot, copy);
            }
            Some(false) => {
                self.recovery.pop_front();
                self.recovery_drops.push(slot);
                self.info(t("Discarded the recovered changes"));
            }
            None => {}
        }
    }
}
