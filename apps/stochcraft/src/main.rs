//! StochCraft on the desktop: the shared interface ([`stochcraft_ui_egui::App`]) in a native
//! window, on the Craft suite's foundation (craft-libs):
//!
//! - craft-dialogs: the operating system's file dialogs, which never block the event loop;
//! - craft-settings: preferences (appearance, recent files) in the platform's config folder,
//!   saved atomically while the app runs;
//! - craft-window: who draws the window frame on this desktop;
//! - craft-quit: quitting the platform's own way (on macOS through `NSApplication terminate:`);
//! - craft-menu: the system menu bar on macOS (elsewhere the menus are in the window);
//! - craft-atomic: crash-safe saves (a model file is never left half-written).
//!
//! `stochcraft [FILE]` opens FILE at start. `stochcraft --control PORT` also listens on
//! `127.0.0.1:PORT` for engine commands from the MCP server (`stochcraft-cli mcp --connect`,
//! docs/mcp.md), so a person can watch an agent work in the window.
//!
//! On Windows a release build is a window-only program, so starting it opens no console window
//! beside it; what it prints (the `--control` address, a usage error) is then not shown.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::time::Instant;

use craft_dialogs::{Answer, Dialogs, FileDialog};
use craft_settings::{Location, Settings, SettingsStore};
use stochcraft_mcp::control;
use stochcraft_ui_egui::i18n::{t, tf};
use stochcraft_ui_egui::{App, Autosave, Platform, Prefs, Request};

/// What a file dialog's answer is for.
enum Then {
    Open,
    Save,
    /// An export's bytes, to write where the user chose.
    Export(Vec<u8>),
}

/// The app's preferences as craft-settings stores them (`config.toml`).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
struct Stored(Prefs);

impl Settings for Stored {
    const APP_ID: &'static str = "stochcraft";
    const SCHEMA_VERSION: u32 = 1;
}

struct Desktop {
    app: App,
    dialogs: Dialogs<Then>,
    settings: SettingsStore<Stored>,
    /// Closing was confirmed (nothing unsaved, or the user chose).
    may_close: bool,
    /// When the recovery copy was last brought up to date.
    autosaved_at: Instant,
    /// The system menu bar (macOS).
    #[cfg(target_os = "macos")]
    native_menu: Option<craft_menu::NativeMenu>,
    /// Commands from the control channel (`--control`), waiting for the next frame.
    control: Option<std::sync::mpsc::Receiver<control::Call>>,
}

impl Desktop {
    fn new(ctx: &egui::Context, file: Option<PathBuf>, control_port: Option<u16>) -> Self {
        let wake = ctx.clone();
        let mut app = App::new(Platform::DESKTOP);
        let (settings, report) = SettingsStore::<Stored>::open(Location::Auto);
        app.set_prefs(&settings.get().0);
        if report.has_problems() {
            app.report_error("the settings could not all be read; defaults are used where not");
        }
        if let Some(copy) = recovery_path().and_then(|p| std::fs::read_to_string(p).ok()) {
            app.offer_recovery(&copy);
        }
        if let Some(path) = file {
            open(&mut app, &path);
        }
        let control = control_port.and_then(|port| {
            let wake = ctx.clone();
            match control::listen(port, move || wake.request_repaint()) {
                Ok((addr, calls)) => {
                    eprintln!("stochcraft: listening for engine commands on {addr}");
                    Some(calls)
                }
                Err(e) => {
                    app.report_error(format!("--control {port}: {e}"));
                    None
                }
            }
        });
        #[cfg(target_os = "macos")]
        let native_menu = {
            let bar = app.menu_bar();
            let menu = craft_menu::mac::install(ctx, &bar);
            app.set_native_menu(menu.is_some());
            menu
        };
        Self {
            app,
            dialogs: Dialogs::new(std::sync::Arc::new(move || wake.request_repaint())),
            settings,
            may_close: false,
            autosaved_at: Instant::now(),
            #[cfg(target_os = "macos")]
            native_menu,
            control,
        }
    }

