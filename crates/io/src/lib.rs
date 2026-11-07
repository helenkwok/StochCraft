//! StochCraft's model files: BPSim 2.0 (and 1.0) on BPMN 2.0 or XPDL 2.2 (ROADMAP.md, "Model
//! files: no new format").
//!
//! - [`read()`] / [`read_scenario`] open a file: the model, notes on anything StochCraft ignored or
//!   could not use, and an [`Opened`] document that remembers the original bytes.
//! - [`write_new`] writes a model StochCraft created as BPMN 2.0 with BPSim 2.0, in the order the
//!   BPSim XML Schema requires, with a diagram other BPMN editors can display.
//! - [`Opened::save`] writes an opened file back: unchanged models byte for byte, edited ones with
//!   only their changes written in and everything else kept as it was, checked by reading the
//!   result back.
//!
//! Reading is lenient and writing strict: element order, BPSim 1.0 namespaces and other time units
//! are accepted on reading; everything written follows the schema.

pub mod bpsim;
pub mod duration;
pub mod read;
mod save;
pub mod selection;
pub mod write;

pub use write::write_new;
pub mod xml;

/// How serious a note is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// For information: something kept in the file but not used.
    Info,
    /// Something StochCraft ignored or interpreted; the model may differ from the author's intent.
    Warning,
    /// Something StochCraft could not read; the model will not run as the file describes.
    Error,
}

/// Something noticed while reading a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub severity: Severity,
    /// The id of the element it concerns, if any.
    pub at: Option<String>,
    pub message: String,
}

impl Note {
    fn new(severity: Severity, at: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            severity,
            at: at.map(str::to_owned),
            message: message.into(),
        }
    }

    pub fn info(at: Option<&str>, message: impl Into<String>) -> Self {
        Self::new(Severity::Info, at, message)
    }

    pub fn warning(at: Option<&str>, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, at, message)
    }

    pub fn error(at: Option<&str>, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, at, message)
    }
}

impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self.severity {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        match &self.at {
            Some(at) => write!(f, "{level} at `{at}`: {}", self.message),
            None => write!(f, "{level}: {}", self.message),
        }
    }
}

/// The process format a model file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// BPMN 2.0 with BPSim in a `relationship`'s extension elements.
    Bpmn,
    /// XPDL 2.2 with BPSim inside the package.
    Xpdl,
}

/// Why a file could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// Not well-formed XML.
    Xml(String),
    /// Well-formed, but not a BPMN or XPDL model, or the requested scenario does not exist.
    NotAModel(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Xml(e) | ReadError::NotAModel(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for ReadError {}

/// A model file as it was opened.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
    pub format: Format,
    /// The scenario the model was read from, if the file has BPSim data.
    pub scenario: Option<String>,
    /// Every input scenario in the file: (id, name).
    pub scenarios: Vec<(String, Option<String>)>,
    bytes: Vec<u8>,
    model_at_load: stochcraft_model::Model,
    /// The file's ids that are not the model's: elements StochCraft keeps but does not model,
    /// lanes, resources of single tasks, diagram shapes, scenarios and so on.
    other_ids: std::collections::BTreeSet<String>,
}

/// What [`read()`] returns.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub model: stochcraft_model::Model,
    /// Things StochCraft ignored, interpreted or could not use, errors first.
    pub notes: Vec<Note>,
    pub opened: Opened,
}

/// Reads a model file with its first input scenario.
pub fn read(bytes: &[u8]) -> Result<Loaded, ReadError> {
    read_scenario(bytes, None)
}

/// Reads a model file with scenario `id` (or the first input scenario).
pub fn read_scenario(bytes: &[u8], id: Option<&str>) -> Result<Loaded, ReadError> {
    let doc = xml::parse(bytes).map_err(|e| ReadError::Xml(e.to_string()))?;
    let mut notes = Vec::new();
    let (format, scenario, model) =
        read::read_model(&doc.root, id, &mut notes).map_err(ReadError::NotAModel)?;
    let scenarios = bpsim::find_data(&doc.root)
        .map(bpsim::scenario_list)
        .unwrap_or_default();
    // Most severe first; a note raised once per user of a shared element is shown once.
    let mut seen = std::collections::BTreeSet::new();
    notes.retain(|n| seen.insert((n.severity, n.at.clone(), n.message.clone())));
    notes.sort_by_key(|n| std::cmp::Reverse(n.severity));
    let other_ids = save::other_ids(&doc.root, &model);
    let opened = Opened {
        format,
        scenario: scenario.map(|s| s.settings.id),
        scenarios,
        bytes: bytes.to_vec(),
        model_at_load: model.clone(),
        other_ids,
    };
    Ok(Loaded {
        model,
        notes,
        opened,
    })
}

/// Why an edited model could not be saved into the file it was opened from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The change cannot be written into this file (yet), with the reason. [`write_new`] can
    /// still save the model as a new file.
    Unsupported(String),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Unsupported(reason) => {
                write!(f, "cannot save the changes into the opened file: {reason}")
            }
        }
    }
}

impl std::error::Error for SaveError {}

impl Opened {
    /// Whether the file uses `id` for something other than the model's elements, connections and
    /// pools (which saving keeps), so a new element, connection or pool may not take it.
    pub fn id_in_use(&self, id: &str) -> bool {
        self.other_ids.contains(id)
    }

    /// The file's bytes for `model`: exactly the original bytes when the model is unchanged;
    /// otherwise the original file with the changes written in, and everything StochCraft does
    /// not understand kept as it was. The result is checked by reading it back.
    pub fn save(&self, model: &stochcraft_model::Model) -> Result<Vec<u8>, SaveError> {
        if *model == self.model_at_load {
            Ok(self.bytes.clone())
        } else {
            save::save_edited(
                &self.bytes,
                self.format,
                self.scenario.as_deref(),
                &self.model_at_load,
                model,
            )
        }
    }
}
