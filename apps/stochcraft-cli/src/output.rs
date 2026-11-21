//! Text output: results as a table that reads like a textbook result, or as CSV.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::Value;
use stochcraft_engine::wire::csv_field;
pub use stochcraft_engine::wire::results_csv as csv;

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
    if results["sequential"].is_object() {
        let _ = writeln!(
            out,
            "± is estimated precision after adaptive stopping: the half-width of a 95% Student-t \
             interval across replications whose number was chosen from the results, so its \
             coverage is not guaranteed to be 95%"
        );
    } else {
        let _ = writeln!(
            out,
            "± is the half-width of a 95% Student-t confidence interval across replications"
        );
    }
}

/// A sequential run's outcome (`sequential`): how many replications it ran and after which
/// looks, why it stopped, and the results it could not settle.
fn sequential_lines(results: &Value, out: &mut String) {
    let q = &results["sequential"];
    if !q.is_object() {
        return;
    }
    let stages: Vec<String> = q["stages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(Value::to_string)
        .collect();
    let why = match s(&q["reason"]) {
        "target_met" => "every result watched that varies is precise enough".to_owned(),
        "no_spread" => format!(
            "the maximum of {} replications was reached, with results that had no spread in the \
             runs made",
            q["replications"]
        ),
        "mean_near_zero" => format!(
            "the maximum of {} replications was reached, with results whose mean is at or near \
             zero (give the result an absolute half-width instead)",
            q["replications"]
        ),
        _ => format!(
            "the maximum of {} replications was reached before every result watched was precise \
             enough",
            q["replications"]
        ),
    };
    let _ = writeln!(
        out,
        "\nsequential: {} replications (looked at {}); stopped because {why}",
        q["replications"],
        stages.join(", ")
    );
    let open: Vec<String> = q["measures"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["status"] != "settled")
        .map(|m| {
            let status = match s(&m["status"]) {
                "no_spread" => "no spread in the runs made",
                "mean_near_zero" => "mean at or near zero",
                "no_interval" => "no interval",
                "no_value" => "no value",
                _ => "wider than the target",
            };
            format!("{}.{} ({status})", s(&m["element"]), s(&m["metric"]))
        })
        .collect();
    if !open.is_empty() {
        let _ = writeln!(out, "not settled: {}", open.join(", "));
    }
    let no_spread = q["no_spread"].as_array().is_some_and(|l| !l.is_empty());
    if let (true, Some(p)) = (no_spread, q["unseen_rate"].as_f64()) {
        let _ = writeln!(
            out,
            "no spread: these results were the same in all {} runs; an event none of them met \
             could still happen in up to {:.1}% of runs (95% confidence)",
            q["replications"],
            p * 100.0
        );
    }
}

/// `rows` as aligned columns, two spaces apart: text left where `left` says so, numbers right.
fn aligned<R: AsRef<[String]>>(rows: &[R], left: &[bool], out: &mut String) {
    let cols = rows.iter().map(|r| r.as_ref().len()).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.as_ref().get(c))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in rows {
        let line: Vec<String> = row
            .as_ref()
            .iter()
            .enumerate()
            .map(|(c, cell)| {
                let pad = " ".repeat(widths[c] - cell.chars().count());
                if left.get(c).copied().unwrap_or(false) {
                    format!("{cell}{pad}")
                } else {
                    format!("{pad}{cell}")
                }
            })
            .collect();
        let _ = writeln!(out, "{}", line.join("  ").trim_end());
    }
}

/// A capacity table number (`{value, bound}` or `{unknown}`) for a table: `≤` or `≥` before a
/// bound, `?` when not known; `percent` prints it as a percentage.
fn bounded(v: &Value, percent: bool) -> String {
    let Some(x) = v["value"].as_f64() else {
        return "?".into();
    };
    let shown = if percent {
        format!("{}%", number((x * 1000.0).round() / 10.0))
    } else {
        number(x)
    };
    match s(&v["bound"]) {
        "at_most" => format!("≤ {shown}"),
        "at_least" => format!("≥ {shown}"),
        _ => shown,
    }
}

