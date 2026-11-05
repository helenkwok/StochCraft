//! StochCraft's interface on egui, shared by the desktop app (`apps/stochcraft`) and the browser
//! app (`apps/stochcraft-web`).
//!
//! [`App`] draws the window: menus, the model outline, the results of the last run and a status
//! bar. Every change it makes to the model goes through the engine's commands (ROADMAP principle
//! 2), so undo, redo and the journal work as they do from the command line.
//!
//! What differs between platforms stays in the shells. The app never touches the file system or
//! shows a native dialog: it hands the shell a [`Request`] (pick a file to open, pick where to
//! save, write these bytes, offer them as a download) through [`App::take_requests`], and the shell
//! answers by calling [`App::open_file`], [`App::save_to`], [`App::written`] or
//! [`App::report_error`].

mod charts;
mod editor;
pub mod form;
mod results;
pub mod theme;

use egui::{Key, KeyboardShortcut, Modifiers};
use serde_json::{Value, json};
use stochcraft_engine::Engine;

/// The models offered under File ▸ Open Example, built in so the browser app has something to
/// open: the golden models of `stochcraft-testkit`.
pub const EXAMPLES: &[Example] = &[
    Example {
        title: "M/M/1 queue",
        about: "One clerk, random arrivals: the textbook queue, checked against theory.",
        file: "mm1.bpmn",
        bytes: include_bytes!("../../../tests/models/mm1.bpmn"),
    },
    Example {
        title: "Tandem line",
        about: "Two stations in series, read from an XPDL file.",
        file: "tandem.xpdl",
        bytes: include_bytes!("../../../tests/models/tandem.xpdl"),
    },
    Example {
        title: "Every distribution",
        about: "One task per BPSim distribution, to compare their spread.",
        file: "distributions.bpmn",
        bytes: include_bytes!("../../../tests/models/distributions.bpmn"),
    },
];

/// A built-in example model.
pub struct Example {
    pub title: &'static str,
    /// One line on what the example shows.
    pub about: &'static str,
    pub file: &'static str,
    pub bytes: &'static [u8],
}

/// Something only the shell can do, asked for by the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Ask for a model file; answer with [`App::open_file`] (or nothing, if the user cancels).
    PickOpen,
    /// Ask where to save, suggesting `name`; answer with [`App::save_to`].
    PickSave { name: String },
    /// Write `bytes` to `path`; answer with [`App::written`] or [`App::report_error`].
    Write { path: String, bytes: Vec<u8> },
    /// Offer `bytes` as a download named `name` (where there is no file system); answer with
    /// [`App::written`].
    Download { name: String, bytes: Vec<u8> },
    /// Save an exported file (results as CSV, charts as PNG) named `name`: ask where (or
    /// download it); answer with [`App::exported`] or [`App::report_error`].
    Export { name: String, bytes: Vec<u8> },
    /// Close the window: the user confirmed, or there was nothing unsaved.
    Quit,
}

/// What the platform offers the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    /// Files can be written to a path (desktop). Without it, saving downloads the file.
    pub file_system: bool,
    /// The app can close its own window (desktop; a browser tab closes itself).
    pub can_quit: bool,
}

impl Platform {
    pub const DESKTOP: Self = Self {
        file_system: true,
        can_quit: true,
    };
    pub const BROWSER: Self = Self {
        file_system: false,
        can_quit: false,
    };
}

/// An action that replaces or closes the model, held while the user decides what to do with
/// unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replace {
    New,
    Open,
    Example(usize),
    /// A file dropped on the window (held in `App::dropped`).
    Dropped,
    Quit,
}

/// A run in the background: its last reported status.
struct Running {
    done: u64,
    total: u64,
}

/// The keyboard shortcuts, shown in the menus. Cmd is Ctrl outside macOS.
mod keys {
    use super::{Key, KeyboardShortcut, Modifiers};
    pub const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
    pub const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
    pub const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
    pub const SAVE_AS: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
    pub const UNDO: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
    pub const REDO: KeyboardShortcut =
        KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    pub const RUN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
    pub const QUIT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);
}

