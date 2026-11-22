//! The document: opening, saving, exporting and replacing the model, running engine commands on it,
//! and undo and redo.

use serde_json::{Value, json};
use stochcraft_engine::{CommandError, ErrorCode};

use crate::i18n::{t, td, tf, tn, tnf};
use crate::{
    App, EXAMPLES, NewFile, PendingPng, RECENT_FILES, Replace, Request, canvas, change_label,
    charts, comparable, extension, file_name, help, layout_view, view3d,
};

/// A file being read in the background, from [`App::begin_open`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenTicket {
    /// The request's number: a newer request makes this one stale.
    serial: u64,
    /// The document shown when the read began (`doc_gen`).
    doc: u64,
    /// The engine's revision then: it differs once the model has been edited.
    revision: u64,
}

impl App {
    /// Opens a model file read from `path` and lists it under File ▸ Open Recent.
    pub fn open_path(&mut self, path: &str, bytes: &[u8]) {
        self.open_file(path, bytes);
        if self.engine.path() == Some(path) || self.engine.system_dynamics_path() == Some(path) {
            self.remember(path);
        }
    }

    fn remember(&mut self, path: &str) {
        if !self.platform.file_system {
            return;
        }
        self.recent.retain(|p| p != path);
        self.recent.insert(0, path.to_owned());
        self.recent.truncate(RECENT_FILES);
    }

    /// Whether closing now would lose changes.
    pub fn has_unsaved_changes(&self) -> bool {
        self.unsaved_changes()
    }

    /// Whether closing or replacing the document would lose changes: the engine's own answer,
    /// which covers a system dynamics model as well as a process model. The title, the header
    /// and the guards on closing all read this, so they agree.
    pub(crate) fn unsaved_changes(&self) -> bool {
        self.engine.is_dirty()
    }

    pub(crate) fn document_name(&self) -> String {
        if let Some(path) = self
            .engine
            .path()
            .or_else(|| self.engine.system_dynamics_path())
        {
            return file_name(path).to_owned();
        }
        let name = &self.engine.model().name;
        if name.is_empty() {
            t("Untitled").into()
        } else {
            name.clone()
        }
    }

    /// The labels of the change Undo and Redo would reverse or repeat.
    pub(crate) fn engine_history(&mut self) -> (Option<String>, Option<String>) {
        let h = self
            .engine
            .execute("history", json!({}))
            .unwrap_or_default();
        let last = |k: &str| {
            h[k].as_array()
                .and_then(|v| v.last())
                .and_then(Value::as_str)
                .map(|c| change_label(c).to_owned())
        };
        (last("undo"), last("redo"))
    }

    pub(crate) fn info(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = false;
        self.status_at = None;
    }

    /// Shows `message` as an error in the status bar (for the shell's failures). A save in
    /// progress carries on: only [`App::save_failed`] ends it.
    pub fn report_error(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = true;
        self.status_at = None;
    }

    /// The save in progress failed (the Save dialog could not be shown, or the file could not be
    /// written): shows `message` as an error, and drops what was to follow the save (quit, new,
    /// open) and the agreement to a new BPMN file, so neither carries over to a later save.
    pub fn save_failed(&mut self, message: impl Into<String>) {
        self.report_error(message);
        self.after_save = None;
        self.as_new = false;
        self.pick_for = None;
    }

    /// Runs an engine command, showing its error in the status bar if it fails, with the block
    /// of the first error its diagnostics place (a run that finds a problem only by running).
    /// Every change the interface makes to the model goes through here.
    pub fn execute(&mut self, command: &str, params: Value) -> Option<Value> {
        match self.engine.execute(command, params) {
            Ok(v) => Some(v),
            Err(e) => {
                self.command_failed(e);
                None
            }
        }
    }

    /// Shows a command's error in the status bar, which a click takes to the block it is on.
    pub(crate) fn command_failed(&mut self, e: CommandError) {
        self.report_error(e.message);
        self.status_at = e.details.as_ref().and_then(|d| self.error_block(d));
    }