/// The capacity table (`model.capacity`) as text: a row per station, the bottleneck, then what
/// is not known before a run and why no formula holds where none does.
pub fn capacity(file: &Path, t: &Value) -> String {
    let mut out = String::new();
    let unit = s(&t["time_unit"]);
    let target = t["target"].as_f64().unwrap_or(0.85);
    let _ = writeln!(
        out,
        "Capacity of {} before any run (rates per {unit}, target {}%)\n",
        file.display(),
        number(target * 100.0)
    );
    let stations = t["stations"].as_array().cloned().unwrap_or_default();
    if stations.is_empty() {
        out.push_str("No servers, shared tasks, delays or pools.\n");
        return out;
    }
    let mut rows = vec![vec![
        "Station".to_owned(),
        "Kind".into(),
        "Arrivals".into(),
        "Service".into(),
        "Units".into(),
        "Utilisation".into(),
        "Load".into(),
        "Status".into(),
        format!("Units for {}%", number(target * 100.0)),
        "Formula".into(),
        "Mean wait".into(),
    ]];
    let (mut unknown, mut no_formula) = (Vec::new(), Vec::new());
    for st in &stations {
        let name = s(&st["name"]).to_owned();
        let kind = s(&st["kind"]).replace('_', " ");
        let units = match st["units"].as_u64() {
            None => "-".to_owned(),
            Some(u) if st["units_vary"] == true => format!("up to {u}"),
            Some(u) => u.to_string(),
        };
        let pool = st["kind"] == "pool";
        let delay = st["kind"] == "delay";
        for (what, key) in [
            ("arrival rate", "arrival_rate"),
            ("service time", "mean_service"),
            ("utilisation", "utilisation"),
        ] {
            if pool && key != "utilisation" {
                continue;
            }
            if let Some(m) = st[key]["unknown"]["message"].as_str() {
                unknown.push(format!("  {name}: {what}: {m}."));
                break;
            }
        }
        if !pool && let Some(m) = st["no_formula"]["message"].as_str() {
            no_formula.push(format!("  {name}: {m}."));
        }
        rows.push(vec![
            name,
            kind,
            if pool {
                "-".into()
            } else {
                bounded(&st["arrival_rate"], false)
            },
            if pool {
                "-".into()
            } else {
                bounded(&st["mean_service"], false)
            },
            units,
            if delay {
                "-".into()
            } else {
                bounded(&st["utilisation"], true)
            },
            if delay {
                "-".into()
            } else {
                bounded(&st["load"], true)
            },
            s(&st["status"]).to_owned(),
            st["units_for_target"]
                .as_u64()
                .map_or_else(|| "-".into(), |u| u.to_string()),
            s(&st["formula"]["name"]).to_owned(),
            if delay {
                "-".into()
            } else {
                f(&st["formula"]["mean_wait"])
            },
        ]);
    }
    aligned(&rows, &[true, true], &mut out);
    if let Some(b) = t["bottleneck"].as_str()
        && let Some(st) = stations.iter().find(|st| st["id"] == b)
    {
        let _ = writeln!(
            out,
            "\nBottleneck: {} (load {}).",
            s(&st["name"]),
            bounded(&st["load"], true)
        );
    } else if stations
        .iter()
        .any(|st| matches!(s(&st["kind"]), "server" | "pool" | "conveyor" | "transport"))
    {
        out.push_str(
            "
Bottleneck: not known before a run (a load not known, or only bounded, could be the highest).
",
        );
    }
    if !unknown.is_empty() {
        out.push_str("\nNot known before a run:\n");
        for l in &unknown {
            let _ = writeln!(out, "{l}");
        }
    }
    if !no_formula.is_empty() {
        out.push_str("\nNo exact formula for the wait:\n");
        for l in &no_formula {
            let _ = writeln!(out, "{l}");
        }
    }
    out.push_str(
        "\nUtilisation is arrivals × service ÷ units, as the results count it; the load is that \
         over the units' availability. A run adds what variability and blocking cost.\n",
    );
    out
}

