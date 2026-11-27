//! "Choose a warm-up" under the results: Welch's plot of a block's quantity over the run (the
//! mean across runs at each recorded time, smoothed by Welch's moving average with a window the
//! user sets; `stochcraft_stats::warmup`), with MSER-5's cut on the same series marked, the
//! current warm-up shaded, and a new warm-up chosen on the plot or typed, which the experiment
//! then takes (Law, *Simulation Modeling and Analysis*, 5th ed., section 9.5.1).

use egui::{Align2, FontId, Rect, Stroke, pos2, vec2};
use serde_json::Value;
use stochcraft_stats::warmup::{Truncation, mser5, welch_max_window, welch_moving_average};

use crate::charts::{axis, nice_step};
use crate::decide::{choose, number};
use crate::i18n::{t, tf};
use crate::results::{grouped, round_warmup, unit_label};
use crate::theme;

/// The section's choices.
#[derive(Default)]
pub(crate) struct Welch {
    /// The element and quantity plotted: an element id, and a key of [`QUANTITIES`].
    element: Option<String>,
    quantity: Option<String>,
    /// Points each side of the moving average's window.
    window: Option<f64>,
    /// The warm-up to set, with the results, element and quantity it was chosen for: another
    /// series starts again from its own MSER-5 cut.
    chosen: Option<(String, f64)>,
}

/// The quantities a plot can show: the key, the quantity recorded, whether it is a count since
/// time 0 shown as the number in each interval (such a count never levels off; its rate does,
/// like the hourly throughput of Law's Example 9.25), and the label.
const QUANTITIES: &[(&str, &str, bool, &str)] = &[
    (
        "work_in_progress",
        "work_in_progress",
        false,
        "Number inside",
    ),
    ("queue_length", "queue_length", false, "Number waiting"),
    ("busy", "busy", false, "Busy units"),
    ("left_per_interval", "left", true, "Leaving per interval"),
    (
        "entered_per_interval",
        "entered",
        true,
        "Entering per interval",
    ),
];

/// The plotted series: its times and values, cut where fewer runs reached a time than the first.
fn series(results: &Value, element: &str, quantity: &str) -> Option<(Vec<f64>, Vec<f64>, bool)> {
    let s = &results["series"];
    let floats = |v: &Value| -> Vec<f64> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .collect()
    };
    let times = floats(&s["times"]);
    let n: Vec<u64> = s["n"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect();
    let &(_, recorded, per_interval, _) = QUANTITIES.iter().find(|q| q.0 == quantity)?;
    let mean = floats(&s["elements"][element][recorded]["mean"]);
    if times.is_empty() || mean.len() != times.len() {
        return None;
    }
    // Every run to the end, or as far as the first run count holds.
    let full = n.first().copied();
    let reach = n
        .iter()
        .position(|k| Some(*k) != full)
        .unwrap_or(times.len());
    let cut = reach < times.len();
    let (times, mean) = (&times[..reach], &mean[..reach]);
    if per_interval {
        let per: Vec<f64> = mean.windows(2).map(|w| w[1] - w[0]).collect();
        Some((times.get(1..)?.to_vec(), per, cut))
    } else {
        Some((times.to_vec(), mean.to_vec(), cut))
    }
}