    /// Opens a model file from its bytes (`name` is its path, or its name where there are no
    /// paths).
    pub fn open_file(&mut self, name: &str, bytes: &[u8]) {
        match self.engine.open_bytes(name, bytes, None) {
            Ok(opened) => {
                self.forget_view();
                // Warnings and errors from reading the file and checking the model; information
                // notes are left for the model view.
                let count = |k: &str| {
                    opened[k]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|n| n["severity"] != "info")
                        .count()
                };
                let n = count("notes") + count("diagnostics");
                let file = file_name(name);
                let message = if n > 0 {
                    tnf(
                        n as u64,
                        "Opened {file} ({n} warning)",
                        "Opened {file} ({n} warnings)",
                        &[("file", file)],
                    )
                } else {
                    tf("Opened {file}", &[("file", file)])
                };
                self.info(message);
            }
            Err(e) => self.report_error(e.message),
        }
    }

    /// What the window shows of a model that has been replaced: results, selection, the
    /// canvas's view, a watched run and half-typed fields. A run in progress is cancelled, so
    /// its results cannot turn up later over the new model. It is another document, so an
    /// answer asked for the last one (the Save dialog's) is not applied to it.
    pub(crate) fn forget_view(&mut self) {
        self.doc_gen += 1;
        // The Monte Carlo screens' state, and a Monte Carlo run in progress, which stops.
        self.mc = crate::montecarlo::Mc::default();
        // The system dynamics screens' state: its results are of the model that is gone.
        self.sd = crate::sd::Sd::default();
        if self.running.is_some() {
            self.stop_workers();
            let _ = self.engine.execute("run.cancel", json!({}));
            self.running = None;
            self.run_model = None;
        }
        self.clear_results();
        self.building = false;
        self.guide.on = false;
        self.help.read_results = false;
        self.selected = None;
        self.canvas = canvas::Canvas::default();
        self.view3d = view3d::View3d::default();
        self.layout_view = layout_view::LayoutView::default();
        self.watching = None;
        self.drafts.clear();
        self.show_results = false;
        self.show_analysis = false;
        self.show_capacity = false;
        self.show_3d = false;
    }

    /// Runs an engine command for an agent (the MCP server, through the desktop app's control
    /// channel), so the person at the window sees what it does: a model it opens or starts
    /// replaces the one shown, and the results of its runs are shown. Background runs are the
    /// window's own (Model ▸ Run), so `run.start`, `run.status` and `run.cancel` are refused:
    /// `run` runs at once.
    pub fn control(&mut self, command: &str, params: Value) -> Result<Value, CommandError> {
        if matches!(command, "run.start" | "run.status" | "run.cancel") {
            return Err(CommandError::new(
                stochcraft_engine::ErrorCode::Unsupported,
                format!("`{command}` is not available in the app: use `run`, which runs at once"),
            ));
        }
        let before = matches!(command, "run" | "mc.run").then(|| comparable(self.engine.model()));
        let result = self.engine.execute(command, params)?;
        match command {
            "file.open" | "model.new" | "model.set" | "sd.open" => {
                self.forget_view();
                if let Some(path) = result["path"].as_str() {
                    self.info(tf("Opened {file}", &[("file", file_name(path))]));
                }
            }
            "run" => {
                let model = before.unwrap_or_else(|| comparable(self.engine.model()));
                self.set_results(result.clone(), model);
                let n = result["replications"].as_u64().unwrap_or(0);
                self.info(tn(n, "Finished {n} run", "Finished {n} runs"));
            }
            "mc.run" => {
                let model = before.unwrap_or_else(|| comparable(self.engine.model()));
                self.set_results(result.clone(), model);
                let n = result["trials"].as_u64().unwrap_or(0);
                self.info(tn(n, "Finished {n} trial", "Finished {n} trials"));
            }
            // The results of a system dynamics run, shown on its Results tab.
            "sd.run" => self.sd_ran(result.clone()),
            _ => {}
        }
        Ok(result)
    }

    /// A shell that reads a file in the background (the browser's picker, a dropped file) calls
    /// this when it starts, and hands the ticket back to [`App::finish_open`] with what it read.
    /// The completion may come late or out of order, so the ticket says which model was open and
    /// which request was the newest when the read began.
    pub fn begin_open(&mut self) -> OpenTicket {
        self.open_serial += 1;
        OpenTicket {
            serial: self.open_serial,
            doc: self.doc_gen,
            revision: self.engine.revision(),
        }
    }

    /// A file read in the background arrived (`dropped`: it was dropped on the window rather
    /// than chosen in the picker).
    ///
    /// It is not opened if another file was asked for since, or the model was replaced
    /// meanwhile (a stale completion is dropped, and the status bar says so). A picked file
    /// opens at once only while the model is as it was when the picker was shown, since the
    /// unsaved changes were asked about before that; if it was edited since, the question is
    /// asked again, as for a dropped file, which is always asked about unsaved changes.
    pub fn finish_open(&mut self, ticket: OpenTicket, name: &str, bytes: Vec<u8>, dropped: bool) {
        if ticket.serial != self.open_serial || ticket.doc != self.doc_gen {
            self.info(tf(
                "Did not open {file}: a different model was opened in the meantime",
                &[("file", file_name(name))],
            ));
        } else if dropped || ticket.revision != self.engine.revision() {
            self.drop_file(name, bytes);
        } else {
            self.open_file(name, &bytes);
        }
    }

    /// Opens a file dropped on the window, first asking about unsaved changes.
    pub fn drop_file(&mut self, name: &str, bytes: Vec<u8>) {
        self.dropped = Some((name.to_owned(), bytes));
        self.replace(Replace::Dropped);
    }

    /// The shell saved an export as `path`.
    pub fn exported(&mut self, path: &str) {
        self.info(tf("Exported {file}", &[("file", file_name(path))]));
    }

    /// Exports the results table as CSV.
    pub fn export_csv(&mut self) {
        if self.is_sd() {
            if let Some(text) = self.sd_table_csv() {
                let name = format!("{}-results.csv", self.export_stem());
                self.requests.push(Request::Export {
                    name,
                    bytes: text.into_bytes(),
                });
            }
            return;
        }
        if let Some(r) = &self.results {
            let name = format!("{}-results.csv", self.export_stem());
            let text = if r["kind"] == "monte_carlo" {
                stochcraft_engine::mc::results_csv(r)
            } else {
                stochcraft_engine::wire::results_csv(r)
            };
            let bytes = text.into_bytes();
            self.requests.push(Request::Export { name, bytes });
        }
    }

    /// Exports the charts as a PNG image of the window's pixels (taken on the next frame).
    pub fn export_png(&mut self, ctx: &egui::Context) {
        let Some(area) = self.charts_rect else {
            self.report_error(t("there are no charts to export"));
            return;
        };
        // One capture at a time (a frame laid out twice must not ask twice).
        if self.pending_png.is_some() {
            return;
        }
        self.pending_png = Some(PendingPng {
            area,
            pixels_per_point: ctx.pixels_per_point(),
            results: self.results_serial,
            covered: self.charts_covered(ctx),
            spoiled: false,
        });
        let slot = std::sync::Arc::clone(&self.screenshot);
        let wake = ctx.clone();
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
            egui::ScreenshotCallback::new(move |image| {
                if let Ok(mut s) = slot.lock() {
                    *s = Some(image);
                }
                wake.request_repaint();
            }),
        ));
    }

    /// Turns a screenshot that has arrived into the charts' PNG export.
    pub(crate) fn take_screenshot(&mut self) {
        let Some(image) = self.screenshot.lock().ok().and_then(|mut s| s.take()) else {
            return;
        };
        if let Some(pending) = self.pending_png.take() {
            let png = if pending.spoiled {
                None
            } else {
                charts::png(&image, pending.area, pending.pixels_per_point)
            };
            match png {
                Some(bytes) => {
                    let name = format!("{}-charts.png", self.export_stem());
                    self.requests.push(Request::Export { name, bytes });
                }
                None => self.report_error(t("the charts could not be captured")),
            }
        }
    }

    /// Checks a charts export waiting for its screenshot against this frame, once it is laid
    /// out: if the charts are not where they were, at the same scale, with the same results, or
    /// a menu or dialog has opened over the window since, the export is spoiled, as the web's
    /// screenshot (of a frame after the one that asked) would not hold them.
    pub(crate) fn check_pending_png(&mut self, ctx: &egui::Context) {
        let covered = self.charts_covered(ctx);
        let (area, results) = (self.charts_rect, self.results_serial);
        let pixels_per_point = ctx.pixels_per_point();
        if let Some(pending) = &mut self.pending_png
            && (area != Some(pending.area)
                || pixels_per_point != pending.pixels_per_point
                || results != pending.results
                || (covered && !pending.covered))
        {
            pending.spoiled = true;
        }
    }

    /// Whether a menu, popup or dialog is open over the window.
    fn charts_covered(&self, ctx: &egui::Context) -> bool {
        self.modal_open() || self.about || egui::Popup::is_any_open(ctx)
    }

    /// The start of an export's file name: the model's file name without its extension.
    pub(crate) fn export_stem(&self) -> String {
        let name = self.document_name();
        name.rsplit_once('.')
            .map_or(name.clone(), |(stem, _)| stem.to_owned())
    }

    /// Saves to `path`, chosen by the user for Save As, in the format its extension names
    /// (`.bpmn`, `.xpdl`; otherwise the opened file's).
    ///
    /// If the model was replaced while the Save dialog was open (an example opened, a file
    /// dropped, an agent's command), `path` was chosen for the model that is gone: nothing is
    /// written, and what was to follow the save (quit, new, open) is dropped.
    pub fn save_to(&mut self, path: &str) {
        if self
            .pick_for
            .take()
            .is_some_and(|asked| asked != self.doc_gen)
        {
            self.new_file = None;
            self.save_failed(t(
                "The model changed while choosing where to save it, so it was not saved. Save again.",
            ));
            return;
        }
        let agreed = std::mem::take(&mut self.as_new);
        self.save_file(path.to_owned(), true, agreed);
    }

    /// Saves to `path` (written on the desktop, downloaded under its name in the browser): into
    /// the opened file when it is that file's format and can take the changes, else as a new
    /// BPMN file once the user has agreed (`agreed`: already, in the question that asked for
    /// `path`). `chosen`: the user chose `path` for Save As, so its extension says the format;
    /// otherwise it is the opened file's own.
    fn save_file(&mut self, path: String, chosen: bool, agreed: bool) {
        // A system dynamics model is written as XMILE, into the file it came from or a new one;
        // the BPMN and XPDL questions below are for process models. A chosen name is never
        // rewritten (the dialog confirmed that file and no other, so a rewritten name could
        // replace a file nobody agreed to replace): one that does not name an XMILE file is
        // refused.
        if self.is_sd() {
            if chosen && !matches!(extension(&path).as_str(), "xmile" | "stmx") {
                self.save_failed(tf(
                    "{file}: a system dynamics model is saved as XMILE; choose a name ending in .xmile",
                    &[("file", file_name(&path))],
                ));
                return;
            }
            // Into the opened file: a plain Save, or Save As naming that file.
            let into_opened = !chosen
                || self
                    .engine
                    .system_dynamics_path()
                    .is_some_and(|opened| same_path(opened, &path));
            if into_opened && self.platform.file_system && !self.engine.save_drops().is_empty() {
                // The opened file has more than the model keeps: ask before replacing it.
                let notes = self.engine.save_drops();
                let mut lost = notes
                    .iter()
                    .take(SD_LOST_SHOWN)
                    .cloned()
                    .collect::<Vec<_>>();
                if notes.len() > SD_LOST_SHOWN {
                    lost.push(tf(
                        "and {n} more",
                        &[("n", &(notes.len() - SD_LOST_SHOWN).to_string())],
                    ));
                }
                let reason = tf(
                    "Saving into {file} would leave out what StochCraft does not keep of it: {what}.",
                    &[("file", file_name(&path)), ("what", &lost.join("; "))],
                );
                self.new_file = Some(NewFile {
                    reason: Some(reason),
                    path: Some(path),
                    for_doc: self.doc_gen,
                });
                return;
            }
            self.write_sd(path);
            return;
        }
        let opened = self.engine.format();
        let named = match extension(&path).as_str() {
            "bpmn" => Some("bpmn"),
            "xpdl" => Some("xpdl"),
            _ => None,
        };
        let wanted = match (chosen, named, opened) {
            (true, Some(f), _) => f,
            (_, _, Some(f)) if !agreed => f,
            _ => "bpmn",
        };
        if wanted == "xpdl" && (agreed || opened != Some("xpdl")) {
            self.save_failed(tf(
                "{file}: StochCraft writes a new file only as BPMN; choose a name ending in .bpmn",
                &[("file", file_name(&path))],
            ));
            return;
        }
        if agreed || opened.is_some_and(|f| f != wanted) {
            if agreed {
                self.write_new(path);
            } else {
                // BPMN chosen for an XPDL file.
                self.new_file = Some(NewFile {
                    reason: None,
                    path: Some(path),
                    for_doc: self.doc_gen,
                });
            }
            return;
        }
        match self.engine.save_bytes(false) {
            Ok(bytes) => self.deliver(path, bytes),
            Err(e) if e.code == ErrorCode::Unsupported => {
                self.new_file = Some(NewFile {
                    reason: Some(e.message),
                    path: (chosen && wanted == "bpmn").then_some(path),
                    for_doc: self.doc_gen,
                });
            }
            Err(e) => self.save_failed(e.message),
        }
    }

    /// Writes (or downloads) the system dynamics model to `path` as XMILE.
    pub(crate) fn write_sd(&mut self, path: String) {
        match self.engine.save_bytes(false) {
            Ok(bytes) => self.deliver(path, bytes),
            Err(e) => self.save_failed(e.message),
        }
    }

    /// Writes (or downloads) the model to `path` as a new BPMN file.
    pub(crate) fn write_new(&mut self, path: String) {
        match self.engine.save_bytes(true) {
            Ok(bytes) => self.deliver(path, bytes),
            Err(e) => self.save_failed(e.message),
        }
    }

    fn deliver(&mut self, path: String, bytes: Vec<u8>) {
        self.requests.push(if self.platform.file_system {
            Request::Write { path, bytes }
        } else {
            Request::Download { name: path, bytes }
        });
    }

    /// The user closed the Save dialog without choosing a file: nothing was saved, so what was
    /// to follow the save (quit, new, open) is dropped rather than run after a later save.
    pub fn save_cancelled(&mut self) {
        self.after_save = None;
        self.as_new = false;
        self.pick_for = None;
    }

    /// The shell wrote `bytes` to `path`: the model is saved there.
    pub fn written(&mut self, path: &str, bytes: &[u8]) {
        match self.engine.saved_to(path, bytes) {
            Ok(_) => {
                self.info(tf("Saved {file}", &[("file", file_name(path))]));
                self.remember(path);
                if let Some(next) = self.after_save.take() {
                    self.replace_now(next);
                }
            }
            Err(e) => self.save_failed(e.message),
        }
    }

    /// The shell started the download of the model as `name` ([`Request::Download`]).
    ///
    /// A browser can't tell whether the user kept the file or cancelled it, so the model is not
    /// marked saved: the unsaved mark, the guard on closing the tab and the recovery copy all
    /// stay until a model replaces this one. What was to follow the save (New, Open…) goes ahead,
    /// as the user chose Download in the unsaved-changes question.
    pub fn downloaded(&mut self, name: &str) {
        self.info(tf("Downloaded {file}", &[("file", file_name(name))]));
        if let Some(next) = self.after_save.take() {
            self.replace_now(next);
        }
    }

    /// File ▸ Save: into the file the model came from, or as a new file.
    pub fn save(&mut self) {
        match (
            self.engine.path().map(str::to_owned),
            self.platform.file_system,
        ) {
            (Some(path), true) => self.save_file(path, false, false),
            (None, true) => self.save_as(),
            (path, false) => self.download(path),
        }
    }

    /// File ▸ Save As.
    pub fn save_as(&mut self) {
        if self.platform.file_system {
            let name = self.suggested_name();
            let xpdl = self.engine.format() == Some("xpdl");
            self.pick_save(name, xpdl);
        } else {
            self.download(None);
        }
    }

    /// Asks the shell where to save ([`Request::PickSave`]), for the model open now. While a
    /// Save dialog is open already, nothing more is asked: its answer saves the model it was
    /// opened for, or nothing if that model has been replaced.
    pub(crate) fn pick_save(&mut self, name: String, xpdl: bool) {
        if self.pick_for.is_some() {
            return;
        }
        self.pick_for = Some(self.doc_gen);
        self.requests.push(Request::PickSave { name, xpdl });
    }

    pub(crate) fn suggested_name(&self) -> String {
        if self.is_sd() {
            // The name it came from, or the model's: always an XMILE file's.
            let name = self.document_name();
            let name = if name.is_empty() {
                t("Untitled").to_owned()
            } else {
                name
            };
            return xmile_path(name);
        }
        match self.engine.path() {
            Some(path) => file_name(path).to_owned(),
            None => format!("{}.bpmn", self.document_name()),
        }
    }

    fn download(&mut self, name: Option<String>) {
        let name = name.unwrap_or_else(|| self.suggested_name());
        self.save_file(name, false, false);
    }

    /// Replaces or closes the model, first asking about unsaved changes.
    pub(crate) fn replace(&mut self, action: Replace) {
        if self.unsaved_changes() {
            self.confirm = Some(action);
        } else {
            self.replace_now(action);
        }
    }

    pub(crate) fn replace_now(&mut self, action: Replace) {
        match action {
            Replace::New => {
                if self.execute("model.new", json!({})).is_some() {
                    self.forget_view();
                    self.info(t("New model"));
                }
            }
            Replace::NewMonteCarlo => self.new_monte_carlo(),
            Replace::Build { getting_started } => {
                if self.execute("model.new", json!({})).is_some() {
                    self.forget_view();
                    self.building = true;
                    if getting_started {
                        self.show_help_at(help::Section::GettingStarted);
                        self.start_guide();
                    }
                }
            }
            Replace::Open => self.requests.push(Request::PickOpen),
            Replace::Example(i) => {
                if let Some(e) = EXAMPLES.get(i) {
                    self.open_file(e.file, e.bytes);
                }
            }
            Replace::Recent(i) => {
                if let Some(path) = self.recent.get(i).cloned() {
                    self.requests.push(Request::OpenPath { path });
                }
            }
            Replace::Dropped => {
                if let Some((name, bytes)) = self.dropped.take() {
                    self.open_file(&name, &bytes);
                }
            }
            Replace::Quit => {
                self.quitting = true;
                self.requests.push(Request::Quit);
            }
        }
    }

    /// The window is closing (the user closed it): returns whether it may close now. If not, the
    /// app asks about unsaved changes and requests [`Request::Quit`] once they are dealt with.
    pub fn close_requested(&mut self) -> bool {
        if self.unsaved_changes() {
            self.confirm = Some(Replace::Quit);
            false
        } else {
            true
        }
    }

    pub fn undo(&mut self) {
        if let Some(r) = self.execute("undo", json!({})) {
            let label = change_label(r["undone"].as_str().unwrap_or(""));
            self.info(tf("Undid {action}", &[("action", &td(label))]));
        }
    }

    pub fn redo(&mut self) {
        if let Some(r) = self.execute("redo", json!({})) {
            let label = change_label(r["redone"].as_str().unwrap_or(""));
            self.info(tf("Redid {action}", &[("action", &td(label))]));
        }
    }
}

/// How many of the file's not-kept items the question before replacing it names.
const SD_LOST_SHOWN: usize = 4;

/// Whether `a` and `b` name the same file, as the window can tell without the file system: the
/// same text, with either slash, and in any letter case on Windows, whose file names ignore it.
fn same_path(a: &str, b: &str) -> bool {
    let plain = |s: &str| {
        let s = s.replace('\\', "/");
        if cfg!(windows) { s.to_lowercase() } else { s }
    };
    plain(a) == plain(b)
}

/// `path` as the name Save As suggests for an XMILE file (never applied to a name the user
/// chose): one that ends in `.xmile` stays as it is; another extension of a model file (`.bpmn`,
/// `.xpdl`, `.xml`, `.stmx`) is replaced by it, and a name without one gets it added.
fn xmile_path(path: String) -> String {
    match extension(&path).as_str() {
        "xmile" => path,
        "bpmn" | "xpdl" | "xml" | "stmx" => match path.rsplit_once('.') {
            Some((stem, _)) => format!("{stem}.xmile"),
            None => format!("{path}.xmile"),
        },
        _ => format!("{path}.xmile"),
    }
}
