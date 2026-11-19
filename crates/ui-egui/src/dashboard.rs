//! The Dashboard panel (M5.4): four tiles beside the model, the panel people share. Throughput
//! (everything that left through the sinks, per hour), how each resource spends its time, how long
//! items spend on the conveyors, and how many are on them over time. They follow a run being
//! watched, and show the results of the last run once it has ended.
//!
//! Marks follow the dataviz rules, as in `charts`: thin marks with a 2 px gap between stacked
//! segments, a recessive grid, text in text colours, a legend for the several series, a tooltip
//! with the exact values on every chart, and the same numbers as text for screen readers.

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, pos2, vec2};
use serde_json::Value;
use stochcraft_model::{ElementKind, TimeUnit};

use crate::chart_palette::series_color;
use crate::charts::{axis, truncate};
use crate::i18n::{t, tf};
use crate::live::Glimpse;
use crate::results::{Unit, grouped, metric_label, round_sig, rounded, said};
use crate::{App, theme};

/// The most resources the utilisation tile draws; the rest are counted.
const MOST_BARS: usize = 8;
/// The histogram's bins.
const BINS: usize = 20;
/// Values this close (as a share of the larger) are one value.
const SAME: f64 = 1e-9;

/// The states a resource's time divides into, in the order they are drawn and coloured.
const STATES: [&str; 6] = [
    "time_processing",
    "time_setup",
    "time_blocked",
    "time_starved",
    "time_failed",
    "time_off_shift",
];

/// One resource's bar.
struct Row {
    name: String,
    /// The share of time in each of [`STATES`].
    shares: [f64; 6],
    /// The busy share, as the results say it ("65%").
    busy: String,
    busy_value: f64,
}

impl App {
    /// The Dashboard panel, on the right of the work area.
    pub(crate) fn dashboard_panel(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        egui::Panel::right("dashboard")
            .resizable(true)
            .default_size(400.0)
            .size_range(300.0..=700.0)
            .frame(
                egui::Frame::new()
                    .fill(p.raised)
                    .stroke(Stroke::new(1.0, p.border))
                    .inner_margin(egui::Margin::symmetric(14, 12)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.dashboard_tiles(ui, p));
            });
    }

    fn dashboard_tiles(&self, ui: &mut egui::Ui, p: &theme::Palette) {
        let model = self.engine.model();
        let unit = self.time_unit_label();
        let per_hour = per_hour(model.experiment.time_unit);
        let conveyors = model
            .elements
            .iter()
            .any(|e| matches!(e.kind, ElementKind::Conveyor { .. }));
        let results: Option<&Value> = self.results.as_deref();
        let glimpse: Option<Glimpse> = self.watching.as_ref().map(|w| w.glimpse());
        // What is left inside a tile: its margins and border, and the scroll bar.
        let width = (ui.available_width() - 44.0).max(120.0);

        ui.label(theme::strong(t("Dashboard")).size(16.0));
        if results.is_none() && glimpse.is_none() {
            ui.label(theme::weak(ui, t("Run the model to fill the dashboard")));
        } else if glimpse.is_some() && results.is_some() {
            ui.label(theme::weak(
                ui,
                t("The resource and conveyor charts show the last run until this one ends"),
            ));
        }
        ui.add_space(8.0);

        // 1. Throughput.
        tile(ui, p, t("Throughput"), |ui| {
            throughput(ui, results, glimpse.as_ref(), per_hour, model)
        });
        // 2. Utilisation by resource.
        tile(ui, p, t("Utilisation by resource"), |ui| {
            utilisation(ui, p, results, width);
            String::new()
        });
        // 3. Time on conveyors.
        tile(ui, p, t("Time on conveyors"), |ui| {
            if conveyors {
                time_on(ui, p, results, glimpse.is_some(), &unit, width);
                String::new()
            } else {
                let none = t("No conveyors in this model");
                ui.label(none);
                none.to_owned()
            }
        });
        // 4. Items on conveyors over time.
        let title = if conveyors {
            t("Items on conveyors over time")
        } else {
            t("Items in the model over time")
        };
        tile(ui, p, title, |ui| {
            over_time(
                ui,
                p,
                results,
                glimpse.as_ref(),
                conveyors,
                title,
                &unit,
                width,
            );
            String::new()
        });
    }
}

