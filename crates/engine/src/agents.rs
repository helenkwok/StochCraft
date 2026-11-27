//! Agent-based models through the engine (ROADMAP M4.3, stage A4): the commands `agents.open`,
//! `agents.get` and `agents.run`, and `file.open` and `file.save` on a BPMN file with the
//! `stochcraft:agents` extension.
//!
//! An agent-based model is the model's [`Agents`] section on a model with no process, and each
//! agent type's state chart (and each list of links) is a file of its own beside it, named by a
//! relative reference. The engine keeps those files' bytes with the document: `file.open` reads
//! them from beside the model's file, `agents.open` takes their texts, and `file.save` writes them
//! beside the file it saves to. They are read again as each run starts, in the experiment's time
//! unit then. Its run lasts the experiment's warm-up plus its duration, in its replications
//! from its seed, and its results are of the time after the warm-up; `run` refuses it, naming
//! `agents.run`. Editing populations and charts comes with the window (stage A5);
//! `experiment.set` changes the run's length, replications and seed.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_agents::described::{Files, build};
use stochcraft_agents::{ChartSpec, ModelError, Run, bands};
use stochcraft_model::agents::{Agents, NetworkSpec};
use stochcraft_random::Rng;

use crate::commands::parse;
use crate::{CommandError, Engine, ErrorCode};

/// The most replications one `agents.run` makes.
pub const MAX_REPLICATIONS: u64 = 100_000;

/// The model `agents.open`'s example opens: a few people passing on a rumour.
pub(crate) const EXAMPLE_BPMN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<bpmn:definitions xmlns:bpmn="http://www.omg.org/spec/BPMN/20100524/MODEL" xmlns:bpsim="http://www.bpsim.org/schemas/2.0" id="rumour" name="Rumour" targetNamespace="https://github.com/helenkwok/StochCraft/model">
  <bpmn:relationship type="BPSimData">
    <bpmn:extensionElements>
      <bpsim:BPSimData>
        <bpsim:Scenario id="base" name="Rumour">
          <bpsim:ScenarioParameters replication="5" seed="1" baseTimeUnit="day">
            <bpsim:Duration><bpsim:FloatingParameter value="10"/></bpsim:Duration>
          </bpsim:ScenarioParameters>
          <bpsim:VendorExtension name="stochcraft:agents" xmlns:stochcraft="https://helenkwok.github.io/StochCraft/bpsim/1">
            <stochcraft:agents saveEvery="1">
              <stochcraft:agentType name="person" chart="person.scxml"/>
              <stochcraft:population name="people" type="person" count="20"/>
              <stochcraft:statistic name="heard" population="people" measure="count" condition="In('heard')"/>
            </stochcraft:agents>
          </bpsim:VendorExtension>
        </bpsim:Scenario>
      </bpsim:BPSimData>
    </bpmn:extensionElements>
    <bpmn:source>rumour</bpmn:source>
    <bpmn:target>rumour</bpmn:target>
  </bpmn:relationship>
</bpmn:definitions>
"#;

/// The state chart of `agents.open`'s example.
pub(crate) const EXAMPLE_SCXML: &str = r#"<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:stochcraft="https://helenkwok.github.io/StochCraft/scxml/1" version="1.0" datamodel="stochcraft" name="person">
  <state id="start">
    <transition cond="serial() = 0" target="heard"/>
    <transition target="unaware"/>
  </state>
  <state id="unaware">
    <transition event="tell" target="heard"/>
  </state>
  <state id="heard">
    <onentry><send event="talk" delayexpr="exponential(1 div (1))" stochcraft:rate="1"/></onentry>
    <transition event="talk">
      <send event="tell" type="https://helenkwok.github.io/StochCraft/scxml/1#agents" stochcraft:to="random" stochcraft:population="people"/>
      <send event="talk" delayexpr="exponential(1 div (1))" stochcraft:rate="1"/>
    </transition>
  </state>
</scxml>
"#;

