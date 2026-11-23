//! The engine: every change to a model and every run is a command.
//!
//! The command line, the desktop app and agents over MCP all drive a model the same way, by name
//! and JSON parameters:
//!
//! ```
//! use serde_json::json;
//! use stochcraft_engine::Engine;
//!
//! let mut engine = Engine::new();
//! let arrivals = engine.execute("block.add", json!({
//!     "kind": "source", "id": "arrivals",
//!     "interarrival": {"type": "exponential", "mean": 2.0}
//! }))?;
//! engine.execute("block.add", json!({"kind": "server", "id": "desk"}))?;
//! engine.execute("block.add", json!({"kind": "sink", "id": "exit"}))?;
//! engine.execute("connect", json!({"from": "arrivals", "to": "desk"}))?;
//! engine.execute("connect", json!({"from": "desk", "to": "exit"}))?;
//! engine.execute("experiment.set", json!({"replications": 4, "duration": 200.0}))?;
//! let results = engine.execute("run", json!({}))?;
//! assert_eq!(results["replications"], 4);
//!
//! engine.execute("undo", json!({}))?; // back to the default experiment
//! assert_eq!(engine.model().experiment.replications, 10);
//! # Ok::<(), stochcraft_engine::CommandError>(())
//! ```
//!
//! - [`COMMANDS`] lists every command with a summary, the JSON schema of its parameters and an
//!   example: what the CLI's `schema` command and the MCP tool list are built from.
//! - Edits are undone and redone through their exact inverses ([`CommandKind::Edit`]).
//! - Every command that is not a query is recorded in the [`Engine::journal`], which
//!   [`Engine::replay`] runs again as a script. A new or opened document starts a new journal,
//!   and [`Engine::journal_document`] gives it with its format version, the StochCraft version
//!   and the state it started from ([`Journal`]). [`Engine::replay_journal`] checks that state
//!   and by default re-applies only the edits, leaving out saves, runs and experiments.
//! - `run` runs an experiment at once; `run.start`, `run.status` and `run.cancel` run one in the
//!   background with progress and cancellation (worker threads natively; in the browser, the
//!   page's web workers, see [`Worker`], or slices of [`SLICE_EVENTS`] events between frames).
//! - A Monte Carlo model (a calculation with no process: inputs that are distributions, outputs
//!   that are expressions over them) is edited with the `mc.*` commands and run with `mc.run`
//!   (see [`mc`]); `run` refuses one. `mc.sobol` gives each input's share of an output's variance.
//! - A system dynamics model (stocks and flows over time) is an XMILE file: `file.open` or `sd.open`
//!   opens it, `sd.run` runs it (once, or in replications when it has a random function, giving
//!   the mean, median and 5% to 95% band of every variable over time) and `sd.get` describes it
//!   (see [`sd`]); `run` refuses one. `sd.new` starts an empty one and `sd.variable.add`, `.set`,
//!   `.remove`, `sd.specs.set` and `sd.layout.set` edit it, on the same undo and redo as any other
//!   edit; `file.save` writes it as an XMILE file.
//! - An agent-based model (populations of agents with state charts) is a BPMN file with no
//!   process and the `stochcraft:agents` extension, beside an SCXML file per agent type:
//!   `file.open` or `agents.open` opens it, `agents.run` runs it (each statistic over time, with
//!   its band across replications) and `agents.get` describes it (see [`agents`]); `run` refuses
//!   one, and `file.save` writes the charts beside the model.
//! - Files are BPSim on BPMN or XPDL (`stochcraft-io`). `file.save` writes edits into the file
//!   the model was opened from, keeping everything StochCraft does not understand and every line
//!   it did not change; saved unchanged, the file comes back byte for byte. An edit that cannot
//!   be written into that file is refused with the reason, and `as_new` then writes a new BPMN
//!   file from the model alone.

pub mod agents;
mod commands;
mod compare;
mod decide;
mod design;
mod distribution;
mod drawn;
mod edit;
mod evaluate;
mod fit;
pub mod mc;
pub mod observed;
mod optimize;
pub mod precision;
mod run;
pub mod sd;
mod sd_edit;
mod sd_window;
pub mod wire;
mod workers;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_blocks::compile;
use stochcraft_experiment::Experiment;
use stochcraft_io::{Format, Opened};
use stochcraft_model::Model;

pub use commands::{COMMANDS, Command, CommandKind};
pub use run::{RunMode, SLICE_EVENTS};
pub use stochcraft_blocks::{Live, LiveState, RunError};
pub use workers::Worker;

use edit::Op;
use run::{Job, Plan};

/// Why a command failed. `code` is stable; `message` is for people.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
    /// More detail, such as the diagnostics of a model that cannot run.
    pub details: Option<Value>,
}

/// The kind of failure, for programs and agents to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    UnknownCommand,
    InvalidParams,
    NotFound,
    /// An id is already used, or a run is already in progress.
    Conflict,
    NothingToUndo,
    NothingToRedo,
    /// The model has errors and cannot run; `details` holds the diagnostics.
    InvalidModel,
    Io,
    /// The file could not be read as a model.
    BadFile,
    Unsupported,
    Cancelled,
    /// A replication stopped because simulated time stopped moving (entities going round a loop
    /// whose steps all take no time); `details` names the element and the time.
    TimeStopped,
    Internal,
}

impl ErrorCode {
    pub fn name(self) -> &'static str {
        match self {
            ErrorCode::UnknownCommand => "unknown_command",
            ErrorCode::InvalidParams => "invalid_params",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Conflict => "conflict",
            ErrorCode::NothingToUndo => "nothing_to_undo",
            ErrorCode::NothingToRedo => "nothing_to_redo",
            ErrorCode::InvalidModel => "invalid_model",
            ErrorCode::Io => "io",
            ErrorCode::BadFile => "bad_file",
            ErrorCode::Unsupported => "unsupported",
            ErrorCode::Cancelled => "cancelled",
            ErrorCode::TimeStopped => "time_stopped",
            ErrorCode::Internal => "internal",
        }
    }

    const ALL: [ErrorCode; 13] = [
        ErrorCode::UnknownCommand,
        ErrorCode::InvalidParams,
        ErrorCode::NotFound,
        ErrorCode::Conflict,
        ErrorCode::NothingToUndo,
        ErrorCode::NothingToRedo,
        ErrorCode::InvalidModel,
        ErrorCode::Io,
        ErrorCode::BadFile,
        ErrorCode::Unsupported,
        ErrorCode::Cancelled,
        ErrorCode::TimeStopped,
        ErrorCode::Internal,
    ];

    /// The code with this [`ErrorCode::name`].
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({ "code": self.code.name(), "message": self.message });
        if let Some(d) = &self.details {
            v["details"] = d.clone();
        }
        v
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.name(), self.message)
    }
}

