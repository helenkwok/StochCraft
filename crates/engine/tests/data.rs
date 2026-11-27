//! Results made by another tool (`data.analyse`, `data.warmup`): hand-made tables whose answers
//! are worked out here from the formulas.
//!
//! The tables are in `tests/data/`: `runs-wide.csv` (two scenarios, five replications each,
//! the second listed out of order, one value missing, a text column), `runs-long.csv` (the same
//! values as a long table, semicolon-delimited with decimal commas), `runs-split.csv` (twelve
//! replications, one measure split into two outcomes) and `series-step.csv` (a time series with
//! a start-up of ten times).
//!
//! Independent check: the same values were computed outside StochCraft with SciPy 1.18.1
//! (`scipy.stats.ttest_rel(...).confidence_interval()`, `scipy.stats.t.ppf`). StochCraft's t
//! critical values come from a table to three decimals, so they agree to about 5e-4: the paired
//! half-width is 0.680087 there (0.679979 here), the one desk's wait 1.963243 (1.962929), and
//! Welch's half-width with the degrees of freedom rounded down 2.061432 (2.061760).

#![allow(clippy::panic)] // A wrong answer fails the test.

use serde_json::{Value, json};
use stochcraft_engine::{Engine, ErrorCode};

const WIDE: &str = include_str!("../../../tests/data/runs-wide.csv");
const LONG: &str = include_str!("../../../tests/data/runs-long.csv");
const SPLIT: &str = include_str!("../../../tests/data/runs-split.csv");
const STEP: &str = include_str!("../../../tests/data/series-step.csv");

fn ok(command: &str, params: Value) -> Value {
    let r = Engine::new().execute(command, params.clone());
    r.unwrap_or_else(|e| panic!("{command} {params}: {e:?}"))
}

/// The error a request gives, with its message.
fn err(command: &str, params: Value) -> (ErrorCode, String) {
    match Engine::new().execute(command, params.clone()) {
        Ok(r) => panic!("{command} {params} should fail: {r}"),
        Err(e) => (e.code, e.message),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("not a number: {v}"))
}

/// The result of `measure` in `scenario` in an answer ("" for the one scenario of a table
/// without a scenario column).
fn result<'a>(r: &'a Value, scenario: &str, measure: &str) -> &'a Value {
    r["results"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|x| x["scenario"].as_str().unwrap_or("") == scenario && x["measure"] == measure)
        .unwrap_or_else(|| panic!("no {scenario} {measure} in {r}"))
}

fn comparison<'a>(r: &'a Value, measure: &str) -> &'a Value {
    r["comparisons"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|x| x["measure"] == measure)
        .unwrap_or_else(|| panic!("no comparison of {measure} in {r}"))
}

/// Mean and sample variance (n − 1).
fn moments(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n - 1.0);
    (mean, var)
}

const ONE_DESK: [f64; 5] = [4.0, 6.0, 5.0, 7.0, 8.0];
/// The two desks' waits by replication id 1 to 5 (the file lists them 3, 1, 2, 5, 4).
const TWO_DESKS: [f64; 5] = [3.0, 4.0, 4.0, 5.0, 6.0];

