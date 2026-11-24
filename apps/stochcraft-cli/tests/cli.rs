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
            "nothing_moved",
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

    let sobol = || {
        ok(&[
            "design",
            &m,
            "--design",
            "sobol",
            "--points",
            "4",
            "--batches",
            "3",
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
    let text = sobol();
    assert!(
        text.contains("Sobol design of 2 factors, 12 points"),
        "{text}"
    );
    assert!(
        text.contains("3 batches of 4 Sobol points") && text.contains("batch mean"),
        "{text}"
    );
    assert_eq!(sobol(), text);
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

/// A Monte Carlo model, written through an engine script: `run` finds it is one and runs its
/// trials, `validate` checks it, and `inputs` prints and reads its input table.
#[test]
fn a_monte_carlo_model_runs_from_the_command_line() {
    let file = scratch("cost.bpmn");
    let f = file.to_string_lossy().into_owned();
    let script = scratch("make-cost.json");
    let _ = std::fs::write(
        &script,
        format!(
            r#"[{{"command": "model.new", "params": {{"name": "Project cost"}}}},
               {{"command": "mc.input.set", "params": {{"name": "labour", "distribution": {{"type": "triangular", "min": 80, "mode": 100, "max": 150}}, "unit": "h"}}}},
               {{"command": "mc.input.set", "params": {{"name": "rate", "distribution": {{"type": "constant", "value": 45}}}}}},
               {{"command": "mc.output.set", "params": {{"name": "cost", "expression": "$labour * $rate", "unit": "USD", "at_most": 5000}}}},
               {{"command": "file.save", "params": {{"path": {f:?}}}}}]"#
        ),
    );
    ok(&["exec", &script.to_string_lossy()]);
    let v = ok(&["validate", &f]);
    assert!(v.contains("0 errors"), "{v}");

    let r = json(&ok(&[
        "run", &f, "--reps", "4000", "--seed", "2", "--format", "json",
    ]));
    assert_eq!(r["kind"], "monte_carlo");
    assert_eq!(r["trials"], 4000);
    assert_eq!(r["record"]["file"], f.as_str());
    // E[labour] × 45 = 110 × 45.
    let mean = r["outputs"][0]["mean"].as_f64().unwrap_or(f64::NAN);
    let hw = r["outputs"][0]["half_width"].as_f64().unwrap_or(f64::NAN);
    assert!((mean - 4_950.0).abs() < 2.6 * hw, "{mean} ± {hw}");

    let table = ok(&["run", &f, "--reps", "4000", "--seed", "2"]);
    assert!(
        table.contains("Monte Carlo: 4000 trials, seed 2"),
        "{table}"
    );
    assert!(table.contains("cost is at most 5000 USD in"), "{table}");
    assert!(
        table.contains("intervals: Student-t across 4000 independent trials"),
        "{table}"
    );
    // A Latin hypercube run is whole batches, judged across them, and says so.
    let lhs = ok(&["exec", &{
        let script = scratch("make-cost-lhs.json");
        let _ = std::fs::write(
            &script,
            format!(
                r#"[{{"command": "file.open", "params": {{"path": {f:?}}}}},
                       {{"command": "mc.set", "params": {{"sampling": "latin_hypercube"}}}},
                       {{"command": "file.save", "params": {{}}}}]"#
            ),
        );
        script.to_string_lossy().into_owned()
    }]);
    assert!(!lhs.is_empty());
    let table = ok(&["run", &f, "--reps", "2500", "--seed", "2"]);
    assert!(table.contains("Monte Carlo: 3000 trials"), "{table}");
    assert!(
        table.contains("2500 trials asked for, rounded up to whole batches of 1000"),
        "{table}"
    );
    assert!(
        table.contains(
            "intervals: Student-t across 3 independent Latin hypercube batches of 1000 trials"
        ),
        "{table}"
    );
    ok(&["exec", &{
        let script = scratch("make-cost-random.json");
        let _ = std::fs::write(
            &script,
            format!(
                r#"[{{"command": "file.open", "params": {{"path": {f:?}}}}},
                       {{"command": "mc.set", "params": {{"sampling": "random"}}}},
                       {{"command": "file.save", "params": {{}}}}]"#
            ),
        );
        script.to_string_lossy().into_owned()
    }]);
    // `--sampling` changes how one run draws, whatever the file says: antithetic pairs are
    // judged across pairs, Sobol points across randomised batches.
    let table = ok(&[
        "run",
        &f,
        "--reps",
        "4001",
        "--seed",
        "2",
        "--sampling",
        "antithetic",
    ]);
    assert!(
        table.contains("Monte Carlo: 4002 trials, seed 2, antithetic sampling"),
        "{table}"
    );
    assert!(
        table.contains("intervals: Student-t across 2001 independent antithetic pairs of trials"),
        "{table}"
    );
    let table = ok(&[
        "run",
        &f,
        "--reps",
        "2500",
        "--seed",
        "2",
        "--sampling",
        "quasi-random",
    ]);
    assert!(table.contains("Monte Carlo: 3000 trials"), "{table}");
    assert!(
        table.contains("intervals: Student-t across 3 independent randomised Sobol batches"),
        "{table}"
    );
    // `sobol` is accepted as another name for it.
    let alias = ok(&[
        "run",
        &f,
        "--reps",
        "2500",
        "--seed",
        "2",
        "--sampling",
        "sobol",
    ]);
    assert_eq!(alias, table);
    let csv = ok(&[
        "run", &f, "--reps", "4000", "--seed", "2", "--format", "csv",
    ]);
    assert!(csv.starts_with("output,unit,trials,"), "{csv}");
    assert!(csv.contains("\ncost,USD,4000,4000,"), "{csv}");
    // Sequential: until the mean is within ±0.5%.
    let s = json(&ok(&[
        "run", &f, "--reps", "1000", "--until", "0.5%", "--format", "json",
    ]));
    assert_eq!(s["stop"]["reason"], "target_met", "{}", s["stop"]);
    // A process run's options that mean nothing here are refused.
    assert!(!cli(&["run", &f, "--warmup", "10"]).status.success());

    // The input table out, changed, and back in.
    let inputs = ok(&["inputs", &f]);
    assert!(
        inputs.contains("labour,triangular,min=80;mode=100;max=150,h"),
        "{inputs}"
    );
    let table_file = scratch("cost-inputs.csv");
    let _ = std::fs::write(
        &table_file,
        "name,distribution,parameters,unit\nrate,uniform,min=40;max=50,USD/h\n",
    );
    ok(&["inputs", &f, "--import", &table_file.to_string_lossy()]);
    let after = ok(&["inputs", &f]);
    assert!(
        after.contains("rate,uniform,min=40;max=50,USD/h"),
        "{after}"
    );
    assert!(after.contains("labour,triangular"), "{after}");
}

