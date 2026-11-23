//! Agent-based models through the engine's commands (ROADMAP M4.3, stage A4): opening one with
//! the files beside it, describing it, running it with results over time, saving it with its
//! charts, and its journal replayed.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use serde_json::{Value, json};
use stochcraft_engine::{
    Engine, ErrorCode, ReplayMode, ReplayOptions, ReplayStep, agents::results_csv,
};

fn ok(e: &mut Engine, command: &str, params: Value) -> Value {
    let r = e.execute(command, params.clone());
    assert!(r.is_ok(), "{command} {params}: {r:?}");
    r.unwrap_or_default()
}

fn err(e: &mut Engine, command: &str, params: Value) -> (ErrorCode, String) {
    let r = e.execute(command, params.clone());
    assert!(r.is_err(), "{command} {params} should fail: {r:?}");
    r.err().map_or((ErrorCode::Internal, String::new()), |e| {
        (e.code, e.message)
    })
}

fn models() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models")
}

fn sir_path() -> String {
    models()
        .join("sir-agents.bpmn")
        .to_string_lossy()
        .into_owned()
}

/// A folder of its own for one test, made new (never one left by another run or another test)
/// and removed when the test ends, however it ends.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let d = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
                "agents-{tag}-{}-{}",
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

    fn join(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn floats(v: &Value) -> Vec<f64> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_f64)
        .collect()
}

fn statistic<'a>(r: &'a Value, name: &str) -> &'a Value {
    r["statistics"]
        .as_array()
        .and_then(|s| s.iter().find(|x| x["name"] == name))
        .unwrap()
}

#[test]
fn a_file_opens_with_its_charts_and_describes_itself() {
    let mut e = Engine::new();
    let opened = ok(&mut e, "file.open", json!({ "path": sir_path() }));
    assert_eq!(opened["kind"], "agents");
    let got = ok(&mut e, "agents.get", json!({}));
    assert_eq!(got["time_unit"], "day");
    assert_eq!(got["end"], 60.0);
    assert_eq!(got["types"][0]["chart"], "sir-person.scxml");
    assert_eq!(
        got["types"][0]["states"],
        json!(["start", "susceptible", "infected", "recovered"])
    );
    assert_eq!(got["populations"][0]["count"], 100);
    assert_eq!(got["files"], json!(["sir-person.scxml"]));
    assert_eq!(got["problem"], Value::Null);
    assert_eq!(got["has_results"], false);
    // `run` is for process models.
    let (code, message) = err(&mut e, "run", json!({}));
    assert_eq!(code, ErrorCode::Unsupported);
    assert!(message.contains("agents.run"), "{message}");
}

#[test]
fn a_run_gives_each_statistic_over_time_with_its_band() {
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({ "path": sir_path() }));
    let r = ok(&mut e, "agents.run", json!({ "replications": 8 }));
    let times = floats(&r["times"]);
    assert_eq!(times.first(), Some(&0.0));
    assert_eq!(times.last(), Some(&60.0));
    assert!(times.windows(2).all(|w| w[1] > w[0]), "{times:?}");
    let (s, i, rec) = (
        floats(&statistic(&r, "susceptible")["mean"]),
        floats(&statistic(&r, "infected")["mean"]),
        floats(&statistic(&r, "recovered")["mean"]),
    );
    assert_eq!(s.len(), times.len());
    // Everyone is in one of the three states at every recorded time.
    for k in 0..times.len() {
        assert!(
            (s[k] + i[k] + rec[k] - 100.0).abs() < 1e-9,
            "at {}",
            times[k]
        );
    }
    // One infected to start; the susceptible never grow.
    assert_eq!(i[0], 1.0);
    assert!(s.windows(2).all(|w| w[1] <= w[0]));
    for name in ["median", "p5", "p95"] {
        assert_eq!(
            statistic(&r, "infected")[name].as_array().map(Vec::len),
            Some(times.len())
        );
    }
    assert_eq!(r["populations"][0]["created_mean"], 100.0);
    assert_eq!(r["replications"], 8);
    // The same run again gives the same numbers; another seed does not.
    let again = ok(&mut e, "agents.run", json!({ "replications": 8 }));
    assert_eq!(again["statistics"], r["statistics"]);
    let other = ok(
        &mut e,
        "agents.run",
        json!({ "replications": 8, "seed": 99 }),
    );
    assert_ne!(other["statistics"], r["statistics"]);
    // As CSV: a header and a row per time.
    let csv = results_csv(&r);
    assert!(
        csv.starts_with("time (day),susceptible,susceptible median,"),
        "{csv}"
    );
    assert_eq!(csv.lines().count(), times.len() + 1);
    let got = ok(&mut e, "agents.get", json!({}));
    assert_eq!(got["has_results"], true);
    assert_eq!(got["results_stale"], false);
    ok(&mut e, "experiment.set", json!({ "duration": 30.0 }));
    assert_eq!(ok(&mut e, "agents.get", json!({}))["results_stale"], true);
    let short = ok(&mut e, "agents.run", json!({ "replications": 2 }));
    assert_eq!(floats(&short["times"]).last(), Some(&30.0));
}