/// The files an open agent-based model reads besides its own, and its last run.
pub(crate) struct AgentsDoc {
    /// Each file the model names, by its reference, as it was read.
    pub files: BTreeMap<String, Vec<u8>>,
    /// The texts `agents.open` was given, by their references: what another of the file's
    /// scenarios reads in place of files beside it (`file.reopen`). Empty for a model opened
    /// from disk.
    pub given: BTreeMap<String, Vec<u8>>,
    /// The last run's results, and the engine's revision when it ran.
    pub last: Option<(Value, u64)>,
}

/// Whether `href` is a reference StochCraft reads beside the model: an SCXML chart (`.scxml`),
/// a CSV list of links (`.csv`) or a DMN file of decision tables (`.dmn`), relative, in the
/// model's folder or below it (no `..`, no root, drive or scheme, and no part that Windows takes
/// for a device, such as `nul`).
pub fn is_safe_reference(href: &str) -> bool {
    let lower = href.to_ascii_lowercase();
    let device = |part: &str| {
        let stem = part
            .split('.')
            .next()
            .unwrap_or(part)
            .trim_end()
            .to_ascii_lowercase();
        let numbered = (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.len() == 4
            && stem.chars().nth(3).is_some_and(|c| c.is_ascii_digit());
        numbered || matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
    };
    (lower.ends_with(".scxml") || lower.ends_with(".csv") || lower.ends_with(".dmn"))
        && !href.starts_with(['/', '\\'])
        && !href.contains(':')
        && href
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != "." && part != ".." && !device(part))
}

/// The file `href` names beside a model in `dir`, once links are followed: refused if it leads
/// out of `dir` (through a link to elsewhere), so a model reads and writes files only in its own
/// folder and below. For a file that does not exist yet, its nearest existing folder is checked.
pub fn beside(dir: &std::path::Path, href: &str) -> Result<std::path::PathBuf, String> {
    if !is_safe_reference(href) {
        return Err(format!(
            "`{href}` is not a relative reference to an .scxml, .csv or .dmn file beside the model"
        ));
    }
    let dir = if dir.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        dir
    };
    let base = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = base.join(href);
    let mut existing = path.as_path();
    while !existing.exists() {
        match existing.parent() {
            Some(p) => existing = p,
            None => break,
        }
    }
    let real = std::fs::canonicalize(existing).map_err(|e| format!("{href}: {e}"))?;
    if real.starts_with(&base) {
        Ok(path)
    } else {
        Err(format!("`{href}` leads out of the model's folder"))
    }
}

pub(crate) fn no_model() -> CommandError {
    CommandError::new(
        ErrorCode::NotFound,
        "no agent-based model is open: open one with file.open, or give agents.open its text",
    )
}

fn model_error(e: ModelError) -> CommandError {
    match e {
        ModelError::Invalid(c) => CommandError::new(ErrorCode::InvalidModel, c.to_string()),
        ModelError::Run(r) => {
            CommandError::new(ErrorCode::InvalidModel, format!("the run stopped: {r}"))
        }
    }
}

/// The charts and lists of links of `files`, read in the time unit `seconds` long.
fn read_files(
    agents: &Agents,
    files: &BTreeMap<String, Vec<u8>>,
    seconds: f64,
) -> Result<Files, CommandError> {
    let mut out = Files::default();
    let get = |href: &str| {
        files.get(href).ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                format!("the file `{href}` the model names was not read"),
            )
        })
    };
    for t in &agents.types {
        if out.charts.contains_key(&t.chart) {
            continue;
        }
        let chart: ChartSpec = stochcraft_io::scxml::read_scxml(get(&t.chart)?, seconds)
            .map_err(|e| CommandError::new(ErrorCode::BadFile, format!("{}: {e}", t.chart)))?;
        out.charts.insert(t.chart.clone(), chart);
    }
    for p in &agents.populations {
        if let Some(NetworkSpec::File { href }) = &p.network {
            let text = std::str::from_utf8(get(href)?).map_err(|_| {
                CommandError::new(ErrorCode::BadFile, format!("{href}: not UTF-8 text"))
            })?;
            out.links.insert(href.clone(), text.to_owned());
        }
    }
    Ok(out)
}

impl Engine {
    /// The agent-based part of the open model, if it is one.
    fn agents_model(&self) -> Option<&Agents> {
        self.model
            .agents
            .as_ref()
            .filter(|_| self.model.is_agents())
    }

