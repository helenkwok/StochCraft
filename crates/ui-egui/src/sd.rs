//! System dynamics models in the window (ROADMAP M4.2): a fourth kind of document beside process
//! models, Monte Carlo models and the empty start page. An XMILE file (opened with File ▸ Open, a
//! drop, or an agent's `file.open` and `sd.open`) is held by the engine as its own document, with
//! an empty process model beside it, so the window shows these screens and none of a process
//! model's views.
//!
//! It shows two tabs. **Model** reads the model as `sd.get` describes it: its settings, the
//! checks' warnings, and a table of its variables (kind, equation, units, a stock's flows). It is
//! read-only for now; editing, the diagram and saving come next, so Save is refused with a
//! message rather than writing the engine's empty process model over the file. **Results** runs
//! the model (`sd.run`) and answers first, in words: where each stock ends up, with, for a model
//! that has a random function, the range 9 runs in 10 end in. Under it, a chart for each variable
//! picked (stocks first, then flows, then auxiliaries) over time: the value itself for one run, or
//! the median with the band from the 5th to the 95th percentile and the mean dashed for runs in
//! replications; or the same values as a table, with a CSV export of the variables shown.
//!
//! A run is made in one piece on the frame after Run is pressed (the status bar says "Running…"
//! meanwhile), not a few replications a frame as a Monte Carlo run is: `stochcraft-sd` runs
//! whole replications and gives only the aggregates, so a stepped job would need a new interface
//! in that crate. The window refuses a run larger than [`WORK_LIMIT`] (a guess at what finishes
//! in about a second) with the way out, though the engine would allow it; the command line has
//! no such limit beyond the engine's own.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;
use std::rc::Rc;

use egui::{Align2, Color32, FontId, Rect, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::chart_palette::series_color;
use crate::charts::nice_step;
use crate::editor::section;
use crate::i18n::{t, tf, tn};
use crate::results::{grouped, round_sig};
use crate::{App, commands, theme};

/// The most work one run may do in the window: steps, times replications, times variables. A
/// guess, not a measurement, of what finishes in about a second in the browser; a larger run is
/// refused with the way out, though the engine would allow it.
const WORK_LIMIT: f64 = 20_000_000.0;

/// The most replications the Runs field takes (the engine's own limit).
const MOST_RUNS: f64 = 100_000.0;

/// The height of a table row, and the width of its columns.
const ROW_HEIGHT: f32 = 20.0;
const COLUMN_WIDTH: f32 = 140.0;

/// What the system dynamics screens keep between frames.
pub(crate) struct Sd {
    /// `sd.get`'s description of the model, with the engine's revision it was read at.
    info: Option<(u64, Rc<Value>)>,
    /// `sd.run`'s results on show.
    results: Option<Rc<Value>>,
    /// The runs and the seed of the next run, for a model with a random function.
    runs: f64,
    seed: f64,
    /// Run was pressed: the run is made on the next frame, after the status bar has said so.
    pending: bool,
    /// The variables shown, by name; `None` until the user picks (then the stocks).
    chosen: Option<BTreeSet<String>>,
    /// The Results tab shows the table in place of the charts.
    table: bool,
}

impl Default for Sd {
    fn default() -> Self {
        Self {
            info: None,
            results: None,
            runs: 100.0,
            seed: 1.0,
            pending: false,
            chosen: None,
            table: false,
        }
    }
}

/// A variable over time as the chart draws it.
struct Plot {
    times: Vec<f64>,
    /// The line: the median across runs, or the value itself for one run.
    line: Vec<f64>,
    /// The mean across runs (the same as `line` for one run).
    mean: Vec<f64>,
    /// The 5th and 95th percentiles across runs, for a model run in replications.
    band: Option<(Vec<f64>, Vec<f64>)>,
}

/// A column of numbers from results JSON; a value that is not a number (the engine writes `null`
/// for one that is not finite) is NaN, so the columns stay aligned with the times.
fn column(v: &Value) -> Vec<f64> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|x| x.as_f64().unwrap_or(f64::NAN))
        .collect()
}

