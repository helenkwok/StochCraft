//! Validation against observed data (M2.9, #226) in the window:
//!
//! - in the model overview, **Observed data**: real measurements the results are checked against,
//!   each an observed value (with the half-width of its own 95% interval, where known) or a bound
//!   (at most, at least), added, changed and removed by the engine's `observation.add`,
//!   `observation.set` and `observation.remove`;
//! - in the results, **Checked against observed data**: for each, the observed value or bound
//!   beside the simulated mean and its 95% half-width, and whether they agree, in words; and a
//!   finding at the top for each that differs or fails.
//!
//! The engine decides ([`stochcraft_engine::observed`]); the window words what it decided.

use egui::RichText;
use serde_json::{Value, json};
use stochcraft_engine::precision::unit;

use crate::decide::{choose, kind_of, name_of, number, outputs};
use crate::editor::section;
use crate::i18n::{t, tf};
use crate::results::{grouped, metric, metric_label, rounded, scale_and_unit, unit_label};
use crate::{App, theme};

/// What a result is checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Value,
    AtMost,
    AtLeast,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Value, Kind::AtMost, Kind::AtLeast];

    /// The name the engine's `check` has.
    fn key(self) -> &'static str {
        match self {
            Kind::Value => "value",
            Kind::AtMost => "at_most",
            Kind::AtLeast => "at_least",
        }
    }

    fn of(observation: &Value) -> Self {
        match observation["check"].as_str() {
            Some("at_most") => Kind::AtMost,
            Some("at_least") => Kind::AtLeast,
            _ => Kind::Value,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Value => t("Observed value"),
            Kind::AtMost => t("At most"),
            Kind::AtLeast => t("At least"),
        }
    }
}

/// The engine's parameters for an observation: a value with the half-width of its own interval
/// (left out when 0, which says the same) and a tolerance (left out when 0: none), or a bound.
fn spec(
    element: &str,
    metric: &str,
    kind: Kind,
    value: f64,
    half_width: f64,
    tolerance: f64,
) -> Value {
    let mut s = json!({ "check": kind.key(), "element": element, "metric": metric });
    match kind {
        Kind::Value => {
            s["value"] = json!(value);
            if half_width > 0.0 {
                s["half_width"] = json!(half_width);
            }
            if tolerance > 0.0 {
                s["tolerance"] = json!(tolerance);
            }
        }
        Kind::AtMost | Kind::AtLeast => s["bound"] = json!(value),
    }
    s
}

/// A number after its unit as a sentence writes them: a share's `%` follows it directly, a time's
/// unit after a space.
fn with_unit(v: &str, unit: &str) -> String {
    match unit {
        "" => v.to_owned(),
        "%" => format!("{v}%"),
        u => format!("{v} {u}"),
    }
}

/// A value, with the half-width of its interval where it has one, rounded together to what the
/// interval supports and shown with the unit once. "–" without a value.
fn interval(mean: Option<f64>, half_width: Option<f64>, scale: f64, unit: &str) -> String {
    match (mean, half_width) {
        (Some(m), Some(h)) if h.is_finite() && h > 0.0 => {
            let (m, h) = rounded(m * scale, h * scale);
            with_unit(&format!("{m} ± {h}"), unit)
        }
        (Some(m), _) => with_unit(&grouped(m * scale), unit),
        (None, _) => "–".to_owned(),
    }
}

/// One check of the results in words.
struct Said {
    /// Agrees (within the tolerance), differs, no difference detected, passes, fails,
    /// inconclusive, or no such result.
    verdict: String,
    /// The run found the result wrong: it differs, or violates its bound.
    bad: bool,
    /// The run found the result right: it agrees, or meets its bound.
    good: bool,
    /// The result, what it was checked against and the simulated mean.
    sentence: String,
    /// For an observed value: the difference, with its own interval.
    difference: Option<String>,
    /// For no difference detected: that it is not evidence of agreement.
    note: Option<String>,
}