impl std::error::Error for CommandError {}

/// One command as it was executed: the journal's entries and a script's steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub command: String,
    #[serde(default)]
    pub params: Value,
}

const DOCUMENT_PROCESS: &str = "process";
const DOCUMENT_SD: &str = "system_dynamics";
const DOCUMENT_AGENTS: &str = "agents";

/// The version of the journal format this engine writes and reads (see [`Journal`]).
pub const JOURNAL_FORMAT: u32 = 1;

/// How the document a journal belongs to began.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartKind {
    /// The engine was made with a model ([`Engine::new`], [`Engine::with_model`]) and no
    /// command started a document since.
    Engine,
    /// `model.new` started a new, empty model.
    New,
    /// A file was opened (`file.open`, or [`Engine::open_bytes`]).
    File,
    /// `sd.new` started a new, empty system dynamics model; its settings are the journal
    /// start's `params`.
    NewSd,
}

/// The state a journal started from, so a replay can tell whether it starts from the same place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalStart {
    pub kind: StartKind,
    /// [`model_digest`] of the model at the start.
    pub model_digest: String,
    /// For [`StartKind::New`] the model's name; for [`StartKind::File`] the name the file was
    /// opened under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// For [`StartKind::File`], the digest (`sha256:` and hex) of the file's bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_digest: Option<String>,
    /// For [`StartKind::File`], the scenario that was opened, if the file has scenarios.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario: Option<String>,
    /// For [`StartKind::NewSd`], the parameters `sd.new` was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    /// What kind of document it was: `"process"` or `"system_dynamics"`. A replay refuses a
    /// document of the other kind. Absent in a journal written before this was recorded, whose
    /// system dynamics models were identified by a placeholder digest (see
    /// [`Engine::begin_replay`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
}

impl JournalStart {
    /// A start of `kind`; the model's digest is filled in by the engine when the document begins.
    fn new(kind: StartKind, name: Option<String>) -> Self {
        Self {
            kind,
            model_digest: String::new(),
            name,
            file_digest: None,
            scenario: None,
            params: None,
            document: None,
        }
    }
}

/// A journal as it is written to a file: what it started from, then its commands. A new or
/// opened document starts a new journal, so it is one document's history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Journal {
    /// The journal format ([`JOURNAL_FORMAT`]).
    pub format: u32,
    /// The StochCraft version that wrote it.
    pub stochcraft: String,
    pub start: JournalStart,
    pub entries: Vec<JournalEntry>,
    /// The build of StochCraft that wrote it ([`build_id`]): the version, the platform and, when
    /// the build was given one, the source revision. Numbers are reproduced only by the same
    /// build, so a replay says whether this one is it. Absent in older journals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
}

/// Names this build of the engine: the version, the platform and, when the environment variable
/// `STOCHCRAFT_REVISION` was set while building, the source revision.
pub fn build_id() -> String {
    let mut id = format!(
        "{} {}-{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    if let Some(revision) = option_env!("STOCHCRAFT_REVISION") {
        id.push(' ');
        id.push_str(revision);
    }
    id
}

/// Whether the build that wrote a journal (`journal`, a [`build_id`]) is the build `here`.
///
/// A version and a platform do not name a build: two builds of one version from different
/// commits can give different numbers from the same seed (a change to a sampler is enough). So
/// only two ids that both carry a source revision, and agree in full, are the same build; ids
/// that differ in version, platform or revision are different builds; anything else (an id with
/// no revision, or a journal with no id) is unknown.
pub fn build_match(journal: Option<&str>, here: &str) -> BuildMatch {
    let Some(journal) = journal else {
        return BuildMatch::Unknown;
    };
    // The version and platform, and the revision if there is one.
    type Parts<'a> = ((Option<&'a str>, Option<&'a str>), Option<&'a str>);
    fn split(id: &str) -> Parts<'_> {
        let mut words = id.split_whitespace();
        let base = (words.next(), words.next());
        (base, words.next())
    }
    let ((base, revision), (here_base, here_revision)) = (split(journal), split(here));
    match (revision, here_revision) {
        _ if base != here_base => BuildMatch::Different,
        (Some(a), Some(b)) if a == b => BuildMatch::Same,
        (Some(_), Some(_)) => BuildMatch::Different,
        _ => BuildMatch::Unknown,
    }
}

/// What [`build_match`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildMatch {
    /// The same version, platform and source revision.
    Same,
    /// A different version, platform or source revision.
    Different,
    /// Not known: a build id without a source revision, or no id at all.
    Unknown,
}

impl BuildMatch {
    /// Its name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            BuildMatch::Same => "same",
            BuildMatch::Different => "different",
            BuildMatch::Unknown => "unknown",
        }
    }
}

/// Which steps of a journal a replay carries out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayMode {
    /// Re-applies the edits (and undo, redo and starting a document). Saving a file, runs and
    /// experiments are skipped: they write files or take time, and change nothing in the model
    /// (an optimisation that applied its answer is journalled with the change as a separate
    /// `decisions.set` edit, which is replayed).
    #[default]
    EditsOnly,
    /// Carries out every step, as [`Engine::replay`] does.
    All,
}

/// The commands [`ReplayMode::EditsOnly`] skips.
const SKIPPED_BY_DEFAULT: [&str; 14] = [
    "file.save",
    "run",
    "mc.run",
    "mc.sobol",
    "sd.run",
    "agents.run",
    "run.start",
    "run.cancel",
    "experiment.sweep",
    "experiment.sensitivity",
    "experiment.design",
    "experiment.size",
    "experiment.evaluate",
    "experiment.optimise",
];

impl ReplayMode {
    /// Whether a replay in this mode carries out `command`.
    pub fn applies(self, command: &str) -> bool {
        self == ReplayMode::All || !SKIPPED_BY_DEFAULT.contains(&command)
    }
}

