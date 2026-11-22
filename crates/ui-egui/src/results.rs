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
/// percentage. The engine's, so the window judges results as sequential runs do.
pub(crate) use stochcraft_engine::precision::Unit;
/// The engine's: the rule sequential runs use too.
pub use stochcraft_engine::precision::runs_needed;
pub(crate) use stochcraft_engine::precision::{SETTLED, hidden, key_of};

/// The label and unit of metric `name` on an element of kind `kind`.
pub(crate) fn metric<'a>(name: &'a str, kind: &str) -> (&'a str, Unit) {
    (label(name, kind), stochcraft_engine::precision::unit(name))
}

/// The label of metric `name` on an element of kind `kind`.
fn label<'a>(name: &'a str, kind: &str) -> &'a str {
    match name {
        "entered" => t("Entered"),
        "left" => t("Left"),
        "mean_queue_length" => t("Mean queue length"),
        "mean_wait" => t("Mean wait"),
        "utilisation" => t("Utilisation"),
        "mean_contents" => t("Mean number inside"),
        "work_in_progress" => t("Still inside at the end"),
        "mean_work_in_progress" => t("Mean work in progress"),
        "mean_time_in_system" => t("Mean time in system"),
        "mean_blocked_time" => t("Mean time blocked"),
        "mean_time_on" => t("Mean time on the conveyor"),
        // A conveyor has no units: the share of time its front item waits at the end.
        "blocking_probability" if kind == "conveyor" => t("Time blocked"),
        "blocking_probability" => t("Share of units blocked"),
        "lost" if kind == "exclusive_gateway" => t("Lost (no way out)"),
        "lost" => t("Lost (waiting room full)"),
        "loss_probability" => t("Share of arrivals lost"),
        "abandoned" => t("Gave up"),
        "abandoned_share" => t("Share that gave up"),
        "abandoned_wait" => t("Mean wait before giving up"),
        "balked" => t("Balked"),
        "balked_share" => t("Share balked"),
        "wasted" => t("Work wasted on jobs that did not finish"),
        "wasted_share" => t("Share of work wasted"),
        "taken_out" => t("Units taken out by health checks"),
        "mean_in_service" => t("Mean units in service"),
        "availability" => t("Availability"),
        "time_off_shift" => t("Time off shift"),
        "time_processing" => t("Time processing"),
        "time_starved" => t("Time starved"),
        "time_blocked" => t("Time blocked"),
        "time_failed" => t("Time failed"),
        "time_restarting" => t("Time restarting"),
        "time_setup" => t("Time setting up"),
        "oee_availability" => t("OEE availability"),
        "oee_effectiveness" => t("OEE effectiveness"),
        "setup_ratio" => t("Setup ratio"),
        "quality_ratio" => t("Quality ratio"),
        "scrap_ratio" => t("Scrap ratio"),
        "rework_ratio" => t("Rework ratio"),
        "oee" => t("OEE"),
        "takt" => t("Takt time"),
        "demand" => t("Demand"),
        "demand_met" => t("Demand met"),
        "cycle_time" => t("Cycle time"),
        "energy" => t("Energy"),
        "specific_energy" => t("Energy per part"),
        "started" => t("Inside at the start"),
        other => other,
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
        "conveyor" => t("Conveyor"),
        "transport" => t("transport"),
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

/// A fix "How sure are these numbers?" offers, for the app to make in the model and run again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fix {
    /// Set the number of runs.
    Runs(u64),
    /// Set the warm-up, in the model's time unit.
    Warmup(f64),
    /// Let a sequential run go on to this many runs at most (its settings otherwise kept, as
    /// the results give them).
    Max(u64),
}

/// The block and result names of the watched results a sequential run lists under `status` in
/// its summary (`no_spread`, `mean_near_zero`): "Mean wait (Desk), Entered (Exit)".
fn listed(results: &Value, status: &str) -> String {
    named(
        results,
        results["sequential"][status]
            .as_array()
            .into_iter()
            .flatten(),
    )
}

