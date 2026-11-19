//! Watching a run (M3.5): one replication of the experiment, stepped at a chosen speed, with each
//! block's state drawn on the canvas as it runs (entities waiting, units busy, how many came and
//! went) and the selected block's queue over time in the inspector. Play, pause, step one event,
//! or pause at a time. It is replication 1 of the experiment: the same events, and at its end the
//! same statistics, as replication 1 of Run.
//!
//! Watching only reads the model. Changing anything a run depends on stops it; moving shapes
//! does not.

use std::collections::BTreeMap;

use std::time::Duration;

use egui::{Align2, FontId, Pos2, Rect, Stroke, Vec2};
use stochcraft_engine::{Live, LiveState, RunError};
use stochcraft_model::{ElementKind, Model};

use crate::chart_palette::series_color;
use crate::i18n::{t, tf};
use crate::results::grouped;
use crate::{App, theme};

/// Simulated time units per second of real time to choose from.
const SPEEDS: [f64; 16] = [
    0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0,
    10_000.0,
];
/// The most samples the queue-over-time charts keep; past it every other one is dropped.
const MAX_SAMPLES: usize = 4_000;
/// The real time a frame may spend running events (#76). Watching runs on the page's own thread,
/// so a frame that ran every event up to its target could freeze the page: a jump far ahead on a
/// busy model, a high speed, or a loop at one time (up to the stuck-time limit's 10 million
/// events). Half of a 60-per-second frame is left for drawing; what doesn't fit carries on in
/// the next frame, with the same events in the same order.
const FRAME_BUDGET: Duration = Duration::from_millis(8);
/// Events run between looks at the clock: about 0.2 to 0.5 ms on the desktop.
const CHUNK_EVENTS: u64 = 1_000;

/// Runs `run` towards `until` for at most about `budget` of real time (at least one chunk of
/// events); returns whether it got there.
fn advance_for(run: &mut Live<'static>, until: f64, budget: Duration) -> Result<bool, RunError> {
    let start = web_time::Instant::now();
    loop {
        if run.advance_toward(until, CHUNK_EVENTS)? {
            return Ok(true);
        }
        if start.elapsed() >= budget {
            return Ok(false);
        }
    }
}

/// A run being watched.
pub(crate) struct Watching {
    run: Live<'static>,
    pub(crate) playing: bool,
    /// Simulated time units per second.
    speed: f64,
    /// Run as fast as the frame's budget allows, whatever the speed says.
    fast: bool,
    /// The model as it was when watching started.
    model: Model,
    /// Over time: each element's entities waiting and units busy (or entities inside).
    trace: Vec<(f64, Vec<(f64, f64)>)>,
    /// The state now, by element id.
    states: BTreeMap<String, LiveState>,
    /// Text typed into the "Pause at" field.
    pause_text: String,
    /// The field's text wasn't a time when Pause at was last pressed; cleared by typing.
    pause_invalid: bool,
    /// A time to pause at: the run plays on, at its speed, until the clock gets there.
    pause_at: Option<f64>,
    /// A time a step is going to, over as many frames as it takes, and whether to say so when
    /// it gets to the end.
    heading: Option<(f64, bool)>,
}

/// The slowest and fastest speeds halving and doubling reach, in time units per second.
const SPEED_RANGE: (f64, f64) = (0.01, 1_000_000.0);

/// A speed as the bar shows it: "×10", with at most three decimals.
fn speed_text(speed: f64) -> String {
    format!("×{}", grouped((speed * 1_000.0).round() / 1_000.0))
}

/// Whether `b` would run exactly as `a`: everything but the names and the diagram.
fn same_run(a: &Model, b: &Model) -> bool {
    a.elements.len() == b.elements.len()
        && a.elements
            .iter()
            .zip(&b.elements)
            .all(|(x, y)| x.id == y.id && x.kind == y.kind && x.properties == y.properties)
        && a.connections == b.connections
        && a.resources == b.resources
        && a.experiment == b.experiment
        && a.calendars == b.calendars
}