/// Check `c` of `results["observed"]` in words.
fn said(results: &Value, c: &Value) -> Said {
    let id = c["element"].as_str().unwrap_or("");
    let kind = kind_of_results(results, id);
    let name = c["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id);
    let (label, measure) = metric(c["metric"].as_str().unwrap_or(""), kind);
    let time = unit_label(results["record"]["time_unit"].as_str().unwrap_or(""));
    let (scale, u) = scale_and_unit(&measure, time);
    let simulated = if c["simulated"].is_object() {
        interval(
            c["simulated"]["mean"].as_f64(),
            c["simulated"]["half_width"].as_f64(),
            scale,
            u,
        )
    } else {
        "–".to_owned()
    };
    let mut args: Vec<(&str, String)> = vec![
        ("result", label.to_owned()),
        ("block", name.to_owned()),
        ("simulated", simulated),
    ];
    let sentence = match c["check"].as_str() {
        Some(bound @ ("at_most" | "at_least")) => {
            args.push(("bound", interval(c["bound"].as_f64(), None, scale, u)));
            let args = refs(&args);
            if bound == "at_most" {
                tf(
                    "{result} ({block}): must be at most {bound}, simulated {simulated}.",
                    &args,
                )
            } else {
                tf(
                    "{result} ({block}): must be at least {bound}, simulated {simulated}.",
                    &args,
                )
            }
        }
        _ => {
            let observed = interval(
                c["observed"]["value"].as_f64(),
                c["observed"]["half_width"].as_f64(),
                scale,
                u,
            );
            args.push(("observed", observed));
            tf(
                "{result} ({block}): observed {observed}, simulated {simulated}.",
                &refs(&args),
            )
        }
    };
    let difference = c["difference"].as_f64().map(|d| {
        tf(
            "Simulated minus observed: {difference}.",
            &[(
                "difference",
                &interval(Some(d), c["difference_half_width"].as_f64(), scale, u),
            )],
        )
    });
    let mut note = None;
    let (verdict, bad, good) = match c["verdict"].as_str() {
        Some("agrees") => {
            let tolerance = interval(c["observed"]["tolerance"].as_f64(), None, scale, u);
            (
                tf("Agrees within ±{tolerance}", &[("tolerance", &tolerance)]),
                false,
                true,
            )
        }
        Some("differs") => (t("Differs").to_owned(), true, false),
        Some("no_difference_detected") => {
            note = Some(
                t("This is not evidence of agreement: set a tolerance to test for it.").to_owned(),
            );
            (t("No difference detected").to_owned(), false, false)
        }
        Some("passes") => (t("Passes").to_owned(), false, true),
        Some("fails") => (t("Fails").to_owned(), true, false),
        Some("no_result") => (t("No result").to_owned(), false, false),
        _ => (
            t("Inconclusive at current precision").to_owned(),
            false,
            false,
        ),
    };
    Said {
        verdict,
        bad,
        good,
        sentence,
        difference,
        note,
    }
}

/// `args` as the pairs of text [`tf`] takes.
fn refs<'a>(args: &'a [(&'a str, String)]) -> Vec<(&'a str, &'a str)> {
    args.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

/// The kind of element `id` in the results (empty where the run has no such element).
fn kind_of_results<'a>(results: &'a Value, id: &str) -> &'a str {
    results["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|e| e["id"] == id)
        .and_then(|e| e["kind"].as_str())
        .unwrap_or("")
}

/// The checks the run made, if it made any.
fn checks(results: &Value) -> &[Value] {
    results["observed"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
}

/// A finding at the top of the results for each check that differs or fails (the engine lists
/// those first), so the answer comes before the tables.
pub fn findings(ui: &mut egui::Ui, results: &Value) {
    let p = theme::palette(ui.ctx());
    for c in checks(results) {
        let s = said(results, c);
        if !s.bad {
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::strong(t("Finding")).color(p.accent));
            ui.label(theme::strong(s.verdict.as_str()).color(p.error));
            ui.label(s.sentence.as_str());
        });
    }
}

/// "Checked against observed data": every check, differences first, with what it was checked
/// against, the simulated mean with its interval, and the verdict in words. Draws nothing for a
/// run without observed data.
pub fn section_of_results(ui: &mut egui::Ui, results: &Value) {
    let all = checks(results);
    if all.is_empty() {
        return;
    }
    let p = theme::palette(ui.ctx());
    ui.label(theme::strong(t("Checked against observed data")).size(15.0));
    ui.add_space(4.0);
    for c in all {
        let s = said(results, c);
        let colour = if s.bad {
            p.error
        } else if s.good {
            p.accent
        } else {
            p.text
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::strong(s.verdict.as_str()).color(colour));
            ui.label(s.sentence.as_str());
            if let Some(d) = &s.difference {
                ui.label(theme::weak(ui, d.as_str()));
            }
            if let Some(n) = &s.note {
                ui.label(theme::weak(ui, n.as_str()));
            }
        });
    }
    ui.add_space(10.0);
}

