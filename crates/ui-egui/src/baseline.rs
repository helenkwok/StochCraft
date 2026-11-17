//! Baseline and now (#148): results kept as a baseline, and the results of a later run compared
//! with them, measure by measure. The comparison is the engine's `results.compare`: two runs with
//! the same seed and number of runs meet the same random numbers run by run, so their runs are
//! compared in pairs (the paired-t interval); otherwise they are compared as independent runs
//! (Welch's interval), which is wider. A difference is called real when its 95% interval leaves
//! out zero, each measure on its own.
//!
//! The window shows, over the rest of the results: how the two runs were run, what the change
//! did in sentences, and a table of each block's key measures with the baseline, now, the
//! difference with its interval, a mark of the change against a zero line, and a verdict in
//! words with an icon.

use egui::RichText;
use serde_json::{Value, json};

use crate::i18n::{t, tf, tnf};
use crate::results::{self, Unit};
use crate::theme;

/// Results kept to compare later runs with.
#[derive(Clone, Debug)]
pub struct Baseline {
    /// The results, as the run returned them.
    pub results: Value,
    /// The model they came from, as the app keeps results against it, for what has changed
    /// since ([`crate::changes::changes`]).
    pub model: Value,
    /// Which results of this window they were (the app counts every results it shows).
    pub serial: u64,
}

/// The results to compare, as `results.compare` takes them: each block's key measures in either
/// run ([`results::key_of`]), where both runs report them, in the order of `now`. Times are left
/// out when the runs measured time in different units.
pub fn wanted(baseline: &Value, now: &Value) -> Vec<Value> {
    let same_unit = time_unit(baseline) == time_unit(now);
    let mut out = Vec::new();
    for e in now["elements"].as_array().into_iter().flatten() {
        let id = e["id"].as_str().unwrap_or("");
        let Some(b) = element(baseline, id) else {
            continue;
        };
        let kind = e["kind"].as_str().unwrap_or("");
        let mut keys = results::key_of(e);
        for k in results::key_of(b) {
            if !keys.contains(&k) {
                keys.push(k);
            }
        }
        for k in keys {
            let time = matches!(results::metric(k, kind).1, Unit::Time);
            if (same_unit || !time) && !e["metrics"][k].is_null() && !b["metrics"][k].is_null() {
                out.push(json!({ "element": id, "metric": k }));
            }
        }
    }
    out
}

fn time_unit(results: &Value) -> &str {
    results["record"]["time_unit"].as_str().unwrap_or("")
}

fn element<'a>(results: &'a Value, id: &str) -> Option<&'a Value> {
    results["elements"]
        .as_array()?
        .iter()
        .find(|e| e["id"] == id)
}

/// What a comparison says about one measure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    /// The interval leaves out zero, below it.
    Lower,
    /// The interval leaves out zero, above it.
    Higher,
    /// The interval takes in zero.
    Chance,
    /// Every run gave the same value in both: nothing moved.
    Same,
    /// Too few runs had a value to give an interval.
    Unknown,
}

/// The verdict on comparison `c` (an entry of `results.compare`'s `comparisons`).
pub fn verdict(c: &Value) -> Verdict {
    let d = c["difference"].as_f64();
    let hw = c["half_width"].as_f64();
    match (c["significant"].as_bool(), d) {
        (_, Some(d)) if d == 0.0 && hw == Some(0.0) => Verdict::Same,
        (Some(true), Some(d)) if d < 0.0 => Verdict::Lower,
        (Some(true), Some(_)) => Verdict::Higher,
        (Some(false), _) => Verdict::Chance,
        _ => Verdict::Unknown,
    }
}

impl Verdict {
    fn words(self) -> &'static str {
        match self {
            Verdict::Lower => t("Real: lower"),
            Verdict::Higher => t("Real: higher"),
            Verdict::Chance => t("Could be chance"),
            Verdict::Same => t("No change"),
            Verdict::Unknown => t("Too few runs to tell"),
        }
    }

    fn real(self) -> bool {
        matches!(self, Verdict::Lower | Verdict::Higher)
    }
}