fn heuristic_title(name: &str) -> &str {
    match name {
        "largest_candidate" => "Largest candidate rule",
        "kilbridge_wester" => "Kilbridge and Wester",
        "ranked_positional_weight" => "Ranked positional weight",
        other => other,
    }
}

/// Line balance (`line.balance`) as text: the cycle time, work content and theoretical minimum,
/// then for each heuristic its stations (number, elements and time) and its efficiency, delay and
/// smoothness.
pub fn balance(file: &Path, r: &Value) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Line balance of {} (cycle {}, work content {}, theoretical minimum {} station{})",
        file.display(),
        f(&r["cycle"]),
        f(&r["work_content"]),
        r["theoretical_min"],
        if r["theoretical_min"].as_u64() == Some(1) {
            ""
        } else {
            "s"
        }
    );
    for b in r["balances"].as_array().into_iter().flatten() {
        let stations = b["stations"].as_array().map_or(&[][..], Vec::as_slice);
        let _ = writeln!(
            out,
            "\n{} ({} station{}, longest {}):",
            heuristic_title(s(&b["heuristic"])),
            stations.len(),
            if stations.len() == 1 { "" } else { "s" },
            f(&b["longest"])
        );
        let mut rows = vec![["Station".to_owned(), "Elements".into(), "Time".into()]];
        for (i, st) in stations.iter().enumerate() {
            let elements: Vec<&str> = st["elements"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            rows.push([(i + 1).to_string(), elements.join(", "), f(&st["time"])]);
        }
        aligned(&rows, &[false, true, false], &mut out);
        let _ = writeln!(
            out,
            "  efficiency {:.2}, delay {:.2}, smoothness {:.3}",
            b["efficiency"].as_f64().unwrap_or(0.0),
            b["delay"].as_f64().unwrap_or(0.0),
            b["smoothness"].as_f64().unwrap_or(0.0)
        );
    }
    out
}

/// A factor or parameter `{id, field}` as `id.field`.
fn label(v: &Value) -> String {
    format!("{}.{}", s(&v["id"]), s(&v["field"]))
}

fn yes_no(v: &Value) -> &'static str {
    match v.as_bool() {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    }
}