/// The block and result names of `watched` (each `{element, metric}`), joined by commas.
fn named<'a>(results: &Value, watched: impl Iterator<Item = &'a Value>) -> String {
    let elements = results["elements"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    watched
        .map(|w| {
            let id = w["element"].as_str().unwrap_or("");
            let key = w["metric"].as_str().unwrap_or("");
            let e = elements.iter().find(|e| e["id"] == id);
            let kind = e.and_then(|e| e["kind"].as_str()).unwrap_or("");
            let block = e
                .and_then(|e| e["name"].as_str())
                .filter(|n| !n.is_empty())
                .unwrap_or(id);
            format!("{} ({block})", metric(key, kind).0)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// "How sure are these numbers?"'s rows for a sequential run (`results["sequential"]`): why it
/// stopped after how many runs, with a fix (more runs allowed) when the most was reached first,
/// the results not yet within its own targets, and those it could not settle by their spread or
/// their mean. Judged by the run's targets (`measures`, as the run judged them), not the fixed
/// ± 5% of a fixed run. Draws nothing for a fixed run. Returns the fix clicked, if any.
fn sequential_rows(ui: &mut egui::Ui, p: &theme::Palette, results: &Value) -> Option<Fix> {
    let q = &results["sequential"];
    let n = q["replications"].as_u64()?;
    let count = n.to_string();
    let measures = q["measures"].as_array().map_or(&[][..], Vec::as_slice);
    // The widest score against the run's own target, among those not within it.
    let widest = measures
        .iter()
        .filter(|m| m["status"] != "settled")
        .filter_map(|m| m["score"].as_f64())
        .fold(None, |w: Option<f64>, s| Some(w.map_or(s, |w| w.max(s))));
    // A run watching one named result (set from a file, the command line or an agent) speaks of
    // that result, not of every key measure.
    let one = q["settings"]["measure"]
        .is_object()
        .then(|| named(results, std::iter::once(&q["settings"]["measure"])));
    let mut fix = None;
    if q["reason"] == "target_met" {
        let text = match &one {
            Some(r) => tnf(
                n,
                "Stopped after {n} run: {result} is within its target.",
                "Stopped after {n} runs: {result} is within its target.",
                &[("result", r)],
            ),
            None => tnf(
                n,
                "Stopped after {n} run: every key measure that varies is within its target.",
                "Stopped after {n} runs: every key measure that varies is within its target.",
                &[],
            ),
        };
        trust_row(ui, p, true, &text, None);
    } else if q["reason"] == "max_reached" {
        let text = match &one {
            Some(r) => tnf(
                n,
                "Stopped at the most allowed, {n} run, before {result} was within its target.",
                "Stopped at the most allowed, {n} runs, before {result} was within its target.",
                &[("result", r)],
            ),
            None => tnf(
                n,
                "Stopped at the most allowed, {n} run, before every key measure was within its target.",
                "Stopped at the most allowed, {n} runs, before every key measure was within its target.",
                &[],
            ),
        };
        let needed = runs_needed(n, widest.filter(|s| *s > 1.0).unwrap_or(2.0));
        let label = tnf(needed, "Allow up to {n} run", "Allow up to {n} runs", &[]);
        if trust_row(ui, p, false, &text, Some(&label)) {
            fix = Some(Fix::Max(needed));
        }
    }
    let wide = named(
        results,
        measures.iter().filter(|m| {
            matches!(
                m["status"].as_str(),
                Some("wide" | "no_interval" | "no_value")
            )
        }),
    );
    if !wide.is_empty() {
        let text = tf(
            "Not yet within its target: {results}.",
            &[("results", &wide)],
        );
        trust_row(ui, p, false, &text, None);
    }
    let same = listed(results, "no_spread");
    if !same.is_empty() {
        let rate = q["unseen_rate"]
            .as_f64()
            .map_or_else(String::new, |r| format!("{}%", grouped(up_to(r * 100.0))));
        let text = tf(
            "{results}: the same in all {n} runs, so not counted as settled. Something none of the runs met could still happen in up to {rate} of runs.",
            &[("results", &same), ("n", &count), ("rate", &rate)],
        );
        trust_row(ui, p, false, &text, None);
    }
    let zero = listed(results, "mean_near_zero");
    if !zero.is_empty() {
        let text = tf(
            "{results}: the mean is at or near zero, so it cannot be known to a share of itself.",
            &[("results", &zero)],
        );
        trust_row(ui, p, false, &text, None);
    }
    fix
}

/// The score above which a key measure gets a warning: ± 10% of its mean, ± 2 points for a share.
const WIDE: f64 = 2.0;

/// How sure the runs are of result `m` (with `kind`, from [`metric`]): the engine's
/// [`stochcraft_engine::precision::score`], its half-width over what counts as settled, so 1 or
/// less is settled and more than [`WIDE`] gets a warning. `None` without an interval, or for a
/// mean of zero that is not a share.
fn score(m: &Value, kind: &Unit) -> Option<f64> {
    stochcraft_engine::precision::score(m, *kind)
}

/// How well the runs pin down each block's key measures ([`key_of`]), by [`score`]. Returns the
/// widest (its block, its result, its score), how many get a warning, how many are settled, and
/// how many were judged.
fn precision(results: &Value) -> (Option<(&Value, &'static str, f64)>, usize, usize, usize) {
    let mut widest: Option<(&Value, &'static str, f64)> = None;
    let (mut wide, mut settled, mut judged) = (0, 0, 0);
    for e in results["elements"].as_array().into_iter().flatten() {
        let kind = e["kind"].as_str().unwrap_or("");
        for key in key_of(e) {
            let Some(s) = score(&e["metrics"][key], &metric(key, kind).1) else {
                continue;
            };
            judged += 1;
            if s <= 1.0 {
                settled += 1;
            }
            if s > WIDE {
                wide += 1;
            }
            if widest.is_none_or(|(_, _, w)| s > w) {
                widest = Some((e, key, s));
            }
        }
    }
    (widest, wide, settled, judged)
}

/// A warm-up as a person would set it: two significant figures (437.2 → 440, 0.0234 → 0.023).
fn round_warmup(w: f64) -> f64 {
    if !(w.is_finite() && w > 0.0) {
        return w;
    }
    let places = 1 - magnitude(w);
    round_to(w, places)
}

/// One row of "How sure are these numbers?": its mark, its words and, where there is one, the
/// button for its fix. Returns whether the button was clicked.
fn trust_row(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    ok: bool,
    text: &str,
    button: Option<&str>,
) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 18.0), egui::Sense::hover());
        if ok {
            theme::paint_check_mark(ui.painter(), rect.center(), 6.5, p);
        } else {
            theme::paint_warning_mark(ui.painter(), rect.center(), 6.5, p);
        }
        let room = (ui.available_width() - if button.is_some() { 230.0 } else { 0.0 }).max(200.0);
        ui.scope(|ui| {
            ui.set_max_width(room);
            ui.add(egui::Label::new(text).wrap());
        });
        if let Some(label) = button {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clicked = ui.add(theme::secondary(ui, label)).clicked();
            });
        }
    });
    clicked
}

