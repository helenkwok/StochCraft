//! An agent type's state chart as a W3C SCXML 1.0 document (ROADMAP M4.3, decisions D16 and
//! D17): [`read_scxml`] and [`write_scxml`] between the file and a
//! [`ChartSpec`](stochcraft_agents::ChartSpec).
//!
//! What SCXML covers is written as SCXML 1.0 and is valid against W3C's XML Schema: `<scxml
//! version="1.0" datamodel="stochcraft">`, `<datamodel>` and `<data id expr>`, `<state>`,
//! `<parallel>`, `<final>`, `<history type>`, `<onentry>`, `<onexit>`, `<transition event cond
//! target type>`, and the executable content `<assign>`, `<raise>`, `<send>` (with `<content
//! expr>` for the data it carries), `<cancel>` and `<log>`.
//!
//! What only StochCraft does is in the namespace [`STOCHCRAFT_SCXML_NS`] (prefix `stochcraft`,
//! declared by `stochcraft-scxml-1.xsd`), which SCXML allows on any element. Generic SCXML tools
//! read the chart's structure but will not run it as StochCraft does:
//!
//! - the expressions are D17's `stochcraft` data model (BPSim's XPath subset with `In()` and the
//!   functions of [`stochcraft_agents::FUNCTIONS`]), and a `delayexpr` gives a time in the
//!   model's time unit, not a CSS2 time;
//! - a rate (`<send stochcraft:rate="…">`, an exponential delay drawn again when what the rate
//!   reads changes) also carries `delayexpr="exponential(1 div (…))"`, the same delay drawn once;
//! - a fixed `delay` is a CSS2 time in seconds, converted from the model's time unit; where that
//!   conversion would not give the same number back, `stochcraft:delay` carries the exact value
//!   in the model's time unit, and a delay that is no CSS2 time (below zero) is written only
//!   there;
//! - a message to other agents (`stochcraft:to`, `stochcraft:population`, `stochcraft:serial`)
//!   names StochCraft's own event I/O processor in its `type` ([`AGENTS_PROCESSOR`]), which
//!   other tools do not have and report as an error rather than send to the agent itself;
//! - creating, removing, linking and moving agents are elements of the `stochcraft` namespace
//!   (`<stochcraft:create population>`, `remove`, `connect`, `disconnect`, `moveTo`,
//!   `moveRandom`).
//!
//! Reading refuses, by name, anything a chart cannot hold yet (`<if>`, `<foreach>`, `<script>`,
//! `<invoke>`, `<param>`, `<donedata>`, nested `<datamodel>`s, late binding, other data models,
//! computed event names, targets and ids), rather than dropping it and running something else.
//! Writing refuses what SCXML's schema would not accept: an id that is not an XML name, two
//! states, variables or sends with one id (SCXML gives them one id space), and an event name
//! that is not one.

use stochcraft_agents::{
    Action, ChartSpec, Data, Delay, History, Kind, SendSpec, StateSpec, Target, To, TransitionSpec,
};

use crate::xml::{self, Document, Element, Node};

/// SCXML 1.0's namespace.
pub const SCXML_NS: &str = "http://www.w3.org/2005/07/scxml";

/// The namespace of StochCraft's extensions to SCXML.
pub const STOCHCRAFT_SCXML_NS: &str = "https://helenkwok.github.io/StochCraft/scxml/1";

/// The `type` of a `<send>` to other agents: StochCraft's own event I/O processor.
pub const AGENTS_PROCESSOR: &str = "https://helenkwok.github.io/StochCraft/scxml/1#agents";

/// SCXML's own event I/O processor, the default `type` of a `<send>`.
const SCXML_PROCESSOR: &str = "http://www.w3.org/TR/scxml/#SCXMLEventProcessor";

/// Why a chart cannot be read or written: where, and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScxmlError {
    pub place: String,
    pub message: String,
}

impl std::fmt::Display for ScxmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.place.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.place, self.message)
        }
    }
}

impl std::error::Error for ScxmlError {}

fn err<T>(place: &str, message: impl Into<String>) -> Result<T, ScxmlError> {
    Err(ScxmlError {
        place: place.to_owned(),
        message: message.into(),
    })
}

// ---------------------------------------------------------------------------------------------
// Reading

/// The namespace prefixes in scope, innermost last.
type Scope = Vec<(String, String)>;

fn push_scope(scope: &mut Scope, e: &Element) -> usize {
    let mut n = 0;
    for (name, value) in &e.attrs {
        if let Some(p) = name.strip_prefix("xmlns:") {
            scope.push((p.to_owned(), xml::unescape(value)));
            n += 1;
        }
    }
    n
}

/// The value of the attribute `local` in namespace `ns`, whatever its prefix.
fn ns_attr(e: &Element, scope: &Scope, ns: &str, local: &str) -> Option<String> {
    e.attrs.iter().find_map(|(name, value)| {
        let (p, l) = name.split_once(':')?;
        if l != local || p == "xmlns" {
            return None;
        }
        let uri = scope.iter().rev().find(|(q, _)| q == p).map(|(_, u)| u)?;
        (uri == ns).then(|| xml::unescape(value))
    })
}