/// One measure of the comparison, as the table and the sentences show it.
struct Row {
    block: String,
    measure: String,
    before: String,
    after: String,
    /// The difference with its interval, rounded together, with its unit ("−6 ± 2 points").
    change: String,
    /// The difference and its half-width as shares of the baseline's mean, for the mark.
    relative: Option<(f64, f64)>,
    verdict: Verdict,
    /// Either run's result splits into two outcomes: its mean mixes them, so no sentence.
    split: bool,
}

/// `-` as a minus sign, and `+` before a number above zero.
fn signed(text: String, v: f64) -> String {
    if let Some(rest) = text.strip_prefix('-') {
        format!("\u{2212}{rest}")
    } else if v > 0.0 && text.chars().any(|c| c.is_ascii_digit() && c != '0') {
        format!("+{text}")
    } else {
        text
    }
}

fn rows(baseline: &Value, now: &Value, comparison: &Value) -> Vec<Row> {
    let time = results::unit_label(time_unit(now));
    comparison["comparisons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let id = c["element"].as_str()?;
            let key = c["metric"].as_str()?;
            let (e, b) = (element(now, id)?, element(baseline, id)?);
            let kind = e["kind"].as_str().unwrap_or("");
            let (label, unit) = results::metric(key, kind);
            let (scale, unit_text) = results::scale_and_unit(&unit, time);
            let (m_now, m_base) = (&e["metrics"][key], &b["metrics"][key]);
            let d = c["difference"].as_f64();
            let hw = c["half_width"].as_f64();
            let change = match (d, hw) {
                (Some(d), Some(hw)) => {
                    let (m, h) = results::rounded(d * scale, hw * scale);
                    format!("{} ± {h}", signed(m, d))
                }
                (Some(d), None) => signed(results::grouped(results::round_sig(d * scale, 3)), d),
                _ => "–".to_owned(),
            };
            let change = match unit_text {
                _ if change == "–" => change,
                "%" => tf("{value} points", &[("value", &change)]),
                "" => change,
                u => format!("{change} {u}"),
            };
            let base = c["a"]["mean"].as_f64();
            let relative = match (d, base) {
                (Some(d), Some(a)) if a != 0.0 => Some((d / a.abs(), hw.unwrap_or(0.0) / a.abs())),
                _ => None,
            };
            Some(Row {
                block: results::block_name(e).to_owned(),
                measure: label.to_owned(),
                before: results::said(m_base, &unit, time).unwrap_or_else(|| "–".into()),
                after: results::said(m_now, &unit, time).unwrap_or_else(|| "–".into()),
                change,
                relative,
                verdict: verdict(c),
                split: !m_now["split"].is_null() || !m_base["split"].is_null(),
            })
        })
        .collect()
}

/// How the two runs were run: paired (the same seed and runs) or not, and a time unit that
/// differs.
fn how_compared(baseline: &Value, now: &Value, comparison: &Value) -> Vec<String> {
    let (a, b) = (&comparison["a"], &comparison["b"]);
    let n = |v: &Value| v["replications"].as_u64().unwrap_or(0);
    let seed = |v: &Value| v["seed"].to_string();
    let mut lines = Vec::new();
    if comparison["paired"] == true {
        lines.push(tnf(
            n(b),
            "Both used seed {seed} and the same {n} run, so each run meets the same random numbers as its pair: the differences come from the change, not from chance draws.",
            "Both used seed {seed} and the same {n} runs, so each run meets the same random numbers as its pair: the differences come from the change, not from chance draws.",
            &[("seed", &seed(b))],
        ));
    } else {
        lines.push(tf(
            "The baseline used seed {seed_a} and {n_a} runs, this run seed {seed_b} and {n_b} runs, so they are compared as independent runs and the intervals are wider. The same seed and number of runs gives a paired comparison.",
            &[
                ("seed_a", &seed(a)),
                ("n_a", &n(a).to_string()),
                ("seed_b", &seed(b)),
                ("n_b", &n(b).to_string()),
            ],
        ));
    }
    let (ua, ub) = (time_unit(baseline), time_unit(now));
    if ua != ub {
        lines.push(tf(
            "The baseline measured time in {a} and this run in {b}, so times are not compared.",
            &[
                ("a", results::unit_label(ua)),
                ("b", results::unit_label(ub)),
            ],
        ));
    }
    lines
}