/// How a step that ran is kept in the journal. An optimisation that applied its answer is kept
/// as the search without `apply`, then its change as a `decisions.set` edit, so an edits-only
/// replay makes the change without searching again and a full replay searches once and applies
/// once. `experiment.size` is kept the same way. A batch keeps its steps the same way, one by one with their results, nested batches
/// included, and stays one step (one undo step when replayed).
fn journalled(command: &str, params: Value, result: &Value) -> Vec<JournalEntry> {
    let entry = |command: &str, params: Value| JournalEntry {
        command: command.to_owned(),
        params,
    };
    match command {
        "experiment.optimise" | "experiment.size"
            if result.get("applied") == Some(&json!(true)) =>
        {
            let decisions = if command == "experiment.size" {
                result["applied_decisions"].clone()
            } else {
                result["chosen"]["decisions"].clone()
            };
            let mut search = params;
            search["apply"] = json!(false);
            vec![
                entry(command, search),
                entry("decisions.set", json!({ "decisions": decisions })),
            ]
        }
        "batch" => {
            let mut params = params;
            let results = result["results"].as_array().cloned().unwrap_or_default();
            if let Some(steps) = params.get("commands").and_then(Value::as_array) {
                let mut kept = Vec::new();
                for (i, step) in steps.iter().enumerate() {
                    let name = step["command"].as_str().unwrap_or_default();
                    let inner = if step["params"].is_null() {
                        json!({})
                    } else {
                        step["params"].clone()
                    };
                    let r = results.get(i).cloned().unwrap_or(Value::Null);
                    let applied = r.get("applied") == Some(&json!(true));
                    let analysis = matches!(name, "experiment.optimise" | "experiment.size");
                    if name == "batch" || (analysis && applied) {
                        for e in journalled(name, inner, &r) {
                            kept.push(json!({ "command": e.command, "params": e.params }));
                        }
                    } else {
                        // Every other step is kept as it was given.
                        kept.push(step.clone());
                    }
                }
                params["commands"] = Value::Array(kept);
            }
            vec![entry(command, params)]
        }
        _ => vec![entry(command, params)],
    }
}

/// A batch's parameters with the steps `mode` skips left out, nested batches included.
fn batch_for(mode: ReplayMode, params: &Value) -> Value {
    let mut params = params.clone();
    if let Some(steps) = params.get("commands").and_then(Value::as_array) {
        let kept: Vec<Value> = steps
            .iter()
            .filter(|s| {
                let name = s["command"].as_str().unwrap_or_default();
                // A sizing or an optimisation recorded with `apply` true comes from a journal
                // written before applied changes were kept as their own `decisions.set` step:
                // it is replayed as it was then, searching again and applying.
                let older = matches!(name, "experiment.size" | "experiment.optimise")
                    && s["params"]["apply"] == json!(true);
                mode.applies(name) || older
            })
            .map(|s| {
                let mut s = s.clone();
                if s["command"] == "batch" {
                    s["params"] = batch_for(mode, &s["params"]);
                }
                s
            })
            .collect();
        params["commands"] = Value::Array(kept);
    }
    params
}

/// What a replay of `journal` in `mode` claims, separating the edits it re-applies from the
/// numerical results it makes again. Re-running a step does not compare its numbers with the
/// original run's (a journal keeps no results), and numbers are only expected to be the same
/// from the same build, so this says which build wrote the journal and whether it is this one.
pub fn replay_summary(journal: &Journal, mode: ReplayMode) -> Value {
    let produces_results = |c: &str| c != "file.save" && SKIPPED_BY_DEFAULT.contains(&c);
    let (mut edits, mut runs, mut skipped) = (0u64, 0u64, 0u64);
    for step in &journal.entries {
        if !mode.applies(&step.command) {
            skipped += 1;
        } else if produces_results(&step.command) {
            runs += 1;
        } else {
            edits += 1;
        }
    }
    let here = build_id();
    let matched = build_match(journal.engine.as_deref(), &here);
    let numbers = match matched {
        _ if runs == 0 => "none: no run was made again, only the edits were replayed",
        BuildMatch::Same => {
            "made again by the same build that wrote the journal (the same version, platform and source revision); they are not compared with the original numbers"
        }
        BuildMatch::Different => {
            "made again by a different build than the one that wrote the journal, so they may differ from the original numbers"
        }
        BuildMatch::Unknown => {
            "made again by a build that may not be the one that wrote the journal (build match unknown: a source revision is missing, and a version and platform alone do not name a build), so they may differ from the original numbers"
        }
    };
    json!({
        "replay": {
            "edits_replayed": edits,
            "runs_made_again": runs,
            "steps_skipped": skipped,
            "journal_engine": journal.engine,
            "this_engine": here,
            "build_match": matched.name(),
            "numerical_results": numbers,
        }
    })
}

/// How [`Engine::replay_journal`] replays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReplayOptions {
    pub mode: ReplayMode,
    /// Replays although the model is not the one the journal started from.
    pub force: bool,
}

/// What a replay did with one step.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplayStep {
    /// The command ran, with this result.
    Applied(Value),
    /// The mode left the command out.
    Skipped,
}

/// A script step that failed during [`Engine::replay`], or the reason a journal was refused.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayError {
    /// The position of the step in the script.
    pub index: usize,
    /// The step's command; empty when the journal itself was refused (its format, or a
    /// starting state that is not the model's).
    pub command: String,
    pub error: CommandError,
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.command.is_empty() {
            write!(f, "{}", self.error)
        } else {
            write!(f, "step {} ({}): {}", self.index, self.command, self.error)
        }
    }
}

impl std::error::Error for ReplayError {}

/// What undoes (or redoes) one edit: an operation on the process model, or the whole system
/// dynamics model as it was (those are small, so an edit keeps a snapshot, as `Op::SetModel`
/// does for a process model).
// An entry was a whole `Op` before system dynamics joined the history, so `Model` keeps that size.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum Undo {
    Model(Op),
    Sd(Box<stochcraft_sd::SdModel>),
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    label: &'static str,
    op: Undo,
}

/// The file a model belongs to.
#[derive(Debug, Clone)]
struct FileState {
    path: Option<String>,
    opened: Opened,
    /// What StochCraft noted when reading the file, for run records.
    notes: Vec<Value>,
}

/// A model, its undo history, its journal, its file and its background run.
pub struct Engine {
    model: Model,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    journal: Vec<JournalEntry>,
    /// What the journal started from.
    journal_start: JournalStart,
    /// Counts the documents started (a new or opened model), so `execute` can tell that a
    /// command started one and left the journal to its start record.
    documents: u64,
    file: Option<FileState>,
    /// The system dynamics model, if the document is one (see [`sd`]).
    sd: Option<sd::SdDoc>,
    /// The files an agent-based model reads beside its own, and its last run (see [`agents`]).
    agents: Option<agents::AgentsDoc>,
    /// The model as last opened or saved, to tell whether there are unsaved changes.
    saved: Model,
    job: Option<Job>,
    /// The background run's results have been kept (`poll_run` reports them at every poll).
    job_kept: bool,
    /// The results of the last run that finished, for `results.distribution`.
    last_results: Option<Value>,
    /// The results of the run that finished before it, for `results.compare`.
    previous_results: Option<Value>,
    /// The sweeps, sensitivity analyses and designed experiments done on the model, carried in
    /// the run record
    /// (NASA-STD-7009B [M&S 30]).
    sensitivity: Vec<Value>,
    /// Counts changes to the model (edits, undo, redo, a new or opened document, a save that
    /// fixes its shapes), so a view can tell it changed without comparing it.
    revision: u64,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// An engine with a new, empty model.
    pub fn new() -> Self {
        Self::with_model(Model::default())
    }