/// How many of the model's time units make an hour, inverted: units per hour.
fn per_hour(unit: TimeUnit) -> f64 {
    match unit {
        TimeUnit::Milliseconds => 3_600_000.0,
        TimeUnit::Seconds => 3_600.0,
        TimeUnit::Minutes => 60.0,
        TimeUnit::Hours => 1.0,
        TimeUnit::Days => 1.0 / 24.0,
        TimeUnit::Years => 1.0 / 8_760.0,
    }
}

/// A tile: a framed block with a title, labelled as a group for screen readers.
/// A tile: `title` above what `body` draws. `body` returns what its text says (or nothing), which
/// screen readers hear with the title, as the text inside a tile is not exposed on its own.
fn tile(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    title: &str,
    body: impl FnOnce(&mut egui::Ui) -> String,
) {
    let r = egui::Frame::new()
        .fill(p.canvas)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::strong(title).size(13.0));
            ui.add_space(4.0);
            body(ui)
        });
    let said = if r.inner.is_empty() {
        title.to_owned()
    } else {
        format!("{title}: {}", r.inner)
    };
    r.response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, &said));
    ui.add_space(8.0);
}

/// A tooltip at the pointer.
fn tip(ui: &egui::Ui, key: (&'static str, usize), text: String) {
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        egui::Id::unique(("dashboard-tip", key)),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.label(text);
    });
}