#[test]
fn each_scenario_has_its_mean_interval_and_precision() {
    let r = ok("data.analyse", json!({ "csv": WIDE }));
    assert_eq!(r["shape"], "wide");
    assert_eq!(r["delimiter"], "comma");
    assert_eq!(r["columns"]["scenario"], "scenario");
    assert_eq!(r["columns"]["replication"], "replication");
    // The text column is left out, by name.
    assert_eq!(r["columns"]["ignored"], json!(["note"]));
    assert_eq!(r["measures"], json!(["wait", "served"]));
    assert_eq!(r["scenarios"], json!(["one desk", "two desks"]));
    assert_eq!(r["baseline"], "one desk");

    // One desk: mean 6, sample variance 2.5, 95% half-width t(4) s / √5 with t(4) = 2.776.
    let (mean, var) = moments(&ONE_DESK);
    assert!(close(mean, 6.0) && close(var, 2.5));
    let hw = 2.776 * (var / 5.0).sqrt();
    let a = result(&r, "one desk", "wait");
    assert_eq!(a["n"], 5);
    assert_eq!(a["missing"], 0);
    assert!(close(f(&a["mean"]), mean));
    assert!(close(f(&a["half_width"]), hw));
    assert!(close(f(&a["std_dev"]), var.sqrt()));
    assert_eq!((f(&a["min"]), f(&a["max"])), (4.0, 8.0));
    // Against ±5% of the mean: score hw / (0.05 × 6) = 6.54, wide; about n × score²
    // replications would settle it (214.1), rounded up to tens.
    let score = hw / (0.05 * mean);
    assert_eq!(a["precision"]["status"], "wide");
    assert!(close(f(&a["precision"]["score"]), score));
    let needed = ((5.0 * score * score).ceil() as u64).div_ceil(10) * 10;
    assert_eq!(needed, 220);
    assert_eq!(a["precision"]["runs_needed"], needed);
    assert_eq!(a["precision"]["target"], json!({ "relative": 0.05 }));
    // Rounded as the tables round: the half-width 1.96 to one figure, the mean to the same
    // place.
    assert_eq!(a["text"], "6 ± 2");
    assert_eq!(a["decimals"], 0);
    assert!(a["split"].is_null());

    // Two desks: mean 4.4, variance 1.3; the half-width 1.42 starts with a 1, so two figures.
    let (mean_b, var_b) = moments(&TWO_DESKS);
    let b = result(&r, "two desks", "wait");
    assert!(
        close(f(&b["mean"]), mean_b) && close(f(&b["half_width"]), 2.776 * (var_b / 5.0).sqrt())
    );
    assert_eq!(b["text"], "4.4 ± 1.4");
    assert_eq!(b["decimals"], 1);

    // The empty cell is a replication with no value: four values.
    let served = result(&r, "one desk", "served");
    assert_eq!(
        (served["n"].as_u64(), served["missing"].as_u64()),
        (Some(4), Some(1))
    );
    let (m, v) = moments(&[100.0, 102.0, 98.0, 101.0]);
    assert!(close(f(&served["mean"]), m));
    assert!(close(f(&served["half_width"]), 3.182 * (v / 4.0).sqrt()));
    assert_eq!(served["precision"]["status"], "settled");
    assert!(served["precision"]["runs_needed"].is_null());
}

#[test]
fn independent_scenarios_are_compared_by_welch() {
    let r = ok("data.analyse", json!({ "csv": WIDE }));
    assert_eq!(r["paired"], false);
    let c = comparison(&r, "wait");
    assert_eq!(
        (c["scenario"].as_str(), c["baseline"].as_str()),
        (Some("two desks"), Some("one desk"))
    );
    assert_eq!(c["interval"], "welch_t");
    // Welch: variances of the means 2.5/5 and 1.3/5, Welch–Satterthwaite df 7.27, rounded down
    // to 7 (t = 2.365).
    let (ma, va) = moments(&ONE_DESK);
    let (mb, vb) = moments(&TWO_DESKS);
    let (ua, ub) = (va / 5.0, vb / 5.0);
    let df = ((ua + ub) * (ua + ub) / (ua * ua / 4.0 + ub * ub / 4.0)).floor();
    assert_eq!(df, 7.0);
    assert_eq!(c["df"], 7);
    assert!(close(f(&c["difference"]), mb - ma));
    assert!(close(f(&c["half_width"]), 2.365 * (ua + ub).sqrt()));
    assert_eq!(
        (c["n_baseline"].as_u64(), c["n_scenario"].as_u64()),
        (Some(5), Some(5))
    );
    // −1.6 ± 2.06 holds 0.
    assert_eq!(c["verdict"], "no_difference_detected");
    assert_eq!(c["text"], "-2 ± 2");
    assert!(c.get("pairs").is_none());
    // The same ids in both: the answer says that pairing could be declared.
    assert!(
        r["notes"][0]
            .as_str()
            .is_some_and(|n| n.contains("`paired`")),
        "{r}"
    );
}

