//! The results of a run, in the words of the model: one section per element with its metrics,
//! their units and the half-width of each 95% confidence interval, under the run record that
//! makes them reproducible (the same content as `stochcraft-cli run`). A metric that only some
//! replications have a value for says how many ("n = 4 of 10"), and notes under the tables say
//! what the means leave out. A result whose replications split into two outcomes (the engine's
//! `split`) says so in words under its row, with a strip of every run's value, and in the
//! findings at the top of the results ([`findings`]).

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
        "balked" => (t("Balked"), Unit::Count),
        "balked_share" => (t("Share balked"), Unit::Share),
        "wasted" => (t("Work wasted on jobs that did not finish"), Unit::Time),
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

pub(crate) fn kind_label(kind: &str) -> &str {
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
            "{n} run · seed {seed} · warm-up {warmup} {unit} · {duration} {unit} measured",
            "{n} runs · seed {seed} · warm-up {warmup} {unit} · {duration} {unit} measured",
            &[
                ("seed", &r["seed"].to_string()),
                ("warmup", &num(&r["warmup"])),
                ("duration", &num(measured)),
                ("unit", unit),
            ],
        ),
    ));
}

/// "n = k of N" for result `m` where only k of the N replications of `results` had a value;
/// empty otherwise.
fn n_of(results: &Value, m: &Value) -> String {
    let replications = results["replications"]
        .as_u64()
        .or_else(|| results["record"]["replications"].as_u64());
    match (m["n"].as_u64(), replications) {
        (Some(k), Some(n)) if k < n => tf(
            "n = {k} of {n}",
            &[("k", &k.to_string()), ("n", &n.to_string())],
        ),
        _ => String::new(),
    }
}

/// The order of the results' tables: the engine's `metrics`.
fn order(results: &Value) -> Vec<&str> {
    results["metrics"]
        .as_array()
        .map(|m| m.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The top of the results under the findings: the warnings, then each block's key measures
/// ([`key_measures`]).
pub fn key(ui: &mut egui::Ui, results: &Value) {
    let p = theme::palette(ui.ctx());
    let r = &results["record"];
    let unit = unit_label(r["time_unit"].as_str().unwrap_or(""));
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
    ui.vertical(|ui| {
        ui.set_max_width(COLUMNS[4].1);
        for n in &notes {
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::strong(t("Warning")).color(p.warning));
                ui.label(*n);
            });
        }
    });
    if !notes.is_empty() {
        ui.add_space(10.0);
    }
    key_measures(ui, &p, results, unit, &|m| n_of(results, m));
}

