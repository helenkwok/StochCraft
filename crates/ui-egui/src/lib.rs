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

mod autosave;
mod calendars;
mod canvas;
mod charts;
pub mod commands;
mod decide;
pub use autosave::Autosave;
mod editor;
pub mod form;
pub mod i18n;
mod live;
mod pools;
mod results;
pub mod theme;

use egui::{Key, Modifiers};
use serde_json::{Value, json};
use stochcraft_engine::{CommandError, Engine, ErrorCode};

use crate::i18n::{t, td, tf, tn, tnf};

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
    /// Read the file at `path` (File ▸ Open Recent); answer with [`App::open_file`], or
    /// [`App::report_error`] if it can't be read.
    OpenPath { path: String },
    /// Ask where to save, suggesting `name`; answer with [`App::save_to`]. BPMN is offered, and
    /// with `xpdl` XPDL too: the model came from an XPDL file, which it can be saved into.
    PickSave { name: String, xpdl: bool },
    /// Write `bytes` to `path`; answer with [`App::written`] or [`App::report_error`].
    Write { path: String, bytes: Vec<u8> },
    /// Offer `bytes` as a download named `name` (where there is no file system); answer with
    /// [`App::downloaded`] once the download has started, or [`App::report_error`].
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
    /// File ▸ Open Recent: the recent file at this index.
    Recent(usize),
    /// A file dropped on the window (held in `App::dropped`).
    Dropped,
    Quit,
}

/// A save that needs a new BPMN file, written from the model alone, held while the user decides.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NewFile {
    /// Why the model can't be saved into the opened file; none when the user chose BPMN for an
    /// XPDL file.
    reason: Option<String>,
    /// The file chosen in Save As. Without one, the user is asked where (desktop), or the
    /// download is named for a BPMN file (browser).
    path: Option<String>,
}

/// The appearance the user chose: the system's, or always light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark]
            .into_iter()
            .find(|t| t.key() == key)
    }

    fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        }
    }
}

/// What the app remembers between sessions: kept by the shell (craft-settings on the desktop,
/// the browser's storage on the web).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub appearance: Appearance,
    pub files: Files,
    pub keyboard: Keyboard,
}

/// The user's own shortcuts: craft-keymap's overrides of the defaults (`{"file.save": "Ctrl+S"}`,
/// `""` for none), as Settings ▸ Keyboard Shortcuts… saves them.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Keyboard {
    /// Left out when the user changed nothing (TOML has no null).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcuts: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    /// The interface language's code (`de`, `pt-br`); left out to follow the system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Files {
    /// Recently opened or saved model files, newest first.
    pub recent: Vec<String>,
}

/// How many files File ▸ Open Recent lists.
pub const RECENT_FILES: usize = 10;

/// A run in the background: its last reported status.
struct Running {
    done: u64,
    /// How far the replication under way has got (0 to 1), for a run in slices.
    partial: f64,
    total: u64,
    /// The shell's [`Workers`] run it, rather than the page between frames.
    on_workers: bool,
}

