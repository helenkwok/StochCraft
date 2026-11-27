//! The views the work area shows (the model flowchart, the results, the outline, the start page, a
//! run's progress) and the first-model guide.

use serde_json::{Value, json};

use crate::i18n::{t, tf, tnf};
use crate::{
    App, EXAMPLES, Replace, baseline, charts, commands, editor, file_name, guide, help, kind_text,
    results, start, theme, time_unit,
};

impl App {
    /// Starts the guide at the first step not done.
    pub(crate) fn start_guide(&mut self) {
        // From an empty window (Show me in Help over the start page), the canvas and its palette
        // replace the start page, so the first step's chip is there to point at.
        if self.engine.model().elements.is_empty() {
            self.building = true;
        }
        self.guide.on = true;
        self.guide.last = None;
    }

    /// The guide's ring and callout, drawn over everything once the controls it points at are.
    pub(crate) fn guide_overlay(&mut self, ctx: &egui::Context) {
        if !self.guide.on {
            return;
        }
        let model = self.wire_model();
        let ran = self.results().is_some();
        let hint = guide::hint(&model, ran, self.help.read_results);
        let target = hint.as_ref().map(|h| h.target.clone());
        let fresh = self.guide.last != target;
        if fresh {
            match &target {
                // A dot shows on the selected block; a chip or a dot is on the model view.
                Some(guide::Target::Handle(id)) => {
                    self.select(std::collections::BTreeSet::from([id.clone()]));
                    self.show_results = false;
                    self.show_analysis = false;
                    self.show_capacity = false;
                    self.show_3d = false;
                }
                Some(guide::Target::Chip(_)) => {
                    self.show_results = false;
                    self.show_analysis = false;
                    self.show_capacity = false;
                    self.show_3d = false;
                }
                _ => {}
            }
            self.guide.last = target;
            ctx.request_repaint();
        }
        self.guide.fresh = fresh;
        let steps = help::steps(&model, ran, self.help.read_results).len();
        if guide::show(ctx, &self.guide, hint.as_ref(), steps) == Some(guide::Action::Stop) {
            self.guide.on = false;
        }
    }

    /// The model: the flowchart, with the inspector beside it (below it, in a narrow window).
    pub(crate) fn model_view(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        // Side by side only where the canvas and its toolbar keep about 400 px beside the
        // inspector (it is narrower with Help open); otherwise the inspector goes below.
        let wide = ui.available_width() >= 760.0;
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
        self.problems_chip(ui, p);
        self.canvas(ui, p);
    }