/// A speed for watching a run of length `end` in about a minute.
fn default_speed(end: f64) -> f64 {
    let want = end / 60.0;
    SPEEDS
        .iter()
        .copied()
        .min_by(|a, b| (a - want).abs().total_cmp(&(b - want).abs()))
        .unwrap_or(1.0)
}

impl Watching {
    fn new(run: Live<'static>, model: Model) -> Self {
        let speed = default_speed(run.end());
        let mut w = Self {
            run,
            playing: true,
            speed,
            fast: false,
            model,
            trace: Vec::new(),
            states: BTreeMap::new(),
            pause_text: String::new(),
            pause_invalid: false,
            pause_at: None,
            heading: None,
        };
        w.sample();
        w
    }

    /// Records the state now.
    fn sample(&mut self) {
        let states = self.run.states();
        let now = self.run.now();
        let values: Vec<(f64, f64)> = states.iter().map(|s| (s.waiting, s.busy)).collect();
        if self.trace.last().is_some_and(|(t, _)| *t == now) {
            self.trace.pop();
        }
        self.trace.push((now, values));
        if self.trace.len() > MAX_SAMPLES {
            let mut i = 0;
            self.trace.retain(|_| {
                i += 1;
                i % 2 == 1
            });
        }
        self.states = states
            .into_iter()
            .map(|s| (s.id.as_str().to_owned(), s))
            .collect();
    }

    pub(crate) fn state(&self, id: &str) -> Option<&LiveState> {
        self.states.get(id)
    }

    /// The items on conveyor `id` now: how far along (0 to 1), entity, product key.
    pub(crate) fn conveyor_items(&self, id: &str) -> Vec<(f32, u64, u32)> {
        self.run.conveyor_items(id)
    }
    /// What the Dashboard shows of the run so far.
    pub(crate) fn glimpse(&self) -> Glimpse {
        let now = self.run.now();
        let warmup = self.model.experiment.warmup;
        let mut left = 0.0;
        let mut conveyors = false;
        // Per element index: how the trace's (waiting, busy) pair counts items inside, and
        // whether it is a conveyor.
        let mut shape: Vec<(bool, bool)> = Vec::with_capacity(self.model.elements.len());
        for e in &self.model.elements {
            match &e.kind {
                ElementKind::Sink { .. } => {
                    left += self
                        .states
                        .get(e.id.as_str())
                        .map_or(0.0, |s| s.left as f64);
                    shape.push((false, false));
                }
                ElementKind::Server { .. } | ElementKind::SharedTask { .. } => {
                    shape.push((true, false));
                }
                ElementKind::Delay { .. } => shape.push((false, false)),
                ElementKind::Conveyor { .. } => {
                    conveyors = true;
                    shape.push((false, true));
                }
                _ => shape.push((false, false)),
            }
        }
        let kinds_inside: Vec<bool> = self
            .model
            .elements
            .iter()
            .map(|e| {
                matches!(
                    e.kind,
                    ElementKind::Server { .. }
                        | ElementKind::SharedTask { .. }
                        | ElementKind::Delay { .. }
                        | ElementKind::Conveyor { .. }
                )
            })
            .collect();
        let sum = |only_conveyors: bool| -> Vec<(f64, f64)> {
            self.trace
                .iter()
                .map(|(time, v)| {
                    let total: f64 = v
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| {
                            kinds_inside.get(*i).copied().unwrap_or(false)
                                && (!only_conveyors || shape.get(*i).is_some_and(|s| s.1))
                        })
                        .map(|(i, (waiting, busy))| {
                            if shape.get(i).is_some_and(|s| s.0) {
                                waiting + busy
                            } else {
                                *busy
                            }
                        })
                        .sum();
                    (*time, total)
                })
                .collect()
        };
        Glimpse {
            left,
            measured: if now > warmup { now - warmup } else { now },
            end: self.run.end(),
            on_conveyors: conveyors.then(|| sum(true)),
            in_model: sum(false),
        }
    }
}