/// Workers the shell offers for runs: in the browser, web workers, one per core. The app hands a
/// run's replications to them (`"mode": "workers"`, see [`stochcraft_engine::Worker`]) instead of
/// running them on the page in slices of events between frames. The results are the same either
/// way.
pub trait Workers {
    /// Starts running `task` (from [`Engine::worker_task`]), whose replications are numbered
    /// from 0 to `replications - 1`: each worker makes a [`stochcraft_engine::Worker`] from it
    /// and runs the replications it is given. False if no worker could start; the app then runs
    /// the replications itself.
    fn start(&mut self, task: &str, replications: u64) -> bool;
    /// The workers' answers that have arrived since the last call, for [`Engine::deliver`].
    fn answers(&mut self) -> Vec<String>;
    /// Stops every worker: the run finished, failed or was cancelled.
    fn stop(&mut self);
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
    /// The shell's workers for runs, if it has any.
    workers: Option<Box<dyn Workers>>,
    /// The last message for the status bar: what happened, or what went wrong.
    status: String,
    /// The status is an error.
    status_error: bool,
    /// Waiting for the user to save or discard changes before this.
    confirm: Option<Replace>,
    /// Do this once the save in progress has been written.
    after_save: Option<Replace>,
    /// Waiting for the user to agree to saving as a new BPMN file.
    new_file: Option<NewFile>,
    /// The user agreed to a new BPMN file and is choosing where it goes.
    as_new: bool,
    about: bool,
    /// A file dropped on the window, waiting on the unsaved-changes question.
    dropped: Option<(String, Vec<u8>)>,
    requests: Vec<Request>,
    /// The title last shown, so the shell sets it only when it changes.
    title: String,
    /// The theme is installed (on the first frame).
    styled: bool,
    /// The language whose font stack is installed: the fonts are installed again when the
    /// interface language changes, so its faces lead (and, on the web, are fetched).
    fonts_lang: Option<craft_i18n::Lang>,
    /// The languages View ▸ Language names in English for now, because the font for their own
    /// script hasn't been fetched (the web; see [`theme::names_waiting_for_fonts`]).
    names_in_english: Vec<craft_i18n::Lang>,
    /// The block the inspector shows; none shows the model overview (or, with several blocks
    /// selected on the canvas, what can be done to them).
    selected: Option<String>,
    /// The flowchart's view, its multiple selection and its clipboard.
    canvas: canvas::Canvas,
    /// A run being watched on the canvas (M3.5).
    watching: Option<live::Watching>,
    /// The model last validated and its problems, so a frame validates at most once.
    problems_of: Option<(stochcraft_model::Model, Vec<editor::Problem>)>,
    /// Text being typed into form fields, until it commits.
    drafts: std::collections::HashMap<String, String>,
    /// The connections table's choices, before Connect.
    connect_from: Option<String>,
    connect_to: Option<String>,
    /// The work area shows the results rather than the model.
    show_results: bool,
    /// The work area shows the Analysis view (sweeps, sensitivity, sizing), unless the results
    /// are shown.
    show_analysis: bool,
    /// The M2.8 screens' choices and last answers.
    decide: decide::Decide,
    /// Where the charts were drawn last frame, for exporting them.
    charts_rect: Option<egui::Rect>,
    /// A screenshot asked for to export the charts, filled in by the renderer.
    screenshot: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<egui::ColorImage>>>>,
    /// The charts' area and scale when the screenshot was asked for.
    pending_png: Option<(egui::Rect, f32)>,
    /// Commands and their shortcuts (craft-keymap), and the frame's key dispatch.
    keymap: craft_keymap::Keymap,
    dispatcher: craft_keymap::egui::Dispatcher,
    /// Recently opened or saved files, newest first (desktop only).
    recent: Vec<String>,
    theme_choice: ThemeChoice,
    /// The interface language as the user chose it (`None`: the system's), and as shown.
    language: Option<String>,
    lang: craft_i18n::Lang,
    /// The theme last applied to the context.
    applied_theme: Option<ThemeChoice>,
    /// The shell shows the menus in the system's menu bar (macOS), not in the window.
    native_menu: bool,
    /// The digest of the model last handed out as a recovery copy.
    autosaved: Option<String>,
    /// The recovery copy can go (a damaged one, or the user discarded it).
    autosave_clear_pending: bool,
    /// A recovery copy found at start, waiting for the user's answer.
    recovery: Option<Value>,
    /// The user chose to quit (discarding any changes): no recovery copy is kept.
    quitting: bool,
    /// Settings ▸ Keyboard Shortcuts…, while it is open (craft-keymap's dialog).
    shortcuts_dialog: Option<craft_keymap::ui::DialogState>,
    /// The app draws the window frame (craft-window chose it): the header is the title bar and
    /// shows these window buttons.
    title_bar: Option<craft_window::Layout>,
    /// How numbers are written and read in the fields: the user's region (craft-i18n).
    numbers: craft_ui::localized::LocaleNumbers,
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
            workers: None,
            status: String::new(),
            status_error: false,
            confirm: None,
            after_save: None,
            new_file: None,
            as_new: false,
            about: false,
            dropped: None,
            requests: Vec::new(),
            title: String::new(),
            styled: false,
            fonts_lang: None,
            names_in_english: Vec::new(),
            selected: None,
            canvas: canvas::Canvas::default(),
            watching: None,
            problems_of: None,
            drafts: std::collections::HashMap::new(),
            connect_from: None,
            connect_to: None,
            show_results: false,
            show_analysis: false,
            decide: decide::Decide::default(),
            charts_rect: None,
            screenshot: std::sync::Arc::default(),
            pending_png: None,
            keymap: commands::keymap(craft_keymap::Platform::current()),
            dispatcher: craft_keymap::egui::Dispatcher::new(),
            recent: Vec::new(),
            theme_choice: ThemeChoice::System,
            language: None,
            lang: i18n::I18N.system_lang(),
            applied_theme: None,
            native_menu: false,
            shortcuts_dialog: None,
            autosaved: None,
            autosave_clear_pending: false,
            recovery: None,
            quitting: false,
            title_bar: None,
            numbers: craft_ui::localized::LocaleNumbers::new(std::sync::Arc::new(
                craft_i18n::Format::new(&craft_i18n::Region::system(), craft_i18n::Digits::Locale),
            )),
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

    /// What to remember between sessions.
    pub fn prefs(&self) -> Prefs {
        Prefs {
            appearance: Appearance {
                theme: self.theme_choice,
                language: self.language.clone(),
            },
            files: Files {
                recent: self.recent.clone(),
            },
            keyboard: Keyboard {
                shortcuts: serde_json::to_value(self.keymap.overrides())
                    .ok()
                    .filter(|v| match v {
                        Value::Object(m) => !m.is_empty(),
                        Value::Array(a) => !a.is_empty(),
                        _ => false,
                    }),
            },
        }
    }

    /// Restores what was remembered.
    pub fn set_prefs(&mut self, prefs: &Prefs) {
        self.theme_choice = prefs.appearance.theme;
        self.set_language(prefs.appearance.language.as_deref());
        self.recent = prefs.files.recent.clone();
        self.recent.truncate(RECENT_FILES);
        let overrides = prefs
            .keyboard
            .shortcuts
            .clone()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        self.keymap.set_overrides(overrides);
    }

    /// Shows the interface in the language with this code, or the system's (`None`, or a code no
    /// longer embedded).
    pub fn set_language(&mut self, code: Option<&str>) {
        i18n::apply_setting(code);
        self.lang = i18n::lang();
        // Kept as chosen, so a build without that language doesn't forget it.
        self.language = code.map(str::to_owned);
    }