    /// The files an open agent-based model reads besides its own, by their references (empty
    /// for any other model): what `file.save` writes beside the model.
    pub fn agent_files(&self) -> Vec<(String, Vec<u8>)> {
        match (&self.agents, self.agents_model()) {
            (Some(d), Some(_)) => d
                .files
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// The digest that identifies an agent-based model: of the model, its populations and the
    /// bytes of every file it reads.
    pub(crate) fn agents_digest(&self) -> String {
        let mut text = crate::model_digest(&self.model);
        if let Some(a) = &self.model.agents {
            text.push('\n');
            text.push_str(&stochcraft_io::agents::text(a));
        }
        if let Some(d) = &self.agents {
            for (href, bytes) in &d.files {
                text.push_str(&format!("\n{href} {}", crate::bytes_digest(bytes)));
            }
        }
        crate::bytes_digest(text.as_bytes())
    }

    /// Opens a model file from its bytes, reading the files an agent-based model names with
    /// `resolve` (by their references), and replaces the document as [`Engine::open_bytes`]
    /// does. A process or Monte Carlo model names no files and opens as with `open_bytes`.
    pub fn open_bytes_with(
        &mut self,
        name: &str,
        bytes: &[u8],
        scenario: Option<&str>,
        resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<Value, CommandError> {
        if stochcraft_sd::looks_like_xmile(bytes) {
            return self.open_sd(name, bytes, true);
        }
        let mut loaded = stochcraft_io::read_scenario(bytes, scenario)
            .map_err(|e| CommandError::new(ErrorCode::BadFile, format!("{name}: {e}")))?;
        let mut files = BTreeMap::new();
        if let Some(a) = loaded
            .model
            .agents
            .as_ref()
            .filter(|_| loaded.model.is_agents())
        {
            for href in a.files() {
                if !is_safe_reference(href) {
                    return Err(CommandError::new(
                        ErrorCode::BadFile,
                        format!(
                            "{name}: `{href}` is not a relative reference to an .scxml or .csv file beside the model (no `..`, root, drive, scheme or device name)"
                        ),
                    ));
                }
                let got = resolve(href).map_err(|e| {
                    CommandError::new(ErrorCode::Io, format!("{name}: {href}: {e}"))
                })?;
                files.insert(href.to_owned(), got);
            }
            // Each chart must read now, so a model that opens is one that can be described.
            read_files(a, &files, loaded.model.experiment.time_unit.seconds()).map_err(
                |mut e| {
                    e.message = format!("{name}: {}", e.message);
                    e
                },
            )?;
        }
        let is_agents = loaded.model.is_agents();
        // A process model's decision tables (D30), from beside it.
        let dmn = if is_agents {
            BTreeMap::new()
        } else {
            crate::dmn::load(&mut loaded, resolve)
        };
        let result = self.open_loaded(name, bytes, loaded, is_agents)?;
        self.dmn = dmn;
        self.dmn_at = self.path().map(str::to_owned);
        if is_agents {
            self.agents = Some(AgentsDoc {
                files,
                given: BTreeMap::new(),
                last: None,
            });
            self.journal_start.model_digest = self.agents_digest();
        }
        Ok(result)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Open {
    /// The text of a BPMN file with the `stochcraft:agents` extension. (A file on disk is opened
    /// with `file.open`, which reads the files it names from beside it.)
    pub text: String,
    /// The text of each file the model names (each agent type's SCXML state chart, each CSV list
    /// of links), by the reference the model gives it.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    /// The name to report it under; by default `model.bpmn`.
    #[serde(default)]
    pub name: Option<String>,
    /// The scenario to open, if the file has several.
    #[serde(default)]
    pub scenario: Option<String>,
}

pub(crate) fn open(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: Open = parse(p)?;
    let name = p.name.unwrap_or_else(|| "model.bpmn".to_owned());
    let loaded = stochcraft_io::read_scenario(p.text.as_bytes(), p.scenario.as_deref())
        .map_err(|err| CommandError::new(ErrorCode::BadFile, format!("{name}: {err}")))?;
    if !loaded.model.is_agents() {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!(
                "{name} is not an agent-based model (a BPMN file with no process and the stochcraft:agents extension): open it with file.open"
            ),
        ));
    }
    let files = p.files;
    let result = e.open_bytes_with(
        &name,
        p.text.as_bytes(),
        p.scenario.as_deref(),
        &mut |href: &str| {
            files
                .get(href)
                .map(|t| t.as_bytes().to_vec())
                .ok_or_else(|| "not given in `files`".to_owned())
        },
    )?;
    // Opened from text, it has no file yet: `file.save` needs a path.
    if let Some(f) = e.file.as_mut() {
        f.path = None;
    }
    // Another of the file's scenarios reads its charts and lists from these texts too.
    if let Some(d) = e.agents.as_mut() {
        d.given = files
            .into_iter()
            .map(|(href, text)| (href, text.into_bytes()))
            .collect();
    }
    Ok(result)
}

/// The model as JSON: the run's settings, each agent type with its chart's states and variables,
/// the populations, the environment, the statistics, the stepped mode and the files it reads.
fn describe(e: &Engine, a: &Agents) -> Value {
    let x = &e.model.experiment;
    let charts = e
        .agents
        .as_ref()
        .map(|d| read_files(a, &d.files, x.time_unit.seconds()));
    let (charts, problem) = match charts {
        Some(Ok(f)) => (Some(f), None),
        Some(Err(err)) => (None, Some(err.message)),
        None => (None, Some("its files were not read".to_owned())),
    };
    let types: Vec<Value> = a
        .types
        .iter()
        .map(|t| {
            let mut o = json!({ "name": t.name, "chart": t.chart });
            if let Some(c) = charts.as_ref().and_then(|f| f.charts.get(&t.chart)) {
                let mut states = Vec::new();
                fn walk(s: &stochcraft_agents::StateSpec, out: &mut Vec<String>) {
                    out.push(s.id.clone());
                    for c in &s.children {
                        walk(c, out);
                    }
                }
                for s in &c.states {
                    walk(s, &mut states);
                }
                o["states"] = json!(states);
                o["variables"] = json!(
                    c.data
                        .iter()
                        .map(|d| json!({ "name": d.id, "first_value": d.expr }))
                        .collect::<Vec<_>>()
                );
            }
            o
        })
        .collect();
    let populations: Vec<Value> = a
        .populations
        .iter()
        .map(|p| {
            let mut o = json!({
                "name": p.name,
                "type": p.agent_type,
                "count": p.count,
                "arrival_rate": p.arrival_rate,
            });
            if let Some(n) = &p.network {
                o["network"] = network_json(n);
            }
            if let Some(g) = &p.grid {
                o["grid"] = json!({
                    "rows": g.rows,
                    "columns": g.columns,
                    "neighbourhood": if g.von_neumann { "von_neumann" } else { "moore" },
                    "wrap": g.wrap,
                    "per_cell": if g.many_per_cell { "many" } else { "one" },
                    "placement": if g.random_placement { "random" } else { "in_order" },
                });
            }
            o
        })
        .collect();
    let statistics: Vec<Value> = a
        .statistics
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "population": s.population,
                "measure": s.measure.file_name(),
                "expression": s.expression,
                "condition": s.condition,
            })
        })
        .collect();
    let problem = problem.or_else(|| {
        charts
            .as_ref()
            .and_then(|f| build(a, f).err())
            .map(|err| model_error(err).message)
    });
    json!({
        "name": e.model.name,
        "time_unit": x.time_unit.bpsim_name(),
        "end": x.warmup + x.duration,
        "replications": x.replications,
        "seed": x.seed,
        "save_every": a.save_every,
        "types": types,
        "populations": populations,
        "environment": a.environment,
        "statistics": statistics,
        "stepping": a.stepping.map(|s| json!({
            "every": s.every,
            "order": if s.random_order { "random" } else { "fixed" },
        })),
        "microstep_limit": a.microstep_limit,
        "retest_limit": a.retest_limit,
        "files": a.files(),
        "problem": problem,
    })
}

