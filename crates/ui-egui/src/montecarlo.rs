//! Monte Carlo models in the window (ROADMAP M4.1): a third kind of document beside process
//! models, a calculation with uncertain inputs and no process.
//!
//! A model is a Monte Carlo one when it has the engine's Monte Carlo section and no blocks
//! (`Model::is_monte_carlo`, never stored here: undoing the edit that made one turns it back).
//! It shows two tabs, Model and Results, and none of a process model's views. The Model tab
//! edits the inputs (each a distribution, with the same editor as a task's time), the rank
//! correlations between them, the outputs (expressions over the inputs, with an optional target)
//! and the trials, seed, sampling and stop; each edit is one `mc.*` command, so one undo step. A
//! refused edit says why under its field, not only in the status bar.
//!
//! Run makes the trials a few batches a frame ([`stochcraft_engine::mc::McJob`]), so a long run
//! never holds up the window, the browser's page included (there the batches run one after
//! another on the page, not on its workers); the results are `mc.run`'s.
//!
//! The results answer the question first, as established Monte Carlo tools do: for each output
//! with a target, the chance of missing it in words ("about a 1 in 5 chance that cost is more than
//! 7,000 USD") with its interval; for each without, where 9 trials in 10 fall. Then each output's
//! mean with its interval, its percentiles, minimum and maximum, a histogram and the cumulative
//! curve with the target drawn on both, which inputs matter (in words, a tornado chart and each
//! input's rank correlation with the output), and how the run went (trials asked for and run, the
//! sampling, the seed, and a sequential stop's outcome).
//!
//! Under "Which inputs matter" a button works out each input's share of the output's spread
//! (Sobol indices, `mc.sobol`), on demand because it takes N × (inputs + 2) evaluations of the
//! model. It is made a few pieces a frame like a run ([`stochcraft_engine::mc::SobolJob`]), with
//! its progress in the status bar, and Cancel or Esc stops it. The shares are of the results on
//! show: they are dropped when the model is edited (and come back if the edit is undone, as the
//! results do), and a new run drops them. A model with correlated inputs has none, and says why.

use std::collections::HashMap;

use egui::{Align2, FontId, Rect, Stroke, pos2, vec2};
use serde_json::{Value, json};
use stochcraft_engine::mc::{McJob, SobolJob};

use crate::charts::nice_step;
use crate::decide::{Bound, bound_choice, choose, grid, number, percent, row_label};
use crate::editor::{params_schema, section};
use crate::form::{self, Schema};
use crate::i18n::{t, tf, tn, tnf};
use crate::results::{grouped, round_sig, rounded};
use crate::{App, Request, Running, commands, comparable, theme};

/// An edit to one output made while drawing the table: its row's key, the `mc.output.set`
/// parameters, and for a typed expression its field and text, kept there if the engine refuses it.
type OutputEdit = (String, Value, Option<(String, String)>);

/// The time a frame spends making trials before it draws.
pub(crate) const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(8);

/// The first distribution of a new input: a three-point estimate, the most common input.
fn first_distribution() -> Value {
    json!({ "type": "triangular", "min": 1.0, "mode": 2.0, "max": 4.0 })
}

/// What the Monte Carlo screens keep between frames.
#[derive(Default)]
pub(crate) struct Mc {
    /// The run in progress, made a few batches a frame.
    pub(crate) job: Option<McJob>,
    /// The shares of the spread being worked out, a few pieces a frame.
    sobol_job: Option<SobolJob>,
    /// The finished shares (`mc.sobol`'s results) and the results (their serial) they are of.
    shares: Option<(Value, u64)>,
    /// The serial of the results the job in progress is for.
    shares_for: u64,
    /// Why an edit was refused, by the key of the field or row it was made in.
    errors: HashMap<String, String>,
    /// The name typed for a new input, and for a new output.
    new_input: String,
    new_output: String,
    /// The pair chosen for a new correlation.
    pair: (Option<String>, Option<String>),
    /// A table of inputs pasted in, and whether it replaces every input.
    paste: String,
    replace: bool,
}

/// A number with its unit after it, when it has one.
fn with_unit(text: String, unit: &str) -> String {
    if unit.is_empty() {
        text
    } else {
        format!("{text} {unit}")
    }
}

/// A value for a sentence or a table: about four significant figures, in the user's region.
fn value_text(v: f64) -> String {
    grouped(round_sig(v, 4))
}

/// A chance as a percentage: to a tenth of a percent from 1% up, and below that to one
/// significant figure, so a small chance never reads as 0%.
fn chance(p: f64) -> String {
    let v = p * 100.0;
    if v > 0.0 && v < 1.0 {
        format!("{}%", grouped(round_sig(v, 1)))
    } else {
        percent(p)
    }
}

/// A share of the spread as a whole percentage, "61%", from 0% to 100%, and never with an
/// exponent. A Sobol estimate from a small sample can fall a little below 0 or above 1 (they are
/// estimates, and `mc.sobol` does not clamp them), which would read as an impossible share, so
/// those show as 0% and 100%.
pub(crate) fn share_text(x: f64) -> String {
    if x.is_finite() {
        // `+ 0.0` turns a negative zero into a zero, so it never reads "-0%".
        format!("{}%", grouped((x.clamp(0.0, 1.0) * 100.0).round() + 0.0))
    } else {
        "–".to_owned()
    }
}

/// An estimate's 95% interval in words, when it has one.
fn share_interval(e: &Value) -> Option<String> {
    let (lo, hi) = (e["interval"][0].as_f64()?, e["interval"][1].as_f64()?);
    Some(tf(
        "95% interval {low} to {high}",
        &[("low", &share_text(lo)), ("high", &share_text(hi))],
    ))
}

/// The shares of output `name`'s spread in `mc.sobol`'s results.
fn shares_of<'a>(shares: Option<&'a Value>, name: &str) -> Option<&'a Value> {
    shares?["outputs"]
        .as_array()?
        .iter()
        .find(|o| o["name"] == name)
}

/// What the "Which inputs matter" sections show of the shares of the spread.
struct SharesView<'a> {
    /// `mc.sobol`'s results, once worked out for the results on show.
    done: Option<&'a Value>,
    /// How far along working them out is (0 to 1), while it goes.
    busy: Option<f64>,
    /// The model evaluations working them out takes; none when no input varies.
    cost: Option<u64>,
    /// The model has correlated inputs, which have no shares.
    correlated: bool,
    /// The results on show are of the model as it is, so shares worked out now fit them.
    fresh: bool,
}

/// A percentile of output `o`'s results (`"5"`, `"95"`…).
fn percentile(o: &Value, p: &str) -> Option<f64> {
    o["percentiles"][p].as_f64()
}

/// The chance written as odds where they read naturally: "1 in 5" for 20%. Only for a chance from
/// 0.1% to 25%, where the nearest "1 in N" is within about 12% of it (the percentage is said
/// beside it anyway).
fn odds(chance: f64) -> Option<u64> {
    if !(0.001..=0.25).contains(&chance) {
        return None;
    }
    let n = (1.0 / chance).round();
    (n >= 4.0).then_some(n as u64)
}

/// An output's finding in words, and a second line on how sure it is: with a target, the chance
/// of missing it with its 95% interval; without one, where 9 trials in 10 fall and the mean.
/// `None` when the output has no value to speak of (no trial gave a number).
#[cfg(test)]
pub(crate) fn finding(o: &Value) -> Option<(String, Option<String>)> {
    finding_across(o, &Value::Null)
}

/// An output's finding (see `finding`, which tests use) with what the run's intervals are across (`interval_basis`), so that a missing
/// interval is explained in batches or in pairs.
pub(crate) fn finding_across(o: &Value, basis: &Value) -> Option<(String, Option<String>)> {
    let pairs = basis["across"] == "pairs";
    let name = o["name"].as_str().unwrap_or("");
    let unit = o["unit"].as_str().unwrap_or("");
    let target = &o["target"];
    if target.is_object() {
        let (at_most, x) = match (target["at_most"].as_f64(), target["at_least"].as_f64()) {
            (Some(x), _) => (true, x),
            (None, Some(x)) => (false, x),
            (None, None) => return None,
        };
        let met = target["probability"].as_f64()?;
        let miss = (1.0 - met).clamp(0.0, 1.0);
        let value = with_unit(value_text(x), unit);
        let shown = chance(miss);
        let in_odds = odds(miss);
        let odds_text = in_odds.map_or_else(String::new, |n| grouped(n as f64));
        let trials = target["of"].as_u64().unwrap_or(0).to_string();
        let args = [
            ("output", name),
            ("value", value.as_str()),
            ("percent", shown.as_str()),
            ("odds", odds_text.as_str()),
            ("n", trials.as_str()),
        ];
        // A controlled chance is an estimate, never a count of trials: no "in none of the trials".
        let estimated = target["controlled"] == true;
        let sentence = match (miss <= 0.0, in_odds.is_some(), at_most) {
            (true, _, true) if estimated => tf(
                "The estimated chance that {output} is more than {value} is {percent}.",
                &args,
            ),
            (true, _, false) if estimated => tf(
                "The estimated chance that {output} is less than {value} is {percent}.",
                &args,
            ),
            (true, _, true) => tf(
                "{output} was more than {value} in none of the {n} trials.",
                &args,
            ),
            (true, _, false) => tf(
                "{output} was less than {value} in none of the {n} trials.",
                &args,
            ),
            (false, true, true) => tf(
                "About a 1 in {odds} chance that {output} is more than {value} ({percent}).",
                &args,
            ),
            (false, true, false) => tf(
                "About a 1 in {odds} chance that {output} is less than {value} ({percent}).",
                &args,
            ),
            (false, false, true) => tf(
                "A {percent} chance that {output} is more than {value}.",
                &args,
            ),
            (false, false, false) => tf(
                "A {percent} chance that {output} is less than {value}.",
                &args,
            ),
        };
        // The interval of the chance of missing: the met share's, turned round.
        let interval = target["interval"][0]
            .as_f64()
            .zip(target["interval"][1].as_f64())
            .map(|(lo, hi)| ((1.0 - hi).clamp(0.0, 1.0), (1.0 - lo).clamp(0.0, 1.0)));
        let sure = match interval {
            Some((_, hi)) if miss <= 0.0 => tf(
                "The chance is under {hi}, with 95% confidence.",
                &[("hi", &chance(hi))],
            ),
            Some((lo, hi)) => tf(
                "95% interval of the chance: {lo} to {hi}.",
                &[("lo", &chance(lo)), ("hi", &chance(hi))],
            ),
            // Two batches or more with no interval: every batch gave the same share.
            None if o["interval_count"].as_u64().is_some_and(|n| n >= 2) => {
                let reason = if pairs {
                    t("No interval: every pair gave the same share, so their spread gives none.")
                } else {
                    t("No interval: every batch gave the same share, so their spread gives none.")
                };
                reason.to_owned()
            }
            None if pairs => t("No interval: too few pairs of trials for one.").to_owned(),
            None => t("No interval: too few batches of trials for one.").to_owned(),
        };
        return Some((sentence, Some(sure)));
    }
    let (low, high, mean) = (
        percentile(o, "5")?,
        percentile(o, "95")?,
        o["mean"].as_f64()?,
    );
    let low = with_unit(value_text(low), unit);
    let high = with_unit(value_text(high), unit);
    let sentence = match o["half_width"].as_f64() {
        Some(hw) => {
            let (m, h) = rounded(mean, hw);
            tf(
                "In 9 trials out of 10, {output} is between {low} and {high}; the mean is {mean} ± {half_width}.",
                &[
                    ("output", name),
                    ("low", &low),
                    ("high", &high),
                    ("mean", &with_unit(m, unit)),
                    ("half_width", &h),
                ],
            )
        }
        None => tf(
            "In 9 trials out of 10, {output} is between {low} and {high}; the mean is {mean}.",
            &[
                ("output", name),
                ("low", &low),
                ("high", &high),
                ("mean", &with_unit(value_text(mean), unit)),
            ],
        ),
    };
    // A squared quantity asked to be summarised by its root mean square: that is the finding,
    // in the quantity's own unit, and the mean of the squares the line beneath it. It is worked
    // out from the mean shown (a controlled one where the controls helped).
    if o["summary"]["kind"] == "rms"
        && let Some((rms, hw)) =
            stochcraft_model::OutputSummary::Rms.of(mean, o["half_width"].as_f64())
    {
        let rms_unit = stochcraft_model::OutputSummary::Rms.unit(unit);
        let headline = match hw {
            Some(hw) => {
                let (m, h) = rounded(rms, hw);
                tf(
                    "The root mean square of {output} is {value} ± {half_width}.",
                    &[
                        ("output", name),
                        ("value", &with_unit(m, &rms_unit)),
                        ("half_width", &h),
                    ],
                )
            }
            None => tf(
                "The root mean square of {output} is {value}.",
                &[
                    ("output", name),
                    ("value", &with_unit(value_text(rms), &rms_unit)),
                ],
            ),
        };
        return Some((headline, Some(sentence)));
    }
    // Every value the same: no interval to give, and no claim that the mean is exact.
    let sure = (o["no_variation"] == true).then(|| {
        t("No variation was observed, so there is no interval for the mean; a rare event the trials did not reach could still move it.")
            .to_owned()
    });
    Some((sentence, sure))
}