/// Variable `name` of `results` as it is plotted.
fn plot_of(results: &Value, name: &str) -> Option<Plot> {
    let var = results["variables"]
        .as_array()?
        .iter()
        .find(|v| v["name"] == name)?;
    let times = column(&results["times"]);
    let mean = column(&var["mean"]);
    let many = results["replications"].as_u64().unwrap_or(1) > 1;
    let (line, band) = if many {
        (
            column(&var["median"]),
            Some((column(&var["p5"]), column(&var["p95"]))),
        )
    } else {
        (mean.clone(), None)
    };
    let n = times.len();
    let aligned = mean.len() == n
        && line.len() == n
        && band
            .as_ref()
            .is_none_or(|(lo, hi)| lo.len() == n && hi.len() == n);
    (n > 0 && aligned).then_some(Plot {
        times,
        line,
        mean,
        band,
    })
}

/// A value for a sentence, a tick or a cell: four significant figures, in the user's region.
fn value_text(v: f64) -> String {
    if v.is_finite() {
        grouped(round_sig(v, 4))
    } else {
        "–".to_owned()
    }
}

/// A number with its unit after it, when it has one.
fn with_unit(text: String, unit: &str) -> String {
    if unit.is_empty() {
        text
    } else {
        format!("{text} {unit}")
    }
}

fn kind_rank(kind: &str) -> u8 {
    match kind {
        "stock" => 0,
        "flow" => 1,
        _ => 2,
    }
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "stock" => t("Stock"),
        "flow" => t("Flow"),
        _ => t("Auxiliary"),
    }
}

/// The variables of `results` with their kinds: stocks first, then flows, then auxiliaries, each
/// in the order of the file.
fn ordered(results: &Value) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = results["variables"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|x| {
            (
                x["name"].as_str().unwrap_or("").to_owned(),
                x["kind"].as_str().unwrap_or("aux").to_owned(),
            )
        })
        .collect();
    v.sort_by_key(|(_, kind)| kind_rank(kind));
    v
}

/// The variables to show: the user's pick, or the stocks (or, in a model without any, the first
/// few variables).
fn shown_names(chosen: Option<&BTreeSet<String>>, results: &Value) -> BTreeSet<String> {
    if let Some(c) = chosen {
        return c.clone();
    }
    let all = ordered(results);
    let stocks: BTreeSet<String> = all
        .iter()
        .filter(|(_, kind)| kind == "stock")
        .map(|(name, _)| name.clone())
        .collect();
    if stocks.is_empty() {
        all.into_iter().take(6).map(|(name, _)| name).collect()
    } else {
        stocks
    }
}

/// The last number of a column of results JSON.
fn last(v: &Value) -> Option<f64> {
    v.as_array()?.last()?.as_f64()
}