/// Reads a state chart from the bytes of an SCXML 1.0 document. `seconds_per_unit` is the
/// length of the model's time unit in seconds, which a fixed `delay` (a CSS2 time) is converted
/// by.
pub fn read_scxml(bytes: &[u8], seconds_per_unit: f64) -> Result<ChartSpec, ScxmlError> {
    let doc = xml::parse(bytes).map_err(|e| ScxmlError {
        place: String::new(),
        message: e.to_string(),
    })?;
    let root = &doc.root;
    if !root.is(SCXML_NS, "scxml") {
        return err(
            "",
            format!(
                "the root element is `{}`, not SCXML's `scxml` (namespace {SCXML_NS})",
                root.name
            ),
        );
    }
    let scope: Scope = vec![("xml".into(), "http://www.w3.org/XML/1998/namespace".into())];
    check_ours(root, &mut scope.clone())?;
    let mut r = Reader {
        scope,
        seconds: seconds_per_unit,
    };
    r.chart(root)
}

/// StochCraft's attributes that a `<send>` may carry.
const SEND_ATTRIBUTES: [&str; 5] = ["rate", "delay", "to", "population", "serial"];

/// Refuses an attribute of StochCraft's namespace that StochCraft does not read where it is
/// (a misspelt one, or one on another element than `<send>`), rather than drop it.
fn check_ours(e: &Element, scope: &mut Scope) -> Result<(), ScxmlError> {
    let pushed = push_scope(scope, e);
    for (name, _) in &e.attrs {
        let Some((p, local)) = name.split_once(':') else {
            continue;
        };
        if p == "xmlns" {
            continue;
        }
        let ours = scope
            .iter()
            .rev()
            .find(|(q, _)| q == p)
            .is_some_and(|(_, u)| u == STOCHCRAFT_SCXML_NS);
        if ours && !(e.is(SCXML_NS, "send") && SEND_ATTRIBUTES.contains(&local)) {
            return err(
                &e.attr("id").map(|i| format!("`{i}`")).unwrap_or_default(),
                format!(
                    "the attribute `{name}` of `<{}>` is not one StochCraft reads there",
                    e.local()
                ),
            );
        }
    }
    for c in e.elements() {
        check_ours(c, scope)?;
    }
    scope.truncate(scope.len() - pushed);
    Ok(())
}

struct Reader {
    scope: Scope,
    seconds: f64,
}

/// The attributes of an SCXML element StochCraft reads; any other unprefixed attribute is
/// refused, so nothing that changes behaviour is dropped.
fn check_attrs(e: &Element, place: &str, known: &[&str]) -> Result<(), ScxmlError> {
    for (name, _) in &e.attrs {
        if name.contains(':') || name == "xmlns" {
            continue;
        }
        if !known.contains(&name.as_str()) {
            return err(
                place,
                format!(
                    "the attribute `{name}` of `<{}>` is not supported",
                    e.local()
                ),
            );
        }
    }
    Ok(())
}

fn ids(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(str::to_owned).collect()
}

/// A child that is text other than white space, or an element SCXML does not define there.
fn no_text(e: &Element, place: &str) -> Result<(), ScxmlError> {
    for n in &e.children {
        match n {
            Node::Text(t) if !xml::unescape(t).trim().is_empty() => {
                return err(
                    place,
                    format!("`<{}>` has text, which is not read", e.local()),
                );
            }
            Node::CData(t) if !t.trim().is_empty() => {
                return err(
                    place,
                    format!("`<{}>` has text, which is not read", e.local()),
                );
            }
            _ => {}
        }
    }
    Ok(())
}

fn unsupported(e: &Element, place: &str) -> ScxmlError {
    let what = match e.local() {
        "if" | "elseif" | "else" => "conditional content (`<if>`) is not supported yet",
        "foreach" => "`<foreach>` is not supported yet",
        "script" => "`<script>` is not part of the stochcraft data model (D17)",
        "invoke" => "`<invoke>` is not supported",
        "param" => "`<param>` is not supported: a send carries one number, in `<content expr>`",
        "donedata" => "`<donedata>` is not supported",
        "datamodel" => {
            "a `<datamodel>` inside a state is not supported: declare every variable at the top"
        }
        _ => "",
    };
    let message = if what.is_empty() {
        format!("`<{}>` is not supported here", e.name)
    } else {
        what.to_owned()
    };
    ScxmlError {
        place: place.to_owned(),
        message,
    }
}