#[test]
fn agents_open_takes_the_files_as_texts() {
    let bpmn = std::fs::read_to_string(models().join("sir-agents.bpmn")).unwrap();
    let chart = std::fs::read_to_string(models().join("sir-person.scxml")).unwrap();
    let mut e = Engine::new();
    let (code, message) = err(&mut e, "agents.open", json!({ "text": bpmn }));
    assert_eq!(code, ErrorCode::Io);
    assert!(message.contains("sir-person.scxml"), "{message}");
    ok(
        &mut e,
        "agents.open",
        json!({ "text": bpmn, "files": { "sir-person.scxml": chart } }),
    );
    let from_text = ok(&mut e, "agents.run", json!({ "replications": 3 }));
    let mut f = Engine::new();
    ok(&mut f, "file.open", json!({ "path": sir_path() }));
    let from_file = ok(&mut f, "agents.run", json!({ "replications": 3 }));
    assert_eq!(from_text["statistics"], from_file["statistics"]);
    assert_eq!(
        from_text["record"]["model_digest"],
        from_file["record"]["model_digest"]
    );
    // A process model is not opened this way.
    let (code, _) = err(
        &mut e,
        "agents.open",
        json!({ "text": stochcraft_io_text_of_a_process_model() }),
    );
    assert_eq!(code, ErrorCode::InvalidParams);
    // A model alone as bytes cannot read its charts.
    let r = Engine::new().open_bytes("sir.bpmn", bpmn.as_bytes(), None);
    assert!(
        r.as_ref().is_err_and(|e| e.message.contains("file.open")),
        "{r:?}"
    );
}

fn stochcraft_io_text_of_a_process_model() -> String {
    std::fs::read_to_string(models().join("mm1.bpmn")).unwrap()
}

#[test]
fn a_chart_that_cannot_be_read_or_a_reference_outside_the_folder_is_refused() {
    let bpmn = std::fs::read_to_string(models().join("sir-agents.bpmn")).unwrap();
    let mut e = Engine::new();
    let (code, message) = err(
        &mut e,
        "agents.open",
        json!({ "text": bpmn, "files": { "sir-person.scxml": "<scxml xmlns=\"http://www.w3.org/2005/07/scxml\" version=\"1.0\"><state id=\"a\"><invoke/></state></scxml>" } }),
    );
    assert_eq!(code, ErrorCode::BadFile);
    assert!(message.contains("<invoke>"), "{message}");
    let outside = bpmn.replace(
        "chart=\"sir-person.scxml\"",
        "chart=\"../sir-person.scxml\"",
    );
    let (code, message) = err(
        &mut e,
        "agents.open",
        json!({ "text": outside, "files": { "../sir-person.scxml": "" } }),
    );
    assert_eq!(code, ErrorCode::BadFile);
    assert!(message.contains("relative reference"), "{message}");
}

#[test]
fn a_run_that_stops_names_the_replication() {
    let bpmn = std::fs::read_to_string(models().join("sir-agents.bpmn")).unwrap();
    let chart = std::fs::read_to_string(models().join("sir-person.scxml"))
        .unwrap()
        .replace(
            "<state id=\"recovered\"/>",
            "<state id=\"recovered\"><transition target=\"recovered\"/></state>",
        );
    let mut e = Engine::new();
    ok(
        &mut e,
        "agents.open",
        json!({ "text": bpmn, "files": { "sir-person.scxml": chart } }),
    );
    let (code, message) = err(&mut e, "agents.run", json!({ "replications": 2 }));
    assert_eq!(code, ErrorCode::InvalidModel);
    assert!(message.contains("replication 1"), "{message}");
}

