//! Charts of a run's results: one measure per chart (one axis), one bar per block with the 95%
//! confidence interval as a whisker, the value at the bar's tip and a tooltip on hover. The
//! table below them holds every number, so these charts carry no legend: each has one series,
//! named by its title.
//!
//! Marks follow the dataviz rules: bars at most 16 px thick, square at the baseline with a 4 px
//! rounded end, a recessive grid, text in text colours (never the bar's). A chart of one series
//! keeps the theme's chart hue, validated per theme against its surface
//! (`theme::Palette::chart`). A chart of several series, such as worked on and waiting in "Where
//! an entity's time goes", gives series `i` the chart palette's colour `i`
//! ([`series_color`], View ▸ Appearance ▸ Chart colours…) and names them in a legend.

use egui::{Align2, Color32, FontId, Rect, Stroke, pos2, vec2};
use serde_json::Value;

use crate::chart_palette::series_color;
use crate::i18n::{t, tf, tnf};
use crate::results::{grouped, round_sig, rounded, unit_label};
use crate::theme;

/// What one chart shows.
struct Measure {
    title: &'static str,
    metric: &'static str,
    /// Values are shares, shown as percentages on a 0–100 axis.
    share: bool,
    /// Values are times, in the model's time unit.
    time: bool,
}

const MEASURES: &[Measure] = &[
    Measure {
        title: "Utilisation",
        metric: "utilisation",
        share: true,
        time: false,
    },
    Measure {
        title: "Mean wait",
        metric: "mean_wait",
        share: false,
        time: true,
    },
    Measure {
        title: "Mean queue length",
        metric: "mean_queue_length",
        share: false,
        time: false,
    },
    Measure {
        title: "Mean time in system",
        metric: "mean_time_in_system",
        share: false,
        time: true,
    },
    // Only where a waiting room is finite (M2.4); empty charts are left out.
    Measure {
        title: "Share of units blocked",
        metric: "blocking_probability",
        share: true,
        time: false,
    },
    Measure {
        title: "Share of arrivals lost",
        metric: "loss_probability",
        share: true,
        time: false,
    },
];

/// The chart that leads where nothing waits but a machine goes down: availability per block.
const AVAILABILITY: Measure = Measure {
    title: "Availability",
    metric: "availability",
    share: true,
    time: false,
};

/// Whether availability is the answer to lead with: every block's mean wait is 0 (or none is
/// reported) and some block is up less than all the time. Otherwise the mean wait leads.
pub(crate) fn availability_leads(results: &Value) -> bool {
    let mean = |e: &Value, metric: &str| e["metrics"][metric]["mean"].as_f64();
    let elements = results["elements"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    elements
        .iter()
        .all(|e| mean(e, "mean_wait").is_none_or(|w| w.abs() < 1e-9))
        && elements
            .iter()
            .any(|e| mean(e, "availability").is_some_and(|a| a < 1.0))
}

/// One bar: the block's name, mean, half-width and replications.
struct Bar {
    name: String,
    mean: f64,
    half_width: f64,
    n: u64,
}

fn bars(results: &Value, m: &Measure) -> Vec<Bar> {
    let scale = if m.share { 100.0 } else { 1.0 };
    results["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let v = &e["metrics"][m.metric];
            let mean = v["mean"].as_f64()?;
            let id = e["id"].as_str().unwrap_or("");
            Some(Bar {
                name: e["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(id)
                    .to_owned(),
                mean: mean * scale,
                half_width: v["half_width"].as_f64().unwrap_or(0.0) * scale,
                n: v["n"].as_u64().unwrap_or(0),
            })
        })
        .collect()
}

/// A round step for about `ticks` gridlines up to `max` (1, 2 or 5 times a power of ten),
/// found without logarithms (which differ between platforms; this only places gridlines).
pub(crate) fn nice_step(max: f64, ticks: f64) -> f64 {
    // Not a size a step can be found for (infinite, not a number): any, rather than a search that
    // never ends.
    if !(max.is_finite() && ticks.is_finite() && ticks > 0.0) {
        return 1.0;
    }
    let raw = (max / ticks).max(f64::MIN_POSITIVE);
    let mut p = 1.0;
    while p > raw {
        p /= 10.0;
    }
    while p * 10.0 <= raw {
        p *= 10.0;
    }
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|k| k * p)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * p)
}

