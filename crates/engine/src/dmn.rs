//! Decision tables through the engine (ROADMAP M5.5, decision D30): the commands `dmn.get`,
//! `dmn.set` and `dmn.use`; the tables in the model's JSON; and the `.dmn` files read beside a
//! model by `file.open` and written beside it by `file.save`.
//!
//! A model names a decision of a `.dmn` file for a block ([`DecisionUse`], the
//! `stochcraft:decision` vendor extension); the file is a reference relative to the model's own.
//! Opening a model reads each file it names (with [`stochcraft_io::dmn::read_dmn`]) into
//! [`Model::dmn`]; a file that cannot be read is a note, and the blocks that use it a diagnostic,
//! so the model opens and can be mended. The engine keeps each file's bytes as read, so saving
//! writes an unchanged file back byte for byte and an edited one with only its edited tables
//! written in ([`stochcraft_io::dmn::save_dmn`]).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_model::dmn::{
    Aggregation, Decision, DecisionUse, Definitions, HitPolicy, Input, Output, Rule, Table,
};
use stochcraft_model::{ElementId, Model};

use crate::commands::parse;
use crate::edit::Op;
use crate::{CommandError, Engine, ErrorCode};

/// A file written beside the model: its reference, its bytes, and the bytes a save may replace
/// (a `.dmn` file as it was opened or last saved; `None` for one that is new).
pub type Companion = (String, Vec<u8>, Option<Vec<u8>>);

/// A `.dmn` file as it was read (or last saved): its bytes, and what they read as.
#[derive(Debug, Clone)]
pub(crate) struct DmnFile {
    pub bytes: Vec<u8>,
    pub read: Definitions,
}

/// A decision table's hit policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HitPolicyDoc {
    /// At most one rule matches any input (DMN's default).
    #[default]
    Unique,
    /// The first matching rule, in rule order.
    First,
    /// The matching rule whose outputs come first in the outputs' lists of values.
    Priority,
    /// Every matching rule, or their aggregation.
    Collect,
}

impl From<HitPolicy> for HitPolicyDoc {
    fn from(h: HitPolicy) -> Self {
        match h {
            HitPolicy::Unique => HitPolicyDoc::Unique,
            HitPolicy::First => HitPolicyDoc::First,
            HitPolicy::Priority => HitPolicyDoc::Priority,
            HitPolicy::Collect => HitPolicyDoc::Collect,
        }
    }
}

impl From<HitPolicyDoc> for HitPolicy {
    fn from(h: HitPolicyDoc) -> Self {
        match h {
            HitPolicyDoc::Unique => HitPolicy::Unique,
            HitPolicyDoc::First => HitPolicy::First,
            HitPolicyDoc::Priority => HitPolicy::Priority,
            HitPolicyDoc::Collect => HitPolicy::Collect,
        }
    }
}

/// COLLECT's aggregation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AggregationDoc {
    Sum,
    Count,
    Min,
    Max,
}

impl From<Aggregation> for AggregationDoc {
    fn from(a: Aggregation) -> Self {
        match a {
            Aggregation::Sum => AggregationDoc::Sum,
            Aggregation::Count => AggregationDoc::Count,
            Aggregation::Min => AggregationDoc::Min,
            Aggregation::Max => AggregationDoc::Max,
        }
    }
}

impl From<AggregationDoc> for Aggregation {
    fn from(a: AggregationDoc) -> Self {
        match a {
            AggregationDoc::Sum => Aggregation::Sum,
            AggregationDoc::Count => Aggregation::Count,
            AggregationDoc::Min => Aggregation::Min,
            AggregationDoc::Max => Aggregation::Max,
        }
    }
}

/// An input column: the entity property it reads, and the values it may take.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputDoc {
    /// The name of the entity property the column reads.
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The column's heading, if not the property's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `number`, `string` or `boolean`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_ref: Option<String>,
    /// The values it may take, as unary tests (`"A", "B"`, `[0..100]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<String>,
}