/// A system dynamics model (an XMILE file): `run` prints each variable's value at the end, CSV
/// and JSON give every variable over time, and `--dt-check` reports the halved-step check.
#[test]
fn a_system_dynamics_model_runs_from_the_command_line() {
    let m = model("sir.xmile");
    let table = ok(&["run", &m]);
    assert!(
        table.contains("system dynamics, rk4, time step 0.25 Days"),
        "{table}"
    );
    assert!(
        table.contains("one run (the model has no random function)"),
        "{table}"
    );
    assert!(table.contains("Susceptible"), "{table}");

    let csv = ok(&["run", &m, "--format", "csv"]);
    assert_eq!(csv.lines().count(), 12, "{csv}");
    assert!(
        csv.starts_with("time (Days),Susceptible,Infected,Recovered"),
        "{csv}"
    );

    let r = json(&ok(&[
        "run",
        &m,
        "--format",
        "json",
        "--dt",
        "1",
        "--dt-check",
    ]));
    assert_eq!(r["dt"], 1.0);
    assert_eq!(r["dt_check"]["half_dt"], 0.5);
    assert!(r["dt_check"]["stocks"].is_array());
    let checked = ok(&["run", &m, "--dt", "1", "--dt-check"]);
    assert!(checked.contains("halved step"), "{checked}");

    // What means nothing for such a model is refused, not ignored.
    let refused = cli(&["run", &m, "--warmup", "5"]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("system dynamics"),
        "{}",
        stderr(&refused)
    );
    assert!(!cli(&["run", &m, "--dt", "0"]).status.success());
}