/// The application: one model document and what the window shows of it.
pub struct App {
    engine: Engine,
    platform: Platform,
    /// Results of the last finished run, until the model changes.
    results: Option<Value>,
    /// The model digest the results belong to.
    results_digest: Option<String>,
    running: Option<Running>,
    /// The last message for the status bar: what happened, or what went wrong.
    status: String,
    /// The status is an error.
    status_error: bool,
    /// Waiting for the user to save or discard changes before this.
    confirm: Option<Replace>,
    /// Do this once the save in progress has been written.
    after_save: Option<Replace>,
    about: bool,
    /// A file dropped on the window, waiting on the unsaved-changes question.
    dropped: Option<(String, Vec<u8>)>,
    requests: Vec<Request>,
    /// The title last shown, so the shell sets it only when it changes.
    title: String,
    /// The fonts and theme are installed (on the first frame).
    styled: bool,
    /// The block the inspector shows; none shows the model overview.
    selected: Option<String>,
    /// Text being typed into form fields, until it commits.
    drafts: std::collections::HashMap<String, String>,
    /// The connections table's choices, before Connect.
    connect_from: Option<String>,
    connect_to: Option<String>,
    /// The work area shows the results rather than the model.
    show_results: bool,
    /// Where the charts were drawn last frame, for exporting them.
    charts_rect: Option<egui::Rect>,
    /// A screenshot asked for to export the charts, filled in by the renderer.
    screenshot: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<egui::ColorImage>>>>,
    /// The charts' area and scale when the screenshot was asked for.
    pending_png: Option<(egui::Rect, f32)>,
}

impl App {
    /// The app with a new, empty model.
    pub fn new(platform: Platform) -> Self {
        Self {
            engine: Engine::new(),
            platform,
            results: None,
            results_digest: None,
            running: None,
            status: String::new(),
            status_error: false,
            confirm: None,
            after_save: None,
            about: false,
            dropped: None,
            requests: Vec::new(),
            title: String::new(),
            styled: false,
            selected: None,
            drafts: std::collections::HashMap::new(),
            connect_from: None,
            connect_to: None,
            show_results: false,
            charts_rect: None,
            screenshot: std::sync::Arc::default(),
            pending_png: None,
        }
    }

    /// The engine, for shells and tests that read the model.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// A run is in progress.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Requests made since the last call, in order.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// The status-bar message.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Results of the last finished run, if the model has not changed since.
    pub fn results(&self) -> Option<&Value> {
        self.results.as_ref()
    }

    /// The window title: the file's name, marked when there are unsaved changes.
    pub fn title(&self) -> String {
        let name = self.document_name();
        let dirty = if self.engine.is_dirty() { " •" } else { "" };
        format!("{name}{dirty} — StochCraft")
    }

    /// The title, when it changed since the last call: for the shell to show.
    pub fn title_if_changed(&mut self) -> Option<String> {
        let title = self.title();
        (title != self.title).then(|| {
            self.title.clone_from(&title);
            title
        })
    }

    /// Whether closing now would lose changes.
    pub fn has_unsaved_changes(&self) -> bool {
        self.engine.is_dirty()
    }

    fn document_name(&self) -> String {
        if let Some(path) = self.engine.path() {
            return file_name(path).to_owned();
        }
        let name = &self.engine.model().name;
        if name.is_empty() {
            "Untitled".into()
        } else {
            name.clone()
        }
    }