fn elements(results: &Value) -> &[Value] {
    results["elements"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// A text in the interface's number style, to three figures.
fn three(v: f64) -> String {
    grouped(round_sig(v, 3))
}

// ---- 1. Throughput ----

fn throughput(
    ui: &mut egui::Ui,
    results: Option<&Value>,
    glimpse: Option<&Glimpse>,
    per_hour: f64,
    model: &stochcraft_model::Model,
) -> String {
    if !model
        .elements
        .iter()
        .any(|e| matches!(e.kind, ElementKind::Sink { .. }))
    {
        let none = t("No sink in this model");
        ui.label(none);
        return none.to_owned();
    }
    // While a run is watched, the value so far; else the results'.
    if let Some(g) = glimpse {
        let rate = if g.measured > 0.0 {
            g.left / g.measured * per_hour
        } else {
            0.0
        };
        let rate = tf("{n} per hour", &[("n", &three(rate))]);
        ui.label(theme::strong(&rate).size(26.0));
        let note = t("So far in the run being watched");
        ui.label(theme::weak(ui, note));
        return format!("{rate}. {note}");
    }
    let Some(r) = results else {
        let about = t("Everything that left the model through its sinks, per hour.");
        ui.label(theme::weak(ui, about));
        return about.to_owned();
    };
    let measured = match &r["measured"] {
        m if m.is_object() => m["mean_duration"].as_f64(),
        _ => r["record"]["duration"].as_f64(),
    }
    .filter(|d| *d > 0.0);
    let (mut mean, mut half, mut any) = (0.0, 0.0, false);
    for e in elements(r).iter().filter(|e| e["kind"] == "sink") {
        let m = if e["metrics"]["left"]["mean"].is_number() {
            &e["metrics"]["left"]
        } else {
            &e["metrics"]["entered"]
        };
        if let Some(v) = m["mean"].as_f64() {
            mean += v;
            half += m["half_width"].as_f64().unwrap_or(0.0);
            any = true;
        }
    }
    let (Some(d), true) = (measured, any) else {
        let none = t("Nothing left the model in this run");
        ui.label(theme::weak(ui, none));
        return none.to_owned();
    };
    let scale = per_hour / d;
    let (mean, half) = (mean * scale, half * scale);
    let runs = r["record"]["replications"].as_u64().unwrap_or(1);
    if runs > 1 && half > 0.0 {
        let rate = tf("{n} per hour", &[("n", &rounded(mean, half).0)]);
        ui.label(theme::strong(&rate).size(26.0));
        let interval = tf(
            "95% interval {low} to {high} per hour",
            &[
                ("low", &rounded(mean - half, half).0),
                ("high", &rounded(mean + half, half).0),
            ],
        );
        ui.label(theme::weak(ui, &interval));
        format!("{rate}. {interval}")
    } else {
        let rate = tf("{n} per hour", &[("n", &three(mean))]);
        ui.label(theme::strong(&rate).size(26.0));
        rate
    }
}

// ---- 2. Utilisation by resource ----

/// The bars: the elements whose results hold how they spend their time.
fn rows(results: &Value) -> Vec<Row> {
    elements(results)
        .iter()
        .filter(|e| {
            let m = &e["metrics"];
            m["utilisation"]["mean"].is_number() || STATES.iter().any(|s| m[s]["mean"].is_number())
        })
        .map(|e| {
            let m = &e["metrics"];
            let share = |k: &str| m[k]["mean"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
            let mut shares = [
                share(STATES[0]),
                share(STATES[1]),
                share(STATES[2]),
                share(STATES[3]),
                share(STATES[4]),
                share(STATES[5]),
            ];
            // A resource pool reports only its utilisation: its busy time is all processing.
            if !STATES.iter().any(|s| m[s]["mean"].is_number()) {
                shares[0] = share("utilisation");
            }
            let busy_value = m["utilisation"]["mean"]
                .as_f64()
                .unwrap_or(shares[0] + shares[1]);
            let busy = said(&m["utilisation"], &Unit::Share, "")
                .unwrap_or_else(|| format!("{}%", three(busy_value * 100.0)));
            Row {
                name: crate::results::block_name(e).to_owned(),
                shares,
                busy,
                busy_value,
            }
        })
        .collect()
}

fn utilisation(ui: &mut egui::Ui, p: &theme::Palette, results: Option<&Value>, width: f32) {
    let Some(r) = results else {
        ui.label(theme::weak(
            ui,
            t("How each resource spends its time, and its busy share."),
        ));
        return;
    };
    let mut all = rows(r);
    if all.is_empty() {
        ui.label(t("No resource reports how it spends its time"));
        return;
    }
    let more = all.len().saturating_sub(MOST_BARS);
    if more > 0 {
        all.sort_by(|a, b| b.busy_value.total_cmp(&a.busy_value));
        all.truncate(MOST_BARS);
    }
    let colours: Vec<Color32> = (0..STATES.len())
        .map(|i| series_color(ui.ctx(), i))
        .collect();
    let idle_colour = p.border;
    let label_w = (width * 0.34).clamp(80.0, 150.0);
    let value_w = 44.0;
    let bar_h = 14.0;
    let row_h = 24.0;
    let labels: Vec<String> = STATES
        .iter()
        .map(|s| metric_label(s, "server").to_owned())
        .collect();

    for (i, row) in all.iter().enumerate() {
        let (rect, response) = ui.allocate_exact_size(vec2(width, row_h), egui::Sense::hover());
        let idle = (1.0 - row.shares.iter().sum::<f64>()).max(0.0);
        let text = tf(
            "{name}: {busy} busy",
            &[("name", &row.name), ("busy", &row.busy)],
        );
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &text));
        let painter = ui.painter_at(rect.expand(2.0));
        let cy = rect.center().y;
        painter.text(
            pos2(rect.left(), cy),
            Align2::LEFT_CENTER,
            truncate(&row.name, label_w - 6.0, ui),
            FontId::proportional(12.5),
            p.text,
        );
        let track = Rect::from_min_max(
            pos2(rect.left() + label_w, cy - bar_h / 2.0),
            pos2(rect.right() - value_w, cy + bar_h / 2.0),
        );
        let mut x = track.left();
        let hover = response.hovered();
        let parts = row
            .shares
            .iter()
            .copied()
            .zip(colours.iter().copied())
            .chain(std::iter::once((idle, idle_colour)));
        for (share, colour) in parts {
            let w = share as f32 * track.width();
            if w >= 1.0 {
                // 2 px of surface between segments.
                let seg =
                    Rect::from_min_size(pos2(x, track.top()), vec2((w - 2.0).max(1.0), bar_h));
                let fill = if hover {
                    colour.lerp_to_gamma(p.text, 0.2)
                } else {
                    colour
                };
                painter.rect_filled(seg, CornerRadius::same(2), fill);
            }
            x += w;
        }
        painter.text(
            pos2(rect.right(), cy),
            Align2::RIGHT_CENTER,
            &row.busy,
            FontId::monospace(12.0),
            p.text,
        );
        if hover {
            let mut lines = vec![text.clone()];
            for (label, share) in labels.iter().zip(row.shares) {
                if share > 0.0 {
                    lines.push(format!("{label}: {}%", three(share * 100.0)));
                }
            }
            lines.push(format!("{}: {}%", t("Idle"), three(idle * 100.0)));
            tip(ui, ("bar", i), lines.join("\n"));
        }
    }
    if more > 0 {
        ui.label(theme::weak(
            ui,
            tf("and {n} more", &[("n", &more.to_string())]),
        ));
    }
    // The legend: the states some bar has, and idle.
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        let present = |k: usize| all.iter().any(|r| r.shares[k] > 0.0);
        let entries = (0..STATES.len())
            .filter(|k| present(*k))
            .map(|k| (colours[k], labels[k].as_str()))
            .chain(std::iter::once((idle_colour, t("Idle"))));
        for (colour, label) in entries {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(2), colour);
            ui.label(theme::weak(ui, label).size(11.0));
            ui.add_space(6.0);
        }
    });
}

