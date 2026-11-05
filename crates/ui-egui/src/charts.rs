//! Charts of a run's results: one measure per chart (one axis), one bar per block with the 95%
//! confidence interval as a whisker, the value at the bar's tip and a tooltip on hover. The
//! table below them holds every number, so the charts carry no legend: each has one series,
//! named by its title.
//!
//! Marks follow the dataviz rules: bars at most 16 px thick, square at the baseline with a 4 px
//! rounded end, a recessive grid, text in text colours (never the bar's). The bar hue is
//! validated per theme against its surface (`theme::Palette::chart`).

use egui::{Align2, FontId, Rect, Stroke, pos2, vec2};
use serde_json::Value;

use crate::results::{grouped, unit_label};
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
];

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
fn nice_step(max: f64, ticks: f64) -> f64 {
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

/// Draws the charts that have data, side by side where they fit. Returns the area they cover
/// (for exporting them as an image), or `None` when no chart applies.
pub fn show(ui: &mut egui::Ui, results: &Value) -> Option<Rect> {
    let unit = unit_label(results["record"]["time_unit"].as_str().unwrap_or(""));
    let charts: Vec<(&Measure, Vec<Bar>)> = MEASURES
        .iter()
        .map(|m| (m, bars(results, m)))
        .filter(|(_, b)| !b.is_empty())
        .collect();
    if charts.is_empty() {
        return None;
    }
    let width = ui.available_width().min(900.0);
    let per_row = if width >= 700.0 { 2 } else { 1 };
    let chart_w = (width - 24.0 * (per_row as f32 - 1.0)) / per_row as f32;
    let top = ui.cursor().min;
    for row in charts.chunks(per_row) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            for (m, bars) in row {
                ui.allocate_ui_with_layout(
                    vec2(chart_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(chart_w);
                        chart(ui, m, bars, unit, chart_w);
                    },
                );
            }
        });
        ui.add_space(14.0);
    }
    Some(Rect::from_min_max(
        top,
        pos2(top.x + width, ui.cursor().min.y),
    ))
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
        m.title.to_owned()
    } else {
        format!("{} ({})", m.title, suffix.trim())
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
    let max = if m.share {
        100.0
    } else {
        bars.iter()
            .map(|b| b.mean + b.half_width)
            .fold(0.0_f64, f64::max)
            .max(f64::MIN_POSITIVE)
    };
    let step = if m.share { 25.0 } else { nice_step(max, 4.0) };
    let top_value = if m.share {
        100.0
    } else {
        (max / step).ceil() * step
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
            grouped(b.mean),
            FontId::monospace(12.0),
            p.text,
        );
        if hover {
            let tip = format!(
                "{}: {}{suffix}\n95% interval {} to {}{suffix}\nacross {} replications",
                b.name,
                grouped(b.mean),
                grouped(b.mean - b.half_width),
                grouped(b.mean + b.half_width),
                b.n
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
fn truncate(text: &str, width: f32, ui: &egui::Ui) -> String {
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

/// The results as CSV, one row per element and metric (and per property it assigns), as
/// `stochcraft-cli run --format csv` writes them.
pub fn csv(results: &Value) -> String {
    fn field(v: &str) -> String {
        if v.contains([',', '"', '\n']) {
            format!("\"{}\"", v.replace('"', "\"\""))
        } else {
            v.to_owned()
        }
    }
    let mut out = String::from("element,name,kind,metric,mean,half_width,std_dev,n\n");
    for e in results["elements"].as_array().into_iter().flatten() {
        let s = |k: &str| e[k].as_str().unwrap_or("").to_owned();
        let mut rows: Vec<(String, &Value)> = results["metrics"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|m| (m.to_owned(), &e["metrics"][m]))
            .filter(|(_, v)| !v.is_null())
            .collect();
        for (name, v) in e["properties"].as_object().into_iter().flatten() {
            rows.push((format!("property:{name}"), v));
        }
        for (metric, m) in rows {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                field(&s("id")),
                field(&s("name")),
                s("kind"),
                field(&metric),
                m["mean"],
                m["half_width"],
                m["std_dev"],
                m["n"]
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gridlines_fall_on_round_values() {
        assert_eq!(nice_step(4.1, 4.0), 2.0);
        assert_eq!(nice_step(100.0, 4.0), 50.0);
        assert_eq!(nice_step(0.07, 4.0), 0.02);
        assert_eq!(nice_step(15000.0, 4.0), 5000.0);
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