    /// The labels of the change Undo and Redo would reverse or repeat.
    fn engine_history(&mut self) -> (Option<String>, Option<String>) {
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

    fn info(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = false;
    }

    /// Shows `message` as an error in the status bar (for the shell's failures).
    pub fn report_error(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_error = true;
        self.after_save = None;
    }

    /// Runs an engine command, showing its error in the status bar if it fails. Every change the
    /// interface makes to the model goes through here.
    pub fn execute(&mut self, command: &str, params: Value) -> Option<Value> {
        match self.engine.execute(command, params) {
            Ok(v) => Some(v),
            Err(e) => {
                self.report_error(e.message);
                None
            }
        }
    }

    /// Opens a model file from its bytes (`name` is its path, or its name where there are no
    /// paths).
    pub fn open_file(&mut self, name: &str, bytes: &[u8]) {
        match self.engine.open_bytes(name, bytes, None) {
            Ok(opened) => {
                self.results = None;
                self.selected = None;
                self.drafts.clear();
                self.show_results = false;
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
                let mut message = format!("Opened {}", file_name(name));
                if n > 0 {
                    message += &format!(" ({n} warning{})", if n == 1 { "" } else { "s" });
                }
                self.info(message);
            }
            Err(e) => self.report_error(e.message),
        }
    }

    /// Opens a file dropped on the window, first asking about unsaved changes.
    pub fn drop_file(&mut self, name: &str, bytes: Vec<u8>) {
        self.dropped = Some((name.to_owned(), bytes));
        self.replace(Replace::Dropped);
    }

    /// The shell saved an export as `path`.
    pub fn exported(&mut self, path: &str) {
        self.info(format!("Exported {}", file_name(path)));
    }

    /// Exports the results table as CSV.
    pub fn export_csv(&mut self) {
        if let Some(r) = &self.results {
            let name = format!("{}-results.csv", self.export_stem());
            let bytes = charts::csv(r).into_bytes();
            self.requests.push(Request::Export { name, bytes });
        }
    }

    /// Exports the charts as a PNG image of the window's pixels (taken on the next frame).
    pub fn export_png(&mut self, ctx: &egui::Context) {
        let Some(area) = self.charts_rect else {
            self.report_error("there are no charts to export");
            return;
        };
        // One capture at a time (a frame laid out twice must not ask twice).
        if self.pending_png.is_some() {
            return;
        }
        self.pending_png = Some((area, ctx.pixels_per_point()));
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
    fn take_screenshot(&mut self) {
        let Some(image) = self.screenshot.lock().ok().and_then(|mut s| s.take()) else {
            return;
        };
        if let Some((area, ppp)) = self.pending_png.take() {
            match charts::png(&image, area, ppp) {
                Some(bytes) => {
                    let name = format!("{}-charts.png", self.export_stem());
                    self.requests.push(Request::Export { name, bytes });
                }
                None => self.report_error("the charts could not be captured"),
            }
        }
    }

    /// The start of an export's file name: the model's file name without its extension.
    fn export_stem(&self) -> String {
        let name = self.document_name();
        name.rsplit_once('.')
            .map_or(name.clone(), |(stem, _)| stem.to_owned())
    }

    /// Saves to `path`, chosen by the user for Save As.
    pub fn save_to(&mut self, path: &str) {
        match self.engine.save_bytes(false) {
            Ok(bytes) => self.requests.push(Request::Write {
                path: path.to_owned(),
                bytes,
            }),
            Err(e) => self.report_error(e.message),
        }
    }

    /// The shell wrote (or downloaded) `bytes` as `path`.
    pub fn written(&mut self, path: &str, bytes: &[u8]) {
        match self.engine.saved_to(path, bytes) {
            Ok(_) => {
                self.info(format!("Saved {}", file_name(path)));
                if let Some(next) = self.after_save.take() {
                    self.replace_now(next);
                }
            }
            Err(e) => self.report_error(e.message),
        }
    }

    /// File ▸ Save: into the file the model came from, or as a new file.
    pub fn save(&mut self) {
        match (
            self.engine.path().map(str::to_owned),
            self.platform.file_system,
        ) {
            (Some(path), true) => self.save_to(&path),
            (None, true) => self.save_as(),
            (path, false) => self.download(path),
        }
    }

    /// File ▸ Save As.
    pub fn save_as(&mut self) {
        if self.platform.file_system {
            let name = self.suggested_name();
            self.requests.push(Request::PickSave { name });
        } else {
            self.download(None);
        }
    }

    fn suggested_name(&self) -> String {
        match self.engine.path() {
            Some(path) => file_name(path).to_owned(),
            None => format!("{}.bpmn", self.document_name()),
        }
    }

    fn download(&mut self, name: Option<String>) {
        let name = name.unwrap_or_else(|| self.suggested_name());
        match self.engine.save_bytes(false) {
            Ok(bytes) => self.requests.push(Request::Download { name, bytes }),
            Err(e) => self.report_error(e.message),
        }
    }

    /// Replaces or closes the model, first asking about unsaved changes.
    fn replace(&mut self, action: Replace) {
        if self.engine.is_dirty() {
            self.confirm = Some(action);
        } else {
            self.replace_now(action);
        }
    }

    fn replace_now(&mut self, action: Replace) {
        match action {
            Replace::New => {
                if self.execute("model.new", json!({})).is_some() {
                    self.results = None;
                    self.selected = None;
                    self.drafts.clear();
                    self.show_results = false;
                    self.info("New model");
                }
            }
            Replace::Open => self.requests.push(Request::PickOpen),
            Replace::Example(i) => {
                if let Some(e) = EXAMPLES.get(i) {
                    self.open_file(e.file, e.bytes);
                }
            }
            Replace::Dropped => {
                if let Some((name, bytes)) = self.dropped.take() {
                    self.open_file(&name, &bytes);
                }
            }
            Replace::Quit => self.requests.push(Request::Quit),
        }
    }

    /// The window is closing (the user closed it): returns whether it may close now. If not, the
    /// app asks about unsaved changes and requests [`Request::Quit`] once they are dealt with.
    pub fn close_requested(&mut self) -> bool {
        if self.engine.is_dirty() {
            self.confirm = Some(Replace::Quit);
            false
        } else {
            true
        }
    }

    pub fn undo(&mut self) {
        if let Some(r) = self.execute("undo", json!({})) {
            let label = change_label(r["undone"].as_str().unwrap_or(""));
            self.info(format!("Undid {label}"));
        }
    }

    pub fn redo(&mut self) {
        if let Some(r) = self.execute("redo", json!({})) {
            let label = change_label(r["redone"].as_str().unwrap_or(""));
            self.info(format!("Redid {label}"));
        }
    }

    /// Model ▸ Run: starts the experiment in the background.
    pub fn run(&mut self) {
        if self.running.is_some() {
            return;
        }
        if let Some(status) = self.execute("run.start", json!({})) {
            self.show_results = true;
            self.running = Some(Running {
                done: 0,
                total: status["total"].as_u64().unwrap_or(0),
            });
            self.info("Running…");
        }
    }

    /// Stops the run in progress.
    pub fn cancel_run(&mut self) {
        if self.running.is_some() {
            let status = self.execute("run.cancel", json!({}));
            self.running = None;
            if status.is_some() {
                self.info("Run cancelled");
            }
        }
    }

    /// Advances and collects the background run. Called every frame; true while it runs.
    pub fn poll(&mut self) -> bool {
        if self.running.is_none() {
            return false;
        }
        // In the browser a run advances one replication per frame, so the page stays responsive.
        let status = self.engine.poll_run(1);
        match status["state"].as_str() {
            Some("running") => {
                if let Some(r) = &mut self.running {
                    r.done = status["done"].as_u64().unwrap_or(r.done);
                }
                true
            }
            Some("finished") => {
                self.running = None;
                self.results_digest = status["result"]["record"]["model_digest"]
                    .as_str()
                    .map(str::to_owned);
                self.results = Some(status["result"].clone());
                self.show_results = true;
                let n = status["total"].as_u64().unwrap_or(0);
                self.info(format!(
                    "Finished {n} replication{}",
                    if n == 1 { "" } else { "s" }
                ));
                false
            }
            _ => {
                self.running = None;
                let message = status["error"]["message"]
                    .as_str()
                    .unwrap_or("the run stopped")
                    .to_owned();
                self.report_error(message);
                false
            }
        }
    }

    /// Results go stale when the model changes; they are dropped then.
    fn drop_stale_results(&mut self) {
        if self.results.is_some() {
            let digest = stochcraft_engine::model_digest(self.engine.model());
            if self.results_digest.as_deref() != Some(digest.as_str()) {
                self.results = None;
                if self.running.is_none() {
                    self.show_results = false;
                }
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.confirm.is_some() || self.about {
            return;
        }
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        // Longest modifiers first, so Cmd+Shift+S is not taken for Cmd+S.
        if pressed(keys::SAVE_AS) {
            self.save_as();
        } else if pressed(keys::SAVE) {
            self.save();
        }
        if pressed(keys::REDO) {
            self.redo();
        } else if pressed(keys::UNDO) {
            self.undo();
        }
        if pressed(keys::NEW) {
            self.replace(Replace::New);
        }
        if pressed(keys::OPEN) {
            self.replace(Replace::Open);
        }
        if pressed(keys::RUN) {
            self.run();
        }
        if self.platform.can_quit && pressed(keys::QUIT) {
            self.replace(Replace::Quit);
        }
        if self.running.is_some() && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.cancel_run();
        }
        // The block list from the keyboard, when no field is being edited.
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing && !self.show_results {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                self.move_selection(true);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                self.move_selection(false);
            }
            if self.selected.is_some()
                && ctx.input_mut(|i| {
                    i.consume_key(Modifiers::NONE, Key::Delete)
                        || i.consume_key(Modifiers::NONE, Key::Backspace)
                })
            {
                self.remove_selected();
            }
            if self.running.is_none()
                && self.selected.is_some()
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            {
                self.selected = None;
            }
        }
    }

    /// Draws the window and handles its input. Call once per frame with the root `Ui`.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.styled {
            theme::install(&ctx);
            self.styled = true;
        }
        if self.poll() {
            ctx.request_repaint();
        }
        self.take_screenshot();
        self.shortcuts(&ctx);
        self.drop_stale_results();
        let p = theme::palette(&ctx);

        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(p.shell)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ui, |ui| self.header(ui, &p));
        egui::Panel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(p.rail)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .inner_margin(egui::Margin::symmetric(12, 4)),
            )
            .show(ui, |ui| self.status_bar(ui, &p));
        egui::Panel::left("outline")
            .resizable(true)
            .default_size(280.0)
            .size_range(220.0..=420.0)
            .frame(
                egui::Frame::new()
                    .fill(p.rail)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .inner_margin(egui::Margin::symmetric(16, 14)),
            )
            .show(ui, |ui| self.outline(ui, &p));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(p.canvas)
                    .inner_margin(egui::Margin::symmetric(28, 22)),
            )
            .show(ui, |ui| {
                let has_model = !self.engine.model().elements.is_empty();
                if has_model || self.results.is_some() || self.running.is_some() {
                    self.tabs(ui, &p);
                    ui.add_space(14.0);
                }
                if self.show_results && self.results.is_some() {
                    self.results_view(ui);
                } else if has_model && !(self.show_results && self.running.is_some()) {
                    self.editor(ui, &p);
                } else {
                    // A readable column, not the full width of a wide window.
                    let size = egui::vec2(ui.available_width().min(640.0), ui.available_height());
                    ui.allocate_ui_with_layout(
                        size,
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            if let Some(r) = &self.running {
                                let (done, total) = (r.done, r.total);
                                self.running_view(ui, done, total);
                            } else if self.engine.model().elements.is_empty() {
                                self.welcome(ui, &p);
                            } else {
                                self.ready(ui);
                            }
                        },
                    );
                }
            });

        self.confirm_dialog(&ctx);
        self.about_window(&ctx);
    }

    /// The results: export buttons, the charts, then the table.
    fn results_view(&mut self, ui: &mut egui::Ui) {
        let Some(results) = self.results.clone() else {
            return;
        };
        let mut csv = false;
        let mut png = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Results");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let verb = if self.platform.file_system {
                            "Export"
                        } else {
                            "Download"
                        };
                        png = ui
                            .add(theme::secondary(ui, &format!("{verb} charts (PNG)")))
                            .clicked();
                        csv = ui
                            .add(theme::secondary(ui, &format!("{verb} table (CSV)")))
                            .clicked();
                    });
                });
                results::record_line(ui, &results);
                ui.add_space(12.0);
                self.charts_rect = charts::show(ui, &results);
                ui.add_space(6.0);
                results::show(ui, &results);
            });
        if csv {
            self.export_csv();
        }
        if png {
            let ctx = ui.ctx().clone();
            self.export_png(&ctx);
        }
    }

    /// The work area's two views: the model (overview or inspector) and the results.
    fn tabs(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        ui.horizontal(|ui| {
            for (label, results) in [("Model", false), ("Results", true)] {
                let on = self.show_results == results;
                let text = if on {
                    theme::strong(label).color(p.text)
                } else {
                    theme::weak(ui, label)
                };
                let r = ui.add(egui::Button::selectable(on, text).min_size(egui::vec2(0.0, 28.0)));
                // The current view is underlined in the accent.
                if on {
                    let rect = r.rect;
                    ui.painter().line_segment(
                        [
                            rect.left_bottom() + egui::vec2(4.0, 2.0),
                            rect.right_bottom() + egui::vec2(-4.0, 2.0),
                        ],
                        egui::Stroke::new(2.0, p.accent),
                    );
                }
                if r.clicked() {
                    self.show_results = results;
                }
            }
            if self.results.is_none() && self.running.is_none() {
                ui.label(theme::weak(ui, "· no results yet"));
            }
        });
        let rect = ui.max_rect();
        let y = ui.cursor().top() + 4.0;
        ui.painter()
            .hline(rect.x_range(), y, egui::Stroke::new(1.0, p.border));
    }

    /// The header: menus on the shell colour, the document's name, and Run.
    fn header(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        // Text and hover on the dark shell.
        let v = ui.visuals_mut();
        v.override_text_color = Some(p.on_shell);
        v.widgets.hovered.weak_bg_fill = p.on_shell.gamma_multiply(0.12);
        v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
        v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        v.widgets.open.weak_bg_fill = p.on_shell.gamma_multiply(0.16);
        let running = self.running.is_some();
        let can_run = !self.engine.model().elements.is_empty();
        let (_, clicked) = egui::Sides::new().shrink_left().show(
            ui,
            |ui| {
                ui.label(theme::strong("StochCraft").size(15.0));
                ui.add_space(8.0);
                self.menu_bar(ui);
                ui.add_space(8.0);
                let dirty = if self.engine.is_dirty() {
                    " — edited"
                } else {
                    ""
                };
                ui.label(
                    egui::RichText::new(format!("{}{dirty}", self.document_name()))
                        .color(p.on_shell_weak),
                );
            },
            |ui| {
                ui.visuals_mut().override_text_color = None;
                if running {
                    let cancel =
                        egui::Button::new(egui::RichText::new("Cancel run").color(p.on_shell))
                            .stroke(egui::Stroke::new(1.0, p.on_shell_weak))
                            .fill(p.shell)
                            .min_size(egui::vec2(0.0, 30.0));
                    ui.add(cancel).on_hover_text("Esc").clicked()
                } else {
                    let shortcut = ui.ctx().format_shortcut(&keys::RUN);
                    ui.add_enabled(can_run, theme::primary(ui, "Run"))
                        .on_hover_text(format!("Run the experiment ({shortcut})"))
                        .on_disabled_hover_text("Open a model first")
                        .clicked()
                }
            },
        );
        if clicked {
            if running {
                self.cancel_run();
            } else {
                self.run();
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let item = |ui: &mut egui::Ui, label: &str, key: Option<KeyboardShortcut>, enabled| {
            let mut b = egui::Button::new(label);
            if let Some(k) = key {
                b = b.shortcut_text(theme::weak(ui, ctx.format_shortcut(&k)));
            }
            ui.add_enabled(enabled, b).clicked()
        };
        // Menus side by side in the header (egui::MenuBar would take the rest of the row).
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                ui.visuals_mut().override_text_color = None;
                if item(ui, "New", Some(keys::NEW), true) {
                    self.replace(Replace::New);
                }
                if item(ui, "Open…", Some(keys::OPEN), true) {
                    self.replace(Replace::Open);
                }
                ui.menu_button("Open Example", |ui| {
                    for (i, e) in EXAMPLES.iter().enumerate() {
                        if ui.button(e.title).clicked() {
                            self.replace(Replace::Example(i));
                        }
                    }
                });
                ui.separator();
                let save = if self.platform.file_system {
                    "Save"
                } else {
                    "Download"
                };
                if item(ui, save, Some(keys::SAVE), true) {
                    self.save();
                }
                if self.platform.file_system && item(ui, "Save As…", Some(keys::SAVE_AS), true) {
                    self.save_as();
                }
                if self.platform.can_quit {
                    ui.separator();
                    if item(ui, "Quit", Some(keys::QUIT), true) {
                        self.replace(Replace::Quit);
                    }
                }
            });
            ui.menu_button("Edit", |ui| {
                ui.visuals_mut().override_text_color = None;
                let history = self.engine_history();
                let undo = history.0.map(|l| format!("Undo {l}"));
                let redo = history.1.map(|l| format!("Redo {l}"));
                if item(
                    ui,
                    undo.as_deref().unwrap_or("Undo"),
                    Some(keys::UNDO),
                    undo.is_some(),
                ) {
                    self.undo();
                }
                if item(
                    ui,
                    redo.as_deref().unwrap_or("Redo"),
                    Some(keys::REDO),
                    redo.is_some(),
                ) {
                    self.redo();
                }
            });
            ui.menu_button("Model", |ui| {
                ui.visuals_mut().override_text_color = None;
                let running = self.running.is_some();
                let has_model = !self.engine.model().elements.is_empty();
                if item(ui, "Run", Some(keys::RUN), !running && has_model) {
                    self.run();
                }
                if item(ui, "Cancel Run", None, running) {
                    self.cancel_run();
                }
            });
            ui.menu_button("Help", |ui| {
                ui.visuals_mut().override_text_color = None;
                if ui.button("About StochCraft").clicked() {
                    self.about = true;
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        ui.horizontal(|ui| {
            if self.status_error {
                ui.label(theme::strong("Error").color(p.error));
                ui.label(egui::RichText::new(&self.status).color(p.error));
            } else if self.status.is_empty() {
                ui.label(theme::weak(ui, "Ready"));
            } else {
                ui.label(theme::weak(ui, &self.status));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(path) = self.engine.path() {
                    ui.label(theme::weak(ui, path));
                }
            });
        });
    }

    /// The model as its process: each element as its BPMN shape on one line from arrival to
    /// departure, then the experiment it runs.
    fn outline(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let problems = self.problems();
        let model = self.engine.model();
        let name = if model.name.is_empty() {
            "Untitled model".to_owned()
        } else {
            model.name.clone()
        };
        let overview = self.selected.is_none() && !self.show_results;
        let r = ui
            .add(
                egui::Button::selectable(overview, theme::strong(&name).size(17.0))
                    .frame_when_inactive(false),
            )
            .on_hover_text("The model: its experiment, connections and problems");
        let mut select: Option<Option<String>> = None;
        if r.clicked() {
            select = Some(None);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(theme::strong("Process").size(12.0).color(p.text_weak));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(
                    theme::strong("+ Add block").size(12.0).color(p.accent),
                    |ui| {
                        for (kind, label) in editor::KINDS {
                            if ui.button(*label).clicked() {
                                self.add_block(kind);
                                self.show_results = false;
                            }
                        }
                    },
                );
            });
        });
        ui.add_space(4.0);
        let model = self.engine.model();
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height(ui.available_height() - 130.0)
            .show(ui, |ui| {
                if model.elements.is_empty() {
                    ui.label(theme::weak(
                        ui,
                        "No blocks yet. Open a model, or add blocks to build one.",
                    ));
                }
                let n = model.elements.len();
                for (i, e) in model.elements.iter().enumerate() {
                    let id = e.id.as_str();
                    let title = if e.name.is_empty() {
                        id
                    } else {
                        e.name.as_str()
                    };
                    let mine = problems.iter().filter(|pr| pr.at.as_deref() == Some(id));
                    let flag = mine.fold(None, |acc: Option<bool>, pr| {
                        Some(acc.unwrap_or(false) || pr.error)
                    });
                    let selected = self.selected.as_deref() == Some(id) && !self.show_results;
                    let row = FlowRow {
                        kind: e.kind.name(),
                        title,
                        first: i == 0,
                        last: i + 1 == n,
                        selected,
                        flag,
                    };
                    if row.show(ui, p).clicked() {
                        select = Some(Some(id.to_owned()));
                    }
                }
            });
        if let Some(sel) = select {
            self.selected = sel;
            self.show_results = false;
        }
        let model = self.engine.model();
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let x = &model.experiment;
            let unit = time_unit(&self.engine);
            ui.label(theme::weak(
                ui,
                format!(
                    "Warm-up {} {unit}, then {} {unit} measured",
                    results::number(x.warmup),
                    results::number(x.duration)
                ),
            ));
            ui.label(format!("{} replications · seed {}", x.replications, x.seed));
            ui.add_space(2.0);
            ui.label(theme::strong("Experiment").size(12.0).color(p.text_weak));
            ui.add_space(4.0);
            ui.separator();
        });
    }

    /// The empty state: what StochCraft opens, and a way in.
    fn welcome(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        ui.add_space(24.0);
        ui.heading("Open a model to simulate it");
        ui.add_space(6.0);
        ui.label(theme::weak(
            ui,
            "StochCraft reads process models as BPMN or XPDL files with BPSim simulation data, \
             runs them as replicated experiments and reports every result with its confidence \
             interval.",
        ));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary(ui, "Open a model…")).clicked() {
                self.replace(Replace::Open);
            }
            ui.label(theme::weak(ui, ui.ctx().format_shortcut(&keys::OPEN)));
        });
        ui.add_space(28.0);
        ui.label(theme::strong("Or start from an example"));
        ui.add_space(6.0);
        for (i, e) in EXAMPLES.iter().enumerate() {
            let r = egui::Frame::new()
                .stroke(egui::Stroke::new(1.0, p.border))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(theme::strong(e.title));
                            ui.label(theme::weak(ui, e.about));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(theme::secondary(ui, "Open")).clicked() {
                                self.replace(Replace::Example(i));
                            }
                            ui.label(theme::weak(ui, e.file).monospace());
                        });
                    });
                });
            let _ = r;
            ui.add_space(6.0);
        }
    }

    /// A model is open and has no results yet.
    fn ready(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.heading("No results yet");
        ui.add_space(6.0);
        let x = &self.engine.model().experiment;
        ui.label(theme::weak(
            ui,
            format!(
                "Runs {} independent replications of the model from seed {}. Each element's \
                 results come with the half-width of a 95% confidence interval across them.",
                x.replications, x.seed
            ),
        ));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary(ui, "Run experiment")).clicked() {
                self.run();
            }
            ui.label(theme::weak(ui, ui.ctx().format_shortcut(&keys::RUN)));
        });
    }

    fn running_view(&mut self, ui: &mut egui::Ui, done: u64, total: u64) {
        ui.add_space(24.0);
        ui.heading("Running");
        ui.add_space(10.0);
        let fraction = if total == 0 {
            0.0
        } else {
            done as f32 / total as f32
        };
        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_height(8.0)
                .fill(theme::palette(ui.ctx()).accent),
        );
        ui.add_space(6.0);
        ui.label(theme::weak(
            ui,
            format!("{done} of {total} replications finished"),
        ));
        ui.add_space(12.0);
        if ui
            .add(theme::secondary(ui, "Cancel run"))
            .on_hover_text("Esc")
            .clicked()
        {
            self.cancel_run();
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.confirm else {
            return;
        };
        let name = self.document_name();
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("unsaved")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.heading(format!("Save changes to {name}?"));
            ui.add_space(4.0);
            ui.label(theme::weak(
                ui,
                "Your changes will be lost if you don't save them.",
            ));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let save = if self.platform.file_system {
                    "Save"
                } else {
                    "Download"
                };
                if ui.add(theme::primary(ui, save)).clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.add(theme::secondary(ui, "Don't Save")).clicked() {
                    choice = Some(Choice::Discard);
                }
                if ui.add(theme::secondary(ui, "Cancel")).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        match choice {
            Some(Choice::Save) => {
                self.confirm = None;
                self.after_save = Some(action);
                self.save();
            }
            Some(Choice::Discard) => {
                self.confirm = None;
                self.replace_now(action);
            }
            Some(Choice::Cancel) => {
                self.confirm = None;
                self.dropped = None;
            }
            None => {}
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.about;
        egui::Window::new("About StochCraft")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.label(
                    theme::strong(format!("StochCraft {}", env!("CARGO_PKG_VERSION"))).size(17.0),
                );
                ui.label(theme::weak(
                    ui,
                    "Discrete-event simulation, from model files to results.",
                ));
                ui.hyperlink("https://github.com/helenkwok/StochCraft");
                ui.add_space(10.0);
                ui.label(
                    "Licensed under the MIT License or the Apache License 2.0, at your option.",
                );
                ui.add_space(10.0);
                ui.label(theme::strong("Built with"));
                egui::Grid::new("credits")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for (name, licence) in CREDITS {
                            ui.label(*name);
                            ui.label(theme::weak(ui, *licence));
                            ui.end_row();
                        }
                    });
            });
        self.about = open && self.about;
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.about = false;
        }
    }
}

