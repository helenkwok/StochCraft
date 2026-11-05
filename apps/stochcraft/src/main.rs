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
//! `stochcraft [FILE]` opens FILE at start.

use std::path::{Path, PathBuf};
use std::time::Instant;

use craft_dialogs::{Answer, Dialogs, FileDialog};
use craft_settings::{Location, Settings, SettingsStore};
use stochcraft_ui_egui::{App, Platform, Prefs, Request};

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
    /// The system menu bar (macOS).
    #[cfg(target_os = "macos")]
    native_menu: Option<craft_menu::NativeMenu>,
}

impl Desktop {
    fn new(ctx: &egui::Context, file: Option<PathBuf>) -> Self {
        let wake = ctx.clone();
        let mut app = App::new(Platform::DESKTOP);
        let (settings, report) = SettingsStore::<Stored>::open(Location::Auto);
        app.set_prefs(&settings.get().0);
        if report.has_problems() {
            app.report_error("the settings could not all be read; defaults are used where not");
        }
        if let Some(path) = file {
            open(&mut app, &path);
        }
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
            #[cfg(target_os = "macos")]
            native_menu,
        }
    }

    /// Carries out what the app asked for, and takes in the dialogs' answers.
    fn serve(&mut self, ctx: &egui::Context) {
        for request in self.app.take_requests() {
            match request {
                Request::PickOpen => self.dialogs.ask(
                    FileDialog::open()
                        .title("Open Model")
                        .filter("Process models with BPSim", &["bpmn", "xpdl", "xml"]),
                    Then::Open,
                ),
                Request::OpenPath { path } => open(&mut self.app, Path::new(&path)),
                Request::PickSave { name } => self.dialogs.ask(
                    FileDialog::save()
                        .title("Save Model")
                        .file_name(name)
                        .filter("BPMN with BPSim", &["bpmn"]),
                    Then::Save,
                ),
                Request::Write { path, bytes } | Request::Download { name: path, bytes } => {
                    match craft_atomic::atomic_write(Path::new(&path), &bytes) {
                        Ok(()) => self.app.written(&path, &bytes),
                        Err(e) => self.app.report_error(format!("{path}: {e}")),
                    }
                }
                Request::Export { name, bytes } => {
                    let ext = name.rsplit('.').next().unwrap_or("").to_owned();
                    let what = if ext == "png" {
                        "PNG image"
                    } else {
                        "CSV table"
                    };
                    self.dialogs.ask(
                        FileDialog::save()
                            .title("Export")
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
                (Then::Save, Answer::Paths(paths)) => {
                    if let Some(path) = paths.first() {
                        self.app.save_to(&path.to_string_lossy());
                    }
                }
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
            self.app
                .report_error(format!("the settings could not be saved: {e}"));
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
        self.dialogs.show_pending(Some(frame));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.sync_settings(None);
        if let Err(e) = self.settings.flush() {
            eprintln!("stochcraft: the settings could not be saved: {e}");
        }
    }
}

fn main() -> eframe::Result {
    let file = std::env::args_os().nth(1).map(PathBuf::from);
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
            let mut desktop = Desktop::new(&cc.egui_ctx, file);
            desktop.app.set_window_frame(&app_frame);
            Ok(Box::new(desktop))
        }),
    )
}
