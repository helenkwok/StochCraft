//! A StochCraft model in memory.
//!
//! This is not a file format. `stochcraft-io` reads and writes models as BPSim on BPMN or XPDL
//! (see ROADMAP.md); this crate is what those files map to, what the engine's commands change and
//! what `stochcraft-blocks` turns into a simulation.
//!
//! - Every element and connection has an [`ElementId`]: the id of its BPMN or XPDL element, kept
//!   stable so files diff cleanly, connections survive edits and agents can name things.
//! - Times are `f64` in the experiment's [`TimeUnit`] (BPSim's `baseTimeUnit`, minutes by default).
//! - [`Model::validate`] reports every problem at once as [`Diagnostic`]s, so a person or an agent
//!   can fix them all in one pass.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub use stochcraft_random::Dist;

/// The id of a model element or connection. Ids follow the XML `NCName` rule that BPMN and XPDL
/// ids obey: a letter or `_` first, then letters, digits, `.`, `-` or `_`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementId(String);

impl ElementId {
    /// The id, if `s` is a valid one.
    pub fn new(s: &str) -> Option<Self> {
        Self::is_valid(s).then(|| Self(s.to_owned()))
    }

    pub fn is_valid(s: &str) -> bool {
        let mut chars = s.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        (first.is_alphabetic() || first == '_')
            && chars.all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ElementId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// BPSim's time units (`TimeUnit`, section 6.1.4 of the specification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeUnit {
    Milliseconds,
    Seconds,
    #[default]
    Minutes,
    Hours,
    Days,
    /// BPSim's `year`, taken as 365 days.
    Years,
}

impl TimeUnit {
    /// The unit's BPSim spelling.
    pub fn bpsim_name(self) -> &'static str {
        match self {
            TimeUnit::Milliseconds => "ms",
            TimeUnit::Seconds => "s",
            TimeUnit::Minutes => "min",
            TimeUnit::Hours => "hour",
            TimeUnit::Days => "day",
            TimeUnit::Years => "year",
        }
    }

    pub fn from_bpsim_name(s: &str) -> Option<Self> {
        Some(match s {
            "ms" => TimeUnit::Milliseconds,
            "s" => TimeUnit::Seconds,
            "min" => TimeUnit::Minutes,
            "hour" => TimeUnit::Hours,
            "day" => TimeUnit::Days,
            "year" => TimeUnit::Years,
            _ => return None,
        })
    }

    /// Seconds in one unit.
    pub fn seconds(self) -> f64 {
        match self {
            TimeUnit::Milliseconds => 0.001,
            TimeUnit::Seconds => 1.0,
            TimeUnit::Minutes => 60.0,
            TimeUnit::Hours => 3_600.0,
            TimeUnit::Days => 86_400.0,
            TimeUnit::Years => 365.0 * 86_400.0,
        }
    }

    /// `value` in `from` units, expressed in `self` units.
    pub fn convert(self, value: f64, from: TimeUnit) -> f64 {
        value * from.seconds() / self.seconds()
    }
}

/// What an element does. M2 adds gateways, resources and the rest (ROADMAP.md).
#[derive(Debug, Clone, PartialEq)]
pub enum ElementKind {
    /// Creates entities (a BPMN start event): time between arrivals, and an optional limit
    /// (BPSim `interTriggerTimer` and `triggerCount`).
    Source {
        interarrival: Dist,
        limit: Option<u64>,
    },
    /// `capacity` identical units serving one entity each, first come first served (a task with
    /// a resource): BPSim `processingTime`.
    Server { capacity: u32, service: Dist },
    /// Holds each entity for a time with no capacity limit (a task with no resource).
    Delay { duration: Dist },
    /// Removes entities (a BPMN end event).
    Sink,
}

impl ElementKind {
    pub fn name(&self) -> &'static str {
        match self {
            ElementKind::Source { .. } => "source",
            ElementKind::Server { .. } => "server",
            ElementKind::Delay { .. } => "delay",
            ElementKind::Sink => "sink",
        }
    }

    fn distributions(&self) -> Vec<(&'static str, &Dist)> {
        match self {
            ElementKind::Source { interarrival, .. } => vec![("interarrival", interarrival)],
            ElementKind::Server { service, .. } => vec![("service", service)],
            ElementKind::Delay { duration } => vec![("duration", duration)],
            ElementKind::Sink => vec![],
        }
    }
}