/// What the Dashboard shows of a run being watched: the items that have left through the sinks
/// and the time measured so far, and the number of items on the conveyors (when the model has
/// any) and in every station over time, as the trace sampled them.
pub(crate) struct Glimpse {
    pub left: f64,
    pub measured: f64,
    pub end: f64,
    pub on_conveyors: Option<Vec<(f64, f64)>>,
    pub in_model: Vec<(f64, f64)>,
}

impl App {
    /// Starts watching replication 1 of the experiment.
    pub(crate) fn start_watching(&mut self) {
        match self.engine.live_run(0) {
            Ok(run) => {
                self.watching = Some(Watching::new(run, self.engine.model().clone()));
                self.info(t("Watching run 1"));
            }
            Err(e) => self.report_error(e.message),
        }
    }

    pub(crate) fn stop_watching(&mut self) {
        if self.watching.take().is_some() {
            self.info(t("Stopped watching"));
        }
    }

    pub(crate) fn is_watching(&self) -> bool {
        self.watching.is_some()
    }

    /// The speed of the run being watched, in time units per second (the run bar's ×N).
    pub fn watch_speed(&self) -> Option<f64> {
        self.watching.as_ref().map(|w| w.speed)
    }

    /// Advances the run by what this frame's time is worth, and stops watching if the model
    /// changed in a way that matters to the run.
    pub(crate) fn tick_watching(&mut self, ctx: &egui::Context) {
        let Some(w) = &mut self.watching else {
            return;
        };
        if !same_run(&w.model, self.engine.model()) {
            self.watching = None;
            self.info(t("The model changed, so watching stopped"));
            return;
        }
        if let Some((to, announce)) = w.heading {
            let r = advance_for(&mut w.run, to, FRAME_BUDGET);
            w.sample();
            ctx.request_repaint();
            match r {
                Ok(false) => {}
                Ok(true) => {
                    w.heading = None;
                    // A step's arrival at the end is not said, as playing's is.
                    let playing = w.playing;
                    w.playing = announce;
                    self.after_advance(Ok(()));
                    if let Some(w) = &mut self.watching {
                        w.playing = w.playing && playing;
                    }
                }
                Err(e) => {
                    w.heading = None;
                    self.after_advance(Err(e));
                }
            }
            return;
        }
        if !w.playing || w.run.is_done() {
            return;
        }
        let dt = f64::from(ctx.input(|i| i.stable_dt).clamp(0.0, 0.25));
        let mut target = if w.fast {
            w.run.end()
        } else {
            w.run.now() + w.speed * dt
        };
        if let Some(at) = w.pause_at {
            target = target.min(at);
        }
        // What doesn't fit in the frame's budget is dropped: the run plays slower than asked
        // rather than freezing the page.
        let r = advance_for(&mut w.run, target, FRAME_BUDGET).map(|_| ());
        w.sample();
        ctx.request_repaint();
        // Got to the time to pause at: pause there (the end of the run is reported as ever).
        if let Some(at) = w.pause_at
            && r.is_ok()
            && w.run.now() >= at
        {
            w.pause_at = None;
            if !w.run.is_done() {
                w.playing = false;
            }
        }
        self.after_advance(r);
    }

    fn after_advance(&mut self, r: Result<(), stochcraft_engine::RunError>) {
        let unit = self.time_unit_label();
        let Some(w) = &mut self.watching else {
            return;
        };
        match r {
            Err(e) => {
                w.playing = false;
                let message = e.to_string();
                self.report_error(message);
            }
            Ok(()) if w.run.is_done() && w.playing => {
                w.playing = false;
                let at = grouped(w.run.now());
                self.info(tf(
                    "Finished at {time} {unit}",
                    &[("time", &at), ("unit", &unit)],
                ));
            }
            Ok(()) => {}
        }
    }

    pub(crate) fn toggle_playing(&mut self) {
        if let Some(w) = &mut self.watching {
            if w.run.is_done() {
                return;
            }
            w.playing = !w.playing;
            if !w.playing {
                w.pause_at = None;
            }
        }
    }