#[test]
fn declared_pairs_are_matched_by_replication_id() {
    let r = ok("data.analyse", json!({ "csv": WIDE, "paired": true }));
    let c = comparison(&r, "wait");
    assert_eq!(c["interval"], "paired_t");
    // Differences by id: −1, −2, −1, −2, −2 (the file lists the second scenario out of order):
    // mean −1.6, variance 0.3, t(4) = 2.776.
    let d: Vec<f64> = ONE_DESK
        .iter()
        .zip(&TWO_DESKS)
        .map(|(a, b)| b - a)
        .collect();
    assert_eq!(d, [-1.0, -2.0, -1.0, -2.0, -2.0]);
    let (md, vd) = moments(&d);
    assert!(close(md, -1.6) && close(vd, 0.3));
    let hw = 2.776 * (vd / 5.0).sqrt();
    assert!(close(f(&c["difference"]), md));
    assert!(close(f(&c["half_width"]), hw));
    assert_eq!(c["df"], 4);
    assert_eq!(c["pairs"], 5);
    assert_eq!(c["verdict"], "differs");
    assert_eq!(c["text"], "-1.6 ± 0.7");
    // A replication with no value leaves its pair out: 4 pairs for `served`.
    assert_eq!(comparison(&r, "served")["pairs"], 4);
    assert!(
        r["notes"][0]
            .as_str()
            .is_some_and(|n| n.contains("cannot check")),
        "{r}"
    );

    // With a tolerance: −1.6 ± 0.68 is [−2.28, −0.92]; within ±3 it agrees, against ±1 it
    // straddles the edge, and wholly beyond ±0.5 it differs.
    let verdict = |t: f64| {
        let r = ok(
            "data.analyse",
            json!({ "csv": WIDE, "paired": true, "tolerance": { "wait": t } }),
        );
        let c = comparison(&r, "wait").clone();
        assert_eq!(c["tolerance"], t);
        c["verdict"].as_str().unwrap_or("").to_owned()
    };
    assert_eq!(verdict(3.0), "agrees");
    assert_eq!(verdict(1.0), "inconclusive");
    assert_eq!(verdict(0.5), "differs");
}

#[test]
fn a_long_table_with_decimal_commas_gives_the_same_answers() {
    let wide = ok("data.analyse", json!({ "csv": WIDE, "paired": true }));
    let long = ok("data.analyse", json!({ "csv": LONG, "paired": true }));
    assert_eq!(long["shape"], "long");
    assert_eq!(long["delimiter"], "semicolon");
    assert_eq!(long["decimal_comma"], true);
    assert_eq!(long["columns"]["measure"], "measure");
    assert_eq!(long["measures"], json!(["wait", "served"]));
    for (k, key) in ["results", "comparisons"].iter().enumerate() {
        assert_eq!(long[key], wide[key], "{k}");
    }
    // Only some measures, by name.
    let one = ok(
        "data.analyse",
        json!({ "csv": LONG, "measures": ["served"] }),
    );
    assert_eq!(one["measures"], json!(["served"]));
    assert_eq!(one["results"].as_array().map(Vec::len), Some(2));
}

#[test]
fn two_outcomes_are_named_and_a_single_scenario_has_no_comparison() {
    let r = ok("data.analyse", json!({ "csv": SPLIT }));
    assert_eq!(r["scenarios"], json!([null]));
    assert_eq!(r["comparisons"], json!([]));
    let b = result(&r, "", "backlog");
    let low = [1.0, 1.1, 0.9, 1.2, 1.05, 0.95];
    let high = [20.0, 21.0, 19.5, 20.5, 19.0, 21.5];
    let s = &b["split"];
    assert!(s.is_object(), "{b}");
    // Halfway between the highest low value and the lowest high one.
    assert!(close(f(&s["cut"]), (1.2 + 19.0) / 2.0));
    assert_eq!(
        (s["low"]["n"].as_u64(), s["high"]["n"].as_u64()),
        (Some(6), Some(6))
    );
    assert!(close(f(&s["low"]["mean"]), moments(&low).0));
    assert!(close(f(&s["high"]["mean"]), moments(&high).0));
    // A single-peaked measure does not split.
    assert!(result(&r, "", "wait")["split"].is_null());
}

