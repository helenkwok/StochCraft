//! View ▸ Appearance ▸ Edit themes…: the colour themes, each one's palettes and the contrast of
//! every palette. Built-in themes are read-only; a duplicate of one is the user's to change.

use egui::{Role, WidgetInfo};

use crate::App;
use crate::i18n::t;
use crate::theme::{self, Palette, Theme};

/// The Themes window's state while it is open.
#[derive(Debug, Default)]
pub(crate) struct Editor {
    /// The theme shown and edited.
    selected: String,
    /// Shows the dark palette rather than the light one.
    dark: bool,
    /// The name being typed for the selected theme, while it is being renamed.
    rename: Option<String>,
}

impl App {
    /// Opens the Themes window, with the theme in use selected.
    pub(crate) fn open_themes(&mut self) {
        if self.themes.is_none() {
            self.themes = Some(Editor {
                selected: self.active_theme().name,
                ..Editor::default()
            });
        }
    }

    /// The theme in use: the one the preferences name, or StochCraft's.
    pub(crate) fn active_theme(&self) -> Theme {
        theme::find(&self.colors, &self.custom_themes)
    }

    /// Draws the Themes window while it is open.
    pub(crate) fn themes_window(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.themes.take() else {
            return;
        };
        let mut open = true;
        let (colors, custom) = (&mut self.colors, &mut self.custom_themes);
        egui::Window::new(t("Themes"))
            .open(&mut open)
            .collapsible(false)
            .default_width(540.0)
            .show(ctx, |ui| themes_body(ui, &mut editor, colors, custom));
        if open {
            self.themes = Some(editor);
        }
    }
}

/// The window's contents: the themes, what can be done to the selected one, and its palette with
/// the contrast of each text pair.
fn themes_body(ui: &mut egui::Ui, ed: &mut Editor, colors: &mut String, custom: &mut Vec<Theme>) {
    ui.label(theme::weak(
        ui,
        t("Built-in themes are read-only. Duplicate one to change its colours."),
    ));
    ui.add_space(6.0);

    let names: Vec<String> = theme::presets()
        .into_iter()
        .map(|t| t.name)
        .chain(custom.iter().map(|t| t.name.clone()))
        .collect();
    for name in &names {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(ed.selected == *name, theme::shown_name(name))
                .clicked()
            {
                ed.selected.clone_from(name);
                ed.rename = None;
            }
            if *colors == *name {
                ui.label(theme::weak(ui, t("In use")));
            }
        });
    }

    let index = custom.iter().position(|t| t.name == ed.selected);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.add(theme::secondary(ui, t("Duplicate"))).clicked() {
            let copy = unique_name(&format!("{} copy", ed.selected), &names);
            let mut new = theme::find(&ed.selected, custom.as_slice());
            new.name.clone_from(&copy);
            custom.push(new);
            ed.selected = copy;
            ed.rename = None;
        }
        if let Some(i) = index {
            if ui.add(theme::secondary(ui, t("Rename"))).clicked() {
                ed.rename = Some(ed.selected.clone());
            }
            if ui.add(theme::secondary(ui, t("Delete"))).clicked() {
                custom.remove(i);
                if *colors == ed.selected {
                    *colors = theme::DEFAULT.to_owned();
                }
                ed.selected = theme::DEFAULT.to_owned();
                ed.rename = None;
            }
        }
    });

    // Duplicating or deleting changed the list, so the selection's place is looked up again.
    let index = custom.iter().position(|t| t.name == ed.selected);

    // Renaming: the name is kept only when it is not empty and no other theme has it.
    let mut renamed: Option<String> = None;
    let mut cancel = false;
    if let Some(i) = index
        && let Some(draft) = ed.rename.as_mut()
    {
        ui.add_space(4.0);
        ui.label(t("Name"));
        ui.add(egui::TextEdit::singleline(draft).desired_width(240.0))
            .widget_info(|| WidgetInfo::labeled(Role::TextInput, true, t("Name")));
        let new = draft.trim().to_owned();
        let free = !new.is_empty() && (new == custom[i].name || !names.contains(&new));
        if !free {
            ui.label(theme::weak(ui, t("The name is empty or already used.")));
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(free, theme::primary(ui, t("Save name")))
                .clicked()
            {
                renamed = Some(new.clone());
            }
            if ui.add(theme::secondary(ui, t("Cancel"))).clicked() {
                cancel = true;
            }
        });
    }
    if cancel {
        ed.rename = None;
    }
    if let (Some(i), Some(new)) = (index, renamed) {
        let old = std::mem::replace(&mut custom[i].name, new.clone());
        if *colors == old {
            *colors = new.clone();
        }
        ed.selected = new;
        ed.rename = None;
    }

    // The palette shown: the user's own is edited in place; a built-in one is read-only.
    ui.add_space(10.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ed.dark, false, t("Light"));
        ui.selectable_value(&mut ed.dark, true, t("Dark"));
    });
    let mut read_only = theme::find(&ed.selected, &[]);
    let editable = index.is_some();
    let pal: &mut Palette = match index {
        Some(i) => {
            if ed.dark {
                &mut custom[i].dark
            } else {
                &mut custom[i].light
            }
        }
        None => {
            if ed.dark {
                &mut read_only.dark
            } else {
                &mut read_only.light
            }
        }
    };
    egui::Grid::new("theme_roles")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (label, colour) in theme::roles(pal) {
                ui.label(label);
                ui.add_enabled_ui(editable, |ui| {
                    // Opaque: themes are saved as #rrggbb, with no alpha.
                    egui::color_picker::color_edit_button_srgba(
                        ui,
                        colour,
                        egui::color_picker::Alpha::Opaque,
                    )
                    .widget_info(|| WidgetInfo::labeled(Role::ColorWell, true, label));
                });
                ui.end_row();
            }
        });

    // Warnings only: nothing is blocked by a low contrast.
    ui.add_space(6.0);
    let found = theme::problems(pal);
    if found.is_empty() {
        ui.label(theme::weak(ui, t("All text meets the contrast minimums.")));
    } else {
        let warning = theme::palette(ui.ctx()).warning;
        for problem in found {
            ui.label(egui::RichText::new(problem).color(warning));
        }
    }
}

/// `base`, or `base` with the first number after it that no theme in `taken` has.
fn unique_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|n| n.as_str() == base) {
        return base.to_owned();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base} {n}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        n += 1;
    }
}
