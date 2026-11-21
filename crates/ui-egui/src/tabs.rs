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
        // A Monte Carlo model has its inputs and outputs, and its results: no process views.
        // A system dynamics model has them too.
        let monte_carlo = self.is_monte_carlo() || self.is_sd();
        ui.horizontal_wrapped(|ui| {
            // 0: the model, 1: the results, 2: the Analysis view, 3: the Capacity view, 4: the 3D view,
            // 5: the Layout view.
            let current = if self.show_results {
                1
            } else if self.show_analysis {
                2
            } else if self.show_3d {
                if self.layout_view.on { 5 } else { 4 }
            } else if self.show_capacity {
                3
            } else {
                0
            };
            let views = [
                (t("Model"), 0),
                (t("Results"), 1),
                (t("Analysis"), 2),
                (t("Capacity"), 3),
                (t("3D"), 4),
                (t("Layout"), 5),
            ];
            let shown = if monte_carlo { 2 } else { views.len() };
            for (label, view) in views.into_iter().take(shown) {
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
                    self.show_3d = view == 4 || view == 5;
                    self.layout_view.on = view == 5;
                }
            }
            if !monte_carlo {
                ui.toggle_value(&mut self.show_dashboard, t("Dashboard"));
            }
            if self.is_sd() {
                if self.sd_results().is_none() {
                    ui.label(theme::weak(ui, t("· no results yet")));
                }
            } else if self.results.is_none() && self.running.is_none() {
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

    /// Whether the model has anything to show: blocks, or a model being built.
    fn has_model(&self) -> bool {
        !self.engine.model().elements.is_empty()
            || self.building
            || self.is_monte_carlo()
            || self.is_sd()
    }

    /// The view the work area draws now, in the order [`App::work_area`] dispatches.
    fn view_shown(&self) -> View {
        if self.is_sd() {
            return if self.show_results {
                View::SdResults
            } else {
                View::SdModel
            };
        }
        if self.is_monte_carlo() {
            return if self.show_results && self.mc_results_shown() && self.running.is_none() {
                View::McResults
            } else if self.show_results || self.running.is_some() {
                // A run's progress, or the Results tab before the first run.
                View::Other
            } else {
                View::McModel
            };
        }
        let has_model = self.has_model();
        if self.show_results && self.results.is_some() && self.running.is_none() {
            View::Results
        } else if self.show_results
            && self.running.is_none()
            && !self.is_watching()
            && !self.engine.model().elements.is_empty()
        {
            // The Results tab before the first run: "No results yet", with its Run button. A run
            // being watched stays on the canvas, where it plays.
            View::Other
        } else if self.show_analysis && has_model && !self.show_results {
            View::Analysis
        } else if self.show_3d && has_model && !self.show_results && self.layout_view.on {
            View::Layout
        } else if self.show_3d && has_model && !self.show_results {
            View::ThreeD
        } else if self.show_capacity && has_model && !self.show_results {
            View::Capacity
        } else if has_model && !(self.show_results && self.running.is_some()) {
            View::Model
        } else {
            View::Other
        }
    }

    /// Whether the run bar is on screen: on the views that show the model (Model, 3D).
    pub(crate) fn run_bar_shown(&self) -> bool {
        matches!(self.view_shown(), View::Model | View::ThreeD)
    }

    /// The work area: the tabs, the run bar over the views that show the model, then the view
    /// the tabs select (or the start page, or a run's progress).
    pub(crate) fn work_area(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        // Set again only if the Results view draws the charts this frame, so a charts export
        // never crops by where they were on a view no longer shown.
        self.charts_rect = None;
        // Here rather than at the top of the frame: the header's menus and the outline, drawn
        // before the work area, can change the model (Edit ▸ Undo, + Add block).
        self.mark_stale_results();
        let has_model = self.has_model();
        if !has_model && self.results.is_none() && self.running.is_none() {
            self.start_page(ui);
            return;
        }
        if has_model || self.results.is_some() || self.running.is_some() {
            self.tabs(ui, p);
            ui.add_space(14.0);
        }
        let view = self.view_shown();
        if matches!(view, View::Model | View::ThreeD) {
            self.watch_bar(ui, p);
        }
        // A run in progress shows its progress, not the results it will replace.
        match view {
            View::Results => self.results_view(ui),
            View::Analysis => self.analysis_view(ui),
            View::Layout => self.layout_view(ui),
            View::ThreeD => self.view3d_view(ui),
            View::Capacity => self.capacity_view(ui),
            View::Model => self.model_view(ui, p),
            View::McModel => self.mc_model_view(ui),
            View::McResults => self.mc_results_view(ui),
            View::SdModel => self.sd_model_view(ui),
            View::SdResults => self.sd_results_view(ui),
            View::Other => {
                // A readable column, not the full width of a wide window.
                let size = egui::vec2(ui.available_width().min(640.0), ui.available_height());
                ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| {
                    if let Some(r) = &self.running {
                        let (done, partial, total) = (r.done, r.partial, r.total);
                        self.running_view(ui, done, partial, total);
                    } else if self.is_monte_carlo() {
                        self.mc_ready(ui);
                    } else if self.engine.model().elements.is_empty() {
                        self.start_page(ui);
                    } else {
                        self.ready(ui);
                    }
                });
            }
        }
    }
}

/// What the work area shows.
enum View {
    Results,
    Analysis,
    ThreeD,
    /// The plan of the floor (M5.5 L1); it does not move with a run, so it has no run bar.
    Layout,
    Capacity,
    Model,
    /// A Monte Carlo model's inputs, outputs and settings.
    McModel,
    /// A Monte Carlo run's results.
    McResults,
    /// A system dynamics model's settings and variables.
    SdModel,
    /// A system dynamics model's run.
    SdResults,
    /// A run's progress, the "ready" page (the Results tab with no results) or the start page.
    Other,
}