/// An output column: the entity property it writes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputDoc {
    /// The property it writes; a table with one output may leave it out, which then writes the
    /// decision's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_ref: Option<String>,
    /// The values it may give, highest priority first (what PRIORITY reads).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<String>,
    /// What it gives when no rule matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// A rule: a cell per input (unary tests, `-` for any value), a value per output, a text per
/// annotation column.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuleDoc {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A decision table, every cell as its FEEL text.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TableDoc {
    #[serde(default)]
    pub hit_policy: HitPolicyDoc,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregation: Option<AggregationDoc>,
    #[serde(default)]
    pub inputs: Vec<InputDoc>,
    pub outputs: Vec<OutputDoc>,
    /// The names of the rule annotation columns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<String>,
    #[serde(default)]
    pub rules: Vec<RuleDoc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl From<&Table> for TableDoc {
    fn from(t: &Table) -> Self {
        Self {
            hit_policy: t.hit_policy.into(),
            aggregation: t.aggregation.map(Into::into),
            inputs: t
                .inputs
                .iter()
                .map(|i| InputDoc {
                    expression: i.expression.clone(),
                    id: i.id.clone(),
                    label: i.label.clone(),
                    type_ref: i.type_ref.clone(),
                    values: i.values.clone(),
                })
                .collect(),
            outputs: t
                .outputs
                .iter()
                .map(|o| OutputDoc {
                    name: o.name.clone(),
                    id: o.id.clone(),
                    label: o.label.clone(),
                    type_ref: o.type_ref.clone(),
                    values: o.values.clone(),
                    default: o.default.clone(),
                })
                .collect(),
            annotations: t.annotations.clone(),
            rules: t
                .rules
                .iter()
                .map(|r| RuleDoc {
                    inputs: r.inputs.clone(),
                    outputs: r.outputs.clone(),
                    annotations: r.annotations.clone(),
                    id: r.id.clone(),
                    description: r.description.clone(),
                })
                .collect(),
            id: t.id.clone(),
        }
    }
}

/// A cell's text as a `.dmn` file gives it back: without the whitespace around it (spaces
/// inside a quoted string stay).
fn cell(s: String) -> String {
    s.trim().to_owned()
}

/// An optional text (an input's or output's values, a default) as a file gives it back.
fn text(s: Option<String>) -> Option<String> {
    s.map(cell)
}

/// An optional attribute (an id, a label, a type) as a file gives it back: none when empty.
fn attr(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

/// A table as given to `dmn.set` (or in a model's JSON), as it reads back from the file it is
/// saved into: what the file does not keep (spaces around a cell, an empty id) is not kept here
/// either, so saving can check that the file reads back as the model.
impl From<TableDoc> for Table {
    fn from(t: TableDoc) -> Self {
        Self {
            id: attr(t.id),
            hit_policy: t.hit_policy.into(),
            aggregation: t.aggregation.map(Into::into),
            inputs: t
                .inputs
                .into_iter()
                .map(|i| Input {
                    id: attr(i.id),
                    label: attr(i.label),
                    expression: cell(i.expression),
                    type_ref: attr(i.type_ref),
                    values: text(i.values),
                })
                .collect(),
            outputs: t
                .outputs
                .into_iter()
                .map(|o| Output {
                    id: attr(o.id),
                    label: attr(o.label),
                    name: attr(o.name),
                    type_ref: attr(o.type_ref),
                    values: text(o.values),
                    default: text(o.default),
                })
                .collect(),
            annotations: t.annotations,
            rules: t
                .rules
                .into_iter()
                .map(|r| Rule {
                    id: attr(r.id),
                    description: text(r.description).filter(|d| !d.is_empty()),
                    inputs: r.inputs.into_iter().map(cell).collect(),
                    outputs: r.outputs.into_iter().map(cell).collect(),
                    annotations: r.annotations.into_iter().map(cell).collect(),
                })
                .collect(),
        }
    }
}

/// A decision: its id (what a block names), its name, and its table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionDoc {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variable: Option<String>,
    pub table: TableDoc,
}