    /// Runs the commands that came in on the control channel, and answers them.
    fn take_control_calls(&mut self) {
        let Some(calls) = &self.control else {
            return;
        };
        for call in calls.try_iter().collect::<Vec<_>>() {
            let result = self.app.control(&call.command, call.params.clone());
            call.answer(result);
        }
    }

    /// Carries out what the app asked for, and takes in the dialogs' answers.
    fn serve(&mut self, ctx: &egui::Context) {
        for request in self.app.take_requests() {
            match request {
                Request::PickOpen => self.dialogs.ask(
                    FileDialog::open()
                        .title(t("Open Model"))
                        .filter(t("Process models with BPSim"), &["bpmn", "xpdl", "xml"]),
                    Then::Open,
                ),
                Request::OpenPath { path } => open(&mut self.app, Path::new(&path)),
                Request::PickSave { name, xpdl } => {
                    let mut dialog = FileDialog::save().title(t("Save Model")).file_name(name);
                    // The opened file's format first.
                    if xpdl {
                        dialog = dialog.filter(t("XPDL with BPSim"), &["xpdl"]);
                    }
                    let dialog = dialog.filter(t("BPMN with BPSim"), &["bpmn"]);
                    self.dialogs.ask(dialog, Then::Save);
                }
                Request::Write { path, bytes } | Request::Download { name: path, bytes } => {
                    match craft_atomic::atomic_write(Path::new(&path), &bytes) {
                        Ok(()) => self.app.written(&path, &bytes),
                        Err(e) => self.app.report_error(format!("{path}: {e}")),
                    }
                }
                Request::Export { name, bytes } => {
                    let ext = name.rsplit('.').next().unwrap_or("").to_owned();
                    let what = if ext == "png" {
                        t("PNG image")
                    } else {
                        t("CSV table")
                    };
                    self.dialogs.ask(
                        FileDialog::save()
                            .title(t("Export"))
                            .file_name(name)
                            .filter(what, &[ext.as_str()]),
                        Then::Export(bytes),
                    );
                }
                Request::Quit => {
                    self.may_close = true;
                    // On macOS the platform's quit; elsewhere closing the window.
                    if !craft_quit::quit() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        for (then, answer) in self.dialogs.take_answers() {
            match (then, answer) {
                (Then::Open, Answer::Paths(paths)) => {
                    if let Some(path) = paths.first() {
                        open(&mut self.app, path);
                    }
                }
                (Then::Save, Answer::Paths(paths)) => match paths.first() {
                    Some(path) => self.app.save_to(&path.to_string_lossy()),
                    None => self.app.save_cancelled(),
                },
                (Then::Export(bytes), Answer::Paths(paths)) => {
                    if let Some(path) = paths.first() {
                        let shown = path.to_string_lossy().into_owned();
                        match craft_atomic::atomic_write(path, &bytes) {
                            Ok(()) => self.app.exported(&shown),
                            Err(e) => self.app.report_error(format!("{shown}: {e}")),
                        }
                    }
                }
                (_, Answer::Unavailable(why)) => self.app.report_error(why),
                (Then::Save, _) => self.app.save_cancelled(),
                _ => {}
            }
        }
    }

    /// Keeps the settings file in step with the app (saved, debounced, as it changes). An idle
    /// window draws no frames, so it asks for one when the save is due.
    fn sync_settings(&mut self, ctx: Option<&egui::Context>) {
        let prefs = self.app.prefs();
        if self.settings.get().0 != prefs {
            self.settings.set(Stored(prefs));
        }
        let now = Instant::now();
        let tick = self.settings.tick(now);
        if let (Some(ctx), Some(due)) = (ctx, tick.next_deadline) {
            ctx.request_repaint_after(due.saturating_duration_since(now));
        }
        if let craft_settings::SaveStatus::Failed(e) = tick.status {
            self.app.report_error(tf(
                "the settings could not be saved: {error}",
                &[("error", &e.to_string())],
            ));
        }
    }
}

/// Where the recovery copy of unsaved changes is kept: the platform's local data folder.
fn recovery_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("stochcraft").join("recovery.json"))
}

/// How often the recovery copy is brought up to date while there are unsaved changes.
const AUTOSAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

impl Desktop {
    /// Keeps the recovery copy in step with the model (atomically written, so a crash while
    /// writing never leaves a broken one), at most every [`AUTOSAVE_EVERY`] unless `now`.
    fn autosave(&mut self, ctx: Option<&egui::Context>, now: bool) {
        let due = now || self.autosaved_at.elapsed() >= AUTOSAVE_EVERY;
        if !due {
            if let Some(ctx) = ctx.filter(|_| self.app.has_unsaved_changes()) {
                ctx.request_repaint_after(
                    AUTOSAVE_EVERY.saturating_sub(self.autosaved_at.elapsed()),
                );
            }
            return;
        }
        self.autosaved_at = Instant::now();
        let Some(path) = recovery_path() else {
            return;
        };
        match self.app.autosave() {
            Autosave::Write(copy) => {
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| craft_atomic::atomic_write(&path, copy.as_bytes()));
                if let Err(e) = written {
                    self.app.report_error(format!("autosave: {e}"));
                }
            }
            Autosave::Clear => {
                let _ = std::fs::remove_file(&path);
            }
            Autosave::Keep => {}
        }
    }
}

