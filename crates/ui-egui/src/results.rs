//! The results of a run, in the words of the model: one section per element with its metrics,
//! their units and the half-width of each 95% confidence interval, under the run record that
//! makes them reproducible (the same content as `stochcraft-cli run`).

use egui::RichText;
use serde_json::Value;

use crate::i18n::{t, tc, tf, tnf};
use crate::theme;

/// A number for a table: about five significant digits, without a run of trailing zeros.
pub fn number(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v == 0.0 {
        return "0".into();
    }
    // The magnitude by repeated scaling rather than `log10`, which `clippy.toml` forbids (it differs
    // between platforms); this only chooses how many digits to print.
    let mut magnitude = 0i32;
    let mut a = v.abs();
    while a >= 10.0 && magnitude < 400 {
        a /= 10.0;
        magnitude += 1;
    }
    while a < 1.0 && magnitude > -400 {
        a *= 10.0;
        magnitude -= 1;
    }
    if !(-3..6).contains(&magnitude) {
        return format!("{v:.4e}");
    }
    let decimals = (4 - magnitude).max(0) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        s
    }
}

/// How a metric reads, and its unit: `Time` takes the model's time unit, `Share` shows as a
/// percentage.
enum Unit {
    Count,
    Time,
    Share,
    Items,
}

/// The label and unit of metric `name` on an element of kind `kind`.
fn metric<'a>(name: &'a str, kind: &str) -> (&'a str, Unit) {
    match name {
        "entered" => (t("Entered"), Unit::Count),
        "left" => (t("Left"), Unit::Count),
        "mean_queue_length" => (t("Mean queue length"), Unit::Items),
        "mean_wait" => (t("Mean wait"), Unit::Time),
        "utilisation" => (t("Utilisation"), Unit::Share),
        "mean_contents" => (t("Mean number inside"), Unit::Items),
        "mean_time_in_system" => (t("Mean time in system"), Unit::Time),
        "mean_blocked_time" => (t("Mean time blocked"), Unit::Time),
        "blocking_probability" => (t("Share of units blocked"), Unit::Share),
        "lost" if kind == "exclusive_gateway" => (t("Lost (no way out)"), Unit::Count),
        "lost" => (t("Lost (waiting room full)"), Unit::Count),
        "loss_probability" => (t("Share of arrivals lost"), Unit::Share),
        "availability" => (t("Availability"), Unit::Share),
        other => (other, Unit::Count),
    }
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "source" => t("Arrivals"),
        "server" => t("Task with a resource"),
        "shared_task" => t("Task with shared resources"),
        "resource" => t("Resource pool"),
        "delay" => t("Task"),
        "sink" => t("Departures"),
        "exclusive_gateway" => t("Decision"),
        "parallel_gateway" => t("Split or join"),
        other => other,
    }
}

/// A time unit as written after a number, in the interface language: the engine's names, with
/// hours, days and years abbreviated so "24 h" reads right where "24 hour" would not. The
/// context `unit` keeps these short words apart from any other use of the same letters.
pub fn unit_label(unit: &str) -> &str {
    match unit {
        "ms" => tc("unit", "ms"),
        "s" => tc("unit", "s"),
        "min" => tc("unit", "min"),
        "hour" => tc("unit", "h"),
        "day" => tc("unit", "d"),
        "year" => tc("unit", "yr"),
        other => other,
    }
}

/// The run record's line under the results' heading: replications, seed, warm-up, duration.
pub fn record_line(ui: &mut egui::Ui, results: &Value) {
    let r = &results["record"];
    let unit = unit_label(r["time_unit"].as_str().unwrap_or(""));
    let num = |v: &Value| v.as_f64().map_or_else(|| "–".to_owned(), grouped);
    ui.label(theme::weak(
        ui,
        tnf(
            r["replications"].as_u64().unwrap_or(0),
            "{n} replication · seed {seed} · warm-up {warmup} {unit} · {duration} {unit} measured",
            "{n} replications · seed {seed} · warm-up {warmup} {unit} · {duration} {unit} measured",
            &[
                ("seed", &r["seed"].to_string()),
                ("warmup", &num(&r["warmup"])),
                ("duration", &num(&r["duration"])),
                ("unit", unit),
            ],
        ),
    ));
}