/// Where each stock ends up, in words: its value at the stop time, and for a model run in
/// replications the range 9 runs in 10 end in.
fn findings(results: &Value) -> Vec<String> {
    let many = results["replications"].as_u64().unwrap_or(1) > 1;
    results["variables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["kind"] == "stock")
        .filter_map(|v| {
            let name = v["name"].as_str()?;
            let unit = v["units"].as_str().unwrap_or("");
            let shown = |x: f64| with_unit(value_text(x), unit);
            Some(if many {
                tf(
                    "{name} ends near {median}: 9 runs in 10 end between {low} and {high}.",
                    &[
                        ("name", name),
                        ("median", &shown(last(&v["median"])?)),
                        ("low", &shown(last(&v["p5"])?)),
                        ("high", &shown(last(&v["p95"])?)),
                    ],
                )
            } else {
                tf(
                    "{name} ends at {value}.",
                    &[("name", name), ("value", &shown(last(&v["mean"])?))],
                )
            })
        })
        .collect()
}

/// The indices of a column of `n` values to draw in a plot `most` points wide: all of them
/// when they fit, else every few, always with the first and the last.
fn picks(n: usize, most: usize) -> Vec<usize> {
    let most = most.max(2);
    if n <= most {
        return (0..n).collect();
    }
    let every = n.div_ceil(most);
    let mut v: Vec<usize> = (0..n).step_by(every).collect();
    if v.last() != Some(&(n - 1)) {
        v.push(n - 1);
    }
    v
}

/// A number field for a whole number from `range`, named `label` for screen readers.
fn count_field(
    ui: &mut egui::Ui,
    numbers: &craft_ui::localized::LocaleNumbers,
    salt: &str,
    label: &str,
    value: &mut f64,
    range: RangeInclusive<f64>,
) {
    let r = ui.add(
        craft_ui::localized::NumberField::new(value, numbers)
            .id_salt(salt)
            .decimals(0..=0)
            .range(range)
            .desired_width(90.0),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
}

/// One variable's chart: the line (the median for runs in replications) with the band from the
/// 5th to the 95th percentile and the mean dashed, scaled to its own values, with gridlines and
/// a tooltip at the time nearest the pointer. It is labelled for screen readers.
fn draw_chart(
    ui: &mut egui::Ui,
    plot: &Plot,
    name: &str,
    label: &str,
    colour: Color32,
    time_unit: &str,
    width: f32,
) {
    let p = theme::palette(ui.ctx());
    let axis_w = 60.0;
    let axis_h = 18.0;
    let height = 150.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, height + axis_h), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, label));
    let area = Rect::from_min_max(
        pos2(rect.left() + axis_w, rect.top() + 4.0),
        pos2(rect.right() - 8.0, rect.bottom() - axis_h),
    );
    if area.width() < 20.0 {
        return;
    }
    // The values' range, from the zero baseline when they start near it.
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let band_values = plot
        .band
        .iter()
        .flat_map(|(low, high)| low.iter().chain(high.iter()));
    for v in plot.line.iter().chain(plot.mean.iter()).chain(band_values) {
        if v.is_finite() {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    let finite_times: Vec<f64> = plot
        .times
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .collect();
    let (Some(first), Some(end)) = (finite_times.first().copied(), finite_times.last().copied())
    else {
        return;
    };
    if !(lo.is_finite() && hi.is_finite()) {
        return;
    }
    if (0.0..0.5 * hi).contains(&lo) {
        lo = 0.0;
    }
    if hi <= lo {
        hi = lo + lo.abs().max(1.0) * 0.1;
    }
    let step = nice_step(hi - lo, 4.0);
    let y_lo = (lo / step).floor() * step;
    let mut y_hi = (hi / step).ceil() * step;
    if y_hi <= y_lo {
        y_hi = y_lo + step;
    }
    let t_end = if end > first { end } else { first + 1.0 };
    let x_step = nice_step(t_end - first, 4.0);
    let x = |v: f64| {
        area.left() + (((v - first) / (t_end - first)).clamp(0.0, 1.0) as f32) * area.width()
    };
    let y = |v: f64| {
        area.bottom() - (((v - y_lo) / (y_hi - y_lo)).clamp(0.0, 1.0) as f32) * area.height()
    };
    let painter = ui.painter_at(rect.expand(2.0));

    // A recessive grid with its values.
    let grid = Stroke::new(1.0, p.border);
    for k in 0..=20_i32 {
        let v = y_lo + f64::from(k) * step;
        if v > y_hi + step * 0.01 {
            break;
        }
        painter.line_segment([pos2(area.left(), y(v)), pos2(area.right(), y(v))], grid);
        painter.text(
            pos2(area.left() - 6.0, y(v)),
            Align2::RIGHT_CENTER,
            value_text(v),
            FontId::proportional(11.0),
            p.text_weak,
        );
    }
    let first_tick = (first / x_step).ceil() * x_step;
    for k in 0..=20_i32 {
        let v = first_tick + f64::from(k) * x_step;
        if v > t_end + x_step * 0.01 {
            break;
        }
        painter.text(
            pos2(x(v), area.bottom() + 3.0),
            Align2::CENTER_TOP,
            value_text(v),
            FontId::proportional(11.0),
            p.text_weak,
        );
    }
    if !time_unit.is_empty() {
        painter.text(
            pos2(area.right(), area.bottom() + 3.0),
            Align2::RIGHT_TOP,
            time_unit,
            FontId::proportional(11.0),
            p.text_weak,
        );
    }

    let at = picks(plot.times.len(), area.width() as usize);
    // The band, as quads between the times drawn.
    if let Some((low, high)) = &plot.band {
        let shade = colour.linear_multiply(0.22);
        for pair in at.windows(2) {
            let (i, j) = (pair[0], pair[1]);
            let (a, b, c, d) = (high[i], high[j], low[j], low[i]);
            if [a, b, c, d, plot.times[i], plot.times[j]]
                .iter()
                .all(|v| v.is_finite())
            {
                let points = vec![
                    pos2(x(plot.times[i]), y(a)),
                    pos2(x(plot.times[j]), y(b)),
                    pos2(x(plot.times[j]), y(c)),
                    pos2(x(plot.times[i]), y(d)),
                ];
                painter.add(egui::Shape::convex_polygon(points, shade, Stroke::NONE));
            }
        }
    }
    let path = |values: &[f64]| -> Vec<egui::Pos2> {
        at.iter()
            .filter(|&&i| values[i].is_finite() && plot.times[i].is_finite())
            .map(|&i| pos2(x(plot.times[i]), y(values[i])))
            .collect()
    };
    if plot.band.is_some() {
        painter.extend(egui::Shape::dashed_line(
            &path(&plot.mean),
            Stroke::new(1.5, p.text),
            6.0,
            4.0,
        ));
    }
    painter.add(egui::Shape::line(
        path(&plot.line),
        Stroke::new(2.0, colour),
    ));

    // A crosshair at the time nearest the pointer, with the numbers there.
    let Some(pointer) = response
        .hover_pos()
        .filter(|pt| area.x_range().contains(pt.x))
    else {
        return;
    };
    let wanted = first + f64::from((pointer.x - area.left()) / area.width()) * (t_end - first);
    let i = plot
        .times
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_finite())
        .min_by(|a, b| (a.1 - wanted).abs().total_cmp(&(b.1 - wanted).abs()))
        .map_or(0, |(i, _)| i);
    let cx = x(plot.times[i]);
    painter.line_segment(
        [pos2(cx, area.top()), pos2(cx, area.bottom())],
        Stroke::new(1.0, p.text_weak),
    );
    if plot.line[i].is_finite() {
        painter.circle(
            pos2(cx, y(plot.line[i])),
            4.0,
            colour,
            Stroke::new(2.0, p.raised),
        );
    }
    let time = value_text(plot.times[i]);
    let tip = match &plot.band {
        Some((low, high)) => tf(
            "{name} at {time}: median {median}, mean {mean}, 9 runs in 10 between {low} and {high}",
            &[
                ("name", name),
                ("time", &time),
                ("median", &value_text(plot.line[i])),
                ("mean", &value_text(plot.mean[i])),
                ("low", &value_text(low[i])),
                ("high", &value_text(high[i])),
            ],
        ),
        None => tf(
            "{name} at {time}: {value}",
            &[
                ("name", name),
                ("time", &time),
                ("value", &value_text(plot.line[i])),
            ],
        ),
    };
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        egui::Id::unique("sd-chart-tip"),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.label(tip);
    });
}