/// The sentences of "What the change did": the real changes, the largest first (at most three),
/// then how many moved no more than chance could.
fn sentences(rows: &[Row]) -> Vec<(bool, String)> {
    let mut real: Vec<&Row> = rows
        .iter()
        .filter(|r| r.verdict.real() && !r.split)
        .collect();
    real.sort_by(|x, y| {
        let size = |r: &Row| r.relative.map_or(0.0, |(d, _)| d.abs());
        size(y).total_cmp(&size(x))
    });
    let mut out: Vec<(bool, String)> = real
        .iter()
        .take(3)
        .map(|r| {
            let text = if r.verdict == Verdict::Lower {
                "{measure} ({block}) fell from {before} to {after}, by {change}: more than chance would explain."
            } else {
                "{measure} ({block}) rose from {before} to {after}, by {change}: more than chance would explain."
            };
            let text = tf(
                text,
                &[
                    ("measure", &r.measure),
                    ("block", &r.block),
                    ("before", &r.before),
                    ("after", &r.after),
                    ("change", &r.change),
                ],
            );
            (true, text)
        })
        .collect();
    let chance = rows.iter().filter(|r| r.verdict == Verdict::Chance).count();
    if real.is_empty() {
        if chance > 0 {
            out.push((
                false,
                t("None of the key measures moved by more than chance: the change may do little, or more runs may show it.").to_owned(),
            ));
        }
    } else if chance > 0 {
        out.push((
            false,
            tnf(
                chance as u64,
                "{n} other key measure moved no more than chance could.",
                "{n} other key measures moved no more than chance could.",
                &[],
            ),
        ));
    }
    out
}

/// The comparison over the results: how the runs were compared, what the change did, and the
/// table of key measures.
pub fn show(
    ui: &mut egui::Ui,
    baseline: &Value,
    now: &Value,
    comparison: &Value,
    changes: &[String],
) {
    let p = theme::palette(ui.ctx());
    let rows = rows(baseline, now, comparison);
    egui::Frame::new()
        .fill(p.stripe)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            changed_line(ui, changes);
            ui.add_space(2.0);
            for line in how_compared(baseline, now, comparison) {
                ui.add(egui::Label::new(theme::weak(ui, line)).wrap());
            }
        });
    ui.add_space(8.0);
    let said = sentences(&rows);
    if !said.is_empty() {
        egui::Frame::new()
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(theme::strong(t("What the change did")));
                ui.add_space(4.0);
                for (real, text) in &said {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
                        if *real {
                            theme::paint_check_mark(ui.painter(), rect.center(), 6.5, &p);
                        } else {
                            paint_about_equal(ui.painter(), rect.center(), &p);
                        }
                        ui.add(egui::Label::new(text.as_str()).wrap());
                    });
                }
            });
        ui.add_space(10.0);
    }
    table(ui, &p, &rows, comparison["paired"] == true);
}

/// The most changes "Changed since the baseline" lists before "and n more changes".
const LISTED: usize = 6;