/// A designed experiment (`experiment.design`) as text: what was run, a table of the points,
/// then the main effects and interactions (a full factorial) or the rank correlations (a Latin
/// hypercube).
pub fn design_table(name: &str, d: &Value) -> String {
    let mut out = String::new();
    let factors = d["factors"].as_array().map_or(&[][..], Vec::as_slice);
    let points = d["points"].as_array().map_or(&[][..], Vec::as_slice);
    let title = if name.is_empty() {
        "(unnamed model)"
    } else {
        name
    };
    let kind = match s(&d["design"]) {
        "latin_hypercube" => "Latin hypercube",
        _ => "full factorial",
    };
    let _ = writeln!(
        out,
        "{title}: {kind} of {} factor{}, {} points × {} replications = {} runs, seed {}",
        factors.len(),
        if factors.len() == 1 { "" } else { "s" },
        points.len(),
        d["replications"],
        d["runs"],
        d["seed"]
    );
    let _ = writeln!(
        out,
        "every point runs on the same seed (common random numbers); ± is the half-width of a 95% \
         Student-t confidence interval across replications\n"
    );
    let mut header = vec!["point".to_owned()];
    header.extend(factors.iter().map(label));
    header.extend(["result", "mean", "± half-width", "n"].map(String::from));
    let mut rows = vec![header];
    for (i, p) in points.iter().enumerate() {
        for (j, r) in p["results"].as_array().into_iter().flatten().enumerate() {
            let mut row = vec![if j == 0 {
                (i + 1).to_string()
            } else {
                String::new()
            }];
            row.extend((0..factors.len()).map(|k| {
                if j == 0 {
                    f(&p["values"][k])
                } else {
                    String::new()
                }
            }));
            row.push(format!("{}.{}", s(&r["element"]), s(&r["metric"])));
            row.push(f(&r["mean"]));
            row.push(f(&r["half_width"]));
            row.push(r["n"].to_string());
            rows.push(row);
        }
    }
    let mut left = vec![true];
    left.extend(std::iter::repeat_n(false, factors.len()));
    left.push(true);
    aligned(&rows, &left, &mut out);
    let effect_rows = |what: &[Value], first: &dyn Fn(&Value) -> Vec<String>| {
        what.iter()
            .map(|e| {
                let mut row = first(e);
                row.push(format!("{}.{}", s(&e["element"]), s(&e["metric"])));
                row.push(f(&e["effect"]));
                row.push(f(&e["half_width"]));
                row.push(yes_no(&e["significant"]).to_owned());
                row
            })
            .collect::<Vec<_>>()
    };
    if let Some(main) = d["main_effects"].as_array().filter(|m| !m.is_empty()) {
        let _ = writeln!(
            out,
            "\nmain effects: the mean at the factor's highest level minus at its lowest, over the \
             other factors' levels, paired replication by replication"
        );
        let mut rows = vec![
            [
                "factor",
                "low",
                "high",
                "result",
                "effect",
                "± half-width",
                "significant",
            ]
            .map(String::from)
            .to_vec(),
        ];
        rows.extend(effect_rows(main, &|e| {
            vec![label(e), f(&e["low"]), f(&e["high"])]
        }));
        aligned(
            &rows,
            &[true, false, false, true, false, false, true],
            &mut out,
        );
    }
    if let Some(inter) = d["interactions"].as_array().filter(|m| !m.is_empty()) {
        let _ = writeln!(
            out,
            "\ntwo-way interactions (two-level factors): half the change in one factor's effect \
             between the other's high and low level"
        );
        let mut rows = vec![
            ["factors", "result", "effect", "± half-width", "significant"]
                .map(String::from)
                .to_vec(),
        ];
        rows.extend(effect_rows(inter, &|e| {
            vec![format!(
                "{} × {}",
                label(&e["factors"][0]),
                label(&e["factors"][1])
            )]
        }));
        aligned(&rows, &[true, true, false, false, true], &mut out);
    }
    if let Some(tests) = d["tests"].as_u64().filter(|t| *t > 1) {
        let _ = writeln!(
            out,
            "\n{tests} effects, each tested at 95% on its own: about one in twenty with no real \
             effect still shows as significant by chance"
        );
    }
    if let Some(corr) = d["correlations"].as_array().filter(|c| !c.is_empty()) {
        let _ = writeln!(
            out,
            "\nrank (Spearman) correlations across the points, from −1 to 1; with few points one \
             factor's effect can hide another's, so a value near 0 does not show no effect"
        );
        let mut rows = vec![
            ["factor", "result", "spearman", "n"]
                .map(String::from)
                .to_vec(),
        ];
        rows.extend(corr.iter().map(|c| {
            vec![
                label(c),
                format!("{}.{}", s(&c["element"]), s(&c["metric"])),
                f(&c["spearman"]),
                c["n"].to_string(),
            ]
        }));
        aligned(&rows, &[true, true, false, false], &mut out);
    }
    // What a run of each point warns of, as for a single run.
    let warned: Vec<String> = points
        .iter()
        .enumerate()
        .flat_map(|(i, p)| {
            p["warnings"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|n| n["severity"] != "info")
                .map(move |n| format!("point {}: {}", i + 1, s(&n["message"])))
        })
        .collect();
    if !warned.is_empty() {
        out.push('\n');
        for w in warned {
            let _ = writeln!(out, "{w}");
        }
    }
    out
}