/// A model element.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub id: ElementId,
    pub name: String,
    pub kind: ElementKind,
}

/// A connection from one element to another (a BPMN sequence flow).
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub id: ElementId,
    pub from: ElementId,
    pub to: ElementId,
}

/// How to run the model (a BPSim scenario's parameters).
#[derive(Debug, Clone, PartialEq)]
pub struct Experiment {
    pub time_unit: TimeUnit,
    /// Simulated time before statistics are collected.
    pub warmup: f64,
    /// Simulated time over which statistics are collected, after the warm-up.
    pub duration: f64,
    pub replications: u32,
    pub seed: u64,
}

impl Default for Experiment {
    fn default() -> Self {
        Self {
            time_unit: TimeUnit::Minutes,
            warmup: 0.0,
            duration: 480.0,
            replications: 10,
            seed: 1,
        }
    }
}

/// A whole model.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    pub name: String,
    pub elements: Vec<Element>,
    pub connections: Vec<Connection>,
    pub experiment: Experiment,
}

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The model runs, but probably not as intended.
    Warning,
    /// The model cannot run.
    Error,
}

/// What a diagnostic is about. Stable, so the UI, the CLI and agents can act on it.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    InvalidId {
        id: String,
    },
    DuplicateId {
        id: ElementId,
    },
    UnknownElement {
        connection: ElementId,
        missing: ElementId,
    },
    IntoSource {
        connection: ElementId,
    },
    OutOfSink {
        connection: ElementId,
    },
    SelfLoop {
        connection: ElementId,
    },
    NoOutput,
    SeveralOutputs {
        count: usize,
    },
    BadDistribution {
        parameter: &'static str,
        error: stochcraft_random::DistError,
    },
    ZeroCapacity,
    NoSource,
    NoSink,
    Unreachable,
    NoPathToSink,
    BadExperiment {
        field: &'static str,
        requirement: &'static str,
    },
}

/// One problem found by [`Model::validate`].
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// The element or connection it concerns, if any.
    pub at: Option<ElementId>,
    pub problem: Problem,
}

impl Diagnostic {
    fn error(at: Option<&ElementId>, problem: Problem) -> Self {
        Self {
            severity: Severity::Error,
            at: at.cloned(),
            problem,
        }
    }

    fn warning(at: Option<&ElementId>, problem: Problem) -> Self {
        Self {
            severity: Severity::Warning,
            at: at.cloned(),
            problem,
        }
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{level}")?;
        if let Some(at) = &self.at {
            write!(f, " at `{at}`")?;
        }
        write!(f, ": ")?;
        match &self.problem {
            Problem::InvalidId { id } => write!(
                f,
                "`{id}` is not a valid id (a letter or _ first, then letters, digits, . - _)"
            ),
            Problem::DuplicateId { id } => write!(f, "the id `{id}` is used more than once"),
            Problem::UnknownElement { missing, .. } => {
                write!(f, "connects to `{missing}`, which does not exist")
            }
            Problem::IntoSource { .. } => write!(f, "connects into a source, which takes no input"),
            Problem::OutOfSink { .. } => write!(f, "leaves a sink, which has no output"),
            Problem::SelfLoop { .. } => write!(f, "connects an element to itself"),
            Problem::NoOutput => write!(f, "has nowhere to send entities"),
            Problem::SeveralOutputs { count } => {
                write!(
                    f,
                    "has {count} outgoing connections; choosing between them needs a gateway (M2)"
                )
            }
            Problem::BadDistribution { parameter, error } => write!(f, "{parameter}: {error}"),
            Problem::ZeroCapacity => write!(f, "a server needs at least 1 unit"),
            Problem::NoSource => write!(f, "the model has no source, so nothing will happen"),
            Problem::NoSink => write!(f, "the model has no sink, so no time in system is measured"),
            Problem::Unreachable => write!(f, "no entity can reach this element from a source"),
            Problem::NoPathToSink => write!(f, "entities here can never reach a sink"),
            Problem::BadExperiment { field, requirement } => {
                write!(f, "experiment {field} must be {requirement}")
            }
        }
    }
}

impl Model {
    pub fn element(&self, id: &ElementId) -> Option<&Element> {
        self.elements.iter().find(|e| &e.id == id)
    }