impl Reader {
    fn chart(&mut self, root: &Element) -> Result<ChartSpec, ScxmlError> {
        let pushed = push_scope(&mut self.scope, root);
        check_attrs(
            root,
            "scxml",
            &["version", "name", "initial", "datamodel", "binding"],
        )?;
        match root.attr("version").as_deref() {
            Some("1.0") => {}
            Some(v) => return err("scxml", format!("version `{v}` is not SCXML 1.0")),
            None => return err("scxml", "the `version` attribute (1.0) is missing"),
        }
        match root.attr("datamodel").as_deref() {
            None | Some("stochcraft") | Some("null") => {}
            Some(other) => {
                return err(
                    "scxml",
                    format!(
                        "the data model `{other}` is not StochCraft's: charts use datamodel=\"stochcraft\" (D17)"
                    ),
                );
            }
        }
        if let Some(b) = root.attr("binding")
            && b != "early"
        {
            return err(
                "scxml",
                format!(
                    "binding `{b}` is not supported: variables take their first values at the start (early binding)"
                ),
            );
        }
        no_text(root, "scxml")?;
        let mut chart = ChartSpec {
            name: root.attr("name").unwrap_or_default(),
            initial: root.attr("initial").map(|s| ids(&s)).unwrap_or_default(),
            data: Vec::new(),
            states: Vec::new(),
        };
        for c in root.elements() {
            if !c.is(SCXML_NS, c.local()) {
                return Err(unsupported(c, "scxml"));
            }
            match c.local() {
                "datamodel" => chart.data.extend(self.datamodel(c)?),
                "state" | "parallel" | "final" => chart.states.push(self.state(c)?),
                _ => return Err(unsupported(c, "scxml")),
            }
        }
        self.scope.truncate(self.scope.len() - pushed);
        Ok(chart)
    }

    fn datamodel(&mut self, e: &Element) -> Result<Vec<Data>, ScxmlError> {
        let pushed = push_scope(&mut self.scope, e);
        check_attrs(e, "datamodel", &[])?;
        no_text(e, "datamodel")?;
        let mut out = Vec::new();
        for d in e.elements() {
            if !d.is(SCXML_NS, "data") {
                return Err(unsupported(d, "datamodel"));
            }
            let Some(id) = d.attr("id") else {
                return err("datamodel", "a `<data>` has no `id`");
            };
            let place = format!("data `{id}`");
            check_attrs(d, &place, &["id", "expr", "src"])?;
            if d.attr("src").is_some() || d.elements().next().is_some() {
                return err(
                    &place,
                    "only `expr` gives a first value: `src` and content are not supported",
                );
            }
            no_text(d, &place)?;
            let Some(expr) = d.attr("expr") else {
                return err(&place, "no `expr` gives its first value");
            };
            out.push(Data { id, expr });
        }
        self.scope.truncate(self.scope.len() - pushed);
        Ok(out)
    }

    fn state(&mut self, e: &Element) -> Result<StateSpec, ScxmlError> {
        let pushed = push_scope(&mut self.scope, e);
        let Some(id) = e.attr("id") else {
            return err("", format!("a `<{}>` has no `id`", e.local()));
        };
        let place = format!("state `{id}`");
        let kind = match e.local() {
            "state" => Kind::State,
            "parallel" => Kind::Parallel,
            "final" => Kind::Final,
            "history" => match e.attr("type").as_deref() {
                None | Some("shallow") => Kind::History(History::Shallow),
                Some("deep") => Kind::History(History::Deep),
                Some(t) => {
                    return err(&place, format!("history type `{t}` is not shallow or deep"));
                }
            },
            _ => return Err(unsupported(e, &place)),
        };
        match kind {
            Kind::State => check_attrs(e, &place, &["id", "initial"])?,
            Kind::History(_) => check_attrs(e, &place, &["id", "type"])?,
            _ => check_attrs(e, &place, &["id"])?,
        }
        no_text(e, &place)?;
        let mut s = StateSpec {
            id: id.clone(),
            kind,
            initial: e.attr("initial").map(|v| ids(&v)).unwrap_or_default(),
            on_entry: Vec::new(),
            on_exit: Vec::new(),
            transitions: Vec::new(),
            children: Vec::new(),
        };
        for c in e.elements() {
            if !c.is(SCXML_NS, c.local()) {
                return Err(unsupported(c, &place));
            }
            let history = matches!(kind, Kind::History(_));
            match c.local() {
                // Each block on its own: an error in one stops only that one (SCXML §4.9).
                "onentry" if !history => s.on_entry.push(self.block(c, &place)?),
                "onexit" if !history => s.on_exit.push(self.block(c, &place)?),
                "transition" if kind != Kind::Final => {
                    s.transitions.push(self.transition(c, &place)?)
                }
                "initial" if kind == Kind::State && s.initial.is_empty() => {
                    s.initial = self.initial_element(c, &place)?;
                }
                "state" | "parallel" | "final" | "history"
                    if matches!(kind, Kind::State | Kind::Parallel) =>
                {
                    s.children.push(self.state(c)?);
                }
                _ => return Err(unsupported(c, &place)),
            }
        }
        if matches!(kind, Kind::History(_)) && s.transitions.len() != 1 {
            return err(&place, "a history state has exactly one default transition");
        }
        self.scope.truncate(self.scope.len() - pushed);
        Ok(s)
    }

    /// `<initial>` with one transition that has targets and no content: the same as the
    /// `initial` attribute.
    fn initial_element(&mut self, e: &Element, place: &str) -> Result<Vec<String>, ScxmlError> {
        check_attrs(e, place, &["id"])?;
        let ts: Vec<&Element> = e.elements().collect();
        match ts.as_slice() {
            [t] if t.is(SCXML_NS, "transition") && t.elements().next().is_none() => {
                check_attrs(t, place, &["target"])?;
                Ok(t.attr("target").map(|v| ids(&v)).unwrap_or_default())
            }
            _ => err(
                place,
                "an `<initial>` element is read only with one transition that has targets and no actions",
            ),
        }
    }