/// Whether output `o` (of a run's results or of the model) is a working step, which the
/// results fold away below the answers.
pub(crate) fn is_working(o: &Value) -> bool {
    o["role"] == "working"
}

/// An output's finding, its second line and what its controls did, as "What the run found"
/// lists them.
fn show_finding(ui: &mut egui::Ui, o: &Value, basis: &Value) {
    // Where the controls narrowed the interval, their result is the headline.
    if let Some((sentence, sure)) = finding_across(&controlled_headline(o), basis) {
        ui.add(egui::Label::new(sentence).wrap());
        if let Some(sure) = sure {
            ui.add(egui::Label::new(theme::weak(ui, sure)).wrap());
        }
        for line in control_lines(o) {
            ui.add(egui::Label::new(theme::weak(ui, line)).wrap());
        }
        ui.add_space(4.0);
    }
}

/// Whether a control's stated mean is far from the mean of its trials (the engine's
/// `mean_mismatch`), which makes the controlled estimate biased.
fn control_mismatch(o: &Value) -> bool {
    o["controlled"]["controls"]
        .as_array()
        .is_some_and(|c| c.iter().any(|u| u["mean_mismatch"] == true))
}

/// The output as the headline reads it: where the controls narrowed an interval by a tenth or
/// more, the controlled mean (for an output with no target) or the controlled chance of meeting
/// the target (with its interval) in place of the plain ones. Everything else is as it was. An
/// interval with no width is no narrowing: the plain result stays. So does it where a control's
/// stated mean is far from the trials' mean (the controlled result is biased until the stated
/// mean is corrected, and is shown beside the plain one, not in place of it). A controlled chance
/// is marked (`controlled`), so that it is worded as an estimate, and kept within 0 and 1.
pub(crate) fn controlled_headline(o: &Value) -> Value {
    let mut shown = o.clone();
    if control_mismatch(o) {
        return shown;
    }
    let e = &o["controlled"]["estimate"];
    if e["helped"] == true
        && !o["target"].is_object()
        && e["half_width"].as_f64().is_some_and(|h| h > 0.0)
    {
        shown["mean"] = e["mean"].clone();
        shown["half_width"] = e["half_width"].clone();
    }
    let t = &e["target"];
    if o["target"].is_object()
        && t["helped"] == true
        && t["interval"].is_array()
        && let Some(p) = t["probability"].as_f64()
    {
        shown["target"]["probability"] = json!(p.clamp(0.0, 1.0));
        shown["target"]["interval"] = t["interval"].clone();
        shown["target"]["controlled"] = json!(true);
    }
    shown
}

/// What an output's control variates did, in lines: which they are, then how much narrower they
/// made the interval of what the headline says (the target's chance for an output with a target,
/// else the mean), or that they did not help or made it wider (the plain result stands), or why
/// there is no controlled result, and a control whose stated mean the trials contradict.
pub(crate) fn control_lines(o: &Value) -> Vec<String> {
    let c = &o["controlled"];
    let Some(controls) = c["controls"].as_array() else {
        return Vec::new();
    };
    let name = o["name"].as_str().unwrap_or("");
    let names: Vec<&str> = controls.iter().filter_map(|u| u["name"].as_str()).collect();
    let mut lines = vec![tf(
        "Controls for {output}: {controls}.",
        &[("output", name), ("controls", &names.join(", "))],
    )];
    let e = &c["estimate"];
    if e.is_null() {
        lines.push(tf(
            "No controlled result: {reason}",
            &[("reason", c["unavailable"].as_str().unwrap_or(""))],
        ));
        return lines;
    }
    let with_target = o["target"].is_object() && e["target"].is_object();
    let what = if with_target { &e["target"] } else { e };
    // The controlled interval has no width where the trials show no spread at all: it is no
    // narrowing, it is no information.
    let flat = if with_target {
        what["interval"].is_null() && what["uncontrolled_interval"].is_array()
    } else {
        e["half_width"].as_f64() == Some(0.0)
    };
    let verdict = match what["narrowing"].as_f64() {
        // A narrower interval around a biased estimate is no gain: the plain result stands.
        Some(n) if what["helped"] == true && control_mismatch(o) => tf(
            "They narrowed the interval by {percent}, but a mean given for a control is far from the trials' mean, so the plain result stands.",
            &[("percent", &share_text(n))],
        ),
        Some(n) if what["helped"] == true => tf(
            "They narrowed the interval by {percent}; this is the result shown above.",
            &[("percent", &share_text(n))],
        ),
        Some(n) if n < 0.0 => tf(
            "They made the interval {percent} wider, so the plain result stands.",
            &[("percent", &share_text(-n))],
        ),
        None if flat => {
            t("Their interval has no width, which no estimate has, so the plain result stands.")
                .to_owned()
        }
        _ => match c["correlation"].as_f64().filter(|r| *r < 0.5) {
            Some(r) => tf(
                "They did not help: their correlation with {output} is weak ({correlation}), so the plain result stands.",
                &[("output", name), ("correlation", &rank_text(r))],
            ),
            None => t("They did not help, so the plain result stands.").to_owned(),
        },
    };
    lines.push(verdict);
    for u in controls.iter().filter(|u| u["mean_mismatch"] == true) {
        lines.push(tf(
            "The mean given for {name}, {stated}, is far from the trials' mean, {sample}: the controlled result is biased until it is corrected.",
            &[
                ("name", u["name"].as_str().unwrap_or("")),
                ("stated", &u["mean"].as_f64().map_or_else(|| "–".to_owned(), value_text)),
                (
                    "sample",
                    &u["sample_mean"].as_f64().map_or_else(|| "–".to_owned(), value_text),
                ),
            ],
        ));
    }
    lines
}

/// A sampling's name in a phrase (`mc.set`'s `sampling`).
fn sampling_name(sampling: &Value) -> &'static str {
    match sampling.as_str() {
        Some("latin_hypercube") => t("Latin hypercube sampling"),
        Some("antithetic") => t("antithetic sampling"),
        Some("sobol") => t("quasi-random (Sobol) sampling"),
        _ => t("random sampling"),
    }
}

/// The run in a line: trials, seed and sampling.
fn record_line(r: &Value) -> String {
    let sampling = sampling_name(&r["sampling"]);
    tnf(
        r["trials"].as_u64().unwrap_or(0),
        "{n} trial · seed {seed} · {sampling}",
        "{n} trials · seed {seed} · {sampling}",
        &[("seed", &r["seed"].to_string()), ("sampling", sampling)],
    )
}

/// What the intervals are across, in words: trials, batches or pairs (and why).
fn basis_line(r: &Value) -> String {
    let b = &r["interval_basis"];
    let count = b["count"].as_u64().unwrap_or(0);
    if b["across"] == "pairs" {
        tf(
            "Each ± is the half-width of the 95% interval of the mean across independent pairs of trials (pairs: {n}). The two trials of a pair are made to oppose each other, so each pair counts as one.",
            &[("n", &grouped(count as f64))],
        )
    } else if b["across"] == "batches" && r["sampling"] == "sobol" {
        tf(
            "Each ± is the half-width of the 95% interval of the mean across independent batches of {size} trials (batches: {n}). Each batch is shifted at random on its own, so each batch counts as one.",
            &[
                ("size", &grouped(b["size"].as_f64().unwrap_or(1000.0))),
                ("n", &grouped(count as f64)),
            ],
        )
    } else if b["across"] == "batches" {
        tnf(
            count,
            "Each ± is the half-width of the 95% interval of the mean across {n} independent batch of {size} trials. Latin hypercube sampling and correlated inputs make the trials within a batch depend on each other, so each batch counts as one.",
            "Each ± is the half-width of the 95% interval of the mean across {n} independent batches of {size} trials. Latin hypercube sampling and correlated inputs make the trials within a batch depend on each other, so each batch counts as one.",
            &[("size", &grouped(b["size"].as_f64().unwrap_or(1000.0)))],
        )
    } else {
        tn(
            count,
            "Each ± is the half-width of the 95% interval of the mean across {n} independent trial.",
            "Each ± is the half-width of the 95% interval of the mean across {n} independent trials.",
        )
    }
}

/// Whether the trials run differ from those asked for, and why, in words: whole batches.
fn rounding_line(r: &Value) -> Option<String> {
    let (run, asked) = (r["trials"].as_u64()?, r["trials_requested"].as_u64()?);
    (r["stop"].is_null() && run != asked).then(|| {
        let args = [
            ("asked", grouped(asked as f64)),
            (
                "size",
                grouped(r["interval_basis"]["size"].as_f64().unwrap_or(1000.0)),
            ),
            ("n", grouped(run as f64)),
        ];
        let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        match r["sampling"].as_str() {
            Some("antithetic") => tf(
                "{asked} trials were asked for; antithetic sampling runs whole pairs of trials, so {n} ran.",
                &args,
            ),
            Some("sobol") => tf(
                "{asked} trials were asked for; Sobol sampling runs whole batches of {size}, so {n} ran.",
                &args,
            ),
            _ => tf(
                "{asked} trials were asked for; with Latin hypercube sampling or correlated inputs a run is whole batches of {size}, so {n} ran.",
                &args,
            ),
        }
    })
}

/// How a sequential run stopped, in words.
fn stop_line(r: &Value) -> Option<String> {
    let stop = r["stop"].as_object()?;
    let n = r["trials"].as_u64().unwrap_or(0);
    let share = percent(stop.get("relative").and_then(Value::as_f64).unwrap_or(0.0));
    // The outputs named in `key` ("met", "not_established"), joined.
    let names = |key: &str| {
        stop.get(key).and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
    };
    Some(
        if stop.get("reason").and_then(Value::as_str) == Some("target_met")
            && let Some(open) = names("not_established").filter(|o| !o.is_empty())
        {
            // Stopped on its target, with some output that never varied: its precision is not
            // established, and an outcome rarer than about 3 in the trials (the rule of three)
            // could have gone unseen.
            let met = names("met").unwrap_or_default();
            let rare = (n / 3).max(1).to_string();
            tnf(
                n,
                "Stopped after {n} trial: precision met for {met}; not established for {open}, which gave the same value in every trial (an outcome rarer than about 1 in {rare} trials could have gone unseen).",
                "Stopped after {n} trials: precision met for {met}; not established for {open}, which gave the same value in every trial (an outcome rarer than about 1 in {rare} trials could have gone unseen).",
                &[("met", &met), ("open", &open), ("rare", &rare)],
            )
        } else if stop.get("reason").and_then(Value::as_str) == Some("target_met") {
            tnf(
                n,
                "Stopped after {n} trial: every output's mean is within ±{share} of itself.",
                "Stopped after {n} trials: every output's mean is within ±{share} of itself.",
                &[("share", &share)],
            )
        } else {
            tnf(
                n,
                "Stopped at the most allowed, {n} trial, before every output's mean was within ±{share} of itself.",
                "Stopped at the most allowed, {n} trials, before every output's mean was within ±{share} of itself.",
                &[("share", &share)],
            )
        },
    )
}

/// Text pasted as a table of inputs, made into the CSV `mc.inputs.import` reads: cells copied
/// from a spreadsheet come separated by tabs, and the header may be left out.
pub(crate) fn table_text(pasted: &str) -> String {
    let mut lines: Vec<String> = pasted
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            if l.contains('\t') {
                l.split('\t')
                    .map(|c| stochcraft_engine::wire::csv_field(c.trim()))
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                l.trim().to_owned()
            }
        })
        .collect();
    let headed = lines.first().is_some_and(|l| {
        l.to_ascii_lowercase()
            .replace([' ', '"'], "")
            .starts_with("name,distribution")
    });
    if !headed {
        lines.insert(0, "name,distribution,parameters,unit".to_owned());
    }
    lines.join("\n") + "\n"
}

impl App {
    /// The model is a Monte Carlo one.
    pub(crate) fn is_monte_carlo(&self) -> bool {
        self.engine.model().is_monte_carlo()
    }

    /// The results on show are a Monte Carlo run's.
    pub(crate) fn mc_results_shown(&self) -> bool {
        self.results
            .as_deref()
            .is_some_and(|r| r["kind"] == "monte_carlo")
    }

    /// Runs an `mc.*` edit made in the field or row `key`: its refusal is shown there.
    fn mc_edit(&mut self, key: &str, command: &str, params: Value) -> Option<Value> {
        match self.engine.execute(command, params) {
            Ok(v) => {
                self.mc.errors.remove(key);
                Some(v)
            }
            Err(e) => {
                self.mc.errors.insert(key.to_owned(), e.message);
                None
            }
        }
    }