/// The axis for bars reaching up to `max`: the gridline step and the value at the right end.
/// With nothing above zero (every bar 0, as a queue that never forms) the axis runs from 0 to 1,
/// so the zero bars sit at its start under round labels.
pub(crate) fn axis(max: f64) -> (f64, f64) {
    let max = if max.is_finite() && max > 0.0 {
        max
    } else {
        1.0
    };
    let step = nice_step(max, 4.0);
    (step, (max / step).ceil() * step)
}

/// The two charts chosen for the question, side by side where they fit: where an entity's time
/// goes (along a straight line of tasks, `line`, in flow order; otherwise the mean wait per
/// block), and the waiting before the busiest task over the run (from the results over time;
/// otherwise utilisation per block). Every other chart with data is folded under "More charts".
/// Returns the area of the two (for exporting them as an image), or `None` when neither applies.
pub fn show(ui: &mut egui::Ui, results: &Value, line: Option<&[String]>) -> Option<Rect> {
    let unit = unit_label(results["record"]["time_unit"].as_str().unwrap_or(""));
    let width = ui.available_width().min(1100.0);
    let per_row = if width >= 760.0 { 2 } else { 1 };
    let chart_w = (width - 32.0 * (per_row as f32 - 1.0)) / per_row as f32;
    let parts = line
        .and_then(|l| parts(results, l))
        .filter(|p| !p.is_empty());
    // A line of zeros says nothing: the second chart is then the utilisation of each block.
    let busiest = crate::results::busiest(results).filter(|b| !queue_never_forms(results, b));
    let measure = |metric: &str| MEASURES.iter().find(|m| m.metric == metric);
    let mut used: Vec<&str> = Vec::new();
    let mut drew = false;
    let top = ui.cursor().min;
    let first = |ui: &mut egui::Ui, used: &mut Vec<&str>| -> bool {
        if let Some(parts) = &parts {
            time_chart(ui, results, parts, unit, chart_w);
            return true;
        }
        let m = if availability_leads(results) {
            &AVAILABILITY
        } else if let Some(m) = measure("mean_wait") {
            m
        } else {
            return false;
        };
        let b = bars(results, m);
        if b.is_empty() {
            return false;
        }
        chart(ui, m, &b, unit, chart_w);
        used.push(m.metric);
        true
    };
    let second = |ui: &mut egui::Ui, used: &mut Vec<&str>| -> bool {
        if busiest.is_some_and(|b| over_run_chart(ui, results, b, unit, chart_w)) {
            return true;
        }
        let Some(m) = measure("utilisation") else {
            return false;
        };
        let b = bars(results, m);
        if b.is_empty() {
            return false;
        }
        chart(ui, m, &b, unit, chart_w);
        used.push(m.metric);
        true
    };
    let column = egui::Layout::top_down(egui::Align::Min);
    if per_row == 2 {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 32.0;
            ui.allocate_ui_with_layout(vec2(chart_w, 0.0), column, |ui| {
                ui.set_width(chart_w);
                drew |= first(ui, &mut used);
            });
            ui.allocate_ui_with_layout(vec2(chart_w, 0.0), column, |ui| {
                ui.set_width(chart_w);
                drew |= second(ui, &mut used);
            });
        });
    } else {
        drew |= first(ui, &mut used);
        ui.add_space(14.0);
        drew |= second(ui, &mut used);
    }
    let area = Rect::from_min_max(top, pos2(top.x + width, ui.cursor().min.y));
    ui.add_space(10.0);

    // Every other chart with data, folded.
    let more: Vec<(&Measure, Vec<Bar>)> = MEASURES
        .iter()
        .filter(|m| !used.contains(&m.metric))
        .map(|m| (m, bars(results, m)))
        .filter(|(_, b)| !b.is_empty())
        .collect();
    if !more.is_empty() {
        egui::CollapsingHeader::new(theme::strong(t("More charts")))
            .id_salt("more-charts")
            .default_open(false)
            .show(ui, |ui| {
                for row in more.chunks(per_row) {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 32.0;
                        for (m, bars) in row {
                            ui.allocate_ui_with_layout(vec2(chart_w, 0.0), column, |ui| {
                                ui.set_width(chart_w);
                                chart(ui, m, bars, unit, chart_w);
                            });
                        }
                    });
                    ui.add_space(14.0);
                }
            });
    }
    drew.then_some(area)
}

/// One task's part of an entity's time: worked on and waiting, in the model's time unit.
struct Part {
    name: String,
    work: f64,
    wait: f64,
}