fn network_json(n: &NetworkSpec) -> Value {
    let mut o = json!({ "kind": n.file_name() });
    match n {
        NetworkSpec::Empty => {}
        NetworkSpec::Random { mean_degree } => o["mean_degree"] = json!(mean_degree),
        NetworkSpec::Ring { each_side } => o["each_side"] = json!(each_side),
        NetworkSpec::SmallWorld { each_side, rewire } => {
            o["each_side"] = json!(each_side);
            o["rewire"] = json!(rewire);
        }
        NetworkSpec::ScaleFree { initial, links } => {
            o["initial"] = json!(initial);
            o["links"] = json!(links);
        }
        NetworkSpec::Distance {
            width,
            height,
            range,
        } => {
            o["width"] = json!(width);
            o["height"] = json!(height);
            o["range"] = json!(range);
        }
        NetworkSpec::Links(l) => o["links"] = json!(l),
        NetworkSpec::File { href } => o["href"] = json!(href),
    }
    o
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Get {}

pub(crate) fn get(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let _: Get = parse(p)?;
    let Some(a) = e.agents_model() else {
        return Err(no_model());
    };
    let mut out = describe(e, a);
    let last = e.agents.as_ref().and_then(|d| d.last.as_ref());
    out["has_results"] = json!(last.is_some());
    out["results_stale"] = json!(last.is_some_and(|(_, rev)| *rev != e.revision));
    Ok(out)
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunParams {
    /// Replications in place of the experiment's (at most 100,000).
    #[serde(default)]
    pub replications: Option<u64>,
    /// The seed in place of the experiment's.
    #[serde(default)]
    pub seed: Option<u64>,
    /// The end time in place of the experiment's warm-up plus duration, in the model's time
    /// unit: no earlier than the warm-up.
    #[serde(default)]
    pub end: Option<f64>,
}

/// The base of replication `r`'s random streams: depends only on the seed and `r`.
fn replication_seed(seed: u64, r: u64) -> u64 {
    Rng::keyed(seed, &format!("replication/{r}")).next_u64()
}

fn finite(v: f64) -> Value {
    if v.is_finite() { json!(v) } else { Value::Null }
}

fn floats(v: &[f64]) -> Value {
    Value::Array(v.iter().map(|x| finite(*x)).collect())
}

pub(crate) fn run(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: RunParams = parse(p)?;
    let Some(a) = e.agents_model() else {
        return Err(no_model());
    };
    let Some(doc) = e.agents.as_ref() else {
        return Err(no_model());
    };
    // The experiment's warm-up and duration, each checked as for a process model.
    let problems: Vec<_> = e
        .model
        .validate()
        .into_iter()
        .filter(|d| d.severity == stochcraft_model::Severity::Error)
        .collect();
    if !problems.is_empty() {
        return Err(CommandError {
            code: ErrorCode::InvalidModel,
            message: "the model has errors and cannot run".into(),
            details: Some(crate::wire::diagnostics(&e.model, &problems)),
        });
    }
    let x = &e.model.experiment;
    let replications = p.replications.unwrap_or(u64::from(x.replications));
    if replications == 0 || replications > MAX_REPLICATIONS {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!("replications must be from 1 to {MAX_REPLICATIONS}"),
        ));
    }
    let seed = p.seed.unwrap_or(x.seed);
    let warmup = x.warmup;
    let end = p.end.unwrap_or(x.warmup + x.duration);
    if !(end.is_finite() && end >= 0.0) {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!("the end time {end} is not a time of 0 or more"),
        ));
    }
    if end < warmup {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!("the end time {end} is before the warm-up ends at {warmup}"),
        ));
    }
    let files = read_files(a, &doc.files, x.time_unit.seconds())?;
    let model = build(a, &files).map_err(model_error)?;
    let mut runs: Vec<Run> = Vec::new();
    for r in 0..replications {
        let run = model
            .run_with_warmup(replication_seed(seed, r), warmup, end)
            .map_err(|err| {
                let mut c = model_error(err);
                c.message = format!("replication {}: {}", r + 1, c.message);
                c.details = Some(json!({ "replication": r + 1 }));
                c
            })?;
        runs.push(run);
    }
    let result = results_json(e, a, &runs, replications, seed, warmup, end);
    let revision = e.revision;
    if let Some(d) = e.agents.as_mut() {
        d.last = Some((result.clone(), revision));
    }
    Ok(result)
}