    /// An engine for `model`, as if it had just been saved.
    pub fn with_model(model: Model) -> Self {
        let mut journal_start = JournalStart::new(StartKind::Engine, None);
        journal_start.model_digest = model_digest(&model);
        journal_start.document = Some(DOCUMENT_PROCESS.to_owned());
        Self {
            saved: model.clone(),
            model,
            undo: Vec::new(),
            redo: Vec::new(),
            journal: Vec::new(),
            journal_start,
            documents: 0,
            file: None,
            sd: None,
            agents: None,
            job: None,
            job_kept: false,
            last_results: None,
            previous_results: None,
            sensitivity: Vec::new(),
            revision: 0,
        }
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// A number that changes whenever the model may have changed (an edit, undo, redo, a new or
    /// opened model) and never otherwise, so a view can redo work only then. Not saved, and not
    /// a count of edits: it only ever grows.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Runs `command` with JSON `params` (`null` counts as `{}`). Every command that is not a
    /// query, and succeeds, is added to the journal, except `model.new` and `file.open`: they
    /// start a new journal, whose [`JournalStart`] records them.
    pub fn execute(&mut self, command: &str, params: Value) -> Result<Value, CommandError> {
        let params = if params.is_null() { json!({}) } else { params };
        let c = COMMANDS.iter().find(|c| c.name == command).ok_or_else(|| {
            CommandError::new(ErrorCode::UnknownCommand, format!("no command `{command}`"))
        })?;
        let documents = self.documents;
        let result = (c.run)(self, params.clone())?;
        // Keep the last two results, for questions about their distributions and to compare
        // them (a background run's are kept as it finishes, in `poll_run`).
        if command == "run" {
            self.keep_results(result.clone());
        }
        if c.kind != CommandKind::Query && self.documents == documents {
            self.journal.extend(journalled(command, params, &result));
        }
        Ok(result)
    }

    /// Every command executed since the document was started that was not a query, in order.
    /// A new or opened document starts a new journal (see [`Engine::journal_start`]).
    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }

    /// What the journal started from: the engine's own model, a new model or an opened file.
    pub fn journal_start(&self) -> &JournalStart {
        &self.journal_start
    }

    /// The journal with its format version, the StochCraft version and its starting state: what
    /// is written to a file and replayed with [`Engine::replay_journal`].
    pub fn journal_document(&self) -> Journal {
        Journal {
            format: JOURNAL_FORMAT,
            stochcraft: env!("CARGO_PKG_VERSION").to_owned(),
            start: self.journal_start.clone(),
            entries: self.journal.clone(),
            engine: Some(build_id()),
        }
    }

    /// Executes each step of `script` in turn, stopping at the first that fails. Every step is
    /// carried out, saves and runs included, and nothing is checked about the starting state:
    /// see [`Engine::replay_journal`] for a journal.
    pub fn replay(&mut self, script: &[JournalEntry]) -> Result<Vec<Value>, ReplayError> {
        script
            .iter()
            .enumerate()
            .map(|(index, step)| {
                self.execute(&step.command, step.params.clone())
                    .inspect(|_| self.finish_started_run(&step.command))
                    .map_err(|error| ReplayError {
                        index,
                        command: step.command.clone(),
                        error,
                    })
            })
            .collect()
    }