/// Where an entity's time goes along a straight line of tasks (`line`, the ids of its tasks in
/// flow order): for each, the mean wait and the mean time worked on, the latter by Little's law
/// as the mean number in service (`mean_contents`, which leaves out the waiting room) over the
/// rate through it. `None` where a task lacks what that needs or nothing passed through.
fn parts(results: &Value, line: &[String]) -> Option<Vec<Part>> {
    let measured = match &results["measured"] {
        m if m.is_object() => m["mean_duration"].as_f64(),
        _ => results["record"]["duration"].as_f64(),
    }
    .filter(|d| *d > 0.0)?;
    let elements = results["elements"].as_array()?;
    line.iter()
        .map(|id| {
            let e = elements.iter().find(|e| e["id"] == id.as_str())?;
            let m = &e["metrics"];
            let wait = m["mean_wait"]["mean"].as_f64().unwrap_or(0.0);
            let work = match work_per_run(results, id) {
                Some(w) => w,
                None => {
                    let left = m["left"]["mean"].as_f64().filter(|l| *l > 0.0)?;
                    m["mean_contents"]["mean"].as_f64()? / (left / measured)
                }
            };
            Some(Part {
                name: e["name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(id)
                    .to_owned(),
                work,
                wait,
            })
        })
        .collect()
}

/// A task's mean time worked on, run by run as the other means are: in each run whose
/// statistics are used, the number in service (`mean_contents`) over the rate through it
/// (`left` over the time that run measured), then the mean over those runs. With a stop
/// condition the runs measure different times, and the means of the parts would not give it.
/// `None` without each run's values and spans.
fn work_per_run(results: &Value, id: &str) -> Option<f64> {
    let spans: Vec<(f64, bool)> = results["measured"]["spans"]
        .as_array()?
        .iter()
        .map(|s| {
            let d = s["to"].as_f64().unwrap_or(0.0) - s["from"].as_f64().unwrap_or(0.0);
            (d, s["used"].as_bool().unwrap_or(true))
        })
        .collect();
    let contents = results["values"][id]["mean_contents"].as_array()?;
    let left = results["values"][id]["left"].as_array()?;
    if contents.len() != spans.len() || left.len() != spans.len() {
        return None;
    }
    let works: Vec<f64> = spans
        .iter()
        .zip(contents.iter().zip(left))
        .filter(|((d, used), _)| *used && *d > 0.0)
        .filter_map(|((d, _), (c, l))| {
            let (c, l) = (c.as_f64()?, l.as_f64().filter(|l| *l > 0.0)?);
            Some(c / (l / d))
        })
        .collect();
    (!works.is_empty()).then(|| works.iter().sum::<f64>() / works.len() as f64)
}

/// "Where an entity's time goes": one bar the length of the time in the system's parts, each
/// task's time worked on filled and its wait outlined (a shape as well as a colour), its name
/// under it where it fits, and a tooltip on each part. Worked on and waiting are two series, so
/// they take the chart palette's first two colours ([`series_color`]).
fn time_chart(ui: &mut egui::Ui, results: &Value, parts: &[Part], unit: &str, width: f32) {
    let p = theme::palette(ui.ctx());
    let series = [series_color(ui.ctx(), 0), series_color(ui.ctx(), 1)];
    // A hovered part moves a third of the way to the text colour: darker on a light theme,
    // lighter on a dark one, whatever the palette.
    let hovered = |c: Color32| c.lerp_to_gamma(p.text, 0.3);
    let sum: f64 = parts.iter().map(|x| x.work + x.wait).sum();
    if sum <= 0.0 {
        return;
    }
    let total = results["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|e| e["kind"] == "sink")
        .and_then(|e| e["metrics"]["mean_time_in_system"]["mean"].as_f64());
    let time = |v: f64| format!("{} {unit}", grouped(round_sig(v, 3)));
    ui.label(
        theme::strong(tf(
            "Where an entity's {total} in the system goes",
            &[("total", &time(total.unwrap_or(sum)))],
        ))
        .size(13.0),
    );
    ui.add_space(6.0);
    let bar_h = 22.0;
    let label_h = 34.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, bar_h + label_h), egui::Sense::hover());
    let painter = ui.painter_at(rect.expand(2.0));
    let scale = width as f64 / sum;
    let mut x = rect.left();
    for (i, part) in parts.iter().enumerate() {
        let start = x;
        // Worked on, filled; then waiting, outlined: 2 px of surface between parts.
        for (j, (v, filled)) in [(part.work, true), (part.wait, false)]
            .into_iter()
            .enumerate()
        {
            let w = (v * scale) as f32;
            if w < 1.0 {
                continue;
            }
            let seg = Rect::from_min_size(pos2(x, rect.top()), vec2((w - 2.0).max(1.0), bar_h));
            let hover = ui.rect_contains_pointer(seg);
            let colour = if hover { hovered(series[j]) } else { series[j] };
            if filled {
                painter.rect_filled(seg, 2.0, colour);
            } else {
                painter.rect_stroke(
                    seg.shrink(0.75),
                    2.0,
                    Stroke::new(1.5, colour),
                    egui::StrokeKind::Inside,
                );
            }
            if hover {
                let tip = tf(
                    "{block}: worked on {work}, waiting {wait}",
                    &[
                        ("block", &part.name),
                        ("work", &time(part.work)),
                        ("wait", &time(part.wait)),
                    ],
                );
                egui::Tooltip::always_open(
                    ui.ctx().clone(),
                    ui.layer_id(),
                    egui::Id::unique(("time-tip", i, j)),
                    egui::PopupAnchor::Pointer,
                )
                .show(|ui| {
                    ui.label(tip);
                });
            }
            x += w;
        }
        // The task's name and its two times under its part, where they fit.
        let room = x - start - 4.0;
        if room >= 56.0 {
            painter.text(
                pos2(start, rect.top() + bar_h + 4.0),
                Align2::LEFT_TOP,
                truncate(&part.name, room, ui),
                FontId::proportional(12.0),
                p.text,
            );
            painter.text(
                pos2(start, rect.top() + bar_h + 19.0),
                Align2::LEFT_TOP,
                format!(
                    "{} + {}",
                    grouped(round_sig(part.work, 3)),
                    grouped(round_sig(part.wait, 3))
                ),
                FontId::proportional(11.0),
                p.text_weak,
            );
        }
    }
    // The legend: the two marks and their words.
    ui.horizontal(|ui| {
        for (colour, filled, label) in [
            (series[0], true, t("Worked on")),
            (series[1], false, t("Waiting")),
        ] {
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
            if filled {
                ui.painter().rect_filled(r, 2.0, colour);
            } else {
                ui.painter().rect_stroke(
                    r.shrink(0.75),
                    2.0,
                    Stroke::new(1.5, colour),
                    egui::StrokeKind::Inside,
                );
            }
            ui.label(theme::weak(ui, label));
            ui.add_space(10.0);
        }
    });
    // The parts are what the tasks account for; time blocked or between tasks is not in them.
    if let Some(total) = total.filter(|t| *t > 0.0 && ((sum - t) / t).abs() > 0.05) {
        ui.label(theme::weak(
            ui,
            tf(
                "The parts add up to {sum}; the rest of the {total} is time blocked or between blocks.",
                &[("sum", &time(sum)), ("total", &time(total))],
            ),
        ));
    }
}