/// A `.dmn` file the model reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DmnFileDoc {
    /// The file, relative to the model's file.
    pub file: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    pub decisions: Vec<DecisionDoc>,
}

/// The decision table a block uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionUseDoc {
    /// The block's id.
    pub element: String,
    /// The `.dmn` file, relative to the model's file.
    pub file: String,
    /// The decision's id in it.
    pub decision: String,
    /// On a gateway whose table has several outputs: the output to route by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
}

/// The decision tables a model uses, and the files they are in.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecisionsDoc {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: Vec<DecisionUseDoc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<DmnFileDoc>,
}

impl DecisionsDoc {
    /// Whether the model uses no decision table (it then leaves `decisions` out).
    pub fn is_empty(&self) -> bool {
        self.uses.is_empty() && self.files.is_empty()
    }

    pub fn of(m: &Model) -> Self {
        Self {
            uses: m
                .decisions
                .iter()
                .map(|(id, u)| DecisionUseDoc {
                    element: id.to_string(),
                    file: u.file.clone(),
                    decision: u.decision.clone(),
                    route: u.route.clone(),
                })
                .collect(),
            files: m
                .dmn
                .iter()
                .map(|(file, d)| DmnFileDoc {
                    file: file.clone(),
                    id: d.id.clone(),
                    name: d.name.clone(),
                    namespace: d.namespace.clone(),
                    decisions: d
                        .decisions
                        .iter()
                        .map(|x| DecisionDoc {
                            id: x.id.clone(),
                            name: x.name.clone(),
                            variable: x.variable.clone(),
                            table: (&x.table).into(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Sets `m`'s decision uses and files to these.
    pub fn apply(self, m: &mut Model) {
        m.decisions = self
            .uses
            .into_iter()
            .map(|u| {
                (
                    ElementId::unchecked(&u.element),
                    DecisionUse {
                        file: u.file,
                        decision: u.decision,
                        route: u.route,
                    },
                )
            })
            .collect();
        m.dmn = self
            .files
            .into_iter()
            .map(|f| {
                (
                    f.file,
                    Definitions {
                        id: f.id,
                        name: f.name,
                        namespace: f.namespace,
                        decisions: f
                            .decisions
                            .into_iter()
                            .map(|d| Decision {
                                id: d.id,
                                name: d.name,
                                variable: d.variable,
                                table: d.table.into(),
                            })
                            .collect(),
                    },
                )
            })
            .collect();
    }
}

/// Whether `href` is a reference to a `.dmn` file beside the model (relative, in its folder or
/// below), as [`crate::agents::is_safe_reference`] decides.
pub fn is_dmn_reference(href: &str) -> bool {
    href.to_ascii_lowercase().ends_with(".dmn") && crate::agents::is_safe_reference(href)
}

/// Reads the `.dmn` files `loaded`'s model names, with `resolve` (by their references), into
/// the model; each that cannot be read is an error note (and the blocks using it diagnostics).
/// Returns each file read, with its bytes.
pub(crate) fn load(
    loaded: &mut stochcraft_io::Loaded,
    resolve: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
) -> BTreeMap<String, DmnFile> {
    let mut files = BTreeMap::new();
    let hrefs: std::collections::BTreeSet<String> = loaded
        .model
        .decisions
        .values()
        .map(|u| u.file.clone())
        .collect();
    for href in hrefs {
        // A warning, not an error: the model has no stand-in for the table, and each block
        // still using it is an error of the model itself (`decision_unusable`), which stops a
        // run while the block uses it and no longer once it does not.
        let note = |message: String| stochcraft_io::Note::warning(None, message);
        if !is_dmn_reference(&href) {
            loaded.notes.push(note(format!(
                "`{href}` is not a relative reference to a .dmn file beside the model (no `..`, root, drive, scheme or device name); the blocks that use it cannot run"
            )));
            continue;
        }
        let bytes = match resolve(&href) {
            Ok(b) => b,
            Err(e) => {
                loaded.notes.push(note(format!(
                    "{href}: {e}; the blocks that use it cannot run"
                )));
                continue;
            }
        };
        match stochcraft_io::dmn::read_dmn(&bytes) {
            Ok(read) => {
                loaded.model.dmn.insert(href.clone(), read.clone());
                files.insert(href, DmnFile { bytes, read });
            }
            Err(e) => loaded.notes.push(note(format!(
                "{href}: {e}; the blocks that use it cannot run"
            ))),
        }
    }
    loaded.notes.sort_by_key(|n| std::cmp::Reverse(n.severity));
    files
}

impl Engine {
    /// The `.dmn` files the model reads, by their references, each as `file.save` writes it,
    /// with the bytes it had when opened or last saved (`None` for a file new since), which a
    /// save may replace.
    pub fn dmn_files(&self) -> Result<Vec<Companion>, CommandError> {
        let mut out = Vec::new();
        for (href, defs) in &self.model.dmn {
            let (bytes, was) = match self.dmn.get(href) {
                Some(f) => (
                    stochcraft_io::dmn::save_dmn(&f.bytes, &f.read, defs).map_err(|e| {
                        CommandError::new(ErrorCode::Unsupported, format!("{href}: {e}"))
                    })?,
                    Some(f.bytes.clone()),
                ),
                None => (stochcraft_io::dmn::write_dmn(defs).into_bytes(), None),
            };
            out.push((href.clone(), bytes, was));
        }
        Ok(out)
    }

    /// Whether a table file the model reads holds other tables than the model: as last read or
    /// written (a file the model no longer reads, or has never been read or written, is not
    /// counted).
    pub(crate) fn dmn_unsaved(&self) -> bool {
        // Saved without its tables into another folder, a model with tables does not have them
        // beside it: the records of the folder they are in do not count there.
        (self.dmn_away && !self.model.dmn.is_empty())
            || self
                .dmn
                .iter()
                .any(|(href, f)| self.model.dmn.get(href).is_some_and(|d| *d != f.read))
    }

    /// Whether a decision table the model holds is not on disk as it is: edited or made since its
    /// `.dmn` file was read or written, or not beside the model's file where that was last saved.
    /// Unlike [`Engine::is_dirty`], it leaves out the model's own file, which a browser's
    /// download writes without the model being marked saved.
    pub fn tables_unsaved(&self) -> bool {
        self.model.dmn != self.saved.dmn || self.dmn_unsaved()
    }

    /// Gives each table file the model holds that is not read from disk yet, and has no id or
    /// namespace of its own, the ones [`stochcraft_io::dmn::write_dmn`] writes for it, in the model
    /// and in what it was last saved as: so the tables recorded as written are those the file
    /// reads back as, and a later save into the file finds it as recorded.
    pub(crate) fn dmn_as_written(&mut self) {
        let mut changed = false;
        for (href, d) in &mut self.model.dmn {
            if !self.dmn.contains_key(href) {
                let w = stochcraft_io::dmn::as_written(d);
                changed |= w != *d;
                *d = w;
            }
        }
        for (href, d) in &mut self.saved.dmn {
            if !self.dmn.contains_key(href) {
                *d = stochcraft_io::dmn::as_written(d);
            }
        }
        if changed {
            self.revision += 1;
        }
    }

    /// Records that the model's `.dmn` files were written as `written` (by reference), so the
    /// next save starts from them.
    pub(crate) fn dmn_saved(&mut self, written: Vec<(String, Vec<u8>)>) {
        for (href, bytes) in written {
            if let Some(read) = self.model.dmn.get(&href).cloned() {
                self.dmn.insert(href, DmnFile { bytes, read });
            }
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DmnGet {
    /// Only this file's tables (by its reference); all when left out.
    #[serde(default)]
    pub file: Option<String>,
}

pub(crate) fn get(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: DmnGet = parse(p)?;
    let mut doc = DecisionsDoc::of(&e.model);
    if let Some(f) = &p.file {
        doc.files.retain(|x| &x.file == f);
        doc.uses.retain(|x| &x.file == f);
    }
    Ok(serde_json::to_value(doc).unwrap_or(Value::Null))
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DmnSet {
    /// The `.dmn` file, relative to the model's file: one the model reads, or a new one, which
    /// `file.save` writes beside the model.
    pub file: String,
    /// The decision's id: one in the file, or a new one (not in a file read from disk, which
    /// cannot take a decision yet).
    pub decision: String,
    /// The decision's name, if it changes (a new decision takes its id when left out).
    #[serde(default)]
    pub name: Option<String>,
    pub table: TableDoc,
}

pub(crate) fn set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: DmnSet = parse(p)?;
    if !is_dmn_reference(&p.file) {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!(
                "`{}` is not a relative reference to a .dmn file beside the model",
                p.file
            ),
        ));
    }
    if !stochcraft_model::ElementId::is_valid(&p.decision)
        || !stochcraft_model::dmn::is_xml_id(&p.decision)
    {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!("`{}` is not a valid decision id", p.decision),
        ));
    }
    let old = e
        .model
        .dmn
        .get(&p.file)
        .and_then(|f| f.decision(&p.decision));
    let decision = Decision {
        id: p.decision.clone(),
        name: p
            .name
            .or_else(|| old.map(|d| d.name.clone()))
            .unwrap_or_else(|| p.decision.clone()),
        variable: old.and_then(|d| d.variable.clone()),
        table: p.table.into(),
    };
    // Every id the table gives is an XML id, as DMN requires of its `id` attributes.
    let t = &decision.table;
    let named = std::iter::once(("the table".to_owned(), t.id.as_deref()))
        .chain(
            t.inputs
                .iter()
                .enumerate()
                .map(|(i, c)| (format!("input {}", i + 1), c.id.as_deref())),
        )
        .chain(
            t.outputs
                .iter()
                .enumerate()
                .map(|(i, c)| (format!("output {}", i + 1), c.id.as_deref())),
        )
        .chain(
            t.rules
                .iter()
                .enumerate()
                .map(|(i, r)| (format!("rule {}", i + 1), r.id.as_deref())),
        );
    for (what, id) in named {
        if let Some(id) = id.filter(|i| !stochcraft_model::dmn::is_xml_id(i)) {
            return Err(CommandError::new(
                ErrorCode::InvalidParams,
                format!(
                    "{}: the id `{id}` of {what} is not an XML id: start it with a letter or `_`, and use letters, digits, `-`, `.` and `_`",
                    p.decision
                ),
            ));
        }
    }
    // A table is set only if it can run.
    decision.compile().map_err(|err| {
        CommandError::new(ErrorCode::InvalidParams, format!("{}: {err}", p.decision))
    })?;
    // A file read from disk keeps its decisions: saving writes edited tables into it, but
    // cannot add a decision to it yet. Refused here, before the model changes, rather than when
    // saving.
    if let Some(f) = e.dmn.get(&p.file)
        && f.read.decision(&p.decision).is_none()
    {
        return Err(CommandError::new(
            ErrorCode::Unsupported,
            format!(
                "decisions cannot be added to {}, a .dmn file the model has read, yet: put the decision `{}` in a new .dmn file",
                p.file, p.decision
            ),
        ));
    }
    if let Some(id) = clashing_id(e, &p.file, &decision) {
        return Err(CommandError::new(
            ErrorCode::InvalidParams,
            format!(
                "the id `{id}` would be used twice in {}: each id names one element of a DMN file; give it another",
                p.file
            ),
        ));
    }
    let findings = decision.compile().map(|c| c.check(&[])).unwrap_or_default();
    e.edit(
        "dmn.set",
        Op::SetDecision {
            file: p.file,
            decision: Box::new(decision),
        },
    )?;
    Ok(json!({ "findings": findings.iter().map(finding_json).collect::<Vec<_>>() }))
}

/// An id that `decision`, set in the file `file`, would give twice in that file: one of its own
/// ids that it gives twice itself, or that another element of the file has (the file itself,
/// another decision or what is in it, or, in a file read from disk, an element the model does not
/// hold, such as input data, a cell or a diagram). DMN's ids are XML ids, one per element of a
/// document.
fn clashing_id(e: &Engine, file: &str, decision: &Decision) -> Option<String> {
    let mut others: Vec<String> = Vec::new();
    if let Some(defs) = e.model.dmn.get(file) {
        others.extend(Some(defs.id.clone()).filter(|i| !i.is_empty()));
        for d in defs.decisions.iter().filter(|d| d.id != decision.id) {
            others.extend(d.ids().into_iter().map(str::to_owned));
        }
    }
    if let Some(f) = e.dmn.get(file) {
        // The file's ids less those of what the model holds of it, as it was read.
        let mut held = f.read.ids();
        for id in stochcraft_io::dmn::ids(&f.bytes).unwrap_or_default() {
            match held.iter().position(|h| *h == id) {
                Some(k) => {
                    held.remove(k);
                }
                None => others.push(id),
            }
        }
    }
    let mine = decision.ids();
    mine.iter()
        .enumerate()
        .find(|(k, id)| mine.iter().take(*k).any(|x| x == *id) || others.iter().any(|o| o == *id))
        .map(|(_, id)| (*id).to_owned())
}

fn finding_json(f: &stochcraft_model::dmn::Finding) -> Value {
    match f {
        stochcraft_model::dmn::Finding::Overlap {
            first,
            second,
            example,
        } => json!({ "kind": "overlap", "rules": [first, second], "example": example }),
        stochcraft_model::dmn::Finding::Gap { example } => {
            json!({ "kind": "gap", "example": example })
        }
        stochcraft_model::dmn::Finding::Unchecked => json!({ "kind": "unchecked" }),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DmnUse {
    /// The block: a task or an exclusive gateway.
    pub element: String,
    /// The `.dmn` file (one the model reads); leave out `file` and `decision` to stop using one.
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub decision: Option<String>,
    /// On a gateway whose table has several outputs: the output to route by.
    #[serde(default)]
    pub route: Option<String>,
}

pub(crate) fn use_table(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: DmnUse = parse(p)?;
    let id = ElementId::unchecked(&p.element);
    if e.model.element(&id).is_none() {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            format!("no element with id `{}`", p.element),
        ));
    }
    let to = match (p.file, p.decision) {
        (Some(file), Some(decision)) => {
            if e.model
                .dmn
                .get(&file)
                .and_then(|f| f.decision(&decision))
                .is_none()
            {
                return Err(CommandError::new(
                    ErrorCode::NotFound,
                    format!(
                        "the model reads no decision `{decision}` in `{file}`: open the model with it beside, or make it with dmn.set"
                    ),
                ));
            }
            Some(DecisionUse {
                file,
                decision,
                // A blank route is none, as the model's file reads it back.
                route: p.route.filter(|r| !r.trim().is_empty()),
            })
        }
        (None, None) => None,
        _ => {
            return Err(CommandError::new(
                ErrorCode::InvalidParams,
                "give both `file` and `decision`, or neither to stop using a table",
            ));
        }
    };
    e.edit("dmn.use", Op::SetDecisionUse(id, to))?;
    Ok(json!({}))
}