/// Opens the model file at `path`, listing it under File ▸ Open Recent.
fn open(app: &mut App, path: &Path) {
    let name = path.to_string_lossy();
    match std::fs::read(path) {
        Ok(bytes) => app.open_path(&name, &bytes),
        Err(e) => app.report_error(format!("{name}: {e}")),
    }
}

impl eframe::App for Desktop {
    /// The close decision, before `ui` (and while the window is hidden): unsaved changes keep the
    /// window open and ask; then craft-quit routes the close the platform's way.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Here rather than in `ui`, which a hidden window does not run.
        self.take_control_calls();
        if ctx.input(|i| i.viewport().close_requested())
            && !self.may_close
            && !self.app.close_requested()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        craft_quit::on_close_request(ctx);
    }

    /// The system menu bar's key equivalents reach egui as the keys they are (macOS).
    #[cfg(target_os = "macos")]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        if let Some(menu) = &mut self.native_menu {
            menu.raw_input(raw);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // A model file dropped on the window opens (after asking about unsaved changes).
        for file in ctx.input(|i| i.raw.dropped_files.clone()) {
            let name = file.path().to_string_lossy().into_owned();
            match file.bytes() {
                Ok(bytes) => self.app.drop_file(&name, bytes),
                Err(e) => self.app.report_error(format!("{name}: {e}")),
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(menu) = &mut self.native_menu {
            menu.set_gate(craft_menu::Gate {
                modal: self.app.has_open_dialog(),
                text_editing: ctx.egui_wants_keyboard_input(),
                ..craft_menu::Gate::default()
            });
            for command in menu.take_clicks(&ctx) {
                self.app.run_command(&command);
            }
        }
        self.app.ui(ui);
        #[cfg(target_os = "macos")]
        if let Some(menu) = &mut self.native_menu {
            let layout = self.app.menu_layout(craft_menu::Target::Mac);
            menu.sync(layout);
        }
        self.serve(&ctx);
        // The requests of this frame may have answers already (a write), so serve again.
        self.serve(&ctx);
        if let Some(title) = self.app.title_if_changed() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        self.sync_settings(Some(&ctx));
        self.autosave(Some(&ctx), false);
        self.dialogs.show_pending(Some(frame));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.autosave(None, true);
        self.sync_settings(None);
        if let Err(e) = self.settings.flush() {
            eprintln!("stochcraft: the settings could not be saved: {e}");
        }
    }
}