    /// Whether an input or an output already has `name`; if so, the reason is shown under the
    /// field `key`, and the new one is not made (setting it would silently replace the old).
    fn mc_name_taken(&mut self, key: &str, name: &str) -> bool {
        let taken = self.engine.model().monte_carlo.as_ref().is_some_and(|m| {
            m.inputs.iter().any(|i| i.name == name) || m.outputs.iter().any(|o| o.name == name)
        });
        if taken {
            self.mc.errors.insert(
                key.to_owned(),
                tf(
                    "There is already an input or an output called {name}.",
                    &[("name", name)],
                ),
            );
        }
        taken
    }

    /// Why the edit in `key` was refused, under it.
    fn mc_error(&self, ui: &mut egui::Ui, key: &str) {
        if let Some(m) = self.mc.errors.get(key) {
            theme::error_message(ui, m.clone());
        }
    }

    /// File ▸ New Monte Carlo Model: an empty model with a Monte Carlo section.
    pub(crate) fn new_monte_carlo(&mut self) {
        if self.execute("model.new", json!({})).is_some() {
            self.forget_view();
            if self.execute("mc.set", json!({})).is_some() {
                self.info(t("New Monte Carlo model"));
            }
        }
    }

    /// Model ▸ Run on a Monte Carlo model: starts the trials, made a few batches each frame. A
    /// model with errors doesn't run: its next error is shown instead.
    pub(crate) fn mc_run(&mut self) {
        if self.running.is_some() || self.go_to_problem() {
            return;
        }
        match self.engine.mc_start(json!({})) {
            Ok(job) => {
                let (done, total) = job.progress();
                self.mc.job = Some(job);
                // The shares were of the results this run replaces.
                self.mc.sobol_job = None;
                self.mc.shares = None;
                self.show_results = true;
                self.run_model = Some(comparable(self.engine.model()));
                self.running = Some(Running {
                    done,
                    partial: 0.0,
                    total,
                    on_workers: false,
                });
                self.info(t("Running…"));
            }
            Err(e) => self.command_failed(e),
        }
    }

    /// Makes trials for about a frame's time; true while the run goes on.
    pub(crate) fn mc_poll(&mut self) -> bool {
        let Some(job) = self.mc.job.as_mut() else {
            return false;
        };
        // Natively the batches of a step are made on every core; in the browser one at a time,
        // so a frame is never held up by more than a batch.
        let batches = if cfg!(target_arch = "wasm32") { 1 } else { 16 };
        let start = web_time::Instant::now();
        let outcome = loop {
            let step = job.step(batches);
            if !matches!(step, Ok(false)) || start.elapsed() >= FRAME_BUDGET {
                break step;
            }
        };
        let (done, total) = job.progress();
        match outcome {
            Ok(false) => {
                if let Some(r) = &mut self.running {
                    r.done = done;
                    r.total = total;
                }
                true
            }
            Ok(true) => {
                let results = self
                    .mc
                    .job
                    .take()
                    .and_then(|mut job| self.engine.mc_results(&mut job));
                self.running = None;
                let model = self
                    .run_model
                    .take()
                    .unwrap_or_else(|| comparable(self.engine.model()));
                if let Some(results) = results {
                    let n = results["trials"].as_u64().unwrap_or(0);
                    self.set_results(results, model);
                    self.info(tn(n, "Finished {n} trial", "Finished {n} trials"));
                }
                false
            }
            Err(e) => {
                self.mc.job = None;
                self.running = None;
                self.report_error(e.message);
                false
            }
        }
    }

    /// The shares of the spread (Sobol indices), when they are of the results on show.
    fn current_shares(&self) -> Option<&Value> {
        let (shares, serial) = self.mc.shares.as_ref()?;
        (*serial == self.results_serial && self.results().is_some()).then_some(shares)
    }

    /// How far along working out the shares of the spread is (0 to 1), while it goes.
    pub(crate) fn shares_progress(&self) -> Option<f64> {
        self.mc.sobol_job.as_ref().map(SobolJob::progress)
    }

    /// The shares of the spread are being worked out.
    pub fn is_working_out_shares(&self) -> bool {
        self.mc.sobol_job.is_some()
    }

    /// Starts working out the shares of the spread for the results on show, a few pieces a
    /// frame. A refusal is reported in the status bar.
    fn mc_shares_start(&mut self) {
        if self.mc.sobol_job.is_some() || self.running.is_some() {
            return;
        }
        match self.engine.sobol_start(json!({})) {
            Ok(job) => {
                self.mc.sobol_job = Some(job);
                self.mc.shares = None;
                self.mc.shares_for = self.results_serial;
            }
            Err(e) => self.command_failed(e),
        }
    }

    /// Stops working out the shares.
    pub(crate) fn cancel_shares(&mut self) {
        if self.mc.sobol_job.take().is_some() {
            self.info(t("Run cancelled"));
        }
    }

    /// Works out the shares for about a frame's time; true while that goes on. They stop when
    /// the results they are for are no longer on show as they were run: another model replaced
    /// them, or the model was edited.
    pub(crate) fn sobol_poll(&mut self) -> bool {
        if self.results().is_none() || self.mc.shares_for != self.results_serial {
            self.mc.sobol_job = None;
            return false;
        }
        let Some(job) = self.mc.sobol_job.as_mut() else {
            return false;
        };
        let start = web_time::Instant::now();
        let done = loop {
            let done = job.step(1);
            if done || start.elapsed() >= FRAME_BUDGET {
                break done;
            }
        };
        if !done {
            return true;
        }
        let shares = self.mc.sobol_job.take().and_then(|mut job| job.results());
        let serial = self.mc.shares_for;
        self.mc.shares = shares.map(|s| (s, serial));
        false
    }