/// The M/M/1 model with its service time replaced by a processing-time expression, which the
/// reader cannot use (it reads as a time of zero and notes an error), written to a scratch file.
fn expression_model(name: &str) -> String {
    let text = std::fs::read_to_string(model("mm1.bpmn")).unwrap_or_default();
    let altered = text.replacen(
        "<bpsim:NegativeExponentialDistribution mean=\"1\"/>",
        "<bpsim:ExpressionParameter value=\"1 + 9\"/>",
        1,
    );
    assert_ne!(text, altered, "the mm1 fixture changed");
    let path = scratch(name);
    std::fs::write(&path, altered).unwrap_or_else(|e| panic!("cannot write a test file: {e}"));
    path.to_string_lossy().into_owned()
}

#[test]
fn a_model_read_with_errors_does_not_run_in_any_format() {
    let file = expression_model("expression.bpmn");
    for format in ["table", "json", "csv"] {
        let o = cli(&["run", &file, "--reps", "1", "--format", format]);
        assert_eq!(o.status.code(), Some(1), "{format}: {}", stdout(&o));
        assert!(stdout(&o).is_empty(), "{format}: {}", stdout(&o));
        assert!(
            stderr(&o).contains("--allow-approximations"),
            "{format}: {}",
            stderr(&o)
        );
        assert!(stderr(&o).contains("serve"), "{format}: {}", stderr(&o));
    }
    let o = cli(&[
        "run",
        &file,
        "--reps",
        "1",
        "--format",
        "csv",
        "--out",
        &scratch("refused.csv").to_string_lossy(),
    ]);
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn allowing_approximations_runs_and_says_what_was_substituted() {
    let file = expression_model("expression-allowed.bpmn");
    let o = cli(&[
        "run",
        &file,
        "--reps",
        "1",
        "--format",
        "json",
        "--allow-approximations",
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).contains("approximation"), "{}", stderr(&o));
    assert!(stderr(&o).contains("`serve`"), "{}", stderr(&o));
    let r = json(&stdout(&o));
    let list = r["record"]["approximations"].as_array().cloned();
    assert!(!list.unwrap_or_default().is_empty(), "{}", r["record"]);
}

#[test]
fn a_model_read_cleanly_runs_without_the_flag_and_says_nothing_on_stderr() {
    let o = cli(&["run", &model("mm1.bpmn"), "--reps", "1", "--format", "csv"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).is_empty(), "{}", stderr(&o));
}

#[test]
fn output_is_never_the_model_it_was_made_from() {
    let copy = scratch("alias.bpmn");
    std::fs::copy(model("mm1.bpmn"), &copy)
        .unwrap_or_else(|e| panic!("cannot copy the model: {e}"));
    let before = std::fs::read(&copy).unwrap_or_default();
    let path = copy.to_string_lossy().into_owned();
    let o = cli(&[
        "run", &path, "--reps", "1", "--format", "csv", "--out", &path,
    ]);
    assert_eq!(o.status.code(), Some(1), "{}", stdout(&o));
    assert!(
        stderr(&o).contains("the output file is the input"),
        "{}",
        stderr(&o)
    );
    assert_eq!(std::fs::read(&copy).unwrap_or_default(), before);
}

#[test]
fn a_journal_is_never_written_over_the_model_or_the_script() {
    let copy = scratch("alias-journal.bpmn");
    std::fs::copy(model("mm1.bpmn"), &copy)
        .unwrap_or_else(|e| panic!("cannot copy the model: {e}"));
    let before = std::fs::read(&copy).unwrap_or_default();
    let path = copy.to_string_lossy().into_owned();
    // A script whose second step fails, so the journal would be written after a failure.
    let script = scratch("alias-journal.json");
    let text =
        r#"[{"command": "history", "params": {}}, {"command": "no.such.command", "params": {}}]"#;
    std::fs::write(&script, text).unwrap_or_else(|e| panic!("cannot write a test file: {e}"));
    let script_path = script.to_string_lossy().into_owned();

    let o = cli(&["exec", &script_path, "--open", &path, "--journal", &path]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("the output file is the input"),
        "{}",
        stderr(&o)
    );
    assert_eq!(
        std::fs::read(&copy).unwrap_or_default(),
        before,
        "the model was replaced"
    );

    let o = cli(&["exec", &script_path, "--journal", &script_path]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&script).unwrap_or_default(), text);

    // Another path is fine, and gets the journal of how far the script got.
    let journal = scratch("alias-journal-out.json");
    let o = cli(&[
        "exec",
        &script_path,
        "--open",
        &path,
        "--journal",
        &journal.to_string_lossy(),
    ]);
    assert_eq!(o.status.code(), Some(1));
    assert!(json(&std::fs::read_to_string(&journal).unwrap_or_default())["entries"].is_array());
}