/// A run's results as JSON: the times; each statistic's mean at each (with, over more than one
/// replication, the median and the 5th and 95th percentiles); each population's counts, as means
/// over the replications; and the run record.
fn results_json(
    e: &Engine,
    a: &Agents,
    runs: &[Run],
    replications: u64,
    seed: u64,
    warmup: f64,
    end: f64,
) -> Value {
    let b = bands(runs);
    let many = replications > 1;
    let statistics: Vec<Value> = b
        .series
        .iter()
        .map(|s| {
            let spec = a.statistics.iter().find(|x| x.name == s.name);
            let mut o = json!({
                "name": s.name,
                "population": spec.map(|x| x.population.clone()),
                "measure": spec.map(|x| x.measure.file_name()),
                "final": s.mean.last().map_or(Value::Null, |v| finite(*v)),
                "mean": floats(&s.mean),
            });
            if many {
                o["median"] = floats(&s.median);
                o["p5"] = floats(&s.low);
                o["p95"] = floats(&s.high);
            }
            o
        })
        .collect();
    let n = runs.len().max(1) as f64;
    let populations: Vec<Value> = a
        .populations
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let sum = |f: &dyn Fn(&stochcraft_agents::Counts) -> u64| -> u64 {
                runs.iter().filter_map(|r| r.counts.get(i)).map(f).sum()
            };
            json!({
                "name": p.name,
                "created_mean": sum(&|c| c.created) as f64 / n,
                "removed_mean": sum(&|c| c.removed) as f64 / n,
                "alive_at_end_mean": sum(&|c| c.alive) as f64 / n,
                "messages_dropped": sum(&|c| c.dropped),
                "messages_undelivered": sum(&|c| c.undelivered),
            })
        })
        .collect();
    let uncertainty = if many {
        "mean, median and the 5th and 95th percentiles across independent replications at each recorded time (Hyndman and Fan's definition 7)"
    } else {
        "one replication: no spread"
    };
    let x = &e.model.experiment;
    json!({
        "model": e.model.name,
        "time_unit": x.time_unit.bpsim_name(),
        "warmup": warmup,
        "end": end,
        "replications": replications,
        "seed": seed,
        "times": floats(&b.times),
        "statistics": statistics,
        "populations": populations,
        "record": {
            "stochcraft": env!("CARGO_PKG_VERSION"),
            "model_digest": e.agents_digest(),
            "file": e.path(),
            "seed": seed,
            "replications": replications,
            "times": "the statistics are recorded at the end of the warm-up (time 0 without one), at every save interval after it, and at the end, each after every event at that time; what happened before the warm-up ends, statistics and population counts alike, is discarded",
            "random_streams": "each replication's streams come from the seed and the replication's number; within one, each agent draws from streams keyed by its population, its serial number and the place in its chart, so adding an agent does not change another's draws (D18)",
            "uncertainty": uncertainty,
        },
    })
}