/// "Changed since the baseline:" and the changes, or that nothing in the model changed.
fn changed_line(ui: &mut egui::Ui, changes: &[String]) {
    if changes.is_empty() {
        ui.label(theme::strong(t(
            "Nothing in the model has changed since the baseline.",
        )));
        return;
    }
    let mut text = changes
        .iter()
        .take(LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    if changes.len() > LISTED {
        let more = (changes.len() - LISTED) as u64;
        text.push_str("; ");
        text.push_str(&tnf(
            more,
            "and {n} more change",
            "and {n} more changes",
            &[],
        ));
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::strong(t("Changed since the baseline:")));
        ui.label(text);
    });
}

/// "≈", the mark of a difference that could be chance.
fn paint_about_equal(painter: &egui::Painter, centre: egui::Pos2, p: &theme::Palette) {
    painter.text(
        centre,
        egui::Align2::CENTER_CENTER,
        "\u{2248}",
        egui::FontId::proportional(15.0),
        p.text_weak,
    );
}

/// Column edges of the table, from the left: the measure, the baseline, now (both right-aligned),
/// the difference (right-aligned), the change mark, the verdict (its icon just before it).
const COLUMNS: [(f32, f32); 6] = [
    (8.0, 250.0),
    (250.0, 330.0),
    (330.0, 410.0),
    (410.0, 590.0),
    (610.0, 790.0),
    (822.0, 970.0),
];

/// The verdict's column where the window is too narrow for the change mark.
const NARROW_VERDICT: (f32, f32) = (622.0, 770.0);

fn table(ui: &mut egui::Ui, p: &theme::Palette, rows: &[Row], paired: bool) {
    if rows.is_empty() {
        return;
    }
    ui.label(theme::strong(t("Baseline and now, key measures")).size(15.0));
    ui.add_space(4.0);
    let wide = ui.available_width() >= COLUMNS[5].1;
    // One scale for every row's mark: the change as a share of the baseline, from the lowest end
    // of an interval to the highest, zero always in view.
    let (lo, hi) = rows
        .iter()
        .filter_map(|r| r.relative)
        .fold((0.0_f64, 0.0_f64), |(lo, hi), (d, hw)| {
            (lo.min(d - hw), hi.max(d + hw))
        });
    let scale = (-nice(-lo), nice(hi));
    let difference = if paired {
        t("Difference (paired, 95%)")
    } else {
        t("Difference (95%)")
    };
    let header = [
        t("Measure"),
        t("Baseline"),
        t("Now"),
        difference,
        if wide { t("Change from baseline") } else { "" },
        t("Verdict"),
    ];
    let _ = row_frame(ui, p, None, wide, |ui, cell| {
        for (c, text) in header.iter().enumerate() {
            if !text.is_empty() {
                cell(ui, c, RichText::new(*text).size(12.0).color(p.text_weak));
            }
        }
    });
    for (i, r) in rows.iter().enumerate() {
        let measure = tf(
            "{block}: {measure}",
            &[("block", &r.block), ("measure", &r.measure)],
        );
        let rect = row_frame(ui, p, Some(i % 2 == 0), wide, |ui, cell| {
            cell(ui, 0, RichText::new(measure.as_str()));
            cell(ui, 1, RichText::new(r.before.as_str()).color(p.text_weak));
            cell(ui, 2, theme::strong(r.after.as_str()));
            cell(ui, 3, RichText::new(r.change.as_str()));
            let color = if r.verdict.real() {
                p.accent
            } else {
                p.text_weak
            };
            cell(ui, 5, RichText::new(r.verdict.words()).color(color));
        });
        // The verdict's icon before its words, then the mark.
        let at = egui::pos2(rect.left() + column(5, wide).0 - 10.0, rect.center().y);
        if r.verdict.real() {
            theme::paint_check_mark(ui.painter(), at, 5.5, p);
        } else if r.verdict == Verdict::Chance {
            paint_about_equal(ui.painter(), at, p);
        }
        if wide {
            let mark = egui::Rect::from_x_y_ranges(
                rect.left() + COLUMNS[4].0..=rect.left() + COLUMNS[4].1,
                rect.y_range(),
            );
            paint_mark(ui, p, mark, r.relative, scale, &r.change);
        }
    }
    if wide && scale.1 > scale.0 {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(COLUMNS[5].1, 18.0), egui::Sense::hover());
        let axis = egui::Rect::from_x_y_ranges(
            rect.left() + COLUMNS[4].0..=rect.left() + COLUMNS[4].1,
            rect.y_range(),
        );
        // The ends and zero, each end's label kept inside the column.
        for (v, align) in [
            (scale.0, egui::Align2::LEFT_CENTER),
            (0.0, egui::Align2::CENTER_CENTER),
            (scale.1, egui::Align2::RIGHT_CENTER),
        ] {
            let inner = axis.shrink2(egui::vec2(5.0, 0.0));
            let x = at_x(inner, scale, v);
            // An end at zero, or too near it for its label, is left to zero's label.
            if align != egui::Align2::CENTER_CENTER && (x - at_x(inner, scale, 0.0)).abs() < 44.0 {
                continue;
            }
            let text = format!("{}%", signed(results::grouped(v * 100.0), v));
            ui.painter().text(
                egui::pos2(x, axis.center().y),
                align,
                text,
                egui::FontId::proportional(11.0),
                p.text_weak,
            );
        }
    }
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(theme::weak(
            ui,
            t("A verdict is real when the difference's 95% interval leaves out zero. Each row is judged on its own."),
        ))
        .wrap(),
    );
}