#[test]
fn output_is_never_a_hard_link_to_the_model() {
    let copy = scratch("hard-linked.bpmn");
    let link = scratch("hard-linked-out.csv");
    let _ = std::fs::remove_file(&link);
    std::fs::copy(model("mm1.bpmn"), &copy)
        .unwrap_or_else(|e| panic!("cannot copy the model: {e}"));
    let before = std::fs::read(&copy).unwrap_or_default();
    std::fs::hard_link(&copy, &link).unwrap_or_else(|e| panic!("cannot link: {e}"));
    let (path, out) = (copy.to_string_lossy(), link.to_string_lossy());
    let o = cli(&[
        "run", &path, "--reps", "1", "--format", "csv", "--out", &out,
    ]);
    assert_eq!(o.status.code(), Some(1), "{}", stdout(&o));
    assert!(
        stderr(&o).contains("the output file is the input"),
        "{}",
        stderr(&o)
    );
    assert_eq!(std::fs::read(&copy).unwrap_or_default(), before);
    assert_eq!(std::fs::read(&link).unwrap_or_default(), before);
}

#[test]
fn inputs_out_is_never_the_model() {
    let copy = scratch("inputs-alias.bpmn");
    std::fs::copy(model("mm1.bpmn"), &copy)
        .unwrap_or_else(|e| panic!("cannot copy the model: {e}"));
    let before = std::fs::read(&copy).unwrap_or_default();
    let path = copy.to_string_lossy().into_owned();
    let o = cli(&["inputs", &path, "--out", &path]);
    assert_eq!(o.status.code(), Some(1), "{}", stdout(&o));
    assert!(
        stderr(&o).contains("the output file is the input"),
        "{}",
        stderr(&o)
    );
    assert_eq!(std::fs::read(&copy).unwrap_or_default(), before);
}

#[test]
fn a_journal_is_never_written_over_a_file_the_script_opened_earlier() {
    let first = scratch("opened-first.bpmn");
    let second = scratch("opened-second.bpmn");
    std::fs::copy(model("mm1.bpmn"), &first)
        .unwrap_or_else(|e| panic!("cannot copy the model: {e}"));
    std::fs::copy(model("mm1.bpmn"), &second)
        .unwrap_or_else(|e| panic!("cannot copy the model: {e}"));
    let before = std::fs::read(&first).unwrap_or_default();
    let steps: Vec<Value> = [&first, &second]
        .iter()
        .map(|p| serde_json::json!({ "command": "file.open", "params": { "path": p.to_string_lossy() } }))
        .collect();
    let script = scratch("opened-earlier.json");
    std::fs::write(&script, Value::Array(steps).to_string())
        .unwrap_or_else(|e| panic!("cannot write a test file: {e}"));
    let o = cli(&[
        "exec",
        &script.to_string_lossy(),
        "--journal",
        &first.to_string_lossy(),
    ]);
    assert_eq!(o.status.code(), Some(1), "{}", stdout(&o));
    assert!(
        stderr(&o).contains("the output file is the input"),
        "{}",
        stderr(&o)
    );
    assert_eq!(std::fs::read(&first).unwrap_or_default(), before);
}