/// Whether the mean number waiting before `block` is 0 at every time recorded (a line that says
/// nothing, so another chart takes its place). Without the series: no.
fn queue_never_forms(results: &Value, block: &Value) -> bool {
    let id = block["id"].as_str().unwrap_or("");
    let mean = results["series"]["elements"][id]["queue_length"]["mean"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect::<Vec<_>>());
    mean.is_some_and(|m| !m.is_empty() && m.iter().all(|v| *v == 0.0))
}

/// "Waiting before {block} over the run": the mean number waiting at the busiest task at each
/// recorded time across the runs, with its 95% band, the warm-up shaded, a crosshair and a
/// tooltip on hover, and a note where it is still settling at the end.
fn over_run_chart(
    ui: &mut egui::Ui,
    results: &Value,
    block: &Value,
    unit: &str,
    width: f32,
) -> bool {
    let p = theme::palette(ui.ctx());
    let id = block["id"].as_str().unwrap_or("");
    let s = &results["series"];
    let q = &s["elements"][id]["queue_length"];
    let times: Vec<f64> = s["times"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    let mean: Vec<f64> = q["mean"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    if times.len() < 2 || mean.len() != times.len() {
        return false;
    }
    let hw: Vec<Option<f64>> = (0..times.len())
        .map(|i| q["half_width"][i].as_f64())
        .collect();
    let name = block["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .unwrap_or(id);
    ui.label(
        theme::strong(tf(
            "Waiting before {block} over the run",
            &[("block", name)],
        ))
        .size(13.0),
    );
    ui.label(theme::weak(ui, t("Mean number waiting, with its 95% band")).size(11.0));
    ui.add_space(4.0);

    let axis_w = 40.0;
    let axis_h = 18.0;
    let height = 150.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, height + axis_h), egui::Sense::hover());
    let plot = Rect::from_min_max(
        pos2(rect.left() + axis_w, rect.top() + 4.0),
        pos2(rect.right() - 8.0, rect.bottom() - axis_h),
    );
    let t_end = *times.last().unwrap_or(&1.0);
    let top = mean
        .iter()
        .zip(&hw)
        .map(|(m, h)| m + h.unwrap_or(0.0))
        .fold(0.0_f64, f64::max);
    let (step, y_top) = axis(top);
    let x_step = nice_step(t_end, 4.0);
    let x_top = t_end;
    let x = |v: f64| plot.left() + (v / x_top).clamp(0.0, 1.0) as f32 * plot.width();
    let y = |v: f64| plot.bottom() - (v / y_top).clamp(0.0, 1.0) as f32 * plot.height();
    let painter = ui.painter_at(rect.expand(2.0));

    // The warm-up, shaded and named.
    let warmup = results["record"]["warmup"].as_f64().unwrap_or(0.0);
    if warmup > 0.0 {
        let shade = Rect::from_min_max(
            pos2(plot.left(), plot.top()),
            pos2(x(warmup), plot.bottom()),
        );
        painter.rect_filled(shade, 0.0, p.stripe);
        painter.text(
            pos2(shade.left() + 4.0, plot.top() + 2.0),
            Align2::LEFT_TOP,
            t("Warm-up"),
            FontId::proportional(11.0),
            p.text_weak,
        );
    }
    // A recessive grid with its values.
    let grid = Stroke::new(1.0, p.border);
    let mut v = 0.0;
    while v <= y_top + step * 0.01 {
        painter.line_segment([pos2(plot.left(), y(v)), pos2(plot.right(), y(v))], grid);
        painter.text(
            pos2(plot.left() - 6.0, y(v)),
            Align2::RIGHT_CENTER,
            grouped(v),
            FontId::proportional(11.0),
            p.text_weak,
        );
        v += step;
    }
    let mut v = 0.0;
    while v <= x_top + x_step * 0.01 {
        painter.text(
            pos2(x(v), plot.bottom() + 3.0),
            // The last label ends at the plot's edge, not centred on it (half of it would be clipped).
            if v + x_step * 0.5 > x_top {
                Align2::RIGHT_TOP
            } else {
                Align2::CENTER_TOP
            },
            format!("{} {unit}", grouped(v)),
            FontId::proportional(11.0),
            p.text_weak,
        );
        v += x_step;
    }
    // The 95% band, as quads between consecutive times that have one.
    let band = p.chart.linear_multiply(0.22);
    for i in 1..times.len() {
        if let (Some(a), Some(b)) = (hw[i - 1], hw[i]) {
            let pts = vec![
                pos2(x(times[i - 1]), y(mean[i - 1] + a)),
                pos2(x(times[i]), y(mean[i] + b)),
                pos2(x(times[i]), y((mean[i] - b).max(0.0))),
                pos2(x(times[i - 1]), y((mean[i - 1] - a).max(0.0))),
            ];
            painter.add(egui::Shape::convex_polygon(pts, band, Stroke::NONE));
        }
    }
    // The mean, a 2 px line.
    let line: Vec<egui::Pos2> = times
        .iter()
        .zip(&mean)
        .map(|(t, m)| pos2(x(*t), y(*m)))
        .collect();
    painter.add(egui::Shape::line(line, Stroke::new(2.0, p.chart)));

    // A crosshair at the nearest recorded time, with its numbers.
    if let Some(pointer) = response
        .hover_pos()
        .filter(|pt| plot.x_range().contains(pt.x))
    {
        let at = ((pointer.x - plot.left()) / plot.width()) as f64 * x_top;
        let i = times
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - at).abs().total_cmp(&(b.1 - at).abs()))
            .map_or(0, |(i, _)| i);
        let cx = x(times[i]);
        painter.line_segment(
            [pos2(cx, plot.top()), pos2(cx, plot.bottom())],
            Stroke::new(1.0, p.text_weak),
        );
        painter.circle(
            pos2(cx, y(mean[i])),
            4.0,
            p.chart,
            Stroke::new(2.0, p.raised),
        );
        let value = match hw[i] {
            Some(h) => {
                let (m, h) = rounded(mean[i], h);
                format!("{m} ± {h}")
            }
            None => grouped(mean[i]),
        };
        let tip = tf(
            "At {time}: {value} waiting",
            &[
                ("time", &format!("{} {unit}", grouped(times[i]))),
                ("value", &value),
            ],
        );
        egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            egui::Id::unique("over-run-tip"),
            egui::PopupAnchor::Pointer,
        )
        .show(|ui| {
            ui.label(tip);
        });
    }
    // Still settling where MSER-5 found no point after which this task is steady.
    let settling = results["warmup_suggestion"]["elements"][id]
        .as_object()
        .is_some_and(|e| e.get("warmup").is_some_and(Value::is_null));
    if settling {
        ui.label(theme::weak(
            ui,
            t("Still settling at the end of the run: run it longer to see where it levels off."),
        ));
    }
    true
}