    /// The run bar's Space: play or pause the run being watched, or start watching one.
    pub(crate) fn run_or_pause(&mut self) {
        if self.is_watching() {
            self.toggle_playing();
        } else if self.running.is_none() && !self.engine.model().elements.is_empty() {
            self.start_watching();
        }
    }

    /// Starts the watched run over, paused at its start, at the same speed.
    pub(crate) fn reset_watching(&mut self) {
        let Some(old) = &self.watching else {
            return;
        };
        let (speed, fast) = (old.speed, old.fast);
        match self.engine.live_run(0) {
            Ok(run) => {
                let mut w = Watching::new(run, self.engine.model().clone());
                w.speed = speed;
                w.fast = fast;
                w.playing = false;
                self.watching = Some(w);
                self.info(t("Watching run 1"));
            }
            Err(e) => self.report_error(e.message),
        }
    }

    /// Halves or doubles the speed (`factor` 0.5 or 2.0), within the range the bar offers;
    /// choosing a speed ends "as fast as possible".
    pub(crate) fn scale_speed(&mut self, factor: f64) {
        if let Some(w) = &mut self.watching {
            w.speed = (w.speed * factor).clamp(SPEED_RANGE.0, SPEED_RANGE.1);
            w.fast = false;
        }
    }

    /// Runs the next event (and every other one at the same time), from the next frame on.
    pub(crate) fn step_watching(&mut self) {
        let Some(w) = &mut self.watching else {
            return;
        };
        w.playing = false;
        let to = w.run.next_event_time().unwrap_or(w.run.end());
        w.heading = Some((to, false));
    }

    /// Plays on until the clock is at `to`, then pauses there (a time already passed pauses
    /// now: the run is not taken back).
    pub(crate) fn pause_at_time(&mut self, to: f64) {
        if let Some(w) = &mut self.watching {
            if w.run.is_done() {
                return;
            }
            if to <= w.run.now() {
                w.playing = false;
                w.pause_at = None;
            } else {
                w.playing = true;
                w.pause_at = Some(to);
            }
        }
    }

