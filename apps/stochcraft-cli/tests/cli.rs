//! The `stochcraft-cli` binary, run as a person or a script would run it.

#![allow(clippy::panic)] // A command that cannot be started stops the test.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stochcraft-cli"))
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("cannot run stochcraft-cli: {e}"))
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn ok(args: &[&str]) -> String {
    let o = cli(args);
    assert!(o.status.success(), "{args:?}: {}", stderr(&o));
    stdout(&o)
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("not JSON ({e}):\n{text}"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli-tests");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

fn model(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/models")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

fn metric(results: &Value, element: &str, metric: &str) -> Value {
    results["elements"]
        .as_array()
        .and_then(|es| es.iter().find(|e| e["id"] == element))
        .map(|e| e["metrics"][metric].clone())
        .unwrap_or_default()
}

fn covers(m: &Value, theory: f64) -> bool {
    let (mean, hw) = (
        m["mean"].as_f64().unwrap_or(f64::NAN),
        m["half_width"].as_f64().unwrap_or(0.0),
    );
    (mean - theory).abs() <= hw
}

#[test]
fn every_template_is_written_validated_and_run() {
    // The names `new --list` gives, so a template added later is covered too.
    let list = ok(&["new", "--list"]);
    let names: Vec<&str> = list
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .collect();
    assert!(names.len() >= 10, "{list}");
    for name in names {
        let path = scratch(&format!("{name}.bpmn"));
        let p = path.to_string_lossy();
        ok(&["new", "--template", name, &p, "--force"]);
        let v = ok(&["validate", &p]);
        assert!(v.contains("0 errors"), "{v}");
        let r = json(&ok(&["run", &p, "--reps", "4", "--format", "json"]));
        assert_eq!(r["replications"], 4, "{name}");
        assert_eq!(r["record"]["file"], p.as_ref());
        // No behaviour check (stochcraft_blocks::checks) finds anything wrong in its results.
        let behaviour = [
            "unbalanced",
            "unaccounted",
            "nothing_out",
            "littles_law",
            "busy_not_working",
            "states_total",
            "out_of_range",
        ];
        let found: Vec<&Value> = r["warnings"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|w| behaviour.contains(&w["code"].as_str().unwrap_or("")))
            .collect();
        assert!(found.is_empty(), "{name}: {found:?}");
    }
}

/// The behaviour checks' warnings are printed with the results: a model whose entities never
/// get through to its exit in the run says so.
#[test]
fn a_run_prints_what_the_behaviour_checks_find() {
    let path = scratch("never-out.bpmn");
    let p = path.to_string_lossy();
    ok(&["new", "--template", "mm1", &p, "--force"]);
    let table = ok(&[
        "run",
        &p,
        "--reps",
        "2",
        "--warmup",
        "100",
        "--duration",
        "0.000001",
    ]);
    assert!(table.contains("No entity left the model"), "{table}");
}

#[test]
fn the_mm1_template_matches_queueing_theory() {
    let path = scratch("theory-mm1.bpmn");
    let p = path.to_string_lossy();
    ok(&["new", "--template", "mm1", &p, "--force"]);
    let r = json(&ok(&["run", &p, "--reps", "16", "--format", "json"]));
    // λ = 0.75, μ = 1: W = 4, Wq = 3, ρ = 0.75.
    for (element, m, theory) in [
        ("exit", "mean_time_in_system", 4.0),
        ("server", "mean_wait", 3.0),
        ("server", "utilisation", 0.75),
    ] {
        let got = metric(&r, element, m);
        assert!(covers(&got, theory), "{element} {m}: {got} vs {theory}");
    }
}

#[test]
fn results_do_not_depend_on_threads_and_follow_the_seed() {
    let m = model("mm1.bpmn");
    let run = |extra: &[&str]| {
        let mut args = vec![
            "run",
            m.as_str(),
            "--reps",
            "6",
            "--duration",
            "2000",
            "--format",
            "json",
        ];
        args.extend_from_slice(extra);
        json(&ok(&args))
    };
    let one = run(&["--threads", "1"]);
    assert_eq!(one, run(&["--threads", "4"]));
    let other = run(&["--seed", "99"]);
    assert_ne!(one["elements"], other["elements"]);
    assert_eq!(other["record"]["seed"], 99);
    assert_eq!(one["record"]["duration"], 2000.0);
    assert_eq!(one["record"]["scenario"], "base");
    // The scenario's own warm-up (PT1000M) is kept unless overridden.
    assert_eq!(one["record"]["warmup"], 1000.0);
}

#[test]
fn run_gives_the_warm_up_suggestion_and_its_warnings() {
    let m = model("mm1.bpmn");
    let table = ok(&["run", &m, "--reps", "3"]);
    assert!(
        table.contains("warm-up suggested (MSER-5 on the number inside): "),
        "{table}"
    );
    // One replication is warned about, in words.
    let one = ok(&["run", &m, "--reps", "1"]);
    assert!(
        one.to_lowercase().contains("the run has one replication"),
        "{one}"
    );
    // As JSON: the values over the run and the suggestion, beside the keys there were.
    let r = json(&ok(&["run", &m, "--reps", "3", "--format", "json"]));
    for key in ["elements", "values", "measured", "warnings", "record"] {
        assert!(!r[key].is_null(), "{key}");
    }
    assert_eq!(r["series"]["times"].as_array().map_or(0, Vec::len), 101);
    assert_eq!(r["warmup_suggestion"]["method"], "mser-5");
}

#[test]
fn table_csv_and_out() {
    let m = model("tandem.xpdl");
    let table = ok(&["run", &m, "--reps", "3"]);
    for expected in [
        "Tandem line",
        "scenario day",
        "3 replications, seed 7",
        "sha256:",
        "95% Student-t",
        "element",
        "± half-width",
        "mean_time_in_system",
    ] {
        assert!(table.contains(expected), "{expected} missing:\n{table}");
    }
    let csv = ok(&["run", &m, "--reps", "3", "--format", "csv"]);
    let mut lines = csv.lines();
    assert_eq!(
        lines.next(),
        Some(
            "element,name,kind,metric,mean,half_width,std_dev,n,\
             split_cut,split_low_n,split_low_mean,split_high_n,split_high_mean,split_group"
        )
    );
    assert!(lines.all(|l| l.split(',').count() == 14), "{csv}");
    let out = scratch("tandem.csv");
    let printed = ok(&[
        "run",
        &m,
        "--reps",
        "3",
        "--format",
        "csv",
        "--out",
        &out.to_string_lossy(),
    ]);
    assert!(printed.is_empty());
    assert_eq!(std::fs::read_to_string(&out).unwrap_or_default(), csv);
}

#[test]
fn validate_exits_with_1_on_errors() {
    // A model with a server and nothing else, written through an engine script.
    let broken = scratch("broken.bpmn");
    let script = scratch("make-broken.json");
    let _ = std::fs::write(
        &script,
        format!(
            r#"[{{"command": "block.add", "params": {{"kind": "server", "id": "lonely"}}}},
               {{"command": "file.save", "params": {{"path": {:?}}}}}]"#,
            broken.to_string_lossy()
        ),
    );
    ok(&["exec", &script.to_string_lossy()]);
    let o = cli(&["validate", &broken.to_string_lossy()]);
    assert_eq!(o.status.code(), Some(1), "{}", stdout(&o));
    assert!(stdout(&o).contains("no Arrivals block"), "{}", stdout(&o));
    let j = cli(&["validate", &broken.to_string_lossy(), "--format", "json"]);
    assert_eq!(j.status.code(), Some(1));
    assert!(json(&stdout(&j))["errors"].as_u64().is_some_and(|n| n > 0));
    // A check is a list of notes, not a table: there is no CSV of it.
    let c = cli(&["validate", &broken.to_string_lossy(), "--format", "csv"]);
    assert_eq!(c.status.code(), Some(2));
    assert!(stdout(&c).is_empty(), "{}", stdout(&c));
    assert!(stderr(&c).contains("table, json"), "{}", stderr(&c));
    // Running it says why it cannot run.
    let r = cli(&["run", &broken.to_string_lossy()]);
    assert_eq!(r.status.code(), Some(1));
    assert!(stderr(&r).contains("cannot run"), "{}", stderr(&r));
}

#[test]
fn exec_replays_scripts_step_by_step() {
    let script = scratch("edit.json");
    let _ = std::fs::write(
        &script,
        r#"[{"command": "block.set", "params": {"id": "serve", "capacity": 2}},
            {"command": "run", "params": {"replications": 2}},
            {"command": "undo"},
            {"command": "model.get"}]"#,
    );
    let out = ok(&[
        "exec",
        &script.to_string_lossy(),
        "--open",
        &model("mm1.bpmn"),
    ]);
    let steps: Vec<Value> = out.lines().map(json).collect();
    assert_eq!(steps.len(), 4);
    assert_eq!(steps[1]["result"]["record"]["unsaved_changes"], true);
    assert_eq!(steps[2]["result"]["undone"], "block.set");
    assert_eq!(steps[3]["result"]["elements"][1]["capacity"], 1);
    // A failing step stops the script with exit code 1, after reporting it.
    let _ = std::fs::write(
        &script,
        r#"[{"command": "block.remove", "params": {"id": "nope"}}, {"command": "model.get"}]"#,
    );
    let o = cli(&["exec", &script.to_string_lossy()]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stdout(&o).lines().count(), 1);
    assert!(stdout(&o).contains("not_found"));
}

#[test]
fn schema_lists_every_command() {
    let all = json(&ok(&["schema"]));
    let names: Vec<&str> = all
        .as_array()
        .map(|a| a.iter().filter_map(|c| c["name"].as_str()).collect())
        .unwrap_or_default();
    for n in ["block.add", "run", "file.save", "undo"] {
        assert!(names.contains(&n), "{names:?}");
    }
    let one = json(&ok(&["schema", "block.set"]));
    assert_eq!(one["params"]["type"], "object");
    assert_eq!(cli(&["schema", "nope"]).status.code(), Some(1));
}

#[test]
fn mistakes_are_reported_plainly() {
    let missing = cli(&["run", "no-such-file.bpmn"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        stderr(&missing).starts_with("error: no-such-file.bpmn"),
        "{}",
        stderr(&missing)
    );
    let path = scratch("exists.bpmn");
    let p = path.to_string_lossy();
    ok(&["new", &p, "--force"]);
    let again = cli(&["new", &p]);
    assert_eq!(again.status.code(), Some(1));
    assert!(stderr(&again).contains("--force"));
    assert_eq!(cli(&["new", "model.xpdl"]).status.code(), Some(1));
    assert_eq!(
        cli(&["new", "--template", "nope", "x.bpmn"]).status.code(),
        Some(1)
    );
    // Usage errors come from clap with its own exit code.
    assert_eq!(cli(&["run"]).status.code(), Some(2));
}

#[test]
fn design_runs_a_factorial_and_a_hypercube() {
    let m = model("mm1.bpmn");
    let factorial = |format: &str| {
        ok(&[
            "design",
            &m,
            "--factor",
            "arrive.mean=1.3333,2",
            "--factor",
            "serve.mean=0.8,1",
            "--result",
            "serve.mean_wait",
            "--result",
            "leave.mean_time_in_system",
            "--reps",
            "4",
            "--format",
            format,
        ])
    };
    let table = factorial("table");
    for expected in [
        "full factorial of 2 factors, 4 points × 4 replications = 16 runs, seed 2026",
        "arrive.mean",
        "serve.mean_wait",
        "± half-width",
        "main effects",
        "two-way interactions",
        "arrive.mean × serve.mean",
        "6 effects, each tested at 95% on its own",
    ] {
        assert!(table.contains(expected), "{expected} missing:\n{table}");
    }
    let r = json(&factorial("json"));
    assert_eq!(r["points"].as_array().map(Vec::len), Some(4));
    assert_eq!(r["points"][1]["values"], serde_json::json!([1.3333, 1.0]));
    // Faster arrivals (a shorter time between them) wait longer: a negative main effect.
    let arrive = r["main_effects"]
        .as_array()
        .and_then(|m| {
            m.iter()
                .find(|e| e["id"] == "arrive" && e["element"] == "serve")
        })
        .cloned()
        .unwrap_or_default();
    assert!(arrive["effect"].as_f64().unwrap_or(0.0) < 0.0, "{arrive}");
    let csv = factorial("csv");
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "point,arrive.mean,serve.mean,element,metric,mean,half_width,n"
    );
    assert_eq!(lines.len(), 1 + 4 * 2, "{csv}");
    assert!(
        lines[1].starts_with("1,1.3333,0.8,serve,mean_wait,"),
        "{csv}"
    );
    assert!(lines.iter().all(|l| l.split(',').count() == 8), "{csv}");

    let hypercube = || {
        ok(&[
            "design",
            &m,
            "--design",
            "latin-hypercube",
            "--points",
            "5",
            "--factor",
            "arrive.mean=1.5..3",
            "--factor",
            "serve.capacity=1..3",
            "--result",
            "serve.mean_wait",
            "--reps",
            "2",
        ])
    };
    let text = hypercube();
    assert!(
        text.contains("Latin hypercube of 2 factors, 5 points"),
        "{text}"
    );
    assert!(text.contains("spearman"), "{text}");
    assert_eq!(hypercube(), text);
}

#[test]
fn a_design_too_large_or_mistyped_is_refused() {
    let m = model("mm1.bpmn");
    let big = cli(&[
        "design",
        &m,
        "--factor",
        "arrive.mean=1.5,3",
        "--result",
        "serve.mean_wait",
        "--reps",
        "6000",
    ]);
    assert_eq!(big.status.code(), Some(1));
    assert!(
        stderr(&big).contains("12000 runs") && stderr(&big).contains("the limit is 10000"),
        "{}",
        stderr(&big)
    );
    let typo = cli(&[
        "design",
        &m,
        "--factor",
        "arrive=1,2",
        "--result",
        "serve.mean_wait",
    ]);
    assert_eq!(typo.status.code(), Some(1));
    assert!(
        stderr(&typo).contains("id.field=1,2,3"),
        "{}",
        stderr(&typo)
    );
    assert_eq!(
        cli(&["design", &m, "--result", "serve.mean_wait"])
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn fit_ranks_families_for_a_column_of_measured_times() {
    // 40 evenly spread values on [1, 4] in the second column, with a header and an empty cell.
    let mut csv = String::from("order,\"service time\"\n");
    for i in 0..40 {
        csv.push_str(&format!(
            "{i},{}\n",
            1.0 + 3.0 * (f64::from(i) + 0.5) / 40.0
        ));
    }
    csv.push_str("40,\n");
    let path = scratch("times.csv");
    let _ = std::fs::write(&path, &csv);
    let p = path.to_string_lossy();

    let table = ok(&["fit", &p, "--column", "service time"]);
    assert!(
        table.starts_with(&format!(
            "40 values from {p}, column service time: mean 2.5"
        )),
        "{table}"
    );
    assert!(table.contains("1 empty cell left out"), "{table}");
    assert!(table.contains("Anderson–Darling"), "{table}");
    let first = table
        .lines()
        .find(|l| l.split_whitespace().next() == Some("1") && !l.contains("empty"))
        .unwrap_or_default();
    assert!(first.contains("uniform"), "{table}");
    assert!(table.contains("\"type\":\"uniform\""), "{table}");

    let r = json(&ok(&["fit", &p, "-c", "2", "--format", "json"]));
    assert_eq!(r["count"], 40);
    assert_eq!(r["candidates"][0]["family"], "uniform");
    assert_eq!(r["best"]["type"], "uniform");

    let r = json(&ok(&[
        "fit",
        &p,
        "-c",
        "2",
        "--families",
        "gamma,lognormal",
        "--format",
        "json",
    ]));
    let families: Vec<&str> = r["candidates"]
        .as_array()
        .map(|a| a.iter().filter_map(|c| c["family"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(families.len(), 2, "{r}");
    assert!(families.contains(&"gamma") && families.contains(&"log_normal"));

    let rows = ok(&["fit", &p, "-c", "2", "--format", "csv"]);
    assert!(rows.starts_with("rank,family,anderson_darling"), "{rows}");
    assert_eq!(rows.lines().count(), 9, "{rows}"); // A header and eight families.

    // A plain list on standard input.
    let mut child = Command::new(env!("CARGO_BIN_EXE_stochcraft-cli"))
        .args(["fit", "-", "--format", "json"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run stochcraft-cli: {e}"));
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write as _;
        let list: String = (1..=25)
            .map(|i| format!("{}\n", f64::from(i) / 10.0))
            .collect();
        let _ = stdin.write_all(list.as_bytes());
    }
    let out = child
        .wait_with_output()
        .unwrap_or_else(|e| panic!("stochcraft-cli: {e}"));
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(json(&stdout(&out))["count"], 25);
}

#[test]
fn fit_refuses_too_few_values_and_data_no_family_fits() {
    let path = scratch("few.csv");
    let _ = std::fs::write(&path, "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
    let p = path.to_string_lossy();
    let few = cli(&["fit", &p]);
    assert_eq!(few.status.code(), Some(1));
    assert!(stderr(&few).contains("at least 20"), "{}", stderr(&few));
    // Ten are enough when asked for.
    ok(&["fit", &p, "--min-count", "10"]);

    let path = scratch("zeros.csv");
    let list: String = (0..30).map(|i| format!("{i}\n")).collect();
    let _ = std::fs::write(&path, list);
    let p = path.to_string_lossy();
    let none = cli(&["fit", &p, "--families", "weibull,gamma"]);
    assert_eq!(none.status.code(), Some(1));
    assert!(
        stderr(&none).contains("needs every value above 0"),
        "{}",
        stderr(&none)
    );
    let path = scratch("two-columns.csv");
    let _ = std::fs::write(&path, "a,b\n1,2\n");
    let two = cli(&["fit", &path.to_string_lossy()]);
    assert_eq!(two.status.code(), Some(1));
    assert!(stderr(&two).contains("--column"), "{}", stderr(&two));
}

#[test]
fn explain_gives_the_capacity_table_without_running() {
    // M/M/1 at λ = 0.75, μ = 1: 75%, a wait of 3.
    let text = ok(&["explain", &model("mm1.bpmn")]);
    assert!(text.contains("M/M/1"), "{text}");
    assert!(text.contains("75%"), "{text}");
    assert!(text.contains("Bottleneck: Serve customer"), "{text}");
    let j = json(&ok(&["explain", &model("mm1.bpmn"), "--format", "json"]));
    let row = &j["stations"][0];
    assert_eq!(row["status"], "fine");
    assert!((row["formula"]["mean_wait"].as_f64().unwrap_or(0.0) - 3.0).abs() < 1e-9);
    // At a target of 70%, the same station is congested and needs a second unit.
    let j = json(&ok(&[
        "explain",
        &model("mm1.bpmn"),
        "--target",
        "0.7",
        "--format",
        "json",
    ]));
    assert_eq!(j["stations"][0]["status"], "congested");
    assert_eq!(j["stations"][0]["units_for_target"], 2);
    // Blocking: Weld can be held up by Paint's full room, and says so.
    let text = ok(&["explain", &model("weld-paint.bpmn")]);
    assert!(text.contains("can be held up by Paint"), "{text}");
    assert!(
        text.contains("Bottleneck: not known before a run"),
        "{text}"
    );
    for bad in ["1.5", "NaN", "inf", "0"] {
        let o = cli(&["explain", &model("mm1.bpmn"), "--target", bad]);
        assert_eq!(o.status.code(), Some(2), "{bad}");
        assert!(stderr(&o).contains("between 0 and 1"), "{}", stderr(&o));
    }
    // Tasks sharing a pool say why they have no formula.
    let text = ok(&["explain", &model("triage.bpmn")]);
    assert!(text.contains("Standard: the pool is shared"), "{text}");
}

#[test]
fn explain_refuses_a_model_with_errors() {
    let broken = scratch("broken-explain.bpmn");
    let script = scratch("make-broken-explain.json");
    let _ = std::fs::write(
        &script,
        format!(
            r#"[{{"command": "block.add", "params": {{"kind": "server", "id": "lonely"}}}},
               {{"command": "file.save", "params": {{"path": {:?}}}}}]"#,
            broken.to_string_lossy()
        ),
    );
    ok(&["exec", &script.to_string_lossy()]);
    let o = cli(&["explain", &broken.to_string_lossy()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("cannot be explained"), "{}", stderr(&o));
    assert!(stdout(&o).is_empty(), "{}", stdout(&o));
}

#[test]
fn balance_solves_groover_csv_and_reports_bad_rows() {
    let csv = "\
id,time,after
1,0.2,
2,0.4,
3,0.7,1
4,0.1,1;2
5,0.3,2
6,0.11,3
7,0.32,3
8,0.6,3;4
9,0.27,6;7;8
10,0.38,5;8
11,0.5,9;10
12,0.12,11
";
    let path = scratch("groover-15-1.csv");
    let _ = std::fs::write(&path, csv);
    let p = path.to_string_lossy();

    let table = ok(&["balance", &p, "--cycle", "1"]);
    assert!(table.contains("theoretical minimum 4"), "{table}");
    assert!(table.contains("Largest candidate rule"), "{table}");
    assert!(table.contains("0.80"), "{table}");

    let j = json(&ok(&[
        "balance",
        &p,
        "--cycle",
        "1",
        "--heuristic",
        "largest_candidate",
        "--format",
        "json",
    ]));
    assert_eq!(j["theoretical_min"], 4);
    assert_eq!(j["balances"].as_array().map(Vec::len), Some(1));
    assert_eq!(j["balances"][0]["heuristic"], "largest_candidate");
    assert_eq!(
        j["balances"][0]["stations"].as_array().map(Vec::len),
        Some(5)
    );

    // Blank lines are ignored and fields trimmed, while a bad row exits with 2 and names the line.
    let bad = scratch("bad-balance.csv");
    let _ = std::fs::write(&bad, "id, time, after\n\n1 , 0.2 , \n2, nope, 1\n");
    let bad_p = bad.to_string_lossy();
    let o = cli(&["balance", &bad_p, "--cycle", "1"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("line 4"), "{}", stderr(&o));
    assert!(stdout(&o).is_empty(), "{}", stdout(&o));

    // Missing column and precedence loop also exit with 2.
    let short = scratch("short-balance.csv");
    let _ = std::fs::write(&short, "id,time,after\n1,0.2\n");
    let o = cli(&["balance", &short.to_string_lossy(), "--cycle", "1"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("line 2"), "{}", stderr(&o));

    let cyc = scratch("loop-balance.csv");
    let _ = std::fs::write(&cyc, "id,time,after\nA,0.5,B\nB,0.5,A\n");
    let o = cli(&["balance", &cyc.to_string_lossy(), "--cycle", "1"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains('A'), "{}", stderr(&o));

    let o = cli(&["balance", &p, "--cycle", "0"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
}

#[test]
fn a_journal_replays_from_the_scenario_it_was_recorded_from() {
    // mm1's second scenario has two clerks, so its model (and digest) differs from the first's.
    let mm1 = model("mm1.bpmn");
    let script = scratch("journal-scenario.json");
    let journal = scratch("journal-scenario-out.json");
    let open_second = serde_json::json!([
        {"command": "file.open", "params": {"path": mm1, "scenario": "two-clerks"}},
        {"command": "block.set", "params": {"id": "serve", "capacity": 3}},
    ]);
    let _ = std::fs::write(&script, open_second.to_string());
    ok(&[
        "exec",
        &script.to_string_lossy(),
        "--journal",
        &journal.to_string_lossy(),
    ]);
    let written = json(&std::fs::read_to_string(&journal).unwrap_or_default());
    assert_eq!(written["start"]["scenario"], "two-clerks", "{written}");

    // Replayed with --open on the same file: the recorded scenario is opened, so the digest
    // matches and the edit is applied.
    let out = ok(&["exec", &journal.to_string_lossy(), "--open", &mm1]);
    let steps: Vec<Value> = out.lines().map(json).collect();
    assert!(
        steps.last().is_some_and(|s| s["result"].is_object()),
        "{out}"
    );

    // A file without that scenario: refused, unless forced, which replays onto its first one.
    let other = scratch("journal-scenario-other.bpmn");
    let text = std::fs::read_to_string(&mm1).unwrap_or_default();
    assert!(text.contains("id=\"two-clerks\""));
    let _ = std::fs::write(
        &other,
        text.replace("id=\"two-clerks\"", "id=\"three-clerks\""),
    );
    let other = other.to_string_lossy();
    let o = cli(&["exec", &journal.to_string_lossy(), "--open", &other]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    let out = ok(&[
        "exec",
        &journal.to_string_lossy(),
        "--open",
        &other,
        "--force",
    ]);
    let steps: Vec<Value> = out.lines().map(json).collect();
    assert!(
        steps.last().is_some_and(|s| s["result"].is_object()),
        "{out}"
    );
}

#[test]
fn exec_writes_the_journal_of_what_it_did_and_replays_it_safely() {
    let script = scratch("journal-edit.json");
    let journal = scratch("journal-out.json");
    let _ = std::fs::write(
        &script,
        r#"[{"command": "block.set", "params": {"id": "serve", "capacity": 2}},
            {"command": "run", "params": {"replications": 2}}]"#,
    );
    let mm1 = model("mm1.bpmn");
    ok(&[
        "exec",
        &script.to_string_lossy(),
        "--open",
        &mm1,
        "--journal",
        &journal.to_string_lossy(),
    ]);
    // The journal records its version and where it started, then the steps.
    let written = json(&std::fs::read_to_string(&journal).unwrap_or_default());
    assert_eq!(written["format"], 1);
    assert_eq!(written["stochcraft"], env!("CARGO_PKG_VERSION"));
    assert_eq!(written["start"]["kind"], "file");
    assert!(
        written["start"]["file_digest"]
            .as_str()
            .is_some_and(|d| d.starts_with("sha256:"))
    );
    assert_eq!(written["entries"].as_array().map(Vec::len), Some(2));
    assert_eq!(written["entries"][0]["command"], "block.set");

    // Replayed on the same file: the edit is applied, the run is skipped.
    let out = ok(&["exec", &journal.to_string_lossy(), "--open", &mm1]);
    let steps: Vec<Value> = out.lines().map(json).collect();
    assert_eq!(steps.len(), 2);
    assert!(steps[0]["result"].is_object());
    assert_eq!(steps[1]["skipped"], true);
    // With --all the run is carried out too.
    let out = ok(&["exec", &journal.to_string_lossy(), "--open", &mm1, "--all"]);
    let steps: Vec<Value> = out.lines().map(json).collect();
    assert_eq!(steps[1]["result"]["replications"], 2);
    // Without the file it started from, it is refused, and nothing is run.
    let o = cli(&["exec", &journal.to_string_lossy()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("starts from"), "{}", stderr(&o));
    assert_eq!(stdout(&o), "");
}

/// `--until`, `--until-points` and `--max` run sequentially: the results say how many
/// replications ran, after which looks and why, and they are those of a fixed run of that many.
#[test]
fn run_until_a_target_half_width() {
    let m = model("mm1.bpmn");
    let base = [
        "run",
        m.as_str(),
        "--reps",
        "4",
        "--duration",
        "2000",
        "--format",
        "json",
    ];
    let mut args = base.to_vec();
    args.extend(["--until", "10%", "--max", "40"]);
    let r = json(&ok(&args));
    let n = r["sequential"]["replications"].as_u64().unwrap_or(0);
    assert!((4..=40).contains(&n), "{}", r["sequential"]);
    assert_eq!(r["replications"], n);
    assert_eq!(r["record"]["replications"], n);
    assert_eq!(r["sequential"]["stages"][0], 4);
    assert_eq!(r["record"]["sequential"]["relative"], 0.1);
    assert_eq!(r["record"]["sequential"]["max"], 40);
    let uncertainty = r["record"]["uncertainty"].as_str().unwrap_or("");
    assert!(uncertainty.starts_with("estimated precision after adaptive stopping"));
    // A fixed run of that many gives the same numbers.
    let reps = n.to_string();
    let fixed = [
        "run",
        m.as_str(),
        "--reps",
        reps.as_str(),
        "--duration",
        "2000",
        "--format",
        "json",
    ];
    let f = json(&ok(&fixed));
    assert_eq!(f["elements"], r["elements"]);
    assert!(f["sequential"].is_null());
    // In words.
    let table = ok(&[
        "run",
        &m,
        "--reps",
        "4",
        "--duration",
        "2000",
        "--until",
        "0.1",
        "--max",
        "40",
    ]);
    assert!(table.contains("\nsequential: "), "{table}");
    assert!(table.contains("estimated precision"), "{table}");
    // A share of the mean is a percentage or a fraction below 1.
    for bad in ["5", "0%", "-3%", "x"] {
        let o = cli(&["run", &m, "--until", bad]);
        assert!(!o.status.success(), "--until {bad}");
    }
    // `--fixed` cannot be given with them.
    assert!(!cli(&["run", &m, "--fixed", "--max", "20"]).status.success());
}