/// Every result, folded under "All measures" (open where runs split, since the findings send the
/// reader to that table), then the notes on what the numbers mean and the run's version line.
pub fn show(ui: &mut egui::Ui, results: &Value) {
    let p = theme::palette(ui.ctx());
    let r = &results["record"];
    let unit = unit_label(r["time_unit"].as_str().unwrap_or(""));
    let order = order(results);
    let n_of = |m: &Value| n_of(results, m);
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
        let shown: usize = results["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| key_of(e).len())
            .sum();
        let rows = results["elements"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                let metrics = order
                    .iter()
                    .filter(|k| !e["metrics"][**k].is_null() && !hidden(e, k))
                    .count();
                metrics + e["properties"].as_object().map_or(0, serde_json::Map::len)
            })
            .sum::<usize>();
        let split = results["splits"].as_array().is_some_and(|s| !s.is_empty());
        ui.add_space(8.0);
        egui::CollapsingHeader::new(theme::strong(tf(
            "All measures ({n} more)",
            &[("n", &rows.saturating_sub(shown).to_string())],
        )))
        .id_salt("all-measures")
        .default_open(split)
        .show(ui, |ui| {
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
                if m.is_null() || hidden(e, key) {
                    continue;
                }
                let (label, kind) = metric(key, e["kind"].as_str().unwrap_or(""));
                let (scale, unit) = scale_and_unit(&kind, unit);
                let (mean, hw) = mean_and_half_width(m, scale);
                row(
                    ui,
                    &p,
                    Some(i % 2 == 0),
                    [label, &mean, &hw, unit, &n_of(m)],
                );
                // Where the runs split into two outcomes, the mean describes none of them: say
                // so under the row, and show every run's value.
                if let Some(sentence) = split_sentence(m, scale, unit) {
                    ui.horizontal(|ui| {
                        ui.add_space(COLUMNS[0].0);
                        ui.add(egui::Label::new(RichText::new(&sentence).color(p.text)).wrap());
                    });
                    split_strip(
                        ui,
                        &p,
                        &results["values"][id][*key],
                        m["split"]["cut"].as_f64().unwrap_or(0.0),
                        &sentence,
                    );
                    ui.add_space(4.0);
                }
            }
            // The mean value this element assigned to each property it sets.
            let shown = order
                .iter()
                .filter(|k| !e["metrics"][**k].is_null() && !hidden(e, k))
                .count();
            for (j, (name, m)) in e["properties"]
                .as_object()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let (mean, hw) = mean_and_half_width(m, 1.0);
                row(
                    ui,
                    &p,
                    Some((shown + j) % 2 == 0),
                    [
                        &tf("{property} (set to, mean)", &[("property", name)]),
                        &mean,
                        &hw,
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
        });
        if partial {
            ui.label(theme::weak(
                ui,
                t("n = k of N: only k of the N runs had a value for that result, and its mean and interval are over those k."),
            ));
        }
        if left_inside {
            ui.label(theme::weak(
                ui,
                t("Entities still inside at the end of a run are left out of the mean wait and the mean time in system."),
            ));
        }
        ui.label(theme::weak(
            ui,
            t(
                "± is the half-width of a 95% Student-t confidence interval of the mean across \
               runs.",
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

/// Whether a result of element `e` is left out of the tables: OEE and its availability where no
/// planned run time is set (without a plan to compare against, they only restate availability).
fn hidden(e: &Value, key: &str) -> bool {
    key.starts_with("oee") && e["metrics"]["oee_effectiveness"].is_null()
}

/// The results a block's key measures are chosen from, in order, by its kind: the first four
/// with a value are shown. Those marked `false` are shown only when not zero (a block that
/// never failed has no "Time failed" row among its key measures).
fn key_candidates(kind: &str) -> &'static [(&'static str, bool)] {
    match kind {
        "server" | "shared_task" => &[
            ("utilisation", true),
            ("mean_wait", true),
            ("mean_queue_length", true),
            ("time_failed", false),
            ("abandoned_share", false),
            ("balked_share", false),
            ("loss_probability", false),
            ("left", true),
        ],
        "delay" | "batch" | "unbatch" => {
            &[("mean_contents", true), ("left", true), ("lost", false)]
        }
        "sink" => &[
            ("entered", true),
            ("mean_time_in_system", true),
            ("cycle_time", true),
            ("demand_met", true),
        ],
        "source" => &[
            ("left", true),
            ("balked", false),
            ("lost", false),
            ("loss_probability", false),
        ],
        "resource" => &[
            ("utilisation", true),
            ("mean_in_service", true),
            ("time_failed", false),
        ],
        "exclusive_gateway" => &[("left", true), ("lost", false)],
        _ => &[("left", true), ("entered", true)],
    }
}

/// The key measures of block `e`: up to four of its results by [`key_candidates`].
fn key_of(e: &Value) -> Vec<&'static str> {
    let kind = e["kind"].as_str().unwrap_or("");
    key_candidates(kind)
        .iter()
        .filter(|(k, always)| {
            let m = &e["metrics"][*k];
            !m.is_null()
                && !hidden(e, k)
                && (*always || m["mean"].as_f64().is_some_and(|v| v != 0.0))
        })
        .map(|(k, _)| *k)
        .take(4)
        .collect()
}

/// Column edges of a key-measures row: the measure, its mean with the unit, the half-width, and
/// how many runs had a value where not all did.
const KEY_COLUMNS: [(f32, f32); 4] = [(8.0, 150.0), (150.0, 236.0), (242.0, 300.0), (306.0, 380.0)];

/// The key measures: up to four results per block in a compact table, the blocks side by side
/// where the width allows.
fn key_measures(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    results: &Value,
    time: &str,
    n_of: &dyn Fn(&Value) -> String,
) {
    let blocks: Vec<(&Value, Vec<&str>)> = results["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| (e, key_of(e)))
        .filter(|(_, keys)| !keys.is_empty())
        .collect();
    if blocks.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.label(theme::strong(t("Key measures")).size(15.0));
        ui.label(theme::weak(
            ui,
            t("Up to 4 per block, rounded to what the interval supports"),
        ));
    });
    ui.add_space(4.0);
    let width = KEY_COLUMNS[3].1;
    let gap = 28.0;
    let per_line = (((ui.available_width() + gap) / (width + gap)).floor() as usize).max(1);
    for line in blocks.chunks(per_line) {
        ui.horizontal_top(|ui| {
            for (e, keys) in line {
                {
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        let id = e["id"].as_str().unwrap_or("");
                        let name = e["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
                        let kind = e["kind"].as_str().unwrap_or("");
                        ui.horizontal(|ui| {
                            ui.label(theme::strong(name));
                            ui.label(theme::weak(ui, kind_label(kind)));
                        });
                        key_row(ui, p, None, [t("Metric"), t("Mean"), t("± 95% CI"), ""]);
                        for (i, key) in keys.iter().enumerate() {
                            let m = &e["metrics"][*key];
                            let (label, unit_kind) = metric(key, kind);
                            let (scale, unit) = scale_and_unit(&unit_kind, time);
                            let (mean, hw) = mean_and_half_width(m, scale);
                            let mean = match unit {
                                "" => mean,
                                "%" if mean != "–" => format!("{mean}%"),
                                u if mean != "–" => format!("{mean} {u}"),
                                _ => mean,
                            };
                            key_row(ui, p, Some(i % 2 == 0), [label, &mean, &hw, &n_of(m)]);
                        }
                    });
                }
                ui.add_space(gap);
            }
        });
        ui.add_space(10.0);
    }
}