/// The columns of the table of `names`: the times, then for each variable its value, or for runs
/// in replications its mean, median and 5th and 95th percentiles. Each is a header and the
/// results' array of numbers.
fn table_columns<'a>(
    results: &'a Value,
    names: &BTreeSet<String>,
    time_unit: &str,
) -> Vec<(String, &'a Value)> {
    let many = results["replications"].as_u64().unwrap_or(1) > 1;
    let time = if time_unit.is_empty() {
        t("Time").to_owned()
    } else {
        tf("Time ({unit})", &[("unit", time_unit)])
    };
    let mut columns = vec![(time, &results["times"])];
    for (name, _) in ordered(results) {
        if !names.contains(&name) {
            continue;
        }
        let Some(var) = results["variables"]
            .as_array()
            .and_then(|vs| vs.iter().find(|v| v["name"] == name.as_str()))
        else {
            continue;
        };
        if many {
            let n = name.as_str();
            columns.push((tf("{name} mean", &[("name", n)]), &var["mean"]));
            columns.push((tf("{name} median", &[("name", n)]), &var["median"]));
            columns.push((tf("{name} 5th percentile", &[("name", n)]), &var["p5"]));
            columns.push((tf("{name} 95th percentile", &[("name", n)]), &var["p95"]));
        } else {
            columns.push((name.clone(), &var["mean"]));
        }
    }
    columns
}