    /// The outline of a Monte Carlo model: its inputs and outputs, and how it runs.
    pub(crate) fn mc_outline(&mut self, ui: &mut egui::Ui, p: &theme::Palette) {
        let model = self.wire_model();
        let name = model["name"]
            .as_str()
            .filter(|n| !n.is_empty())
            .unwrap_or(t("Untitled model"))
            .to_owned();
        let overview = !self.show_results;
        if ui
            .add(
                egui::Button::selectable(overview, theme::strong(&name).size(17.0))
                    .frame_when_inactive(false),
            )
            .on_hover_text(t("The model: its inputs, outputs and trials"))
            .clicked()
        {
            self.show_results = false;
        }
        ui.label(theme::weak(ui, t("Monte Carlo model")));
        let mc = &model["monte_carlo"];
        egui::ScrollArea::vertical()
            .id_salt("mc/outline")
            .auto_shrink([false, true])
            // Room below for the trials, and the scenario choice when the file has scenarios.
            .max_height(ui.available_height() - 110.0 - self.sidebar_scenario_height())
            .show(ui, |ui| {
                for (title, key) in [(t("Inputs"), "inputs"), (t("Outputs"), "outputs")] {
                    ui.add_space(12.0);
                    ui.label(theme::strong(title).size(12.0).color(p.text_weak));
                    ui.add_space(2.0);
                    let rows = mc[key].as_array().cloned().unwrap_or_default();
                    if rows.is_empty() {
                        ui.label(theme::weak(ui, t("None yet")));
                    }
                    for row in rows {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(row["name"].as_str().unwrap_or(""));
                            let kind = row["distribution"]["type"].as_str().map(form::choice_label);
                            if let Some(kind) = kind {
                                ui.label(theme::weak(ui, kind));
                            }
                        });
                    }
                }
            });
        let x = &model["experiment"];
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(theme::weak(ui, sampling_name(&mc["sampling"])));
            if let Some(max) = mc["stop"]["max"].as_u64() {
                ui.label(theme::weak(
                    ui,
                    tf(
                        "then more until precise enough, at most {max}",
                        &[("max", &grouped(max as f64))],
                    ),
                ));
            }
            ui.label(tnf(
                x["replications"].as_u64().unwrap_or(0),
                "{n} trial · seed {seed}",
                "{n} trials · seed {seed}",
                &[("seed", &x["seed"].to_string())],
            ));
            // Laid out from the bottom: the scenario shows under the heading, above the trials.
            self.scenario_choice(ui, "sidebar");
            ui.add_space(2.0);
            ui.label(theme::strong(t("Trials")).size(12.0).color(p.text_weak));
            ui.add_space(4.0);
            ui.separator();
        });
    }

    /// The Model tab of a Monte Carlo model.
    pub(crate) fn mc_model_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui.ctx());
        let model = self.wire_model();
        let mc = model["monte_carlo"].clone();
        let problems = self.problems();
        egui::ScrollArea::vertical()
            .id_salt("mc/model")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(760.0));
                ui.heading(t("Monte Carlo model"));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(theme::weak(
                        ui,
                        t("A calculation with uncertain inputs, not a process: each trial draws every input once and works out every output from them."),
                    ))
                    .wrap(),
                );
                ui.add_space(8.0);
                let name = model["name"].as_str().unwrap_or("").to_owned();
                if let Some(n) = self.text_field(ui, "model/name", t("Name"), &name, true) {
                    self.execute("model.rename", json!({ "name": n }));
                }
                self.mc_inputs(ui, &p, &mc);
                self.mc_correlations(ui, &mc);
                self.mc_outputs(ui, &p, &mc);
                self.mc_settings(ui, &model);
                section(ui, t("Problems"));
                if problems.is_empty() {
                    ui.label(theme::weak(ui, t("None found. The model is ready to run.")));
                }
                for pr in &problems {
                    self.problem_row(ui, &p, pr, false);
                }
            });
    }

    /// A box around one input or output.
    fn mc_card(ui: &mut egui::Ui, p: &theme::Palette, add: impl FnOnce(&mut egui::Ui)) {
        egui::Frame::new()
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                add(ui);
            });
        ui.add_space(6.0);
    }

    /// The inputs: each with its unit and its distribution, a way to add one, and the table to
    /// paste or export.
    fn mc_inputs(&mut self, ui: &mut egui::Ui, p: &theme::Palette, mc: &Value) {
        section(ui, t("Inputs"));
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("Each input is a distribution: every trial draws one value from it."),
            ))
            .wrap(),
        );
        ui.add_space(4.0);
        let schema = params_schema("mc.input.set");
        let inputs = mc["inputs"].as_array().cloned().unwrap_or_default();
        let mut edits: Vec<(String, Value)> = Vec::new();
        let mut remove = None;
        for input in &inputs {
            let name = input["name"].as_str().unwrap_or("").to_owned();
            let unit = input["unit"].as_str().unwrap_or("").to_owned();
            let key = format!("mc/in/{name}");
            Self::mc_card(ui, p, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::strong(&name).size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(theme::secondary(ui, t("Remove")))
                            .on_hover_text(tf("Remove the input {name}", &[("name", &name)]))
                            .clicked()
                        {
                            remove = Some(name.clone());
                        }
                    });
                });
                if let Some(u) = self.text_field(ui, &format!("{key}/unit"), t("Unit"), &unit, true)
                {
                    edits.push((key.clone(), json!({ "name": name, "unit": u })));
                }
                let changed = {
                    let mut cx = form::Cx {
                        time_unit: &unit,
                        numbers: &self.numbers,
                        drafts: &mut self.drafts,
                        calendars: &[],
                    };
                    form::field(
                        ui,
                        &mut cx,
                        Schema::new(&schema),
                        &format!("{key}/distribution"),
                        "distribution",
                        &input["distribution"],
                        false,
                    )
                };
                if let Some(d) = changed {
                    edits.push((key.clone(), json!({ "name": name, "distribution": d })));
                }
                self.mc_error(ui, &key);
            });
        }
        if inputs.is_empty() {
            ui.label(theme::weak(
                ui,
                t("No inputs yet: add one below, or paste a table of them."),
            ));
        }
        for (key, params) in edits {
            self.mc_edit(&key, "mc.input.set", params);
        }
        if let Some(name) = remove {
            self.mc_edit("mc/in", "mc.input.remove", json!({ "name": name }));
        }
        self.mc_error(ui, "mc/in");
        // A new input, named here; it starts as a three-point estimate to change.
        let mut add = false;
        ui.horizontal(|ui| {
            row_label(ui, t("New input"));
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.mc.new_input)
                    .desired_width(180.0)
                    .hint_text(t("a name, such as labour")),
            );
            form::field_outline(ui, &r);
            r.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TextInput, true, t("New input"))
            });
            let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            add = ui.add(theme::secondary(ui, t("Add input"))).clicked() || entered;
        });
        let name = self.mc.new_input.trim().to_owned();
        // A name that is taken is refused here: setting it would replace what has it.
        if add && !name.is_empty() && !self.mc_name_taken("mc/new_input", &name) {
            let params = json!({ "name": name, "distribution": first_distribution() });
            if self
                .mc_edit("mc/new_input", "mc.input.set", params)
                .is_some()
            {
                self.mc.new_input.clear();
            }
        }
        self.mc_error(ui, "mc/new_input");
        ui.add_space(6.0);
        self.mc_table(ui);
    }

    /// Inputs from a pasted table (a spreadsheet's cells, or CSV), and the inputs as a CSV file.
    fn mc_table(&mut self, ui: &mut egui::Ui) {
        let mut import = false;
        egui::CollapsingHeader::new(t("Paste inputs from a table"))
            .id_salt("mc/paste")
            .default_open(false)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(theme::weak(
                        ui,
                        t("One input per line, as name, distribution, parameters and unit, copied from a spreadsheet or written as CSV: for example labour,triangular,min=80;mode=100;max=150,h."),
                    ))
                    .wrap(),
                );
                let r = ui.add(
                    egui::TextEdit::multiline(&mut self.mc.paste)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace),
                );
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, true, t("Inputs as a table"))
                });
                ui.checkbox(&mut self.mc.replace, t("Replace every input"));
                import = ui
                    .add_enabled(
                        !self.mc.paste.trim().is_empty(),
                        theme::secondary(ui, t("Import inputs")),
                    )
                    .clicked();
                self.mc_error(ui, "mc/import");
            });
        if import {
            let params = json!({ "csv": table_text(&self.mc.paste), "replace": self.mc.replace });
            if let Some(r) = self.mc_edit("mc/import", "mc.inputs.import", params) {
                self.mc.paste.clear();
                self.info(tn(
                    r["inputs"].as_u64().unwrap_or(0),
                    "Imported {n} input",
                    "Imported {n} inputs",
                ));
            }
        }
        let export = if self.platform.file_system {
            t("Export inputs (CSV)")
        } else {
            t("Download inputs (CSV)")
        };
        if ui.add(theme::secondary(ui, export)).clicked()
            && let Some(r) = self.execute("mc.inputs.export", json!({}))
        {
            let name = format!("{}-inputs.csv", self.export_stem());
            let bytes = r["csv"].as_str().unwrap_or("").as_bytes().to_vec();
            self.requests.push(Request::Export { name, bytes });
        }
    }

    /// The rank correlations between inputs: each with its value, and a way to add one.
    fn mc_correlations(&mut self, ui: &mut egui::Ui, mc: &Value) {
        section(ui, t("Correlations"));
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("A rank correlation between two inputs, from -1 to 1: above 0 they tend to be high together, below 0 one tends to be high when the other is low. Inputs not paired here are independent; 0 removes a pair."),
            ))
            .wrap(),
        );
        ui.add_space(4.0);
        let pairs = mc["correlations"].as_array().cloned().unwrap_or_default();
        let mut set = None;
        for c in &pairs {
            let (a, b) = (
                c["a"].as_str().unwrap_or("").to_owned(),
                c["b"].as_str().unwrap_or("").to_owned(),
            );
            let before = c["rank"].as_f64().unwrap_or(0.0);
            let mut rank = before;
            let mut remove = false;
            ui.horizontal(|ui| {
                row_label(ui, &format!("{a} ↔ {b}"));
                number(
                    ui,
                    &self.numbers,
                    &format!("mc/corr/{a}/{b}"),
                    t("Rank correlation"),
                    &mut rank,
                    70.0,
                );
                remove = ui
                    .add(theme::secondary(ui, t("Remove")))
                    .on_hover_text(tf(
                        "Remove the correlation of {a} and {b}",
                        &[("a", &a), ("b", &b)],
                    ))
                    .clicked();
            });
            if remove {
                rank = 0.0;
            }
            if rank != before {
                set = Some(json!({ "a": a, "b": b, "rank": rank }));
            }
        }
        if let Some(params) = set {
            self.mc_edit("mc/corr", "mc.correlation.set", params);
        }
        let names: Vec<(String, String)> = mc["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| i["name"].as_str())
            .map(|n| (n.to_owned(), n.to_owned()))
            .collect();
        let mut add = false;
        ui.horizontal_wrapped(|ui| {
            row_label(ui, t("New correlation"));
            choose(
                ui,
                "mc/corr/a",
                t("First input"),
                &names,
                &mut self.mc.pair.0,
                140.0,
            );
            ui.label(theme::weak(ui, "↔"));
            choose(
                ui,
                "mc/corr/b",
                t("Second input"),
                &names,
                &mut self.mc.pair.1,
                140.0,
            );
            let ready = matches!(&self.mc.pair, (Some(a), Some(b)) if a != b);
            add = ui
                .add_enabled(ready, theme::secondary(ui, t("Add correlation")))
                .clicked();
        });
        if add && let (Some(a), Some(b)) = self.mc.pair.clone() {
            let params = json!({ "a": a, "b": b, "rank": 0.5 });
            if self
                .mc_edit("mc/corr", "mc.correlation.set", params)
                .is_some()
            {
                self.mc.pair = (None, None);
            }
        }
        self.mc_error(ui, "mc/corr");
    }

    /// The outputs: each with its expression, unit and target, and a way to add one.
    fn mc_outputs(&mut self, ui: &mut egui::Ui, p: &theme::Palette, mc: &Value) {
        section(ui, t("Outputs"));
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("Each output is worked out in every trial from the inputs and the outputs above it, each named as $name: for example $labour * $rate + $materials. Give it a target to see the chance of meeting it."),
            ))
            .wrap(),
        );
        ui.add_space(4.0);
        let outputs = mc["outputs"].as_array().cloned().unwrap_or_default();
        let mut edits: Vec<OutputEdit> = Vec::new();
        let mut remove = None;
        // What can be a control: the inputs whose means are exact, and the outputs above.
        let usable_inputs = self
            .engine
            .model()
            .monte_carlo
            .as_ref()
            .map(stochcraft_engine::mc::control_inputs)
            .unwrap_or_default();
        for (index, o) in outputs.iter().enumerate() {
            let name = o["name"].as_str().unwrap_or("").to_owned();
            let unit = o["unit"].as_str().unwrap_or("").to_owned();
            let key = format!("mc/out/{name}");
            let earlier: Vec<String> = outputs
                .iter()
                .take(index)
                .filter_map(|x| x["name"].as_str().map(str::to_owned))
                .collect();
            Self::mc_card(ui, p, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::strong(&name).size(15.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(theme::secondary(ui, t("Remove")))
                            .on_hover_text(tf("Remove the output {name}", &[("name", &name)]))
                            .clicked()
                        {
                            remove = Some(name.clone());
                        }
                    });
                });
                // The expression: a refused one stays in its field, with the reason under it.
                let field = format!("{key}/expression");
                let typed = ui
                    .horizontal(|ui| {
                        row_label(ui, t("Expression"));
                        let mut cx = form::Cx {
                            time_unit: "",
                            numbers: &self.numbers,
                            drafts: &mut self.drafts,
                            calendars: &[],
                        };
                        form::text(
                            ui,
                            &mut cx,
                            &field,
                            t("Expression"),
                            &o["expression"],
                            360.0,
                        )
                    })
                    .inner;
                if let Some(x) = typed {
                    let text = x.as_str().unwrap_or("").to_owned();
                    edits.push((
                        key.clone(),
                        json!({ "name": name, "expression": text }),
                        Some((field.clone(), text)),
                    ));
                }
                if let Some(u) = self.text_field(ui, &format!("{key}/unit"), t("Unit"), &unit, true)
                {
                    edits.push((key.clone(), json!({ "name": name, "unit": u }), None));
                }
                // The target: none, at most or at least a value.
                let (bound, value) = match (o["at_most"].as_f64(), o["at_least"].as_f64()) {
                    (Some(x), _) => (Bound::AtMost, x),
                    (None, Some(x)) => (Bound::AtLeast, x),
                    (None, None) => (Bound::None, 0.0),
                };
                let (mut b, mut v) = (bound, value);
                ui.horizontal(|ui| {
                    row_label(ui, t("Target"));
                    bound_choice(ui, &format!("{key}/bound"), t("Target"), &mut b, true);
                    if b != Bound::None {
                        number(
                            ui,
                            &self.numbers,
                            &format!("{key}/target"),
                            t("Target value"),
                            &mut v,
                            110.0,
                        );
                        if !unit.is_empty() {
                            ui.label(theme::weak(ui, &unit));
                        }
                    }
                });
                if (b, v) != (bound, value) {
                    let params = match b.key() {
                        Some(k) => json!({ "name": name, k: v }),
                        None => json!({ "name": name, "at_most": null }),
                    };
                    edits.push((key.clone(), params, None));
                }
                // How the results show it: an answer or a working step (folded away), and the
                // root mean square of a squared quantity beside its mean.
                let mut working = is_working(o);
                let mut rms = o["summary"] == "rms";
                let (was_working, was_rms) = (working, rms);
                ui.horizontal_wrapped(|ui| {
                    row_label(ui, t("In the results"));
                    ui.checkbox(&mut working, t("Working step"))
                        .on_hover_text(t("A step towards an answer: the results list it folded away under the answers."));
                    ui.checkbox(&mut rms, t("Root mean square"))
                        .on_hover_text(t("For a squared quantity, such as a squared error: the results give the square root of its mean, in the quantity's own unit."));
                });
                if working != was_working {
                    let role = if working { "working" } else { "key" };
                    edits.push((key.clone(), json!({ "name": name, "role": role }), None));
                }
                if rms != was_rms {
                    let summary = if rms { json!("rms") } else { Value::Null };
                    edits.push((
                        key.clone(),
                        json!({ "name": name, "summary": summary }),
                        None,
                    ));
                }
                // Control variates: inputs with a known mean, or earlier outputs whose mean the
                // modeller gives; the results use them to narrow this output's interval.
                let controls: Vec<Value> = o["controls"].as_array().cloned().unwrap_or_default();
                let mut new_controls = controls.clone();
                let mut changed = false;
                ui.horizontal_wrapped(|ui| {
                    row_label(ui, t("Controls"));
                    if controls.is_empty() {
                        ui.label(theme::weak(ui, t("None")));
                    }
                    let mut drop = None;
                    for (n, c) in controls.iter().enumerate() {
                        let (label, is_output) = match (c["input"].as_str(), c["output"].as_str()) {
                            (Some(x), _) => (x, false),
                            (None, Some(x)) => (x, true),
                            _ => ("", false),
                        };
                        ui.label(label);
                        if is_output {
                            let mut v = c["expected"].as_f64().unwrap_or(0.0);
                            let before = v;
                            number(
                                ui,
                                &self.numbers,
                                &format!("{key}/control/{n}"),
                                t("Expected mean"),
                                &mut v,
                                90.0,
                            );
                            if v != before
                                && let Some(slot) = new_controls.get_mut(n)
                            {
                                *slot = json!({ "output": label, "expected": v });
                                changed = true;
                            }
                        }
                        if ui
                            .add(theme::secondary(ui, t("Remove")))
                            .on_hover_text(tf("Remove the control {name}", &[("name", label)]))
                            .clicked()
                        {
                            drop = Some(n);
                        }
                    }
                    if let Some(n) = drop {
                        new_controls.remove(n);
                        changed = true;
                    }
                    let used: Vec<&str> = controls
                        .iter()
                        .filter_map(|c| c["input"].as_str().or_else(|| c["output"].as_str()))
                        .collect();
                    let menu = ui.menu_button(t("Add a control"), |ui| {
                        let mut any = false;
                        for x in usable_inputs.iter().filter(|x| !used.contains(&x.as_str())) {
                            any = true;
                            if ui.button(x).clicked() {
                                new_controls.push(json!({ "input": x }));
                                changed = true;
                                ui.close();
                            }
                        }
                        for x in earlier.iter().filter(|x| !used.contains(&x.as_str())) {
                            any = true;
                            let text = tf(
                                "{name} (an output; you give its mean)",
                                &[("name", x.as_str())],
                            );
                            if ui.button(text).clicked() {
                                new_controls.push(json!({ "output": x, "expected": 0.0 }));
                                changed = true;
                                ui.close();
                            }
                        }
                        if !any {
                            ui.label(theme::weak(
                                ui,
                                t("No input or earlier output can be a control."),
                            ));
                        }
                    });
                    menu.response.on_hover_text(t(
                        "A control is an input whose mean is known, or an earlier output whose mean you know. The results use how it moves with this output to narrow the interval of the output's mean and target chance.",
                    ));
                });
                if changed {
                    edits.push((
                        key.clone(),
                        json!({ "name": name, "controls": new_controls }),
                        None,
                    ));
                }
                self.mc_error(ui, &key);
            });
        }
        if outputs.is_empty() {
            ui.label(theme::weak(ui, t("No outputs yet: add one below.")));
        }
        for (key, params, typed) in edits {
            if self.mc_edit(&key, "mc.output.set", params).is_none()
                && let Some((field, text)) = typed
            {
                // Kept in its field, to be fixed rather than typed again.
                self.drafts.insert(field, text);
            }
        }
        if let Some(name) = remove {
            self.mc_edit("mc/out", "mc.output.remove", json!({ "name": name }));
        }
        self.mc_error(ui, "mc/out");
        let mut add = false;
        ui.horizontal(|ui| {
            row_label(ui, t("New output"));
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.mc.new_output)
                    .desired_width(180.0)
                    .hint_text(t("a name, such as cost")),
            );
            form::field_outline(ui, &r);
            r.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TextInput, true, t("New output"))
            });
            let entered = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            add = ui.add(theme::secondary(ui, t("Add output"))).clicked() || entered;
        });
        let name = self.mc.new_output.trim().to_owned();
        if add && !name.is_empty() && !self.mc_name_taken("mc/new_output", &name) {
            // A first expression to change: the first input, or nothing yet.
            let first = mc["inputs"][0]["name"]
                .as_str()
                .map_or_else(|| "0".to_owned(), |n| format!("${n}"));
            let params = json!({ "name": name, "expression": first });
            if self
                .mc_edit("mc/new_output", "mc.output.set", params)
                .is_some()
            {
                self.mc.new_output.clear();
            }
        }
        self.mc_error(ui, "mc/new_output");
    }

    /// The trials, seed and sampling, and a sequential stop.
    fn mc_settings(&mut self, ui: &mut egui::Ui, model: &Value) {
        section(ui, t("Trials and sampling"));
        // Which of the file's scenarios is open, when it has more than one.
        self.scenario_choice(ui, "model");
        let mc = &model["monte_carlo"];
        let x = &model["experiment"];
        let mut schema = params_schema("mc.set");
        schema["properties"]["trials"]["minimum"] = json!(1);
        let mut changed = None;
        for (f, current) in [
            ("trials", x["replications"].clone()),
            ("seed", x["seed"].clone()),
        ] {
            let mut cx = form::Cx {
                time_unit: "",
                numbers: &self.numbers,
                drafts: &mut self.drafts,
                calendars: &[],
            };
            if let Some(v) = form::field(
                ui,
                &mut cx,
                Schema::new(&schema),
                &format!("x/{f}"),
                f,
                &current,
                false,
            ) {
                changed = Some(json!({ f: v }));
            }
        }
        if let Some(params) = changed {
            self.mc_edit("mc/set", "mc.set", params);
        }
        // How the inputs are drawn, with what the chosen way does.
        let modes = [
            ("random", t("Random")),
            ("latin_hypercube", t("Latin hypercube")),
            ("antithetic", t("Antithetic pairs")),
            ("sobol", t("Quasi-random (Sobol)")),
        ];
        let current = mc["sampling"].as_str().unwrap_or("random");
        let mut picked = None;
        ui.horizontal(|ui| {
            row_label(ui, t("Sampling"));
            craft_ui::ComboBox::from_id_salt("mc/sampling")
                .selected_text(modes.iter().find(|m| m.0 == current).map_or("", |m| m.1))
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (name, label) in modes {
                        if ui.selectable_label(current == name, label).clicked() {
                            picked = Some(name);
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("Sampling"))
                });
        });
        if let Some(name) = picked.filter(|n| *n != current) {
            self.mc_edit("mc/set", "mc.set", json!({ "sampling": name }));
        }
        let help = match current {
            "antithetic" => t(
                "Antithetic pairs make each trial's twin from the opposite end of every input's range, so their errors tend to cancel, which usually makes the results more precise for the same trials. Trials run in pairs; it cannot be used with correlated inputs.",
            ),
            "sobol" => t(
                "Quasi-random (Sobol) draws spread the trials evenly over the inputs' ranges, with a fresh random shift in every batch of 1,000, which usually makes the results more precise for the same trials. It has points for up to 64 inputs.",
            ),
            _ => t(
                "Latin hypercube sampling cuts each input's range into equally likely strata and draws once from each, which usually makes the results more precise. With it, or with correlated inputs, trials run in whole batches of 1,000.",
            ),
        };
        ui.add(egui::Label::new(theme::weak(ui, help)).wrap());
        // A sequential stop, as for a process model's runs.
        let stop = &mc["stop"];
        let on = stop.is_object();
        let kinds = [
            (false, t("Exactly that many")),
            (true, t("More until precise enough")),
        ];
        let mut chosen = on;
        ui.horizontal(|ui| {
            row_label(ui, t("How many trials"));
            craft_ui::ComboBox::from_id_salt("mc/stop")
                .selected_text(kinds.iter().find(|k| k.0 == on).map_or("", |k| k.1))
                .width(240.0)
                .show_ui(ui, |ui| {
                    for (k, l) in kinds {
                        if ui.selectable_label(on == k, l).clicked() {
                            chosen = k;
                        }
                    }
                })
                .response
                .widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ComboBox, true, t("How many trials"))
                });
        });
        if chosen != on {
            let spec = if chosen { json!({}) } else { Value::Null };
            self.mc_edit("mc/set", "mc.set", json!({ "stop": spec }));
        } else if on {
            let mut relative = stop["relative"].as_f64().unwrap_or(0.01) * 100.0;
            let mut max = stop["max"].as_f64().unwrap_or(1_000_000.0);
            let before = (relative, max);
            for (salt, label, value) in [
                (
                    "mc/stop/relative",
                    t("Within ± % of the mean"),
                    &mut relative,
                ),
                ("mc/stop/max", t("At most, trials"), &mut max),
            ] {
                ui.horizontal(|ui| {
                    row_label(ui, label);
                    number(ui, &self.numbers, salt, label, value, 90.0);
                });
            }
            ui.add(
                egui::Label::new(theme::weak(
                    ui,
                    t("The trials above come first; more follow in batches until every output's mean is within that share of itself, or the most is reached."),
                ))
                .wrap(),
            );
            if (relative, max) != before {
                let spec = json!({
                    "relative": (relative / 100.0).clamp(0.0001, 0.99),
                    "max": max.round().clamp(1.0, f64::from(u32::MAX)) as u64,
                });
                self.mc_edit("mc/set", "mc.set", json!({ "stop": spec }));
            }
        }
        self.mc_error(ui, "mc/set");
    }

    /// The Results tab before a Monte Carlo model's first run.
    pub(crate) fn mc_ready(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.heading(t("No results yet"));
        ui.add_space(6.0);
        let x = &self.engine.model().experiment;
        ui.label(theme::weak(
            ui,
            tnf(
                u64::from(x.replications),
                "Makes {n} trial from seed {seed}: each draws every input once and works out every output.",
                "Makes {n} trials from seed {seed}: each draws every input once and works out every output.",
                &[("seed", &x.seed.to_string())],
            ),
        ));
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary(ui, t("Run"))).clicked() {
                self.run();
            }
            let key = self.shortcut(commands::RUN, ui.ctx()).unwrap_or_default();
            ui.label(theme::weak(ui, key));
        });
    }

    /// The Results tab of a Monte Carlo run: the findings first, then each output's numbers and
    /// charts, then how the run went.
    pub(crate) fn mc_results_view(&mut self, ui: &mut egui::Ui) {
        let Some(r) = self.results.clone() else {
            return;
        };
        let mut csv = false;
        let mut again = false;
        let mut shares_wanted = false;
        let done = self.current_shares().cloned();
        let shares = SharesView {
            done: done.as_ref(),
            busy: self.shares_progress(),
            cost: self.engine.sobol_evaluations(),
            correlated: self.engine.sobol_correlated(),
            fresh: !self.results_stale,
        };
        egui::ScrollArea::vertical()
            .id_salt("mc/results")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(860.0));
                ui.horizontal(|ui| {
                    ui.heading(t("Results"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let table = if self.platform.file_system {
                            t("Export table (CSV)")
                        } else {
                            t("Download table (CSV)")
                        };
                        csv = ui.add(theme::secondary(ui, table)).clicked();
                    });
                });
                ui.label(theme::weak(ui, record_line(&r)));
                for line in [rounding_line(&r), stop_line(&r)].into_iter().flatten() {
                    ui.add(egui::Label::new(theme::weak(ui, line)).wrap());
                }
                if r["stop"].is_object() {
                    ui.add(
                        egui::Label::new(theme::weak(
                            ui,
                            t("The number of trials was chosen from the results, so an interval may cover the true value less often than 95% of the time."),
                        ))
                        .wrap(),
                    );
                }
                ui.add_space(6.0);
                if self.results_stale {
                    again = self.out_of_date_line(ui);
                    ui.add_space(6.0);
                }
                let outputs = r["outputs"].as_array().cloned().unwrap_or_default();
                // The answers first; the working steps towards them folded away below.
                let (working, key): (Vec<&Value>, Vec<&Value>) =
                    outputs.iter().partition(|o| is_working(o));
                let steps = tf("Working steps ({n})", &[("n", &working.len().to_string())]);
                ui.label(theme::strong(t("What the run found")));
                ui.add_space(2.0);
                for o in &key {
                    show_finding(ui, o, &r["interval_basis"]);
                }
                if !working.is_empty() {
                    egui::CollapsingHeader::new(theme::strong(&steps))
                        .id_salt("mc/working-findings")
                        .default_open(false)
                        .show(ui, |ui| {
                            for o in &working {
                                show_finding(ui, o, &r["interval_basis"]);
                            }
                        });
                    ui.add_space(4.0);
                }
                for w in r["warnings"].as_array().into_iter().flatten() {
                    let text = crate::i18n::engine_message(w);
                    ui.add(egui::Label::new(theme::weak(ui, text)).wrap());
                }
                let correlated = r["correlations"].as_array().is_some_and(|c| !c.is_empty());
                for o in &key {
                    shares_wanted |=
                        output_section(ui, o, &r["interval_basis"], correlated, &shares);
                }
                if !working.is_empty() {
                    ui.add_space(8.0);
                    egui::CollapsingHeader::new(theme::strong(tf(
                        "Working steps in detail ({n})",
                        &[("n", &working.len().to_string())],
                    )))
                    .id_salt("mc/working-sections")
                    .default_open(false)
                    .show(ui, |ui| {
                        for o in &working {
                            shares_wanted |= output_section(
                                ui,
                                o,
                                &r["interval_basis"],
                                correlated,
                                &shares,
                            );
                        }
                    });
                }
                section(ui, t("How sure are these numbers?"));
                ui.add(egui::Label::new(basis_line(&r)).wrap());
                let correlations = r["correlations"].as_array().cloned().unwrap_or_default();
                if !correlations.is_empty() {
                    section(ui, t("Correlations"));
                    for c in &correlations {
                        let shown = |v: &Value| v.as_f64().map_or_else(|| "–".to_owned(), |x| grouped(round_sig(x, 2)));
                        ui.label(tf(
                            "{a} and {b}: {achieved} in the trials, {target} asked for",
                            &[
                                ("a", c["a"].as_str().unwrap_or("")),
                                ("b", c["b"].as_str().unwrap_or("")),
                                ("achieved", &shown(&c["achieved"])),
                                ("target", &shown(&c["target"])),
                            ],
                        ));
                    }
                }
            });
        if again {
            self.run();
        }
        if shares_wanted {
            self.mc_shares_start();
        }
        if csv {
            self.export_csv();
        }
    }
}