    /// After a replayed `run.start`, makes the run complete (a replay has no one to poll it), so
    /// the steps after it find its results, as they did when the journal was recorded.
    pub fn finish_started_run(&mut self, command: &str) {
        if command != "run.start" {
            return;
        }
        loop {
            let status = self.poll_run(u64::MAX, u64::MAX);
            // A run for the caller's own workers waits for their answers: not ours to finish.
            if status["state"] != "running" || self.worker_task().is_some() {
                return;
            }
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// Like [`Engine::replay`], but leaves out the steps `mode` skips and says which.
    pub fn replay_with(
        &mut self,
        script: &[JournalEntry],
        mode: ReplayMode,
    ) -> Result<Vec<ReplayStep>, ReplayError> {
        script
            .iter()
            .enumerate()
            .map(|(index, step)| {
                if !mode.applies(&step.command) {
                    return Ok(ReplayStep::Skipped);
                }
                // A batch replayed for its edits leaves out the steps the mode skips, so an
                // optimisation inside it is not searched again (its change is a
                // `decisions.set` step of the batch).
                let params = if step.command == "batch" {
                    batch_for(mode, &step.params)
                } else {
                    step.params.clone()
                };
                self.execute(&step.command, params)
                    .map(|result| {
                        self.finish_started_run(&step.command);
                        ReplayStep::Applied(result)
                    })
                    .map_err(|error| ReplayError {
                        index,
                        command: step.command.clone(),
                        error,
                    })
            })
            .collect()
    }

    /// Gets the engine ready to replay `journal`: refuses a format newer than this engine reads,
    /// starts a new model for a journal that began with `model.new`, and refuses (with
    /// [`ErrorCode::Conflict`]) a model that is not the one the journal started from, unless
    /// `force`. The steps are then run with [`Engine::replay_with`].
    pub fn begin_replay(&mut self, journal: &Journal, force: bool) -> Result<(), ReplayError> {
        let refuse = |error: CommandError| ReplayError {
            index: 0,
            command: String::new(),
            error,
        };
        if journal.format > JOURNAL_FORMAT {
            return Err(refuse(CommandError::new(
                ErrorCode::Unsupported,
                format!(
                    "the journal is format {}, newer than the format {JOURNAL_FORMAT} this StochCraft reads",
                    journal.format
                ),
            )));
        }
        let start = &journal.start;
        if start.kind == StartKind::New {
            let name = start.name.clone().unwrap_or_default();
            self.execute("model.new", json!({ "name": name }))
                .map_err(refuse)?;
        }
        if start.kind == StartKind::NewSd {
            self.execute("sd.new", start.params.clone().unwrap_or_else(|| json!({})))
                .map_err(refuse)?;
        }
        if force {
            return Ok(());
        }
        // The kind of document, then which one: a journal of one kind is never replayed onto the
        // other, and a model that is not the one it started from is refused.
        if let Some(recorded) = &start.document
            && recorded != self.document_kind()
        {
            return Err(refuse(CommandError {
                code: ErrorCode::Conflict,
                message: format!(
                    "the journal is of a {} document, but the open document is a {} one; open the same file first, or force the replay",
                    recorded.replace('_', " "),
                    self.document_kind().replace('_', " ")
                ),
                details: Some(json!({ "expected": recorded, "found": self.document_kind() })),
            }));
        }
        let found = self.document_digest();
        let legacy_sd = start.document.is_none()
            && self.sd.is_some()
            && start.model_digest == model_digest(&Model::default());
        // A journal written before the document kind was recorded identified an XMILE model by
        // the digest of an empty process model, which says nothing about the model. A new
        // system dynamics model is rebuilt by its own `sd.new` step; an opened file cannot be
        // checked, so it is refused unless forced.
        if legacy_sd && start.kind != StartKind::NewSd {
            return Err(refuse(CommandError {
                code: ErrorCode::Conflict,
                message: "the journal was written before system dynamics models were identified in it, so it cannot tell whether the open model is the one it started from; force the replay to run it anyway".to_owned(),
                details: Some(json!({ "found": found })),
            }));
        }
        if found != start.model_digest && !legacy_sd {
            let from = match (start.kind, &start.name) {
                (StartKind::File, Some(name)) => format!("the file `{name}`"),
                (StartKind::New, _) => "a new model".to_owned(),
                (StartKind::NewSd, _) => "a new system dynamics model".to_owned(),
                _ => "the model the session began with".to_owned(),
            };
            return Err(refuse(CommandError {
                code: ErrorCode::Conflict,
                message: format!(
                    "the journal starts from {from} (model {}), but the model here is {found}; open the same file first, or force the replay",
                    start.model_digest
                ),
                details: Some(json!({ "expected": start.model_digest, "found": found })),
            }));
        }
        // The same model from different bytes: the file was edited (or is another file) since.
        if let (Some(recorded), Some(here)) = (&start.file_digest, &self.journal_start.file_digest)
            && recorded != here
        {
            return Err(refuse(CommandError {
                code: ErrorCode::Conflict,
                message: format!(
                    "the journal started from a file with bytes {recorded}, but the open file has {here}: it was edited, or is another file; open the same file, or force the replay"
                ),
                details: Some(json!({ "expected": recorded, "found": here })),
            }));
        }
        Ok(())
    }

    /// `"process"`, `"system_dynamics"` or `"agents"`: the kind of the open document.
    fn document_kind(&self) -> &'static str {
        if self.sd.is_some() {
            DOCUMENT_SD
        } else if self.agents.is_some() && self.model.is_agents() {
            DOCUMENT_AGENTS
        } else {
            DOCUMENT_PROCESS
        }
    }

    /// The digest that identifies the open document: [`model_digest`] of the process model, or
    /// the system dynamics model's own (without its diagram), or an agent-based model's (with
    /// its populations and the files it reads).
    fn document_digest(&self) -> String {
        match &self.sd {
            Some(d) => sd::sd_digest(&d.model),
            None if self.document_kind() == DOCUMENT_AGENTS => self.agents_digest(),
            None => model_digest(&self.model),
        }
    }

    /// Replays a journal that was written by [`Engine::journal_document`]: checks where it
    /// started ([`Engine::begin_replay`]), then runs its steps as `options.mode` says. By
    /// default only the edits are re-applied.
    pub fn replay_journal(
        &mut self,
        journal: &Journal,
        options: ReplayOptions,
    ) -> Result<Vec<ReplayStep>, ReplayError> {
        self.begin_replay(journal, options.force)?;
        self.replay_with(&journal.entries, options.mode)
    }

    /// Whether the model (the process model, or the system dynamics model with its diagram)
    /// differs from the one last opened or saved.
    pub fn is_dirty(&self) -> bool {
        self.model != self.saved || self.sd.as_ref().is_some_and(|d| d.model != d.saved)
    }

    /// The file the model was opened from or last saved to.
    pub fn path(&self) -> Option<&str> {
        match &self.sd {
            Some(d) => d.file.as_deref(),
            None => self.file.as_ref().and_then(|f| f.path.as_deref()),
        }
    }

    fn edit(&mut self, label: &'static str, op: Op) -> Result<(), CommandError> {
        self.revision += 1;
        let inverse = edit::apply(&mut self.model, op)?;
        self.undo.push(HistoryEntry {
            label,
            op: Undo::Model(inverse),
        });
        self.redo.clear();
        Ok(())
    }

    /// Applies one history entry and gives the entry that takes it back.
    fn apply_history(&mut self, op: Undo) -> Result<Undo, CommandError> {
        match op {
            Undo::Model(op) => edit::apply(&mut self.model, op).map(Undo::Model),
            Undo::Sd(model) => self.swap_sd_model(*model).map(|m| Undo::Sd(Box::new(m))),
        }
    }

    fn undo(&mut self) -> Result<&'static str, CommandError> {
        let entry = self
            .undo
            .pop()
            .ok_or_else(|| CommandError::new(ErrorCode::NothingToUndo, "nothing to undo"))?;
        let label = entry.label;
        self.revision += 1;
        match self.apply_history(entry.op.clone()) {
            Ok(op) => {
                self.redo.push(HistoryEntry { label, op });
                Ok(label)
            }
            Err(e) => {
                self.undo.push(entry);
                Err(e)
            }
        }
    }