fn chart(ui: &mut egui::Ui, m: &Measure, bars: &[Bar], unit: &str, width: f32) {
    let p = theme::palette(ui.ctx());
    let suffix = if m.share {
        " %".to_owned()
    } else if m.time && !unit.is_empty() {
        format!(" {unit}")
    } else {
        String::new()
    };
    let title = if suffix.is_empty() {
        t(m.title).to_owned()
    } else {
        format!("{} ({})", t(m.title), suffix.trim())
    };
    ui.label(theme::strong(title).size(13.0));
    ui.add_space(4.0);

    let label_w = (width * 0.32).clamp(90.0, 160.0);
    let value_w = 76.0;
    let row_h = 26.0;
    let thick = 16.0;
    let axis_h = 18.0;
    let height = bars.len() as f32 * row_h + axis_h;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), egui::Sense::hover());
    let plot = Rect::from_min_max(
        pos2(rect.left() + label_w, rect.top()),
        pos2(rect.right() - value_w, rect.bottom() - axis_h),
    );
    let (step, top_value) = if m.share {
        (25.0, 100.0)
    } else {
        axis(
            bars.iter()
                .map(|b| b.mean + b.half_width)
                .fold(0.0_f64, f64::max),
        )
    };
    let x = |v: f64| plot.left() + (v / top_value).clamp(0.0, 1.0) as f32 * plot.width();
    let painter = ui.painter_at(rect.expand(2.0));

    // The grid: recessive vertical lines at round values, labelled under the plot.
    let grid = Stroke::new(1.0, p.border);
    let mut t = 0.0;
    while t <= top_value + step * 0.01 {
        let gx = x(t);
        painter.line_segment([pos2(gx, plot.top()), pos2(gx, plot.bottom())], grid);
        painter.text(
            pos2(gx, plot.bottom() + 3.0),
            Align2::CENTER_TOP,
            grouped(t),
            FontId::proportional(11.0),
            p.text_weak,
        );
        t += step;
    }
    // The baseline.
    painter.line_segment(
        [
            pos2(plot.left(), plot.top()),
            pos2(plot.left(), plot.bottom()),
        ],
        Stroke::new(1.0, p.text_weak),
    );

    for (i, b) in bars.iter().enumerate() {
        let cy = plot.top() + (i as f32 + 0.5) * row_h;
        let row = Rect::from_min_max(
            pos2(rect.left(), cy - row_h / 2.0),
            pos2(rect.right(), cy + row_h / 2.0),
        );
        let hover = ui.rect_contains_pointer(row);
        painter.text(
            pos2(rect.left(), cy),
            Align2::LEFT_CENTER,
            truncate(&b.name, label_w - 8.0, ui),
            FontId::proportional(13.0),
            p.text,
        );
        let end = x(b.mean).max(plot.left() + 1.0);
        let bar = Rect::from_min_max(
            pos2(plot.left(), cy - thick / 2.0),
            pos2(end, cy + thick / 2.0),
        );
        let fill = if hover { p.chart_hover } else { p.chart };
        painter.rect_filled(
            bar,
            egui::CornerRadius {
                nw: 0,
                sw: 0,
                ne: 4,
                se: 4,
            },
            fill,
        );
        // The 95% confidence interval as a whisker over the bar's end.
        if b.half_width > 0.0 {
            let (lo, hi) = (x(b.mean - b.half_width), x(b.mean + b.half_width));
            let ink = Stroke::new(1.5, p.text);
            painter.line_segment([pos2(lo, cy), pos2(hi, cy)], ink);
            for wx in [lo, hi] {
                painter.line_segment([pos2(wx, cy - 5.0), pos2(wx, cy + 5.0)], ink);
            }
        }
        painter.text(
            pos2(plot.right() + 8.0, cy),
            Align2::LEFT_CENTER,
            // Rounded to what its interval supports, as in the tables.
            rounded(b.mean, b.half_width).0,
            FontId::monospace(12.0),
            p.text,
        );
        if hover {
            let tip = tnf(
                b.n,
                "{name}: {mean}{unit}\n95% interval {low} to {high}{unit}\nacross {n} run",
                "{name}: {mean}{unit}\n95% interval {low} to {high}{unit}\nacross {n} runs",
                &[
                    ("name", &b.name),
                    ("mean", &rounded(b.mean, b.half_width).0),
                    ("low", &rounded(b.mean - b.half_width, b.half_width).0),
                    ("high", &rounded(b.mean + b.half_width, b.half_width).0),
                    ("unit", &suffix),
                ],
            );
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                egui::Id::unique(("chart-tip", m.metric, i)),
                egui::PopupAnchor::Pointer,
            )
            .show(|ui| {
                ui.label(tip);
            });
        }
    }
}