/// The table: a row for each saved time, scrolling on its own and drawing only the rows in view.
fn draw_table(ui: &mut egui::Ui, columns: &[(String, &Value)]) {
    let rows = columns
        .first()
        .and_then(|(_, times)| times.as_array())
        .map_or(0, Vec::len);
    egui::ScrollArea::horizontal()
        .id_salt("sd/table-x")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (header, _) in columns {
                    ui.add_sized(
                        [COLUMN_WIDTH, ROW_HEIGHT],
                        egui::Label::new(theme::strong(header.clone())).truncate(),
                    )
                    .on_hover_text(header.clone());
                }
            });
            egui::ScrollArea::vertical()
                .id_salt("sd/table-y")
                .max_height(380.0)
                .auto_shrink([false, true])
                .show_rows(ui, ROW_HEIGHT, rows, |ui, range| {
                    for row in range {
                        ui.horizontal(|ui| {
                            for (_, values) in columns {
                                let text = values
                                    .get(row)
                                    .and_then(Value::as_f64)
                                    .map_or_else(|| "–".to_owned(), |v| grouped(round_sig(v, 6)));
                                ui.add_sized([COLUMN_WIDTH, ROW_HEIGHT], egui::Label::new(text));
                            }
                        });
                    }
                });
        });
}

impl App {
    /// The document is a system dynamics model.
    pub(crate) fn is_sd(&self) -> bool {
        self.engine.has_system_dynamics()
    }

    /// The document is a system dynamics model (an XMILE file), which the window shows and runs
    /// but does not edit or save yet.
    pub fn is_system_dynamics(&self) -> bool {
        self.is_sd()
    }

    /// The results of the last run of the system dynamics model, as `sd.run` gives them.
    pub fn sd_results(&self) -> Option<&Value> {
        self.sd.results.as_deref()
    }

    /// What the chart of variable `name` draws: each saved time and the value plotted there (the
    /// median across runs for a model run in replications, the value itself for one run). `None`
    /// before a run, or for a name the model does not have.
    pub fn sd_plotted(&self, name: &str) -> Option<Vec<(f64, f64)>> {
        let plot = plot_of(self.sd.results.as_deref()?, name)?;
        Some(plot.times.into_iter().zip(plot.line).collect())
    }

    /// The band the chart of variable `name` shades: each saved time with the 5th and 95th
    /// percentiles there. `None` for a model run once, which has no spread, and before a run.
    pub fn sd_plotted_band(&self, name: &str) -> Option<Vec<(f64, f64)>> {
        let plot = plot_of(self.sd.results.as_deref()?, name)?;
        let (low, high) = plot.band?;
        Some(low.into_iter().zip(high).collect())
    }

    /// Whether closing or replacing the document would lose changes. A system dynamics document
    /// has none: it cannot be edited, and what the engine holds beside it (an empty process
    /// model) is never saved.
    pub(crate) fn unsaved_changes(&self) -> bool {
        self.engine.is_dirty() && !self.is_sd()
    }

    /// Refuses a save of a system dynamics model, which cannot be written yet: saving would
    /// write the engine's empty process model, over the file it came from. True when refused,
    /// with the reason in the status bar; what was to follow the save (quit, open) is dropped.
    pub(crate) fn refuse_sd_save(&mut self) -> bool {
        if !self.is_sd() {
            return false;
        }
        self.save_failed(t("Saving system dynamics models comes with editing."));
        true
    }

    /// `sd.get`'s description of the model, read again only when the engine's revision changed.
    fn sd_info(&mut self) -> Rc<Value> {
        let revision = self.engine.revision();
        if let Some((read_at, info)) = &self.sd.info
            && *read_at == revision
        {
            return Rc::clone(info);
        }
        let info = Rc::new(
            self.engine
                .execute("sd.get", json!({}))
                .unwrap_or(Value::Null),
        );
        self.sd.info = Some((revision, Rc::clone(&info)));
        info
    }

    /// Model ▸ Run on a system dynamics model: the run is made on the next frame ([`App::poll`]),
    /// after the status bar has said it is running.
    pub(crate) fn sd_run(&mut self) {
        if self.sd.pending {
            return;
        }
        self.sd.pending = true;
        self.show_results = true;
        self.info(t("Running…"));
    }

    /// Whether a run was asked for and not made yet.
    pub(crate) fn sd_pending(&self) -> bool {
        self.sd.pending
    }