    /// The interface language (the pseudo locale in tests).
    pub fn set_lang(&mut self, lang: craft_i18n::Lang) {
        self.lang = lang;
        i18n::set_lang(lang);
    }

    /// Settings ▸ Keyboard Shortcuts…: craft-keymap's dialog over the app's keymap.
    pub(crate) fn open_shortcuts(&mut self) {
        if self.shortcuts_dialog.is_none() {
            self.shortcuts_dialog = Some(craft_keymap::ui::DialogState::new());
        }
    }

    /// Draws the shortcuts dialog while it is open. OK keeps the edits (the shell saves them with
    /// the preferences); Cancel puts the keymap back as it was.
    fn shortcuts_window(&mut self, ctx: &egui::Context) {
        let Some(state) = &mut self.shortcuts_dialog else {
            return;
        };
        let text = craft_keymap::ui::I18nText {
            i18n: &i18n::I18N,
            lang: i18n::lang(),
        };
        let outcome =
            craft_keymap::ui::ShortcutsDialog::new(&mut self.keymap, &text).show(ctx, state);
        if !state.is_open() {
            self.shortcuts_dialog = None;
        }
        match outcome {
            Some(craft_keymap::ui::Outcome::Ok) => self.info(t("Keyboard shortcuts saved")),
            Some(craft_keymap::ui::Outcome::Cancel) => {
                self.info(t("Keyboard shortcuts unchanged"));
            }
            _ => {}
        }
    }

    /// The shell shows the menus in the system's menu bar, so the window doesn't (macOS).
    pub fn set_native_menu(&mut self, on: bool) {
        self.native_menu = on;
    }

    /// Who draws the window frame (craft-window's decision): when it is the app, the header acts
    /// as the title bar, with the window buttons where this desktop puts them.
    pub fn set_window_frame(&mut self, frame: &craft_window::Frame) {
        self.title_bar = frame.layout().cloned();
    }

    /// The platform the menus are laid out for: the browser's own layout (no Quit, the
    /// browser's shortcuts left to it) or this desktop's.
    pub fn menu_target(&self) -> craft_menu::Target {
        if self.platform.can_quit {
            craft_menu::Target::current()
        } else {
            craft_menu::Target::Web
        }
    }

    /// Writes and reads numbers in the fields as `tag`'s region does (`de-CH`, `ar-EG`), in place
    /// of the system's region.
    pub fn set_number_region(&mut self, tag: &str) {
        self.numbers = craft_ui::localized::LocaleNumbers::from_tag(tag);
    }

    /// A dialog is open (menu commands wait).
    pub fn has_open_dialog(&self) -> bool {
        self.modal_open()
    }

    /// A command's shortcut as this platform writes it ("F5", "⌘O", "Ctrl+O").
    pub fn shortcut(&self, command: &str, ctx: &egui::Context) -> Option<String> {
        commands::shortcut_text(&self.keymap, command, craft_keymap::egui::platform(ctx))
    }

    /// Opens a model file read from `path` and lists it under File ▸ Open Recent.
    pub fn open_path(&mut self, path: &str, bytes: &[u8]) {
        self.open_file(path, bytes);
        if self.engine.path() == Some(path) {
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
        self.engine.is_dirty()
    }

    fn document_name(&self) -> String {
        if let Some(path) = self.engine.path() {
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
    /// canvas's view, a watched run and half-typed fields.
    fn forget_view(&mut self) {
        self.results = None;
        self.selected = None;
        self.canvas = canvas::Canvas::default();
        self.watching = None;
        self.drafts.clear();
        self.show_results = false;
        self.show_analysis = false;
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
        let result = self.engine.execute(command, params)?;
        match command {
            "file.open" | "model.new" | "model.set" => {
                self.forget_view();
                if let Some(path) = result["path"].as_str() {
                    self.info(tf("Opened {file}", &[("file", file_name(path))]));
                }
            }
            "run" => {
                self.results_digest = result["record"]["model_digest"].as_str().map(str::to_owned);
                self.results = Some(result.clone());
                self.show_results = true;
                let n = result["replications"].as_u64().unwrap_or(0);
                self.info(tn(
                    n,
                    "Finished {n} replication",
                    "Finished {n} replications",
                ));
            }
            _ => {}
        }
        Ok(result)
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
        if let Some(r) = &self.results {
            let name = format!("{}-results.csv", self.export_stem());
            let bytes = charts::csv(r).into_bytes();
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
                None => self.report_error(t("the charts could not be captured")),
            }
        }
    }

    /// The start of an export's file name: the model's file name without its extension.
    fn export_stem(&self) -> String {
        let name = self.document_name();
        name.rsplit_once('.')
            .map_or(name.clone(), |(stem, _)| stem.to_owned())
    }

    /// Saves to `path`, chosen by the user for Save As, in the format its extension names
    /// (`.bpmn`, `.xpdl`; otherwise the opened file's).
    pub fn save_to(&mut self, path: &str) {
        let agreed = std::mem::take(&mut self.as_new);
        self.save_file(path.to_owned(), true, agreed);
    }

    /// Saves to `path` (written on the desktop, downloaded under its name in the browser): into
    /// the opened file when it is that file's format and can take the changes, else as a new
    /// BPMN file once the user has agreed (`agreed`: already, in the question that asked for
    /// `path`). `chosen`: the user chose `path` for Save As, so its extension says the format;
    /// otherwise it is the opened file's own.
    fn save_file(&mut self, path: String, chosen: bool, agreed: bool) {
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
            self.report_error(tf(
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
                });
            }
            Err(e) => self.report_error(e.message),
        }
    }