impl crate::App {
    /// "Choose a warm-up": returns the warm-up to set when its button is clicked.
    pub(crate) fn warmup_section(&mut self, ui: &mut egui::Ui, results: &Value) -> Option<f64> {
        let s = &results["series"];
        let recorded = s["elements"].as_object()?;
        // The blocks with a quantity to plot, in the results' order, by name.
        let options: Vec<(String, String)> = results["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let id = e["id"].as_str()?;
                let q = recorded.get(id)?.as_object()?;
                QUANTITIES
                    .iter()
                    .any(|(_, r, _, _)| q.contains_key(*r))
                    .then(|| {
                        let name = e["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
                        (id.to_owned(), name.to_owned())
                    })
            })
            .collect();
        if options.is_empty() {
            return None;
        }
        crate::editor::section(ui, t("Choose a warm-up"));
        ui.label(theme::weak(
            ui,
            t("Welch's plot: the mean across runs at each recorded time, smoothed by a moving average. Set the warm-up where the curve levels off; MSER-5's cut is only a starting point."),
        ));
        ui.add_space(4.0);

        // The block: as chosen, else where MSER-5's suggestion comes from, else the busiest.
        let at = results["warmup_suggestion"]["at"].as_str();
        let busiest = crate::results::busiest(results).and_then(|b| b["id"].as_str());
        let mut element = self
            .welch
            .element
            .clone()
            .filter(|e| options.iter().any(|(k, _)| k == e))
            .or_else(|| {
                at.or(busiest)
                    .map(str::to_owned)
                    .filter(|e| options.iter().any(|(k, _)| k == e))
            })
            .or_else(|| options.first().map(|(k, _)| k.clone()));
        let unit = unit_label(results["record"]["time_unit"].as_str().unwrap_or("")).to_owned();
        ui.horizontal_wrapped(|ui| {
            choose(ui, "welch/block", t("Block"), &options, &mut element, 220.0);
        });
        let id = element.clone()?;
        let has = &recorded[&id];
        let quantities: Vec<(String, String)> = QUANTITIES
            .iter()
            .filter(|(_, r, _, _)| has.get(*r).is_some())
            .map(|(k, _, _, l)| ((*k).to_owned(), t(l).to_owned()))
            .collect();
        let mut quantity = self
            .welch
            .quantity
            .clone()
            .filter(|q| quantities.iter().any(|(k, _)| k == q))
            .or_else(|| quantities.first().map(|(k, _)| k.clone()));
        ui.horizontal_wrapped(|ui| {
            choose(
                ui,
                "welch/quantity",
                t("Measure"),
                &quantities,
                &mut quantity,
                220.0,
            );
        });
        let q = quantity.clone()?;
        self.welch.element = Some(id.clone());
        self.welch.quantity = Some(q.clone());

        let (times, y, cut_short) = series(results, &id, &q)?;
        let max_w = welch_max_window(y.len());
        if max_w == 0 {
            ui.label(theme::weak(
                ui,
                t("The run recorded too few times to plot."),
            ));
            return None;
        }
        let mut w = self
            .welch
            .window
            .unwrap_or((y.len() / 50) as f64)
            .round()
            .clamp(1.0, max_w as f64);
        ui.horizontal_wrapped(|ui| {
            number(
                ui,
                &self.numbers,
                "welch/window",
                t("Points each side"),
                &mut w,
                70.0,
            );
            ui.label(theme::weak(
                ui,
                tf("1 to {max}", &[("max", &max_w.to_string())]),
            ));
        });
        let w = w.round().clamp(1.0, max_w as f64);
        self.welch.window = Some(w);
        let smooth = welch_moving_average(&y, w as usize)?;
        let shown = &times[..smooth.len()];

        // MSER-5 on the same series, unsmoothed: the time of its first point kept.
        let mser = match mser5(&y) {
            Some(Truncation::At { observations }) => times.get(observations).copied(),
            _ => None,
        };
        let current = results["record"]["warmup"].as_f64().unwrap_or(0.0);
        let key = format!("{}/{id}/{q}", results["record"]["model_digest"]);
        let mut chosen = match &self.welch.chosen {
            Some((k, v)) if *k == key => *v,
            _ => mser.map_or(current, round_warmup),
        };

        let name = options
            .iter()
            .find(|(k, _)| *k == id)
            .map_or(id.as_str(), |(_, n)| n.as_str());
        let measure = quantities
            .iter()
            .find(|(k, _)| *k == q)
            .map_or("", |(_, l)| l.as_str());
        if let Some(t_clicked) = plot(
            ui,
            shown,
            &smooth,
            mser,
            current,
            chosen,
            &unit,
            &tf(
                "Welch's plot of {measure} at {block}",
                &[("measure", measure), ("block", name)],
            ),
        ) {
            chosen = t_clicked;
        }
        let time = |v: f64| format!("{} {unit}", grouped(v)).trim_end().to_owned();
        ui.horizontal_wrapped(|ui| {
            let text = match mser {
                Some(m) => tf("MSER-5 suggests {value}.", &[("value", &time(m))]),
                None => t("MSER-5 suggests nothing here: the series may still be settling halfway through the run.").to_owned(),
            };
            ui.label(text);
            ui.label(tf("The warm-up now is {value}.", &[("value", &time(current))]));
        });
        if cut_short {
            ui.label(theme::weak(
                ui,
                t("Later times, which fewer runs reached, are left out."),
            ));
        }
        let mut set = None;
        ui.horizontal_wrapped(|ui| {
            number(
                ui,
                &self.numbers,
                "welch/chosen",
                t("New warm-up"),
                &mut chosen,
                90.0,
            );
            chosen = chosen.max(0.0);
            let label = tf("Set a {value} warm-up", &[("value", &time(chosen))]);
            if ui.add(theme::secondary(ui, &label)).clicked() {
                set = Some(chosen);
            }
        });
        self.welch.chosen = Some((key, chosen));
        set
    }
}