/// "How sure are these numbers?": whether the runs pin the key measures down, each problem with a
/// one-click fix (more runs, a warm-up), and how many key measures are already settled. Draws
/// nothing for results without runs. Returns the fix clicked, if any.
pub fn trust(ui: &mut egui::Ui, results: &Value) -> Option<Fix> {
    let p = theme::palette(ui.ctx());
    let r = &results["record"];
    let n = r["replications"].as_u64()?;
    let time = unit_label(r["time_unit"].as_str().unwrap_or(""));
    let (widest, wide, settled, judged) = precision(results);
    let no_warmup = results["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(r["warnings"].as_array().into_iter().flatten())
        .any(|w| w["code"] == "no_warmup");
    let mut fix = None;
    let frame = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, p.border))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(14, 10));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(theme::strong(t("How sure are these numbers?")));
        ui.add_space(4.0);
        // A sequential run chose its count itself: more runs come from allowing more, not from
        // setting the first ones.
        let sequential = results["sequential"].is_object();
        if sequential {
            fix = sequential_rows(ui, &p, results);
        }
        if n < 2 {
            let label = tnf(10, "Use {n} run", "Use {n} runs", &[]);
            let text = t("One run gives no interval, so these numbers cannot be told from chance.");
            // A sequential run's fix is in its own row above.
            let button = (!sequential).then_some(label.as_str());
            if trust_row(ui, &p, false, text, button) {
                fix = Some(Fix::Runs(10));
            }
        } else if let Some((e, key, score)) = widest.filter(|(_, _, s)| *s > WIDE) {
            let id = e["id"].as_str().unwrap_or("");
            let block = e["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
            let kind = e["kind"].as_str().unwrap_or("");
            let (label, unit_kind) = metric(key, kind);
            let (scale, unit) = scale_and_unit(&unit_kind, time);
            let (mean, hw) = mean_and_half_width(&e["metrics"][key], scale);
            // A share in points as the table shows it; anything else also as a share of its mean.
            let relative = grouped((score * SETTLED * 100.0).round());
            let value = match unit {
                "%" => format!("{mean} ± {hw}%"),
                "" => format!("{mean} ± {hw}, ± {relative}%"),
                u => format!("{mean} ± {hw} {u}, ± {relative}%"),
            };
            let mut text = tf(
                "{measure} ({block}) is known only to {value} after {n} runs.",
                &[
                    ("measure", label),
                    ("block", block),
                    ("value", &value),
                    ("n", &n.to_string()),
                ],
            );
            if wide > 1 {
                text.push(' ');
                text.push_str(&tf(
                    "Other key measures wider than ± 10% (± 2 points for a share): {k}.",
                    &[("k", &(wide - 1).to_string())],
                ));
            }
            let needed = runs_needed(n, score);
            let label = tnf(
                needed,
                "Use {n} run (about ± 5%)",
                "Use {n} runs (about ± 5%)",
                &[],
            );
            // A sequential run's fix is in its own row above.
            let button = (!sequential).then_some(label.as_str());
            if trust_row(ui, &p, false, &text, button) {
                fix = Some(Fix::Runs(needed));
            }
        }
        if no_warmup && r["warmup"].as_f64().is_none_or(|w| w == 0.0) {
            let text = t("The model starts empty and no warm-up is set, so the start pulls the means away from the long run.");
            match results["warmup_suggestion"]["warmup"].as_f64().map(round_warmup) {
                Some(w) if w > 0.0 => {
                    let label = tf(
                        "Set a {value} warm-up",
                        &[("value", &format!("{} {time}", grouped(w)))],
                    );
                    if trust_row(ui, &p, false, text, Some(&label)) {
                        fix = Some(Fix::Warmup(w));
                    }
                }
                _ => {
                    let text = format!(
                        "{text} {}",
                        t("The run is too short to suggest a warm-up: make it longer, then set one.")
                    );
                    trust_row(ui, &p, false, &text, None);
                }
            }
        }
        // A sequential run judged its key measures by its own targets, in its own rows above.
        if settled > 0 && !sequential {
            let text = tf(
                "{k} of {m} key measures are settled: each is known to ± 5% (± 1 point for a share) or better.",
                &[("k", &settled.to_string()), ("m", &judged.to_string())],
            );
            trust_row(ui, &p, true, &text, None);
        }
    });
    fix
}