/// The example model of the user guide's optimisers.
fn example(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/optimisers")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// `evaluate` with `input` on standard input: its lines, and whether it succeeded.
fn evaluate_lines(args: &[&str], input: &str) -> (Vec<Value>, bool) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_stochcraft-cli"))
        .arg("evaluate")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run stochcraft-cli: {e}"));
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write as _;
        let _ = stdin.write_all(input.as_bytes());
    }
    let out = child
        .wait_with_output()
        .unwrap_or_else(|e| panic!("stochcraft-cli: {e}"));
    let lines = stdout(&out).lines().map(json).collect();
    (lines, out.status.success())
}

/// `evaluate` gives exactly what the engine's `experiment.evaluate` gives, from arguments and
/// from JSON lines on standard input, one line out per line in; a refused line is answered with
/// its error, and the lines after it are still evaluated.
#[test]
fn evaluate_reads_and_writes_json_lines_and_matches_the_engine() {
    let file = example("mm1-cost.bpmn");
    let from_args = ok(&[
        "evaluate",
        &file,
        "--decide",
        "server.mean=0.4",
        "--range",
        "server.mean=0.2..0.9",
        "--result",
        "server.mean_work_in_progress",
        "--first",
        "3",
        "--count",
        "4",
        "--seed",
        "9",
    ]);
    assert_eq!(from_args.lines().count(), 1, "{from_args}");

    let mut engine = stochcraft_engine::Engine::new();
    let opened = engine.execute("file.open", serde_json::json!({ "path": file }));
    assert!(opened.is_ok(), "{opened:?}");
    let direct = engine.execute(
        "experiment.evaluate",
        serde_json::json!({
            "decisions": [{ "id": "server", "field": "mean", "value": 0.4, "min": 0.2, "max": 0.9 }],
            "results": [{ "element": "server", "metric": "mean_work_in_progress" }],
            "first": 3,
            "count": 4,
            "seed": 9,
        }),
    );
    let direct = direct.unwrap_or_else(|e| panic!("{e}"));
    // The same text, so the same numbers to the last bit.
    assert_eq!(from_args.trim_end(), direct.to_string());

    // The same evaluation as a line, then one refused, then the replications after the first
    // ones: the staged values are those of the whole.
    let line = |value: f64, first: u64, count: u64| {
        format!(
            "{{\"decisions\": [{{\"id\": \"server\", \"field\": \"mean\", \"value\": {value}}}], \"first\": {first}, \"count\": {count}}}\n"
        )
    };
    let input = [
        line(0.4, 3, 4),
        "\n".to_owned(),
        line(0.95, 0, 2),
        "not json\n".to_owned(),
        line(0.4, 0, 3),
        line(0.4, 0, 7),
    ]
    .concat();
    let (lines, success) = evaluate_lines(
        &[
            &file,
            "--range",
            "server.mean=0.2..0.9",
            "--result",
            "server.mean_work_in_progress",
            "--seed",
            "9",
        ],
        &input,
    );
    assert!(!success, "a refused line makes the exit code 1");
    assert_eq!(lines.len(), 5, "{lines:?}");
    assert_eq!(lines[0].to_string(), direct.to_string());
    assert_eq!(lines[1]["error"]["code"], "invalid_params");
    assert!(
        lines[1]["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("server.mean")),
        "{}",
        lines[1]
    );
    assert_eq!(lines[2]["error"]["code"], "invalid_params");
    let values = |v: &Value| {
        v["results"][0]["values"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    let mut staged = values(&lines[3]);
    staged.extend(values(&lines[0]));
    assert_eq!(staged, values(&lines[4]));
}

/// `optimize` gives what the engine's `experiment.optimise` gives, as JSON, and a table that
/// starts with the claim in words.
#[test]
fn optimize_matches_the_engine_and_prints_the_claim() {
    let file = model("mm1.bpmn");
    let args = [
        "optimize",
        &file,
        "--decide",
        "serve.capacity=1..4",
        "--minimize",
        "serve.mean_wait*2",
        "--cost",
        "serve.capacity=1",
        "--delta",
        "1",
        "--max-runs",
        "400",
        "--seed",
        "5",
    ];
    let mut with_json = args.to_vec();
    with_json.extend(["--format", "json"]);
    let from_cli = json(&ok(&with_json));

    let mut engine = stochcraft_engine::Engine::new();
    let opened = engine.execute("file.open", serde_json::json!({ "path": file }));
    assert!(opened.is_ok(), "{opened:?}");
    let direct = engine.execute(
        "experiment.optimise",
        serde_json::json!({
            "decisions": [{ "id": "serve", "field": "capacity", "min": 1, "max": 4 }],
            "objective": {
                "results": [{ "element": "serve", "metric": "mean_wait", "weight": 2.0 }],
                "sense": "minimise",
                "costs": [{ "id": "serve", "field": "capacity", "cost": 1.0 }],
            },
            "constraints": [],
            "indifference": 1.0,
            "max_runs": 400,
            "seed": 5,
        }),
    );
    let Ok(direct) = direct else {
        panic!("{direct:?}")
    };
    // Both parsed from their printed text, so a number's last bit is read alike.
    assert_eq!(from_cli, json(&direct.to_string()));
    assert_eq!(from_cli["designs_compared"], 4, "{from_cli}");

    let table = ok(&args);
    let claim = from_cli["claim"].as_str().unwrap_or_default();
    assert!(!claim.is_empty() && table.contains(claim), "{table}");
    assert!(table.contains("serve.capacity"), "{table}");

    let o = cli(&[
        "optimize",
        &file,
        "--decide",
        "serve.capacity=1-4",
        "--delta",
        "1",
    ]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("id.field=min..max"), "{}", stderr(&o));
}

/// `--start` sets a decision's starting value; one for something that is not a decision, or off
/// the range's values, is refused.
#[test]
fn optimize_takes_a_starting_value() {
    let file = model("mm1.bpmn");
    let base = [
        "optimize",
        &file,
        "--decide",
        "serve.capacity=1..4",
        "--minimize",
        "serve.mean_wait*2",
        "--cost",
        "serve.capacity=1",
        "--delta",
        "1",
        "--max-runs",
        "400",
        "--format",
        "json",
    ];
    let mut args = base.to_vec();
    args.extend(["--start", "serve.capacity=3"]);
    let v = json(&ok(&args));
    let start = v["finalists"]
        .as_array()
        .and_then(|fs| fs.iter().find(|f| f["starting_design"] == true))
        .map(|f| f["decisions"][0]["value"].clone());
    assert_eq!(start, Some(serde_json::json!(3.0)), "{v}");

    let mut args = base.to_vec();
    args.extend(["--start", "clerk.quantity=2"]);
    let o = cli(&args);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("is not a decision"), "{}", stderr(&o));

    let mut args = base.to_vec();
    args.extend(["--start", "serve.capacity=7"]);
    let o = cli(&args);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("serve.capacity"), "{}", stderr(&o));
}

/// An agent-based model (a BPMN file with the stochcraft:agents extension and an SCXML chart
/// beside it): `run` prints each statistic at the end with its band, CSV and JSON give each over
/// time, and `validate` checks the charts and populations.
#[test]
fn an_agent_based_model_runs_from_the_command_line() {
    let m = model("sir-agents.bpmn");
    let table = ok(&["run", &m, "--reps", "5"]);
    assert!(table.contains("agent-based, 0 to 60 day"), "{table}");
    assert!(table.contains("5 replications, seed 1"), "{table}");
    assert!(table.contains("recovered"), "{table}");
    assert!(table.contains("people"), "{table}");

    let csv = ok(&[
        "run",
        &m,
        "--reps",
        "3",
        "--duration",
        "10",
        "--format",
        "csv",
    ]);
    assert_eq!(csv.lines().count(), 12, "{csv}");
    assert!(
        csv.starts_with("time (day),susceptible,susceptible median,"),
        "{csv}"
    );

    let r = json(&ok(&[
        "run", &m, "--reps", "2", "--seed", "4", "--format", "json",
    ]));
    assert_eq!(r["replications"], 2);
    assert_eq!(r["seed"], 4);
    assert_eq!(r["statistics"].as_array().map(Vec::len), Some(3));

    let checked = ok(&["validate", &m]);
    assert!(checked.contains("0 errors"), "{checked}");

    // What means nothing for such a model is refused, not ignored.
    let refused = cli(&["run", &m, "--until", "0.05"]);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("agent-based"),
        "{}",
        stderr(&refused)
    );
}