/// One row of a block's key measures. `stripe` is `None` for the header row.
fn key_row(ui: &mut egui::Ui, p: &theme::Palette, stripe: Option<bool>, cells: [&str; 4]) {
    let height = if stripe.is_some() { 26.0 } else { 20.0 };
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(KEY_COLUMNS[3].1, height), egui::Sense::hover());
    if stripe == Some(true) {
        ui.painter().rect_filled(rect, 4.0, p.stripe);
    }
    for (c, text) in cells.into_iter().enumerate() {
        if text.is_empty() {
            continue;
        }
        let (l, r) = KEY_COLUMNS[c];
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
        } else if c == 2 || c == 3 {
            t = t.color(p.text_weak);
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(cell).layout(layout), |ui| {
            ui.label(t);
        });
    }
}

/// A result's mean and half-width as the tables show them, each multiplied by `scale`: rounded
/// together by [`rounded`] where both are known, otherwise as they are ("–" where missing).
fn mean_and_half_width(m: &Value, scale: f64) -> (String, String) {
    let show = |v: Option<f64>| v.map_or_else(|| "–".to_owned(), |v| grouped(v * scale));
    match (m["mean"].as_f64(), m["half_width"].as_f64()) {
        (Some(mean), Some(hw)) => rounded(mean * scale, hw * scale),
        (mean, hw) => (show(mean), show(hw)),
    }
}