    /// The run bar, above the Model and 3D views: Watch a run, or the run's controls and clock.
    /// It is for watching one run; Run in the header runs the whole experiment, fast and
    /// without animation.
    pub(crate) fn watch_bar(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let unit = self.time_unit_label();
        let can_watch = self.running.is_none();
        let Some(w) = &mut self.watching else {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(can_watch, theme::secondary(ui, t("Watch a run")))
                    .on_hover_text(t(
                        "Play run 1 step by step and see each block's queue on the diagram",
                    ))
                    .clicked()
                {
                    self.start_watching();
                }
            });
            ui.add_space(4.0);
            return;
        };
        let mut toggle = false;
        let mut step = false;
        let mut reset = false;
        let mut stop = false;
        let mut slower = false;
        let mut faster = false;
        let mut pause_at: Option<f64> = None;
        let done = w.run.is_done();
        ui.horizontal_wrapped(|ui| {
            let play = if w.playing { t("Pause") } else { t("Play") };
            if ui.add_enabled(!done, theme::primary(ui, play)).clicked() {
                toggle = true;
            }
            if ui.add(theme::secondary(ui, t("Reset"))).clicked() {
                reset = true;
            }
            if ui
                .add_enabled(!done, theme::secondary(ui, t("Step")))
                .on_hover_text(t("Run the next event"))
                .clicked()
            {
                step = true;
            }
            ui.separator();
            ui.label(theme::weak(ui, t("Speed")));
            let halve = ui.add(theme::secondary(ui, "÷2"));
            halve.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, t("Halve Speed"))
            });
            if halve.on_hover_text(t("Halve Speed")).clicked() {
                slower = true;
            }
            ui.label(theme::strong(speed_text(w.speed)))
                .on_hover_text(tf(
                    "{n} {unit} per second",
                    &[("n", &grouped(w.speed)), ("unit", &unit)],
                ));
            let double = ui.add(theme::secondary(ui, "×2"));
            double.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, t("Double Speed"))
            });
            if double.on_hover_text(t("Double Speed")).clicked() {
                faster = true;
            }
            ui.toggle_value(&mut w.fast, t("As fast as possible"));
            ui.separator();
            let field = ui.add(
                egui::TextEdit::singleline(&mut w.pause_text)
                    .desired_width(70.0)
                    .hint_text(t("time")),
            );
            field.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TextInput, true, t("Pause at"))
            });
            if field.changed() {
                w.pause_invalid = false;
            }
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui
                .add_enabled(!done, theme::secondary(ui, t("Pause at")))
                .clicked()
                || entered
            {
                // Text that isn't a time is said so, not ignored (WCAG 3.3.1).
                match self.numbers.parse(w.pause_text.trim()) {
                    Ok(v) if v >= 0.0 && v.is_finite() => {
                        pause_at = Some(v);
                        w.pause_invalid = false;
                    }
                    _ => w.pause_invalid = true,
                }
            }
            ui.separator();
            if ui.add(theme::secondary(ui, t("Stop watching"))).clicked() {
                stop = true;
            }
        });
        // The clock: where the run is, the warm-up, the end, and how far along.
        let now = w.run.now();
        let end = w.run.end();
        if w.pause_invalid {
            theme::error_message(
                ui,
                tf(
                    "Type a time from 0 to {end} {unit}.",
                    &[("end", &grouped(end)), ("unit", &unit)],
                ),
            );
        }
        let warm = w.run.warmup();
        let clock = if now < warm {
            tf(
                "Run 1 · {time} of {end} {unit} · warm-up",
                &[
                    ("time", &grouped(now)),
                    ("end", &grouped(end)),
                    ("unit", &unit),
                ],
            )
        } else {
            tf(
                "Run 1 · {time} of {end} {unit}",
                &[
                    ("time", &grouped(now)),
                    ("end", &grouped(end)),
                    ("unit", &unit),
                ],
            )
        };
        let fraction = if end > 0.0 {
            ((now / end) as f32).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let per_cent = (fraction * 100.0).floor();
        ui.add(
            egui::ProgressBar::new(fraction)
                .text(format!("{clock} · {per_cent}%"))
                .fill(p.accent_soft),
        );
        // One animated run hides the variation between runs: say what this one is.
        let replications = self.engine.model().experiment.replications;
        if replications > 1 {
            ui.label(theme::weak(
                ui,
                tf(
                    "One run of {n}: the results of the others differ.",
                    &[("n", &replications.to_string())],
                ),
            ));
        }
        ui.add_space(4.0);
        if toggle {
            self.toggle_playing();
        }
        if step {
            self.step_watching();
        }
        if reset {
            self.reset_watching();
        }
        if slower {
            self.scale_speed(0.5);
        }
        if faster {
            self.scale_speed(2.0);
        }
        if let Some(v) = pause_at {
            self.pause_at_time(v);
        }
        if step || pause_at.is_some() {
            ui.ctx().request_repaint();
        }
        if stop {
            self.stop_watching();
        }
    }

    /// The run's state on each shape: what is waiting, busy or inside, and how many came and
    /// went.
    pub(crate) fn live_badges(
        &self,
        painter: &egui::Painter,
        shapes: &[(String, Rect)],
        p: &theme::Palette,
        zoom: f32,
    ) {
        let Some(w) = &self.watching else {
            return;
        };
        let font = FontId::proportional((11.0 * zoom).clamp(8.0, 15.0));
        let model = self.engine.model();
        for (id, r) in shapes {
            let (Some(s), Some(e)) = (
                w.state(id),
                model.elements.iter().find(|e| e.id.as_str() == id),
            ) else {
                continue;
            };
            let count = |v: f64| grouped(v.round());
            let pill = |text: String, at: Pos2, anchor: Align2, strong: bool| {
                let g = painter.layout_no_wrap(
                    text,
                    font.clone(),
                    if strong { p.on_accent } else { p.text },
                );
                let size = g.size() + Vec2::new(10.0, 4.0);
                let rect = anchor.anchor_size(at, size);
                painter.rect_filled(rect, size.y / 2.0, if strong { p.accent } else { p.raised });
                painter.rect_stroke(
                    rect,
                    size.y / 2.0,
                    Stroke::new(1.0, p.accent),
                    egui::StrokeKind::Inside,
                );
                painter.galley(rect.min + Vec2::new(5.0, 2.0), g, p.text);
            };
            let above = r.center_top() - Vec2::new(0.0, 4.0);
            match &e.kind {
                ElementKind::Source { .. } => pill(
                    tf("{n} created", &[("n", &count(s.entered as f64))]),
                    above,
                    Align2::CENTER_BOTTOM,
                    false,
                ),
                ElementKind::Sink { .. } => pill(
                    tf("{n} done", &[("n", &count(s.left as f64))]),
                    above,
                    Align2::CENTER_BOTTOM,
                    false,
                ),
                ElementKind::Server { .. } | ElementKind::SharedTask { .. } => {
                    pill(
                        tf("{n} waiting", &[("n", &count(s.waiting))]),
                        above,
                        Align2::CENTER_BOTTOM,
                        s.waiting >= 1.0,
                    );
                    // Busy units as a bar along the bottom.
                    let inner = r.shrink(4.0 * zoom);
                    let bar = Rect::from_min_max(
                        Pos2::new(inner.left(), inner.bottom() - 4.0 * zoom),
                        inner.right_bottom(),
                    );
                    if let Some(units) = s.units.filter(|u| *u > 0.0) {
                        let f = (s.busy / units).clamp(0.0, 1.0) as f32;
                        painter.rect_filled(bar, 2.0, p.stripe);
                        let mut busy = bar;
                        busy.set_right(bar.left() + bar.width() * f);
                        painter.rect_filled(busy, 2.0, p.accent);
                    }
                    pill(
                        tf("{busy} busy", &[("busy", &count(s.busy))]),
                        r.center_bottom() + Vec2::new(0.0, 4.0),
                        Align2::CENTER_TOP,
                        false,
                    );
                }
                ElementKind::Delay { .. } => pill(
                    tf("{n} inside", &[("n", &count(s.busy))]),
                    above,
                    Align2::CENTER_BOTTOM,
                    false,
                ),
                ElementKind::Conveyor { .. } => {
                    pill(
                        tf("{n} inside", &[("n", &count(s.busy))]),
                        above,
                        Align2::CENTER_BOTTOM,
                        false,
                    );
                    // Each item as a small dot along the belt, coloured by its product.
                    let inner = r.shrink(4.0 * zoom);
                    let radius = (2.5 * zoom).clamp(1.5, 4.0);
                    for (fraction, _, key) in w.conveyor_items(id) {
                        let at =
                            Pos2::new(inner.left() + inner.width() * fraction, inner.center().y);
                        let colour = series_color(painter.ctx(), key as usize);
                        painter.circle_filled(at, radius, colour);
                        painter.circle_stroke(at, radius, Stroke::new(0.75, p.raised));
                    }
                }
                _ => {}
            }
        }
    }

    /// The inspector's part while watching: the selected block now, and over time.
    pub(crate) fn live_section(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let unit = self.time_unit_label();
        let Some(w) = &self.watching else {
            return;
        };
        crate::editor::section(ui, t("This run"));
        let Some(id) = self.selected.clone() else {
            ui.label(theme::weak(
                ui,
                t("Select a block to see its queue over time."),
            ));
            return;
        };
        let model = self.engine.model();
        let Some((index, e)) = model
            .elements
            .iter()
            .enumerate()
            .find(|(_, e)| e.id.as_str() == id)
        else {
            return;
        };
        let Some(s) = w.state(&id) else { return };
        let queue = matches!(
            e.kind,
            ElementKind::Server { .. } | ElementKind::SharedTask { .. }
        );
        let count = |v: f64| grouped(v.round());
        ui.label(tf(
            "{entered} in, {left} out, {lost} lost",
            &[
                ("entered", &count(s.entered as f64)),
                ("left", &count(s.left as f64)),
                ("lost", &count(s.lost as f64)),
            ],
        ));
        if !matches!(
            e.kind,
            ElementKind::Server { .. }
                | ElementKind::SharedTask { .. }
                | ElementKind::Delay { .. }
                | ElementKind::Conveyor { .. }
        ) {
            return;
        }
        let title = if queue {
            t("Waiting over time")
        } else {
            t("Inside over time")
        };
        ui.add_space(6.0);
        ui.label(theme::strong(title));
        let points: Vec<(f64, f64)> = w
            .trace
            .iter()
            .filter_map(|(time, v)| {
                v.get(index)
                    .map(|(waiting, busy)| (*time, if queue { *waiting } else { *busy }))
            })
            .collect();
        line_chart(ui, p, &points, w.run.end(), &unit, title);
    }
}