    fn transition(&mut self, e: &Element, state: &str) -> Result<TransitionSpec, ScxmlError> {
        let pushed = push_scope(&mut self.scope, e);
        let place = format!("a transition of {state}");
        check_attrs(e, &place, &["event", "cond", "target", "type"])?;
        let internal = match e.attr("type").as_deref() {
            None | Some("external") => false,
            Some("internal") => true,
            Some(t) => return err(&place, format!("type `{t}` is not internal or external")),
        };
        let t = TransitionSpec {
            event: e.attr("event"),
            cond: e.attr("cond"),
            targets: e.attr("target").map(|v| ids(&v)).unwrap_or_default(),
            internal,
            actions: self.content(e, &place)?,
        };
        self.scope.truncate(self.scope.len() - pushed);
        Ok(t)
    }

    /// `<onentry>` or `<onexit>`.
    fn block(&mut self, e: &Element, state: &str) -> Result<Vec<Action>, ScxmlError> {
        let pushed = push_scope(&mut self.scope, e);
        let place = format!("the {} of {state}", e.local());
        check_attrs(e, &place, &[])?;
        let out = self.content(e, &place)?;
        self.scope.truncate(self.scope.len() - pushed);
        Ok(out)
    }

    /// Executable content.
    fn content(&mut self, e: &Element, place: &str) -> Result<Vec<Action>, ScxmlError> {
        no_text(e, place)?;
        let mut out = Vec::new();
        for c in e.elements() {
            let pushed = push_scope(&mut self.scope, c);
            out.push(self.action(c, place)?);
            self.scope.truncate(self.scope.len() - pushed);
        }
        Ok(out)
    }

    fn action(&mut self, c: &Element, place: &str) -> Result<Action, ScxmlError> {
        let required = |name: &str| -> Result<String, ScxmlError> {
            c.attr(name).ok_or_else(|| ScxmlError {
                place: place.to_owned(),
                message: format!("`<{}>` has no `{name}`", c.local()),
            })
        };
        if c.ns.as_deref() == Some(STOCHCRAFT_SCXML_NS) {
            no_text(c, place)?;
            if let Some(x) = c.elements().next() {
                return Err(unsupported(x, place));
            }
            let a = match c.local() {
                "create" => {
                    check_attrs(c, place, &["population"])?;
                    Action::Create {
                        population: required("population")?,
                    }
                }
                "remove" | "connect" | "disconnect" => {
                    check_attrs(c, place, &["population", "serial"])?;
                    let (population, serial) = (required("population")?, required("serial")?);
                    match c.local() {
                        "remove" => Action::Remove { population, serial },
                        "connect" => Action::Connect { population, serial },
                        _ => Action::Disconnect { population, serial },
                    }
                }
                "moveTo" => {
                    check_attrs(c, place, &["population", "row", "column"])?;
                    Action::MoveTo {
                        population: required("population")?,
                        row: required("row")?,
                        column: required("column")?,
                    }
                }
                "moveRandom" => {
                    check_attrs(c, place, &["population"])?;
                    Action::MoveRandom {
                        population: required("population")?,
                    }
                }
                _ => return Err(unsupported(c, place)),
            };
            return Ok(a);
        }
        if c.ns.as_deref() != Some(SCXML_NS) {
            return Err(unsupported(c, place));
        }
        let simple = |known: &[&str]| -> Result<(), ScxmlError> {
            check_attrs(c, place, known)?;
            no_text(c, place)?;
            match c.elements().next() {
                Some(x) => Err(unsupported(x, place)),
                None => Ok(()),
            }
        };
        match c.local() {
            "assign" => {
                simple(&["location", "expr"])?;
                Ok(Action::Assign {
                    location: required("location")?,
                    expr: required("expr")?,
                })
            }
            "raise" => {
                simple(&["event"])?;
                Ok(Action::Raise {
                    event: required("event")?,
                })
            }
            "cancel" => {
                simple(&["sendid", "sendidexpr"])?;
                let send_id = match (c.attr("sendid"), c.attr("sendidexpr")) {
                    (Some(id), None) => id,
                    (None, Some(x)) => match literal(&x) {
                        Some(id) => id,
                        None => {
                            return err(
                                place,
                                format!(
                                    "a computed send id (`{x}`) is not supported: give a quoted literal"
                                ),
                            );
                        }
                    },
                    _ => {
                        return err(
                            place,
                            "`<cancel>` names its send by `sendid` or `sendidexpr`",
                        );
                    }
                };
                Ok(Action::Cancel { send_id })
            }
            "log" => {
                simple(&["label", "expr"])?;
                Ok(Action::Log {
                    label: c.attr("label").unwrap_or_default(),
                    expr: c.attr("expr"),
                })
            }
            "send" => self.send(c, place).map(Action::SendSpec),
            _ => Err(unsupported(c, place)),
        }
    }

