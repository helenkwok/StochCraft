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
//!   [`Engine::replay`] runs again as a script.
//! - `run` runs an experiment at once; `run.start`, `run.status` and `run.cancel` run one in the
//!   background with progress and cancellation (worker threads natively, slices in the browser).
//! - Files are BPSim on BPMN or XPDL (`stochcraft-io`). `file.save` writes edits into the file
//!   the model was opened from, keeping everything StochCraft does not understand and every line
//!   it did not change; saved unchanged, the file comes back byte for byte. An edit that cannot
//!   be written into that file is refused with the reason, and `as_new` then writes a new BPMN
//!   file from the model alone.

mod commands;
mod edit;
mod run;
pub mod wire;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_blocks::compile;
use stochcraft_experiment::Experiment;
use stochcraft_io::{Format, Opened};
use stochcraft_model::Model;

pub use commands::{COMMANDS, Command, CommandKind};
pub use run::RunMode;

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
            ErrorCode::Internal => "internal",
        }
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

/// A script step that failed during [`Engine::replay`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayError {
    /// The position of the step in the script.
    pub index: usize,
    pub command: String,
    pub error: CommandError,
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "step {} ({}): {}", self.index, self.command, self.error)
    }
}

impl std::error::Error for ReplayError {}

#[derive(Debug, Clone)]
struct HistoryEntry {
    label: &'static str,
    op: Op,
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
    file: Option<FileState>,
    /// The model as last opened or saved, to tell whether there are unsaved changes.
    saved: Model,
    job: Option<Job>,
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
        Self {
            saved: model.clone(),
            model,
            undo: Vec::new(),
            redo: Vec::new(),
            journal: Vec::new(),
            file: None,
            job: None,
        }
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Runs `command` with JSON `params` (`null` counts as `{}`). Every command that is not a
    /// query, and succeeds, is added to the journal.
    pub fn execute(&mut self, command: &str, params: Value) -> Result<Value, CommandError> {
        let params = if params.is_null() { json!({}) } else { params };
        let c = COMMANDS.iter().find(|c| c.name == command).ok_or_else(|| {
            CommandError::new(ErrorCode::UnknownCommand, format!("no command `{command}`"))
        })?;
        let result = (c.run)(self, params.clone())?;
        if c.kind != CommandKind::Query {
            self.journal.push(JournalEntry {
                command: command.to_owned(),
                params,
            });
        }
        Ok(result)
    }

    /// Every command executed so far that was not a query, in order.
    pub fn journal(&self) -> &[JournalEntry] {
        &self.journal
    }

    /// Executes each step of `script` in turn, stopping at the first that fails.
    pub fn replay(&mut self, script: &[JournalEntry]) -> Result<Vec<Value>, ReplayError> {
        script
            .iter()
            .enumerate()
            .map(|(index, step)| {
                self.execute(&step.command, step.params.clone())
                    .map_err(|error| ReplayError {
                        index,
                        command: step.command.clone(),
                        error,
                    })
            })
            .collect()
    }

    /// Whether the model differs from the one last opened or saved.
    pub fn is_dirty(&self) -> bool {
        self.model != self.saved
    }

    /// The file the model was opened from or last saved to.
    pub fn path(&self) -> Option<&str> {
        self.file.as_ref().and_then(|f| f.path.as_deref())
    }

    fn edit(&mut self, label: &'static str, op: Op) -> Result<(), CommandError> {
        let inverse = edit::apply(&mut self.model, op)?;
        self.undo.push(HistoryEntry { label, op: inverse });
        self.redo.clear();
        Ok(())
    }