/// One output's numbers and charts. `correlated`: the run's model has correlated inputs. True
/// when the button for the shares of the spread was clicked.
fn output_section(
    ui: &mut egui::Ui,
    o: &Value,
    basis: &Value,
    correlated: bool,
    shares: &SharesView<'_>,
) -> bool {
    let name = o["name"].as_str().unwrap_or("");
    let unit = o["unit"].as_str().unwrap_or("");
    let title = if unit.is_empty() {
        name.to_owned()
    } else {
        format!("{name} ({unit})")
    };
    section(ui, &title);
    let mean = match (o["mean"].as_f64(), o["half_width"].as_f64()) {
        (Some(m), Some(h)) => {
            let (m, h) = rounded(m, h);
            format!("{m} ± {h}")
        }
        (Some(m), None) => value_text(m),
        _ => "–".to_owned(),
    };
    let shown = |v: &Value| v.as_f64().map_or_else(|| "–".to_owned(), value_text);
    let mut rows = vec![
        (t("Mean").to_owned(), mean),
        (form::label("std_dev"), shown(&o["std_dev"])),
        (form::label("min"), shown(&o["min"])),
        (form::label("max"), shown(&o["max"])),
    ];
    let estimate = &o["controlled"]["estimate"];
    if let Some(m) = estimate["mean"].as_f64() {
        let text = match estimate["half_width"].as_f64() {
            Some(h) => {
                let (m, h) = rounded(m, h);
                format!("{m} ± {h}")
            }
            None => value_text(m),
        };
        rows.insert(1, (t("Controlled mean").to_owned(), text));
    }
    grid(ui, &format!("mc/result/{name}"), 2, |ui| {
        for (label, value) in rows {
            ui.label(theme::weak(ui, label));
            ui.label(with_unit(value, unit));
            ui.end_row();
        }
    });
    ui.add_space(4.0);
    let mut ps: Vec<(f64, f64)> = o["percentiles"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.parse::<f64>().ok()?, v.as_f64()?)))
        .collect();
    ps.sort_by(|a, b| a.0.total_cmp(&b.0));
    grid(ui, &format!("mc/percentiles/{name}"), ps.len() + 1, |ui| {
        ui.label(theme::weak(ui, t("Percentile")));
        for (q, _) in &ps {
            ui.label(theme::weak(ui, grouped(*q)));
        }
        ui.end_row();
        ui.label(theme::weak(ui, t("Value")));
        for (_, v) in &ps {
            ui.label(value_text(*v));
        }
        ui.end_row();
    });
    if let Some(n) = o["invalid"].as_u64().filter(|n| *n > 0) {
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                tn(
                    n,
                    "{n} trial gave no number (a division by zero, say) and is left out.",
                    "{n} trials gave no number (a division by zero, say) and are left out.",
                ),
            ))
            .wrap(),
        );
    }
    // The interval is across fewer values than the run has: say how many.
    if let (Some(n), Some(of)) = (o["interval_count"].as_u64(), basis["count"].as_u64())
        && n != of
        && o["half_width"].is_number()
    {
        let (n, of) = (grouped(n as f64), grouped(of as f64));
        let args = [("name", name), ("n", n.as_str()), ("of", of.as_str())];
        let text = if basis["across"] == "pairs" {
            tf(
                "{name}: the ± is across {n} of the {of} pairs, the rest had no trial with a number.",
                &args,
            )
        } else if basis["across"] == "batches" {
            tf(
                "{name}: the ± is across {n} of the {of} batches, the rest had no trial with a number.",
                &args,
            )
        } else {
            tf(
                "{name}: the ± is across {n} of the {of} trials, the rest gave no number.",
                &args,
            )
        };
        ui.add(egui::Label::new(theme::weak(ui, text)).wrap());
    }
    ui.add_space(6.0);
    let target = o["target"]["at_most"]
        .as_f64()
        .or_else(|| o["target"]["at_least"].as_f64());
    let wide = ui.available_width() >= 700.0;
    let width = if wide {
        (ui.available_width() - 24.0) / 2.0
    } else {
        ui.available_width().min(520.0)
    };
    let charts = |ui: &mut egui::Ui| {
        ui.vertical(|ui| {
            ui.set_width(width);
            histogram_chart(ui, o, target, unit, width);
        });
        ui.vertical(|ui| {
            ui.set_width(width);
            cdf_chart(ui, o, target, unit, width);
        });
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 24.0;
            charts(ui);
        });
    } else {
        charts(ui);
    }
    influence_section(ui, o, correlated, shares)
}

