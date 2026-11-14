//! View ▸ Appearance ▸ Chart colours…: the palettes for chart series, grouped by kind, the one used
//! for series, and the colour-vision and contrast check of the selected palette on the light and
//! dark canvases. Built-in palettes are read-only; a duplicate is the user's to change.
//!
//! A sibling of the Themes window rather than part of it: a theme is a pair of palettes for the
//! whole interface, while chart colours are one categorical palette plus several kinds of scale.

use egui::{Role, WidgetInfo};

use crate::App;
use crate::chart_palette::{self, ChartPalette, Level, PaletteKind};
use crate::i18n::{t, tf};
use crate::theme;
use crate::theme_editor::unique_name;

/// The Chart colours window's state while it is open.
#[derive(Debug, Default)]
pub(crate) struct Editor {
    /// The palette shown and edited.
    selected: String,
    /// The name being typed for the selected palette, while it is being renamed.
    rename: Option<String>,
}

impl App {
    /// Opens the Chart colours window, with the palette in use selected.
    pub(crate) fn open_chart_colours(&mut self) {
        if self.chart_colours.is_none() {
            self.chart_colours = Some(Editor {
                selected: self.active_chart_palette().name,
                ..Editor::default()
            });
        }
    }

    /// The palette series use: the one the preferences name, or the default.
    pub(crate) fn active_chart_palette(&self) -> ChartPalette {
        chart_palette::in_use(&self.chart_palette, &self.custom_palettes)
    }

    /// Draws the Chart colours window while it is open.
    pub(crate) fn chart_colours_window(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.chart_colours.take() else {
            return;
        };
        let theme = self.active_theme();
        let surfaces = [theme.light.canvas, theme.dark.canvas];
        let mut open = true;
        let (in_use, custom) = (&mut self.chart_palette, &mut self.custom_palettes);
        egui::Window::new(t("Chart colours"))
            .open(&mut open)
            .collapsible(false)
            .default_width(540.0)
            .show(ctx, |ui| {
                chart_colours_body(ui, &mut editor, in_use, custom, surfaces);
            });
        if open {
            self.chart_colours = Some(editor);
        }
    }
}