/// One block in the outline's process line: its BPMN shape (a thin circle for a start event, a
/// rounded box for a task, a thick circle for an end event), joined to its neighbours. Clicking
/// it selects the block; a dot marks a problem (red for an error, amber for a warning).
struct FlowRow<'a> {
    kind: &'a str,
    title: &'a str,
    first: bool,
    last: bool,
    selected: bool,
    /// Some(true): an error; Some(false): a warning.
    flag: Option<bool>,
}

impl FlowRow<'_> {
    fn show(&self, ui: &mut egui::Ui, p: &theme::Palette) -> egui::Response {
        let height = 40.0;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), height),
            egui::Sense::click(),
        );
        let what = match self.kind {
            "source" => "arrivals",
            "server" => "task with a resource",
            "delay" => "task",
            "sink" => "departures",
            other => other,
        };
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::ListBoxOption, true, self.selected, self.title)
        });
        let painter = ui.painter_at(rect.expand(2.0));
        if self.selected {
            painter.rect_filled(rect, 6.0, p.accent_soft);
        } else if response.hovered() {
            painter.rect_filled(rect, 6.0, p.stripe);
        }
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                6.0,
                egui::Stroke::new(1.5, p.accent),
                egui::StrokeKind::Inside,
            );
        }
        let cx = rect.left() + 14.0;
        let cy = rect.center().y;
        let line = egui::Stroke::new(1.5, p.border);
        if !self.first {
            painter.line_segment([egui::pos2(cx, rect.top()), egui::pos2(cx, cy)], line);
        }
        if !self.last {
            painter.line_segment([egui::pos2(cx, cy), egui::pos2(cx, rect.bottom())], line);
        }
        let ink = p.text;
        match self.kind {
            "source" => {
                painter.circle(
                    egui::pos2(cx, cy),
                    9.0,
                    p.raised,
                    egui::Stroke::new(1.5, ink),
                );
            }
            "sink" => {
                painter.circle(
                    egui::pos2(cx, cy),
                    9.0,
                    p.raised,
                    egui::Stroke::new(3.0, ink),
                );
            }
            _ => {
                let r = egui::Rect::from_center_size(egui::pos2(cx, cy), egui::vec2(24.0, 18.0));
                let fill = if self.kind == "server" {
                    p.accent_soft
                } else {
                    p.raised
                };
                painter.rect(
                    r,
                    4.0,
                    fill,
                    egui::Stroke::new(1.5, ink),
                    egui::StrokeKind::Middle,
                );
            }
        }
        let text_x = rect.left() + 38.0;
        painter.text(
            egui::pos2(text_x, cy - 8.0),
            egui::Align2::LEFT_CENTER,
            self.title,
            egui::FontId::proportional(14.0),
            p.text,
        );
        painter.text(
            egui::pos2(text_x, cy + 9.0),
            egui::Align2::LEFT_CENTER,
            what,
            egui::FontId::proportional(12.0),
            p.text_weak,
        );
        if let Some(error) = self.flag {
            let color = if error { p.error } else { p.warning };
            painter.circle_filled(egui::pos2(rect.right() - 12.0, cy), 4.0, color);
            return response.on_hover_text(if error {
                "This block has an error"
            } else {
                "This block has a warning"
            });
        }
        response
    }
}

