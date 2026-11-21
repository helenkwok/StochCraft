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
//!   (see [`sd`]); `run` refuses one. It cannot be edited or saved yet.
//! - Files are BPSim on BPMN or XPDL (`stochcraft-io`). `file.save` writes edits into the file
//!   the model was opened from, keeping everything StochCraft does not understand and every line
//!   it did not change; saved unchanged, the file comes back byte for byte. An edit that cannot
//!   be written into that file is refused with the reason, and `as_new` then writes a new BPMN
//!   file from the model alone.

mod commands;
mod compare;
mod decide;
mod design;
mod distribution;
mod edit;
mod fit;
pub mod mc;
pub mod precision;
mod run;
pub mod sd;
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
}

/// Which steps of a journal a replay carries out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayMode {
    /// Re-applies the edits (and undo, redo and starting a document). Saving a file, runs and
    /// experiments are skipped: they write files or take time, and change nothing in the model.
    #[default]
    EditsOnly,
    /// Carries out every step, as [`Engine::replay`] does.
    All,
}

/// The commands [`ReplayMode::EditsOnly`] skips.
const SKIPPED_BY_DEFAULT: [&str; 11] = [
    "file.save",
    "run",
    "mc.run",
    "mc.sobol",
    "sd.run",
    "run.start",
    "run.cancel",
    "experiment.sweep",
    "experiment.sensitivity",
    "experiment.design",
    "experiment.size",
];

impl ReplayMode {
    /// Whether a replay in this mode carries out `command`.
    pub fn applies(self, command: &str) -> bool {
        self == ReplayMode::All || !SKIPPED_BY_DEFAULT.contains(&command)
    }
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
    /// What the journal started from.
    journal_start: JournalStart,
    /// Counts the documents started (a new or opened model), so `execute` can tell that a
    /// command started one and left the journal to its start record.
    documents: u64,
    file: Option<FileState>,
    /// The system dynamics model, if the document is one (see [`sd`]).
    sd: Option<sd::SdDoc>,
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
            self.journal.push(JournalEntry {
                command: command.to_owned(),
                params,
            });
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
                    .map_err(|error| ReplayError {
                        index,
                        command: step.command.clone(),
                        error,
                    })
            })
            .collect()
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
                self.execute(&step.command, step.params.clone())
                    .map(ReplayStep::Applied)
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
        let found = model_digest(&self.model);
        if found != start.model_digest && !force {
            let from = match (start.kind, &start.name) {
                (StartKind::File, Some(name)) => format!("the file `{name}`"),
                (StartKind::New, _) => "a new model".to_owned(),
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
        Ok(())
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

    /// Whether the model differs from the one last opened or saved.
    pub fn is_dirty(&self) -> bool {
        self.model != self.saved
    }

    /// The file the model was opened from or last saved to.
    pub fn path(&self) -> Option<&str> {
        self.file.as_ref().and_then(|f| f.path.as_deref())
    }

    fn edit(&mut self, label: &'static str, op: Op) -> Result<(), CommandError> {
        self.revision += 1;
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
        self.revision += 1;
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
        self.revision += 1;
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

    /// Replaces the document: a new model with no history, no file, no background run and no
    /// sweeps in its record, and a new journal that starts from it (`start` says how it began; its
    /// digest is filled in here). The last results are kept, so `results.compare` can still
    /// compare them with the new model's.
    fn new_document(&mut self, model: Model, mut start: JournalStart) {
        start.model_digest = model_digest(&model);
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
        self.cancel_run();
        self.job = None;
        self.job_kept = false;
        self.sensitivity.clear();
    }

    /// Opens a model file from its bytes (`name` is reported back and used by `file.save`),
    /// replacing the model, clearing the history and starting a new journal.
    pub fn open_bytes(
        &mut self,
        name: &str,
        bytes: &[u8],
        scenario: Option<&str>,
    ) -> Result<Value, CommandError> {
        if stochcraft_sd::looks_like_xmile(bytes) {
            return self.open_sd(name, bytes);
        }
        let loaded = stochcraft_io::read_scenario(bytes, scenario)
            .map_err(|e| CommandError::new(ErrorCode::BadFile, format!("{name}: {e}")))?;
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
        self.file.as_ref().map(|f| format_name(f.opened.format))
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
        let compiled = compile(&self.model).map_err(|ds| CommandError {
            code: ErrorCode::InvalidModel,
            message: "the model has errors and cannot run".into(),
            details: Some(wire::diagnostics(&self.model, &ds)),
        })?;
        let x = &self.model.experiment;
        let rng = Experiment::new(u64::from(x.replications).max(1), x.seed).stream(replication);
        Ok(compiled.live(rng))
    }

    /// The plan of a run of the model's experiment, with these overrides. `sequential` overrides
    /// the experiment's sequential replications: `Some(None)` runs a fixed number, `None` keeps
    /// the experiment's.
    fn plan(
        &self,
        replications: Option<u32>,
        seed: Option<u64>,
        threads: Option<u32>,
        series: bool,
        sequential: Option<Option<stochcraft_model::Sequential>>,
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
        let record = self.record(&experiment, compiled.warnings());
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
        if status["state"] == "finished" && !self.job_kept {
            self.job_kept = true;
            self.keep_results(status["result"].clone());
        }
        status
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

    fn cancel_run(&mut self) -> Value {
        self.job
            .as_mut()
            .map_or_else(|| json!({ "state": "idle" }), Job::cancel)
    }
}

/// A digest of the model as the engine holds it (its JSON form), so results can be matched to
/// the exact model they came from.
pub fn model_digest(model: &Model) -> String {
    let text = serde_json::to_string(&wire::ModelDoc::from(model)).unwrap_or_default();
    bytes_digest(text.as_bytes())
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
            })
        })
        .collect()
}

/// `"monte_carlo"` for a Monte Carlo model, else `"process"`.
fn model_kind(model: &Model) -> &'static str {
    if model.is_monte_carlo() {
        "monte_carlo"
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