/// The most inputs the tornado and the table of rank correlations show.
const MOST_INPUTS: usize = 12;

/// Which inputs an output depends on: the answer in words first, then the tornado (each input
/// moved from its 10th to its 90th percentile with the others at their medians, the swing of
/// decision analysis in Clemen and Reilly, *Making Hard Decisions*), then each input's rank
/// correlation with the output. The table gains each input's share of the output's spread, alone
/// and with its interactions, once they are worked out (`shares`); until then a button works them
/// out, or the reason there are none is told. `correlated`: the run's model has correlated
/// inputs. True when the button was clicked.
fn influence_section(
    ui: &mut egui::Ui,
    o: &Value,
    correlated: bool,
    shares: &SharesView<'_>,
) -> bool {
    let name = o["name"].as_str().unwrap_or("");
    let unit = o["unit"].as_str().unwrap_or("");
    // Every input the run has, widest swing first; the tornado and the rank table show the first
    // `MOST_INPUTS`, the table of shares all of them.
    let all: Vec<&Value> = o["influence"]["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let inputs: Vec<&Value> = all.iter().take(MOST_INPUTS).copied().collect();
    let Some(top) = inputs.first() else {
        return false;
    };
    let mine = shares_of(shares.done, name);
    let have = mine.is_some_and(|m| m["unavailable"].is_null());
    ui.add_space(10.0);
    ui.label(theme::strong(t("Which inputs matter")).size(13.0));
    if let (Some(input), Some(low), Some(high), Some(swing)) = (
        top["input"].as_str(),
        top["low"].as_f64(),
        top["high"].as_f64(),
        top["swing"].as_f64(),
    ) && swing > 0.0
    {
        ui.add(
            egui::Label::new(tf(
                "{output} changes most with {input}: it goes from {low} to {high} as {input} goes from its 10th to its 90th percentile, the other inputs at their medians.",
                &[
                    ("output", name),
                    ("input", input),
                    ("low", &with_unit(value_text(low), unit)),
                    ("high", &with_unit(value_text(high), unit)),
                ],
            ))
            .wrap(),
        );
    }
    // An input the output goes up and down along: its two endpoints understate its effect.
    let bent: Vec<&str> = all
        .iter()
        .filter(|i| i["not_monotone"] == true)
        .filter_map(|i| i["input"].as_str())
        .collect();
    if !bent.is_empty() {
        ui.add(
            egui::Label::new(tf(
                "The output does not move in one direction with {inputs} (its value at the median is outside its values at the 10th and 90th percentiles), so the swing understates the effect.",
                &[("inputs", &bent.join(", "))],
            ))
            .wrap(),
        );
    }
    // The input that explains most of the spread by itself, in words.
    if have
        && let Some((input, x)) = mine
            .and_then(|m| m["inputs"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|i| Some((i["input"].as_str()?, i["first"]["value"].as_f64()?)))
            .filter(|(_, x)| x.is_finite())
            .max_by(|a, b| a.1.total_cmp(&b.1))
        && x > 0.0
    {
        ui.add(
            egui::Label::new(tf(
                "{input} alone explains about {share} of the spread in {output}.",
                &[
                    ("input", input),
                    ("share", &share_text(x)),
                    ("output", name),
                ],
            ))
            .wrap(),
        );
    }
    // An input that does not move the output has no bar (it is still in the table below).
    let bars: Vec<(String, f64, f64)> = inputs
        .iter()
        .filter(|i| i["swing"].as_f64().is_some_and(|s| s > 0.0))
        .filter_map(|i| {
            Some((
                i["input"].as_str()?.to_owned(),
                i["low"].as_f64()?,
                i["high"].as_f64()?,
            ))
        })
        .collect();
    if let Some(base) = o["influence"]["base"].as_f64() {
        let width = ui.available_width().min(560.0);
        tornado_chart(ui, &bars, base, unit, width);
    }
    ui.add_space(4.0);
    let rank_of = |input: &str| {
        all.iter()
            .find(|i| i["input"] == input)
            .and_then(|i| i["rank_correlation"].as_f64())
            .map_or_else(|| "–".to_owned(), rank_text)
    };
    // With shares, a row for every input they are for (not only the tornado's), the rank
    // correlation joined by name; without, the tornado's.
    let rows: Vec<(String, String, Option<&Value>)> = if have {
        // In the tornado's order (widest swing first), so the rows do not move when they arrive.
        all.iter()
            .map(|i| {
                let input = i["input"].as_str().unwrap_or("");
                let indices = mine
                    .and_then(|m| m["inputs"].as_array())
                    .and_then(|is| is.iter().find(|s| s["input"] == input));
                (input.to_owned(), rank_of(input), indices)
            })
            .collect()
    } else {
        inputs
            .iter()
            .map(|i| {
                let input = i["input"].as_str().unwrap_or("");
                (input.to_owned(), rank_of(input), None)
            })
            .collect()
    };
    grid(
        ui,
        &format!("mc/rank/{name}"),
        if have { 4 } else { 2 },
        |ui| {
            ui.label(theme::weak(ui, t("Inputs")));
            ui.label(theme::weak(ui, t("Rank correlation")));
            if have {
                ui.label(theme::weak(ui, t("Alone")));
                ui.label(theme::weak(ui, t("With interactions")));
            }
            ui.end_row();
            for (input, rho, indices) in rows {
                ui.label(input);
                ui.label(rho);
                if have {
                    for key in ["first", "total"] {
                        let estimate = indices.map(|i| &i[key]);
                        let text = estimate
                            .and_then(|e| e["value"].as_f64())
                            .map_or_else(|| "–".to_owned(), share_text);
                        let cell = ui.label(text);
                        if let Some(interval) = estimate.and_then(share_interval) {
                            cell.on_hover_text(interval);
                        }
                    }
                }
                ui.end_row();
            }
        },
    );
    let mut wanted = false;
    if have {
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("Alone is the share of the spread an input explains by itself. With interactions adds what it does together with the other inputs."),
            ))
            .wrap(),
        );
    } else if let Some(why) = mine.and_then(|m| m["unavailable"].as_str()) {
        ui.add(egui::Label::new(theme::weak(ui, why)).wrap());
    } else if shares.correlated {
        ui.add(
            egui::Label::new(theme::weak(
                ui,
                t("The shares of the spread need independent inputs, and some of these inputs are correlated."),
            ))
            .wrap(),
        );
    } else if let Some(progress) = shares.busy {
        ui.label(theme::weak(
            ui,
            tf(
                "Working out the shares of the spread: {percent}",
                &[("percent", &share_text(progress))],
            ),
        ));
    } else if shares.fresh
        && let Some(n) = shares.cost
    {
        ui.horizontal(|ui| {
            wanted = ui
                .add(theme::secondary(
                    ui,
                    t("Work out each input's share of the spread"),
                ))
                .clicked();
            ui.label(theme::weak(
                ui,
                tf("{n} evaluations of the model", &[("n", &grouped(n as f64))]),
            ));
        });
    }
    if correlated {
        for note in [
            t(
                "The bars move one input at a time, so they leave out what correlated inputs do together.",
            ),
            t(
                "A correlated input's figure includes the effect of the inputs it is correlated with.",
            ),
        ] {
            ui.add(egui::Label::new(theme::weak(ui, note)).wrap());
        }
    }
    wanted
}

/// A rank correlation to two decimals, as correlations are usually given: 0.90, −0.42, and 0.00
/// for one too small to show (never −0.00 or an exponent).
fn rank_text(x: f64) -> String {
    let r = (x * 100.0).round() / 100.0;
    if r.abs() < 0.005 {
        "0.00".to_owned()
    } else {
        format!("{r:.2}")
    }
}

/// The tornado: a row for each input (name, bar, the output's value at each end), the widest
/// swing first, the bars drawn around a line at `base`, the output with every input at its
/// median. The words and numbers are in the accessibility tree as one description.
fn tornado_chart(
    ui: &mut egui::Ui,
    bars: &[(String, f64, f64)],
    base: f64,
    unit: &str,
    width: f32,
) {
    let ends: Vec<f64> = bars.iter().flat_map(|b| [b.1, b.2]).chain([base]).collect();
    let lo = ends.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = ends.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if bars.is_empty() || !(lo.is_finite() && hi.is_finite() && (hi - lo).is_finite()) {
        return;
    }
    let span = if hi > lo { hi - lo } else { lo.abs().max(1.0) };
    let caption =
        t("Each input from its 10th to its 90th percentile, with the others at their medians");
    let base_text = tf(
        "Base {value}",
        &[("value", &with_unit(value_text(base), unit))],
    );
    let described = bars
        .iter()
        .map(|(n, l, h)| format!("{n}: {} – {}", value_text(*l), value_text(*h)))
        .collect::<Vec<_>>()
        .join("; ");
    ui.add_space(4.0);
    ui.label(theme::strong(caption).size(13.0));
    let p = theme::palette(ui.ctx());
    let font = FontId::proportional(11.0);
    let name_width = bars
        .iter()
        .map(|b| {
            ui.painter()
                .layout_no_wrap(b.0.clone(), font.clone(), p.text)
                .size()
                .x
        })
        .fold(0.0_f32, f32::max)
        .min(width * 0.3);
    // Room at each end of the bars for the values.
    let margin = 56.0;
    let row = 24.0;
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, 20.0 + row * bars.len() as f32),
        egui::Sense::hover(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Group,
            true,
            format!("{caption}. {base_text}. {described}"),
        )
    });
    let plot = Rect::from_min_max(
        pos2(rect.left() + name_width + 8.0 + margin, rect.top() + 20.0),
        pos2(rect.right() - margin, rect.bottom()),
    );
    if plot.width() < 40.0 {
        return;
    }
    let painter = ui.painter_at(rect.expand(2.0));
    let x = |v: f64| plot.left() + ((v - lo) / span) as f32 * plot.width();
    painter.line_segment(
        [
            pos2(x(base), plot.top() - 2.0),
            pos2(x(base), plot.bottom()),
        ],
        Stroke::new(1.5, p.text_weak),
    );
    painter.text(
        pos2(x(base), rect.top()),
        Align2::CENTER_TOP,
        base_text,
        FontId::proportional(10.0),
        p.text_weak,
    );
    for (i, (name, low, high)) in bars.iter().enumerate() {
        let top = plot.top() + row * i as f32;
        let middle = top + row / 2.0;
        let (a, b) = (low.min(*high), low.max(*high));
        let bar = Rect::from_min_max(
            pos2(x(a), middle - 7.0),
            pos2(x(b).max(x(a) + 1.0), middle + 7.0),
        );
        painter.rect_filled(bar, 2.0, p.chart);
        painter
            .with_clip_rect(Rect::from_min_max(
                pos2(rect.left(), top),
                pos2(rect.left() + name_width + 4.0, top + row),
            ))
            .text(
                pos2(rect.left(), middle),
                Align2::LEFT_CENTER,
                name,
                font.clone(),
                p.text,
            );
        painter.text(
            pos2(bar.left() - 4.0, middle),
            Align2::RIGHT_CENTER,
            value_text(a),
            font.clone(),
            p.text,
        );
        painter.text(
            pos2(bar.right() + 4.0, middle),
            Align2::LEFT_CENTER,
            value_text(b),
            font.clone(),
            p.text,
        );
    }
}