    fn redo(&mut self) -> Result<&'static str, CommandError> {
        let entry = self
            .redo
            .pop()
            .ok_or_else(|| CommandError::new(ErrorCode::NothingToRedo, "nothing to redo"))?;
        let label = entry.label;
        self.revision += 1;
        match self.apply_history(entry.op.clone()) {
            Ok(op) => {
                self.undo.push(HistoryEntry { label, op });
                Ok(label)
            }
            Err(e) => {
                self.redo.push(entry);
                Err(e)
            }
        }
    }

    fn history_json(&self) -> Value {
        let labels = |v: &[HistoryEntry]| v.iter().map(|e| e.label).collect::<Vec<_>>();
        json!({
            "undo": labels(&self.undo),
            "redo": labels(&self.redo),
            "dirty": self.is_dirty(),
            "path": self.path(),
        })
    }

    /// Replaces the document: a new model with no history, no file, no background run and no
    /// sweeps in its record, and a new journal that starts from it (`start` says how it began; its
    /// digest is filled in here). The last results are kept, so `results.compare` can still
    /// compare them with the new model's.
    fn new_document(&mut self, model: Model, mut start: JournalStart) {
        start.model_digest = model_digest(&model);
        start
            .document
            .get_or_insert_with(|| DOCUMENT_PROCESS.to_owned());
        self.journal_start = start;
        self.journal.clear();
        self.documents += 1;
        self.saved = model.clone();
        self.revision += 1;
        self.model = model;
        self.undo.clear();
        self.redo.clear();
        self.file = None;
        self.sd = None;
        self.agents = None;
        self.cancel_run();
        self.job = None;
        self.job_kept = false;
        self.sensitivity.clear();
    }

    /// Opens a model file from its bytes (`name` is reported back and used by `file.save`),
    /// replacing the model, clearing the history and starting a new journal. An agent-based
    /// model, which reads files beside its own, is refused: open it with
    /// [`Engine::open_bytes_with`], `file.open` or `agents.open`.
    pub fn open_bytes(
        &mut self,
        name: &str,
        bytes: &[u8],
        scenario: Option<&str>,
    ) -> Result<Value, CommandError> {
        self.open_bytes_with(name, bytes, scenario, &mut |_| {
            Err("an agent-based model given alone as bytes cannot read the files beside it: open it from its file (file.open), or with agents.open and their texts".to_owned())
        })
    }

    /// Replaces the document with a model read from `bytes` (what `open_bytes` does once the
    /// file is read).
    fn open_loaded(
        &mut self,
        name: &str,
        bytes: &[u8],
        loaded: stochcraft_io::Loaded,
        agents: bool,
    ) -> Result<Value, CommandError> {
        let notes = notes_json(&loaded.notes);
        let opened = loaded.opened;
        let result = json!({
            "path": name,
            "format": format_name(opened.format),
            "kind": model_kind(&loaded.model),
            "scenario": opened.scenario,
            "scenarios": opened.scenarios.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
            "notes": notes.clone(),
            "diagnostics": diagnostics(&loaded.model),
        });
        let mut start = JournalStart::new(StartKind::File, Some(name.to_owned()));
        start.file_digest = Some(bytes_digest(bytes));
        start.scenario = opened.scenario.clone();
        if agents {
            start.document = Some(DOCUMENT_AGENTS.to_owned());
        }
        self.new_document(loaded.model, start);
        self.file = Some(FileState {
            path: Some(name.to_owned()),
            opened,
            notes,
        });
        Ok(result)
    }

    /// Whether the opened file uses `id` for something it keeps that the model does not have, so
    /// a new element, connection or pool may not take it.
    fn file_uses_id(&self, id: &str) -> bool {
        self.file.as_ref().is_some_and(|f| f.opened.id_in_use(id))
    }

    /// The bytes `file.save` writes: the opened file with the model's changes written in (byte
    /// for byte when there are none), or with `as_new` (or no opened file) a new BPMN file from
    /// the model. A change the opened file cannot take is [`ErrorCode::Unsupported`], with the
    /// reason.
    pub fn save_bytes(&self, as_new: bool) -> Result<Vec<u8>, CommandError> {
        if let Some(d) = &self.sd {
            // Unchanged since it was read or written: the same bytes, with whatever the reader
            // does not keep.
            if d.model == d.saved
                && let Some(bytes) = &d.bytes
            {
                return Ok(bytes.clone());
            }
            return Ok(stochcraft_sd::write_xmile(&d.model).into_bytes());
        }
        match &self.file {
            Some(f) if !as_new => f
                .opened
                .save(&self.model)
                .map_err(|e| CommandError::new(ErrorCode::Unsupported, e.to_string())),
            _ => Ok(stochcraft_io::write_new(&self.model)),
        }
    }

    /// The format of the file the model was opened from or saved to (`"bpmn"` or `"xpdl"`),
    /// which saving into it keeps; a new file is always BPMN.
    pub fn format(&self) -> Option<&'static str> {
        if self.sd.is_some() {
            return Some("xmile");
        }
        self.file.as_ref().map(|f| format_name(f.opened.format))
    }

    /// What saving a system dynamics model into the file it was opened from would leave out: what
    /// the reader noted it does not keep (further diagrams, styles, display settings, ...). Empty
    /// when nothing would be lost, there is no such file, or the model was written to it since.
    pub fn save_drops(&self) -> &[String] {
        match &self.sd {
            Some(d) if d.drops_on_save && d.file.is_some() && d.model != d.saved => &d.notes,
            _ => &[],
        }
    }

    /// What saving to `path` would leave out of the file there, which `file.save` refuses to do
    /// unless it is given `allow_loss`: what the reader noted it does not keep of the file the
    /// model was opened from, when `path` is that file and the bytes written would not carry it.
    /// Empty when nothing would be lost (an unchanged model is written as its own bytes).
    pub fn save_loss(&self, path: &str, as_new: bool) -> Vec<String> {
        match &self.sd {
            Some(d) => match &d.file {
                Some(file) if d.drops_on_save && d.model != d.saved && same_file(file, path) => {
                    d.notes.clone()
                }
                _ => Vec::new(),
            },
            None => match &self.file {
                Some(f)
                    if as_new
                        && f.path.as_deref().is_some_and(|p| same_file(p, path))
                        && !f.notes.is_empty() =>
                {
                    f.notes
                        .iter()
                        .filter_map(|n| n["message"].as_str().map(str::to_owned))
                        .collect()
                }
                _ => Vec::new(),
            },
        }
    }

    /// Records that the model was saved as `bytes` at `path`, as the app does once its shell has
    /// written the file. Journalled as a `file.save` step, as saving with the command is.
    pub fn saved_to(&mut self, path: &str, bytes: &[u8]) -> Result<Value, CommandError> {
        let result = self.record_saved(path, bytes)?;
        self.journal.push(JournalEntry {
            command: "file.save".to_owned(),
            params: json!({ "path": path }),
        });
        Ok(result)
    }

    /// What [`Engine::saved_to`] does besides journalling, which `file.save` leaves to `execute`.
    fn record_saved(&mut self, path: &str, bytes: &[u8]) -> Result<Value, CommandError> {
        if let Some(d) = self.sd.as_mut() {
            d.saved = d.model.clone();
            d.file = Some(path.to_owned());
            d.path = path.to_owned();
            // Written as it was read: what the reader noted is still in the file. Written from
            // the model: it is not.
            if d.bytes.as_deref() != Some(bytes) {
                d.drops_on_save = false;
                d.bytes = Some(bytes.to_vec());
            }
            return Ok(json!({ "path": path, "format": "xmile", "bytes": bytes.len() as u64 }));
        }
        // Read back what was written, so the next unchanged save gives the same bytes again, in
        // the scenario that was opened (not the file's first one, which later saves would then
        // write into).
        let scenario = self.file.as_ref().and_then(|f| f.opened.scenario.clone());
        let loaded = stochcraft_io::read_scenario(bytes, scenario.as_deref())
            .or_else(|_| stochcraft_io::read(bytes))
            .map_err(|e| CommandError::new(ErrorCode::Internal, format!("{path}: {e}")))?;
        let format = format_name(loaded.opened.format);
        self.file = Some(FileState {
            path: Some(path.to_owned()),
            opened: loaded.opened,
            notes: notes_json(&loaded.notes),
        });
        // Elements that had no shape were given one in the file; they keep it.
        let layout = loaded.model.layout;
        if layout.keys().all(|id| self.model.element(id).is_some()) {
            self.revision += 1;
            self.model.layout = layout;
        }
        self.saved = self.model.clone();
        Ok(json!({ "path": path, "format": format, "bytes": bytes.len() as u64 }))
    }

    /// Replication `replication` (counted from 0) of the model's experiment, set up to run step
    /// by step for watching: the same random stream, so the same events, as that replication of
    /// `run.start` with the experiment's seed.
    pub fn live_run(&self, replication: u64) -> Result<Live<'static>, CommandError> {
        self.approximations(false)?;
        let compiled = compile(&self.model).map_err(|ds| CommandError {
            code: ErrorCode::InvalidModel,
            message: "the model has errors and cannot run".into(),
            details: Some(wire::diagnostics(&self.model, &ds)),
        })?;
        let x = &self.model.experiment;
        let rng = Experiment::new(u64::from(x.replications).max(1), x.seed).stream(replication);
        Ok(compiled.live(rng))
    }

    /// The errors found when the opened file was read that still stand: each is something
    /// StochCraft could not read, so the model it made substitutes for it (a zero time for a
    /// processing-time expression, say) and does not behave as the file describes. An error
    /// that the reader pinned to one field of one element (`at` and `field`, a task's
    /// processing time) stops standing while that field differs from what was read or last
    /// saved: setting it replaces the stand-in, so nothing of the stand-in is recorded for a run
    /// after it. Renaming the element or editing another of its properties does not clear it,
    /// and undoing the edit brings it back. Any other error (no field to pin) stands until the
    /// notes are read again, by opening the file or saving the model into it.
    fn import_errors(&self) -> Vec<Value> {
        let Some(file) = &self.file else {
            return Vec::new();
        };
        let replaced = |n: &Value| match (n["at"].as_str(), n["field"].as_str()) {
            (Some(at), Some(field)) => {
                stand_in_field(&self.model, at, field) != stand_in_field(&self.saved, at, field)
            }
            _ => false,
        };
        file.notes
            .iter()
            .filter(|n| n["severity"] == "error" && !replaced(n))
            .cloned()
            .collect()
    }

    /// Whether a run may go ahead given the errors found reading the file. Returns the
    /// substitutions the run would make (empty when the file read cleanly), and refuses with
    /// them listed unless `allow` says the approximation is deliberate. Every command that runs
    /// the model asks first, so an invalid result never comes from a file read with errors, in
    /// any output format or through any front end.
    pub(crate) fn approximations(&self, allow: bool) -> Result<Vec<Value>, CommandError> {
        let errors = self.import_errors();
        if errors.is_empty() || allow {
            return Ok(errors);
        }
        let first = errors[0]["message"].as_str().unwrap_or("");
        let first = match errors[0]["at"].as_str() {
            Some(at) => format!("at `{at}`: {first}"),
            None => first.to_owned(),
        };
        let count = if errors.len() == 1 {
            "1 error".to_owned()
        } else {
            format!("{} errors", errors.len())
        };
        Err(CommandError {
            code: ErrorCode::InvalidModel,
            message: format!(
                "the file has {count} StochCraft could not read, so the model would not run as the file describes (the first, {first}). Correct the block (or the file), or run with `allow_approximations` (`--allow-approximations` on the command line) to run what StochCraft substituted instead; the run record then lists each substitution"
            ),
            details: Some(Value::Array(errors)),
        })
    }

    /// The plan of a run of the model's experiment, with these overrides. `sequential` overrides
    /// the experiment's sequential replications: `Some(None)` runs a fixed number, `None` keeps
    /// the experiment's. `allow_approximations` accepts what StochCraft substituted for errors
    /// in the file (see [`Engine::approximations`]).
    fn plan(
        &self,
        replications: Option<u32>,
        seed: Option<u64>,
        threads: Option<u32>,
        series: bool,
        sequential: Option<Option<stochcraft_model::Sequential>>,
        allow_approximations: bool,
    ) -> Result<Plan, CommandError> {
        if self.model.is_monte_carlo() {
            return Err(CommandError::new(
                ErrorCode::Unsupported,
                "this is a Monte Carlo model, a calculation with no process: run it with mc.run",
            ));
        }
        if self.sd.is_some() {
            return Err(CommandError::new(
                ErrorCode::Unsupported,
                "this is a system dynamics model: run it with sd.run",
            ));
        }
        if self.model.is_agents() {
            return Err(CommandError::new(
                ErrorCode::Unsupported,
                "this is an agent-based model, populations with no process: run it with agents.run",
            ));
        }
        let approximations = self.approximations(allow_approximations)?;
        let compiled = compile(&self.model).map_err(|ds| CommandError {
            code: ErrorCode::InvalidModel,
            message: "the model has errors and cannot run".into(),
            details: Some(wire::diagnostics(&self.model, &ds)),
        })?;
        let x = &self.model.experiment;
        // Results over time, for the warm-up suggestion and charts over the run.
        let compiled = compiled.record_every(
            series
                .then(|| stochcraft_blocks::record_interval(x))
                .flatten(),
        );
        let mut experiment = Experiment::new(
            u64::from(replications.unwrap_or(x.replications)),
            seed.unwrap_or(x.seed),
        );
        if let Some(t) = threads {
            experiment = experiment.threads(t as usize);
        }
        let mut record = self.record(&experiment, compiled.warnings());
        if !approximations.is_empty() {
            // The run was asked for although the file had errors: what the model has instead
            // of each is in the record, beside the results it explains.
            record["approximations"] = Value::Array(approximations);
        }
        Ok(Plan {
            compiled,
            experiment,
            record,
            sequential: sequential.unwrap_or_else(|| x.sequential.clone()),
        })
    }

    /// The run record that goes with a run's results: enough to reproduce them and to judge
    /// them (NASA-STD-7009B [M&S 24], [M&S 25], [M&S 27], [M&S 34]; docs/research.md).
    /// Results do not depend on the number of threads, so it is not recorded.
    fn record(&self, experiment: &Experiment, warnings: &[stochcraft_model::Diagnostic]) -> Value {
        let x = &self.model.experiment;
        let file = self.file.as_ref();
        json!({
            "stochcraft": env!("CARGO_PKG_VERSION"),
            "model_digest": model_digest(&self.model),
            "file": file.and_then(|f| f.path.as_deref()),
            "format": file.map(|f| format_name(f.opened.format)),
            "scenario": file.and_then(|f| f.opened.scenario.as_deref()),
            "unsaved_changes": file.is_some() && self.is_dirty(),
            "seed": experiment.seed,
            "replications": experiment.replications,
            "time_unit": wire::TimeUnitSpec::from(x.time_unit),
            "length_unit": wire::LengthUnitSpec::from(x.length_unit),
            "warmup": x.warmup,
            "duration": x.duration,
            "random_streams": "one xoshiro256** stream per replication, seeded from the experiment seed and the replication index; each element draws from its own stream",
            "uncertainty": "95% Student-t confidence interval of the mean across independent replications (half_width); std_dev is the sample standard deviation across replications",
            "warnings": wire::diagnostics(&self.model, warnings),
            "file_notes": file.map_or_else(Vec::new, |f| f.notes.clone()),
            "sensitivity": self.sensitivity,
        })
    }

    fn start_run(&mut self, plan: Plan, mode: RunMode) -> Result<Value, CommandError> {
        if self
            .job
            .as_ref()
            .is_some_and(|j| j.status()["state"] == "running")
        {
            return Err(CommandError::new(
                ErrorCode::Conflict,
                "a run is in progress: cancel it or wait for it to finish",
            ));
        }
        let job = Job::start(plan, mode)?;
        let status = job.status();
        self.job = Some(job);
        self.job_kept = false;
        Ok(status)
    }

    /// Progress of the background run. A run in slices first runs until `step` replications
    /// have finished or `events` events have run, whichever comes first (see [`SLICE_EVENTS`]).
    /// A finished run's results are kept for `results.distribution` and `results.compare`,
    /// however it was followed (`run.status`, or a window polling here).
    pub fn poll_run(&mut self, step: u64, events: u64) -> Value {
        let status = self
            .job
            .as_mut()
            .map_or_else(|| json!({ "state": "idle" }), |j| j.poll(step, events));
        self.keep_if_finished(&status);
        status
    }

    /// Keeps the results of a finished background run, once, however the run came to be
    /// collected (polled, or found finished by a cancel that arrived too late).
    fn keep_if_finished(&mut self, status: &Value) {
        if status["state"] == "finished" && !self.job_kept {
            self.job_kept = true;
            self.keep_results(status["result"].clone());
        }
    }

    /// A finished run's results become the last, and the last the previous.
    fn keep_results(&mut self, results: Value) {
        self.previous_results = self.last_results.replace(results);
    }

    /// The background run as its workers get it, for a run started with `"mode": "workers"`
    /// that is still waiting for answers: the text to send each worker, to make a [`Worker`].
    pub fn worker_task(&self) -> Option<&str> {
        self.job.as_ref().and_then(Job::task)
    }

    /// Takes a worker's answer ([`Worker::run`]) for the background run. Once every replication
    /// has answered, `run.status` reports the results.
    pub fn deliver(&mut self, answer: &str) -> Result<(), CommandError> {
        self.job.as_mut().map_or(Ok(()), |j| j.deliver(answer))
    }

    /// Stops the background run. A run that had in fact completed every replication before the
    /// cancel reached it is reported `finished`, not `cancelled`, and keeps its results as a
    /// poll would have kept them.
    fn cancel_run(&mut self) -> Value {
        let status = self
            .job
            .as_mut()
            .map_or_else(|| json!({ "state": "idle" }), Job::cancel);
        self.keep_if_finished(&status);
        status
    }
}