    /// Every problem with the model, errors first, each group in model order.
    pub fn validate(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        self.check_ids(&mut out);
        let elements: BTreeMap<&ElementId, &Element> =
            self.elements.iter().map(|e| (&e.id, e)).collect();
        let mut outgoing: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
        for c in &self.connections {
            let mut ok = true;
            for end in [&c.from, &c.to] {
                if !elements.contains_key(end) {
                    out.push(Diagnostic::error(
                        Some(&c.id),
                        Problem::UnknownElement {
                            connection: c.id.clone(),
                            missing: end.clone(),
                        },
                    ));
                    ok = false;
                }
            }
            if c.from == c.to {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::SelfLoop {
                        connection: c.id.clone(),
                    },
                ));
                ok = false;
            }
            if let Some(to) = elements.get(&c.to)
                && matches!(to.kind, ElementKind::Source { .. })
            {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::IntoSource {
                        connection: c.id.clone(),
                    },
                ));
                ok = false;
            }
            if let Some(from) = elements.get(&c.from)
                && matches!(from.kind, ElementKind::Sink)
            {
                out.push(Diagnostic::error(
                    Some(&c.id),
                    Problem::OutOfSink {
                        connection: c.id.clone(),
                    },
                ));
                ok = false;
            }
            if ok {
                outgoing.entry(&c.from).or_default().push(&c.to);
            }
        }
        for e in &self.elements {
            let outs = outgoing.get(&e.id).map_or(0, Vec::len);
            match e.kind {
                ElementKind::Sink => {}
                _ if outs == 0 => out.push(Diagnostic::error(Some(&e.id), Problem::NoOutput)),
                _ if outs > 1 => out.push(Diagnostic::error(
                    Some(&e.id),
                    Problem::SeveralOutputs { count: outs },
                )),
                _ => {}
            }
            if let ElementKind::Server { capacity: 0, .. } = e.kind {
                out.push(Diagnostic::error(Some(&e.id), Problem::ZeroCapacity));
            }
            for (parameter, d) in e.kind.distributions() {
                if let Err(error) = d.check() {
                    out.push(Diagnostic::error(
                        Some(&e.id),
                        Problem::BadDistribution { parameter, error },
                    ));
                }
            }
        }
        self.check_experiment(&mut out);
        self.check_reachability(&outgoing, &mut out);
        // Stable order: errors first, then as found (model order).
        out.sort_by_key(|d| std::cmp::Reverse(d.severity));
        out
    }

    /// Whether the model has no errors (warnings allowed).
    pub fn is_runnable(&self) -> bool {
        self.validate().iter().all(|d| d.severity < Severity::Error)
    }

    fn check_ids(&self, out: &mut Vec<Diagnostic>) {
        let mut seen = BTreeSet::new();
        let ids = self
            .elements
            .iter()
            .map(|e| &e.id)
            .chain(self.connections.iter().map(|c| &c.id));
        for id in ids {
            if !ElementId::is_valid(id.as_str()) {
                out.push(Diagnostic::error(
                    None,
                    Problem::InvalidId {
                        id: id.as_str().to_owned(),
                    },
                ));
            }
            if !seen.insert(id) {
                out.push(Diagnostic::error(
                    Some(id),
                    Problem::DuplicateId { id: id.clone() },
                ));
            }
        }
    }

    fn check_experiment(&self, out: &mut Vec<Diagnostic>) {
        let x = &self.experiment;
        if !(x.warmup.is_finite() && x.warmup >= 0.0) {
            out.push(Diagnostic::error(
                None,
                Problem::BadExperiment {
                    field: "warm-up",
                    requirement: "0 or more",
                },
            ));
        }
        if !(x.duration.is_finite() && x.duration > 0.0) {
            out.push(Diagnostic::error(
                None,
                Problem::BadExperiment {
                    field: "duration",
                    requirement: "greater than 0",
                },
            ));
        }
        if x.replications == 0 {
            out.push(Diagnostic::error(
                None,
                Problem::BadExperiment {
                    field: "replications",
                    requirement: "1 or more",
                },
            ));
        }
    }

    fn check_reachability(
        &self,
        outgoing: &BTreeMap<&ElementId, Vec<&ElementId>>,
        out: &mut Vec<Diagnostic>,
    ) {
        let sources: Vec<&ElementId> = self
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Source { .. }))
            .map(|e| &e.id)
            .collect();
        let sinks: Vec<&ElementId> = self
            .elements
            .iter()
            .filter(|e| matches!(e.kind, ElementKind::Sink))
            .map(|e| &e.id)
            .collect();
        if sources.is_empty() && !self.elements.is_empty() {
            out.push(Diagnostic::warning(None, Problem::NoSource));
        }
        if sinks.is_empty() && !self.elements.is_empty() {
            out.push(Diagnostic::warning(None, Problem::NoSink));
        }
        let forward = reach(&sources, |id| outgoing.get(id).cloned().unwrap_or_default());
        let mut incoming: BTreeMap<&ElementId, Vec<&ElementId>> = BTreeMap::new();
        for (from, tos) in outgoing {
            for to in tos {
                incoming.entry(to).or_default().push(from);
            }
        }
        let backward = reach(&sinks, |id| incoming.get(id).cloned().unwrap_or_default());
        for e in &self.elements {
            if !sources.is_empty() && !forward.contains(&e.id) {
                out.push(Diagnostic::warning(Some(&e.id), Problem::Unreachable));
            } else if !sinks.is_empty() && !backward.contains(&e.id) {
                out.push(Diagnostic::warning(Some(&e.id), Problem::NoPathToSink));
            }
        }
    }
}

