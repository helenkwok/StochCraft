//! The window's chrome: the per-frame `App::ui`, the header with its menus and Run, the status
//! bar, keyboard shortcuts, the dialogs (unsaved changes, new file, About, Keyboard Shortcuts) and
//! the Help panel.

use egui::{Key, Modifiers};

use crate::i18n::{t, td, tf, tn};
use crate::{App, Request, chart_palette, commands, guide, help, i18n, results, theme};

impl App {
    /// Opens or closes the Help panel.
    pub fn show_help(&mut self, open: bool) {
        self.help.open = open;
    }

    /// Whether the Help panel is open.
    pub fn help_open(&self) -> bool {
        self.help.open
    }

    /// Opens the Help panel at `section`.
    pub(crate) fn show_help_at(&mut self, section: help::Section) {
        self.help.open = true;
        // A search would hide the section asked for.
        self.help.search.clear();
        self.help.scroll_to = Some(section);
    }

    /// Asks the shell to open `url` in the web browser.
    pub(crate) fn open_url(&mut self, url: &str) {
        self.requests.push(Request::OpenUrl {
            url: url.to_owned(),
        });
    }

    /// The Help panel, at the right of the window.
    fn help_panel(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let model = self.wire_model();
        let steps = help::steps(&model, self.results().is_some(), self.help.read_results);
        let keymap_platform = craft_keymap::egui::platform(ui.ctx());
        let key = |id: &str| commands::shortcut_text(&self.keymap, id, keymap_platform);
        let mut keys: Vec<(String, &'static str)> = [
            (commands::RUN, "Run"),
            (commands::UNDO, "Undo"),
            (commands::REDO, "Redo"),
            (commands::SAVE, "Save"),
            (commands::OPEN, "Open…"),
            (commands::HELP, "Help"),
        ]
        .into_iter()
        .filter_map(|(id, what)| key(id).map(|k| (k, what)))
        .collect();
        keys.push((t("Space").to_owned(), "Play or pause a watched run"));
        for (id, what) in [
            (commands::STEP_WATCHING, "Step"),
            (commands::HALVE_SPEED, "Halve Speed"),
            (commands::DOUBLE_SPEED, "Double Speed"),
        ] {
            if let Some(k) = key(id) {
                keys.push((k, what));
            }
        }
        let action = egui::Panel::right("help")
            .resizable(true)
            .default_size(360.0)
            .size_range(280.0..=520.0)
            .frame(
                egui::Frame::new()
                    .fill(p.raised)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .inner_margin(egui::Margin::symmetric(18, 14)),
            )
            .show(ui, |ui| help::show(ui, &mut self.help, &steps, &keys))
            .inner;
        match action {
            Some(help::Action::Close) => self.help.open = false,
            Some(help::Action::Open(url)) => self.open_url(url),
            Some(help::Action::ChangeKeys) => self.open_shortcuts(),
            Some(help::Action::ShowMe) => self.start_guide(),
            None => {}
        }
    }

