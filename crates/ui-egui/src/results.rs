//! The results of a run, in the words of the model: one section per element with its metrics,
//! their units and the half-width of each 95% confidence interval, under the run record that
//! makes them reproducible (the same content as `stochcraft-cli run`). A metric that only some
//! replications have a value for says how many ("n = 4 of 10"), and notes under the tables say
//! what the means leave out.

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
    /// Kilowatt-hours.
    Energy,
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
        "work_in_progress" => (t("Still inside at the end"), Unit::Items),
        "mean_work_in_progress" => (t("Mean work in progress"), Unit::Items),
        "mean_time_in_system" => (t("Mean time in system"), Unit::Time),
        "mean_blocked_time" => (t("Mean time blocked"), Unit::Time),
        "blocking_probability" => (t("Share of units blocked"), Unit::Share),
        "lost" if kind == "exclusive_gateway" => (t("Lost (no way out)"), Unit::Count),
        "lost" => (t("Lost (waiting room full)"), Unit::Count),
        "loss_probability" => (t("Share of arrivals lost"), Unit::Share),
        "abandoned" => (t("Gave up"), Unit::Count),
        "abandoned_share" => (t("Share that gave up"), Unit::Share),
        "abandoned_wait" => (t("Mean wait before giving up"), Unit::Time),
        "wasted" => (t("Work wasted on those who gave up"), Unit::Time),
        "wasted_share" => (t("Share of work wasted"), Unit::Share),
        "taken_out" => (t("Units taken out by health checks"), Unit::Count),
        "mean_in_service" => (t("Mean units in service"), Unit::Items),
        "availability" => (t("Availability"), Unit::Share),
        "time_off_shift" => (t("Time off shift"), Unit::Share),
        "time_processing" => (t("Time processing"), Unit::Share),
        "time_starved" => (t("Time starved"), Unit::Share),
        "time_blocked" => (t("Time blocked"), Unit::Share),
        "time_failed" => (t("Time failed"), Unit::Share),
        "time_restarting" => (t("Time restarting"), Unit::Share),
        "time_setup" => (t("Time setting up"), Unit::Share),
        "oee_availability" => (t("OEE availability"), Unit::Share),
        "oee_effectiveness" => (t("OEE effectiveness"), Unit::Share),
        "setup_ratio" => (t("Setup ratio"), Unit::Share),
        "quality_ratio" => (t("Quality ratio"), Unit::Share),
        "scrap_ratio" => (t("Scrap ratio"), Unit::Share),
        "rework_ratio" => (t("Rework ratio"), Unit::Share),
        "oee" => (t("OEE"), Unit::Share),
        "takt" => (t("Takt time"), Unit::Time),
        "demand" => (t("Demand"), Unit::Count),
        "demand_met" => (t("Demand met"), Unit::Share),
        "cycle_time" => (t("Cycle time"), Unit::Time),
        "energy" => (t("Energy"), Unit::Energy),
        "specific_energy" => (t("Energy per part"), Unit::Energy),
        other => (other, Unit::Count),
    }
}

/// How metric `name` of an element of kind `kind` reads.
pub(crate) fn metric_label<'a>(name: &'a str, kind: &str) -> &'a str {
    metric(name, kind).0
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
        "batch" => t("Batch"),
        "unbatch" => t("Split batches"),
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