/// A table row: its stripe, and a `cell(ui, column, text)` to fill it with.
fn row_frame(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    stripe: Option<bool>,
    wide: bool,
    fill: impl FnOnce(&mut egui::Ui, &dyn Fn(&mut egui::Ui, usize, RichText)),
) -> egui::Rect {
    let height = if stripe.is_some() { 26.0 } else { 20.0 };
    let width = column(5, wide).1;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    if stripe == Some(true) {
        ui.painter().rect_filled(rect, 4.0, p.stripe);
    }
    let cell = |ui: &mut egui::Ui, c: usize, text: RichText| {
        let (l, r) = column(c, wide);
        let cell = egui::Rect::from_x_y_ranges(rect.left() + l..=rect.left() + r, rect.y_range());
        let layout = if (1..=3).contains(&c) {
            egui::Layout::right_to_left(egui::Align::Center)
        } else {
            egui::Layout::left_to_right(egui::Align::Center)
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(cell).layout(layout), |ui| {
            ui.add(egui::Label::new(text).truncate());
        });
    };
    fill(ui, &cell);
    rect
}

/// Where column `c` goes: the verdict moves left into the mark's place in a narrow window.
fn column(c: usize, wide: bool) -> (f32, f32) {
    if c == 5 && !wide {
        NARROW_VERDICT
    } else {
        COLUMNS[c]
    }
}

/// The smallest of 5%, 10%, 20%, 25%, 50% and the same times powers of ten at or above `v` (a
/// share), so the axis reads in round numbers; 0 for nothing to show.
fn nice(v: f64) -> f64 {
    if !(v.is_finite() && v > 0.0) {
        return 0.0;
    }
    let mut base = 0.01;
    while base < 1.0e6 {
        for m in [5.0, 10.0, 20.0, 25.0, 50.0] {
            if m * base >= v * (1.0 - 1e-12) {
                return m * base;
            }
        }
        base *= 10.0;
    }
    v
}

/// Where `v` (a change as a share of the baseline) goes across `rect` on the axis `scale` (its
/// lowest and highest values).
fn at_x(rect: egui::Rect, scale: (f64, f64), v: f64) -> f32 {
    let span = scale.1 - scale.0;
    if span > 0.0 {
        rect.left() + ((v - scale.0) / span) as f32 * rect.width()
    } else {
        rect.center().x
    }
}

