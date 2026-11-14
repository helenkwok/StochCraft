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
    let list = ok(&["new", "--list"]);
    for name in ["mm1", "mm2", "tandem", "case-a"] {
        assert!(list.contains(name), "{list}");
        let path = scratch(&format!("{name}.bpmn"));
        let p = path.to_string_lossy();
        ok(&["new", "--template", name, &p, "--force"]);
        let v = ok(&["validate", &p]);
        assert!(v.contains("0 errors"), "{v}");
        let r = json(&ok(&["run", &p, "--reps", "4", "--format", "json"]));
        assert_eq!(r["replications"], 4, "{name}");
        assert_eq!(r["record"]["file"], p.as_ref());
    }
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
    assert!(stdout(&o).contains("no source"), "{}", stdout(&o));
    let j = cli(&["validate", &broken.to_string_lossy(), "--format", "json"]);
    assert_eq!(j.status.code(), Some(1));
    assert!(json(&stdout(&j))["errors"].as_u64().is_some_and(|n| n > 0));
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