/// Writes `bytes` to `path` through a temporary file beside it that is then renamed over it, so a
/// crash or a full disk leaves the old file whole rather than half written (as `file.save`
/// writes). For the command line's output files.
pub fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    craft_atomic::atomic_write(path, bytes)
}

/// A digest of the model as the engine holds it (its JSON form), so results can be matched to
/// the exact model they came from.
pub fn model_digest(model: &Model) -> String {
    let text = serde_json::to_string(&wire::ModelDoc::from(model)).unwrap_or_default();
    bytes_digest(text.as_bytes())
}

/// Whether two paths name the same file (the same text, or the same file once resolved).
fn same_file(a: &str, b: &str) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

/// The digest of some bytes, written `sha256:` and hex, as [`model_digest`] writes it.
fn bytes_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(bytes);
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

fn notes_json(notes: &[stochcraft_io::Note]) -> Vec<Value> {
    notes
        .iter()
        .map(|n| {
            json!({
                "severity": format!("{:?}", n.severity).to_lowercase(),
                "at": n.at,
                "message": n.message,
                "field": n.field,
            })
        })
        .collect()
}

/// `"monte_carlo"` for a Monte Carlo model, `"agents"` for an agent-based one, else
/// `"process"`.
fn model_kind(model: &Model) -> &'static str {
    if model.is_monte_carlo() {
        "monte_carlo"
    } else if model.is_agents() {
        "agents"
    } else {
        "process"
    }
}

/// The model's diagnostics as JSON: a process model's from its validation, a Monte Carlo
/// model's from `stochcraft-mc` (in the same shape, without `parts`).
pub(crate) fn diagnostics(model: &Model) -> Value {
    match &model.monte_carlo {
        Some(m) if model.is_monte_carlo() => mc::problems(&stochcraft_mc::check(m)),
        _ => wire::diagnostics(model, &model.validate()),
    }
}

fn format_name(f: Format) -> &'static str {
    match f {
        Format::Bpmn => "bpmn",
        Format::Xpdl => "xpdl",
    }
}

/// The value in element `id` that a file error's stand-in went into (`field` as the reader pins
/// it: `processing_time`), or `None` for no such element or field.
fn stand_in_field<'a>(
    model: &'a Model,
    id: &str,
    field: &str,
) -> Option<&'a stochcraft_model::Dist> {
    use stochcraft_model::ElementKind;
    let el = model.element(&stochcraft_model::ElementId::unchecked(id))?;
    match (field, &el.kind) {
        (
            "processing_time",
            ElementKind::Server { service, .. } | ElementKind::SharedTask { service, .. },
        ) => Some(service),
        ("processing_time", ElementKind::Delay { duration, .. }) => Some(duration),
        _ => None,
    }
}