/// The change mark: a dot at the difference, a bar across its interval, against a zero line.
fn paint_mark(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    rect: egui::Rect,
    relative: Option<(f64, f64)>,
    scale: (f64, f64),
    said: &str,
) {
    let response = ui.interact(
        rect,
        ui.scope_id().with(("change-mark", said)),
        egui::Sense::hover(),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, said));
    let painter = ui.painter_at(rect);
    let y = rect.center().y;
    // Room at either end for the dot.
    let inner = rect.shrink2(egui::vec2(5.0, 0.0));
    let x0 = at_x(inner, scale, 0.0);
    painter.line_segment(
        [egui::pos2(x0, rect.top()), egui::pos2(x0, rect.bottom())],
        egui::Stroke::new(1.0, p.text_weak),
    );
    let (Some((d, hw)), true) = (relative, scale.1 > scale.0) else {
        return;
    };
    let x = |v: f64| at_x(inner, scale, v);
    painter.line_segment(
        [egui::pos2(x(d - hw), y), egui::pos2(x(d + hw), y)],
        egui::Stroke::new(3.0, p.chart),
    );
    painter.circle(
        egui::pos2(x(d), y),
        3.5,
        p.canvas,
        egui::Stroke::new(1.5, p.chart),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_follow_the_interval() {
        let c = |d: f64, hw: f64, s: Option<bool>| json!({ "difference": d, "half_width": hw, "significant": s });
        assert_eq!(verdict(&c(-2.0, 1.0, Some(true))), Verdict::Lower);
        assert_eq!(verdict(&c(2.0, 1.0, Some(true))), Verdict::Higher);
        assert_eq!(verdict(&c(0.5, 1.0, Some(false))), Verdict::Chance);
        assert_eq!(verdict(&c(0.0, 0.0, Some(false))), Verdict::Same);
        assert_eq!(
            verdict(&json!({ "difference": 1.0, "half_width": null, "significant": null })),
            Verdict::Unknown
        );
    }

    #[test]
    fn the_axis_reads_in_round_numbers() {
        assert_eq!(nice(0.0), 0.0);
        assert!((nice(0.03) - 0.05).abs() < 1e-12);
        assert!((nice(0.07) - 0.1).abs() < 1e-12);
        assert!((nice(0.3) - 0.5).abs() < 1e-12);
        assert!((nice(0.8) - 1.0).abs() < 1e-12);
        assert!((nice(1.7) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn signs_are_written_out() {
        assert_eq!(signed("-6".into(), -6.0), "\u{2212}6");
        assert_eq!(signed("0.5".into(), 0.5), "+0.5");
        assert_eq!(signed("0".into(), 0.0), "0");
        assert_eq!(signed("0.0".into(), 0.01), "0.0");
    }

    #[test]
    fn key_measures_both_runs_report_are_compared() {
        let run = |unit: &str, wait: Value| {
            json!({
                "record": { "time_unit": unit },
                "elements": [
                    { "id": "serve", "kind": "server", "metrics": {
                        "utilisation": { "mean": 0.8 }, "mean_wait": wait,
                        "mean_queue_length": { "mean": 3.0 }, "left": { "mean": 90.0 } } },
                    { "id": "new", "kind": "delay", "metrics": { "mean_contents": { "mean": 1.0 } } }
                ]
            })
        };
        let a = run("min", json!({ "mean": 4.0 }));
        let mut b = run("min", json!({ "mean": 2.0 }));
        b["elements"][1]["id"] = json!("other");
        let w = wanted(&a, &b);
        let metrics: Vec<&str> = w.iter().filter_map(|x| x["metric"].as_str()).collect();
        assert_eq!(
            metrics,
            ["utilisation", "mean_wait", "mean_queue_length", "left"]
        );
        // Another time unit: times are left out.
        let c = run("hour", json!({ "mean": 2.0 }));
        let w = wanted(&a, &c);
        assert!(w.iter().all(|x| x["metric"] != "mean_wait"));
    }
}