/// Draws `results` (the engine's `run` result).
pub fn show(ui: &mut egui::Ui, results: &Value) {
    let p = theme::palette(ui.ctx());
    let r = &results["record"];
    let unit = unit_label(r["time_unit"].as_str().unwrap_or(""));

    let order: Vec<&str> = results["metrics"]
        .as_array()
        .map(|m| m.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    ui.vertical(|ui| {
        ui.set_max_width(640.0);
        let notes: Vec<&str> = r["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(r["file_notes"].as_array().into_iter().flatten())
            .filter(|n| n["severity"] != "info")
            .filter_map(|n| n["message"].as_str())
            .collect();
        for n in &notes {
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::strong(t("Warning")).color(p.warning));
                ui.label(*n);
            });
        }
        if !notes.is_empty() {
            ui.add_space(10.0);
        }

        for e in results["elements"].as_array().into_iter().flatten() {
            let id = e["id"].as_str().unwrap_or("");
            let name = e["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
            ui.horizontal(|ui| {
                ui.label(theme::strong(name).size(15.0));
                ui.label(theme::weak(
                    ui,
                    kind_label(e["kind"].as_str().unwrap_or("")),
                ));
            });
            ui.add_space(4.0);
            row(
                ui,
                &p,
                None,
                [t("Metric"), t("Mean"), t("± 95% CI"), t("Unit")],
            );
            for (i, key) in order.iter().enumerate() {
                let m = &e["metrics"][*key];
                if m.is_null() {
                    continue;
                }
                let (label, kind) = metric(key, e["kind"].as_str().unwrap_or(""));
                let (mean, hw) = (m["mean"].as_f64(), m["half_width"].as_f64());
                let (scale, unit) = match kind {
                    Unit::Share => (100.0, "%"),
                    Unit::Time => (1.0, unit),
                    Unit::Count | Unit::Items => (1.0, ""),
                };
                let show =
                    |v: Option<f64>| v.map_or_else(|| "–".to_owned(), |v| grouped(v * scale));
                row(
                    ui,
                    &p,
                    Some(i % 2 == 0),
                    [label, &show(mean), &show(hw), unit],
                );
            }
            // The mean value this element assigned to each property it sets.
            let shown = order
                .iter()
                .filter(|k| !e["metrics"][**k].is_null())
                .count();
            for (j, (name, m)) in e["properties"]
                .as_object()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let show = |v: Option<f64>| v.map_or_else(|| "–".to_owned(), grouped);
                row(
                    ui,
                    &p,
                    Some((shown + j) % 2 == 0),
                    [
                        &tf("{property} (set to, mean)", &[("property", name)]),
                        &show(m["mean"].as_f64()),
                        &show(m["half_width"].as_f64()),
                        "",
                    ],
                );
            }
            ui.add_space(16.0);
        }
        ui.label(theme::weak(
            ui,
            t(
                "± is the half-width of a 95% Student-t confidence interval of the mean across \
               replications.",
            ),
        ));
        ui.label(
            theme::weak(
                ui,
                tf(
                    "StochCraft {version} · model {digest}",
                    &[
                        ("version", r["stochcraft"].as_str().unwrap_or("")),
                        ("digest", r["model_digest"].as_str().unwrap_or("")),
                    ],
                ),
            )
            .size(12.0),
        );
    });
}

/// Column edges of a results row: the metric's name from the left, the mean and the half-width
/// right-aligned on fixed edges (so every element's table lines up), then the unit.
const COLUMNS: [(f32, f32); 4] = [
    (10.0, 230.0),
    (230.0, 360.0),
    (370.0, 500.0),
    (512.0, 580.0),
];

/// One row of an element's results. `stripe` is `None` for the header row.
fn row(ui: &mut egui::Ui, p: &theme::Palette, stripe: Option<bool>, cells: [&str; 4]) {
    let height = if stripe.is_some() { 28.0 } else { 22.0 };
    let width = COLUMNS[3].1;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    if stripe == Some(true) {
        ui.painter().rect_filled(rect, 4.0, p.stripe);
    }
    for (c, text) in cells.into_iter().enumerate() {
        let (l, r) = COLUMNS[c];
        let cell = egui::Rect::from_x_y_ranges(rect.left() + l..=rect.left() + r, rect.y_range());
        let numeric = c == 1 || c == 2;
        let layout = if numeric {
            egui::Layout::right_to_left(egui::Align::Center)
        } else {
            egui::Layout::left_to_right(egui::Align::Center)
        };
        let mut t = RichText::new(text);
        if stripe.is_none() {
            t = t.size(12.0).color(p.text_weak);
        } else if numeric {
            t = t.monospace();
            if c == 2 {
                t = t.color(p.text_weak);
            }
        } else if c == 3 {
            t = t.color(p.text_weak);
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(cell).layout(layout), |ui| {
            ui.label(t);
        });
    }
}

thread_local! {
    /// How the user's region writes numbers, set by the app each frame ([`set_numbers`]).
    static NUMBERS: std::cell::RefCell<Option<craft_ui::localized::LocaleNumbers>> =
        const { std::cell::RefCell::new(None) };
}

/// Writes the results' numbers as `numbers` (the user's region) does from now on, on this thread.
pub fn set_numbers(numbers: &craft_ui::localized::LocaleNumbers) {
    NUMBERS.with(|n| *n.borrow_mut() = Some(numbers.clone()));
}

/// [`number`]'s digits in the user's region: its decimal mark, grouping and digits (`14.990` in
/// German). Without a region set, thousands are grouped with commas.
pub fn grouped(v: f64) -> String {
    let s = number(v);
    if s.contains('e') || !v.is_finite() {
        return s;
    }
    let decimals = s.split_once('.').map_or(0, |(_, f)| f.len());
    if let Some(local) = NUMBERS.with(|n| {
        n.borrow()
            .as_ref()
            .map(|n| n.format(v, decimals..=decimals))
    }) {
        return local;
    }
    let (sign, rest) = s.strip_prefix('-').map_or(("", s.as_str()), |r| ("-", r));
    let (int, frac) = rest
        .split_once('.')
        .map_or((rest, None), |(i, f)| (i, Some(f)));
    if int.len() <= 4 {
        return s;
    }
    let mut out = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    match frac {
        Some(f) => format!("{sign}{out}.{f}"),
        None => format!("{sign}{out}"),
    }
}

#[cfg(test)]
mod tests {
    use super::number;

    #[test]
    fn numbers_read_well() {
        assert_eq!(number(4.0), "4");
        assert_eq!(number(3.98767), "3.9877");
        assert_eq!(number(0.74996), "0.74996");
        assert_eq!(number(1234.5678), "1234.6");
        assert_eq!(number(20000.0), "20000");
        assert_eq!(number(0.0001234), "1.2340e-4");
        assert_eq!(number(-2.5), "-2.5");
        assert_eq!(super::grouped(14990.0), "14,990");
        assert_eq!(super::grouped(1234.5678), "1234.6");
        assert_eq!(super::grouped(-123456.0), "-123,456");
    }
}