    fn send(&mut self, c: &Element, place: &str) -> Result<SendSpec, ScxmlError> {
        check_attrs(
            c,
            place,
            &["event", "target", "type", "id", "delay", "delayexpr"],
        )?;
        no_text(c, place)?;
        let Some(event) = c.attr("event") else {
            return err(
                place,
                "`<send>` has no `event` (a computed `eventexpr` is not supported)",
            );
        };
        let place = &format!("the send of `{event}` in {place}");
        let st = |local: &str| ns_attr(c, &self.scope, STOCHCRAFT_SCXML_NS, local);
        let mut s = SendSpec::new(&event);
        s.id = c.attr("id");
        s.target = match c.attr("target").as_deref() {
            None => Target::External,
            Some("#_internal") => Target::Internal,
            Some(t) => {
                return err(
                    place,
                    format!(
                        "the target `{t}` is not supported: messages to other agents use stochcraft:to"
                    ),
                );
            }
        };
        match c.attr("type").as_deref() {
            None | Some("scxml") | Some(SCXML_PROCESSOR) | Some(AGENTS_PROCESSOR) => {}
            Some(t) => {
                return err(
                    place,
                    format!("the event I/O processor `{t}` is not supported"),
                );
            }
        }
        s.delay = if let Some(rate) = st("rate") {
            Delay::Rate(rate)
        } else if let Some(exact) = st("delay") {
            match exact.trim().parse::<f64>() {
                Ok(v) => Delay::Fixed(v),
                Err(_) => return err(place, format!("stochcraft:delay `{exact}` is not a number")),
            }
        } else if let Some(x) = c.attr("delayexpr") {
            Delay::Expr(x)
        } else if let Some(d) = c.attr("delay") {
            match css_seconds(&d) {
                Some(secs) => Delay::Fixed(secs / self.seconds),
                None => {
                    return err(
                        place,
                        format!("delay `{d}` is not a CSS2 time such as `2s` or `500ms`"),
                    );
                }
            }
        } else {
            Delay::None
        };
        if let Some(to) = st("to") {
            let population = st("population").ok_or_else(|| ScxmlError {
                place: place.clone(),
                message: "a message to other agents names its stochcraft:population".to_owned(),
            })?;
            s.to = Some(match to.as_str() {
                "agent" => To::Agent {
                    population,
                    serial: st("serial").ok_or_else(|| ScxmlError {
                        place: place.clone(),
                        message: "a message to one agent gives its stochcraft:serial".to_owned(),
                    })?,
                },
                "random" => To::Random { population },
                "all" => To::All { population },
                "linked" => To::Linked { population },
                "linkedRandom" => To::LinkedRandom { population },
                "neighbours" => To::Neighbours { population },
                "neighbourRandom" => To::NeighbourRandom { population },
                other => return err(place, format!("stochcraft:to `{other}` is not known")),
            });
        }
        // What a message to other agents carries, without one, would send the event to the agent
        // itself: refused rather than dropped.
        let serial_unused = !matches!(s.to, Some(To::Agent { .. })) && st("serial").is_some();
        if (s.to.is_none() && st("population").is_some()) || serial_unused {
            return err(
                place,
                "stochcraft:population and stochcraft:serial go with a stochcraft:to that uses them",
            );
        }
        if s.to.is_none() && c.attr("type").as_deref() == Some(AGENTS_PROCESSOR) {
            return err(
                place,
                "a send to StochCraft's agents processor says whom with stochcraft:to",
            );
        }
        if goes_at_once_but_does_not(&s) {
            return err(
                place,
                "a message to other agents goes at once: no target, delay or id",
            );
        }
        for x in c.elements() {
            if !x.is(SCXML_NS, "content") || s.data.is_some() {
                return Err(unsupported(x, place));
            }
            check_attrs(x, place, &["expr"])?;
            no_text(x, place)?;
            if x.elements().next().is_some() {
                return err(place, "`<content>` gives the data by `expr` only");
            }
            s.data = Some(x.attr("expr").ok_or_else(|| ScxmlError {
                place: place.clone(),
                message: "`<content>` gives the data by `expr` only".to_owned(),
            })?);
        }
        Ok(s)
    }
}

/// The string an XPath literal (`'a'` or `"a"`) holds.
fn literal(x: &str) -> Option<String> {
    let x = x.trim();
    let q = x.chars().next()?;
    if !(q == '\'' || q == '"') || x.len() < 2 || !x.ends_with(q) {
        return None;
    }
    let inner = &x[1..x.len() - 1];
    (!inner.contains(q)).then(|| inner.to_owned())
}

/// The seconds a CSS2 time (SCXML's `delay`: a decimal number and `ms`, `s`, `m`, `h` or `d`)
/// gives.
fn css_seconds(d: &str) -> Option<f64> {
    let d = d.trim();
    let units: [(&str, f64); 5] = [
        ("ms", 0.001),
        ("s", 1.0),
        ("m", 60.0),
        ("h", 3600.0),
        ("d", 86_400.0),
    ];
    let (number, factor) = units
        .iter()
        .find_map(|(u, f)| d.strip_suffix(u).map(|n| (n, *f)))?;
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    let v: f64 = number.parse().ok()?;
    // The plain seconds are kept exactly; the other units are multiplied out.
    Some(if factor == 1.0 { v } else { v * factor })
}