/// A folder of its own for one test, made new (never one left by another run) and removed
/// when the test ends, however it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let d = std::env::temp_dir().join(format!(
                "stochcraft-cli-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&d) {
                Ok(()) => return Self(d),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => panic!("{}: {e}", d.display()),
            }
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `--warmup` on an agent-based model discards what came before it, as on a process model:
/// the statistics are of the time after it, the same numbers a run from 0 gives from then on.
#[test]
fn an_agent_based_models_warm_up_is_left_out_of_its_results() {
    let m = model("sir-agents.bpmn");
    let run = |warmup: &str, duration: &str| {
        json(&ok(&[
            "run",
            &m,
            "--reps",
            "3",
            "--warmup",
            warmup,
            "--duration",
            duration,
            "--format",
            "json",
        ]))
    };
    let full = run("0", "30");
    let warmed = run("10", "20");
    assert_eq!(warmed["warmup"], 10.0);
    assert_eq!(warmed["end"], 30.0);
    let times = |r: &Value| -> Vec<f64> {
        r["times"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .collect()
    };
    let (t_full, t_warmed) = (times(&full), times(&warmed));
    assert_eq!(t_warmed.first(), Some(&10.0));
    let from = t_full.iter().position(|t| *t == 10.0);
    assert_eq!(from.map(|k| &t_full[k..]), Some(&t_warmed[..]));
    for (a, b) in full["statistics"]
        .as_array()
        .into_iter()
        .flatten()
        .zip(warmed["statistics"].as_array().into_iter().flatten())
    {
        let means = |v: &Value| -> Vec<Value> { v["mean"].as_array().cloned().unwrap_or_default() };
        assert_eq!(from.map(|k| means(a)[k..].to_vec()), Some(means(b)));
    }
    let table = ok(&[
        "run",
        &m,
        "--reps",
        "2",
        "--warmup",
        "10",
        "--duration",
        "20",
    ]);
    assert!(table.contains("agent-based, 10 to 30 day"), "{table}");
}

/// `run --out` never writes over a file an agent-based model reads beside it (its chart), as it
/// never writes over the model, and the chart is left as it was.
#[test]
fn run_out_is_refused_over_an_agent_based_models_chart() {
    let dir = Scratch::new("agents-out");
    let models = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models");
    for f in ["sir-agents.bpmn", "sir-person.scxml"] {
        std::fs::copy(models.join(f), dir.0.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
    }
    let m = dir.0.join("sir-agents.bpmn").to_string_lossy().into_owned();
    let chart = dir.0.join("sir-person.scxml");
    let before = std::fs::read(&chart).unwrap_or_else(|e| panic!("{e}"));
    let o = cli(&["run", &m, "--reps", "1", "--out", &chart.to_string_lossy()]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("sir-person.scxml"), "{}", stderr(&o));
    assert_eq!(std::fs::read(&chart).ok(), Some(before));
    // Another file is written as usual.
    let other = dir.0.join("results.txt");
    ok(&["run", &m, "--reps", "1", "--out", &other.to_string_lossy()]);
    assert!(other.exists());
}

/// A number that is not finite is refused before anything runs, naming its flag: a NaN would
/// reach the engine as `null`, which means "not given".
#[test]
fn optimize_refuses_numbers_that_are_not_finite() {
    let file = model("mm1.bpmn");
    let base = [
        "optimize",
        &file,
        "--decide",
        "serve.capacity=1..4",
        "--cost",
        "serve.capacity=1",
        "--delta",
        "1",
    ];
    for (extra, flag) in [
        (vec!["--minimize", "serve.mean_wait*NaN"], "--minimize"),
        (
            vec!["--minimize", "serve.mean_wait", "--cost-cap", "NaN"],
            "--cost-cap",
        ),
        (
            vec!["--minimize", "serve.mean_wait", "--time-limit", "inf"],
            "--time-limit",
        ),
    ] {
        let mut args = base.to_vec();
        args.extend(extra);
        let o = cli(&args);
        assert!(!o.status.success(), "{flag}");
        assert!(stderr(&o).contains(flag), "{flag}: {}", stderr(&o));
    }
}