/// The top of the results under the findings: the warnings, then each block's key measures
/// ([`key_measures`]).
pub fn key(ui: &mut egui::Ui, results: &Value) {
    let p = theme::palette(ui.ctx());
    crate::observed::section_of_results(ui, results);
    let r = &results["record"];
    let unit = unit_label(r["time_unit"].as_str().unwrap_or(""));
    // Warnings from before the run (the record's), from its results, and the file's notes.
    let notes: Vec<String> = r["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(results["warnings"].as_array().into_iter().flatten())
        .chain(r["file_notes"].as_array().into_iter().flatten())
        .filter(|n| n["severity"] != "info")
        // "How sure are these numbers?" says this one, with its fix ([`trust`]).
        .filter(|n| n["code"] != "no_warmup")
        .filter(|n| n["message"].is_string())
        // In the interface language where the engine gave the words' parts.
        .map(crate::i18n::engine_message)
        .collect();
    ui.vertical(|ui| {
        ui.set_max_width(COLUMNS[4].1);
        for n in &notes {
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::strong(t("Warning")).color(p.warning));
                ui.label(n.as_str());
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
        // A sequential run chose its count from the results, so its interval is an estimate of
        // precision whose coverage is not guaranteed.
        let interval = if results["sequential"].is_object() {
            t("± is the estimated precision: the half-width of a 95% Student-t confidence interval of the mean across runs. The number of runs was chosen from the results, so the interval may cover the true mean less often than 95%.")
        } else {
            t(
                "± is the half-width of a 95% Student-t confidence interval of the mean across \
               runs.",
            )
        };
        ui.label(theme::weak(ui, interval));
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
                        // Brought into view when a finding's "See the numbers" names this block.
                        let wanted = ui.ctx().data(|d| d.get_temp::<String>(scroll_target()));
                        if wanted.as_deref() == Some(id) {
                            ui.scroll_to_cursor(Some(egui::Align::TOP));
                            ui.ctx().data_mut(|d| d.remove::<String>(scroll_target()));
                        }
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
pub(crate) fn magnitude(v: f64) -> i32 {
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
pub(crate) fn pow10(e: i32) -> f64 {
    let f = (0..e.unsigned_abs().min(400)).fold(1.0, |f, _| f * 10.0);
    if e < 0 { 1.0 / f } else { f }
}

/// `v` to `figures` significant figures (60.0014 → 60.0 to three), for a number with no interval
/// to round it by.
pub(crate) fn round_sig(v: f64, figures: i32) -> f64 {
    if !v.is_finite() || v == 0.0 {
        return v;
    }
    round_to(v, figures - 1 - magnitude(v))
}

/// `v` rounded to `places` decimal places (to tens, hundreds… where `places` is negative).
/// An upper bound as a person would read it: rounded up, never down, to one decimal at 1 or
/// more and to two significant figures below (25.86 → 25.9, 0.02995 → 0.03), so a bound above
/// zero never shows as 0.
fn up_to(v: f64) -> f64 {
    if !(v.is_finite() && v > 0.0) {
        return v;
    }
    let places = if v >= 1.0 { 1 } else { 1 - magnitude(v) };
    let f = pow10(places);
    // Less a hair, so a product just above a whole number (259.000…01) does not round up past it.
    (v * f - 1e-9).ceil() / f
}

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
pub(crate) fn scale_and_unit<'a>(kind: &Unit, time: &'a str) -> (f64, &'a str) {
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

/// Where a finding's "See the numbers" asks the results to scroll: the id of a block whose key
/// measures [`key_measures`] brings into view on its next frame.
fn scroll_target() -> egui::Id {
    egui::Id::unique("results-scroll-to-block")
}

/// A block's name, or its id where it has none.
pub(crate) fn block_name(e: &Value) -> &str {
    let id = e["id"].as_str().unwrap_or("");
    e["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id)
}

/// A result's mean as a finding writes it: rounded to what its interval supports, with its unit
/// (`time` for times). `None` without a mean.
pub(crate) fn said(m: &Value, kind: &Unit, time: &str) -> Option<String> {
    let (scale, unit) = scale_and_unit(kind, time);
    let mean = m["mean"].as_f64()?;
    let text = match m["half_width"].as_f64() {
        Some(hw) => rounded(mean * scale, hw * scale).0,
        None => grouped(round_sig(mean * scale, 3)),
    };
    Some(match unit {
        "" => text,
        "%" => format!("{text}%"),
        u => format!("{text} {u}"),
    })
}

/// The busiest task: the server or task with shared resources with the highest mean utilisation.
pub(crate) fn busiest(results: &Value) -> Option<&Value> {
    results["elements"]
        .as_array()?
        .iter()
        .filter(|e| matches!(e["kind"].as_str(), Some("server" | "shared_task")))
        .filter(|e| e["metrics"]["utilisation"]["mean"].as_f64().is_some())
        .max_by(|a, b| {
            let u = |e: &Value| e["metrics"]["utilisation"]["mean"].as_f64().unwrap_or(0.0);
            u(a).total_cmp(&u(b))
        })
}

/// A block is said to limit the flow when it is busy at least this share of the time…
const LIMITS_BUSY: f64 = 0.85;
/// …and blocked by what comes after it less than this share (a block that is often blocked is
/// held back by a later one, so it is only the busiest).
const LIMITS_BLOCKED: f64 = 0.05;

/// The answers at the top of the results, before the trust line and the tables: what limits the
/// flow (or the busiest block, where the evidence for a limit is not clear), where an entity's
/// time goes, and what leaves against what arrives. Each comes with the block whose key measures
/// hold its numbers.
fn answers(results: &Value) -> Vec<(String, String)> {
    let time = unit_label(results["record"]["time_unit"].as_str().unwrap_or(""));
    let elements = results["elements"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut out = Vec::new();
    // A mean the runs split on describes none of them: a sentence built on one is left out, and
    // the split's own finding says what happened instead.
    let split = |m: &Value| m["split"].is_object();

    // What limits the flow.
    if let Some(e) = busiest(results).filter(|e| {
        !split(&e["metrics"]["utilisation"]) && !split(&e["metrics"]["mean_queue_length"])
    }) {
        let m = &e["metrics"];
        let busy = m["utilisation"]["mean"].as_f64().unwrap_or(0.0);
        let blocked = m["time_blocked"]["mean"].as_f64().unwrap_or(0.0);
        let queue = said(&m["mean_queue_length"], &Unit::Items, time).unwrap_or_else(|| "0".into());
        let args = [
            ("block", block_name(e)),
            (
                "busy",
                &said(&m["utilisation"], &Unit::Share, time).unwrap_or_default(),
            ),
            ("queue", &queue),
        ];
        let text = if busy >= LIMITS_BUSY && blocked < LIMITS_BLOCKED {
            tf(
                "{block} limits the flow: busy {busy} of the time, with {queue} waiting in front of it on average.",
                &args,
            )
        } else {
            tf(
                "{block} is the busiest block: busy {busy} of the time, with {queue} waiting in front of it on average.",
                &args,
            )
        };
        out.push((text, e["id"].as_str().unwrap_or("").to_owned()));
    }

    // Where the time goes: the time in the system, and the longest wait in it. Only with one
    // departures block, so that every wait is part of the time in the system it reports (with
    // two, the longest wait may be on the other one's route).
    let sinks: Vec<&Value> = elements.iter().filter(|e| e["kind"] == "sink").collect();
    let sink = match sinks.as_slice() {
        [s] if s["metrics"]["mean_time_in_system"]["mean"]
            .as_f64()
            .is_some()
            && !split(&s["metrics"]["mean_time_in_system"]) =>
        {
            Some(*s)
        }
        _ => None,
    };
    let longest = elements
        .iter()
        .filter(|e| {
            e["metrics"]["mean_wait"]["mean"]
                .as_f64()
                .is_some_and(|w| w > 0.0)
        })
        .max_by(|a, b| {
            let w = |e: &Value| e["metrics"]["mean_wait"]["mean"].as_f64().unwrap_or(0.0);
            w(a).total_cmp(&w(b))
        });
    if let (Some(s), Some(w)) = (sink, longest.filter(|w| !split(&w["metrics"]["mean_wait"]))) {
        let text = tf(
            "An entity spends {total} in the system, {wait} of it waiting at {block}.",
            &[
                (
                    "total",
                    &said(&s["metrics"]["mean_time_in_system"], &Unit::Time, time)
                        .unwrap_or_default(),
                ),
                (
                    "wait",
                    &said(&w["metrics"]["mean_wait"], &Unit::Time, time).unwrap_or_default(),
                ),
                ("block", block_name(w)),
            ],
        );
        out.push((text, w["id"].as_str().unwrap_or("").to_owned()));
    }

    // What leaves against what arrives, as rates over the time measured.
    let measured = match &results["measured"] {
        m if m.is_object() => m["mean_duration"].as_f64(),
        _ => results["record"]["duration"].as_f64(),
    }
    .filter(|d| *d > 0.0);
    let total = |kind: &str, key: &str| -> Option<(f64, f64)> {
        let ms: Vec<&Value> = elements
            .iter()
            .filter(|e| e["kind"] == kind)
            .map(|e| &e["metrics"][key])
            .filter(|m| m["mean"].as_f64().is_some())
            .collect();
        (!ms.is_empty() && !ms.iter().any(|m| split(m))).then(|| {
            ms.iter().fold((0.0, 0.0), |(mean, hw), m| {
                (
                    mean + m["mean"].as_f64().unwrap_or(0.0),
                    hw + m["half_width"].as_f64().unwrap_or(0.0),
                )
            })
        })
    };
    if let (Some(d), Some(out_n), Some(in_n), Some(s)) = (
        measured,
        total("sink", "entered"),
        total("source", "entered").or_else(|| total("source", "left")),
        elements.iter().find(|e| e["kind"] == "sink"),
    ) {
        // Per hour where the model's unit is shorter, so a rate reads "8.9 per h", not "0.0025 per s".
        let unit = results["record"]["time_unit"].as_str().unwrap_or("");
        let (per, label) = match unit {
            "ms" => (3_600_000.0, unit_label("hour")),
            "s" => (3_600.0, unit_label("hour")),
            "min" => (60.0, unit_label("hour")),
            _ => (1.0, time),
        };
        let rate = |(mean, hw): (f64, f64)| {
            let mean = mean / d * per;
            if hw > 0.0 {
                rounded(mean, hw / d * per).0
            } else {
                grouped(round_sig(mean, 3))
            }
        };
        let text = tf(
            "{out} per {unit} leave the system, from {in} per {unit} arriving.",
            &[("out", &rate(out_n)), ("in", &rate(in_n)), ("unit", label)],
        );
        out.push((text, s["id"].as_str().unwrap_or("").to_owned()));
    }
    out
}

/// The findings at the top of the results, so the answer comes before the tables: one per group
/// of results whose replications split into two outcomes the same way (the engine's `splits`),
/// so one collapse seen in many results reads as one finding. Draws nothing when there are none.
pub fn findings(ui: &mut egui::Ui, results: &Value) {
    let p = theme::palette(ui.ctx());
    // What differs from the observed data, or violates a bound, comes first.
    crate::observed::findings(ui, results);
    let answers = answers(results);
    if !answers.is_empty() {
        ui.label(theme::strong(t("What the run found")));
        ui.add_space(2.0);
    }
    for (text, block) in answers {
        ui.horizontal_wrapped(|ui| {
            ui.label(text);
            if ui.link(t("See the numbers")).clicked() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(scroll_target(), block.clone()));
            }
        });
    }
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

    #[test]
    fn more_runs_narrow_an_interval_with_their_square_root() {
        use super::runs_needed;
        // ± 32% after 10 runs (a score of 6.4): 6.4² × 10 = 409.6, so 410.
        assert_eq!(runs_needed(10, 6.4), 410);
        // ± 10%, or ± 2 points for a share, after 16: 64.
        assert_eq!(runs_needed(16, 2.0), 70);
        // Always more than now, even when nearly there.
        assert_eq!(runs_needed(25, 1.002), 30);
        assert_eq!(runs_needed(30, 1.002), 40);
        assert_eq!(super::round_warmup(437.2), 440.0);
        // An upper bound rounds up, and a small one keeps two figures rather than showing 0.
        assert_eq!(super::up_to(25.89), 25.9);
        assert_eq!(super::up_to(25.9), 25.9);
        assert_eq!(super::up_to(0.02995), 0.03);
        assert_eq!(super::up_to(0.0011), 0.0011);
        assert_eq!(super::round_warmup(0.0234), 0.023);
    }

    /// `trust` drawn for `results`, returning the fix clicked when `click` names a button.
    fn trust_with(results: serde_json::Value, click: Option<&str>) -> Option<super::Fix> {
        use egui_kittest::kittest::Queryable;
        let fix = std::rc::Rc::new(std::cell::Cell::new(None));
        let seen = std::rc::Rc::clone(&fix);
        let mut fonts = false;
        let mut h = egui_kittest::Harness::new_ui(move |ui| {
            if !fonts {
                crate::theme::install_fonts(ui.ctx(), "en");
                crate::theme::install(ui.ctx(), &crate::theme::default_theme());
                fonts = true;
                ui.ctx().request_repaint();
                return;
            }
            if let Some(f) = super::trust(ui, &results) {
                seen.set(Some(f));
            }
        });
        h.run();
        h.get_by_label("How sure are these numbers?");
        if let Some(label) = click {
            h.get_by_label(label).click();
            h.run();
        }
        fix.get()
    }

    /// A single-server queue run 10 times from empty: a wait known to ± 32%, busy to ± 2%.
    fn unsure() -> serde_json::Value {
        let m = |mean: f64, hw: f64| serde_json::json!({ "mean": mean, "half_width": hw, "std_dev": 1.0, "n": 10 });
        serde_json::json!({
            "replications": 10,
            "record": { "replications": 10, "time_unit": "min", "warmup": 0.0 },
            "metrics": ["left", "mean_queue_length", "mean_wait", "utilisation"],
            "elements": [{
                "id": "paint", "name": "Paint", "kind": "server",
                "metrics": {
                    "left": m(79.0, 3.0),
                    "mean_queue_length": m(6.0, 1.5),
                    "mean_wait": m(38.0, 12.16),
                    // 92% ± 0.8 points: settled, judged in points.
                    "utilisation": m(0.92, 0.008),
                },
            }],
            "warnings": [{ "code": "no_warmup", "severity": "warning", "message": "the run has no warm-up" }],
            "warmup_suggestion": { "warmup": 57.3 },
        })
    }

    #[test]
    fn the_trust_line_says_how_sure_and_offers_the_fixes() {
        use super::Fix;
        // The widest key measure, with more runs as its fix: 10 × (32 / 5)² = 409.6.
        assert_eq!(
            trust_with(unsure(), Some("Use 410 runs (about ± 5%)")),
            Some(Fix::Runs(410))
        );
        // No warm-up: the suggestion, rounded to two figures, as the other fix.
        assert_eq!(
            trust_with(unsure(), Some("Set a 57 min warm-up")),
            Some(Fix::Warmup(57.0))
        );
        // The rest is drawn without a click; the sentence names the measure and how sure it is.
        assert_eq!(trust_with(unsure(), None), None);

        // Narrow intervals and a warm-up set: nothing to fix, every key measure settled.
        let mut sure = unsure();
        for (k, hw) in [
            ("left", 0.5),
            ("mean_queue_length", 0.1),
            ("mean_wait", 0.5),
            ("utilisation", 0.01),
        ] {
            sure["elements"][0]["metrics"][k]["half_width"] = hw.into();
        }
        sure["warnings"] = serde_json::json!([]);
        sure["record"]["warmup"] = 60.0.into();
        assert_eq!(trust_with(sure, None), None);

        // One run: no interval at all.
        let mut one = unsure();
        one["record"]["replications"] = 1.into();
        assert_eq!(trust_with(one, Some("Use 10 runs")), Some(Fix::Runs(10)));
    }

    /// `unsure` as a sequential run that reached its most, 10 runs, with a count that was the
    /// same in every run.
    fn sequential(reason: &str) -> serde_json::Value {
        let mut r = unsure();
        r["sequential"] = serde_json::json!({
            "replications": 10,
            "stages": [10],
            "reason": reason,
            "met": reason == "target_met",
            "no_spread": [{ "element": "paint", "metric": "left" }],
            "mean_near_zero": [],
            "unseen_rate": 0.2589,
            // As the run judged them, against its own targets: the wait at 6.4 times its target.
            "measures": [
                { "element": "paint", "metric": "utilisation", "status": "settled", "score": 0.8 },
                { "element": "paint", "metric": "mean_wait", "status": "wide", "score": 6.4 },
                { "element": "paint", "metric": "left", "status": "no_spread", "score": null },
            ],
        });
        r
    }

    /// A sequential run that reached its most offers more runs allowed, not more first runs,
    /// and names the result that was the same in every run with how often something unseen may
    /// still happen.
    #[test]
    fn a_sequential_run_says_why_it_stopped() {
        use super::Fix;
        use egui_kittest::kittest::Queryable;
        // From the run's own score: 10 × 6.4² = 409.6, as the most allowed.
        assert_eq!(
            trust_with(sequential("max_reached"), Some("Allow up to 410 runs")),
            Some(Fix::Max(410))
        );
        let results = sequential("max_reached");
        let mut fonts = false;
        let mut h = egui_kittest::Harness::new_ui(move |ui| {
            if !fonts {
                crate::theme::install_fonts(ui.ctx(), "en");
                crate::theme::install(ui.ctx(), &crate::theme::default_theme());
                fonts = true;
                ui.ctx().request_repaint();
                return;
            }
            super::trust(ui, &results);
        });
        h.run();
        assert!(h.query_by_label("Use 410 runs (about ± 5%)").is_none());
        h.get_by_label(
            "Stopped at the most allowed, 10 runs, before every key measure was within its target.",
        );
        h.get_by_label("Not yet within its target: Mean wait (Paint).");
        // Judged by the run's targets only: no "settled at ± 5%" row beside it.
        assert!(
            h.query_by_label_contains("key measures are settled")
                .is_none()
        );
        h.get_by_label("Left (Paint): the same in all 10 runs, so not counted as settled. Something none of the runs met could still happen in up to 25.9% of runs.");
        // Stopped with the target met: said with a check mark, nothing to click for it.
        assert_eq!(
            trust_with(sequential("target_met"), Some("Set a 57 min warm-up")),
            Some(Fix::Warmup(57.0))
        );
        // A run watching one named result speaks of that result, not of every key measure.
        let mut one = sequential("target_met");
        one["sequential"]["settings"] =
            serde_json::json!({ "measure": { "element": "paint", "metric": "mean_wait" } });
        assert_eq!(
            trust_with(
                one,
                Some("Stopped after 10 runs: Mean wait (Paint) is within its target.")
            ),
            None
        );
    }

    #[test]
    fn the_trust_line_words_name_the_widest_measure() {
        use egui_kittest::kittest::Queryable;
        let results = unsure();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::new_ui(move |ui| {
            if !fonts {
                crate::theme::install_fonts(ui.ctx(), "en");
                crate::theme::install(ui.ctx(), &crate::theme::default_theme());
                fonts = true;
                ui.ctx().request_repaint();
                return;
            }
            super::trust(ui, &results);
        });
        h.run();
        h.get_by_label(
            "Mean wait (Paint) is known only to 38 ± 12 min, ± 32% after 10 runs. \
             Other key measures wider than ± 10% (± 2 points for a share): 1.",
        );
        h.get_by_label_contains("no warm-up is set");
        h.get_by_label(
            "2 of 4 key measures are settled: each is known to ± 5% (± 1 point for a share) or better.",
        );
    }

    #[test]
    fn a_share_is_judged_in_points() {
        use super::{Unit, score};
        let m = |mean: f64, hw: f64| serde_json::json!({ "mean": mean, "half_width": hw });
        // 1.7% ± 0.6 points: ± 35% of itself, but settled in points.
        assert!(score(&m(0.017, 0.006), &Unit::Share).is_some_and(|s| s <= 1.0));
        // 92% ± 3 points: a warning.
        assert!(score(&m(0.92, 0.03), &Unit::Share).is_some_and(|s| s > 2.0));
        // A time is judged against its mean, and a time of zero not at all.
        assert!(score(&m(38.0, 12.16), &Unit::Time).is_some_and(|s| (s - 6.4).abs() < 1e-9));
        assert!(score(&m(0.0, 0.0), &Unit::Time).is_none());
    }

    /// A line of two tasks over 1,000 min: Paint busy 92% and rarely blocked.
    fn line(blocked: f64) -> serde_json::Value {
        let m = |mean: f64, hw: f64| serde_json::json!({ "mean": mean, "half_width": hw, "n": 10 });
        serde_json::json!({
            "replications": 10,
            "record": { "replications": 10, "time_unit": "min", "duration": 1000.0 },
            "measured": { "mean_duration": 1000.0 },
            "elements": [
                { "id": "in", "name": "Parts arrive", "kind": "source",
                  "metrics": { "entered": m(150.0, 4.0), "left": m(150.0, 4.0) } },
                { "id": "weld", "name": "Weld", "kind": "server",
                  "metrics": { "utilisation": m(0.68, 0.02), "mean_wait": m(2.0, 0.6),
                               "mean_queue_length": m(0.3, 0.08), "time_blocked": m(0.0, 0.0) } },
                { "id": "paint", "name": "Paint", "kind": "server",
                  "metrics": { "utilisation": m(0.92, 0.02), "mean_wait": m(38.4, 12.3),
                               "mean_queue_length": m(6.04, 2.1), "time_blocked": m(blocked, 0.01) } },
                { "id": "out", "name": "Shipped", "kind": "sink",
                  "metrics": { "entered": m(142.0, 4.0), "mean_time_in_system": m(54.0, 13.0) } },
            ],
        })
    }

    #[test]
    fn the_findings_name_what_limits_the_flow_where_the_time_goes_and_the_rates() {
        let said: Vec<String> = super::answers(&line(0.01))
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        assert_eq!(
            said,
            [
                "Paint limits the flow: busy 92% of the time, with 6 waiting in front of it on average.",
                "An entity spends 54 min in the system, 38 min of it waiting at Paint.",
                // 142 and 150 in 1,000 min, per hour.
                "8.5 per h leave the system, from 9.0 per h arriving.",
            ]
        );
        // Often blocked by what comes after it: only the busiest, not the limit.
        let blocked = super::answers(&line(0.2));
        assert!(
            blocked[0]
                .0
                .starts_with("Paint is the busiest block: busy 92%")
        );
        // Each finding names the block whose numbers back it.
        let blocks: Vec<String> = super::answers(&line(0.01))
            .into_iter()
            .map(|(_, b)| b)
            .collect();
        assert_eq!(blocks, ["paint", "paint", "out"]);
    }

    #[test]
    fn a_finding_leaves_out_a_mean_the_runs_split_on_and_a_second_route() {
        // Paint's utilisation split: no sentence on it (the split's own finding says so).
        let mut split = line(0.01);
        split["elements"][2]["metrics"]["utilisation"]["split"] = serde_json::json!({ "cut": 0.5 });
        let said: Vec<String> = super::answers(&split).into_iter().map(|(t, _)| t).collect();
        assert!(said.iter().all(|t| !t.contains("busy")), "{said:?}");
        assert_eq!(said.len(), 2);
        // Two departures blocks: the longest wait may be on the other route, so no time sentence.
        let mut two = line(0.01);
        if let Some(es) = two["elements"].as_array_mut() {
            es.push(
                serde_json::json!({ "id": "scrap", "name": "Scrapped", "kind": "sink",
                "metrics": { "entered": { "mean": 8.0, "half_width": 1.0 },
                             "mean_time_in_system": { "mean": 2.0, "half_width": 0.5 } } }),
            );
        }
        let said: Vec<String> = super::answers(&two).into_iter().map(|(t, _)| t).collect();
        assert!(
            said.iter().all(|t| !t.contains("in the system,")),
            "{said:?}"
        );
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