/// The model's time unit as written in results ("min", "h", …).
fn time_unit(engine: &Engine) -> String {
    serde_json::to_value(stochcraft_engine::wire::TimeUnitSpec::from(
        engine.model().experiment.time_unit,
    ))
    .ok()
    .and_then(|v| v.as_str().map(|u| results::unit_label(u).to_owned()))
    .unwrap_or_default()
}

/// The main libraries the app is built on, for the About box. Every dependency's licence is
/// checked by `cargo deny` against the allowed list in `deny.toml`.
const CREDITS: &[(&str, &str)] = &[
    ("egui and eframe (Craft suite fork)", "MIT OR Apache-2.0"),
    ("craft-libs", "MIT OR Apache-2.0"),
    ("serde, serde_json", "MIT OR Apache-2.0"),
    ("libm", "MIT OR Apache-2.0"),
    ("sha2", "MIT OR Apache-2.0"),
];

#[derive(Clone, Copy)]
enum Choice {
    Save,
    Discard,
    Cancel,
}

/// How the Edit menu and the status bar name a change, from the command that made it.
fn change_label(command: &str) -> &str {
    match command {
        "model.set" => "Replace Model",
        "model.rename" => "Rename Model",
        "block.add" => "Add Block",
        "block.set" => "Change Block",
        "block.rename" => "Rename Block",
        "block.remove" => "Remove Block",
        "connect" => "Connect",
        "disconnect" => "Disconnect",
        "experiment.set" => "Experiment Settings",
        "property.set" => "Set Property",
        "property.remove" => "Remove Property",
        other => other,
    }
}

/// The last component of a path, either separator.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}