// ---------------------------------------------------------------------------------------------
// Writing

/// Escapes an attribute value, with line breaks and tabs as character references so that XML's
/// attribute-value normalisation keeps them.
pub(crate) fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
    out
}

fn el(local: &str) -> Element {
    Element::new(local, Some(SCXML_NS))
}

fn st_el(local: &str) -> Element {
    Element::new(&format!("stochcraft:{local}"), Some(STOCHCRAFT_SCXML_NS))
}

fn set(e: &mut Element, name: &str, value: &str) {
    e.set_attr_escaped(name, &escape_attr(value));
}

/// Whether `s` is an XML name without a colon (an `NCName`), which SCXML's ids must be.
fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_')
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '\u{b7}'))
}

/// The states `ids` name, as SCXML's `IDREFS` (a `target` or `initial`): each an XML name
/// without a colon, separated by spaces. Refuses, with the place, one that is not.
fn references(ids: &[String], place: &str) -> Result<String, ScxmlError> {
    if let Some(bad) = ids.iter().find(|id| !is_ncname(id)) {
        return err(
            place,
            format!(
                "`{bad}` is not a reference to a state SCXML can write (an XML name: a letter or `_` first, then letters, digits, `.`, `-`, `_`)"
            ),
        );
    }
    Ok(ids.join(" "))
}

/// Whether a message to other agents (`to`) also has what only a send to the agent itself
/// can have: a target, a delay or an id. Reading and writing both refuse it.
fn goes_at_once_but_does_not(s: &SendSpec) -> bool {
    s.to.is_some() && (s.target == Target::Internal || s.delay != Delay::None || s.id.is_some())
}

/// Whether `s` is an XML name token (`NMTOKEN`), which the chart's `name` must be.
fn is_nmtoken(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '\u{b7}'))
}

/// One part of an event name: SCXML's `(\i|\d|\-)+`.
fn is_event_part(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '-'))
}

/// An event name as SCXML's schema has it: parts separated by single dots.
fn is_event_name(s: &str) -> bool {
    s.split('.').all(is_event_part)
}

/// A transition's `event`: descriptors separated by spaces, each an event name, optionally
/// ending `.*`, or `*`.
fn is_descriptors(s: &str) -> bool {
    let mut any = false;
    for d in s.split_whitespace() {
        any = true;
        let ok = d == "*" || d == ".*" || is_event_name(d.strip_suffix(".*").unwrap_or(d));
        if !ok {
            return false;
        }
    }
    any
}

/// A string as an XPath literal, for `sendidexpr`.
fn quote(s: &str) -> Option<String> {
    if !s.contains('\'') {
        Some(format!("'{s}'"))
    } else if !s.contains('"') {
        Some(format!("\"{s}\""))
    } else {
        None
    }
}

/// Writes a chart as an SCXML 1.0 document. `seconds_per_unit` is the length of the model's
/// time unit in seconds, which a fixed delay is converted by. Refuses, with the place, what
/// SCXML's schema would not accept (see the [module](self)).
pub fn write_scxml(chart: &ChartSpec, seconds_per_unit: f64) -> Result<String, ScxmlError> {
    let mut w = Writer {
        seconds: seconds_per_unit,
        ids: Vec::new(),
        send_ids: Vec::new(),
    };
    w.collect(chart)?;
    let mut root = el("scxml")
        .with_attr("xmlns", SCXML_NS)
        .with_attr("xmlns:stochcraft", STOCHCRAFT_SCXML_NS)
        .with_attr("version", "1.0")
        .with_attr("datamodel", "stochcraft");
    if !chart.name.is_empty() {
        if !is_nmtoken(&chart.name) {
            return err(
                "scxml",
                format!(
                    "the chart's name `{}` is not an XML name token (letters, digits, `.`, `-`, `_`, no spaces)",
                    chart.name
                ),
            );
        }
        set(&mut root, "name", &chart.name);
    }
    if !chart.initial.is_empty() {
        set(&mut root, "initial", &references(&chart.initial, "scxml")?);
    }
    if !chart.data.is_empty() {
        let mut dm = el("datamodel");
        for d in &chart.data {
            let mut x = el("data");
            set(&mut x, "id", &d.id);
            set(&mut x, "expr", &d.expr);
            dm.push(x);
        }
        root.push(dm);
    }
    for s in &chart.states {
        root.push(w.state(s)?);
    }
    Ok(xml::write(&Document::new(root)))
}

struct Writer {
    seconds: f64,
    /// Every id the document gives (states, variables, sends), to find two that are the same.
    ids: Vec<(String, String)>,
    send_ids: Vec<String>,
}

impl Writer {
    fn id(&mut self, id: &str, place: String) -> Result<(), ScxmlError> {
        if !is_ncname(id) {
            return err(
                &place,
                format!(
                    "`{id}` is not an XML name (a letter or `_` first, then letters, digits, `.`, `-`, `_`), as SCXML's ids must be"
                ),
            );
        }
        if let Some((_, other)) = self.ids.iter().find(|(i, _)| i == id) {
            return err(
                &place,
                format!(
                    "the id `{id}` is also {other}: SCXML gives states, variables and sends one set of ids, so each must be different"
                ),
            );
        }
        self.ids.push((id.to_owned(), place));
        Ok(())
    }