#[test]
fn saved_elsewhere_the_charts_go_beside_it_and_it_opens_the_same() {
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({ "path": sir_path() }));
    let before = ok(&mut e, "agents.run", json!({ "replications": 2 }));
    let dir = Scratch::new("saved");
    let to = dir.join("sir.bpmn");
    ok(&mut e, "file.save", json!({ "path": to.to_string_lossy() }));
    let chart = std::fs::read(dir.join("sir-person.scxml")).unwrap();
    assert_eq!(
        chart,
        std::fs::read(models().join("sir-person.scxml")).unwrap()
    );
    let mut f = Engine::new();
    ok(&mut f, "file.open", json!({ "path": to.to_string_lossy() }));
    let after = ok(&mut f, "agents.run", json!({ "replications": 2 }));
    assert_eq!(before["statistics"], after["statistics"]);
    assert_eq!(
        before["record"]["model_digest"],
        after["record"]["model_digest"]
    );
    // A different file of a chart's name beside it is never replaced: the save is refused and
    // nothing is written.
    let bpmn_before = std::fs::read(&to).unwrap();
    std::fs::write(dir.join("sir-person.scxml"), "another chart").unwrap();
    ok(&mut e, "experiment.set", json!({ "duration": 10.0 }));
    let (code, message) = err(&mut e, "file.save", json!({ "path": to.to_string_lossy() }));
    assert_eq!(code, ErrorCode::Conflict);
    assert!(message.contains("differs"), "{message}");
    assert_eq!(std::fs::read(&to).unwrap(), bpmn_before);
    assert_eq!(
        std::fs::read(dir.join("sir-person.scxml")).unwrap(),
        b"another chart"
    );
}

/// The model is never saved over a file it reads beside it: not over its chart by name, nor
/// over another name for the same file (a hard link).
#[test]
fn the_model_is_never_saved_over_its_own_chart() {
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({ "path": sir_path() }));
    let dir = Scratch::new("alias");
    // By name: nothing is written.
    let chart = dir.join("sir-person.scxml");
    let (code, message) = err(
        &mut e,
        "file.save",
        json!({ "path": chart.to_string_lossy() }),
    );
    assert_eq!(code, ErrorCode::InvalidParams);
    assert!(message.contains("sir-person.scxml"), "{message}");
    assert!(!chart.exists());
    // Saved properly, then to another name of its chart.
    let model = dir.join("sir.bpmn");
    ok(
        &mut e,
        "file.save",
        json!({ "path": model.to_string_lossy() }),
    );
    let original = std::fs::read(&chart).unwrap();
    let alias = dir.join("alias.bpmn");
    std::fs::hard_link(&chart, &alias).unwrap();
    let (code, message) = err(
        &mut e,
        "file.save",
        json!({ "path": alias.to_string_lossy() }),
    );
    assert_eq!(code, ErrorCode::InvalidParams);
    assert!(message.contains("sir-person.scxml"), "{message}");
    assert_eq!(std::fs::read(&chart).unwrap(), original);
    // A model of its own name beside the chart saves as before.
    ok(
        &mut e,
        "file.save",
        json!({ "path": model.to_string_lossy() }),
    );
}

/// A chart's place that is there but cannot be read (here a folder) stops the save before
/// anything is written: only a file that is not there is written.
#[test]
fn a_chart_that_cannot_be_read_stops_the_save() {
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({ "path": sir_path() }));
    let dir = Scratch::new("unreadable");
    std::fs::create_dir(dir.join("sir-person.scxml")).unwrap();
    let model = dir.join("sir.bpmn");
    let (code, _) = err(
        &mut e,
        "file.save",
        json!({ "path": model.to_string_lossy() }),
    );
    assert_eq!(code, ErrorCode::Io);
    assert!(!model.exists());
    assert!(dir.join("sir-person.scxml").is_dir());
}

/// The charts are written before the model, so a chart that cannot be written leaves the old
/// model file as it was. Here the chart `a.scxml/b.scxml` needs a folder where the chart
/// `a.scxml` has just been written.
#[test]
fn a_chart_that_cannot_be_written_leaves_the_old_model() {
    let bpmn = std::fs::read_to_string(models().join("sir-agents.bpmn"))
        .unwrap()
        .replace(
            "<stochcraft:agentType name=\"person\" chart=\"sir-person.scxml\"/>",
            "<stochcraft:agentType name=\"person\" chart=\"a.scxml\"/><stochcraft:agentType name=\"other\" chart=\"a.scxml/b.scxml\"/>",
        );
    let chart = std::fs::read_to_string(models().join("sir-person.scxml")).unwrap();
    let mut e = Engine::new();
    ok(
        &mut e,
        "agents.open",
        json!({ "text": bpmn, "files": { "a.scxml": chart, "a.scxml/b.scxml": chart } }),
    );
    let dir = Scratch::new("order");
    let model = dir.join("m.bpmn");
    std::fs::write(&model, "the old model").unwrap();
    let (code, _) = err(
        &mut e,
        "file.save",
        json!({ "path": model.to_string_lossy() }),
    );
    assert_eq!(code, ErrorCode::Io);
    assert_eq!(std::fs::read(&model).unwrap(), b"the old model");
}

