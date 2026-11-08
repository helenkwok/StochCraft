//! Text output: results as a table that reads like a textbook result, or as CSV.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;

/// A number for a table: about five significant digits, without a run of trailing zeros.
pub fn number(v: f64) -> String {
    if !v.is_finite() {
        return v.to_string();
    }
    if v == 0.0 {
        return "0".into();
    }
    // Only chooses how many digits to print; results never depend on it.
    #[allow(clippy::disallowed_methods)]
    let magnitude = v.abs().log10().floor() as i32;
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

fn f(v: &Value) -> String {
    v.as_f64().map_or_else(|| "-".into(), number)
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The run record as header lines.
fn record_lines(name: &str, results: &Value, out: &mut String) {
    let r = &results["record"];
    let unit = s(&r["time_unit"]);
    let title = if name.is_empty() {
        "(unnamed model)"
    } else {
        name
    };
    let _ = write!(out, "{title}");
    if let Some(file) = r["file"].as_str() {
        let _ = write!(out, "  {file}");
        if let Some(sc) = r["scenario"].as_str() {
            let _ = write!(out, ", scenario {sc}");
        }
        if r["unsaved_changes"] == true {
            let _ = write!(out, " (with unsaved changes)");
        }
    }
    out.push('\n');
    // The time actually measured: a stop condition can end replications early (the mean over
    // those whose statistics are used).
    let measured = match &results["measured"] {
        m if m.is_object() => &m["mean_duration"],
        _ => &r["duration"],
    };
    let _ = writeln!(
        out,
        "{} replications, seed {}, warm-up {} {unit}, then {} {unit} measured",
        r["replications"],
        r["seed"],
        f(&r["warmup"]),
        f(measured)
    );
    let _ = writeln!(
        out,
        "StochCraft {}, model {}",
        s(&r["stochcraft"]),
        s(&r["model_digest"])
    );
    let _ = writeln!(
        out,
        "± is the half-width of a 95% Student-t confidence interval across replications"
    );
}

/// Results as an aligned table: element, metric, mean, ± half-width, n.
pub fn table(name: &str, results: &Value) -> String {
    let mut out = String::new();
    record_lines(name, results, &mut out);
    out.push('\n');
    let mut rows: Vec<[String; 5]> = vec![[
        "element".into(),
        "metric".into(),
        "mean".into(),
        "± half-width".into(),
        "n".into(),
    ]];
    let order: Vec<&str> = results["metrics"]
        .as_array()
        .map(|m| m.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for e in results["elements"].as_array().into_iter().flatten() {
        let mut first = true;
        for metric in &order {
            let m = &e["metrics"][*metric];
            if m.is_null() {
                continue;
            }
            rows.push([
                if first {
                    s(&e["id"]).to_owned()
                } else {
                    String::new()
                },
                (*metric).to_owned(),
                f(&m["mean"]),
                f(&m["half_width"]),
                // How many replications had a value: "4 of 10" where not all did.
                match (m["n"].as_u64(), results["replications"].as_u64()) {
                    (Some(k), Some(n)) if k < n => format!("{k} of {n}"),
                    _ => m["n"].to_string(),
                },
            ]);
            first = false;
        }
    }
    let widths: Vec<usize> = (0..5)
        .map(|c| rows.iter().map(|r| r[c].chars().count()).max().unwrap_or(0))
        .collect();
    for row in &rows {
        let line: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(c, cell)| {
                let w = widths[c];
                let pad = w - cell.chars().count();
                // Text left, numbers right.
                if c < 2 {
                    format!("{cell}{}", " ".repeat(pad))
                } else {
                    format!("{}{cell}", " ".repeat(pad))
                }
            })
            .collect();
        let _ = writeln!(out, "{}", line.join("  ").trim_end());
    }
    let left_inside = results["elements"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|e| {
            e["metrics"]["work_in_progress"]["mean"]
                .as_f64()
                .is_some_and(|w| w > 0.0)
        });
    if left_inside {
        let _ = writeln!(
            out,
            "mean_wait and mean_time_in_system leave out entities still inside at the end"
        );
    }
    stop_lines(results, &mut out);
    let notes: Vec<&Value> = results["record"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(results["warnings"].as_array().into_iter().flatten())
        .chain(
            results["record"]["file_notes"]
                .as_array()
                .into_iter()
                .flatten(),
        )
        .filter(|n| n["severity"] != "info")
        .collect();
    if !notes.is_empty() {
        out.push('\n');
        for n in notes {
            let _ = writeln!(out, "{}", s(&n["message"]));
        }
    }
    out
}

/// A terminating run's stop: how many replications met it and when.
fn stop_lines(results: &Value, out: &mut String) {
    let stop = &results["stop"];
    if stop.is_null() {
        return;
    }
    let unit = s(&results["time_unit"]);
    let what = match s(&stop["when"]) {
        "count" => format!("{} entities left {}", stop["count"], s(&stop["element"])),
        _ => format!("{} emptied", s(&stop["element"])),
    };
    let _ = write!(
        out,
        "\nstopped when {what}: in {} of {} replications",
        stop["met"], results["replications"]
    );
    let t = &stop["time"];
    if !t.is_null() {
        let _ = write!(
            out,
            ", at {} ± {} {unit}",
            f(&t["mean"]),
            f(&t["half_width"])
        );
        if stop["met"].as_u64() < results["replications"].as_u64() {
            let _ = write!(out, " (over those that met it only)");
        }
    }
    out.push('\n');
}

fn csv_field(v: &str) -> String {
    if v.contains([',', '"', '\n']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_owned()
    }
}

/// Results as CSV, one row per element and metric.
pub fn csv(results: &Value) -> String {
    let mut out = String::from("element,name,kind,metric,mean,half_width,std_dev,n\n");
    for e in results["elements"].as_array().into_iter().flatten() {
        for metric in results["metrics"].as_array().into_iter().flatten() {
            let m = &e["metrics"][s(metric)];
            if m.is_null() {
                continue;
            }
            let _ = writeln!(
                out,
                "{},{},{},{},{},{},{},{}",
                csv_field(s(&e["id"])),
                csv_field(s(&e["name"])),
                s(&e["kind"]),
                s(metric),
                m["mean"],
                m["half_width"],
                m["std_dev"],
                m["n"]
            );
        }
    }
    out
}

/// `validate` as text: the file's notes, the model's diagnostics and a summary.
pub fn validation(file: &Path, opened: &Value, errors: usize) -> String {
    let mut out = String::new();
    let _ = write!(out, "{} ({}", file.display(), s(&opened["format"]));
    if let Some(sc) = opened["scenario"].as_str() {
        let _ = write!(out, ", scenario {sc}");
        let n = opened["scenarios"].as_array().map_or(0, Vec::len);
        if n > 1 {
            let _ = write!(out, " of {n}");
        }
    }
    out.push_str(")\n");
    let mut warnings = 0;
    for (heading, list) in [
        ("Reading the file", &opened["notes"]),
        ("The model", &opened["diagnostics"]),
    ] {
        let items = list.as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n{heading}:");
        for n in &items {
            let sev = s(&n["severity"]);
            if sev == "warning" {
                warnings += 1;
            }
            let at = n["at"]
                .as_str()
                .map(|a| format!(" at `{a}`"))
                .unwrap_or_default();
            // Diagnostics carry their severity in the message already.
            let msg = s(&n["message"]);
            if msg.starts_with(sev) {
                let _ = writeln!(out, "  {msg}");
            } else {
                let _ = writeln!(out, "  {sev}{at}: {msg}");
            }
        }
    }
    let _ = writeln!(
        out,
        "\n{errors} error{}, {warnings} warning{}",
        if errors == 1 { "" } else { "s" },
        if warnings == 1 { "" } else { "s" }
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_well() {
        assert_eq!(number(4.0), "4");
        assert_eq!(number(3.98767), "3.9877");
        assert_eq!(number(0.74996), "0.74996");
        assert_eq!(number(1234.5678), "1234.6");
        assert_eq!(number(20000.0), "20000");
        assert_eq!(number(0.0001234), "1.2340e-4");
        assert_eq!(number(-2.5), "-2.5");
        assert_eq!(csv_field("a, b"), "\"a, b\"");
    }

    #[test]
    fn the_table_says_how_many_replications_a_result_is_over() {
        let m = |mean: f64, n: u64| serde_json::json!({ "mean": mean, "half_width": 0.5, "n": n });
        let mut results = serde_json::json!({
            "replications": 10,
            "time_unit": "min",
            "record": { "replications": 10, "time_unit": "min" },
            "metrics": ["entered", "mean_wait", "work_in_progress"],
            "elements": [{ "id": "desk", "metrics": {
                "entered": m(12.0, 10), "mean_wait": m(3.0, 4), "work_in_progress": m(0.4, 10),
            } }],
            "stop": { "when": "empty", "element": "desk", "met": 7, "time": m(50.0, 7) },
        });
        let out = table("q", &results);
        assert!(out.contains("4 of 10"), "{out}");
        assert!(!out.contains("10 of 10"), "{out}");
        assert!(
            out.contains("leave out entities still inside at the end"),
            "{out}"
        );
        assert!(
            out.contains("in 7 of 10 replications, at 50 ± 0.5 min (over those that met it only)"),
            "{out}"
        );
        results["elements"][0]["metrics"]["work_in_progress"]["mean"] = 0.0.into();
        results["stop"]["met"] = 10.into();
        let out = table("q", &results);
        assert!(!out.contains("still inside"), "{out}");
        assert!(!out.contains("over those that met it"), "{out}");
    }
}