    /// Makes the run Run asked for: the model's own, or for a model with a random function the
    /// Runs and Seed fields'. One too large for the window is refused.
    pub(crate) fn sd_run_now(&mut self) {
        self.sd.pending = false;
        let info = self.sd_info();
        let random = info["random"].as_bool().unwrap_or(false);
        let runs = self.sd.runs.round().clamp(1.0, MOST_RUNS);
        let steps = ((info["stop"].as_f64().unwrap_or(0.0)
            - info["start"].as_f64().unwrap_or(0.0))
            / info["dt"].as_f64().unwrap_or(1.0))
        .round()
        .max(1.0);
        let variables = info["variables"].as_array().map_or(1, Vec::len).max(1) as f64;
        let work = steps * variables * if random { runs } else { 1.0 };
        if work.is_nan() || work > WORK_LIMIT {
            self.report_error(t(
                "This run is too large to make in the window: shorten it, raise the time step or lower the runs, or run it from the command line.",
            ));
            return;
        }
        let params = if random {
            json!({ "replications": runs as u64, "seed": self.sd.seed.round().max(0.0) as u64 })
        } else {
            json!({})
        };
        match self.engine.execute("sd.run", params) {
            Ok(results) => {
                let n = results["replications"].as_u64().unwrap_or(1);
                self.sd.results = Some(Rc::new(results));
                self.info(tn(n, "Finished {n} run", "Finished {n} runs"));
            }
            Err(e) => self.report_error(e.message),
        }
    }

    /// An agent's `sd.run` ran: its results are the ones on show.
    pub(crate) fn sd_ran(&mut self, results: Value) {
        let n = results["replications"].as_u64().unwrap_or(1);
        self.sd.results = Some(Rc::new(results));
        self.info(tn(n, "Finished {n} run", "Finished {n} runs"));
    }

    /// The table on show as CSV (the variables picked, as `stochcraft-cli run --format csv`
    /// writes them), for the Export table button.
    pub(crate) fn sd_table_csv(&self) -> Option<String> {
        let mut results = Value::clone(self.sd.results.as_deref()?);
        let names = shown_names(self.sd.chosen.as_ref(), &results);
        if let Some(vars) = results["variables"].as_array_mut() {
            vars.retain(|v| v["name"].as_str().is_some_and(|n| names.contains(n)));
        }
        Some(stochcraft_engine::sd::results_csv(&results))
    }

