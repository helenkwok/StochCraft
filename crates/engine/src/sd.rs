//! System dynamics models through the engine (ROADMAP M4.2): the commands `sd.open`, `sd.run`
//! and `sd.get`, and `file.open` and `file.save` on an XMILE file. The edits (`sd.new`,
//! `sd.variable.add`, `.set` and `.remove`, `sd.specs.set` and `sd.layout.set`) are in
//! [`crate::sd_edit`].
//!
//! An XMILE model is its own document, held beside the (empty) process model: opening one
//! starts a new document and a new journal, as `file.open` on a BPMN file does, and `run`
//! refuses it, as it refuses a Monte Carlo model, with the command to use. The runs are
//! `stochcraft-sd`'s.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_model::Model;
use stochcraft_sd::{
    ErrorKind, Program, RunOptions, SdError, SdModel, SdResults, VarKind, canonical, flow_ends,
};

use crate::commands::parse;
use crate::{CommandError, Engine, ErrorCode, JournalStart, StartKind};

/// The model `sd.open`'s example opens.
pub(crate) const EXAMPLE_XMILE: &str = "<xmile version=\"1.0\" xmlns=\"http://docs.oasis-open.org/xmile/ns/XMILE/v1.0\"><header><name>Growth</name></header><sim_specs method=\"Euler\"><start>0</start><stop>10</stop><dt>0.25</dt></sim_specs><model><variables><stock name=\"Population\"><eqn>100</eqn><inflow>growth</inflow></stock><flow name=\"growth\"><eqn>0.1 * Population</eqn></flow></variables></model></xmile>";

/// The open system dynamics model, and its last run.
pub(crate) struct SdDoc {
    /// The name it was opened under (the path, for a file); empty for a new model.
    pub path: String,
    /// The file it was opened from or saved to, which `file.save` writes to.
    pub file: Option<String>,
    pub model: SdModel,
    /// The model as last opened or saved, to tell whether there are unsaved changes.
    pub saved: SdModel,
    pub program: Program,
    pub notes: Vec<String>,
    /// Saving into `file` would leave out what the reader noted it does not keep: the model was
    /// read from that file and not written back to it since.
    pub drops_on_save: bool,
    pub last: Option<SdResults>,
    /// The model was edited since `last` was run.
    pub last_stale: bool,
}

pub(crate) fn sd_error(e: SdError) -> CommandError {
    let code = match e.kind {
        ErrorKind::Xml => ErrorCode::BadFile,
        ErrorKind::Unsupported => ErrorCode::Unsupported,
        ErrorKind::Equation | ErrorKind::Loop | ErrorKind::Model => ErrorCode::InvalidModel,
        ErrorKind::Run => ErrorCode::InvalidParams,
    };
    let mut c = CommandError::new(code, e.message);
    c.details = Some(json!({ "kind": e.kind.name() }));
    c
}

pub(crate) fn no_model() -> CommandError {
    CommandError::new(
        ErrorCode::NotFound,
        "no system dynamics model is open: open an XMILE file with file.open, or give sd.open its text",
    )
}

pub(crate) fn warnings_json(program: &Program) -> Value {
    program
        .warnings
        .iter()
        .map(|w| json!({ "severity": "warning", "code": "sd_check", "message": format!("warning: {w}") }))
        .collect()
}

impl Engine {
    /// Opens an XMILE model from its bytes, replacing the document (what `file.open` does for
    /// an XMILE file, and `sd.open` for its text).
    pub(crate) fn open_sd(
        &mut self,
        name: &str,
        bytes: &[u8],
        is_file: bool,
    ) -> Result<Value, CommandError> {
        let text = std::str::from_utf8(bytes).map_err(|_| {
            CommandError::new(ErrorCode::BadFile, format!("{name}: not UTF-8 text"))
        })?;
        let file = stochcraft_sd::read_xmile(text).map_err(|e| {
            let mut c = sd_error(e);
            c.message = format!("{name}: {}", c.message);
            c
        })?;
        let program = stochcraft_sd::compile(&file.model).map_err(|e| {
            let mut c = sd_error(e);
            c.message = format!("{name}: {}", c.message);
            c
        })?;
        let mut start = JournalStart::new(StartKind::File, Some(name.to_owned()));
        start.file_digest = Some(crate::bytes_digest(bytes));
        let result = json!({
            "path": name,
            "format": "xmile",
            "kind": "system_dynamics",
            "scenario": Value::Null,
            "scenarios": [],
            "notes": file.notes.iter().map(|n| json!({ "severity": "info", "at": Value::Null, "message": n })).collect::<Vec<_>>(),
            "diagnostics": warnings_json(&program),
            "model": describe(&file.model, &program),
        });
        self.new_document(Model::default(), start);
        self.sd = Some(SdDoc {
            path: name.to_owned(),
            file: is_file.then(|| name.to_owned()),
            saved: file.model.clone(),
            model: file.model,
            program,
            drops_on_save: is_file && !file.notes.is_empty(),
            notes: file.notes,
            last: None,
            last_stale: false,
        });
        Ok(result)
    }
}