/// A mean and the half-width of its interval, rounded to what the interval supports: the
/// half-width to one significant figure (two where that figure would be a 1), and the mean to
/// the same decimal place, so 38.4 ± 12.3 reads "38 ± 12" and 2.04 ± 0.61 reads "2.0 ± 0.6".
/// A half-width of zero (every run gave the same value) or one that is not a number leaves the
/// mean as [`grouped`] writes it.
pub fn rounded(mean: f64, half_width: f64) -> (String, String) {
    if !(mean.is_finite() && half_width.is_finite() && half_width > 0.0) {
        return (grouped(mean), grouped(half_width));
    }
    // The decimal place of the half-width's first significant figure; one place further where,
    // once rounded there, it reads as a 1 (0.96 → 1.0, 0.14 → 0.14, 12.3 → 12).
    let first = |v: f64| -magnitude(v);
    let mut places = first(half_width);
    let hw = round_to(half_width, places);
    if (hw / pow10(magnitude(hw))).round() as i64 == 1 {
        places = first(hw) + 1;
    }
    (
        fixed(round_to(mean, places), places),
        fixed(round_to(half_width, places), places),
    )
}

/// The power of ten of `v`'s first significant figure (`v` finite and not zero), by repeated
/// scaling rather than `log10`, as in [`number`].
fn magnitude(v: f64) -> i32 {
    let mut m = 0i32;
    let mut a = v.abs();
    while a >= 10.0 && m < 400 {
        a /= 10.0;
        m += 1;
    }
    while a < 1.0 && m > -400 {
        a *= 10.0;
        m -= 1;
    }
    m
}

/// Ten to the power `e`, by repeated multiplication (`powi` is disallowed: it can differ between
/// platforms); exact for the few places a table rounds to.
fn pow10(e: i32) -> f64 {
    let f = (0..e.unsigned_abs().min(400)).fold(1.0, |f, _| f * 10.0);
    if e < 0 { 1.0 / f } else { f }
}

/// `v` rounded to `places` decimal places (to tens, hundreds… where `places` is negative).
fn round_to(v: f64, places: i32) -> f64 {
    let f = pow10(places);
    (v * f).round() / f
}

/// `v` with `places` decimals (none where `places` is negative) in the user's region, grouped
/// as [`grouped`] groups.
fn fixed(v: f64, places: i32) -> String {
    let places = places.clamp(0, 12) as usize;
    // No "-0" where a small negative mean rounds to zero.
    let v = if v == 0.0 { 0.0 } else { v };
    if let Some(local) = NUMBERS.with(|n| n.borrow().as_ref().map(|n| n.format(v, places..=places)))
    {
        return local;
    }
    group(&format!("{v:.places$}"))
}

/// How a metric of `kind` is shown: the factor its values are multiplied by, and its unit
/// (`time` for times).
fn scale_and_unit<'a>(kind: &Unit, time: &'a str) -> (f64, &'a str) {
    match kind {
        Unit::Share => (100.0, "%"),
        Unit::Time => (1.0, time),
        Unit::Energy => (1.0, "kWh"),
        Unit::Count | Unit::Items => (1.0, ""),
    }
}

/// A value of a result as a sentence writes it: the table's number, with its unit.
fn with_unit(v: f64, scale: f64, unit: &str) -> String {
    let n = grouped(v * scale);
    match unit {
        "" => n,
        "%" => format!("{n}%"),
        u => format!("{n} {u}"),
    }
}

/// What a result whose replications split into two outcomes (its `split`) says in words: the
/// two groups, and that the mean mixes them. `None` when it does not split.
fn split_sentence(m: &Value, scale: f64, unit: &str) -> Option<String> {
    let s = &m["split"];
    let (low_n, high_n) = (s["low"]["n"].as_u64()?, s["high"]["n"].as_u64()?);
    let v = |x: &Value| x.as_f64().map(|x| with_unit(x, scale, unit));
    Some(tf(
        "Runs split: {low_n} of {n} near {low}, {high_n} near {high}. The mean ({mean}) mixes two outcomes; look at each group.",
        &[
            ("low_n", &low_n.to_string()),
            ("n", &(low_n + high_n).to_string()),
            ("low", &v(&s["low"]["mean"])?),
            ("high_n", &high_n.to_string()),
            ("high", &v(&s["high"]["mean"])?),
            ("mean", &v(&m["mean"])?),
        ],
    ))
}