/// The x range two charts of an output share: its values and its target, with a margin.
fn x_range(o: &Value, target: Option<f64>) -> Option<(f64, f64)> {
    let lo = o["min"].as_f64()?;
    let hi = o["max"].as_f64()?;
    let (lo, hi) = match target {
        Some(x) => (lo.min(x), hi.max(x)),
        None => (lo, hi),
    };
    let span = if hi > lo { hi - lo } else { lo.abs().max(1.0) };
    let (lo, hi) = (lo - span * 0.03, hi + span * 0.03);
    // Values so far apart that their span overflows cannot be drawn on one axis.
    (lo.is_finite() && hi.is_finite() && (hi - lo).is_finite() && hi > lo).then_some((lo, hi))
}

/// Round values along the x axis, labelled under the plot, and the target's line.
fn x_axis(
    painter: &egui::Painter,
    plot: Rect,
    (lo, hi): (f64, f64),
    target: Option<f64>,
    unit: &str,
) {
    let p = theme::palette(painter.ctx());
    let x = |v: f64| plot.left() + ((v - lo) / (hi - lo)) as f32 * plot.width();
    let step = nice_step(hi - lo, 4.0);
    let mut v = (lo / step).ceil() * step;
    let grid = Stroke::new(1.0, p.border);
    let mut ticks = 0;
    while v <= hi && ticks < 12 {
        painter.line_segment([pos2(x(v), plot.top()), pos2(x(v), plot.bottom())], grid);
        painter.text(
            pos2(x(v), plot.bottom() + 3.0),
            Align2::CENTER_TOP,
            grouped(v),
            FontId::proportional(10.0),
            p.text_weak,
        );
        v += step;
        ticks += 1;
    }
    if let Some(tx) = target {
        let at = x(tx);
        painter.line_segment(
            [pos2(at, plot.top()), pos2(at, plot.bottom())],
            Stroke::new(2.0, p.warning),
        );
        painter.text(
            pos2(at + 3.0, plot.top()),
            Align2::LEFT_TOP,
            tf(
                "Target {value}",
                &[("value", &with_unit(value_text(tx), unit))],
            ),
            FontId::proportional(10.0),
            p.warning,
        );
    }
}

/// How many trials gave each value: the output's histogram, with its target.
fn histogram_chart(ui: &mut egui::Ui, o: &Value, target: Option<f64>, unit: &str, width: f32) {
    let edges: Vec<f64> = o["histogram"]["edges"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect();
    let counts: Vec<u64> = o["histogram"]["counts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect();
    let Some(range) = x_range(o, target) else {
        return;
    };
    if counts.is_empty() || edges.len() != counts.len() + 1 {
        return;
    }
    let caption = t("How many trials gave each value");
    ui.label(theme::strong(caption).size(13.0));
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(width, 160.0), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, caption));
    let plot = Rect::from_min_max(
        pos2(rect.left() + 4.0, rect.top() + 14.0),
        pos2(rect.right() - 4.0, rect.bottom() - 18.0),
    );
    let painter = ui.painter_at(rect.expand(2.0));
    let (lo, hi) = range;
    let x = |v: f64| plot.left() + ((v - lo) / (hi - lo)) as f32 * plot.width();
    let most = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
    for (i, c) in counts.iter().enumerate() {
        let (Some(a), Some(b)) = (edges.get(i), edges.get(i + 1)) else {
            continue;
        };
        let top = plot.bottom() - *c as f32 / most * plot.height();
        let bar = Rect::from_min_max(
            pos2(x(*a), top),
            pos2(x(*b).max(x(*a) + 1.0), plot.bottom()),
        );
        painter.rect_filled(bar, 0.0, p.accent.gamma_multiply(0.75));
    }
    painter.line_segment(
        [
            pos2(plot.left(), plot.bottom()),
            pos2(plot.right(), plot.bottom()),
        ],
        Stroke::new(1.0, p.text_weak),
    );
    x_axis(&painter, plot, range, target, unit);
}