impl App {
    /// "Observed data" in the model overview: the observed values and bounds the results are
    /// checked against, each editable in its row, and a choice of result to add one for.
    pub(crate) fn observed_section(&mut self, ui: &mut egui::Ui, model: &Value) {
        section(ui, t("Observed data"));
        ui.label(theme::weak(
            ui,
            t("Real measurements to check the results against: the value seen in the real system, or a bound the result must meet."),
        ));
        let list = model["experiment"]["observed"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let time = model["experiment"]["time_unit"].as_str().unwrap_or("");
        for (i, o) in list.iter().enumerate() {
            if self.observation_row(ui, model, i, o, time) {
                // The list changed under the rows: draw it again next frame.
                ui.ctx().request_repaint();
                return;
            }
        }
        // Results not yet observed.
        let taken: Vec<String> = list
            .iter()
            .map(|o| {
                format!(
                    "{}/{}",
                    o["element"].as_str().unwrap_or(""),
                    o["metric"].as_str().unwrap_or("")
                )
            })
            .collect();
        let options: Vec<(String, String)> = outputs(model, self.results())
            .into_iter()
            .filter(|(key, _)| !taken.contains(key))
            .collect();
        let mut picked: Option<String> = None;
        ui.horizontal(|ui| {
            crate::decide::row_label(ui, t("Add an observed value"));
            choose(
                ui,
                "x/observed/add",
                t("Add an observed value"),
                &options,
                &mut picked,
                260.0,
            );
        });
        if let Some((element, metric)) = picked.as_deref().and_then(|k| k.split_once('/')) {
            self.execute(
                "observation.add",
                spec(element, metric, Kind::Value, 0.0, 0.0, 0.0),
            );
        }
    }

    /// The row of observation `i` (`o`): its result, what it is checked as, the number or two,
    /// and a button to remove it. Whether the list changed.
    fn observation_row(
        &mut self,
        ui: &mut egui::Ui,
        model: &Value,
        i: usize,
        o: &Value,
        time: &str,
    ) -> bool {
        let element = o["element"].as_str().unwrap_or("").to_owned();
        let metric_name = o["metric"].as_str().unwrap_or("").to_owned();
        let kind_name = kind_of(model, &element);
        let result = format!(
            "{} · {}",
            name_of(model, &element),
            metric_label(&metric_name, &kind_name)
        );
        let measure = unit(&metric_name);
        // A share is a fraction in the engine and a percentage here.
        let (scale, shown_unit) = scale_and_unit(&measure, unit_label(time));
        let kind = Kind::of(o);
        let key = if kind == Kind::Value {
            "value"
        } else {
            "bound"
        };
        let mut value = o[key].as_f64().unwrap_or(0.0) * scale;
        let mut half_width = o["half_width"].as_f64().unwrap_or(0.0) * scale;
        let mut tolerance = o["tolerance"].as_f64().unwrap_or(0.0) * scale;
        let before = (value, half_width, tolerance);
        let mut chosen = kind;
        let mut remove = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(result.as_str());
            craft_ui::ComboBox::from_id_salt(("x/observed/kind", i))
                .selected_text(kind.label())
                .width(130.0)
                .show_ui(ui, |ui| {
                    for k in Kind::ALL {
                        if ui.selectable_label(kind == k, k.label()).clicked() {
                            chosen = k;
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Checked as"))
                });
            number(
                ui,
                &self.numbers,
                &format!("x/observed/{i}/value"),
                if kind == Kind::Value {
                    t("Observed value")
                } else {
                    t("Value")
                },
                &mut value,
                90.0,
            );
            if !shown_unit.is_empty() {
                ui.label(RichText::new(shown_unit));
            }
            if kind == Kind::Value {
                ui.label("±");
                number(
                    ui,
                    &self.numbers,
                    &format!("x/observed/{i}/half_width"),
                    t("Its 95% half-width"),
                    &mut half_width,
                    70.0,
                );
            }
            if kind == Kind::Value {
                ui.label(t("Tolerance (0 for none)"));
                number(
                    ui,
                    &self.numbers,
                    &format!("x/observed/{i}/tolerance"),
                    t("Tolerance (0 for none)"),
                    &mut tolerance,
                    70.0,
                );
            }
            remove = ui.add(theme::secondary(ui, t("Remove"))).clicked();
        });
        if remove {
            self.execute(
                "observation.remove",
                json!({ "element": element, "metric": metric_name }),
            );
            return true;
        }
        if chosen != kind || (value, half_width, tolerance) != before {
            // Kept within what the engine takes: a half-width of 0 or more.
            let params = spec(
                &element,
                &metric_name,
                chosen,
                value / scale,
                half_width.max(0.0) / scale,
                tolerance.max(0.0) / scale,
            );
            self.execute("observation.set", params);
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stochcraft_engine::precision::Unit;

    fn results(checks: Value) -> Value {
        json!({
            "replications": 10,
            "record": { "replications": 10, "time_unit": "min" },
            "elements": [
                { "id": "desk", "name": "Desk", "kind": "server", "metrics": {} },
            ],
            "observed": checks,
        })
    }

    fn mean_wait(verdict: &str) -> Value {
        json!({
            "element": "desk", "name": "Desk", "metric": "mean_wait", "check": "value",
            "verdict": verdict,
            "observed": { "value": 3.2, "half_width": 0.4 },
            "simulated": { "mean": 5.1, "half_width": 0.3, "n": 10 },
            "difference": 1.9, "difference_half_width": 0.5,
        })
    }

    #[test]
    fn a_check_reads_as_a_sentence_with_its_verdict() {
        let r = results(json!([mean_wait("differs")]));
        let s = said(&r, &r["observed"][0]);
        assert_eq!(s.verdict, "Differs");
        assert!(s.bad && !s.good);
        assert_eq!(
            s.sentence,
            "Mean wait (Desk): observed 3.2 ± 0.4 min, simulated 5.1 ± 0.3 min."
        );
        assert_eq!(
            s.difference.as_deref(),
            Some("Simulated minus observed: 1.9 ± 0.5 min.")
        );
        // With a tolerance, agreeing says within what.
        let mut agrees = mean_wait("agrees");
        agrees["observed"]["tolerance"] = json!(0.5);
        let s = said(&r, &agrees);
        assert!(s.good && !s.bad);
        assert_eq!(s.verdict, "Agrees within ±0.5 min");
        // With none, no difference is not agreement, and says so.
        let s = said(&r, &mean_wait("no_difference_detected"));
        assert_eq!(s.verdict, "No difference detected");
        assert!(!s.good && !s.bad);
        assert!(
            s.note
                .is_some_and(|n| n.contains("not evidence of agreement"))
        );
    }

    #[test]
    fn a_bound_reads_with_its_unit_and_a_missing_result_with_a_dash() {
        let r = results(json!([]));
        let bound = json!({
            "element": "desk", "name": "Desk", "metric": "utilisation", "check": "at_most",
            "verdict": "inconclusive", "bound": 0.85,
            "simulated": { "mean": 0.83, "half_width": 0.02, "n": 10 },
        });
        let s = said(&r, &bound);
        assert_eq!(
            s.sentence,
            "Utilisation (Desk): must be at most 85%, simulated 83 ± 2%."
        );
        assert_eq!(s.verdict, "Inconclusive at current precision");
        assert!(!s.bad && !s.good && s.difference.is_none());
        let none = json!({
            "element": "gone", "name": "gone", "metric": "left", "check": "at_least",
            "verdict": "no_result", "bound": 5.0, "simulated": null,
        });
        let s = said(&r, &none);
        assert_eq!(s.verdict, "No result");
        assert_eq!(s.sentence, "Left (gone): must be at least 5, simulated –.");
    }

    #[test]
    fn the_parameters_are_the_engines() {
        assert_eq!(
            spec("desk", "mean_wait", Kind::Value, 3.2, 0.0, 0.0),
            json!({ "check": "value", "element": "desk", "metric": "mean_wait", "value": 3.2 })
        );
        assert_eq!(
            spec("desk", "mean_wait", Kind::Value, 3.2, 0.4, 0.5)["half_width"],
            0.4
        );
        assert_eq!(
            spec("desk", "mean_wait", Kind::Value, 3.2, 0.0, 0.5)["tolerance"],
            0.5
        );
        assert_eq!(
            spec("desk", "utilisation", Kind::AtMost, 0.85, 0.4, 0.5),
            json!({ "check": "at_most", "element": "desk", "metric": "utilisation", "bound": 0.85 })
        );
        assert_eq!(unit("utilisation"), Unit::Share);
    }
}