/// The model as JSON: its settings, and each variable with its kind, equation and units.
pub(crate) fn describe(model: &SdModel, program: &Program) -> Value {
    let s = &model.specs;
    // A variable's name as the model writes it, from its canonical one.
    let shown = |c: &str| {
        model
            .vars
            .iter()
            .find(|v| canonical(&v.name) == c)
            .map(|v| v.name.clone())
    };
    let vars: Vec<Value> = model
        .vars
        .iter()
        .map(|v| {
            let (inflows, outflows, non_negative) = match &v.kind {
                stochcraft_sd::VarKind::Stock {
                    inflows,
                    outflows,
                    non_negative,
                } => (inflows.clone(), outflows.clone(), *non_negative),
                stochcraft_sd::VarKind::Flow { non_negative } => (vec![], vec![], *non_negative),
                stochcraft_sd::VarKind::Aux => (vec![], vec![], false),
            };
            let mut o = json!({
                "name": v.name,
                "kind": v.kind.name(),
                "equation": v.eqn,
                "units": v.units,
            });
            if v.kind.name() == "stock" {
                o["inflows"] = json!(inflows);
                o["outflows"] = json!(outflows);
            }
            if non_negative {
                o["non_negative"] = json!(true);
            }
            if let Some(g) = &v.gf {
                o["graphical_function"] =
                    json!({ "type": g.kind.name(), "points": g.points().ok() });
            }
            if let Some(doc) = &v.doc {
                o["doc"] = json!(doc);
            }
            if let Some(shape) = model.layout.shapes.get(&canonical(&v.name)) {
                o["x"] = json!(shape.x);
                o["y"] = json!(shape.y);
                if matches!(v.kind, VarKind::Flow { .. }) {
                    o["pipe"] = json!(shape.pts.iter().map(|p| [p.0, p.1]).collect::<Vec<_>>());
                    let (src, dst) = flow_ends(model, &v.name);
                    o["source"] = json!(src.as_deref().and_then(&shown));
                    o["sink"] = json!(dst.as_deref().and_then(&shown));
                } else {
                    let (w, h) = shape.size(&v.kind);
                    o["width"] = json!(w);
                    o["height"] = json!(h);
                }
            }
            o
        })
        .collect();
    let connectors: Vec<Value> = model
        .layout
        .connectors
        .iter()
        .map(|c| json!({ "from": shown(&c.from), "to": shown(&c.to), "angle": c.angle }))
        .collect();
    json!({
        "name": model.name,
        "method": s.method.name(),
        "start": s.start,
        "stop": s.stop,
        "dt": program.dt(),
        "save_step": s.save_step.unwrap_or_else(|| program.dt()),
        "time_unit": s.time_units,
        "random": program.is_random(),
        "variables": vars,
        "connectors": connectors,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Open {
    /// The text of an XMILE 1.0 file. (A file on disk is opened with `file.open`.)
    pub text: String,
    /// The name to report it under; by default `model.xmile`.
    #[serde(default)]
    pub name: Option<String>,
}

pub(crate) fn open(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Open = parse(p)?;
    let name = p.name.unwrap_or_else(|| "model.xmile".to_owned());
    e.open_sd(&name, p.text.as_bytes(), false)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Run {
    /// Replications, for a model with a random function (default 100); a model without one is
    /// run once.
    #[serde(default)]
    pub replications: Option<u64>,
    /// The seed of the random streams (default 1).
    #[serde(default)]
    pub seed: Option<u64>,
    /// A time step in place of the file's, in the model's time unit.
    #[serde(default)]
    pub dt: Option<f64>,
    /// Also run at half the step and report the largest relative difference of each stock.
    #[serde(default)]
    pub dt_check: Option<bool>,
    /// Report times in this unit (second, minute, hour, day, week, month or year) instead of
    /// the model's.
    #[serde(default)]
    pub time_unit: Option<String>,
}

pub(crate) fn run(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let mut job = e.sd_start(p)?;
    job.step(usize::MAX)?;
    e.sd_finish(job)
}

/// A system dynamics run made a few replications at a time ([`Engine::sd_start`]), so the window
/// can draw between them and a long run never holds up the page: each [`SdJob::step`] makes
/// some replications, and [`Engine::sd_results`] gives the results once all are made. `sd.run`
/// is the same run with every replication made at once, so the results are the same however the
/// replications are spread over steps. One replication is not interrupted.
///
/// The job runs the model as it was when it started. [`Engine::sd_results`] refuses it if the
/// model was changed since (its [`SdJob::revision`] is no longer the engine's), so results of an
/// old model are never shown as the open one's.
pub struct SdJob {
    job: stochcraft_sd::SdJob,
    /// `sd.run`'s parameters, for the journal.
    params: Value,
    revision: u64,
}

impl SdJob {
    /// Makes up to `replications` (at least one) of the replications still to make. True once
    /// every one is made.
    pub fn step(&mut self, replications: usize) -> Result<bool, CommandError> {
        self.job.step(replications).map_err(sd_error)?;
        Ok(self.job.is_done())
    }

    /// The replications made, and those the run is to make (a halved-step check counts its own).
    pub fn progress(&self) -> (u64, u64) {
        self.job.progress()
    }

    /// The engine's [`Engine::revision`] when the run started.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

impl Engine {
    /// Gets a run of the open system dynamics model ready, with `sd.run`'s parameters, without
    /// making any replication yet: the window makes them a few a frame with [`SdJob::step`].
    pub fn sd_start(&self, params: Value) -> Result<SdJob, CommandError> {
        let params = if params.is_null() { json!({}) } else { params };
        let p: Run = parse(params.clone())?;
        let Some(doc) = self.sd.as_ref() else {
            return Err(no_model());
        };
        let options = RunOptions {
            replications: p.replications,
            seed: p.seed.unwrap_or(1),
            dt: p.dt,
            dt_check: p.dt_check.unwrap_or(false),
            time_unit: p.time_unit,
        };
        let job = stochcraft_sd::SdJob::new(&doc.program, &options).map_err(sd_error)?;
        Ok(SdJob {
            job,
            params,
            revision: self.revision,
        })
    }

    /// A finished run's results, as `sd.run` gives them, which are now the model's last results
    /// (so `sd.get` and the stale marking behave as after `sd.run`), and the run is journalled as an `sd.run`
    /// step. Makes any replication not made yet. Refused with [`ErrorCode::Conflict`] if the
    /// model was changed, or another opened, since the run started.
    pub fn sd_results(&mut self, mut job: SdJob) -> Result<Value, CommandError> {
        job.step(usize::MAX)?;
        let params = job.params.clone();
        let result = self.sd_finish(job)?;
        self.journal.push(crate::JournalEntry {
            command: "sd.run".to_owned(),
            params,
        });
        Ok(result)
    }

    /// Makes the results of a run whose replications are all made the model's last, without
    /// journalling (`execute` does that for `sd.run`).
    fn sd_finish(&mut self, job: SdJob) -> Result<Value, CommandError> {
        if job.revision != self.revision {
            return Err(CommandError::new(
                ErrorCode::Conflict,
                "the model was changed while it was run: run it again",
            ));
        }
        let Some(doc) = self.sd.as_mut() else {
            return Err(no_model());
        };
        let results = job.job.finish().map_err(sd_error)?;
        // The digest is of the model without its diagram: moving a shape does not change it.
        let digest = crate::bytes_digest(stochcraft_sd::model_text(&doc.model).as_bytes());
        let mut json = results_json(&results, &digest, &doc.notes);
        json["record"]["file"] = if doc.path.is_empty() {
            Value::Null
        } else {
            json!(doc.path)
        };
        doc.last = Some(results);
        doc.last_stale = false;
        Ok(json)
    }
}

fn finite(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

fn floats(v: &[f64]) -> Value {
    Value::Array(v.iter().map(|x| finite(*x)).collect())
}

/// A run's results as JSON: the times, and each variable's mean (with, for a model that was
/// replicated, the median and the 5th and 95th percentiles) at each of them.
pub fn results_json(r: &SdResults, digest: &str, notes: &[String]) -> Value {
    let variables: Vec<Value> = r
        .series
        .iter()
        .map(|s| {
            let mut o = json!({
                "name": s.name,
                "kind": s.kind,
                "units": s.units,
                "final": s.mean.last().map_or(Value::Null, |v| finite(*v)),
                "mean": floats(&s.mean),
            });
            if r.replications > 1 {
                o["median"] = floats(&s.median);
                o["p5"] = floats(&s.low);
                o["p95"] = floats(&s.high);
            }
            o
        })
        .collect();
    let dt_check = r.dt_check.as_ref().map(|c| {
        json!({
            "dt": c.dt,
            "half_dt": c.half_dt,
            "stocks": c.stocks.iter().map(|s| json!({
                "name": s.name,
                "max_relative_difference": finite(s.max_relative_difference),
                "at_time": s.at_time,
                "compared_times": s.compared,
                "inconclusive": s.inconclusive,
            })).collect::<Vec<_>>(),
            "max_relative_difference": finite(c.max_relative_difference),
            "threshold": stochcraft_sd::DT_CHECK_THRESHOLD,
            "within_threshold": c.within_threshold,
            "inconclusive": c.inconclusive,
            "draws_matched": c.draws_matched,
            "notes": c.notes,
            "method": "the model run again at half the time step, to the same end time, with the same seed (the random values are the same ones when `draws_matched` is true); for each stock, the largest over the saved times both runs share (compared by time, after the start) of the difference between the two mean paths, relative to the larger of the value at half the step and 0.1% of the stock's largest value; inconclusive, not a difference of 0, when a stock has no shared time or a value that is missing or not a number",
        })
    });
    let uncertainty = if r.replications > 1 {
        "mean, median and the 5th and 95th percentiles across independent replications at each saved time (Hyndman and Fan's definition 7)"
    } else {
        "one run: a model with no random function has no spread"
    };
    json!({
        "model": r.name,
        "method": r.method,
        "dt": r.dt,
        "start": r.start,
        "stop": r.stop,
        "time_unit": r.time_unit,
        "save_step": r.save_step,
        "replications": r.replications,
        "random": r.random,
        "seed": r.seed,
        "steps": r.steps,
        "times": floats(&r.times),
        "variables": variables,
        "warnings": r.warnings,
        "dt_check": dt_check,
        "record": {
            "stochcraft": env!("CARGO_PKG_VERSION"),
            "model_digest": digest,
            "file": Value::Null,
            "seed": r.seed,
            "replications": r.replications,
            "time": "start + n * dt from an integer step counter, never added up",
            "random_streams": "each random function draws from a stream keyed by its variable, which call in it, the seed it is given and the number of the draw, under a stream for the replication from the run's seed, so results do not shift when another variable is added or the step is halved",
            "uncertainty": uncertainty,
            "file_notes": notes,
        },
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Get {
    /// A variable: also give its values from the last `sd.run`.
    #[serde(default)]
    pub variable: Option<String>,
}

pub(crate) fn get(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Get = parse(p)?;
    let Some(doc) = e.sd.as_ref() else {
        return Err(no_model());
    };
    let mut out = describe(&doc.model, &doc.program);
    out["diagnostics"] = warnings_json(&doc.program);
    out["notes"] = json!(doc.notes);
    out["results_stale"] = json!(doc.last.is_some() && doc.last_stale);
    if let Some(name) = p.variable {
        let want = stochcraft_sd::canonical(&name);
        if !doc
            .model
            .vars
            .iter()
            .any(|v| stochcraft_sd::canonical(&v.name) == want)
        {
            return Err(CommandError::new(
                ErrorCode::NotFound,
                format!("no variable `{name}`"),
            ));
        }
        out["values"] = match &doc.last {
            Some(r) => match r.series(&name) {
                Some(s) => {
                    let mut o = json!({
                        "name": s.name,
                        "time_unit": r.time_unit,
                        "times": floats(&r.times),
                        "mean": floats(&s.mean),
                        "stale": doc.last_stale,
                    });
                    if r.replications > 1 {
                        o["median"] = floats(&s.median);
                        o["p5"] = floats(&s.low);
                        o["p95"] = floats(&s.high);
                    }
                    o
                }
                None => Value::Null,
            },
            None => Value::Null,
        };
    }
    Ok(out)
}

/// A system dynamics run's results (as [`results_json`] gives them) as CSV: a row per saved time,
/// the time and then each variable's mean, with its median and 5th and 95th percentiles after
/// it where the model was replicated. This is what `stochcraft-cli run --format csv` writes.
pub fn results_csv(r: &Value) -> String {
    let vars: Vec<&Value> = r["variables"].as_array().into_iter().flatten().collect();
    let many = r["replications"].as_u64().unwrap_or(1) > 1;
    let name = |v: &Value| crate::wire::text_cell(v["name"].as_str().unwrap_or(""));
    let mut header = vec![format!("time ({})", r["time_unit"].as_str().unwrap_or(""))];
    for v in &vars {
        let n = name(v);
        header.push(n.clone());
        if many {
            header.push(format!("{n} median"));
            header.push(format!("{n} p5"));
            header.push(format!("{n} p95"));
        }
    }
    let mut out = header
        .iter()
        .map(|h| crate::wire::csv_field(h))
        .collect::<Vec<_>>()
        .join(",");
    out.push('\n');
    let cell = |v: &Value, k: usize| {
        v.get(k)
            .and_then(Value::as_f64)
            .map(|x| x.to_string())
            .unwrap_or_default()
    };
    for (k, t) in r["times"].as_array().into_iter().flatten().enumerate() {
        let mut row = vec![t.as_f64().map(|x| x.to_string()).unwrap_or_default()];
        for v in &vars {
            row.push(cell(&v["mean"], k));
            if many {
                row.push(cell(&v["median"], k));
                row.push(cell(&v["p5"], k));
                row.push(cell(&v["p95"], k));
            }
        }
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}