    /// The window title: the file's name, marked when there are unsaved changes.
    pub fn title(&self) -> String {
        let name = self.document_name();
        let dirty = if self.unsaved_changes() { " •" } else { "" };
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

    /// A dialog is open (menu commands wait).
    pub fn has_open_dialog(&self) -> bool {
        self.modal_open()
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
        if self.shares_progress().is_some() && ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.cancel_shares();
        }
        // Esc stops the guide first, before it would clear the selection the guide made (the
        // block whose connect dot it points at), unless a field is being typed in or a run goes.
        if self.guide.on
            && !ctx.egui_wants_keyboard_input()
            && self.running.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.guide.on = false;
        }
        // The block list from the keyboard, when no field is being edited.
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing && !self.show_results {
            // The run bar's Space: play or pause (or start watching), where the bar is.
            if (self.is_watching() || self.run_bar_shown())
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space))
            {
                self.run_or_pause();
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
        let theme = self.active_theme();
        let canvases = [theme.light.canvas, theme.dark.canvas];
        theme::use_palettes(&ctx, &theme);
        if self.styled.as_ref() != Some(&theme) {
            if self.styled.is_none() {
                theme::install(&ctx, &theme);
            } else {
                theme::apply(&ctx, &theme);
            }
            self.styled = Some(theme);
        }
        if self.poll() {
            ctx.request_repaint();
        }
        self.take_screenshot();
        if self.applied_theme != Some(self.theme_choice) {
            ctx.set_theme(self.theme_choice.preference());
            self.applied_theme = Some(self.theme_choice);
        }
        // After the theme, so the order follows the canvas charts are drawn on now.
        let canvas = canvases[usize::from(ctx.theme() == egui::Theme::Dark)];
        chart_palette::use_palette(&ctx, &self.active_chart_palette(), canvas);
        self.shortcuts(&ctx);
        self.tick_watching(&ctx);
        self.guide.targets = guide::Targets::default();
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
        if self.help.open {
            self.help_panel(ui, &p);
        }
        // The dashboard reads a process model's results; a Monte Carlo model has none.
        if self.show_dashboard && !self.is_monte_carlo() && !self.is_sd() {
            self.dashboard_panel(ui, &p);
        }
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
            .show(ui, |ui| self.work_area(ui, &p));

        self.confirm_dialog(&ctx);
        self.new_file_dialog(&ctx);
        self.about_window(&ctx);
        self.shortcuts_window(&ctx);
        self.themes_window(&ctx);
        self.chart_colours_window(&ctx);
        self.recovery_dialog(&ctx);
        self.guide_overlay(&ctx);
        // Once everything of this frame is laid out, including what is drawn over the charts.
        self.check_pending_png(&ctx);
        // A system dynamics run is made a few replications a frame; keep the frames coming.
        if self.sd_running() {
            ctx.request_repaint();
        }
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
        // A focused item takes egui's `active` look: lit like an open one, never the soft accent,
        // which header text can't be read on (theme::problems checks the pair). Its outline is
        // drawn below, once the header's widgets are laid out.
        let v = ui.visuals_mut();
        v.override_text_color = Some(p.on_shell);
        v.widgets.hovered.weak_bg_fill = theme::header_hover(p);
        v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
        v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        v.widgets.open.weak_bg_fill = theme::header_lit(p);
        v.widgets.active.weak_bg_fill = theme::header_lit(p);
        v.widgets.active.bg_fill = theme::header_lit(p);
        v.widgets.active.bg_stroke = egui::Stroke::NONE;
        let header = ui.max_rect();
        let running = self.running.is_some();
        let can_run =
            !self.engine.model().elements.is_empty() || self.is_monte_carlo() || self.is_sd();
        // With errors, Run says how many; clicking it shows them one by one rather than run.
        // (A system dynamics model has no process model to find problems in.)
        let errors = if self.is_sd() {
            0
        } else {
            self.problems().iter().filter(|p| p.error).count() as u64
        };
        let run_label = if errors > 0 {
            tn(errors, "Run ({n} problem)", "Run ({n} problems)")
        } else {
            t("Run").to_owned()
        };
        let mut run_rect = None;
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
                let shown = if self.unsaved_changes() {
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
                    let r = ui
                        .add_enabled(can_run, theme::primary(ui, &run_label))
                        .on_hover_text(tf(
                            "Run the experiment ({shortcut})",
                            &[("shortcut", &shortcut)],
                        ))
                        .on_disabled_hover_text(t("Open a model first"));
                    run_rect = Some(r.rect);
                    r.clicked()
                }
            },
        );
        self.guide.targets.run = run_rect;
        // The keyboard's place in the header: a ring in the header's text colour around the
        // focused menu title or button (WCAG 2.4.7). The menu bar draws its titles without one.
        if let Some(focused) = ui.memory(|m| m.focused())
            && let Some(r) = ui.ctx().read_response(focused)
            && header.contains_rect(r.rect)
        {
            ui.painter().rect_stroke(
                r.rect.expand(1.0),
                6.0,
                egui::Stroke::new(1.5, p.on_shell),
                egui::StrokeKind::Outside,
            );
        }
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
            if let Some(progress) = self.shares_progress() {
                // The shares of a Monte Carlo output's spread, worked out beside the results.
                ui.label(theme::weak(
                    ui,
                    tf(
                        "Working out the shares of the spread: {percent}",
                        &[("percent", &crate::montecarlo::share_text(progress))],
                    ),
                ));
                if ui
                    .add(egui::Button::new(t("Cancel")).small())
                    .on_hover_text("Esc")
                    .clicked()
                {
                    self.cancel_shares();
                }
            } else if self.status_error {
                ui.label(theme::strong(t("Error")).color(p.error));
                let text = egui::RichText::new(&self.status).color(p.error);
                match self.status_at.clone() {
                    // An error with its place: a click (or Enter) shows that block.
                    Some(at) => {
                        let name = self.block_name(&at);
                        if ui
                            .add(egui::Button::new(text.underline()).frame(false))
                            .on_hover_text(tf("Show {id}", &[("id", &name)]))
                            .clicked()
                        {
                            self.show_block(Some(at));
                        }
                    }
                    None => {
                        ui.label(text);
                    }
                }
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
        // The model was replaced while the question was open (an agent's command): its answer
        // would write the new model into the file asked about, so the question and what was to
        // follow the save are dropped.
        if pending.for_doc != self.doc_gen {
            self.new_file = None;
            self.after_save = None;
            self.save_failed(t(
                "The model changed while choosing where to save it, so it was not saved. Save again.",
            ));
            return;
        }
        let name = self.document_name();
        let sd = self.is_sd();
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("new file")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading(if sd {
                t("Replace the XMILE file?")
            } else {
                t("Save as a new BPMN file?")
            });
            ui.add_space(4.0);
            if let Some(reason) = &pending.reason {
                ui.label(reason);
                ui.add_space(4.0);
            }
            ui.label(theme::weak(
                ui,
                if sd {
                    tf(
                        "Save As writes a new file and keeps {file} as it is. Replacing {file} leaves out what StochCraft does not keep.",
                        &[("file", &name)],
                    )
                } else {
                    tf(
                        "A new file holds the model alone: what StochCraft does not understand in {file} is left out.",
                        &[("file", &name)],
                    )
                },
            ));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if sd {
                    // Save As keeps the original; the primary choice replaces it.
                    if ui.add(theme::secondary(ui, t("Save As…"))).clicked() {
                        choice = Some(Choice::Discard);
                    }
                }
                let go = match (&pending.path, self.platform.file_system) {
                    (Some(_), _) if sd => t("Replace"),
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
        // A system dynamics model's question: replace the opened file (Save), or choose a new
        // one (Save As, answered as Discard here). Nothing is written before the answer.
        if sd {
            match choice {
                Some(Choice::Save) => {
                    self.new_file = None;
                    if let Some(path) = pending.path {
                        self.write_sd(path);
                    }
                }
                Some(Choice::Discard) => {
                    self.new_file = None;
                    let name = self.suggested_name();
                    self.pick_save(name, false);
                }
                Some(Choice::Cancel) => {
                    self.new_file = None;
                    self.after_save = None;
                }
                None => {}
            }
            return;
        }
        match choice {
            Some(Choice::Save) => {
                self.new_file = None;
                let bpmn = format!("{}.bpmn", self.export_stem());
                match pending.path {
                    Some(path) => self.write_new(path),
                    None if self.platform.file_system => {
                        self.as_new = true;
                        self.pick_save(bpmn, false);
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
            // A short window (a small browser window) scrolls to the licence link at the end.
            .vscroll(true)
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
                ui.add_space(10.0);
                ui.label(theme::strong(t("Fonts")));
                for (names, licence) in FONTS {
                    // Font names and the SPDX licence: data, laid out to wrap, as the longer
                    // names do not fit the credits' two columns.
                    ui.horizontal_wrapped(|ui| {
                        ui.label(td(names));
                        ui.label(theme::weak(ui, td(licence)));
                    });
                }
                ui.add_space(10.0);
                // The list the build makes of every Rust crate with its licence text: a file
                // in the download, a page beside the app on the web.
                #[cfg(not(target_arch = "wasm32"))]
                ui.label(t(
                    "The full list of third-party licences is in the file THIRD-PARTY-LICENSES.html, which comes with the download.",
                ));
                // In a new tab: following it in this one would leave the app, and the model.
                #[cfg(target_arch = "wasm32")]
                ui.add(
                    egui::Hyperlink::from_label_and_url(
                        t("Full list of third-party licences"),
                        "THIRD-PARTY-LICENSES.html",
                    )
                    .open_in_new_tab(true),
                );
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

/// The main libraries the app is built on, for the About box. Every dependency's licence is
/// checked by `cargo deny` against the allowed list in `deny.toml`.
const CREDITS: &[(&str, &str)] = &[
    ("egui and eframe (Craft suite fork)", "MIT OR Apache-2.0"),
    ("craft-libs", "MIT OR Apache-2.0"),
    ("serde, serde_json", "MIT OR Apache-2.0"),
    ("libm", "MIT OR Apache-2.0"),
    ("sha2", "MIT OR Apache-2.0"),
];

/// The fonts the app carries, for the About box (ATTRIBUTION.md, Assets): IBM Plex, and egui's
/// Hack and Noto Emoji for the symbols Plex lacks, are embedded everywhere; the browser app also
/// serves the craft-fonts faces beside it. The desktop release builds embed none of the others
/// (no `CRAFT_FONTS_DIR`).
const FONTS: &[(&str, &str)] = &[
    ("IBM Plex Sans, IBM Plex Mono", "OFL-1.1"),
    // Hack-Regular.txt in egui's epaint_default_fonts: Hack is MIT, built on Bitstream Vera.
    ("Hack", "MIT AND Bitstream-Vera"),
    ("Noto Emoji", "OFL-1.1"),
    #[cfg(target_arch = "wasm32")]
    ("BIZ UDPGothic, Noto Sans CJK SC, NanumGothic", "OFL-1.1"),
    #[cfg(target_arch = "wasm32")]
    (
        "Noto Sans Arabic, Thai, Georgian, Hebrew, Devanagari, Symbols, Symbols 2",
        "OFL-1.1",
    ),
];

#[derive(Clone, Copy)]
enum Choice {
    Save,
    Discard,
    Cancel,
}