#[test]
fn a_warm_up_is_found_where_the_start_up_ends() {
    let r = ok("data.warmup", json!({ "csv": STEP }));
    assert_eq!(r["columns"]["time"], "time");
    assert_eq!(r["columns"]["measures"], json!(["queue"]));
    let s = &r["series"][0];
    assert_eq!(s["replications"], 2);
    let times: Vec<f64> = (1..=40).map(f64::from).collect();
    assert_eq!(s["times"], json!(times));
    // The average across the two replications is each batch's mean, five times over.
    let means = [5.0, 5.0, 1.0, 1.2, 0.8, 1.1, 0.9, 1.0];
    let average: Vec<f64> = s["average"]
        .as_array()
        .into_iter()
        .flatten()
        .map(f)
        .collect();
    assert_eq!(average.len(), 40);
    for (k, a) in average.iter().enumerate() {
        assert!((a - means[k / 5]).abs() < 1e-12, "time {}: {a}", k + 1);
    }
    // MSER(d) over the batch means for d = 0 … 4 batches deleted: the squared standard error of
    // the rest, Σ(z − z̄)² / (8 − d)². The start-up is the first two batches, so d = 2 is least.
    let mser = |d: usize| {
        let kept = &means[d..];
        let k = kept.len() as f64;
        let m = kept.iter().sum::<f64>() / k;
        kept.iter().map(|z| (z - m) * (z - m)).sum::<f64>() / (k * k)
    };
    let best = (0..=4).min_by(|a, b| mser(*a).total_cmp(&mser(*b)));
    assert_eq!(best, Some(2));
    assert_eq!(s["warmup"]["status"], "found");
    assert_eq!(s["warmup"]["observations"], 10);
    // The first time kept.
    assert_eq!(s["warmup"]["time"], 11.0);

    // Fewer than 20 times: too few for MSER-5.
    let short: String = STEP
        .lines()
        .filter(|l| {
            let time = l.split(',').next().and_then(|t| t.parse::<u32>().ok());
            time.is_none_or(|t| t <= 15)
        })
        .map(|l| format!("{l}\n"))
        .collect();
    let r = ok("data.warmup", json!({ "csv": short }));
    assert_eq!(r["series"][0]["warmup"]["status"], "too_few");
}