    /// Checks every id before writing, and notes the sends' ids for `<cancel>`.
    fn collect(&mut self, chart: &ChartSpec) -> Result<(), ScxmlError> {
        for d in &chart.data {
            self.id(&d.id, format!("variable `{}`", d.id))?;
        }
        fn actions<'a>(s: &'a StateSpec, out: &mut Vec<(&'a Action, String)>) {
            let place = format!("state `{}`", s.id);
            for a in s.on_entry.iter().chain(&s.on_exit).flatten() {
                out.push((a, place.clone()));
            }
            for t in &s.transitions {
                for a in &t.actions {
                    out.push((a, place.clone()));
                }
            }
            for c in &s.children {
                actions(c, out);
            }
        }
        fn states<'a>(s: &'a StateSpec, out: &mut Vec<&'a StateSpec>) {
            out.push(s);
            for c in &s.children {
                states(c, out);
            }
        }
        let mut all = Vec::new();
        for s in &chart.states {
            states(s, &mut all);
        }
        for s in &all {
            self.id(&s.id, format!("state `{}`", s.id))?;
        }
        let mut acts = Vec::new();
        for s in &chart.states {
            actions(s, &mut acts);
        }
        for (a, place) in acts {
            if let Action::SendSpec(send) = a
                && let Some(id) = &send.id
            {
                self.id(id, format!("the send `{id}` in {place}"))?;
                self.send_ids.push(id.clone());
            }
        }
        Ok(())
    }

    fn state(&mut self, s: &StateSpec) -> Result<Element, ScxmlError> {
        let place = format!("state `{}`", s.id);
        let mut e = match s.kind {
            Kind::State => el("state"),
            Kind::Parallel => el("parallel"),
            Kind::Final => el("final"),
            Kind::History(h) => {
                let mut e = el("history");
                set(&mut e, "id", &s.id);
                if h == History::Deep {
                    set(&mut e, "type", "deep");
                }
                for t in &s.transitions {
                    e.push(self.transition(t, &place)?);
                }
                return Ok(e);
            }
        };
        set(&mut e, "id", &s.id);
        if !s.initial.is_empty() {
            if s.kind != Kind::State {
                return err(&place, "only a `<state>` has an `initial` attribute");
            }
            set(&mut e, "initial", &references(&s.initial, &place)?);
        }
        // A block each, so an error in one still stops only that one when read back.
        for block in &s.on_entry {
            let mut x = el("onentry");
            for a in block {
                x.push(self.action(a, &place)?);
            }
            e.push(x);
        }
        for block in &s.on_exit {
            let mut x = el("onexit");
            for a in block {
                x.push(self.action(a, &place)?);
            }
            e.push(x);
        }
        if s.kind == Kind::Final && !s.transitions.is_empty() {
            return err(&place, "a final state has no transitions");
        }
        for t in &s.transitions {
            e.push(self.transition(t, &place)?);
        }
        for c in &s.children {
            e.push(self.state(c)?);
        }
        Ok(e)
    }

    fn transition(&mut self, t: &TransitionSpec, place: &str) -> Result<Element, ScxmlError> {
        let mut e = el("transition");
        if let Some(ev) = &t.event {
            if !is_descriptors(ev) {
                return err(
                    place,
                    format!(
                        "the event descriptor `{ev}` is not one SCXML's schema accepts (names of letters, digits, `_`, `-` and `:`, parts separated by `.`)"
                    ),
                );
            }
            set(&mut e, "event", ev);
        }
        if let Some(c) = &t.cond {
            set(&mut e, "cond", c);
        }
        if !t.targets.is_empty() {
            set(&mut e, "target", &references(&t.targets, place)?);
        }
        if t.internal {
            set(&mut e, "type", "internal");
        }
        for a in &t.actions {
            e.push(self.action(a, place)?);
        }
        Ok(e)
    }

    fn action(&mut self, a: &Action, place: &str) -> Result<Element, ScxmlError> {
        let event_name = |ev: &str| -> Result<(), ScxmlError> {
            if is_event_name(ev) {
                Ok(())
            } else {
                err(
                    place,
                    format!(
                        "the event name `{ev}` is not one SCXML's schema accepts (letters, digits, `_`, `-` and `:`, parts separated by `.`)"
                    ),
                )
            }
        };
        Ok(match a {
            Action::Assign { location, expr } => {
                let mut e = el("assign");
                set(&mut e, "location", location);
                set(&mut e, "expr", expr);
                e
            }
            Action::Raise { event } => {
                event_name(event)?;
                let mut e = el("raise");
                set(&mut e, "event", event);
                e
            }
            Action::Cancel { send_id } => {
                let mut e = el("cancel");
                if self.send_ids.contains(send_id) {
                    set(&mut e, "sendid", send_id);
                } else {
                    // An id no send of this chart gives is no IDREF: written as an expression.
                    let Some(q) = quote(send_id) else {
                        return err(
                            place,
                            format!(
                                "the send id `{send_id}` has both kinds of quote and cannot be written"
                            ),
                        );
                    };
                    set(&mut e, "sendidexpr", &q);
                }
                e
            }
            Action::Log { label, expr } => {
                let mut e = el("log");
                if !label.is_empty() {
                    set(&mut e, "label", label);
                }
                if let Some(x) = expr {
                    set(&mut e, "expr", x);
                }
                e
            }
            Action::SendSpec(s) => {
                event_name(&s.event)?;
                self.send(s, place)?
            }
            Action::Create { population } => {
                let mut e = st_el("create");
                set(&mut e, "population", population);
                e
            }
            Action::Remove { population, serial } => {
                let mut e = st_el("remove");
                set(&mut e, "population", population);
                set(&mut e, "serial", serial);
                e
            }
            Action::Connect { population, serial } => {
                let mut e = st_el("connect");
                set(&mut e, "population", population);
                set(&mut e, "serial", serial);
                e
            }
            Action::Disconnect { population, serial } => {
                let mut e = st_el("disconnect");
                set(&mut e, "population", population);
                set(&mut e, "serial", serial);
                e
            }
            Action::MoveTo {
                population,
                row,
                column,
            } => {
                let mut e = st_el("moveTo");
                set(&mut e, "population", population);
                set(&mut e, "row", row);
                set(&mut e, "column", column);
                e
            }
            Action::MoveRandom { population } => {
                let mut e = st_el("moveRandom");
                set(&mut e, "population", population);
                e
            }
        })
    }

    fn send(&mut self, s: &SendSpec, place: &str) -> Result<Element, ScxmlError> {
        if goes_at_once_but_does_not(s) {
            return err(
                place,
                "a message to other agents goes at once: no target, delay or id",
            );
        }
        let mut e = el("send");
        set(&mut e, "event", &s.event);
        if s.target == Target::Internal {
            set(&mut e, "target", "#_internal");
        }
        if s.to.is_some() {
            set(&mut e, "type", AGENTS_PROCESSOR);
        }
        if let Some(id) = &s.id {
            set(&mut e, "id", id);
        }
        match &s.delay {
            Delay::None => {}
            Delay::Fixed(x) => {
                let secs = x * self.seconds;
                let css = secs.is_finite() && secs >= 0.0 && !secs.is_sign_negative();
                if css {
                    let text = format!("{secs}");
                    set(&mut e, "delay", &format!("{text}s"));
                    let back = text.parse::<f64>().map(|v| v / self.seconds);
                    if back.map(f64::to_bits) != Ok(x.to_bits()) {
                        set(&mut e, "stochcraft:delay", &format!("{x}"));
                    }
                } else {
                    set(&mut e, "stochcraft:delay", &format!("{x}"));
                }
            }
            Delay::Expr(x) => set(&mut e, "delayexpr", x),
            Delay::Rate(r) => {
                set(&mut e, "delayexpr", &format!("exponential(1 div ({r}))"));
                set(&mut e, "stochcraft:rate", r);
            }
        }
        if let Some(to) = &s.to {
            let (kind, serial) = match to {
                To::Agent { serial, .. } => ("agent", Some(serial)),
                To::Random { .. } => ("random", None),
                To::All { .. } => ("all", None),
                To::Linked { .. } => ("linked", None),
                To::LinkedRandom { .. } => ("linkedRandom", None),
                To::Neighbours { .. } => ("neighbours", None),
                To::NeighbourRandom { .. } => ("neighbourRandom", None),
            };
            set(&mut e, "stochcraft:to", kind);
            set(&mut e, "stochcraft:population", to.population());
            if let Some(serial) = serial {
                set(&mut e, "stochcraft:serial", serial);
            }
        }
        if let Some(d) = &s.data {
            let mut c = el("content");
            set(&mut c, "expr", d);
            e.push(c);
        }
        Ok(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_times_read_in_their_units() {
        assert_eq!(css_seconds("2s"), Some(2.0));
        assert_eq!(css_seconds("500ms"), Some(0.5));
        assert_eq!(css_seconds("1.5m"), Some(90.0));
        assert_eq!(css_seconds("2h"), Some(7200.0));
        assert_eq!(css_seconds("1d"), Some(86_400.0));
        assert_eq!(css_seconds(".5s"), Some(0.5));
        for bad in ["", "s", "-1s", "1e3s", "2", "2 min"] {
            assert_eq!(css_seconds(bad), None, "{bad}");
        }
    }

    #[test]
    fn literals_and_names() {
        assert_eq!(literal("'t'"), Some("t".to_owned()));
        assert_eq!(literal("\"a'b\""), Some("a'b".to_owned()));
        assert_eq!(literal("$x"), None);
        assert!(is_ncname("idle") && is_ncname("_a.b-c") && !is_ncname("1a") && !is_ncname("a b"));
        assert!(is_descriptors("advert wom") && is_descriptors("*") && is_descriptors("done.*"));
        assert!(!is_descriptors("a..b") && !is_descriptors(""));
        assert!(is_event_name("done.state.x") && !is_event_name("a b"));
    }
}