/// A designed experiment as CSV, one row per point and result: the point's number, each
/// factor's value (a column named `id.field`), the result, its mean, half-width and n.
pub fn design_csv(d: &Value) -> String {
    let factors = d["factors"].as_array().map_or(&[][..], Vec::as_slice);
    let mut out = String::from("point");
    for fa in factors {
        out.push(',');
        out.push_str(&csv_field(&label(fa)));
    }
    out.push_str(",element,metric,mean,half_width,n\n");
    let num = |v: &Value| {
        if v.is_null() {
            String::new()
        } else {
            v.to_string()
        }
    };
    for (i, p) in d["points"].as_array().into_iter().flatten().enumerate() {
        let values: Vec<String> = (0..factors.len()).map(|k| num(&p["values"][k])).collect();
        for r in p["results"].as_array().into_iter().flatten() {
            let _ = write!(out, "{}", i + 1);
            for v in &values {
                let _ = write!(out, ",{v}");
            }
            let _ = writeln!(
                out,
                ",{},{},{},{},{}",
                csv_field(s(&r["element"])),
                csv_field(s(&r["metric"])),
                num(&r["mean"]),
                num(&r["half_width"]),
                num(&r["n"])
            );
        }
    }
    out
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
    aligned(&rows, &[true, true, false, false, false], &mut out);
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
    split_lines(results, &mut out);
    stop_lines(results, &mut out);
    sequential_lines(results, &mut out);
    warmup_line(results, &mut out);
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

/// A block for each group of results whose replications split into two outcomes the same way
/// (`splits`): how the runs split, then each result's two values, a few to a line.
fn split_lines(results: &Value, out: &mut String) {
    for group in results["splits"].as_array().into_iter().flatten() {
        let members = group["results"].as_array().map_or(&[][..], Vec::as_slice);
        let count = match members.len() {
            1 => "1 result".to_owned(),
            k => format!("{k} results split the same way"),
        };
        let _ = writeln!(
            out,
            "runs split into two outcomes: {} of {} in one, {} in the other; {count}:",
            group["sizes"][0], group["n"], group["sizes"][1]
        );
        let items: Vec<String> = members
            .iter()
            .filter_map(|r| {
                let (id, metric) = (r[0].as_str()?, r[1].as_str()?);
                let e = results["elements"]
                    .as_array()?
                    .iter()
                    .find(|e| e["id"] == id)?;
                let split = &e["metrics"][metric]["split"];
                Some(format!(
                    "{id} {metric}: low ≈ {}, high ≈ {}",
                    f(&split["low"]["mean"]),
                    f(&split["high"]["mean"])
                ))
            })
            .collect();
        // A few to a line, so a group of two dozen results stays a short block.
        let mut line = String::new();
        for item in items {
            if !line.is_empty() && line.chars().count() + item.chars().count() > 96 {
                let _ = writeln!(out, "  {line};");
                line.clear();
            }
            if !line.is_empty() {
                line.push_str("; ");
            }
            line.push_str(&item);
        }
        if !line.is_empty() {
            let _ = writeln!(out, "  {line}");
        }
    }
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

/// The warm-up MSER-5 suggests from the number inside each task over the run, if the run
/// recorded it (`warmup_suggestion`).
fn warmup_line(results: &Value, out: &mut String) {
    let w = &results["warmup_suggestion"];
    if w.is_null() {
        return;
    }
    let unit = s(&results["time_unit"]);
    let _ = write!(out, "\nwarm-up suggested (MSER-5 on the number inside): ");
    match w["warmup"].as_f64() {
        Some(t) if t > 0.0 => {
            let _ = write!(out, "{} {unit}, at {}", number(t), s(&w["at"]));
            if results["warmup"].as_f64().is_some_and(|x| x >= t) {
                let _ = write!(out, " (the run's warm-up covers it)");
            }
        }
        Some(_) => {
            let _ = write!(out, "none, no start-up shows");
        }
        None => {
            let _ = write!(out, "cannot tell, still settling halfway through the run");
        }
    }
    // MSER-5 cuts something from many runs with no start-up at all: say when the start of the
    // run is no clearer than chance.
    if w["matters"] == false && w["warmup"].as_f64() != Some(0.0) {
        let _ = write!(out, " (no clearer than chance)");
    }
    if w["too_short"] == true && w["warmup"].is_number() {
        let _ = write!(out, "; some task is still settling halfway through the run");
    }
    out.push('\n');
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

/// A percentile of a Monte Carlo output's results (`percentiles` keyed by `5`, `50`…).
fn percentile(o: &Value, p: &str) -> String {
    f(&o["percentiles"][p])
}

/// A Monte Carlo output's target and the share of trials meeting it, with its interval, in words.
fn target_line(o: &Value) -> Option<String> {
    let t = o.get("target")?;
    let (word, x) = if let Some(x) = t.get("at_most") {
        ("at most", x)
    } else {
        ("at least", t.get("at_least")?)
    };
    let pct = |v: &Value| v.as_f64().map_or_else(|| "-".into(), |p| number(p * 100.0));
    Some(format!(
        "{} is {word} {} {}in {}% of trials (95% interval {}% to {}%)",
        s(&o["name"]),
        f(x),
        if s(&o["unit"]).is_empty() {
            String::new()
        } else {
            format!("{} ", s(&o["unit"]))
        },
        pct(&t["probability"]),
        pct(&t["interval"][0]),
        pct(&t["interval"][1]),
    ))
}

/// A Monte Carlo run's results as a table: a row per output with its mean, half-width, standard
/// deviation and percentiles, then each target in words, the correlations achieved and the run
/// record.
pub fn mc_table(name: &str, r: &Value) -> String {
    let mut out = String::new();
    let rec = &r["record"];
    let title = if name.is_empty() {
        "(unnamed model)"
    } else {
        name
    };
    let _ = write!(out, "{title}");
    if let Some(file) = rec["file"].as_str() {
        let _ = write!(out, "  {file}");
        if rec["unsaved_changes"] == true {
            let _ = write!(out, " (with unsaved changes)");
        }
    }
    out.push('\n');
    let sampling = match s(&rec["sampling"]) {
        "latin_hypercube" => "Latin hypercube sampling",
        _ => "random sampling",
    };
    let _ = writeln!(
        out,
        "Monte Carlo: {} trials, seed {}, {sampling}",
        r["trials"], rec["seed"]
    );
    if let (Some(run), Some(asked)) = (r["trials"].as_u64(), rec["trials_requested"].as_u64())
        && r.get("stop").is_none()
        && run != asked
    {
        let _ = writeln!(
            out,
            "{asked} trials asked for, rounded up to whole batches of 1000"
        );
    }
    let _ = writeln!(out, "intervals: {}", s(&rec["interval"]));
    let _ = writeln!(
        out,
        "StochCraft {}, model {}",
        s(&rec["stochcraft"]),
        s(&rec["model_digest"])
    );
    out.push('\n');
    let mut rows: Vec<Vec<String>> = vec![
        [
            "output",
            "unit",
            "mean",
            "± half-width",
            "std dev",
            "P5",
            "P50",
            "P95",
            "min",
            "max",
        ]
        .iter()
        .map(|h| (*h).to_owned())
        .collect(),
    ];
    for o in r["outputs"].as_array().into_iter().flatten() {
        rows.push(vec![
            s(&o["name"]).to_owned(),
            s(&o["unit"]).to_owned(),
            f(&o["mean"]),
            f(&o["half_width"]),
            f(&o["std_dev"]),
            percentile(o, "5"),
            percentile(o, "50"),
            percentile(o, "95"),
            f(&o["min"]),
            f(&o["max"]),
        ]);
    }
    aligned(&rows, &[true, true], &mut out);
    for o in r["outputs"].as_array().into_iter().flatten() {
        if let Some(line) = target_line(o) {
            let _ = writeln!(out, "{line}");
        }
        if o["invalid"].as_u64().is_some_and(|n| n > 0) {
            let _ = writeln!(
                out,
                "{}: {} trials gave no number (a division by zero, say) and are left out",
                s(&o["name"]),
                o["invalid"]
            );
        }
        // An interval across fewer values than the run has (trials with no number, batches
        // without one) says how many.
        if let (Some(n), Some(of)) = (
            o["interval_count"].as_u64(),
            r["interval_basis"]["count"].as_u64(),
        ) && n != of
        {
            let _ = writeln!(
                out,
                "{}: its interval is across {n} of the {of} {}",
                s(&o["name"]),
                s(&r["interval_basis"]["across"])
            );
        }
    }
    for c in r["correlations"].as_array().into_iter().flatten() {
        let _ = writeln!(
            out,
            "rank correlation of {} and {}: {} (asked for {})",
            s(&c["a"]),
            s(&c["b"]),
            f(&c["achieved"]),
            f(&c["target"])
        );
    }
    if let Some(stop) = r.get("stop") {
        let why = match s(&stop["reason"]) {
            "target_met" => "every output's mean is within the share asked for".to_owned(),
            "max_reached" => format!("it reached the most trials, {}", stop["max"]),
            other => other.replace('_', " "),
        };
        let _ = writeln!(
            out,
            "sequential: stopped after {} trials because {why} (looks at {})",
            r["trials"],
            stop["looks"]
                .as_array()
                .map(|l| l
                    .iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default()
        );
    }
    let notes: Vec<&Value> = r["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(rec["file_notes"].as_array().into_iter().flatten())
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

/// A Monte Carlo run's results as CSV: a row per output, as the app exports them.
pub fn mc_csv(r: &Value) -> String {
    stochcraft_engine::mc::results_csv(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_that_split_get_a_block_per_group_and_csv_columns() {
        let m = |mean: f64, low: f64, high: f64, group: u64| {
            serde_json::json!({
                "mean": mean, "half_width": 0.6, "std_dev": 3.4, "n": 128,
                "split": {
                    "cut": 5.8,
                    "low": { "n": 99, "mean": low },
                    "high": { "n": 29, "mean": high },
                    "group": group,
                },
            })
        };
        let results = serde_json::json!({
            "replications": 128,
            "metrics": ["entered", "left", "mean_queue_length", "mean_wait", "mean_in_service"],
            "elements": [{
                "id": "desk", "name": "Desk", "kind": "server",
                "metrics": {
                    "entered": { "mean": 500.0, "half_width": 3.0, "std_dev": 9.0, "n": 128 },
                    "left": m(400.0, 380.0, 470.0, 2),
                    "mean_queue_length": m(40.0, 1.5, 164.0, 1),
                    "mean_wait": m(12.0, 0.2, 29.7, 1),
                    "mean_in_service": m(3.8, 1.7, 9.9, 1),
                },
            }],
            "splits": [
                {
                    "id": 1, "n": 128, "sizes": [99, 29],
                    "results": [
                        ["desk", "mean_queue_length"],
                        ["desk", "mean_wait"],
                        ["desk", "mean_in_service"],
                    ],
                },
                { "id": 2, "n": 128, "sizes": [99, 29], "results": [["desk", "left"]] },
            ],
        });
        let text = table("m", &results);
        let expected = "runs split into two outcomes: 99 of 128 in one, 29 in the other; \
                        3 results split the same way:\n  \
                        desk mean_queue_length: low ≈ 1.5, high ≈ 164; \
                        desk mean_wait: low ≈ 0.2, high ≈ 29.7;\n  \
                        desk mean_in_service: low ≈ 1.7, high ≈ 9.9\n\
                        runs split into two outcomes: 99 of 128 in one, 29 in the other; \
                        1 result:\n  \
                        desk left: low ≈ 380, high ≈ 470\n";
        assert!(text.contains(expected), "{text}");
        assert_eq!(text.matches("runs split").count(), 2, "{text}");
        let csv = csv(&results);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[1], "desk,Desk,server,entered,500.0,3.0,9.0,128,,,,,,");
        assert_eq!(
            lines[5],
            "desk,Desk,server,mean_in_service,3.8,0.6,3.4,128,5.8,99,1.7,29,9.9,1"
        );
    }

    #[test]
    fn csv_has_the_rows_the_app_exports() {
        let m = serde_json::json!({ "mean": 2.0, "half_width": 0.5, "std_dev": 1.0, "n": 4 });
        let results = serde_json::json!({
            "metrics": ["entered", "custom,metric"],
            "elements": [{
                "id": "desk", "name": "Desk", "kind": "server",
                "metrics": { "entered": m, "custom,metric": m },
                "properties": { "size": m },
            }],
        });
        let text = csv(&results);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[1], "desk,Desk,server,entered,2.0,0.5,1.0,4,,,,,,");
        assert_eq!(
            lines[2],
            "desk,Desk,server,\"custom,metric\",2.0,0.5,1.0,4,,,,,,"
        );
        assert_eq!(
            lines[3],
            "desk,Desk,server,property:size,2.0,0.5,1.0,4,,,,,,"
        );
        assert_eq!(text, stochcraft_engine::wire::results_csv(&results));
    }

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

    #[test]
    fn a_sequential_run_says_how_many_runs_and_why_it_stopped() {
        let m = serde_json::json!({ "mean": 1.0, "half_width": 0.0, "std_dev": 0.0, "n": 200 });
        let mut results = serde_json::json!({
            "replications": 200,
            "time_unit": "min",
            "record": { "replications": 200, "time_unit": "min" },
            "metrics": ["entered"],
            "elements": [{ "id": "exit", "metrics": { "entered": m } }],
            "sequential": {
                "replications": 200,
                "stages": [10, 20, 40, 80, 160, 200],
                "reason": "no_spread",
                "measures": [{ "element": "exit", "metric": "entered", "status": "no_spread" }],
            },
        });
        let out = table("q", &results);
        assert!(
            out.contains("estimated precision after adaptive stopping"),
            "{out}"
        );
        assert!(!out.contains("± is the half-width of a 95%"), "{out}");
        assert!(
            out.contains("sequential: 200 replications (looked at 10, 20, 40, 80, 160, 200)"),
            "{out}"
        );
        assert!(out.contains("no spread in the runs made"), "{out}");
        assert!(
            out.contains("not settled: exit.entered (no spread in the runs made)"),
            "{out}"
        );
        // A fixed run says neither.
        results["sequential"] = Value::Null;
        let out = table("q", &results);
        assert!(!out.contains("sequential:"), "{out}");
        assert!(out.contains("± is the half-width of a 95%"), "{out}");
    }

    /// An output whose interval is across fewer values than the run has says how many.
    #[test]
    fn an_mc_interval_over_fewer_values_says_how_many() {
        let r = serde_json::json!({
            "trials": 100,
            "interval_basis": { "across": "trials", "count": 100 },
            "record": { "interval": "Student-t across 100 independent trials" },
            "outputs": [
                { "name": "a", "unit": "", "interval_count": 100, "invalid": 0 },
                { "name": "b", "unit": "", "interval_count": 40, "invalid": 60 },
            ],
        });
        let out = mc_table("m", &r);
        assert!(
            out.contains("b: its interval is across 40 of the 100 trials"),
            "{out}"
        );
        assert!(!out.contains("a: its interval"), "{out}");
    }
}