/// `[FILE] [--control PORT]`, in any order. PORT is 1 to 65535: 0 would let the system pick one,
/// whose number the app prints only to a console, which a Windows release build does not have.
fn arguments(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> Result<(Option<PathBuf>, Option<u16>), String> {
    let mut args = args.into_iter();
    let (mut file, mut port) = (None, None);
    let parse = |v: Option<std::ffi::OsString>| -> Result<u16, String> {
        v.and_then(|v| v.into_string().ok())
            .and_then(|v| v.parse().ok())
            .filter(|&p: &u16| p != 0)
            .ok_or_else(|| "--control takes a port number from 1 to 65535".to_owned())
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--control") => port = Some(parse(args.next())?),
            Some(a) if a.starts_with("--control=") => {
                port = Some(parse(Some(a.trim_start_matches("--control=").into()))?);
            }
            Some(a) if a.starts_with("--") => return Err(format!("unknown option {a}")),
            _ if file.is_none() => file = Some(PathBuf::from(arg)),
            _ => return Err("only one file can be opened at start".to_owned()),
        }
    }
    Ok((file, port))
}

fn main() -> eframe::Result {
    let (file, control_port) = match arguments(std::env::args_os().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("stochcraft: {e}\nusage: stochcraft [FILE] [--control PORT]");
            std::process::exit(2);
        }
    };
    // Who draws the frame on this desktop (STOCHCRAFT_TITLEBAR=app|system overrides it).
    let frame = craft_window::Choice::probe(craft_window::Prefix::new("STOCHCRAFT")).into_frame();
    let viewport = egui::ViewportBuilder::default()
        .with_title("StochCraft")
        .with_app_id("stochcraft")
        .with_inner_size([1100.0, 720.0])
        .with_min_inner_size([640.0, 400.0]);
    let options = eframe::NativeOptions {
        viewport: craft_window::decorate(viewport, &frame),
        ..Default::default()
    };
    let app_frame = frame.clone();
    eframe::run_native(
        "StochCraft",
        options,
        Box::new(move |cc| {
            let mut desktop = Desktop::new(&cc.egui_ctx, file, control_port);
            desktop.app.set_window_frame(&app_frame);
            Ok(Box::new(desktop))
        }),
    )
}

#[cfg(test)]
#[allow(clippy::panic)] // A settings file that doesn't read back fails the test.
mod tests {
    use super::*;

    #[test]
    fn a_file_and_a_control_port_in_any_order() {
        let a = |v: &[&str]| arguments(v.iter().map(std::ffi::OsString::from));
        assert_eq!(a(&[]), Ok((None, None)));
        assert_eq!(a(&["m.bpmn"]), Ok((Some("m.bpmn".into()), None)));
        assert_eq!(
            a(&["--control", "7070", "m.bpmn"]),
            Ok((Some("m.bpmn".into()), Some(7070)))
        );
        assert_eq!(a(&["--control=7071"]), Ok((None, Some(7071))));
        assert!(a(&["--control"]).is_err());
        assert!(a(&["--control", "x"]).is_err());
        assert!(a(&["--control", "0"]).is_err());
        assert!(a(&["--control=0"]).is_err());
        assert!(a(&["--nope"]).is_err());
        assert!(a(&["a.bpmn", "b.bpmn"]).is_err());
    }

    /// The preferences survive `config.toml`: appearance, recent files and the user's shortcuts.
    #[test]
    fn preferences_round_trip_through_the_settings_file() {
        let mut prefs = Prefs::default();
        prefs.files.recent = vec!["/models/a.bpmn".into()];
        prefs.keyboard.shortcuts = Some(serde_json::json!({ "model.run": "F6", "file.new": "" }));
        let mut doc = craft_settings::document::new_document(Stored::SCHEMA_VERSION);
        assert!(craft_settings::document::merge_into(&mut doc, &Stored(prefs.clone())).is_ok());
        let text = craft_settings::document::render(&doc);
        let back = craft_settings::document::parse(&text)
            .map(|d| craft_settings::document::deserialize_sections::<Stored>(&d));
        let Ok((Stored(read), problems)) = back else {
            panic!("the file didn't parse: {text}");
        };
        assert!(problems.is_empty(), "{text}");
        assert_eq!(read, prefs, "{text}");
    }
}