/// A strip of every replication's value of a result (one tick per run, from the lowest to the
/// highest), with the cut between its two outcomes marked. `label` is what a screen reader
/// says for it.
fn split_strip(ui: &mut egui::Ui, p: &theme::Palette, values: &Value, cut: f64, label: &str) {
    let xs: Vec<f64> = values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    let (lo, hi) = xs
        .iter()
        .fold((cut, cut), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
    let height = 18.0;
    let (outer, response) =
        ui.allocate_exact_size(egui::vec2(COLUMNS[4].1, height), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, label));
    let rect = egui::Rect::from_min_size(
        outer.min + egui::vec2(COLUMNS[0].0, 0.0),
        egui::vec2(COLUMNS[2].1 - COLUMNS[0].0, height),
    );
    let x = |v: f64| {
        if hi > lo {
            rect.left() + ((v - lo) / (hi - lo)) as f32 * rect.width()
        } else {
            rect.center().x
        }
    };
    let painter = ui.painter_at(outer);
    let y = rect.center().y;
    painter.line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        egui::Stroke::new(1.0, p.border),
    );
    for v in &xs {
        let at = x(*v);
        painter.line_segment(
            [
                egui::pos2(at, rect.top() + 4.0),
                egui::pos2(at, rect.bottom() - 4.0),
            ],
            egui::Stroke::new(1.0, p.chart),
        );
    }
    let at = x(cut);
    painter.line_segment(
        [egui::pos2(at, rect.top()), egui::pos2(at, rect.bottom())],
        egui::Stroke::new(2.0, p.accent),
    );
}

/// The findings at the top of the results, so the answer comes before the tables: one per group
/// of results whose replications split into two outcomes the same way (the engine's `splits`),
/// so one collapse seen in many results reads as one finding. Draws nothing when there are none.
pub fn findings(ui: &mut egui::Ui, results: &Value) {
    let p = theme::palette(ui.ctx());
    for group in results["splits"].as_array().into_iter().flatten() {
        if let Some(text) = finding(results, group) {
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::strong(t("Finding")).color(p.accent));
                ui.label(text);
            });
        }
    }
}