    /// The model's errors above the canvas, with the blocks they are on ("2 problems: Paint,
    /// Capacity"); a click shows the next, as Run does. Nothing when there are none.
    fn problems_chip(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let errors: Vec<editor::Problem> =
            self.problems().into_iter().filter(|p| p.error).collect();
        if errors.is_empty() {
            return;
        }
        let mut names: Vec<String> = Vec::new();
        for pr in &errors {
            let name = match self.block_of(pr.at.as_deref()) {
                Some(b) => self.block_name(&b),
                None => t("Model").to_owned(),
            };
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let text = tnf(
            errors.len() as u64,
            "{n} problem: {blocks}",
            "{n} problems: {blocks}",
            &[("blocks", &names.join(", "))],
        );
        let clicked = ui
            .horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                theme::paint_error_mark(ui.painter(), rect.center(), 6.5, p);
                ui.add(
                    egui::Button::new(egui::RichText::new(text).color(p.error))
                        .stroke(egui::Stroke::new(1.0, p.error))
                        .corner_radius(12.0),
                )
                .clicked()
            })
            .inner;
        ui.add_space(8.0);
        if clicked {
            self.go_to_problem();
        }
    }

    /// The block an engine error's details place it on: the first error of its diagnostics, or
    /// the element a run stopped at.
    pub(crate) fn error_block(&self, details: &Value) -> Option<String> {
        let at = match details.as_array() {
            Some(ds) => ds
                .iter()
                .filter(|d| d["severity"] == "error")
                .find_map(|d| d["at"].as_str()),
            None => details["at"].as_str(),
        };
        self.block_of(at)
    }

    /// A block's name, or its id when it has none.
    pub(crate) fn block_name(&self, id: &str) -> String {
        self.engine
            .model()
            .elements
            .iter()
            .find(|e| e.id.as_str() == id && !e.name.is_empty())
            .map_or_else(|| id.to_owned(), |e| e.name.clone())
    }

    /// The tasks of a model that is one straight line, in flow order: one arrivals block, one
    /// departures block, and between them tasks each with one way in and one way out, no
    /// decisions, splits or batches. `None` for any other model, where the time an entity spends
    /// is not the sum of each task's.
    fn straight_line(&self) -> Option<Vec<String>> {
        let model = self.engine.model();
        let kind = |id: &str| {
            model
                .elements
                .iter()
                .find(|e| e.id.as_str() == id)
                .map(|e| e.kind.name())
        };
        let sources: Vec<_> = model
            .elements
            .iter()
            .filter(|e| e.kind.name() == "source")
            .collect();
        let sinks = model
            .elements
            .iter()
            .filter(|e| e.kind.name() == "sink")
            .count();
        if sources.len() != 1 || sinks != 1 {
            return None;
        }
        let mut line = Vec::new();
        let mut at = sources[0].id.as_str().to_owned();
        for _ in 0..=model.elements.len() {
            let out: Vec<_> = model
                .connections
                .iter()
                .filter(|c| c.from.as_str() == at)
                .collect();
            let [next] = out.as_slice() else {
                return None;
            };
            let to = next.to.as_str();
            if model
                .connections
                .iter()
                .filter(|c| c.to.as_str() == to)
                .count()
                != 1
            {
                return None;
            }
            match kind(to)? {
                "sink" => return Some(line),
                "server" | "shared_task" | "delay" => {
                    line.push(to.to_owned());
                    at = to.to_owned();
                }
                _ => return None,
            }
        }
        None
    }

    /// The results: export buttons, the charts, then the table.
    pub(crate) fn results_view(&mut self, ui: &mut egui::Ui) {
        let Some(results) = self.results.clone() else {
            return;
        };
        let mut csv = false;
        let mut png = false;
        let mut fix = None;
        let mut again = false;
        let mut baseline_action = None;
        let compared = self.compared();
        if !self.results_stale {
            self.help.read_results = true;
        }
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
                        if ui.add(theme::secondary(ui, charts)).clicked() {
                            self.png_scroll = 4;
                        }
                        csv = ui.add(theme::secondary(ui, table)).clicked();
                    });
                });
                results::record_line(ui, &results);
                self.stop_line(ui, &results);
                ui.add_space(6.0);
                if self.results_stale {
                    again = self.out_of_date_line(ui);
                    ui.add_space(6.0);
                }
                baseline_action = self.baseline_line(ui);
                ui.add_space(6.0);
                if let Some((before, now, comparison, changes)) = &compared {
                    baseline::show(ui, before, now, comparison, changes);
                    ui.add_space(12.0);
                }
                results::findings(ui, &results);
                ui.add_space(6.0);
                fix = results::trust(ui, &results);
                ui.add_space(10.0);
                results::key(ui, &results);
                ui.add_space(6.0);
                // The line's blocks as the model has them now: only while the results are of it.
                let line = self.straight_line().filter(|_| !self.results_stale);
                self.charts_rect = charts::show(ui, &results, line.as_deref());
                // An export asked for: scroll the charts into view, and capture them once they
                // are all on screen. If they are still not after a few frames (a window too
                // short for them), nothing is exported: a capture would hold part of them, or
                // what is drawn over them.
                if self.png_scroll > 0 {
                    if let Some(area) = self.charts_rect {
                        if ui.clip_rect().contains_rect(area) {
                            self.png_scroll = 0;
                            png = true;
                        } else if self.png_scroll == 1 {
                            self.png_scroll = 0;
                            self.report_error(t(
                                "The charts are taller than the window: make the window taller, then export them again.",
                            ));
                        } else {
                            // At once, not animated, so the charts are in place by the next
                            // frame rather than part way through a scroll.
                            ui.scroll_to_rect_animation(
                                area,
                                Some(egui::Align::TOP),
                                egui::style::ScrollAnimation::none(),
                            );
                            self.png_scroll -= 1;
                            ui.ctx().request_repaint();
                        }
                    } else {
                        self.png_scroll = 0;
                        png = true;
                    }
                }
                ui.add_space(6.0);
                results::show(ui, &results);
                self.distribution_section(ui, &results);
            });
        // A fix from "How sure are these numbers?": made in the model as one undoable step, then
        // the experiment runs again.
        if let Some(fix) = fix {
            let params = match fix {
                results::Fix::Runs(n) => json!({ "replications": n }),
                results::Fix::Warmup(w) => json!({ "warmup": w }),
                results::Fix::Max(n) => {
                    // The model's sequential settings with the new most; the defaults if the
                    // model has none now.
                    let mut q = self
                        .engine
                        .model()
                        .experiment
                        .sequential
                        .as_ref()
                        .map_or_else(
                            || json!({}),
                            |x| json!(stochcraft_engine::wire::SequentialSpec::from(x)),
                        );
                    q["max"] = json!(n);
                    json!({ "sequential": q })
                }
            };
            if self.execute("experiment.set", params).is_some() {
                self.run();
            }
        }
        if again {
            self.run();
        }
        match baseline_action {
            Some(true) => self.keep_baseline(),
            Some(false) => self.clear_baseline(),
            None => {}
        }
        if csv {
            self.export_csv();
        }
        if png {
            let ctx = ui.ctx().clone();
            self.export_png(&ctx);
        }
    }

    /// The line over results whose model has changed since their run: they stay on show, marked,
    /// with Run again. Returns whether Run again was clicked.
    pub(crate) fn out_of_date_line(&self, ui: &mut egui::Ui) -> bool {
        let p = theme::palette(ui.ctx());
        let mut again = false;
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, p.warning))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
                    theme::paint_warning_mark(ui.painter(), rect.center(), 6.5, &p);
                    ui.label(theme::strong(t("Out of date:")));
                    ui.label(t("the model has changed since this run."));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        again = ui.add(theme::primary(ui, t("Run again"))).clicked();
                    });
                });
            });
        again
    }

    /// The baseline's line under the run record: Keep as baseline, or which run is the baseline,
    /// with Clear baseline. Returns `Some(true)` to keep these results as the baseline,
    /// `Some(false)` to clear it.
    fn baseline_line(&self, ui: &mut egui::Ui) -> Option<bool> {
        let mut action = None;
        ui.horizontal_wrapped(|ui| {
            match &self.baseline {
                Some(b) if b.serial == self.results_serial => {
                    ui.label(theme::strong(t("This run is the baseline.")));
                    ui.label(theme::weak(
                        ui,
                        t("Change the model and run it again to compare."),
                    ));
                }
                Some(b) => {
                    let r = &b.results["record"];
                    ui.label(theme::strong(t("Compared with the baseline:")));
                    ui.label(theme::weak(
                        ui,
                        tnf(
                            r["replications"].as_u64().unwrap_or(0),
                            "{n} run · seed {seed}",
                            "{n} runs · seed {seed}",
                            &[("seed", &r["seed"].to_string())],
                        ),
                    ));
                    if ui
                        .add(theme::secondary(ui, t("Keep this as the baseline")))
                        .clicked()
                    {
                        action = Some(true);
                    }
                }
                None => {
                    if ui
                        .add(theme::secondary(ui, t("Keep as baseline")))
                        .on_hover_text(t(
                            "Keep these results, change the model, run it again, and see what the change did.",
                        ))
                        .clicked()
                    {
                        action = Some(true);
                    }
                }
            }
            if self.baseline.is_some() && ui.link(t("Clear baseline")).clicked() {
                action = Some(false);
            }
        });
        action
    }

    /// The model as its process: each element as its BPMN shape on one line from arrival to
    /// departure, then the experiment it runs.
    pub(crate) fn outline(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        if self.is_monte_carlo() {
            self.mc_outline(ui, p);
            return;
        }
        if self.is_sd() {
            self.sd_outline(ui, p);
            return;
        }
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
                                self.show_capacity = false;
                                self.show_3d = false;
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
            self.show_capacity = false;
            self.show_3d = false;
        }
        // A copy, as the scenario choice below may reopen the model.
        let x = self.engine.model().experiment.clone();
        let unit = time_unit(&self.engine);
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
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
            // Laid out from the bottom: this line shows under the runs.
            if let Some(q) = &x.sequential {
                ui.label(theme::weak(
                    ui,
                    tf(
                        "then more until precise enough, at most {max}",
                        &[("max", &q.max.to_string())],
                    ),
                ));
            }
            ui.label(tnf(
                u64::from(x.replications),
                "{n} run · seed {seed}",
                "{n} runs · seed {seed}",
                &[("seed", &x.seed.to_string())],
            ));
            // Laid out from the bottom: the scenario shows under the heading, above the runs.
            self.scenario_choice(ui, "sidebar");
            ui.add_space(2.0);
            ui.label(theme::strong(t("Experiment")).size(12.0).color(p.text_weak));
            ui.add_space(4.0);
            ui.separator();
        });
    }

    /// The start page, for an empty model not being built: examples by question, and ways in.
    pub(crate) fn start_page(&mut self, ui: &mut egui::Ui) {
        let cards: Vec<start::Item> = EXAMPLES
            .iter()
            .zip(crate::example_scenarios())
            .map(|(e, &scenarios)| start::Item {
                title: e.title,
                about: e.about,
                card: &e.card,
                scenarios,
            })
            .collect();
        let recent: Vec<String> = self
            .recent
            .iter()
            .map(|p| file_name(p).to_owned())
            .collect();
        let key = self.shortcut(commands::OPEN, ui.ctx()).unwrap_or_default();
        let action = start::show(ui, &mut self.start, &cards, &recent, &key);
        match action {
            Some(start::Action::Example(i)) => self.replace(Replace::Example(i)),
            // A new document (asking about unsaved changes first), so nothing of the last one's
            // file or undo history carries over.
            Some(start::Action::FirstModel) => self.replace(Replace::Build {
                getting_started: true,
            }),
            Some(start::Action::Switching) => self.open_url(help::SWITCHING),
            Some(start::Action::Open) => self.replace(Replace::Open),
            Some(start::Action::Recent(i)) => self.replace(Replace::Recent(i)),
            Some(start::Action::Blank) => self.replace(Replace::Build {
                getting_started: false,
            }),
            None => {}
        }
    }

    /// A model is open and has no results yet.
    pub(crate) fn ready(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.heading(t("No results yet"));
        ui.add_space(6.0);
        let x = &self.engine.model().experiment;
        ui.label(theme::weak(
            ui,
            tnf(
                u64::from(x.replications),
                "Makes {n} independent run of the model from seed {seed}. Each \
                 block's results come with the half-width of a 95% confidence interval.",
                "Makes {n} independent runs of the model from seed {seed}. Each \
                 block's results come with the half-width of a 95% confidence interval across \
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

    pub(crate) fn running_view(&mut self, ui: &mut egui::Ui, done: u64, partial: f64, total: u64) {
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
        let progress = if self.is_monte_carlo() {
            tnf(
                total,
                "{done} of {n} trial made",
                "{done} of {n} trials made",
                &[("done", &results::grouped(done as f64))],
            )
        } else {
            tnf(
                total,
                "{done} of {n} run finished",
                "{done} of {n} runs finished",
                &[("done", &done.to_string())],
            )
        };
        ui.label(theme::weak(ui, progress));
        ui.add_space(12.0);
        if ui
            .add(theme::secondary(ui, t("Cancel run")))
            .on_hover_text("Esc")
            .clicked()
        {
            self.cancel_run();
        }
    }
}

/// One block in the outline's process line: its BPMN shape (a thin circle for a start event, a
/// rounded box for a task, a thick circle for an end event), joined to its neighbours. Clicking
/// it selects the block; a mark shows a problem (theme's error and warning marks).
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
            // A shape as well as a colour: a circle with × for an error, a triangle with ! for a
            // warning; and the same words for screen readers, after the block's name.
            let at = egui::pos2(rect.right() - 14.0, cy);
            let words = if error {
                theme::paint_error_mark(&painter, at, 6.5, p);
                t("This block has an error")
            } else {
                theme::paint_warning_mark(&painter, at, 6.5, p);
                t("This block has a warning")
            };
            ui.ctx()
                .accesskit_node_builder(response.id, |n| n.set_description(words));
            return response.on_hover_text(words);
        }
        response
    }
}