    fn undo(&mut self) -> Result<&'static str, CommandError> {
        let entry = self
            .undo
            .pop()
            .ok_or_else(|| CommandError::new(ErrorCode::NothingToUndo, "nothing to undo"))?;
        let label = entry.label;
        match edit::apply(&mut self.model, entry.op.clone()) {
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
        match edit::apply(&mut self.model, entry.op.clone()) {
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

    /// Replaces the document: a new model with no history and no file.
    fn new_document(&mut self, model: Model) {
        self.saved = model.clone();
        self.model = model;
        self.undo.clear();
        self.redo.clear();
        self.file = None;
    }

    /// Opens a model file from its bytes (`name` is reported back and used by `file.save`),
    /// replacing the model and clearing the history.
    pub fn open_bytes(
        &mut self,
        name: &str,
        bytes: &[u8],
        scenario: Option<&str>,
    ) -> Result<Value, CommandError> {
        let loaded = stochcraft_io::read_scenario(bytes, scenario)
            .map_err(|e| CommandError::new(ErrorCode::BadFile, format!("{name}: {e}")))?;
        let notes = notes_json(&loaded.notes);
        let opened = loaded.opened;
        let result = json!({
            "path": name,
            "format": format_name(opened.format),
            "scenario": opened.scenario,
            "scenarios": opened.scenarios.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
            "notes": notes.clone(),
            "diagnostics": wire::diagnostics(&loaded.model.validate()),
        });
        self.new_document(loaded.model);
        self.file = Some(FileState {
            path: Some(name.to_owned()),
            opened,
            notes,
        });
        Ok(result)
    }

    /// The bytes `file.save` writes: the opened file with the model's changes written in (byte
    /// for byte when there are none), or with `as_new` (or no opened file) a new BPMN file from
    /// the model.
    pub fn save_bytes(&self, as_new: bool) -> Result<Vec<u8>, CommandError> {
        match &self.file {
            Some(f) if !as_new => f.opened.save(&self.model).map_err(|e| {
                CommandError::new(
                    ErrorCode::Unsupported,
                    format!("{e}; `\"as_new\": true` writes the model as a new BPMN file instead, without the content StochCraft does not understand"),
                )
            }),
            _ => Ok(stochcraft_io::write_new(&self.model)),
        }
    }

    /// Records that the model was saved as `bytes` at `path`.
    pub fn saved_to(&mut self, path: &str, bytes: &[u8]) -> Result<Value, CommandError> {
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
        self.saved = self.model.clone();
        Ok(json!({ "path": path, "format": format, "bytes": bytes.len() as u64 }))
    }

    fn plan(
        &self,
        replications: Option<u32>,
        seed: Option<u64>,
        threads: Option<u32>,
    ) -> Result<Plan, CommandError> {
        let compiled = compile(&self.model).map_err(|ds| CommandError {
            code: ErrorCode::InvalidModel,
            message: "the model has errors and cannot run".into(),
            details: Some(wire::diagnostics(&ds)),
        })?;
        let x = &self.model.experiment;
        let mut experiment = Experiment::new(
            u64::from(replications.unwrap_or(x.replications)),
            seed.unwrap_or(x.seed),
        );
        if let Some(t) = threads {
            experiment = experiment.threads(t as usize);
        }
        let record = self.record(&experiment, compiled.warnings());
        Ok(Plan {
            compiled,
            experiment,
            record,
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
            "warmup": x.warmup,
            "duration": x.duration,
            "random_streams": "one xoshiro256** stream per replication, seeded from the experiment seed and the replication index; each element draws from its own stream",
            "uncertainty": "95% Student-t confidence interval of the mean across independent replications (half_width); std_dev is the sample standard deviation across replications",
            "warnings": wire::diagnostics(warnings),
            "file_notes": file.map_or_else(Vec::new, |f| f.notes.clone()),
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
        Ok(status)
    }

    /// Progress of the background run (advancing a run in slices by up to `step` replications).
    pub fn poll_run(&mut self, step: u64) -> Value {
        self.job
            .as_mut()
            .map_or_else(|| json!({ "state": "idle" }), |j| j.poll(step))
    }

    fn cancel_run(&mut self) -> Value {
        self.job
            .as_mut()
            .map_or_else(|| json!({ "state": "idle" }), Job::cancel)
    }
}

/// A digest of the model as the engine holds it (its JSON form), so results can be matched to
/// the exact model they came from.
pub fn model_digest(model: &Model) -> String {
    use sha2::{Digest, Sha256};
    let text = serde_json::to_string(&wire::ModelDoc::from(model)).unwrap_or_default();
    let hash = Sha256::digest(text.as_bytes());
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
            })
        })
        .collect()
}

fn format_name(f: Format) -> &'static str {
    match f {
        Format::Bpmn => "bpmn",
        Format::Xpdl => "xpdl",
    }
}
