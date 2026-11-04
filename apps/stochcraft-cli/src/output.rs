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
    let _ = writeln!(
        out,
        "{} replications, seed {}, warm-up {} {unit}, then {} {unit} measured",
        r["replications"],
        r["seed"],
        f(&r["warmup"]),
        f(&r["duration"])
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
                m["n"].to_string(),
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
    let notes: Vec<&Value> = results["record"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
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
}