/// Draws the moving average over time with MSER-5's cut, the current warm-up and the chosen one;
/// returns the recorded time nearest a click on it.
#[allow(clippy::too_many_arguments)]
fn plot(
    ui: &mut egui::Ui,
    times: &[f64],
    smooth: &[f64],
    mser: Option<f64>,
    current: f64,
    chosen: f64,
    unit: &str,
    label: &str,
) -> Option<f64> {
    let p = theme::palette(ui.ctx());
    let width = ui.available_width().min(1100.0);
    let axis_w = 48.0;
    let axis_h = 18.0;
    let height = 180.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, height + axis_h), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, label));
    let plot = Rect::from_min_max(
        pos2(rect.left() + axis_w, rect.top() + 14.0),
        pos2(rect.right() - 8.0, rect.bottom() - axis_h),
    );
    let x_top = times.last().copied().filter(|t| *t > 0.0).unwrap_or(1.0);
    let top = smooth.iter().copied().fold(0.0_f64, f64::max);
    let bottom = smooth.iter().copied().fold(0.0_f64, f64::min);
    let (step, y_top) = axis(top);
    let y_bottom = if bottom < 0.0 { -axis(-bottom).1 } else { 0.0 };
    let x = |v: f64| plot.left() + (v / x_top).clamp(0.0, 1.0) as f32 * plot.width();
    let y = |v: f64| {
        plot.bottom() - ((v - y_bottom) / (y_top - y_bottom)).clamp(0.0, 1.0) as f32 * plot.height()
    };
    let painter = ui.painter_at(rect.expand(2.0));
    let small = FontId::proportional(11.0);

    // The current warm-up, shaded and named.
    if current > 0.0 {
        let shade = Rect::from_min_max(
            pos2(plot.left(), plot.top()),
            pos2(x(current), plot.bottom()),
        );
        painter.rect_filled(shade, 0.0, p.stripe);
        painter.text(
            pos2(shade.left() + 4.0, plot.top() + 2.0),
            Align2::LEFT_TOP,
            t("Warm-up"),
            small.clone(),
            p.text_weak,
        );
    }
    // A recessive grid with its values.
    let grid = Stroke::new(1.0, p.border);
    let mut v = y_bottom;
    while v <= y_top + step * 0.01 {
        painter.line_segment([pos2(plot.left(), y(v)), pos2(plot.right(), y(v))], grid);
        painter.text(
            pos2(plot.left() - 6.0, y(v)),
            Align2::RIGHT_CENTER,
            grouped(v),
            small.clone(),
            p.text_weak,
        );
        v += step;
    }
    let x_step = nice_step(x_top, 4.0);
    let mut v = 0.0;
    while v <= x_top + x_step * 0.01 {
        painter.text(
            pos2(x(v), plot.bottom() + 3.0),
            if v + x_step * 0.5 > x_top {
                Align2::RIGHT_TOP
            } else {
                Align2::CENTER_TOP
            },
            format!("{} {unit}", grouped(v)),
            small.clone(),
            p.text_weak,
        );
        v += x_step;
    }
    // MSER-5's cut, a dashed line named at its top.
    if let Some(m) = mser {
        let mx = x(m);
        painter.extend(egui::Shape::dashed_line(
            &[pos2(mx, plot.top()), pos2(mx, plot.bottom())],
            Stroke::new(1.5, p.text_weak),
            4.0,
            3.0,
        ));
        painter.text(
            pos2(mx + 3.0, plot.top() - 13.0),
            Align2::LEFT_TOP,
            "MSER-5",
            small.clone(),
            p.text_weak,
        );
    }
    // The moving average, a 2 px line.
    let line: Vec<egui::Pos2> = times
        .iter()
        .zip(smooth)
        .map(|(t, m)| pos2(x(*t), y(*m)))
        .collect();
    painter.add(egui::Shape::line(line, Stroke::new(2.0, p.chart)));
    // The new warm-up, in the accent colour.
    let cx = x(chosen);
    painter.line_segment(
        [pos2(cx, plot.top()), pos2(cx, plot.bottom())],
        Stroke::new(2.0, p.accent),
    );
    painter.text(
        pos2(cx - 3.0, plot.top() - 13.0),
        Align2::RIGHT_TOP,
        t("New warm-up"),
        small,
        p.accent,
    );

    let nearest = |px: f32| {
        let at = ((px - plot.left()) / plot.width()) as f64 * x_top;
        times
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - at).abs().total_cmp(&(b.1 - at).abs()))
            .map(|(i, _)| i)
    };
    // A guide at the nearest recorded time, with its numbers.
    if let Some(i) = response
        .hover_pos()
        .filter(|pt| plot.x_range().contains(pt.x))
        .and_then(|pt| nearest(pt.x))
    {
        let gx = x(times[i]);
        painter.line_segment(
            [pos2(gx, plot.top()), pos2(gx, plot.bottom())],
            Stroke::new(1.0, p.text_weak),
        );
        painter.circle(
            pos2(gx, y(smooth[i])),
            4.0,
            p.chart,
            Stroke::new(2.0, p.raised),
        );
        let tip = format!("{} {unit}: {}", grouped(times[i]), grouped(smooth[i]));
        egui::Tooltip::always_open(
            ui.ctx().clone(),
            ui.layer_id(),
            egui::Id::unique("welch-tip"),
            egui::PopupAnchor::Pointer,
        )
        .show(|ui| {
            ui.label(tip);
        });
    }
    response
        .clicked()
        .then(|| response.interact_pointer_pos())
        .flatten()
        .filter(|pt| plot.x_range().contains(pt.x))
        .and_then(|pt| nearest(pt.x))
        .map(|i| times[i])
}