/// An agent-based run's results (as `agents.run` gives them) as CSV: a row per recorded time,
/// the time and then each statistic's mean, with its median and 5th and 95th percentiles after
/// it where there was more than one replication. This is what `stochcraft-cli run --format csv`
/// writes for an agent-based model.
pub fn results_csv(r: &Value) -> String {
    let stats: Vec<&Value> = r["statistics"].as_array().into_iter().flatten().collect();
    let many = r["replications"].as_u64().unwrap_or(1) > 1;
    let name = |v: &Value| crate::wire::text_cell(v["name"].as_str().unwrap_or(""));
    let mut header = vec![format!("time ({})", r["time_unit"].as_str().unwrap_or(""))];
    for v in &stats {
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
        for v in &stats {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_references_beside_the_model_are_read() {
        for ok in [
            "person.scxml",
            "charts/person.scxml",
            "a b.scxml",
            "rules/sorting.dmn",
            "x\\y.csv",
        ] {
            assert!(is_safe_reference(ok), "{ok}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "C:/x.scxml",
            "../x.scxml",
            "a/../../x",
            "./x",
            "a//b",
            "https://example.org/x.scxml",
            "../x.scxml",
            "charts/../../x.csv",
            "nul.scxml",
            "charts/COM1.csv",
            ".cargo/config.toml",
            "person.xml",
        ] {
            assert!(!is_safe_reference(bad), "{bad}");
        }
    }
}