// ---- 3. Time on conveyors ----

/// Every conveyor's histogram added together on common bins (each count placed by its bin's
/// middle): `(start, width, counts)`; a width of 0 is one value.
fn merged_histogram(results: &Value) -> Option<(f64, f64, Vec<u64>)> {
    let parts: Vec<(f64, f64, Vec<u64>)> = elements(results)
        .iter()
        .filter_map(|e| {
            let h = &e["time_on_histogram"];
            let counts: Vec<u64> = h["counts"]
                .as_array()?
                .iter()
                .filter_map(Value::as_u64)
                .collect();
            Some((h["start"].as_f64()?, h["width"].as_f64()?, counts))
        })
        .collect();
    let lo = parts.iter().map(|h| h.0).fold(f64::INFINITY, f64::min);
    let hi = parts
        .iter()
        .map(|h| h.0 + h.1 * h.2.len() as f64)
        .fold(f64::NEG_INFINITY, f64::max);
    let total: u64 = parts.iter().flat_map(|h| h.2.iter()).sum();
    if parts.is_empty() || total == 0 || !lo.is_finite() || !hi.is_finite() {
        return None;
    }
    if hi - lo <= SAME * hi.abs().max(lo.abs()).max(1e-300) {
        return Some((lo, 0.0, vec![total]));
    }
    let width = (hi - lo) / BINS as f64;
    let mut counts = vec![0u64; BINS];
    for (start, w, c) in &parts {
        for (i, n) in c.iter().enumerate() {
            let mid = start + w * (i as f64 + 0.5);
            let k = ((mid - lo) / width).max(0.0) as usize;
            counts[k.min(BINS - 1)] += n;
        }
    }
    Some((lo, width, counts))
}

fn time_on(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    results: Option<&Value>,
    watching: bool,
    unit: &str,
    width: f32,
) {
    let Some(r) = results else {
        ui.label(theme::weak(
            ui,
            if watching {
                t("Fills in when the run ends")
            } else {
                t("How long each item spends on the conveyors.")
            },
        ));
        return;
    };
    let Some((start, bin, counts)) = merged_histogram(r) else {
        ui.label(theme::weak(ui, t("No item has left a conveyor yet")));
        return;
    };
    let total: u64 = counts.iter().sum();
    let share = |n: u64| n as f64 / total as f64 * 100.0;
    let end = start + bin * counts.len() as f64;
    ui.label(theme::weak(
        ui,
        tf(
            "Share of items by time on a conveyor, in {unit}",
            &[("unit", unit)],
        ),
    ));
    let axis_w = 34.0;
    let axis_h = 16.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, 130.0 + axis_h), egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Image,
            true,
            tf(
                "Time on conveyors: {n} items between {lo} and {hi} {unit}",
                &[
                    ("n", &grouped(total as f64)),
                    ("lo", &three(start)),
                    ("hi", &three(end)),
                    ("unit", unit),
                ],
            ),
        )
    });
    let painter = ui.painter_at(rect.expand(2.0));
    let plot = Rect::from_min_max(
        pos2(rect.left() + axis_w, rect.top() + 6.0),
        pos2(rect.right() - 4.0, rect.bottom() - axis_h),
    );
    let most = counts.iter().copied().map(share).fold(0.0, f64::max);
    let (step, top) = axis(most);
    let small = FontId::proportional(10.5);
    let mut g = 0.0;
    while g <= top + step * 0.01 {
        let y = plot.bottom() - plot.height() * (g / top) as f32;
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(if g == 0.0 { 1.0 } else { 0.5 }, p.border),
        );
        painter.text(
            pos2(plot.left() - 4.0, y),
            Align2::RIGHT_CENTER,
            format!("{}%", grouped(g)),
            small.clone(),
            p.text_weak,
        );
        g += step;
    }
    let slot = plot.width() / counts.len() as f32;
    let col_w = if bin == 0.0 {
        slot.min(48.0)
    } else {
        (slot - 2.0).max(1.0)
    };
    for (i, n) in counts.iter().copied().enumerate() {
        let cx = plot.left() + slot * (i as f32 + 0.5);
        let h = plot.height() * (share(n) / top) as f32;
        let col = Rect::from_min_max(
            pos2(cx - col_w / 2.0, plot.bottom() - h),
            pos2(cx + col_w / 2.0, plot.bottom()),
        );
        // The hit area is the whole slot, so a short column can be hovered.
        let slot_rect = Rect::from_min_max(
            pos2(cx - slot / 2.0, plot.top()),
            pos2(cx + slot / 2.0, plot.bottom()),
        );
        let hover = n > 0 && ui.rect_contains_pointer(slot_rect);
        if n > 0 {
            painter.rect_filled(
                col,
                CornerRadius {
                    nw: 3,
                    ne: 3,
                    sw: 0,
                    se: 0,
                },
                if hover { p.chart_hover } else { p.chart },
            );
        }
        if hover {
            let from = start + bin * i as f64;
            tip(
                ui,
                ("bin", i),
                tf(
                    "{from} to {to} {unit}: {share} of items ({n})",
                    &[
                        ("from", &three(from)),
                        ("to", &three(from + bin)),
                        ("unit", unit),
                        ("share", &format!("{}%", three(share(n)))),
                        ("n", &grouped(n as f64)),
                    ],
                ),
            );
        }
    }
    // The time axis: its two ends (one value when all are equal).
    let label = |v: f64| format!("{} {unit}", three(v));
    if bin == 0.0 {
        painter.text(
            pos2(plot.center().x, plot.bottom() + 3.0),
            Align2::CENTER_TOP,
            label(start),
            small,
            p.text_weak,
        );
    } else {
        painter.text(
            pos2(plot.left(), plot.bottom() + 3.0),
            Align2::LEFT_TOP,
            label(start),
            small.clone(),
            p.text_weak,
        );
        painter.text(
            pos2(plot.right(), plot.bottom() + 3.0),
            Align2::RIGHT_TOP,
            label(end),
            small,
            p.text_weak,
        );
    }
}