/// A step chart of `points` (time, value) from 0 to `end`, as values hold until they change.
fn line_chart(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    points: &[(f64, f64)],
    end: f64,
    unit: &str,
    title: &str,
) {
    let width = ui.available_width().clamp(160.0, 420.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 120.0), egui::Sense::hover());
    let most = points.iter().map(|(_, v)| *v).fold(1.0f64, f64::max).ceil();
    let last = points.last().map_or(0.0, |(_, v)| *v);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Image,
            true,
            tf(
                "{title}: now {now}, at most {most}",
                &[
                    ("title", title),
                    ("now", &grouped(last)),
                    ("most", &grouped(most)),
                ],
            ),
        )
    });
    let painter = ui.painter_at(rect);
    let plot = Rect::from_min_max(
        rect.min + Vec2::new(28.0, 6.0),
        rect.max - Vec2::new(6.0, 18.0),
    );
    let small = FontId::proportional(10.5);
    // A recessive frame and gridline at the top value.
    painter.line_segment(
        [plot.left_bottom(), plot.right_bottom()],
        Stroke::new(1.0, p.border),
    );
    painter.line_segment(
        [plot.left_top(), plot.right_top()],
        Stroke::new(0.5, p.border),
    );
    painter.text(
        plot.left_top() - Vec2::new(4.0, 0.0),
        Align2::RIGHT_CENTER,
        grouped(most),
        small.clone(),
        p.text_weak,
    );
    painter.text(
        plot.left_bottom() - Vec2::new(4.0, 0.0),
        Align2::RIGHT_CENTER,
        "0",
        small.clone(),
        p.text_weak,
    );
    painter.text(
        plot.right_bottom() + Vec2::new(0.0, 3.0),
        Align2::RIGHT_TOP,
        format!("{} {unit}", grouped(end)),
        small,
        p.text_weak,
    );
    if end <= 0.0 || points.is_empty() {
        return;
    }
    let at = |time: f64, v: f64| {
        Pos2::new(
            plot.left() + plot.width() * (time / end).clamp(0.0, 1.0) as f32,
            plot.bottom() - plot.height() * (v / most).clamp(0.0, 1.0) as f32,
        )
    };
    let mut line: Vec<Pos2> = Vec::with_capacity(points.len() * 2);
    let mut prev: Option<f64> = None;
    for (time, v) in points {
        if let Some(pv) = prev {
            line.push(at(*time, pv));
        }
        line.push(at(*time, *v));
        prev = Some(*v);
    }
    painter.add(egui::Shape::line(line, Stroke::new(1.5, p.chart)));
}

#[cfg(test)]
mod tests {
    use super::default_speed;

    #[test]
    fn a_run_takes_about_a_minute_to_watch() {
        assert_eq!(default_speed(540.0), 10.0);
        assert_eq!(default_speed(60.0), 1.0);
        assert_eq!(default_speed(0.0), 0.1);
    }
}
