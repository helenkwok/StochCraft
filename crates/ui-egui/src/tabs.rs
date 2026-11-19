//! The work area: the view tabs (Model, Results, Analysis, Capacity, 3D) and the dispatch to the
//! view the current tab selects.

use crate::i18n::t;
use crate::{App, theme};

impl App {
    /// The work area's two views: the model (overview or inspector) and the results.
    fn tabs(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        // craft-ui's Tab: an accessible tab (role, keyboard focus) the app lays out and paints.
        let font = egui::FontId::new(14.0, egui::FontFamily::Name(theme::STRONG.into()));
        // Wrapped: four tabs and the results note must not widen the work area past the window.
        ui.horizontal_wrapped(|ui| {
            // 0: the model, 1: the results, 2: the Analysis view, 3: the Capacity view, 4: the 3D view.
            let current = if self.show_results {
                1
            } else if self.show_analysis {
                2
            } else if self.show_3d {
                4
            } else if self.show_capacity {
                3
            } else {
                0
            };
            for (label, view) in [
                (t("Model"), 0),
                (t("Results"), 1),
                (t("Analysis"), 2),
                (t("Capacity"), 3),
                (t("3D"), 4),
            ] {
                let on = current == view;
                let width = ui.fonts_mut(|f| {
                    f.layout_no_wrap(label.into(), font.clone(), p.text)
                        .size()
                        .x
                }) + 24.0;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::hover());
                if view == 1 {
                    self.guide.targets.results_tab = Some(rect);
                }
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
                    self.show_capacity = view == 3;
                    self.show_3d = view == 4;
                }
            }
            ui.toggle_value(&mut self.show_dashboard, t("Dashboard"));
            if self.results.is_none() && self.running.is_none() {
                ui.label(theme::weak(ui, t("· no results yet")));
            } else if self.results_stale && self.running.is_none() {
                ui.label(theme::weak(ui, t("· out of date")));
            }
        });
        let rect = ui.max_rect();
        let y = ui.cursor().top() + 4.0;
        ui.painter()
            .hline(rect.x_range(), y, egui::Stroke::new(1.0, p.border));
    }

    /// The work area: the tabs, then the view they select (or the start page, or a run's progress).
    pub(crate) fn work_area(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let has_model = !self.engine.model().elements.is_empty() || self.building;
        if !has_model && self.results.is_none() && self.running.is_none() {
            self.start_page(ui);
            return;
        }
        if has_model || self.results.is_some() || self.running.is_some() {
            self.tabs(ui, p);
            ui.add_space(14.0);
        }
        // A run in progress shows its progress, not the results it will replace.
        if self.show_results && self.results.is_some() && self.running.is_none() {
            self.results_view(ui);
        } else if self.show_analysis && has_model && !self.show_results {
            self.analysis_view(ui);
        } else if self.show_3d && has_model && !self.show_results {
            self.view3d_view(ui);
        } else if self.show_capacity && has_model && !self.show_results {
            self.capacity_view(ui);
        } else if has_model && !(self.show_results && self.running.is_some()) {
            self.model_view(ui, p);
        } else {
            // A readable column, not the full width of a wide window.
            let size = egui::vec2(ui.available_width().min(640.0), ui.available_height());
            ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
                if let Some(r) = &self.running {
                    let (done, partial, total) = (r.done, r.partial, r.total);
                    self.running_view(ui, done, partial, total);
                } else if self.engine.model().elements.is_empty() {
                    self.start_page(ui);
                } else {
                    self.ready(ui);
                }
            });
        }
    }
}