/// The share of trials at or below each value: the output's cumulative curve, with its target.
fn cdf_chart(ui: &mut egui::Ui, o: &Value, target: Option<f64>, unit: &str, width: f32) {
    let points: Vec<(f64, f64)> = o["cdf"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p[0].as_f64()?, p[1].as_f64()?)))
        .collect();
    let Some(range) = x_range(o, target) else {
        return;
    };
    if points.is_empty() {
        return;
    }
    let caption = t("Share of trials at or below each value");
    ui.label(theme::strong(caption).size(13.0));
    let p = theme::palette(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(width, 160.0), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Group, true, caption));
    let plot = Rect::from_min_max(
        pos2(rect.left() + 36.0, rect.top() + 14.0),
        pos2(rect.right() - 4.0, rect.bottom() - 18.0),
    );
    let painter = ui.painter_at(rect.expand(2.0));
    let (lo, hi) = range;
    let x = |v: f64| plot.left() + ((v - lo) / (hi - lo)) as f32 * plot.width();
    let y = |f: f64| plot.bottom() - f.clamp(0.0, 1.0) as f32 * plot.height();
    for f in [0.0, 0.5, 1.0] {
        painter.line_segment(
            [pos2(plot.left(), y(f)), pos2(plot.right(), y(f))],
            Stroke::new(1.0, p.border),
        );
        painter.text(
            pos2(plot.left() - 4.0, y(f)),
            Align2::RIGHT_CENTER,
            percent(f),
            FontId::proportional(10.0),
            p.text_weak,
        );
    }
    x_axis(&painter, plot, range, target, unit);
    let mut line: Vec<egui::Pos2> = vec![pos2(plot.left(), y(0.0))];
    line.extend(points.iter().map(|(v, f)| pos2(x(*v), y(*f))));
    line.push(pos2(plot.right(), y(1.0)));
    painter.add(egui::Shape::line(line, Stroke::new(2.0, p.accent)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rank_correlation_has_two_decimals_and_no_exponent() {
        assert_eq!(rank_text(0.9012), "0.90");
        assert_eq!(rank_text(-0.416), "-0.42");
        assert_eq!(rank_text(1.4e-5), "0.00");
        assert_eq!(rank_text(-0.004), "0.00");
        assert_eq!(rank_text(1.0), "1.00");
    }

    fn output(target: Value, probability: f64, interval: Value) -> Value {
        let mut t = target;
        t["probability"] = json!(probability);
        t["interval"] = interval;
        t["of"] = json!(10_000);
        json!({
            "name": "cost", "unit": "USD", "mean": 6_500.0, "half_width": 12.3,
            "percentiles": { "5": 5_000.0, "95": 8_000.0 },
            "target": t,
        })
    }

    /// A target missed one time in five reads as odds, with the percentage and its interval
    /// turned round from the share that met it.
    #[test]
    fn a_target_reads_as_the_chance_of_missing_it() {
        let o = output(json!({ "at_most": 7_000.0 }), 0.8, json!([0.78, 0.82]));
        let (sentence, sure) = finding(&o).unwrap_or_default();
        // Numbers as written with no region set (four-digit numbers are not grouped).
        assert_eq!(
            sentence,
            "About a 1 in 5 chance that cost is more than 7000 USD (20%)."
        );
        assert_eq!(
            sure.as_deref(),
            Some("95% interval of the chance: 18% to 22%.")
        );
        // At least: the chance of falling short; a large chance as a percentage.
        let o = output(json!({ "at_least": 6_000.0 }), 0.4, Value::Null);
        let (sentence, sure) = finding(&o).unwrap_or_default();
        assert_eq!(sentence, "A 60% chance that cost is less than 6000 USD.");
        assert_eq!(
            sure.as_deref(),
            Some("No interval: too few batches of trials for one.")
        );
        // Never missed: the bound of the chance, which is small but not 0%.
        let o = output(json!({ "at_most": 9_000.0 }), 1.0, json!([0.9996, 1.0]));
        let (sentence, sure) = finding(&o).unwrap_or_default();
        assert_eq!(
            sentence,
            "cost was more than 9000 USD in none of the 10000 trials."
        );
        assert_eq!(
            sure.as_deref(),
            Some("The chance is under 0.04%, with 95% confidence.")
        );
    }

    /// A squared error summarised by its root mean square reads as that, in the error's own
    /// unit, with the half-width by the delta method; the mean of the squares is the line
    /// beneath. 38.0689 m² ± 0.8 is √38.0689 = 6.17 m ± 0.8 / (2 × 6.17) = 0.065 m, shown to its
    /// first figure as 0.06.
    #[test]
    fn a_squared_error_reads_as_its_root_mean_square() {
        let o = json!({
            "name": "position_error_1", "unit": "m²", "mean": 38.0689, "half_width": 0.8,
            "percentiles": { "5": 2.0, "95": 110.0 },
            "summary": { "kind": "rms", "value": 6.17, "half_width": 0.065, "unit": "m" },
        });
        let (sentence, mean) = finding(&o).unwrap_or_default();
        assert_eq!(
            sentence,
            "The root mean square of position_error_1 is 6.17 m ± 0.06."
        );
        assert_eq!(
            mean.as_deref(),
            Some(
                "In 9 trials out of 10, position_error_1 is between 2 m² and 110 m²; the mean is 38.1 m² ± 0.8."
            )
        );
        // With no interval, the value alone.
        let mut o = o;
        o["half_width"] = Value::Null;
        let (sentence, _) = finding(&o).unwrap_or_default();
        assert_eq!(
            sentence,
            "The root mean square of position_error_1 is 6.17 m."
        );
        // Only a working step is folded away.
        assert!(is_working(&json!({ "role": "working" })));
        assert!(!is_working(&json!({ "role": "key" })));
        assert!(!is_working(&json!({})));
    }

    /// Without a target: where 9 trials in 10 fall, and the mean rounded to its interval.
    #[test]
    fn an_output_without_a_target_reads_as_its_range() {
        let o = json!({
            "name": "days", "unit": "", "mean": 41.37, "half_width": 0.123,
            "percentiles": { "5": 35.2, "95": 48.9 },
        });
        let (sentence, sure) = finding(&o).unwrap_or_default();
        assert_eq!(
            sentence,
            "In 9 trials out of 10, days is between 35.2 and 48.9; the mean is 41.37 ± 0.12."
        );
        assert!(sure.is_none());
        assert!(finding(&json!({ "name": "x" })).is_none());
    }

    #[test]
    fn odds_only_where_they_read_naturally() {
        assert_eq!(odds(0.2), Some(5));
        assert_eq!(odds(0.01), Some(100));
        assert_eq!(odds(0.4), None);
        assert_eq!(odds(0.0001), None);
    }

    /// Cells copied from a spreadsheet, with or without the header, become the CSV the engine
    /// reads.
    #[test]
    fn a_pasted_table_becomes_csv() {
        assert_eq!(
            table_text(
                "labour\ttriangular\tmin=80;mode=100;max=150\th\n\nrate\tuniform\tmin=40;max=50\tUSD/h"
            ),
            "name,distribution,parameters,unit\nlabour,triangular,min=80;mode=100;max=150,h\nrate,uniform,min=40;max=50,USD/h\n"
        );
        let csv = "name,distribution,parameters,unit\nx,normal,mean=1;std_dev=2,\n";
        assert_eq!(table_text(csv), csv);
    }

    /// A header with its names in quotes is still a header.
    #[test]
    fn a_quoted_header_is_kept_as_the_header() {
        let csv = "\"name\",\"distribution\",\"parameters\",\"unit\"\nx,normal,mean=1;std_dev=2,\n";
        assert_eq!(table_text(csv), csv);
    }

    /// Values so far apart that the axis would overflow are not drawn, and the chart's step
    /// search ends.
    #[test]
    fn an_axis_that_cannot_be_drawn_has_no_range() {
        let wide = json!({ "min": -1e308, "max": 1e308 });
        assert_eq!(x_range(&wide, None), None);
        assert_eq!(
            x_range(&json!({ "min": 1.0, "max": 1e308 }), Some(-1e308)),
            None
        );
        let (lo, hi) = x_range(&json!({ "min": 10.0, "max": 20.0 }), None).unwrap_or_default();
        assert!(lo < 10.0 && hi > 20.0);
        assert!(nice_step(f64::INFINITY, 4.0) > 0.0);
    }

    /// With an interval across pairs the same two reasons are told in pairs.
    #[test]
    fn the_reason_for_no_interval_names_what_it_is_across() {
        let pairs = json!({ "across": "pairs", "count": 4, "size": 2 });
        let batches = json!({ "across": "batches", "count": 4, "size": 1000 });
        let mut o = output(json!({ "at_most": 7_000.0 }), 1.0, Value::Null);
        for (basis, same, few) in [
            (
                &pairs,
                "No interval: every pair gave the same share, so their spread gives none.",
                "No interval: too few pairs of trials for one.",
            ),
            (
                &batches,
                "No interval: every batch gave the same share, so their spread gives none.",
                "No interval: too few batches of trials for one.",
            ),
        ] {
            o["interval_count"] = json!(4);
            let (_, sure) = finding_across(&o, basis).unwrap_or_default();
            assert_eq!(sure.as_deref(), Some(same));
            o["interval_count"] = json!(1);
            let (_, sure) = finding_across(&o, basis).unwrap_or_default();
            assert_eq!(sure.as_deref(), Some(few));
        }
    }

    /// When every batch gave the same share the interval is withheld for that reason, not
    /// "too few batches"; with one batch it is the few batches.
    #[test]
    fn the_reason_for_no_interval_is_told() {
        let mut o = output(json!({ "at_most": 7_000.0 }), 1.0, Value::Null);
        o["interval_count"] = json!(4);
        let (_, sure) = finding(&o).unwrap_or_default();
        assert_eq!(
            sure.as_deref(),
            Some("No interval: every batch gave the same share, so their spread gives none.")
        );
        o["interval_count"] = json!(1);
        let (_, sure) = finding(&o).unwrap_or_default();
        assert_eq!(
            sure.as_deref(),
            Some("No interval: too few batches of trials for one.")
        );
    }

    /// Where the controls narrowed the interval, their mean and target chance are the headline;
    /// where they did not, the plain ones stay, and the lines say what happened.
    #[test]
    fn controls_are_the_headline_only_where_they_helped() {
        let mut o = json!({
            "name": "cost", "unit": "", "mean": 100.0, "half_width": 2.0,
            "percentiles": { "5": 90.0, "95": 110.0 },
            "controlled": {
                "controls": [{ "name": "labour", "kind": "input", "mean": 5.0,
                               "coefficient": 2.0, "sample_mean": 5.0, "mean_mismatch": false }],
                "pilot_trials": 1000, "correlation": 0.9, "unavailable": null,
                "estimate": { "mean": 101.0, "half_width": 0.7, "narrowing": 0.65,
                              "helped": true, "target": null },
            },
        });
        let shown = controlled_headline(&o);
        assert_eq!(shown["mean"], 101.0);
        assert_eq!(shown["half_width"], 0.7);
        let lines = control_lines(&o);
        assert_eq!(lines[0], "Controls for cost: labour.");
        assert!(
            lines[1].contains("narrowed the interval by 65%"),
            "{lines:?}"
        );
        o["controlled"]["estimate"]["helped"] = json!(false);
        o["controlled"]["estimate"]["narrowing"] = json!(0.02);
        o["controlled"]["correlation"] = json!(0.08);
        assert_eq!(controlled_headline(&o)["mean"], 100.0);
        let lines = control_lines(&o);
        assert!(
            lines[1].contains("did not help: their correlation with cost is weak (0.08)"),
            "{lines:?}"
        );
        o["controlled"]["estimate"]["narrowing"] = json!(-0.25);
        assert!(control_lines(&o)[1].contains("25% wider"));
        o["controlled"]["controls"][0]["mean_mismatch"] = json!(true);
        assert!(control_lines(&o).iter().any(|l| l.contains("biased")));
        o["controlled"]["estimate"] = Value::Null;
        o["controlled"]["unavailable"] = json!("too short");
        assert_eq!(control_lines(&o)[1], "No controlled result: too short");
    }

    /// A controlled chance is an estimate: it is never worded as a count of trials, is kept
    /// within 0 and 1, and an interval with no width is no narrowing, so the plain result stands
    /// and the lines say why.
    #[test]
    fn a_controlled_chance_is_an_estimate_and_a_flat_interval_is_no_narrowing() {
        let controlled = |t: Value| {
            let mut o = output(json!({ "at_most": 7_000.0 }), 0.5, json!([0.48, 0.52]));
            o["controlled"] = json!({
                "controls": [{ "name": "c", "kind": "output", "mean": 1.0,
                               "coefficient": 1.0, "sample_mean": 0.5, "mean_mismatch": false }],
                "pilot_trials": 1000, "correlation": 0.9, "unavailable": null,
                "estimate": { "mean": 1.0, "half_width": 0.1, "narrowing": 0.0,
                              "helped": false, "target": t },
            });
            o
        };
        // A chance of 1 that helped: worded as an estimate, not as "in none of the trials".
        let o = controlled(
            json!({ "probability": 1.0000001, "interval": [0.99, 1.0], "narrowing": 0.4,
                    "helped": true, "uncontrolled_interval": [0.48, 0.52] }),
        );
        let shown = controlled_headline(&o);
        assert_eq!(shown["target"]["probability"], 1.0);
        let (sentence, _) = finding(&shown).unwrap_or_default();
        assert_eq!(
            sentence,
            "The estimated chance that cost is more than 7000 USD is 0%."
        );
        assert!(!sentence.contains("none of the"), "{sentence}");
        // The plain result still counts trials.
        let (plain, _) = finding(&o).unwrap_or_default();
        assert!(plain.contains("A 50% chance"), "{plain}");
        let mut at_least = shown.clone();
        at_least["target"] = json!({ "at_least": 7_000.0, "probability": 1.0, "controlled": true,
                                      "interval": [0.99, 1.0], "of": 10_000 });
        let (sentence, _) = finding(&at_least).unwrap_or_default();
        assert_eq!(
            sentence,
            "The estimated chance that cost is less than 7000 USD is 0%."
        );
        // A zero-width interval: no interval, no narrowing, the plain chance stays.
        let flat = controlled(
            json!({ "probability": 1.0, "interval": null, "narrowing": null,
                    "helped": false, "uncontrolled_interval": [0.48, 0.52] }),
        );
        assert_eq!(controlled_headline(&flat)["target"]["probability"], 0.5);
        assert!(
            control_lines(&flat)[1].contains("no width"),
            "{:?}",
            control_lines(&flat)
        );
        // Even where a stale result says it helped, a missing interval keeps the plain one.
        let mut stale = flat.clone();
        stale["controlled"]["estimate"]["target"]["helped"] = json!(true);
        assert_eq!(controlled_headline(&stale)["target"]["probability"], 0.5);
    }

    /// A mean whose controlled half-width is zero is not shown as a narrowed one either.
    #[test]
    fn a_controlled_mean_with_no_width_stays_plain() {
        let o = json!({
            "name": "cost", "unit": "", "mean": 100.0, "half_width": 2.0,
            "percentiles": { "5": 90.0, "95": 110.0 },
            "controlled": {
                "controls": [{ "name": "labour", "kind": "input", "mean": 5.0,
                               "coefficient": 2.0, "sample_mean": 5.0, "mean_mismatch": false }],
                "pilot_trials": 1000, "correlation": 1.0, "unavailable": null,
                "estimate": { "mean": 101.0, "half_width": 0.0, "narrowing": null,
                              "helped": false, "target": null },
            },
        });
        assert_eq!(controlled_headline(&o)["mean"], 100.0);
        assert!(control_lines(&o)[1].contains("no width"));
        let mut stale = o.clone();
        stale["controlled"]["estimate"]["helped"] = json!(true);
        assert_eq!(controlled_headline(&stale)["mean"], 100.0);
    }

    /// An output whose trials show no spread has no ±, and the line under its finding says so
    /// instead of leaving a mean that looks exact.
    #[test]
    fn no_variation_is_said_not_shown_as_an_exact_mean() {
        let o = json!({
            "name": "y", "unit": "", "mean": 1.0, "half_width": null, "no_variation": true,
            "percentiles": { "5": 1.0, "95": 1.0 },
        });
        let (sentence, sure) = finding(&o).unwrap_or_default();
        assert!(sentence.contains("the mean is 1."), "{sentence}");
        assert!(!sentence.contains('±'), "{sentence}");
        let sure = sure.unwrap_or_default();
        assert!(sure.starts_with("No variation was observed"), "{sure}");
        // Without the flag there is no such line.
        let mut plain = o.clone();
        plain["no_variation"] = json!(false);
        assert_eq!(finding(&plain).map(|f| f.1), Some(None));
    }

    /// A stop on its target with an output that never varied does not say every output met the
    /// precision.
    #[test]
    fn a_stop_names_the_outputs_whose_precision_is_not_established() {
        let mut r = json!({
            "trials": 1000,
            "stop": { "reason": "target_met", "relative": 0.05, "met": ["A"], "not_established": ["B"] },
        });
        let line = stop_line(&r).unwrap_or_default();
        assert!(
            line.contains("precision met for A; not established for B"),
            "{line}"
        );
        assert!(line.contains("about 1 in 333 trials"), "{line}");
        assert!(!line.contains("every output"), "{line}");
        r["stop"]["not_established"] = json!([]);
        let line = stop_line(&r).unwrap_or_default();
        assert!(line.contains("every output's mean is within"), "{line}");
    }

    /// A control whose stated mean the trials contradict (a wrong mean gives a narrow interval
    /// around a wrong value) never takes the headline, for a mean or for a target's chance; the
    /// lines say the plain result stands.
    #[test]
    fn a_flagged_control_never_displaces_the_plain_headline() {
        let controls = json!([{ "name": "c", "kind": "input", "mean": 0.0,
                                "coefficient": 2.0, "sample_mean": 0.5, "mean_mismatch": true }]);
        let mut o = output(json!({ "at_most": 1.0 }), 0.5019, json!([0.49, 0.51]));
        o["name"] = json!("y");
        o["mean"] = json!(0.9976);
        o["half_width"] = json!(0.01);
        o["controlled"] = json!({
            "controls": controls, "pilot_trials": 1000, "correlation": 1.0, "unavailable": null,
            "estimate": { "mean": -0.0018, "half_width": 0.001, "narrowing": 0.9, "helped": true,
                          "target": { "probability": 1.0, "interval": [0.99, 1.0],
                                      "narrowing": 0.9, "helped": true } },
        });
        let shown = controlled_headline(&o);
        assert_eq!(shown["target"]["probability"], 0.5019);
        assert_eq!(shown["target"]["interval"], json!([0.49, 0.51]));
        assert!(shown["target"]["controlled"].is_null());
        let lines = control_lines(&o);
        assert!(lines[1].contains("plain result stands"), "{lines:?}");
        assert!(!lines[1].contains("shown above"), "{lines:?}");
        // The same for a mean.
        let mut m = o.clone();
        m["target"] = Value::Null;
        m["controlled"]["estimate"]["target"] = Value::Null;
        let shown = controlled_headline(&m);
        assert_eq!(
            (shown["mean"].as_f64(), shown["half_width"].as_f64()),
            (Some(0.9976), Some(0.01))
        );
        // With the flag cleared the controlled mean is the headline again.
        m["controlled"]["controls"][0]["mean_mismatch"] = json!(false);
        assert_eq!(controlled_headline(&m)["mean"], -0.0018);
    }

    /// With a target, the controlled chance of meeting it is the headline when it helped.
    #[test]
    fn a_controlled_target_chance_replaces_the_plain_one() {
        let mut o = output(json!({ "at_most": 7_000.0 }), 0.8, json!([0.7, 0.9]));
        o["controlled"] = json!({
            "controls": [{ "name": "labour", "kind": "input", "mean": 5.0,
                           "coefficient": 2.0, "sample_mean": 5.0, "mean_mismatch": false }],
            "pilot_trials": 1000, "correlation": 0.9, "unavailable": null,
            "estimate": { "mean": 1.0, "half_width": 0.1, "narrowing": 0.0, "helped": false,
                          "target": { "probability": 0.85, "interval": [0.82, 0.88],
                                      "narrowing": 0.4, "helped": true } },
        });
        let shown = controlled_headline(&o);
        assert_eq!(shown["target"]["probability"], 0.85);
        assert_eq!(shown["target"]["interval"], json!([0.82, 0.88]));
        assert!(control_lines(&o)[1].contains("narrowed the interval by 40%"));
    }
}