    /// Writes (or downloads) the model to `path` as a new BPMN file.
    fn write_new(&mut self, path: String) {
        match self.engine.save_bytes(true) {
            Ok(bytes) => self.deliver(path, bytes),
            Err(e) => self.report_error(e.message),
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
            Err(e) => self.report_error(e.message),
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
            self.requests.push(Request::PickSave { name, xpdl });
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
        self.save_file(name, false, false);
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
                    self.canvas = canvas::Canvas::default();
                    self.watching = None;
                    self.drafts.clear();
                    self.show_results = false;
                    self.show_analysis = false;
                    self.info(t("New model"));
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
            self.info(tf("Undid {action}", &[("action", &td(label))]));
        }
    }

    pub fn redo(&mut self) {
        if let Some(r) = self.execute("redo", json!({})) {
            let label = change_label(r["redone"].as_str().unwrap_or(""));
            self.info(tf("Redid {action}", &[("action", &td(label))]));
        }
    }

    /// Runs go to these workers from now on (the browser's web workers).
    pub fn set_workers(&mut self, workers: Box<dyn Workers>) {
        self.workers = Some(workers);
    }

    /// Model ▸ Run: starts the experiment in the background.
    pub fn run(&mut self) {
        if self.running.is_some() {
            return;
        }
        let on_workers = self.workers.is_some() && self.start_on_workers();
        let status = if on_workers {
            Some(self.engine.poll_run(0, 0))
        } else {
            self.execute("run.start", json!({}))
        };
        if let Some(status) = status {
            self.show_results = true;
            self.running = Some(Running {
                done: 0,
                partial: 0.0,
                total: status["total"].as_u64().unwrap_or(0),
                on_workers,
            });
            self.info(t("Running…"));
        }
    }

    /// Starts the run on the shell's workers; false (with nothing left running) if it could not
    /// start there, so the app runs it itself.
    fn start_on_workers(&mut self) -> bool {
        let Ok(status) = self
            .engine
            .execute("run.start", json!({ "mode": "workers" }))
        else {
            return false;
        };
        let total = status["total"].as_u64().unwrap_or(0);
        let started = match (self.engine.worker_task(), self.workers.as_mut()) {
            (Some(task), Some(workers)) => workers.start(task, total),
            _ => false,
        };
        if !started {
            let _ = self.engine.execute("run.cancel", json!({}));
        }
        started
    }

    /// Stops the run in progress.
    pub fn cancel_run(&mut self) {
        if self.running.is_some() {
            self.stop_workers();
            let status = self.execute("run.cancel", json!({}));
            self.running = None;
            if status.is_some() {
                self.info(t("Run cancelled"));
            }
        }
    }