// ---- 4. Items over time ----

#[allow(clippy::too_many_arguments)]
fn over_time(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    results: Option<&Value>,
    glimpse: Option<&Glimpse>,
    conveyors: bool,
    title: &str,
    unit: &str,
    width: f32,
) {
    let (points, end): (Vec<(f64, f64)>, f64) = if let Some(g) = glimpse {
        let pts = if conveyors {
            g.on_conveyors.clone().unwrap_or_default()
        } else {
            g.in_model.clone()
        };
        (pts, g.end)
    } else if let Some(r) = results {
        let points = recorded(r, conveyors);
        let end = points.last().map_or(0.0, |(time, _)| *time);
        (points, end)
    } else {
        ui.label(theme::weak(
            ui,
            if conveyors {
                t("How many items are on the conveyors over time.")
            } else {
                t("How many items are in the model over time.")
            },
        ));
        return;
    };
    if points.len() < 2 || end <= 0.0 {
        ui.label(theme::weak(
            ui,
            t("Nothing was recorded over time in these results"),
        ));
        return;
    }
    let most = points.iter().map(|(_, v)| *v).fold(0.0, f64::max);
    let last = points.last().map_or(0.0, |(_, v)| *v);
    let (step, top) = axis(most);
    let axis_w = 34.0;
    let axis_h = 16.0;
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, 130.0 + axis_h), egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Image,
            true,
            tf(
                "{title}: now {now}, at most {most}",
                &[
                    ("title", title),
                    ("now", &grouped(last)),
                    ("most", &grouped(most.ceil())),
                ],
            ),
        )
    });
    let painter = ui.painter_at(rect.expand(2.0));
    let plot = Rect::from_min_max(
        pos2(rect.left() + axis_w, rect.top() + 6.0),
        pos2(rect.right() - 6.0, rect.bottom() - axis_h),
    );
    let small = FontId::proportional(10.5);
    let mut g = 0.0;
    while g <= top + step * 0.01 {
        let y = plot.bottom() - plot.height() * (g / top) as f32;
        painter.line_segment(
            [pos2(plot.left(), y), pos2(plot.right(), y)],
            Stroke::new(if g == 0.0 { 1.0 } else { 0.5 }, p.border),
        );
        painter.text(
            pos2(plot.left() - 4.0, y),
            Align2::RIGHT_CENTER,
            grouped(g),
            small.clone(),
            p.text_weak,
        );
        g += step;
    }
    painter.text(
        pos2(plot.left(), plot.bottom() + 3.0),
        Align2::LEFT_TOP,
        "0",
        small.clone(),
        p.text_weak,
    );
    painter.text(
        pos2(plot.right(), plot.bottom() + 3.0),
        Align2::RIGHT_TOP,
        format!("{} {unit}", grouped(round_sig(end, 4))),
        small,
        p.text_weak,
    );
    let at = |time: f64, v: f64| -> Pos2 {
        pos2(
            plot.left() + plot.width() * (time / end).clamp(0.0, 1.0) as f32,
            plot.bottom() - plot.height() * (v / top).clamp(0.0, 1.0) as f32,
        )
    };
    let line: Vec<Pos2> = points.iter().map(|(time, v)| at(*time, *v)).collect();
    painter.add(egui::Shape::line(line, Stroke::new(2.0, p.chart)));
    // The crosshair and its tooltip: the recorded point nearest the pointer.
    if let Some(pointer) = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|q| plot.contains(*q))
    {
        let time = f64::from((pointer.x - plot.left()) / plot.width()) * end;
        let nearest = points
            .iter()
            .min_by(|a, b| (a.0 - time).abs().total_cmp(&(b.0 - time).abs()));
        if let Some((tm, v)) = nearest {
            let q = at(*tm, *v);
            painter.line_segment(
                [pos2(q.x, plot.top()), pos2(q.x, plot.bottom())],
                Stroke::new(1.0, p.text_weak),
            );
            // A marker with a ring of the surface round it.
            painter.circle_filled(q, 5.0, p.canvas);
            painter.circle_filled(q, 3.5, p.chart_hover);
            tip(
                ui,
                ("line", 0),
                tf(
                    "At {time} {unit}: {n} items",
                    &[("time", &three(*tm)), ("unit", unit), ("n", &three(*v))],
                ),
            );
        }
    }
}

