//! `stochcraft-cli analyse` and `stochcraft-cli warmup`: per-replication results made by any
//! simulation tool, spreadsheet or script that can write CSV, analysed by the engine's
//! `data.analyse` and `data.warmup`, without a model.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::Path;

use serde_json::{Value, json};
use stochcraft_engine::Engine;

use crate::Failure;
use crate::output::{aligned, number};

fn read(file: &Path) -> Result<(String, String), Failure> {
    if file == Path::new("-") {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| Failure(format!("cannot read standard input: {e}")))?;
        Ok((s, "standard input".to_owned()))
    } else {
        let s = std::fs::read_to_string(file)
            .map_err(|e| Failure(format!("cannot read {}: {e}", file.display())))?;
        Ok((s, file.display().to_string()))
    }
}

pub struct AnalyseArgs<'a> {
    pub file: &'a Path,
    pub scenario: Option<&'a str>,
    pub replication: Option<&'a str>,
    pub measure: Option<&'a str>,
    pub value: Option<&'a str>,
    pub baseline: Option<&'a str>,
    pub paired: bool,
    /// `measure=value` pairs.
    pub tolerance: &'a [String],
    pub json: bool,
}

/// The engine's parameters for `a`, with the file's text.
fn analyse_params(a: &AnalyseArgs, csv: String) -> Result<Value, Failure> {
    let mut p = json!({ "csv": csv, "paired": a.paired });
    for (key, v) in [
        ("scenario", a.scenario),
        ("replication", a.replication),
        ("measure", a.measure),
        ("value", a.value),
        ("baseline", a.baseline),
    ] {
        if let Some(v) = v {
            p[key] = json!(v);
        }
    }
    if !a.tolerance.is_empty() {
        let mut t = serde_json::Map::new();
        for item in a.tolerance {
            let (name, value) = item
                .rsplit_once('=')
                .and_then(|(n, v)| Some((n.trim(), v.trim().parse::<f64>().ok()?)))
                .ok_or_else(|| {
                    Failure(format!(
                        "--tolerance `{item}`: give a measure and a number, as wait=0.5"
                    ))
                })?;
            t.insert(name.to_owned(), json!(value));
        }
        p["tolerance"] = Value::Object(t);
    }
    Ok(p)
}

pub fn analyse(a: &AnalyseArgs) -> Result<u8, Failure> {
    let (csv, source) = read(a.file)?;
    let r = Engine::new().execute("data.analyse", analyse_params(a, csv)?)?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
    } else {
        print!("{}", analyse_text(&source, &r));
    }
    Ok(0)
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// A mean ± half-width as the answer rounds it, or the plain mean where there is no interval.
fn interval(e: &Value, mean: &str) -> String {
    match e["text"].as_str() {
        Some(t) => t.to_owned(),
        None => e[mean].as_f64().map_or_else(|| "-".to_owned(), number),
    }
}

fn status_words(p: &Value) -> String {
    match s(&p["status"]) {
        "settled" => "settled".into(),
        "wide" => match p["runs_needed"].as_u64() {
            Some(n) => format!("wide (about {n} runs)"),
            None => "wide".into(),
        },
        "no_spread" => "no spread".into(),
        "mean_near_zero" => "mean near zero".into(),
        "no_interval" => "no interval".into(),
        _ => "no value".into(),
    }
}

fn verdict_words(v: &str) -> &'static str {
    match v {
        "agrees" => "agrees within tolerance",
        "differs" => "differs",
        "no_difference_detected" => "no difference detected",
        _ => "inconclusive",
    }
}