    /// The left rail of a system dynamics model: its name, and its variables with their kinds.
    pub(crate) fn sd_outline(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let info = self.sd_info();
        let name = self.sd_name(&info);
        if ui
            .add(
                egui::Button::selectable(!self.show_results, theme::strong(&name).size(17.0))
                    .frame_when_inactive(false),
            )
            .on_hover_text(t("The model: its settings and variables"))
            .clicked()
        {
            self.show_results = false;
        }
        ui.label(theme::weak(ui, t("System dynamics model")));
        ui.add_space(12.0);
        ui.label(theme::strong(t("Variables")).size(12.0).color(p.text_weak));
        ui.add_space(2.0);
        egui::ScrollArea::vertical()
            .id_salt("sd/outline")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for v in info["variables"].as_array().into_iter().flatten() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(v["name"].as_str().unwrap_or(""));
                        ui.label(theme::weak(
                            ui,
                            kind_label(v["kind"].as_str().unwrap_or("")),
                        ));
                    });
                }
            });
    }

    /// The model's name: from the file, else the file's name.
    fn sd_name(&self, info: &Value) -> String {
        info["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .map_or_else(|| self.document_name(), str::to_owned)
    }

    /// The Model tab of a system dynamics model: read-only for now.
    pub(crate) fn sd_model_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let info = self.sd_info();
        let name = self.sd_name(&info);
        egui::ScrollArea::vertical()
            .id_salt("sd/model")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(900.0));
                ui.heading(t("System dynamics model"));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(theme::weak(
                        ui,
                        t("This is the model as its file has it. Editing comes next."),
                    ))
                    .wrap(),
                );
                ui.add_space(8.0);
                ui.label(theme::strong(&name).size(17.0));
                let unit = info["time_unit"].as_str().unwrap_or("");
                let method = match info["method"].as_str() {
                    Some("rk4") => "RK4",
                    Some("euler") => "Euler",
                    Some(other) => other,
                    None => "",
                };
                let number =
                    |key: &str| with_unit(value_text(info[key].as_f64().unwrap_or(f64::NAN)), unit);
                ui.add(
                    egui::Label::new(tf(
                        "From {start} to {stop}, in steps of {dt}, by the {method} method.",
                        &[
                            ("start", &number("start")),
                            ("stop", &number("stop")),
                            ("dt", &number("dt")),
                            ("method", method),
                        ],
                    ))
                    .wrap(),
                );

                section(ui, t("Problems"));
                let warnings: Vec<&str> = info["diagnostics"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|d| d["message"].as_str())
                    .map(|m| m.strip_prefix("warning: ").unwrap_or(m))
                    .collect();
                if warnings.is_empty() {
                    ui.label(theme::weak(ui, t("None found. The model is ready to run.")));
                }
                for w in warnings {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(theme::strong(t("Warning")).color(p.warning));
                        ui.label(w);
                    });
                }
                let notes: Vec<&str> = info["notes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                if !notes.is_empty() {
                    section(ui, t("Notes from the file"));
                    for n in notes {
                        ui.add(egui::Label::new(theme::weak(ui, n)).wrap());
                    }
                }

                section(ui, t("Variables"));
                crate::decide::grid(ui, "sd/variables", 5, |ui| {
                    for header in [
                        t("Name"),
                        t("Kind"),
                        t("Equation"),
                        t("Unit"),
                        t("Inflows and outflows"),
                    ] {
                        ui.label(theme::strong(header));
                    }
                    ui.end_row();
                    for v in info["variables"].as_array().into_iter().flatten() {
                        let kind = v["kind"].as_str().unwrap_or("");
                        ui.label(v["name"].as_str().unwrap_or(""));
                        ui.label(kind_label(kind));
                        let equation = v["equation"].as_str().unwrap_or("");
                        let equation = if kind == "stock" {
                            tf("starts at {value}", &[("value", equation)])
                        } else if equation.is_empty() {
                            "–".to_owned()
                        } else {
                            equation.to_owned()
                        };
                        ui.add(egui::Label::new(equation).wrap());
                        ui.label(v["units"].as_str().unwrap_or(""));
                        let names = |key: &str| -> String {
                            v[key]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let mut flows = Vec::new();
                        if kind == "stock" {
                            let (into, out) = (names("inflows"), names("outflows"));
                            if !into.is_empty() {
                                flows.push(tf("In: {flows}", &[("flows", &into)]));
                            }
                            if !out.is_empty() {
                                flows.push(tf("Out: {flows}", &[("flows", &out)]));
                            }
                        }
                        ui.label(flows.join("\n"));
                        ui.end_row();
                    }
                });
            });
    }

    /// The Results tab of a system dynamics model: Run, where each stock ends up in words, then
    /// the charts or the table of the variables picked.
    pub(crate) fn sd_results_view(&mut self, ui: &mut egui::Ui) {
        let info = self.sd_info();
        let random = info["random"].as_bool().unwrap_or(false);
        let results = self.sd.results.clone();
        let mut runs = self.sd.runs;
        let mut seed = self.sd.seed;
        let mut table = self.sd.table;
        let mut pick: Option<BTreeSet<String>> = None;
        let (mut run, mut export) = (false, false);
        let key = self.shortcut(commands::RUN, ui.ctx()).unwrap_or_default();
        let numbers = &self.numbers;
        let file_system = self.platform.file_system;
        let chosen = self.sd.chosen.clone();
        egui::ScrollArea::vertical()
            .id_salt("sd/results")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(900.0));
                ui.horizontal(|ui| {
                    ui.heading(t("Results"));
                    if table && results.is_some() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = if file_system {
                                t("Export table (CSV)")
                            } else {
                                t("Download table (CSV)")
                            };
                            export = ui.add(theme::secondary(ui, label)).clicked();
                        });
                    }
                });
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(theme::weak(
                        ui,
                        if random {
                            t("This model has a random function, so every run differs: make several to see how far they spread.")
                        } else {
                            t("This model has no random function, so one run gives its answer.")
                        },
                    ))
                    .wrap(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if random {
                        ui.label(t("Runs"));
                        count_field(ui, numbers, "sd/runs", t("Runs"), &mut runs, 1.0..=MOST_RUNS);
                        ui.label(t("Seed"));
                        count_field(ui, numbers, "sd/seed", t("Seed"), &mut seed, 0.0..=9_007_199_254_740_991.0);
                    }
                    let label = if results.is_some() { t("Run again") } else { t("Run") };
                    run = ui.add(theme::primary(ui, label)).clicked();
                    ui.label(theme::weak(ui, key));
                });
                let Some(results) = &results else {
                    ui.add_space(16.0);
                    ui.heading(t("No results yet"));
                    return;
                };
                ui.add_space(8.0);
                let n = results["replications"].as_u64().unwrap_or(1);
                ui.label(theme::weak(
                    ui,
                    if n > 1 {
                        tf(
                            "Runs: {n} · seed {seed}",
                            &[("n", &grouped(n as f64)), ("seed", &results["seed"].to_string())],
                        )
                    } else {
                        t("One run").to_owned()
                    },
                ));
                ui.add_space(6.0);
                for sentence in findings(results) {
                    ui.add(egui::Label::new(sentence).wrap());
                    ui.add_space(2.0);
                }
                for w in results["warnings"].as_array().into_iter().flatten() {
                    if let Some(w) = w.as_str() {
                        ui.add(egui::Label::new(theme::weak(ui, w)).wrap());
                    }
                }

                // Which variables, and charts or a table.
                ui.add_space(10.0);
                ui.label(theme::strong(t("Show on the chart")));
                let mut shown = shown_names(chosen.as_ref(), results);
                let mut changed = false;
                ui.horizontal_wrapped(|ui| {
                    for (name, _) in ordered(results) {
                        let mut on = shown.contains(&name);
                        if ui.checkbox(&mut on, name.clone()).changed() {
                            if on {
                                shown.insert(name);
                            } else {
                                shown.remove(&name);
                            }
                            changed = true;
                        }
                    }
                });
                if changed {
                    pick = Some(shown.clone());
                }
                ui.horizontal(|ui| {
                    if ui.selectable_label(!table, t("Chart")).clicked() {
                        table = false;
                    }
                    if ui.selectable_label(table, t("Table")).clicked() {
                        table = true;
                    }
                });
                ui.add_space(6.0);
                let time_unit = results["time_unit"].as_str().unwrap_or("");
                if table {
                    draw_table(ui, &table_columns(results, &shown, time_unit));
                    return;
                }
                if n > 1 {
                    ui.add(
                        egui::Label::new(theme::weak(
                            ui,
                            t("Solid line: median. Dashed line: mean. Shaded band: 9 runs in 10 fell inside it (5th to 95th percentile)."),
                        ))
                        .wrap(),
                    );
                    ui.add_space(6.0);
                }
                if shown.is_empty() {
                    ui.label(theme::weak(ui, t("Pick a variable to see it over time.")));
                }
                let width = ui.available_width();
                for (i, (name, kind)) in ordered(results).into_iter().enumerate() {
                    if !shown.contains(&name) {
                        continue;
                    }
                    let Some(plot) = plot_of(results, &name) else {
                        continue;
                    };
                    let units = results["variables"]
                        .as_array()
                        .and_then(|vs| vs.iter().find(|v| v["name"] == name.as_str()))
                        .and_then(|v| v["units"].as_str())
                        .unwrap_or("");
                    ui.horizontal(|ui| {
                        ui.label(theme::strong(&name).size(13.0));
                        ui.label(theme::weak(ui, kind_label(&kind)));
                        if !units.is_empty() {
                            ui.label(theme::weak(ui, units));
                        }
                    });
                    let label = tf("{name} over time", &[("name", &name)]);
                    draw_chart(ui, &plot, &name, &label, series_color(ui.ctx(), i), time_unit, width);
                    ui.add_space(10.0);
                }
            });
        self.sd.runs = runs;
        self.sd.seed = seed;
        self.sd.table = table;
        if pick.is_some() {
            self.sd.chosen = pick;
        }
        if run {
            self.run();
        }
        if export {
            self.export_csv();
        }
    }
}