/// The finding for one group of results whose runs split the same way (an entry of the
/// engine's `splits`), named by its lead: the first of them in table order.
fn finding(results: &Value, group: &Value) -> Option<String> {
    let members = group["results"].as_array()?;
    let lead = members.first()?;
    let (id, key) = (lead[0].as_str()?, lead[1].as_str()?);
    let e = results["elements"]
        .as_array()?
        .iter()
        .find(|e| e["id"] == id)?;
    let name = e["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
    let (label, _) = metric(key, e["kind"].as_str().unwrap_or(""));
    if members.len() == 1 {
        return Some(tf(
            "{result} ({element}) split into two outcomes: see the table below.",
            &[("result", label), ("element", name)],
        ));
    }
    let count = |v: &Value| v.as_u64().map(|n| n.to_string());
    Some(tf(
        "The runs split into two outcomes: {a} of {n} in one, {b} in the other. {count} results split the same way, among them {result} ({element}); see the table below.",
        &[
            ("a", &count(&group["sizes"][0])?),
            ("n", &count(&group["n"])?),
            ("b", &count(&group["sizes"][1])?),
            ("count", &members.len().to_string()),
            ("result", label),
            ("element", name),
        ],
    ))
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
    group(&s)
}

/// A number written with a decimal point, its thousands grouped with commas where the whole part
/// has five digits or more.
fn group(s: &str) -> String {
    let (sign, rest) = s.strip_prefix('-').map_or(("", s), |r| ("-", r));
    let (int, frac) = rest
        .split_once('.')
        .map_or((rest, None), |(i, f)| (i, Some(f)));
    if int.len() <= 4 {
        return s.to_owned();
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

    #[test]
    fn a_mean_is_rounded_to_what_its_interval_supports() {
        use super::rounded;
        let r = |m, h| {
            let (m, h) = rounded(m, h);
            format!("{m} ± {h}")
        };
        // One significant figure in the half-width, the mean to the same place.
        assert_eq!(r(80.696, 2.8373), "81 ± 3");
        assert_eq!(r(2.04, 0.61), "2.0 ± 0.6");
        assert_eq!(r(0.3042, 0.0813), "0.30 ± 0.08");
        // Two where the first figure is a 1, also once rounded (0.96 → 1.0).
        assert_eq!(r(38.4, 12.3), "38 ± 12");
        assert_eq!(r(5.03, 0.14), "5.03 ± 0.14");
        assert_eq!(r(7.37, 0.96), "7.4 ± 1.0");
        // Tens and hundreds where the interval is that wide; thousands grouped.
        assert_eq!(r(14990.0, 312.0), "15,000 ± 300");
        assert_eq!(r(1234.0, 56.0), "1230 ± 60");
        // No "-0", and a mean every run agreed on is left as it is.
        assert_eq!(r(-0.004, 0.2), "0.0 ± 0.2");
        assert_eq!(r(12.5, 0.0), "12.5 ± 0");
    }

    #[test]
    fn each_block_shows_its_key_measures_and_folds_the_rest() {
        use egui_kittest::kittest::Queryable;
        let m = |mean: f64, hw: f64| serde_json::json!({ "mean": mean, "half_width": hw, "std_dev": 1.0, "n": 10 });
        let results = serde_json::json!({
            "replications": 10,
            "record": { "replications": 10, "time_unit": "min" },
            "metrics": ["entered", "left", "mean_queue_length", "mean_wait", "utilisation",
                        "time_failed", "oee_availability", "oee"],
            "elements": [{
                "id": "paint", "name": "Paint", "kind": "server",
                "metrics": {
                    "entered": m(80.0, 3.0),
                    "left": m(79.0, 3.0),
                    "mean_queue_length": m(6.04, 2.1),
                    "mean_wait": m(38.4, 12.3),
                    "utilisation": m(0.9213, 0.0207),
                    "time_failed": m(0.0, 0.0),
                    // No planned run time: no effectiveness, so neither OEE row is shown.
                    "oee_availability": m(0.95, 0.01),
                },
            }],
        });
        let h = draw(results);
        h.get_by_label("Key measures");
        // Busy, the wait and the waiting room, rounded, with their units; a block that never
        // failed has no "Time failed" row among them, so the fourth is what left.
        h.get_by_label("92%");
        h.get_by_label("38 min");
        h.get_by_label("12");
        h.get_by_label("6");
        h.get_by_label("79");
        // Six rows in all (OEE left out), four of them key: two more, folded.
        h.get_by_label("All measures (2 more)");
        assert!(h.query_by_label("Time failed").is_none());
        assert!(h.query_by_label("OEE availability").is_none());
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
                crate::theme::install(ui.ctx(), &crate::theme::default_theme());
                fonts = true;
                ui.ctx().request_repaint();
                return;
            }
            super::findings(ui, &results);
            super::key(ui, &results);
            super::show(ui, &results);
        });
        h.run();
        h
    }

    /// Results of 128 replications whose units in service split: 99 runs near 1.7, 29 near
    /// 9.9, with the share in service (a percentage) split the same way.
    fn split() -> serde_json::Value {
        let runs: Vec<f64> = (0..128)
            .map(|i| if i % 9 == 4 || i < 17 { 9.9 } else { 1.7 })
            .collect();
        let low = runs.iter().filter(|v| **v < 5.0).count();
        assert_eq!(low, 99);
        let mean = runs.iter().sum::<f64>() / 128.0;
        let split = |scale: f64| {
            serde_json::json!({
                "cut": 5.8 * scale,
                "low": { "n": 99, "mean": 1.7 * scale },
                "high": { "n": 29, "mean": 9.9 * scale },
            })
        };
        let m = |mean: f64, split: serde_json::Value| serde_json::json!({ "mean": mean, "half_width": 0.6, "std_dev": 3.4, "n": 128, "split": split });
        serde_json::json!({
            "replications": 128,
            "record": { "replications": 128, "time_unit": "min" },
            "metrics": ["entered", "mean_in_service", "utilisation"],
            "elements": [{
                "id": "desk", "name": "Desk", "kind": "server",
                "metrics": {
                    "entered": { "mean": 500.0, "half_width": 3.0, "std_dev": 9.0, "n": 128 },
                    "mean_in_service": m(mean, split(1.0)),
                    "utilisation": m(mean / 10.0, split(0.1)),
                },
            }],
            "values": { "desk": {
                "mean_in_service": runs,
                "utilisation": runs.iter().map(|v| v / 10.0).collect::<Vec<_>>(),
            } },
            // Both split the same runs: one group, led by the first in table order.
            "splits": [{
                "id": 1, "n": 128, "sizes": [99, 29],
                "results": [["desk", "mean_in_service"], ["desk", "utilisation"]],
            }],
        })
    }

    #[test]
    fn runs_that_split_into_two_outcomes_say_so_in_words() {
        use egui_kittest::kittest::Queryable;
        let h = draw(split());
        // The row's sentence, and the strip of runs read out with the same words.
        let sentence = "Runs split: 99 of 128 near 1.7, 29 near 9.9. The mean (3.5578) mixes two outcomes; look at each group.";
        assert_eq!(h.query_all_by_label(sentence).count(), 2);
        // A share reads as the table's percentage.
        let share = "Runs split: 99 of 128 near 17%, 29 near 99%. The mean (35.578%) mixes two outcomes; look at each group.";
        assert_eq!(h.query_all_by_label(share).count(), 2);
        // And one finding at the top for the group, named by its lead.
        h.get_by_label(
            "The runs split into two outcomes: 99 of 128 in one, 29 in the other. 2 results \
             split the same way, among them Mean units in service (Desk); see the table below.",
        );
        assert_eq!(h.query_all_by_label("Finding").count(), 1);

        // Two groups of one (different runs): a finding each, naming its result.
        let mut apart = split();
        apart["splits"] = serde_json::json!([
            { "id": 1, "n": 128, "sizes": [99, 29], "results": [["desk", "mean_in_service"]] },
            { "id": 2, "n": 128, "sizes": [70, 58], "results": [["desk", "utilisation"]] },
        ]);
        let h = draw(apart);
        h.get_by_label(
            "Mean units in service (Desk) split into two outcomes: see the table below.",
        );
        h.get_by_label("Utilisation (Desk) split into two outcomes: see the table below.");
        assert_eq!(h.query_all_by_label("Finding").count(), 2);
        assert!(h.query_by_label_contains("split the same way").is_none());

        // Without a split: no sentence and no finding.
        let mut none = split();
        for m in ["mean_in_service", "utilisation"] {
            if let Some(o) = none["elements"][0]["metrics"][m].as_object_mut() {
                o.remove("split");
            }
        }
        if let Some(o) = none.as_object_mut() {
            o.remove("splits");
        }
        let h = draw(none);
        assert!(h.query_by_label_contains("Runs split").is_none());
        assert!(h.query_by_label_contains("two outcomes").is_none());
        assert!(h.query_by_label("Finding").is_none());
    }

    #[test]
    fn a_result_from_fewer_than_all_replications_says_how_many() {
        use egui_kittest::kittest::Queryable;
        let h = draw(partial());
        // Only the mean wait is from fewer than all: one "n = k of N", and the note explaining it.
        assert_eq!(h.query_all_by_label("n = 4 of 10").count(), 1);
        assert!(h.query_by_label("n = 10 of 10").is_none());
        h.get_by_label_contains("n = k of N: only k of the N runs");
        h.get_by_label_contains("Entities still inside at the end of a run are left out");

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