/// The run's results are of the time after the warm-up, the same numbers a run from 0 gives
/// from then on; a warm-up or duration that is no time, or an end before the warm-up, is
/// refused.
#[test]
fn the_warm_up_is_left_out_and_the_run_length_checked() {
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({ "path": sir_path() }));
    ok(&mut e, "experiment.set", json!({ "duration": 30.0 }));
    let full = ok(&mut e, "agents.run", json!({ "replications": 4 }));
    ok(
        &mut e,
        "experiment.set",
        json!({ "warmup": 10.0, "duration": 20.0 }),
    );
    let warmed = ok(&mut e, "agents.run", json!({ "replications": 4 }));
    assert_eq!(warmed["warmup"], 10.0);
    assert_eq!(warmed["end"], 30.0);
    let (t_full, t_warmed) = (floats(&full["times"]), floats(&warmed["times"]));
    assert_eq!(t_warmed.first(), Some(&10.0));
    assert_eq!(t_full[10..], t_warmed[..]);
    for name in ["susceptible", "infected", "recovered"] {
        for k in ["mean", "median", "p5", "p95"] {
            let a = statistic(&full, name)[k].as_array().unwrap();
            let b = statistic(&warmed, name)[k].as_array().unwrap();
            assert_eq!(a[10..], b[..], "{name} {k}");
        }
    }
    // No one is created after the warm-up.
    assert_eq!(warmed["populations"][0]["created_mean"], 0.0);
    let (code, _) = err(&mut e, "agents.run", json!({ "end": 5.0 }));
    assert_eq!(code, ErrorCode::InvalidParams);
    // A duration below 0 that the warm-up would hide in their sum.
    ok(
        &mut e,
        "experiment.set",
        json!({ "warmup": 2.0, "duration": -1.0 }),
    );
    let (code, message) = err(&mut e, "agents.run", json!({}));
    assert_eq!(code, ErrorCode::InvalidModel);
    assert!(message.contains("errors"), "{message}");
    ok(
        &mut e,
        "experiment.set",
        json!({ "warmup": -1.0, "duration": 10.0 }),
    );
    let (code, _) = err(&mut e, "agents.run", json!({}));
    assert_eq!(code, ErrorCode::InvalidModel);
}

#[test]
fn only_charts_and_lists_of_links_inside_the_folder_are_read() {
    let bpmn = std::fs::read_to_string(models().join("sir-agents.bpmn")).unwrap();
    let mut e = Engine::new();
    for chart in ["config.toml", "nul.scxml", "../x.scxml"] {
        let text = bpmn.replace("chart=\"sir-person.scxml\"", &format!("chart=\"{chart}\""));
        let (code, message) = err(
            &mut e,
            "agents.open",
            json!({ "text": text, "files": { chart: "" } }),
        );
        assert_eq!(code, ErrorCode::BadFile, "{chart}");
        assert!(message.contains("relative reference"), "{message}");
    }
}

#[test]
fn its_journal_replays_with_the_same_numbers() {
    let mut e = Engine::new();
    ok(&mut e, "file.open", json!({ "path": sir_path() }));
    ok(&mut e, "experiment.set", json!({ "duration": 20.0 }));
    let first = ok(
        &mut e,
        "agents.run",
        json!({ "replications": 3, "seed": 5 }),
    );
    let journal = e.journal_document();
    assert_eq!(journal.start.document.as_deref(), Some("agents"));
    let commands: Vec<&str> = journal.entries.iter().map(|j| j.command.as_str()).collect();
    assert_eq!(commands, ["experiment.set", "agents.run"]);
    // Replayed on the same file, every step: the same numbers.
    let mut f = Engine::new();
    ok(&mut f, "file.open", json!({ "path": sir_path() }));
    let steps = f
        .replay_journal(
            &journal,
            ReplayOptions {
                mode: ReplayMode::All,
                force: false,
            },
        )
        .unwrap();
    match &steps[1] {
        ReplayStep::Applied(r) => assert_eq!(r["statistics"], first["statistics"]),
        other => panic!("{other:?}"),
    }
    // By default the run is skipped.
    let mut g = Engine::new();
    ok(&mut g, "file.open", json!({ "path": sir_path() }));
    let steps = g
        .replay_journal(&journal, ReplayOptions::default())
        .unwrap();
    assert_eq!(steps[1], ReplayStep::Skipped);
    // Not onto another model.
    let mut h = Engine::new();
    let other = std::fs::read_to_string(models().join("sir-person.scxml"))
        .unwrap()
        .replace("stochcraft:rate=\"1\"", "stochcraft:rate=\"2\"");
    ok(
        &mut h,
        "agents.open",
        json!({
            "text": std::fs::read_to_string(models().join("sir-agents.bpmn")).unwrap(),
            "files": { "sir-person.scxml": other },
        }),
    );
    let r = h.replay_journal(&journal, ReplayOptions::default());
    assert!(r.is_err_and(|e| e.error.code == ErrorCode::Conflict));
}
