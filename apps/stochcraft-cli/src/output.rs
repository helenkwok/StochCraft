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

/// A rank correlation to two decimals: 0.90, -0.42, and 0.00 for one too small to show.
fn rank_text(x: f64) -> String {
    let r = (x * 100.0).round() / 100.0;
    if r.abs() < 0.005 {
        "0.00".into()
    } else {
        format!("{r:.2}")
    }
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
    observed_lines(results, &mut out);
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

/// `v` of a result of metric `metric` as the table of observed data shows it: a share as a
/// percentage, a time with the run's time unit.
fn observed_number(v: f64, metric: &str, time_unit: &str) -> String {
    use stochcraft_engine::precision::{Unit, unit};
    match unit(metric) {
        Unit::Share => format!("{}%", number(v * 100.0)),
        Unit::Time => format!("{} {time_unit}", number(v)),
        Unit::Energy => format!("{} kWh", number(v)),
        Unit::Count | Unit::Items => number(v),
    }
}

/// `v ± half` in the units of [`observed_number`] (the unit once, after both).
fn observed_interval(v: &Value, half: &Value, metric: &str, time_unit: &str) -> String {
    use stochcraft_engine::precision::{Unit, unit};
    let (scale, suffix) = match unit(metric) {
        Unit::Share => (100.0, "%".to_owned()),
        Unit::Time => (1.0, format!(" {time_unit}")),
        Unit::Energy => (1.0, " kWh".to_owned()),
        Unit::Count | Unit::Items => (1.0, String::new()),
    };
    let Some(mean) = v.as_f64() else {
        return "-".into();
    };
    match half.as_f64() {
        Some(h) => format!("{} ± {}{suffix}", number(mean * scale), number(h * scale)),
        None => format!("{}{suffix}", number(mean * scale)),
    }
}

/// The results checked against observed data (`observed`): for each, the observed value or bound,
/// the simulated mean with its 95% half-width, the difference of the two (with its own interval)
/// for an observed value, and whether they agree. Those that differ or fail come first.
fn observed_lines(results: &Value, out: &mut String) {
    let Some(checks) = results["observed"].as_array().filter(|c| !c.is_empty()) else {
        return;
    };
    let time_unit = s(&results["time_unit"]);
    let mut rows: Vec<[String; 5]> = vec![[
        "verdict".into(),
        "result".into(),
        "observed".into(),
        "simulated".into(),
        "difference".into(),
    ]];
    let mut undecided = false;
    for c in checks {
        let metric = s(&c["metric"]);
        let verdict = match s(&c["verdict"]) {
            "agrees" => "agrees",
            "differs" => "differs",
            "passes" => "passes",
            "fails" => "fails",
            "no_result" => "no result",
            _ => {
                undecided = true;
                "can't tell yet"
            }
        };
        let observed = match s(&c["check"]) {
            "at_most" => format!(
                "at most {}",
                observed_number(c["bound"].as_f64().unwrap_or(f64::NAN), metric, time_unit)
            ),
            "at_least" => format!(
                "at least {}",
                observed_number(c["bound"].as_f64().unwrap_or(f64::NAN), metric, time_unit)
            ),
            _ => observed_interval(
                &c["observed"]["value"],
                &c["observed"]["half_width"],
                metric,
                time_unit,
            ),
        };
        let simulated = if c["simulated"].is_object() {
            observed_interval(
                &c["simulated"]["mean"],
                &c["simulated"]["half_width"],
                metric,
                time_unit,
            )
        } else {
            "-".to_owned()
        };
        let difference = match c["difference"].as_f64() {
            Some(d) => observed_interval(
                &Value::from(d),
                &c["difference_half_width"],
                metric,
                time_unit,
            ),
            None => String::new(),
        };
        rows.push([
            verdict.to_owned(),
            format!("{}.{metric}", s(&c["element"])),
            observed,
            simulated,
            difference,
        ]);
    }
    out.push_str("\nchecked against observed data:\n");
    aligned(&rows, &[true, true, false, false, false], out);
    if undecided {
        out.push_str(
            "can't tell yet: the 95% interval is too wide to decide, or there are fewer than 2 replications; more replications would decide\n",
        );
    }
    if checks.iter().any(|c| s(&c["verdict"]) == "no_result") {
        out.push_str(
            "no result: the run has no such result (the block is gone, or does not report that metric)\n",
        );
    }
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

/// Which inputs a Monte Carlo output depends on, as the window says it: the widest swing in
/// words, then a row for each input with the output at its 10th and its 90th percentile (the
/// others at their medians), the swing between, and the rank correlation with the output.
/// `correlated`: the model has correlated inputs.
fn influence_lines(o: &Value, correlated: bool, out: &mut String) {
    let Some(inputs) = o["influence"]["inputs"]
        .as_array()
        .filter(|i| !i.is_empty())
    else {
        return;
    };
    let name = s(&o["name"]);
    let unit = match s(&o["unit"]) {
        "" => String::new(),
        u => format!(" {u}"),
    };
    let _ = writeln!(out, "\nwhich inputs matter for {name}:");
    if let Some(top) = inputs.first()
        && top["swing"].as_f64().is_some_and(|w| w > 0.0)
    {
        let _ = writeln!(
            out,
            "{name} depends most on {input}: it goes from {low}{unit} to {high}{unit} as {input} goes from its 10th to its 90th percentile",
            input = s(&top["input"]),
            low = f(&top["low"]),
            high = f(&top["high"]),
        );
    }
    if let Some(base) = o["influence"]["base"].as_f64() {
        let _ = writeln!(
            out,
            "with every input at its median, {name} is {}{unit}",
            number(base)
        );
    }
    let mut rows: Vec<Vec<String>> = vec![
        [
            "input",
            "at its P10",
            "at its P90",
            "swing",
            "rank correlation",
        ]
        .iter()
        .map(|h| (*h).to_owned())
        .collect(),
    ];
    for i in inputs {
        rows.push(vec![
            s(&i["input"]).to_owned(),
            f(&i["low"]),
            f(&i["high"]),
            f(&i["swing"]),
            i["rank_correlation"]
                .as_f64()
                .map_or_else(|| "-".into(), rank_text),
        ]);
    }
    aligned(&rows, &[true], out);
    if correlated {
        let _ = writeln!(
            out,
            "the swings move one input at a time, so they leave out what correlated inputs do together; a correlated input's rank correlation includes the effect of the inputs it is correlated with"
        );
    }
}

/// A Monte Carlo run's results as a table: a row per output with its mean, half-width, standard
/// deviation and percentiles, then each target in words, which inputs matter for each output,
/// the correlations achieved and the run record.
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
        "antithetic" => "antithetic sampling",
        "sobol" => "Sobol (randomised quasi-Monte Carlo) sampling",
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
        let whole = if s(&rec["sampling"]) == "antithetic" {
            "pairs"
        } else {
            "batches of 1000"
        };
        let _ = writeln!(out, "{asked} trials asked for, rounded up to whole {whole}");
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
    let correlated = r["correlations"].as_array().is_some_and(|c| !c.is_empty());
    for o in r["outputs"].as_array().into_iter().flatten() {
        influence_lines(o, correlated, &mut out);
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

/// A share of a variance as a whole percentage: below 0 shown as 0% and above 1 as 100%, as a
/// small sample can overshoot, and never an exponent. `-` for no number.
fn share_text(x: f64) -> String {
    if x.is_finite() {
        // `+ 0.0` turns a negative zero into a zero, so it never reads "-0%".
        format!("{:.0}%", x.clamp(0.0, 1.0) * 100.0 + 0.0)
    } else {
        "-".into()
    }
}

/// An estimate's 95% interval as `55% to 67%`, or `-` when it has none.
fn share_interval(e: &Value) -> String {
    match (e["interval"][0].as_f64(), e["interval"][1].as_f64()) {
        (Some(lo), Some(hi)) => format!("{} to {}", share_text(lo), share_text(hi)),
        _ => "-".into(),
    }
}

/// `mc.sobol`'s results as text: for each output a table of each input's share of its variation
/// alone and with its interactions, each with its 95% interval, and in a sentence the input that
/// accounts for most of it alone.
pub fn sobol_lines(r: &Value) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "\nshares of the variation (Sobol indices from {} rows of each of two samples, {} model evaluations, seed {}):",
        r["rows"], r["evaluations"], r["seed"]
    );
    let mut overshoot = false;
    for o in r["outputs"].as_array().into_iter().flatten() {
        let name = s(&o["name"]);
        if let Some(why) = o["unavailable"].as_str() {
            let _ = writeln!(out, "{name}: no shares: {why}");
            continue;
        }
        let mut inputs: Vec<&Value> = o["inputs"].as_array().into_iter().flatten().collect();
        inputs.sort_by(|a, b| {
            let first = |i: &Value| i["first"]["value"].as_f64().unwrap_or(f64::NEG_INFINITY);
            first(b).total_cmp(&first(a))
        });
        let _ = writeln!(out, "{name}:");
        if let Some(top) = inputs.first()
            && let Some(x) = top["first"]["value"].as_f64()
            && x > 0.0
        {
            let _ = writeln!(
                out,
                "{} alone accounts for about {} of the variation in {name}",
                s(&top["input"]),
                share_text(x)
            );
        }
        let mut rows: Vec<Vec<String>> = vec![
            [
                "input",
                "alone",
                "95% interval",
                "with interactions",
                "95% interval",
            ]
            .iter()
            .map(|h| (*h).to_owned())
            .collect(),
        ];
        for i in &inputs {
            for e in [&i["first"], &i["total"]] {
                overshoot |= e["value"]
                    .as_f64()
                    .is_some_and(|x| !(0.0..=1.0).contains(&x));
            }
            rows.push(vec![
                s(&i["input"]).to_owned(),
                i["first"]["value"]
                    .as_f64()
                    .map_or_else(|| "-".into(), share_text),
                share_interval(&i["first"]),
                i["total"]["value"]
                    .as_f64()
                    .map_or_else(|| "-".into(), share_text),
                share_interval(&i["total"]),
            ]);
        }
        aligned(&rows, &[true, false, true, false, true], &mut out);
        let dropped = o["rows_dropped"].as_u64().unwrap_or(0);
        if dropped > 0 {
            let _ = writeln!(
                out,
                "{dropped} of {} rows gave no number for {name} and are left out",
                r["rows"]
            );
        }
    }
    let constants: Vec<&str> = r["constants"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !constants.is_empty() {
        let _ = writeln!(out, "constants, left out: {}", constants.join(", "));
    }
    if overshoot {
        let _ = writeln!(
            out,
            "a share estimated from few rows can fall a little below 0% or above 100%: it is shown as 0% or 100%"
        );
    }
    out
}

/// A system dynamics run's results as a table: a row per variable with its kind, its value at
/// the end of the run (the mean, and the 5% to 95% band, for a model run in replications), its
/// units, then what the checks found and the halved-step check.
pub fn sd_table(name: &str, r: &Value) -> String {
    let mut out = String::new();
    let title = if name.is_empty() {
        s(&r["model"])
    } else {
        name
    };
    let title = if title.is_empty() {
        "(unnamed model)"
    } else {
        title
    };
    let unit = s(&r["time_unit"]);
    let _ = writeln!(
        out,
        "{title}  (system dynamics, {}, time step {} {unit}, {} to {} {unit})",
        s(&r["method"]),
        f(&r["dt"]),
        f(&r["start"]),
        f(&r["stop"])
    );
    let many = r["replications"].as_u64().unwrap_or(1) > 1;
    if many {
        let _ = writeln!(
            out,
            "{} replications, seed {}; band: the 5th and 95th percentiles across them",
            r["replications"], r["seed"]
        );
    } else if r["random"] == true {
        let _ = writeln!(out, "one run, seed {}", r["seed"]);
    } else {
        let _ = writeln!(out, "one run (the model has no random function)");
    }
    let _ = writeln!(
        out,
        "StochCraft {}, model {}",
        s(&r["record"]["stochcraft"]),
        s(&r["record"]["model_digest"])
    );
    out.push('\n');
    let mut head: Vec<String> = ["variable", "kind", "at the end"]
        .iter()
        .map(|h| (*h).to_owned())
        .collect();
    if many {
        head.push("P5".to_owned());
        head.push("P50".to_owned());
        head.push("P95".to_owned());
    }
    head.push("units".to_owned());
    let mut rows = vec![head];
    for v in r["variables"].as_array().into_iter().flatten() {
        let last = |key: &str| {
            v[key]
                .as_array()
                .and_then(|a| a.last())
                .map_or_else(|| "-".to_owned(), f)
        };
        let mut row = vec![
            s(&v["name"]).to_owned(),
            s(&v["kind"]).to_owned(),
            f(&v["final"]),
        ];
        if many {
            row.push(last("p5"));
            row.push(last("median"));
            row.push(last("p95"));
        }
        row.push(s(&v["units"]).to_owned());
        rows.push(row);
    }
    let mut left = vec![true, true];
    if many {
        left.extend([false, false, false, false]);
    } else {
        left.push(false);
    }
    left.push(true);
    aligned(&rows, &left, &mut out);
    if let Some(c) = r["dt_check"].as_object() {
        let _ = writeln!(
            out,
            "\nhalved step: time step {} against {}",
            f(&c["dt"]),
            f(&c["half_dt"])
        );
        for st in c["stocks"].as_array().into_iter().flatten() {
            let _ = writeln!(
                out,
                "  {}: largest relative difference {} (at {} {unit})",
                s(&st["name"]),
                share_text_exact(&st["max_relative_difference"]),
                f(&st["at_time"])
            );
        }
        let verdict = if c["within_threshold"] == true {
            "results do not depend on the step beyond 1%"
        } else {
            "results depend on the step by more than 1%: use a smaller one"
        };
        let _ = writeln!(out, "{verdict}");
    }
    let notes: Vec<&str> = r["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !notes.is_empty() {
        out.push('\n');
        for n in notes {
            let _ = writeln!(out, "warning: {n}");
        }
    }
    out
}

/// A relative difference as a percentage to three significant digits.
fn share_text_exact(v: &Value) -> String {
    v.as_f64()
        .map_or_else(|| "-".to_owned(), |x| format!("{}%", number(x * 100.0)))
}

/// A system dynamics run's results as CSV: a row per saved time.
pub fn sd_csv(r: &Value) -> String {
    stochcraft_engine::sd::results_csv(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_read_as_percentages_with_their_intervals() {
        let est =
            |v: f64, lo: f64, hi: f64| serde_json::json!({ "value": v, "interval": [lo, hi] });
        let r = serde_json::json!({
            "rows": 1000, "evaluations": 4000, "seed": 7,
            "constants": ["fee"],
            "outputs": [
                {
                    "name": "duration", "rows_dropped": 0, "unavailable": null,
                    "inputs": [
                        { "input": "design", "first": est(0.2049, 0.15, 0.26), "total": est(0.2193, 0.17, 0.27) },
                        { "input": "build", "first": est(0.6112, 0.55, 0.67), "total": est(1.03, 0.9, 1.1) },
                        { "input": "test", "first": est(-0.02, -0.06, 0.01), "total": est(0.0, 0.0, 0.03) },
                    ],
                },
                { "name": "cost", "unavailable": "it does not vary", "inputs": [] },
            ],
        });
        let text = sobol_lines(&r);
        // The columns' padding does not matter here: each line with its spaces collapsed.
        let lines: Vec<String> = text
            .lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        let want = [
            "",
            "shares of the variation (Sobol indices from 1000 rows of each of two samples, 4000 model evaluations, seed 7):",
            "duration:",
            "build alone accounts for about 61% of the variation in duration",
            "input alone 95% interval with interactions 95% interval",
            "build 61% 55% to 67% 100% 90% to 100%",
            "design 20% 15% to 26% 22% 17% to 27%",
            "test 0% 0% to 1% 0% 0% to 3%",
            "cost: no shares: it does not vary",
            "constants, left out: fee",
            "a share estimated from few rows can fall a little below 0% or above 100%: it is shown as 0% or 100%",
        ];
        assert_eq!(lines, want, "{text}");
    }

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

    #[test]
    fn the_results_are_checked_against_observed_data_with_differences_first() {
        let m = |mean: f64, hw: f64| serde_json::json!({ "mean": mean, "half_width": hw, "std_dev": 1.0, "n": 10 });
        let mut results = serde_json::json!({
            "replications": 10,
            "time_unit": "min",
            "record": { "replications": 10, "time_unit": "min" },
            "metrics": ["mean_wait", "utilisation"],
            "elements": [{ "id": "desk", "metrics": {
                "mean_wait": m(5.1, 0.3), "utilisation": m(0.83, 0.02) } }],
            "observed": [
                { "element": "desk", "metric": "mean_wait", "check": "value", "verdict": "differs",
                  "observed": { "value": 3.2, "half_width": 0.4 },
                  "simulated": { "mean": 5.1, "half_width": 0.3, "n": 10 },
                  "difference": 1.9, "difference_half_width": 0.5 },
                { "element": "desk", "metric": "utilisation", "check": "at_most", "verdict": "cant_tell",
                  "bound": 0.85, "simulated": { "mean": 0.83, "half_width": 0.02, "n": 10 } },
                { "element": "gone", "metric": "left", "check": "at_least", "verdict": "no_result",
                  "bound": 5.0, "simulated": null },
            ],
        });
        let out = table("q", &results);
        let lines: Vec<&str> = out.lines().collect();
        let at = lines
            .iter()
            .position(|l| *l == "checked against observed data:");
        assert!(at.is_some(), "no section: {out}");
        let section = lines[at.unwrap_or(0) + 1..].join("\n");
        let differs = section.find("differs").unwrap_or(usize::MAX);
        let tell = section.find("can't tell yet").unwrap_or(usize::MAX);
        assert!(differs < tell, "{section}");
        assert!(
            section.contains("desk.mean_wait") && section.contains("3.2 ± 0.4 min"),
            "{section}"
        );
        assert!(section.contains("5.1 ± 0.3 min"), "{section}");
        // The difference with its own interval, and a share as a percentage.
        assert!(section.contains("1.9 ± 0.5 min"), "{section}");
        assert!(section.contains("at most 85%"), "{section}");
        assert!(section.contains("83 ± 2%"), "{section}");
        assert!(section.contains("no result"), "{section}");
        assert!(
            section.contains("more replications would decide"),
            "{section}"
        );
        // Without observed data there is no section.
        results["observed"] = Value::Null;
        assert!(!table("q", &results).contains("observed data"));
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

    /// Which inputs matter: the widest swing in words, a row for each input, and a note on
    /// correlated inputs only when the model has some.
    #[test]
    fn an_mc_table_says_which_inputs_matter() {
        let mut r = serde_json::json!({
            "trials": 100,
            "interval_basis": { "across": "trials", "count": 100 },
            "record": { "interval": "Student-t across 100 independent trials" },
            "outputs": [{
                "name": "duration", "unit": "days", "interval_count": 100, "invalid": 0,
                "influence": {
                    "base": 33.0,
                    "inputs": [
                        { "input": "build", "p10": 17.9, "p90": 27.5, "low": 29.0, "high": 38.6,
                          "swing": 9.6, "rank_correlation": 0.87 },
                        { "input": "design", "p10": 9.3, "p90": 13.8, "low": 31.0, "high": 35.5,
                          "swing": 4.5, "rank_correlation": 0.49 },
                    ],
                },
            }],
        });
        let out = mc_table("m", &r);
        assert!(
            out.contains("duration depends most on build: it goes from 29 days to 38.6 days as build goes from its 10th to its 90th percentile"),
            "{out}"
        );
        assert!(
            out.contains("with every input at its median, duration is 33 days"),
            "{out}"
        );
        assert!(out.contains("design"), "{out}");
        // Rank correlations to two decimals.
        assert!(out.contains("0.87") && out.contains("0.49"), "{out}");
        assert!(!out.contains("correlated inputs"), "{out}");
        r["correlations"] =
            serde_json::json!([{ "a": "build", "b": "design", "achieved": 0.4, "target": 0.4 }]);
        assert!(
            mc_table("m", &r).contains("includes the effect of the inputs it is correlated with")
        );
    }
}