/// The window's contents: the palettes by kind, what can be done to the selected one, its colours,
/// and its check on each canvas.
fn chart_colours_body(
    ui: &mut egui::Ui,
    ed: &mut Editor,
    in_use: &mut String,
    custom: &mut Vec<ChartPalette>,
    surfaces: [egui::Color32; 2],
) {
    ui.label(theme::weak(
        ui,
        t("Built-in palettes are read-only. Duplicate one to change its colours."),
    ));
    ui.add_space(6.0);

    let kinds = [
        PaletteKind::Categorical,
        PaletteKind::Sequential,
        PaletteKind::Diverging,
    ];
    let names: Vec<String> = chart_palette::presets()
        .into_iter()
        .map(|p| p.name)
        .chain(custom.iter().map(|p| p.name.clone()))
        .collect();
    for kind in kinds {
        ui.add_space(4.0);
        ui.label(theme::strong(kind.label()).size(13.0));
        let all: Vec<ChartPalette> = chart_palette::presets()
            .into_iter()
            .chain(custom.iter().cloned())
            .filter(|p| p.kind == kind)
            .collect();
        for palette in &all {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(ed.selected == palette.name, &palette.name)
                    .clicked()
                {
                    ed.selected.clone_from(&palette.name);
                    ed.rename = None;
                }
                swatches(ui, &palette.colors);
                if kind == PaletteKind::Categorical && *in_use == palette.name {
                    ui.label(theme::weak(ui, t("In use")));
                }
            });
        }
    }

    let index = custom.iter().position(|p| p.name == ed.selected);
    let selected = chart_palette::find(&ed.selected, custom.as_slice());
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.add(theme::secondary(ui, t("Duplicate"))).clicked() {
            let copy = unique_name(&format!("{} copy", ed.selected), &names);
            let mut new = selected.clone();
            new.name.clone_from(&copy);
            custom.push(new);
            ed.selected = copy;
            ed.rename = None;
        }
        let categorical = selected.kind == PaletteKind::Categorical;
        let already = *in_use == selected.name;
        if ui
            .add_enabled(
                categorical && !already,
                theme::secondary(ui, t("Use for charts")),
            )
            .clicked()
        {
            in_use.clone_from(&selected.name);
        }
        if let Some(i) = index {
            if ui.add(theme::secondary(ui, t("Rename"))).clicked() {
                ed.rename = Some(ed.selected.clone());
            }
            if ui.add(theme::secondary(ui, t("Delete"))).clicked() {
                custom.remove(i);
                if *in_use == ed.selected {
                    *in_use = chart_palette::DEFAULT.to_owned();
                }
                ed.selected = chart_palette::DEFAULT.to_owned();
                ed.rename = None;
            }
        }
    });

    // Duplicating or deleting changed the list, so the selection's place is looked up again.
    let index = custom.iter().position(|p| p.name == ed.selected);

    // Renaming: kept only when the name is not empty and no other palette has it.
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
        if *in_use == old {
            in_use.clone_from(&new);
        }
        ed.selected = new;
        ed.rename = None;
    }

    // The colours: a custom palette's are edited in place; a built-in one's are read-only.
    let editable = index.is_some();
    let mut shown = chart_palette::find(&ed.selected, custom.as_slice());
    ui.add_space(10.0);
    ui.separator();
    let min = shown.kind.min_colours();
    if editable {
        if let Some(i) = index {
            let palette = &mut custom[i];
            ui.horizontal_wrapped(|ui| {
                for (n, colour) in palette.colors.iter_mut().enumerate() {
                    let label = slot_label(n);
                    ui.vertical(|ui| {
                        ui.label(theme::weak(ui, label.clone()));
                        // Opaque: palettes are saved as #rrggbb, with no alpha.
                        egui::color_picker::color_edit_button_srgba(
                            ui,
                            colour,
                            egui::color_picker::Alpha::Opaque,
                        )
                        .widget_info(|| WidgetInfo::labeled(Role::ColorWell, true, label.clone()));
                    });
                }
            });
            ui.horizontal(|ui| {
                if ui.add(theme::secondary(ui, t("Add colour"))).clicked() {
                    let last = palette
                        .colors
                        .last()
                        .copied()
                        .unwrap_or(egui::Color32::GRAY);
                    palette.colors.push(last);
                }
                let can_remove = palette.colors.len() > min;
                if ui
                    .add_enabled(can_remove, theme::secondary(ui, t("Remove colour")))
                    .clicked()
                {
                    palette.colors.pop();
                }
            });
            shown = palette.clone();
        }
    } else {
        ui.horizontal_wrapped(|ui| {
            for (n, colour) in shown.colors.iter().enumerate() {
                let label = slot_label(n);
                ui.vertical(|ui| {
                    ui.label(theme::weak(ui, label.clone()));
                    let mut c = *colour;
                    ui.add_enabled_ui(false, |ui| {
                        egui::color_picker::color_edit_button_srgba(
                            ui,
                            &mut c,
                            egui::color_picker::Alpha::Opaque,
                        )
                        .widget_info(|| WidgetInfo::labeled(Role::ColorWell, true, label.clone()));
                    });
                });
            }
        });
    }

    // The check: warnings and failures on each canvas, in words beside their colour.
    ui.add_space(8.0);
    ui.label(theme::strong(t("Checks")));
    let palette_ui = theme::palette(ui.ctx());
    for (label, surface) in [
        (t("On the light canvas"), surfaces[0]),
        (t("On the dark canvas"), surfaces[1]),
    ] {
        ui.label(theme::weak(ui, label));
        let findings = chart_palette::check(&shown, surface);
        if findings.is_empty() {
            ui.label(theme::weak(ui, t("All checks pass.")));
        }
        for finding in findings {
            let (word, colour) = match finding.level {
                Level::Fail => (t("Fail"), palette_ui.error),
                Level::Warn => (t("Warning"), palette_ui.warning),
            };
            ui.label(
                egui::RichText::new(tf(
                    "{word}: {message}",
                    &[("word", word), ("message", &finding.message)],
                ))
                .color(colour),
            );
        }
    }
}

/// The name of colour slot `n` (0-based), shown and read by screen readers.
fn slot_label(n: usize) -> String {
    tf("Colour {n}", &[("n", &(n + 1).to_string())])
}

/// A palette's colours as a row of small squares, so the list shows each palette at a glance.
fn swatches(ui: &mut egui::Ui, colours: &[egui::Color32]) {
    for colour in colours {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 14.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, *colour);
    }
}
