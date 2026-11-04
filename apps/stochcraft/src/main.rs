//! StochCraft on the desktop: the shared interface ([`stochcraft_ui_egui::App`]) in a native
//! window, with the operating system's file dialogs (craft-dialogs, which never block the event
//! loop) and the file system.
//!
//! `stochcraft [FILE]` opens FILE at start.

use std::path::PathBuf;

use craft_dialogs::{Answer, Dialogs, FileDialog};
use stochcraft_ui_egui::{App, Platform, Request};

/// What a file dialog's answer is for.
enum Then {
    Open,
    Save,
}

struct Desktop {
    app: App,
    dialogs: Dialogs<Then>,
    /// Closing was confirmed (nothing unsaved, or the user chose).
    may_close: bool,
}

impl Desktop {
    fn new(ctx: &egui::Context, file: Option<PathBuf>) -> Self {
        let wake = ctx.clone();
        let mut app = App::new(Platform::DESKTOP);
        if let Some(path) = file {
            open(&mut app, &path);
        }
        Self {
            app,
            dialogs: Dialogs::new(std::sync::Arc::new(move || wake.request_repaint())),
            may_close: false,
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
                Request::PickSave { name } => self.dialogs.ask(
                    FileDialog::save()
                        .title("Save Model")
                        .file_name(name)
                        .filter("BPMN with BPSim", &["bpmn"]),
                    Then::Save,
                ),
                Request::Write { path, bytes } | Request::Download { name: path, bytes } => {
                    match std::fs::write(&path, &bytes) {
                        Ok(()) => self.app.written(&path, &bytes),
                        Err(e) => self.app.report_error(format!("{path}: {e}")),
                    }
                }
                Request::Quit => {
                    self.may_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
                (_, Answer::Unavailable(why)) => self.app.report_error(why),
                _ => {}
            }
        }
    }
}

fn open(app: &mut App, path: &std::path::Path) {
    let name = path.to_string_lossy();
    match std::fs::read(path) {
        Ok(bytes) => app.open_file(&name, &bytes),
        Err(e) => app.report_error(format!("{name}: {e}")),
    }
}

impl eframe::App for Desktop {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.viewport().close_requested()) && !self.may_close {
            if self.app.close_requested() {
                self.may_close = true;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        // A model file dropped on the window opens (after asking about unsaved changes).
        for file in ctx.input(|i| i.raw.dropped_files.clone()) {
            let name = file.path().to_string_lossy().into_owned();
            match file.bytes() {
                Ok(bytes) => self.app.drop_file(&name, bytes),
                Err(e) => self.app.report_error(format!("{name}: {e}")),
            }
        }
        self.app.ui(ui);
        self.serve(&ctx);
        // The requests of this frame may have answers already (a write), so serve again.
        self.serve(&ctx);
        if let Some(title) = self.app.title_if_changed() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
        self.dialogs.show_pending(Some(frame));
    }
}

fn main() -> eframe::Result {
    let file = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("StochCraft")
            .with_app_id("stochcraft")
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native(
        "StochCraft",
        options,
        Box::new(|cc| Ok(Box::new(Desktop::new(&cc.egui_ctx, file)))),
    )
}