    /// Advances and collects the background run. Called every frame; true while it runs.
    pub fn poll(&mut self) -> bool {
        if self.running.is_none() {
            return false;
        }
        // Answers from the shell's workers; otherwise, in the browser, a run advances a slice of
        // events per frame (a long replication over several frames), so the page stays
        // responsive and Cancel acts within a frame.
        if self.running.as_ref().is_some_and(|r| r.on_workers)
            && let Some(workers) = self.workers.as_mut()
        {
            let refused = workers
                .answers()
                .iter()
                .find_map(|answer| self.engine.deliver(answer).err());
            // Not expected: the workers run this same engine. The run stops rather than wait for
            // an answer that will not come.
            if let Some(e) = refused {
                self.stop_workers();
                let _ = self.engine.execute("run.cancel", json!({}));
                self.running = None;
                self.report_error(e.message);
                return false;
            }
        }
        let status = self.engine.poll_run(1, stochcraft_engine::SLICE_EVENTS);
        if status["state"] != "running" {
            self.stop_workers();
        }
        match status["state"].as_str() {
            Some("running") => {
                if let Some(r) = &mut self.running {
                    r.done = status["done"].as_u64().unwrap_or(r.done);
                    r.partial = status["partial"].as_f64().unwrap_or(0.0);
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
                self.info(tn(
                    n,
                    "Finished {n} replication",
                    "Finished {n} replications",
                ));
                false
            }
            _ => {
                self.running = None;
                let message = status["error"]["message"]
                    .as_str()
                    .unwrap_or(t("the run stopped"))
                    .to_owned();
                self.report_error(message);
                false
            }
        }
    }

    fn stop_workers(&mut self) {
        if self.running.as_ref().is_some_and(|r| r.on_workers)
            && let Some(workers) = self.workers.as_mut()
        {
            workers.stop();
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
                    self.show_analysis = false;
                }
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        // No block keys while a dialog is open: Delete pressed to rebind a shortcut must not
        // delete the selection.
        if self.modal_open() || self.about {
            return;
        }
        self.dispatch_shortcuts(ctx);
        if self.running.is_some() && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.cancel_run();
        }
        // The block list from the keyboard, when no field is being edited.
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing && !self.show_results {
            if self.is_watching() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space)) {
                self.toggle_playing();
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                self.move_selection(true);
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                self.move_selection(false);
            }
            let something = !self.selection().is_empty() || self.canvas.flow.is_some();
            if something
                && ctx.input_mut(|i| {
                    i.consume_key(Modifiers::NONE, Key::Delete)
                        || i.consume_key(Modifiers::NONE, Key::Backspace)
                })
            {
                self.delete_selection();
            }
            if self.running.is_none()
                && something
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            {
                self.select(std::collections::BTreeSet::new());
            }
        }
    }

    /// Draws the window and handles its input. Call once per frame with the root `Ui`.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        i18n::set_lang(self.lang);
        results::set_numbers(&self.numbers);
        if self.fonts_lang != Some(self.lang) {
            theme::install_fonts(&ctx, i18n::code(self.lang));
            self.fonts_lang = Some(self.lang);
        }
        if !self.styled {
            theme::install(&ctx);
            self.styled = true;
        }
        if self.poll() {
            ctx.request_repaint();
        }
        self.take_screenshot();
        if self.applied_theme != Some(self.theme_choice) {
            ctx.set_theme(self.theme_choice.preference());
            self.applied_theme = Some(self.theme_choice);
        }
        self.shortcuts(&ctx);
        self.tick_watching(&ctx);
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
                } else if self.show_analysis && has_model && !self.show_results {
                    self.analysis_view(ui);
                } else if has_model && !(self.show_results && self.running.is_some()) {
                    self.model_view(ui, &p);
                } else {
                    // A readable column, not the full width of a wide window.
                    let size = egui::vec2(ui.available_width().min(640.0), ui.available_height());
                    ui.allocate_ui_with_layout(
                        size,
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            if let Some(r) = &self.running {
                                let (done, partial, total) = (r.done, r.partial, r.total);
                                self.running_view(ui, done, partial, total);
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
        self.new_file_dialog(&ctx);
        self.about_window(&ctx);
        self.shortcuts_window(&ctx);
        self.recovery_dialog(&ctx);
    }

    /// The model: the flowchart, with the inspector beside it (below it, in a narrow window).
    fn model_view(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let wide = ui.available_width() >= 720.0;
        let frame = egui::Frame::new().inner_margin(egui::Margin {
            left: if wide { 18 } else { 0 },
            top: if wide { 0 } else { 14 },
            ..egui::Margin::ZERO
        });
        let panel = if wide {
            egui::Panel::right("inspector")
                .resizable(true)
                .default_size(400.0)
                .size_range(300.0..=(ui.available_width() - 320.0).clamp(300.0, 640.0))
        } else {
            egui::Panel::bottom("inspector")
                .resizable(true)
                .default_size(ui.available_height() * 0.5)
                .size_range(160.0..=ui.available_height().max(200.0) * 0.8)
        };
        panel.frame(frame).show_separator_line(true).show(ui, |ui| {
            // The panel keeps its width; wide rows wrap or scroll rather than widen it.
            ui.set_max_width(ui.available_width());
            self.editor(ui, p);
        });
        self.watch_bar(ui, p);
        self.canvas(ui, p);
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
                    ui.heading(t("Results"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (charts, table) = if self.platform.file_system {
                            (t("Export charts (PNG)"), t("Export table (CSV)"))
                        } else {
                            (t("Download charts (PNG)"), t("Download table (CSV)"))
                        };
                        png = ui.add(theme::secondary(ui, charts)).clicked();
                        csv = ui.add(theme::secondary(ui, table)).clicked();
                    });
                });
                results::record_line(ui, &results);
                self.stop_line(ui, &results);
                ui.add_space(12.0);
                self.charts_rect = charts::show(ui, &results);
                ui.add_space(6.0);
                results::show(ui, &results);
                self.distribution_section(ui, &results);
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
        // craft-ui's Tab: an accessible tab (role, keyboard focus) the app lays out and paints.
        let font = egui::FontId::new(14.0, egui::FontFamily::Name(theme::STRONG.into()));
        ui.horizontal(|ui| {
            // 0: the model, 1: the results, 2: the Analysis view.
            let current = if self.show_results {
                1
            } else if self.show_analysis {
                2
            } else {
                0
            };
            for (label, view) in [(t("Model"), 0), (t("Results"), 1), (t("Analysis"), 2)] {
                let on = current == view;
                let width = ui.fonts_mut(|f| {
                    f.layout_no_wrap(label.into(), font.clone(), p.text)
                        .size()
                        .x
                }) + 24.0;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::hover());
                let tab = craft_ui::tabs::Tab::new(egui::Id::unique(("view-tab", view)), label, on)
                    .focus_stroke(egui::Stroke::new(1.5, p.accent));
                let r = tab.show_at(ui, rect, |ui, r| {
                    let painter = ui.painter_at(rect);
                    if r.hovered() && !on {
                        painter.rect_filled(rect.shrink2(egui::vec2(2.0, 3.0)), 6.0, p.stripe);
                    }
                    let color = if on { p.text } else { p.text_weak };
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        label,
                        font.clone(),
                        color,
                    );
                    // The current view is underlined in the accent.
                    if on {
                        painter.line_segment(
                            [
                                rect.left_bottom() + egui::vec2(6.0, -1.0),
                                rect.right_bottom() + egui::vec2(-6.0, -1.0),
                            ],
                            egui::Stroke::new(2.0, p.accent),
                        );
                    }
                });
                if r.clicked() {
                    self.show_results = view == 1;
                    self.show_analysis = view == 2;
                }
            }
            if self.results.is_none() && self.running.is_none() {
                ui.label(theme::weak(ui, t("· no results yet")));
            }
        });
        let rect = ui.max_rect();
        let y = ui.cursor().top() + 4.0;
        ui.painter()
            .hline(rect.x_range(), y, egui::Stroke::new(1.0, p.border));
    }

    /// The header: menus on the shell colour, the document's name, and Run.
    fn header(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let run_key = self.shortcut(commands::RUN, ui.ctx()).unwrap_or_default();
        // As the title bar: its empty part moves the window and double-clicks as the system does.
        let window_buttons = self.title_bar.clone();
        if window_buttons.is_some() {
            craft_window::title_bar::empty_area(ui, ui.max_rect());
        }
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
                // The window buttons this desktop puts on the left, outermost first.
                for b in window_buttons.iter().flat_map(|l| l.left.iter().rev()) {
                    window_button(ui, p, *b);
                }
                ui.label(theme::strong("StochCraft").size(15.0));
                ui.add_space(8.0);
                self.menu_bar_ui(ui, p);
                ui.add_space(8.0);
                let name = self.document_name();
                let shown = if self.engine.is_dirty() {
                    tf("{name} — edited", &[("name", &name)])
                } else {
                    name
                };
                ui.label(egui::RichText::new(shown).color(p.on_shell_weak));
            },
            |ui| {
                ui.visuals_mut().override_text_color = None;
                // The window buttons on the right, outermost (rightmost) first.
                for b in window_buttons.iter().flat_map(|l| l.right.iter().rev()) {
                    window_button(ui, p, *b);
                }
                if running {
                    let cancel =
                        egui::Button::new(egui::RichText::new(t("Cancel run")).color(p.on_shell))
                            .stroke(egui::Stroke::new(1.0, p.on_shell_weak))
                            .fill(p.shell)
                            .min_size(egui::vec2(0.0, 30.0));
                    ui.add(cancel).on_hover_text("Esc").clicked()
                } else {
                    let shortcut = run_key.clone();
                    ui.add_enabled(can_run, theme::primary(ui, t("Run")))
                        .on_hover_text(tf(
                            "Run the experiment ({shortcut})",
                            &[("shortcut", &shortcut)],
                        ))
                        .on_disabled_hover_text(t("Open a model first"))
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

    /// The in-window menu bar (craft-menu), laid out for this platform. Not drawn when the shell
    /// shows the menus in the system's menu bar.
    fn menu_bar_ui(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        if self.native_menu {
            return;
        }
        self.names_in_english = theme::names_waiting_for_fonts(ui.ctx());
        let target = self.menu_target();
        let layout = self.menu_layout(target);
        let mut style = craft_menu::BarStyle::from_style(ui.style());
        style.title_color = p.on_shell;
        let gate = craft_menu::Gate {
            modal: self.modal_open(),
            ..craft_menu::Gate::default()
        };
        // As wide as its titles need, so the document's name fits beside it.
        let room = (ui.available_width() - 260.0).max(120.0);
        let chosen = craft_menu::MenuBarUi::new(&layout.bar)
            .target(target)
            .style(style)
            .gate(gate)
            .shrink_to_fit(room)
            .show(ui);
        for command in chosen {
            self.run_command(&command);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        ui.horizontal(|ui| {
            if self.status_error {
                ui.label(theme::strong(t("Error")).color(p.error));
                ui.label(egui::RichText::new(&self.status).color(p.error));
            } else if self.status.is_empty() {
                ui.label(theme::weak(ui, t("Ready")));
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
            t("Untitled model").to_owned()
        } else {
            model.name.clone()
        };
        let overview = self.selected.is_none() && !self.show_results;
        let r = ui
            .add(
                egui::Button::selectable(overview, theme::strong(&name).size(17.0))
                    .frame_when_inactive(false),
            )
            .on_hover_text(t("The model: its experiment, connections and problems"));
        let mut select: Option<Option<String>> = None;
        if r.clicked() {
            select = Some(None);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(theme::strong(t("Process")).size(12.0).color(p.text_weak));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button(
                    theme::strong(t("+ Add block")).size(12.0).color(p.accent),
                    |ui| {
                        for (kind, label) in editor::KINDS {
                            if ui.button(t(label)).clicked() {
                                self.add_block(kind);
                                self.show_results = false;
                                self.show_analysis = false;
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
                        t("No blocks yet. Open a model, or add blocks to build one."),
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
                    // Its own problems and its outgoing flows' (a decision's conditions).
                    let outs: Vec<&str> = model
                        .connections
                        .iter()
                        .filter(|c| c.from.as_str() == id)
                        .map(|c| c.id.as_str())
                        .collect();
                    let mine = problems.iter().filter(|pr| {
                        pr.at
                            .as_deref()
                            .is_some_and(|at| at == id || outs.contains(&at))
                    });
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
            self.show_analysis = false;
        }
        let model = self.engine.model();
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let x = &model.experiment;
            let unit = time_unit(&self.engine);
            ui.label(theme::weak(
                ui,
                tf(
                    "Warm-up {warmup} {unit}, then {duration} {unit} measured",
                    &[
                        ("warmup", &results::grouped(x.warmup)),
                        ("duration", &results::grouped(x.duration)),
                        ("unit", &unit),
                    ],
                ),
            ));
            ui.label(tnf(
                u64::from(x.replications),
                "{n} replication · seed {seed}",
                "{n} replications · seed {seed}",
                &[("seed", &x.seed.to_string())],
            ));
            ui.add_space(2.0);
            ui.label(theme::strong(t("Experiment")).size(12.0).color(p.text_weak));
            ui.add_space(4.0);
            ui.separator();
        });
    }

    /// The empty state: what StochCraft opens, and a way in.
    fn welcome(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        ui.add_space(24.0);
        ui.heading(t("Open a model to simulate it"));
        ui.add_space(6.0);
        ui.label(theme::weak(
            ui,
            t(
                "StochCraft reads process models as BPMN or XPDL files with BPSim simulation data, \
               runs them as replicated experiments and reports every result with its confidence \
               interval.",
            ),
        ));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary(ui, t("Open a model…"))).clicked() {
                self.replace(Replace::Open);
            }
            let key = self.shortcut(commands::OPEN, ui.ctx()).unwrap_or_default();
            ui.label(theme::weak(ui, key));
        });
        ui.add_space(28.0);
        ui.label(theme::strong(t("Or start from an example")));
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
                            ui.label(theme::strong(t(e.title)));
                            ui.label(theme::weak(ui, t(e.about)));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(theme::secondary(ui, t("Open"))).clicked() {
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
        ui.heading(t("No results yet"));
        ui.add_space(6.0);
        let x = &self.engine.model().experiment;
        ui.label(theme::weak(
            ui,
            tnf(
                u64::from(x.replications),
                "Runs {n} independent replication of the model from seed {seed}. Each \
                 element's results come with the half-width of a 95% confidence interval.",
                "Runs {n} independent replications of the model from seed {seed}. Each \
                 element's results come with the half-width of a 95% confidence interval across \
                 them.",
                &[("seed", &x.seed.to_string())],
            ),
        ));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary(ui, t("Run experiment"))).clicked() {
                self.run();
            }
            let key = self.shortcut(commands::RUN, ui.ctx()).unwrap_or_default();
            ui.label(theme::weak(ui, key));
        });
    }

    fn running_view(&mut self, ui: &mut egui::Ui, done: u64, partial: f64, total: u64) {
        ui.add_space(24.0);
        ui.heading(t("Running"));
        ui.add_space(10.0);
        let fraction = if total == 0 {
            0.0
        } else {
            ((done as f64 + partial) / total as f64).min(1.0) as f32
        };
        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_height(8.0)
                .fill(theme::palette(ui.ctx()).accent),
        );
        ui.add_space(6.0);
        ui.label(theme::weak(
            ui,
            tnf(
                total,
                "{done} of {n} replication finished",
                "{done} of {n} replications finished",
                &[("done", &done.to_string())],
            ),
        ));
        ui.add_space(12.0);
        if ui
            .add(theme::secondary(ui, t("Cancel run")))
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
            ui.heading(tf("Save changes to {name}?", &[("name", &name)]));
            ui.add_space(4.0);
            ui.label(theme::weak(
                ui,
                t("Your changes will be lost if you don't save them."),
            ));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let save = if self.platform.file_system {
                    t("Save")
                } else {
                    t("Download")
                };
                if ui.add(theme::primary(ui, save)).clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.add(theme::secondary(ui, t("Don't Save"))).clicked() {
                    choice = Some(Choice::Discard);
                }
                if ui.add(theme::secondary(ui, t("Cancel"))).clicked()
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

    /// Asks before a save writes a new BPMN file, which leaves out what StochCraft does not
    /// understand in the opened file, saying why the opened file can't be saved into.
    fn new_file_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.new_file.clone() else {
            return;
        };
        let name = self.document_name();
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("new file")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(t("Save as a new BPMN file?"));
            ui.add_space(4.0);
            if let Some(reason) = &pending.reason {
                ui.label(reason);
                ui.add_space(4.0);
            }
            ui.label(theme::weak(
                ui,
                tf(
                    "A new file holds the model alone: what StochCraft does not understand in {file} is left out.",
                    &[("file", &name)],
                ),
            ));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let go = match (&pending.path, self.platform.file_system) {
                    (Some(_), _) => t("Save"),
                    (None, true) => t("Save As…"),
                    (None, false) => t("Download"),
                };
                if ui.add(theme::primary(ui, go)).clicked() {
                    choice = Some(Choice::Save);
                }
                if ui.add(theme::secondary(ui, t("Cancel"))).clicked()
                    || ui.input(|i| i.key_pressed(Key::Escape))
                {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        match choice {
            Some(Choice::Save) => {
                self.new_file = None;
                let bpmn = format!("{}.bpmn", self.export_stem());
                match pending.path {
                    Some(path) => self.write_new(path),
                    None if self.platform.file_system => {
                        self.as_new = true;
                        self.requests.push(Request::PickSave {
                            name: bpmn,
                            xpdl: false,
                        });
                    }
                    None => self.write_new(bpmn),
                }
            }
            Some(Choice::Discard | Choice::Cancel) => {
                // Nothing was saved, so what was to follow the save is dropped.
                self.new_file = None;
                self.after_save = None;
            }
            None => {}
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.about;
        egui::Window::new(t("About StochCraft"))
            .id(egui::Id::unique("about"))
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
                    t("Discrete-event simulation, from model files to results."),
                ));
                ui.hyperlink("https://github.com/helenkwok/StochCraft");
                ui.add_space(10.0);
                ui.label(t(
                    "Licensed under the MIT License or the Apache License 2.0, at your option.",
                ));
                ui.add_space(10.0);
                ui.label(theme::strong(t("Built with")));
                egui::Grid::new("credits")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        for (name, licence) in CREDITS {
                            // Names and SPDX licences, through the catalog for the words
                            // some carry ("Craft suite fork").
                            ui.label(td(name));
                            ui.label(theme::weak(ui, td(licence)));
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

/// A window button of an app-drawn title bar, drawn as a vector mark (minimise: a bar; maximise:
/// a square, or two when maximised; close: a cross).
fn window_button(ui: &mut egui::Ui, p: &theme::Palette, button: craft_window::Button) {
    use craft_window::Button;
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    let (label, command) = match button {
        Button::Minimize => (t("Minimise"), egui::ViewportCommand::Minimized(true)),
        Button::Maximize if maximized => (t("Restore"), egui::ViewportCommand::Maximized(false)),
        Button::Maximize => (t("Maximise"), egui::ViewportCommand::Maximized(true)),
        Button::Close => (t("Close"), egui::ViewportCommand::Close),
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let painter = ui.painter_at(rect);
    if response.hovered() {
        let fill = if matches!(button, Button::Close) {
            p.error
        } else {
            p.on_shell.gamma_multiply(0.14)
        };
        painter.rect_filled(rect.shrink(3.0), 6.0, fill);
    }
    let ink = egui::Stroke::new(1.3, p.on_shell);
    let c = rect.center();
    let r = 5.0;
    match button {
        Button::Minimize => {
            painter.line_segment([c + egui::vec2(-r, 0.0), c + egui::vec2(r, 0.0)], ink);
        }
        Button::Maximize => {
            let square = egui::Rect::from_center_size(c, egui::vec2(2.0 * r, 2.0 * r));
            if maximized {
                let back = square.translate(egui::vec2(2.0, -2.0));
                painter.rect_stroke(back, 1.0, ink, egui::StrokeKind::Middle);
                painter.rect_filled(square, 1.0, p.shell);
            }
            painter.rect_stroke(square, 1.0, ink, egui::StrokeKind::Middle);
        }
        Button::Close => {
            painter.line_segment([c + egui::vec2(-r, -r), c + egui::vec2(r, r)], ink);
            painter.line_segment([c + egui::vec2(-r, r), c + egui::vec2(r, -r)], ink);
        }
    }
    if response.on_hover_text(label).clicked() {
        ui.ctx().send_viewport_cmd(command);
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
        let what = kind_text(self.kind);
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
            "exclusive_gateway" | "parallel_gateway" => {
                // A BPMN gateway: a diamond, with × (exclusive) or + (parallel) inside.
                let c = egui::pos2(cx, cy);
                let r = 10.0;
                let diamond = vec![
                    c + egui::vec2(0.0, -r),
                    c + egui::vec2(r, 0.0),
                    c + egui::vec2(0.0, r),
                    c + egui::vec2(-r, 0.0),
                ];
                painter.add(egui::Shape::convex_polygon(
                    diamond,
                    p.raised,
                    egui::Stroke::new(1.5, ink),
                ));
                let m = 4.0;
                let mark = egui::Stroke::new(1.5, ink);
                if self.kind == "parallel_gateway" {
                    painter.line_segment([c + egui::vec2(-m, 0.0), c + egui::vec2(m, 0.0)], mark);
                    painter.line_segment([c + egui::vec2(0.0, -m), c + egui::vec2(0.0, m)], mark);
                } else {
                    painter.line_segment([c + egui::vec2(-m, -m), c + egui::vec2(m, m)], mark);
                    painter.line_segment([c + egui::vec2(-m, m), c + egui::vec2(m, -m)], mark);
                }
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
                t("This block has an error")
            } else {
                t("This block has a warning")
            });
        }
        response
    }
}

/// What a block of `kind` is, in a few lowercase words ("task with a resource").
pub(crate) fn kind_text(kind: &str) -> &str {
    match kind {
        "source" => t("arrivals"),
        "server" => t("task with a resource"),
        "shared_task" => t("task with shared resources"),
        "delay" => t("task"),
        "sink" => t("departures"),
        "exclusive_gateway" => t("decision: one way"),
        "parallel_gateway" => t("split or join: all ways"),
        "batch" => t("batch"),
        "unbatch" => t("split batches"),
        other => other,
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
        "flow.set" => "Change Flow",
        "disconnect" => "Disconnect",
        "experiment.set" => "Experiment Settings",
        "property.set" => "Set Property",
        "property.remove" => "Remove Property",
        "resource.add" => "Add Pool",
        "resource.set" => "Change Pool",
        "resource.remove" => "Remove Pool",
        "calendar.add" => "Add Calendar",
        "calendar.set" => "Change Calendar",
        "calendar.remove" => "Remove Calendar",
        "layout.set" => "Move",
        "batch" => "Changes",
        "paste" => "Paste",
        "delete" => "Delete",
        "align" => "Align",
        "duplicate" => "Duplicate",
        other => other,
    }
}

/// The last component of a path, either separator.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A file's extension in lower case, without the dot (empty if it has none).
fn extension(path: &str) -> String {
    file_name(path)
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}