#[test]
fn every_error_says_what_to_fix() {
    let bad = |command: &str, params: Value, says: &str| {
        let (code, message) = err(command, params);
        assert_eq!(code, ErrorCode::InvalidParams, "{message}");
        assert!(message.contains(says), "{says}: {message}");
    };
    let a = |csv: &str| json!({ "csv": csv });
    // The table itself.
    bad("data.analyse", a(""), "empty");
    bad("data.analyse", a("wait\n"), "no rows");
    bad(
        "data.analyse",
        a("wait,wait\n1,2\n"),
        "two columns are headed `wait`",
    );
    bad("data.analyse", a("wait\n\"1\n"), "not closed");
    bad("data.analyse", a("a,b\n1,2,3\n"), "line 2: 3 cells");
    bad("data.analyse", a("note\nx\n"), "no column of numbers");
    // A cell that is not a number, by line and column.
    bad(
        "data.analyse",
        a("replication,wait\n1,4\n2,abc\n"),
        "line 3, column `wait`: `abc`",
    );
    bad(
        "data.analyse",
        a("replication,wait\n1,4\n2,inf\n"),
        "line 3, column `wait`",
    );
    // Roles.
    bad(
        "data.analyse",
        a("replication,wait\n1,4\n1,5\n"),
        "replication `1` appears twice",
    );
    bad(
        "data.analyse",
        a("scenario,wait\nx,4\n,5\n"),
        "line 3: the `scenario` cell is empty",
    );
    bad(
        "data.analyse",
        json!({ "csv": WIDE, "scenario": "desk" }),
        "no column `desk`",
    );
    bad(
        "data.analyse",
        json!({ "csv": WIDE, "measures": ["scenario"] }),
        "not a measure",
    );
    bad(
        "data.analyse",
        json!({ "csv": WIDE, "measure": "wait" }),
        "long table",
    );
    bad(
        "data.analyse",
        json!({ "csv": LONG, "measures": ["queue"] }),
        "no measure `queue`",
    );
    bad(
        "data.analyse",
        a("scenario,replication,measure,value\nx,1,wait,1\nx,1,wait,2\n"),
        "second value for `wait`",
    );
    // The baseline, pairing, tolerances and targets.
    bad(
        "data.analyse",
        json!({ "csv": WIDE, "baseline": "three" }),
        "no scenario `three`",
    );
    bad(
        "data.analyse",
        json!({ "csv": SPLIT, "baseline": "x" }),
        "no scenario column",
    );
    bad(
        "data.analyse",
        json!({ "csv": "scenario,wait\nx,1\nx,2\ny,3\ny,4\n", "paired": true }),
        "needs a replication column",
    );
    let odd = WIDE.replace("two desks,4,", "two desks,6,");
    bad(
        "data.analyse",
        json!({ "csv": odd, "paired": true }),
        "same replication ids",
    );
    bad(
        "data.analyse",
        json!({ "csv": WIDE, "tolerance": { "queue": 1.0 } }),
        "not a measure",
    );
    bad(
        "data.analyse",
        json!({ "csv": WIDE, "tolerance": { "wait": 0.0 } }),
        "above 0",
    );
    bad(
        "data.analyse",
        json!({ "csv": WIDE, "targets": { "wait": { "relative": -1.0 } } }),
        "not above 0",
    );
    bad(
        "data.analyse",
        json!({ "csv": WIDE, "targets": { "queue": { "points": 1.0 } } }),
        "not a measure",
    );
    bad(
        "data.analyse",
        json!({ "csv": WIDE, "colour": "red" }),
        "unknown field",
    );

    // Time series.
    bad(
        "data.warmup",
        a("clock,replication,q\n1,1,2\n"),
        "which column is the time",
    );
    bad(
        "data.warmup",
        a("time,q\n1,2\n"),
        "which column is the replication",
    );
    bad(
        "data.warmup",
        a("time,replication,q\n1,1,2\n2,1,\n"),
        "every replication needs a value",
    );
    bad(
        "data.warmup",
        a("time,replication,q\n2,1,2\n1,1,3\n"),
        "time order",
    );
    bad(
        "data.warmup",
        a("time,replication,q\n,1,2\n"),
        "the `time` cell is empty",
    );
    let gap: String = STEP
        .lines()
        .filter(|l| *l != "40,2,0.5")
        .map(|l| format!("{l}\n"))
        .collect();
    assert!(gap.len() < STEP.len());
    bad(
        "data.warmup",
        json!({ "csv": gap }),
        "recorded at other times",
    );
}

#[test]
fn a_precision_target_of_its_own_per_measure() {
    let r = ok(
        "data.analyse",
        json!({ "csv": WIDE, "target": { "half_width": 2.5 }, "targets": { "served": { "points": 1.0 } } }),
    );
    let a = result(&r, "one desk", "wait");
    // 1.96 within ±2.5 in the measure's own unit.
    assert_eq!(a["precision"]["target"], json!({ "half_width": 2.5 }));
    assert_eq!(a["precision"]["status"], "settled");
    assert!(close(
        f(&a["precision"]["score"]),
        2.776 * (2.5_f64 / 5.0).sqrt() / 2.5
    ));
    assert_eq!(
        result(&r, "one desk", "served")["precision"]["target"],
        json!({ "points": 1.0 })
    );
}