/// The answer of `data.analyse` as text.
pub fn analyse_text(source: &str, r: &Value) -> String {
    let mut out = String::new();
    let scenarios: Vec<&str> = r["scenarios"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let _ = writeln!(
        out,
        "runs from {source}: {} rows, a {} table, {} measures{}",
        r["rows"],
        s(&r["shape"]),
        r["measures"].as_array().map_or(0, Vec::len),
        if scenarios.is_empty() {
            String::new()
        } else {
            format!(", scenarios {}", scenarios.join(", "))
        }
    );
    let named = !scenarios.is_empty();
    let mut rows: Vec<Vec<String>> = vec![
        [
            "scenario",
            "measure",
            "n",
            "mean ± 95%",
            "precision",
            "min",
            "max",
        ]
        .iter()
        .map(|h| (*h).to_owned())
        .collect(),
    ];
    let mut splits = Vec::new();
    for e in r["results"].as_array().into_iter().flatten() {
        let n = match e["missing"].as_u64() {
            Some(m) if m > 0 => format!("{} ({m} empty)", e["n"]),
            _ => e["n"].to_string(),
        };
        let f = |v: &Value| v.as_f64().map_or_else(|| "-".to_owned(), number);
        rows.push(vec![
            s(&e["scenario"]).to_owned(),
            s(&e["measure"]).to_owned(),
            n,
            interval(e, "mean"),
            status_words(&e["precision"]),
            f(&e["min"]),
            f(&e["max"]),
        ]);
        if e["split"].is_object() {
            let g = &e["split"];
            splits.push(format!(
                "{}{} splits into two outcomes: {} runs near {} and {} near {} (the mean describes no run)",
                if named {
                    format!("{} ", s(&e["scenario"]))
                } else {
                    String::new()
                },
                s(&e["measure"]),
                g["low"]["n"],
                f(&g["low"]["mean"]),
                g["high"]["n"],
                f(&g["high"]["mean"]),
            ));
        }
    }
    if !named {
        rows.iter_mut().for_each(|r| {
            r.remove(0);
        });
    }
    out.push('\n');
    let left = if named {
        vec![true, true, false, false, true]
    } else {
        vec![true, false, false, true]
    };
    aligned(&rows, &left, &mut out);
    for line in &splits {
        let _ = writeln!(out, "{line}");
    }
    let comparisons = r["comparisons"].as_array().map_or(&[][..], Vec::as_slice);
    if !comparisons.is_empty() {
        let how = if r["paired"] == true {
            "in pairs by replication (paired t)"
        } else {
            "as independent runs (Welch's t)"
        };
        let _ = writeln!(out, "\nagainst the baseline {}, {how}:", s(&r["baseline"]));
        let mut rows: Vec<[String; 4]> = vec![[
            "scenario".into(),
            "measure".into(),
            "difference ± 95%".into(),
            "verdict".into(),
        ]];
        for c in comparisons {
            let verdict = match c["tolerance"].as_f64() {
                Some(t) => format!("{} (±{})", verdict_words(s(&c["verdict"])), number(t)),
                None => verdict_words(s(&c["verdict"])).to_owned(),
            };
            rows.push([
                s(&c["scenario"]).to_owned(),
                s(&c["measure"]).to_owned(),
                interval(c, "difference"),
                verdict,
            ]);
        }
        aligned(&rows, &[true, true, false, true], &mut out);
        let verdicts: Vec<&str> = comparisons.iter().map(|c| s(&c["verdict"])).collect();
        if verdicts.contains(&"no_difference_detected") {
            out.push_str("no difference detected: the 95% interval of the difference holds 0. This is not evidence of agreement unless a tolerance is set: a wide interval holds 0 whatever the truth\n");
        }
        if verdicts.contains(&"inconclusive") {
            out.push_str("inconclusive: the 95% interval is neither inside nor outside the tolerance, or there are fewer than 2 values; more runs would narrow it\n");
        }
    }
    for n in r["notes"].as_array().into_iter().flatten() {
        let _ = writeln!(out, "note: {}", s(n));
    }
    out
}

pub struct WarmupArgs<'a> {
    pub file: &'a Path,
    pub time: Option<&'a str>,
    pub replication: Option<&'a str>,
    pub scenario: Option<&'a str>,
    pub json: bool,
}

pub fn warmup(a: &WarmupArgs) -> Result<u8, Failure> {
    let (csv, source) = read(a.file)?;
    let mut p = json!({ "csv": csv });
    for (key, v) in [
        ("time", a.time),
        ("replication", a.replication),
        ("scenario", a.scenario),
    ] {
        if let Some(v) = v {
            p[key] = json!(v);
        }
    }
    let r = Engine::new().execute("data.warmup", p)?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
    } else {
        print!("{}", warmup_text(&source, &r));
    }
    Ok(0)
}

/// The answer of `data.warmup` as text.
pub fn warmup_text(source: &str, r: &Value) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "warm-up by MSER-5 from {source} (each measure averaged across the replications at each time):"
    );
    for x in r["series"].as_array().into_iter().flatten() {
        let name = match x["scenario"].as_str() {
            Some(sc) => format!("{sc}, {}", s(&x["measure"])),
            None => s(&x["measure"]).to_owned(),
        };
        let times = x["times"].as_array().map_or(0, Vec::len);
        let w = &x["warmup"];
        let said = match s(&w["status"]) {
            "found" => format!(
                "delete the first {} of {times} times: steady from time {}",
                w["observations"],
                w["time"].as_f64().map_or_else(|| "-".to_owned(), number)
            ),
            "too_short" => {
                "the series is too short to show where the start-up ends: record longer runs"
                    .to_owned()
            }
            _ => format!(
                "too few times ({times}): MSER-5 needs at least {}",
                r["min_times"]
            ),
        };
        let _ = writeln!(out, "{name} ({} replications): {said}", x["replications"]);
    }
    out
}

#[cfg(test)]
#[allow(clippy::panic)] // A wrong answer fails the test.
mod tests {
    use super::*;

    #[test]
    fn a_tolerance_is_a_measure_and_a_number() {
        fn args(t: &[String]) -> AnalyseArgs<'_> {
            AnalyseArgs {
                file: Path::new("x.csv"),
                scenario: None,
                replication: Some("rep"),
                measure: None,
                value: None,
                baseline: None,
                paired: true,
                tolerance: t,
                json: false,
            }
        }
        let t = vec!["wait = 0.5".to_owned(), "a=b=2".to_owned()];
        let p =
            analyse_params(&args(&t), "w\n1\n".into()).unwrap_or_else(|Failure(m)| panic!("{m}"));
        assert_eq!(p["tolerance"], json!({ "wait": 0.5, "a=b": 2.0 }));
        assert_eq!(
            (p["replication"].as_str(), p["paired"].as_bool()),
            (Some("rep"), Some(true))
        );
        assert!(p.get("scenario").is_none());
        let bad = vec!["wait".to_owned()];
        assert!(analyse_params(&args(&bad), String::new()).is_err());
    }
}