fn reach<'a>(
    start: &[&'a ElementId],
    next: impl Fn(&'a ElementId) -> Vec<&'a ElementId>,
) -> BTreeSet<&'a ElementId> {
    let mut seen: BTreeSet<&ElementId> = start.iter().copied().collect();
    let mut queue: VecDeque<&ElementId> = start.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        for n in next(id) {
            if seen.insert(n) {
                queue.push_back(n);
            }
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> ElementId {
        ElementId(s.to_owned())
    }

    fn el(i: &str, kind: ElementKind) -> Element {
        Element {
            id: id(i),
            name: i.to_owned(),
            kind,
        }
    }

    fn link(i: &str, from: &str, to: &str) -> Connection {
        Connection {
            id: id(i),
            from: id(from),
            to: id(to),
        }
    }

    fn mm1() -> Model {
        Model {
            name: "M/M/1".into(),
            elements: vec![
                el(
                    "arrivals",
                    ElementKind::Source {
                        interarrival: Dist::exponential_rate(0.75),
                        limit: None,
                    },
                ),
                el(
                    "machine",
                    ElementKind::Server {
                        capacity: 1,
                        service: Dist::exponential_rate(1.0),
                    },
                ),
                el("exit", ElementKind::Sink),
            ],
            connections: vec![
                link("f1", "arrivals", "machine"),
                link("f2", "machine", "exit"),
            ],
            experiment: Experiment::default(),
        }
    }

    fn problems(m: &Model) -> Vec<(Severity, Option<String>, String)> {
        m.validate()
            .into_iter()
            .map(|d| {
                (
                    d.severity,
                    d.at.as_ref().map(|a| a.to_string()),
                    format!("{:?}", d.problem),
                )
            })
            .collect()
    }

    #[test]
    fn a_valid_model_has_no_diagnostics() {
        assert_eq!(mm1().validate(), vec![]);
        assert!(mm1().is_runnable());
    }

    #[test]
    fn ids_follow_ncname() {
        for ok in [
            "a",
            "_x",
            "machine-1",
            "Task_1.2",
            "_FA77D7D8-9C02-4006-942A-B003AEDA5E3C",
            "工位",
        ] {
            assert!(ElementId::is_valid(ok), "{ok}");
        }
        for bad in ["", "1abc", "-a", "a b", "a:b", ".x"] {
            assert!(!ElementId::is_valid(bad), "{bad}");
            assert_eq!(ElementId::new(bad), None);
        }
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let mut m = mm1();
        m.elements.push(el(
            "machine",
            ElementKind::Delay {
                duration: Dist::Constant(1.0),
            },
        ));
        m.elements.push(el(
            "idle",
            ElementKind::Server {
                capacity: 0,
                service: Dist::Uniform { min: 2.0, max: 1.0 },
            },
        ));
        m.connections.push(link("f3", "exit", "arrivals"));
        m.connections.push(link("f4", "ghost", "exit"));
        m.experiment.replications = 0;
        let p = problems(&m);
        let has = |frag: &str| p.iter().any(|(_, _, s)| s.contains(frag));
        for frag in [
            "DuplicateId",
            "IntoSource",
            "OutOfSink",
            "UnknownElement",
            "ZeroCapacity",
            "BadDistribution",
            "NoOutput",
            "BadExperiment",
        ] {
            assert!(has(frag), "missing {frag} in {p:#?}");
        }
        // Errors come before warnings.
        let first_warning = p
            .iter()
            .position(|(s, _, _)| *s == Severity::Warning)
            .unwrap_or(p.len());
        assert!(
            p[first_warning..]
                .iter()
                .all(|(s, _, _)| *s == Severity::Warning)
        );
        assert!(!m.is_runnable());
    }

    #[test]
    fn branching_needs_a_gateway() {
        let mut m = mm1();
        m.elements.push(el("scrap", ElementKind::Sink));
        m.connections.push(link("f3", "machine", "scrap"));
        assert_eq!(
            problems(&m),
            vec![(
                Severity::Error,
                Some("machine".into()),
                "SeveralOutputs { count: 2 }".into()
            )]
        );
    }

    #[test]
    fn reachability_warnings() {
        let mut m = mm1();
        // An island that no source feeds, and a loop entities can never leave.
        m.elements.push(el(
            "orphan",
            ElementKind::Delay {
                duration: Dist::Constant(1.0),
            },
        ));
        m.connections.push(link("f3", "orphan", "exit"));
        m.elements.push(el(
            "a",
            ElementKind::Delay {
                duration: Dist::Constant(1.0),
            },
        ));
        m.elements.push(el(
            "b",
            ElementKind::Delay {
                duration: Dist::Constant(1.0),
            },
        ));
        m.elements.push(el(
            "src2",
            ElementKind::Source {
                interarrival: Dist::Constant(1.0),
                limit: Some(3),
            },
        ));
        m.connections.push(link("f4", "src2", "a"));
        m.connections.push(link("f5", "a", "b"));
        m.connections.push(link("f6", "b", "a"));
        let p = problems(&m);
        assert!(
            p.contains(&(
                Severity::Warning,
                Some("orphan".into()),
                "Unreachable".into()
            )),
            "{p:#?}"
        );
        for x in ["src2", "a", "b"] {
            assert!(
                p.contains(&(Severity::Warning, Some(x.into()), "NoPathToSink".into())),
                "{x}: {p:#?}"
            );
        }
        assert!(m.is_runnable(), "warnings alone do not stop a run");
    }

    #[test]
    fn self_loops_and_empty_models() {
        let mut m = mm1();
        m.connections.push(link("f3", "machine", "machine"));
        assert!(
            problems(&m)
                .iter()
                .any(|(_, at, s)| at.as_deref() == Some("f3") && s.contains("SelfLoop"))
        );
        let empty = Model::default();
        assert_eq!(empty.validate(), vec![]);
        let lonely = Model {
            elements: vec![el("x", ElementKind::Sink)],
            ..Model::default()
        };
        assert!(problems(&lonely).iter().any(|(_, _, s)| s == "NoSource"));
    }

    #[test]
    fn time_units_match_bpsim() {
        for u in [
            TimeUnit::Milliseconds,
            TimeUnit::Seconds,
            TimeUnit::Minutes,
            TimeUnit::Hours,
            TimeUnit::Days,
            TimeUnit::Years,
        ] {
            assert_eq!(TimeUnit::from_bpsim_name(u.bpsim_name()), Some(u));
        }
        assert_eq!(
            TimeUnit::default(),
            TimeUnit::Minutes,
            "BPSim's default base time unit"
        );
        assert_eq!(TimeUnit::Minutes.convert(2.0, TimeUnit::Hours), 120.0);
        assert_eq!(TimeUnit::Hours.convert(90.0, TimeUnit::Minutes), 1.5);
        assert_eq!(TimeUnit::from_bpsim_name("minutes"), None);
    }

    #[test]
    fn diagnostics_read_well() {
        let mut m = mm1();
        m.connections.retain(|c| c.id.as_str() != "f2");
        let text: Vec<String> = m.validate().iter().map(ToString::to_string).collect();
        assert!(
            text.contains(&"error at `machine`: has nowhere to send entities".to_string()),
            "{text:#?}"
        );
    }
}