/// The run record's line under the results' heading: replications, seed, warm-up, and the time
/// actually measured (the mean over the replications whose statistics are used, which a stop
/// condition can make shorter than the duration; "–" when none is used).
pub fn record_line(ui: &mut egui::Ui, results: &Value) {
    let r = &results["record"];
    let unit = unit_label(r["time_unit"].as_str().unwrap_or(""));
    let num = |v: &Value| v.as_f64().map_or_else(|| "–".to_owned(), grouped);
    let measured = match &results["measured"] {
        m if m.is_object() => &m["mean_duration"],
        _ => &r["duration"],
    };
    ui.label(theme::weak(
        ui,
        tnf(
            r["replications"].as_u64().unwrap_or(0),
            "{n} replication · seed {seed} · warm-up {warmup} {unit} · {duration} {unit} measured",
            "{n} replications · seed {seed} · warm-up {warmup} {unit} · {duration} {unit} measured",
            &[
                ("seed", &r["seed"].to_string()),
                ("warmup", &num(&r["warmup"])),
                ("duration", &num(measured)),
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
    let replications = results["replications"]
        .as_u64()
        .or_else(|| r["replications"].as_u64());
    // "n = k of N" where only k of the N replications had a value (empty otherwise).
    let n_of = |m: &Value| match (m["n"].as_u64(), replications) {
        (Some(k), Some(n)) if k < n => tf(
            "n = {k} of {n}",
            &[("k", &k.to_string()), ("n", &n.to_string())],
        ),
        _ => String::new(),
    };
    let partial = results["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|e| {
            let metrics = e["metrics"].as_object().into_iter().flatten();
            let properties = e["properties"].as_object().into_iter().flatten();
            metrics.chain(properties).map(|(_, m)| m)
        })
        .any(|m| !n_of(m).is_empty());
    // Whether entities were still inside at the end of a replication somewhere.
    let left_inside = results["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|e| {
            e["metrics"]["work_in_progress"]["mean"]
                .as_f64()
                .is_some_and(|w| w > 0.0)
        });
    ui.vertical(|ui| {
        ui.set_max_width(COLUMNS[4].1);
        // Warnings from before the run (the record's), from its results, and the file's notes.
        let notes: Vec<&str> = r["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(results["warnings"].as_array().into_iter().flatten())
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
                [t("Metric"), t("Mean"), t("± 95% CI"), t("Unit"), ""],
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
                    Unit::Energy => (1.0, "kWh"),
                    Unit::Count | Unit::Items => (1.0, ""),
                };
                let show =
                    |v: Option<f64>| v.map_or_else(|| "–".to_owned(), |v| grouped(v * scale));
                row(
                    ui,
                    &p,
                    Some(i % 2 == 0),
                    [label, &show(mean), &show(hw), unit, &n_of(m)],
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
                        &n_of(m),
                    ],
                );
            }
            // Without scrap or rework marked, OEE counts quality as 100%: say so.
            if !e["metrics"]["oee"].is_null() && e["metrics"]["quality_ratio"].is_null() {
                ui.add_space(2.0);
                ui.label(theme::weak(
                    ui,
                    t("No scrap or rework is marked in this model, so OEE counts quality as 100%."),
                ));
            }
            // OEE needs a plan to compare against; say how to get it rather than guess one.
            if !e["metrics"]["oee_availability"].is_null()
                && e["metrics"]["oee_effectiveness"].is_null()
            {
                ui.add_space(2.0);
                ui.label(theme::weak(
                    ui,
                    t("Set a planned run time per item on this block to see its effectiveness."),
                ));
            }
            ui.add_space(16.0);
        }
        if partial {
            ui.label(theme::weak(
                ui,
                t("n = k of N: only k of the N replications had a value for that result, and its mean and interval are over those k."),
            ));
        }
        if left_inside {
            ui.label(theme::weak(
                ui,
                t("Entities still inside at the end of a replication are left out of the mean wait and the mean time in system."),
            ));
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
/// right-aligned on fixed edges (so every element's table lines up), the unit, then how many
/// replications had a value where not all did.
const COLUMNS: [(f32, f32); 5] = [
    (10.0, 230.0),
    (230.0, 360.0),
    (370.0, 500.0),
    (512.0, 580.0),
    (588.0, 690.0),
];

/// One row of an element's results. `stripe` is `None` for the header row.
fn row(ui: &mut egui::Ui, p: &theme::Palette, stripe: Option<bool>, cells: [&str; 5]) {
    let height = if stripe.is_some() { 28.0 } else { 22.0 };
    let width = COLUMNS[4].1;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    if stripe == Some(true) {
        ui.painter().rect_filled(rect, 4.0, p.stripe);
    }
    for (c, text) in cells.into_iter().enumerate() {
        if text.is_empty() {
            continue;
        }
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
        } else if c >= 3 {
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

    /// Results of 10 replications in which only 4 had a mean wait, and work was left inside.
    fn partial() -> serde_json::Value {
        let m = |mean: f64, n: u64| serde_json::json!({ "mean": mean, "half_width": 0.5, "std_dev": 1.0, "n": n });
        serde_json::json!({
            "replications": 10,
            "record": { "replications": 10, "time_unit": "min" },
            "metrics": ["entered", "mean_wait", "work_in_progress"],
            "elements": [{
                "id": "desk", "name": "Desk", "kind": "server",
                "metrics": {
                    "entered": m(12.0, 10),
                    "mean_wait": m(3.0, 4),
                    "work_in_progress": m(0.4, 10),
                },
            }],
        })
    }

    /// `results` drawn with the app's fonts (installed in the first frame).
    fn draw(results: serde_json::Value) -> egui_kittest::Harness<'static> {
        let mut fonts = false;
        let mut h = egui_kittest::Harness::new_ui(move |ui| {
            if !fonts {
                crate::theme::install_fonts(ui.ctx(), "en");
                crate::theme::install(ui.ctx());
                fonts = true;
                ui.ctx().request_repaint();
                return;
            }
            super::show(ui, &results);
        });
        h.run();
        h
    }

    #[test]
    fn a_result_from_fewer_than_all_replications_says_how_many() {
        use egui_kittest::kittest::Queryable;
        let h = draw(partial());
        // Only the mean wait is from fewer than all: one "n = k of N", and the note explaining it.
        assert_eq!(h.query_all_by_label("n = 4 of 10").count(), 1);
        assert!(h.query_by_label("n = 10 of 10").is_none());
        h.get_by_label_contains("n = k of N: only k of the N replications");
        h.get_by_label_contains("Entities still inside at the end of a replication are left out");

        // Every replication had every value and nothing was left inside: no n and no notes.
        let mut all = partial();
        all["elements"][0]["metrics"]["mean_wait"]["n"] = 10.into();
        all["elements"][0]["metrics"]["work_in_progress"]["mean"] = 0.0.into();
        let h = draw(all);
        assert!(h.query_by_label_contains("n = ").is_none());
        assert!(
            h.query_by_label_contains("still inside at the end")
                .is_none()
        );
    }
}