/// The number of items on the conveyors (or, with none, in every station) at each recorded time
/// of the results: the elements' `work_in_progress` series added up.
fn recorded(results: &Value, conveyors: bool) -> Vec<(f64, f64)> {
    let s = &results["series"];
    let times: Vec<f64> = s["times"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    let mut sums = vec![0.0; times.len()];
    for e in elements(results) {
        let counted = match e["kind"].as_str() {
            Some("conveyor") => true,
            Some("server" | "shared_task" | "delay") => !conveyors,
            _ => false,
        };
        if !counted {
            continue;
        }
        let id = e["id"].as_str().unwrap_or("");
        let mean = &s["elements"][id]["work_in_progress"]["mean"];
        for (i, sum) in sums.iter_mut().enumerate() {
            *sum += mean[i].as_f64().unwrap_or(0.0);
        }
    }
    times.into_iter().zip(sums).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{merged_histogram, recorded};

    #[test]
    fn histograms_of_conveyors_merge_onto_common_bins() {
        let r = json!({ "elements": [
            { "id": "a", "kind": "conveyor",
              "time_on_histogram": { "start": 10.0, "width": 0.0, "counts": [6] } },
            { "id": "b", "kind": "conveyor",
              "time_on_histogram": { "start": 10.0, "width": 0.0, "counts": [4] } },
            { "id": "c", "kind": "server" },
        ]});
        assert_eq!(merged_histogram(&r), Some((10.0, 0.0, vec![10])));
        let r = json!({ "elements": [
            { "id": "a", "kind": "conveyor",
              "time_on_histogram": { "start": 4.0, "width": 0.1, "counts": [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1] } },
            { "id": "b", "kind": "conveyor",
              "time_on_histogram": { "start": 10.0, "width": 0.0, "counts": [2] } },
        ]});
        let merged = merged_histogram(&r).map(|h| (h.0, h.2.iter().sum::<u64>(), h.2.len()));
        assert_eq!(merged, Some((4.0, 4, 20)));
    }

    #[test]
    fn conveyors_series_add_up_and_stations_stand_in_without_them() {
        let r = json!({
            "elements": [
                { "id": "c", "kind": "conveyor" },
                { "id": "s", "kind": "server" },
            ],
            "series": { "times": [0.0, 1.0], "elements": {
                "c": { "work_in_progress": { "mean": [1.0, 2.0] } },
                "s": { "work_in_progress": { "mean": [3.0, 4.0] } },
            }},
        });
        assert_eq!(recorded(&r, true), vec![(0.0, 1.0), (1.0, 2.0)]);
        assert_eq!(recorded(&r, false), vec![(0.0, 4.0), (1.0, 6.0)]);
    }
}