/// `text`, shortened with an ellipsis to fit `width`.
pub(crate) fn truncate(text: &str, width: f32, ui: &egui::Ui) -> String {
    let font = FontId::proportional(13.0);
    let fits = |s: &str| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
        }) <= width
    };
    if fits(text) {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let s: String = chars.iter().collect::<String>() + "…";
        if fits(&s) {
            return s;
        }
    }
    "…".into()
}

/// The pixels of `area` (in points) cut from a screenshot taken at `pixels_per_point`, as PNG.
pub fn png(image: &egui::ColorImage, area: Rect, pixels_per_point: f32) -> Option<Vec<u8>> {
    let [w, h] = image.size;
    let px = |v: f32| (v * pixels_per_point).round().max(0.0) as usize;
    let (x0, y0) = (px(area.left()).min(w), px(area.top()).min(h));
    let (x1, y1) = (px(area.right()).min(w), px(area.bottom()).min(h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut rgba = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        for x in x0..x1 {
            rgba.extend_from_slice(&image.pixels[y * w + x].to_srgba_unmultiplied());
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, (x1 - x0) as u32, (y1 - y0) as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entitys_time_splits_into_work_and_waiting_by_littles_law() {
        let m = |mean: f64| serde_json::json!({ "mean": mean });
        let results = serde_json::json!({
            "record": { "duration": 20000.0 },
            "measured": { "mean_duration": 20000.0 },
            "elements": [{ "id": "serve", "name": "Serve", "kind": "server", "metrics": {
                "left": m(15000.0), "mean_contents": m(0.75), "mean_wait": m(3.0),
            } }],
        });
        let parts = super::parts(&results, &["serve".to_owned()]).unwrap_or_default();
        assert_eq!(parts.len(), 1);
        // 0.75 in service at 0.75 per minute through: 1 minute worked on, then 3 waiting.
        assert!((parts[0].work - 1.0).abs() < 1e-12);
        assert_eq!(parts[0].wait, 3.0);
        // A task with nothing through it has no parts.
        let mut none = results.clone();
        none["elements"][0]["metrics"]["left"] = m(0.0);
        assert!(super::parts(&none, &["serve".to_owned()]).is_none());
    }

    #[test]
    fn work_is_worked_out_run_by_run_when_runs_measure_different_times() {
        // Two runs measuring 10 and 100: 10 and 50 left, 1 and 0.5 in service, so 1 worked on
        // in each. The means of the parts (0.75 in service, 30 left in 55) would give 1.375.
        let results = serde_json::json!({
            "measured": { "mean_duration": 55.0, "spans": [
                { "from": 0.0, "to": 10.0, "used": true },
                { "from": 0.0, "to": 100.0, "used": true },
            ] },
            "elements": [{ "id": "serve", "name": "Serve", "kind": "server", "metrics": {
                "left": { "mean": 30.0 }, "mean_contents": { "mean": 0.75 }, "mean_wait": { "mean": 2.0 },
            } }],
            "values": { "serve": { "mean_contents": [1.0, 0.5], "left": [10.0, 50.0] } },
        });
        let parts = super::parts(&results, &["serve".to_owned()]).unwrap_or_default();
        assert_eq!(parts.len(), 1);
        assert!((parts[0].work - 1.0).abs() < 1e-12, "{}", parts[0].work);
    }

    /// The fills and outline colours one frame of `time_chart` paints with `palette` in use.
    fn time_chart_colours(
        palette: &crate::chart_palette::ChartPalette,
    ) -> (Vec<Color32>, Vec<Color32>) {
        let ctx = egui::Context::default();
        let parts = [
            Part {
                name: "Serve".into(),
                work: 1.0,
                wait: 3.0,
            },
            Part {
                name: "Pack".into(),
                work: 2.0,
                wait: 2.0,
            },
        ];
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::chart_palette::use_palette(ui.ctx(), palette, theme::palette(ui.ctx()).canvas);
            time_chart(ui, &Value::Null, &parts, "min", 600.0);
        });
        // Nothing is rendered: the font textures egui made are not needed.
        output.textures_delta.clear();
        let (mut fills, mut strokes) = (Vec::new(), Vec::new());
        for clipped in &output.shapes {
            if let egui::Shape::Rect(r) = &clipped.shape {
                if r.fill != Color32::TRANSPARENT {
                    fills.push(r.fill);
                }
                if r.stroke.width > 0.0 {
                    strokes.push(r.stroke.color);
                }
            }
        }
        (fills, strokes)
    }

    #[test]
    fn worked_on_and_waiting_take_the_chosen_palettes_first_two_colours() {
        let ctx = egui::Context::default();
        for name in ["Tol bright", "ColorBrewer Dark2"] {
            let palette = crate::chart_palette::find(name, &[]);
            assert_eq!(palette.name, name);
            let canvas = theme::palette(&ctx).canvas;
            let order = crate::chart_palette::draw_order(&palette, canvas);
            let (first, second) = (palette.colors[order[0]], palette.colors[order[1]]);
            let (fills, strokes) = time_chart_colours(&palette);
            // Two worked-on parts and the legend's swatch filled with the first colour; two
            // waits and the legend's swatch outlined with the second.
            assert_eq!(
                fills.iter().filter(|c| **c == first).count(),
                3,
                "{name}: {fills:?}"
            );
            assert_eq!(
                strokes.iter().filter(|c| **c == second).count(),
                3,
                "{name}: {strokes:?}"
            );
            // Nothing in the theme's single-series hue.
            let theme_hue = theme::palette(&ctx).chart;
            assert!(!fills.contains(&theme_hue) && !strokes.contains(&theme_hue));
        }
    }

    #[test]
    fn gridlines_fall_on_round_values() {
        assert_eq!(nice_step(4.1, 4.0), 2.0);
        assert_eq!(nice_step(100.0, 4.0), 50.0);
        assert_eq!(nice_step(0.07, 4.0), 0.02);
        assert_eq!(nice_step(15000.0, 4.0), 5000.0);
        // A size no step fits ends the search at once.
        assert_eq!(nice_step(f64::INFINITY, 4.0), 1.0);
        assert_eq!(nice_step(f64::NAN, 4.0), 1.0);
        assert_eq!(nice_step(1.0, 0.0), 1.0);
        assert!(nice_step(1e308, 4.0).is_finite());
    }

    #[test]
    fn an_axis_of_zeros_runs_from_0_to_1() {
        assert_eq!(axis(0.0), (0.5, 1.0));
        assert_eq!(axis(f64::NAN), (0.5, 1.0));
        assert_eq!(axis(-3.0), (0.5, 1.0));
        assert_eq!(grouped(axis(0.0).1), "1");
        // Values above zero keep their own scale, however small.
        assert_eq!(axis(4.1), (2.0, 6.0));
        assert_eq!(axis(0.95052), (0.5, 1.0));
        let (step, top) = axis(3e-6);
        assert!(
            top >= 3e-6 && step > 0.0 && top / step <= 5.0,
            "{step} {top}"
        );
    }

    #[test]
    fn a_cut_of_a_screenshot_is_a_png() {
        let image = egui::ColorImage::new([20, 10], vec![egui::Color32::RED; 200]);
        let bytes = png(
            &image,
            Rect::from_min_max(pos2(2.0, 1.0), pos2(6.0, 4.0)),
            2.0,
        )
        .unwrap_or_default();
        assert_eq!(&bytes[1..4], b"PNG");
        // Width 8 px, height 6 px (twice the points).
        assert_eq!(&bytes[16..24], &[0, 0, 0, 8, 0, 0, 0, 6]);
    }
}
