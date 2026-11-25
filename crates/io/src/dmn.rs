//! Decision tables in DMN files (OMG DMN 1.6, decision D30): [`read_dmn`] reads a `.dmn`
//! file's decisions into [`Definitions`], [`write_dmn`] writes new ones, and [`save_dmn`] writes
//! edited tables back into the file they came from.
//!
//! A model names a decision with the `stochcraft:decision` vendor extension
//! ([`crate::bpsim::DECISION`]) in an element's BPSim parameters, the file by a reference
//! relative to the model's file. What StochCraft evaluates is in [`stochcraft_model::dmn`].
//!
//! Reading takes DMN 1.1 to 1.6 (the namespaces of [`DMN_NAMESPACES`]) and refuses, naming it,
//! what a decision table run cannot honour: a decision whose logic is not a decision table (a
//! literal expression, a context, an invocation and so on), a decision that requires another
//! decision (a decision requirement graph) or a business knowledge model, business knowledge
//! models and decision services themselves, imports, and an expression language other than
//! FEEL. Each table is checked as it is read ([`stochcraft_model::dmn::Decision::compile`]), so
//! a file that opens is one whose tables evaluate. Input data with the information requirements
//! that name it, item definitions, text annotations, knowledge sources and the diagram (DMNDI)
//! are kept in the file and not used.
//!
//! Writing follows the DMN 1.6 schema's order. Saving an edited file changes only the decision
//! tables that changed, each written again in place inside its decision (and a decision's name,
//! if that changed); everything else is kept byte for byte.

use stochcraft_model::dmn::{
    Aggregation, DMN_NAMESPACES, DMN_NS, Decision, Definitions, DmnError, HitPolicy, Input, Output,
    Rule, Table,
};

use crate::xml::{self, Document, Element, Node};

fn err<T>(place: impl Into<String>, message: impl Into<String>) -> Result<T, DmnError> {
    Err(DmnError {
        place: place.into(),
        message: message.into(),
    })
}

/// The URIs by which DMN 1.1 to 1.6 name FEEL (each version's schema default for
/// `expressionLanguage`).
const FEEL_URIS: &[&str] = &[
    "https://www.omg.org/spec/DMN/20240513/FEEL/",
    "https://www.omg.org/spec/DMN/20230324/FEEL/",
    "https://www.omg.org/spec/DMN/20211108/FEEL/",
    "https://www.omg.org/spec/DMN/20191111/FEEL/",
    "http://www.omg.org/spec/DMN/20180521/FEEL/",
    "http://www.omg.org/spec/FEEL/20140401",
];

/// Whether `lang` names FEEL: one of [`FEEL_URIS`], whatever its scheme (`http` or `https`),
/// case or final `/`, or the bare name `feel` some tools write.
fn is_feel(lang: &str) -> bool {
    let bare = |s: &str| -> String {
        let s = s.trim().to_ascii_lowercase();
        s.strip_prefix("https://")
            .or_else(|| s.strip_prefix("http://"))
            .unwrap_or(&s)
            .trim_end_matches('/')
            .to_owned()
    };
    lang.trim().eq_ignore_ascii_case("feel") || FEEL_URIS.iter().any(|u| bare(u) == bare(lang))
}

/// Whether an `expressionLanguage` attribute names FEEL (or is absent, FEEL by default).
fn feel(e: &Element, place: &str) -> Result<(), DmnError> {
    match e.attr("expressionLanguage") {
        Some(lang) if !is_feel(&lang) => err(
            place,
            format!("the expression language “{lang}” is not supported (only FEEL is)"),
        ),
        _ => Ok(()),
    }
}

/// The text of a literal expression (an output entry or a default output entry): `None` without
/// one. Its language must be FEEL, and values imported from elsewhere are refused.
fn literal_text(e: &Element, ns: &str, place: &str) -> Result<Option<String>, DmnError> {
    feel(e, place)?;
    if e.child(&[ns], "importedValues").is_some() {
        return err(
            place,
            "imported values are not supported yet: write the value as text",
        );
    }
    Ok(text_of(e, ns))
}

/// The children of a decision that are not its logic.
const DECISION_PARTS: &[&str] = &[
    "description",
    "extensionElements",
    "question",
    "allowedAnswers",
    "variable",
    "informationRequirement",
    "knowledgeRequirement",
    "authorityRequirement",
    "supportedObjective",
    "impactedPerformanceIndicator",
    "decisionMaker",
    "decisionOwner",
    "usingProcess",
    "usingTask",
];

/// Top-level elements kept in the file and not used.
const KEPT: &[&str] = &[
    "description",
    "extensionElements",
    "itemDefinition",
    "inputData",
    "knowledgeSource",
    "textAnnotation",
    "association",
    "group",
    "elementCollection",
    "performanceIndicator",
    "organizationUnit",
];

/// Reads the decisions of a `.dmn` file.
pub fn read_dmn(bytes: &[u8]) -> Result<Definitions, DmnError> {
    let doc = xml::parse(bytes).map_err(|e| DmnError::new(format!("not well-formed XML: {e}")))?;
    let root = &doc.root;
    let Some(ns) = root
        .ns
        .as_deref()
        .filter(|n| DMN_NAMESPACES.contains(n) && root.local() == "definitions")
    else {
        return err(
            "",
            "not a DMN file: its root is not a definitions element of DMN 1.1 to 1.6",
        );
    };
    feel(root, "the definitions")?;
    let mut out = Definitions {
        id: root.attr("id").unwrap_or_default(),
        name: root.attr("name").unwrap_or_default(),
        namespace: root.attr("namespace").unwrap_or_default(),
        decisions: Vec::new(),
    };
    for e in root.elements() {
        if e.ns.as_deref() != Some(ns) {
            continue;
        }
        let what = e.local();
        let named = e.attr("name").or_else(|| e.attr("id")).unwrap_or_default();
        match what {
            "decision" => out.decisions.push(read_decision(e, ns)?),
            "import" => return err("", "imports of other models are not supported yet"),
            "businessKnowledgeModel" => {
                return err(
                    format!("“{named}”"),
                    "business knowledge models are not supported yet",
                );
            }
            "decisionService" => {
                return err(
                    format!("“{named}”"),
                    "decision services are not supported yet",
                );
            }
            _ if KEPT.contains(&what) => {}
            other => {
                return err("", format!("the element {other} is not supported yet"));
            }
        }
    }
    let mut ids = std::collections::BTreeSet::new();
    for d in &out.decisions {
        if !ids.insert(d.id.as_str()) {
            return err("", format!("two decisions have the id “{}”", d.id));
        }
    }
    Ok(out)
}

fn text_of(e: &Element, ns: &str) -> Option<String> {
    e.child(&[ns], "text").map(Element::text)
}

fn opt_attr(e: &Element, name: &str) -> Option<String> {
    e.attr(name).filter(|v| !v.is_empty())
}

fn read_decision(e: &Element, ns: &str) -> Result<Decision, DmnError> {
    let Some(id) = opt_attr(e, "id") else {
        return err("a decision", "it has no id, by which a model names it");
    };
    let name = e.attr("name").unwrap_or_default();
    let place = format!("decision “{}”", if name.is_empty() { &id } else { &name });
    for r in e.children_named(&[ns], "informationRequirement") {
        if r.child(&[ns], "requiredDecision").is_some() {
            return err(
                place,
                "it requires another decision: decision requirement graphs are not supported yet",
            );
        }
    }
    if e.child(&[ns], "knowledgeRequirement").is_some() {
        return err(
            place,
            "it requires a business knowledge model, which is not supported yet",
        );
    }
    let variable = e.child(&[ns], "variable").and_then(|v| opt_attr(v, "name"));
    let logic: Vec<&Element> = e
        .elements()
        .filter(|c| c.ns.as_deref() == Some(ns) && !DECISION_PARTS.contains(&c.local()))
        .collect();
    let table = match logic.as_slice() {
        [t] if t.local() == "decisionTable" => read_table(t, ns).map_err(|x| prefix(x, &place))?,
        [other] => {
            return err(
                place,
                format!(
                    "its logic is a {}: only decision tables are supported yet",
                    other.local()
                ),
            );
        }
        [] => return err(place, "it has no decision table"),
        _ => return err(place, "it has more than one decision logic"),
    };
    let d = Decision {
        id,
        name,
        variable,
        table,
    };
    d.compile().map_err(|x| prefix(x, &place))?;
    Ok(d)
}

fn prefix(mut e: DmnError, place: &str) -> DmnError {
    e.place = if e.place.is_empty() {
        place.to_owned()
    } else {
        format!("{place}, {}", e.place)
    };
    e
}

fn read_table(t: &Element, ns: &str) -> Result<Table, DmnError> {
    feel(t, "")?;
    let hit_policy = match t.attr("hitPolicy") {
        Some(h) => HitPolicy::from_name(&h)?,
        None => HitPolicy::Unique,
    };
    let aggregation = match t.attr("aggregation") {
        Some(a) => Some(Aggregation::from_name(&a).ok_or_else(|| {
            DmnError::new(format!(
                "“{a}” is not a DMN aggregation (SUM, COUNT, MIN or MAX)"
            ))
        })?),
        None => None,
    };
    let mut table = Table {
        id: opt_attr(t, "id"),
        hit_policy,
        aggregation,
        ..Table::default()
    };
    for (i, c) in t.children_named(&[ns], "input").enumerate() {
        let place = format!("input {}", i + 1);
        let Some(x) = c.child(&[ns], "inputExpression") else {
            return err(place, "it has no input expression");
        };
        feel(x, &place)?;
        let values = match c.child(&[ns], "inputValues") {
            Some(v) => {
                feel(v, &place)?;
                text_of(v, ns)
            }
            None => None,
        };
        table.inputs.push(Input {
            id: opt_attr(c, "id"),
            label: opt_attr(c, "label"),
            expression: text_of(x, ns).unwrap_or_default(),
            type_ref: opt_attr(x, "typeRef"),
            values,
        });
    }
    for (j, c) in t.children_named(&[ns], "output").enumerate() {
        let place = format!("output {}", j + 1);
        let values = match c.child(&[ns], "outputValues") {
            Some(v) => {
                feel(v, &place)?;
                text_of(v, ns)
            }
            None => None,
        };
        let default = match c.child(&[ns], "defaultOutputEntry") {
            Some(v) => literal_text(v, ns, &format!("{place} default"))?,
            None => None,
        };
        table.outputs.push(Output {
            id: opt_attr(c, "id"),
            label: opt_attr(c, "label"),
            name: opt_attr(c, "name"),
            type_ref: opt_attr(c, "typeRef"),
            values,
            default,
        });
    }
    for c in t.children_named(&[ns], "annotation") {
        table.annotations.push(c.attr("name").unwrap_or_default());
    }
    for (r, c) in t.children_named(&[ns], "rule").enumerate() {
        let place = format!("rule {}", r + 1);
        let mut rule = Rule {
            id: opt_attr(c, "id"),
            description: c
                .child(&[ns], "description")
                .map(Element::text)
                .filter(|d| !d.is_empty()),
            ..Rule::default()
        };
        for x in c.children_named(&[ns], "inputEntry") {
            feel(x, &place)?;
            rule.inputs.push(text_of(x, ns).unwrap_or_default());
        }
        for x in c.children_named(&[ns], "outputEntry") {
            rule.outputs
                .push(literal_text(x, ns, &place)?.unwrap_or_default());
        }
        for x in c.children_named(&[ns], "annotationEntry") {
            rule.annotations.push(text_of(x, ns).unwrap_or_default());
        }
        table.rules.push(rule);
    }
    for other in t.elements() {
        let known = [
            "description",
            "extensionElements",
            "input",
            "output",
            "annotation",
            "rule",
        ];
        if other.ns.as_deref() == Some(ns) && !known.contains(&other.local()) {
            return err(
                "",
                format!("the element {} is not supported yet", other.local()),
            );
        }
    }
    Ok(table)
}

/// The element `local` in the DMN namespace, named with `prefix` (`""` or `dmn:`).
fn el(prefix: &str, local: &str) -> Element {
    Element::new(&format!("{prefix}{local}"), Some(DMN_NS))
}

/// An element holding one `text` child.
fn texted(prefix: &str, local: &str, text: &str) -> Element {
    el(prefix, local).with_child(el(prefix, "text").with_text(text))
}

fn set_opt(e: &mut Element, name: &str, value: Option<&str>) {
    match value {
        Some(v) => e.set_attr(name, v),
        None => e.remove_attr(name),
    }
}

fn input_element(prefix: &str, input: &Input) -> Element {
    let mut c = el(prefix, "input");
    set_opt(&mut c, "id", input.id.as_deref());
    set_opt(&mut c, "label", input.label.as_deref());
    let mut x = texted(prefix, "inputExpression", &input.expression);
    set_opt(&mut x, "typeRef", input.type_ref.as_deref());
    c.push(x);
    if let Some(v) = &input.values {
        c.push(texted(prefix, "inputValues", v));
    }
    c
}

fn output_element(prefix: &str, output: &Output) -> Element {
    let mut c = el(prefix, "output");
    set_opt(&mut c, "id", output.id.as_deref());
    set_opt(&mut c, "label", output.label.as_deref());
    set_opt(&mut c, "name", output.name.as_deref());
    set_opt(&mut c, "typeRef", output.type_ref.as_deref());
    if let Some(v) = &output.values {
        c.push(texted(prefix, "outputValues", v));
    }
    if let Some(v) = &output.default {
        c.push(texted(prefix, "defaultOutputEntry", v));
    }
    c
}

fn rule_element(prefix: &str, rule: &Rule) -> Element {
    let mut c = el(prefix, "rule");
    set_opt(&mut c, "id", rule.id.as_deref());
    if let Some(d) = &rule.description {
        c.push(el(prefix, "description").with_text(d));
    }
    for x in &rule.inputs {
        c.push(texted(prefix, "inputEntry", x));
    }
    for x in &rule.outputs {
        c.push(texted(prefix, "outputEntry", x));
    }
    for x in &rule.annotations {
        c.push(texted(prefix, "annotationEntry", x));
    }
    c
}

/// The children of a decision table: its inputs, outputs, annotation columns and rules, in the
/// schema's order.
fn table_children(prefix: &str, t: &Table) -> Vec<Element> {
    let mut out = Vec::new();
    out.extend(t.inputs.iter().map(|c| input_element(prefix, c)));
    out.extend(t.outputs.iter().map(|c| output_element(prefix, c)));
    out.extend(
        t.annotations
            .iter()
            .map(|a| el(prefix, "annotation").with_attr("name", a)),
    );
    out.extend(t.rules.iter().map(|r| rule_element(prefix, r)));
    out
}

/// Sets a decision table element's attributes to `t`'s.
fn table_attrs(e: &mut Element, t: &Table) {
    set_opt(e, "id", t.id.as_deref());
    match t.hit_policy {
        // UNIQUE is the default; a file that wrote it keeps it.
        HitPolicy::Unique if e.attr("hitPolicy").is_none() => {}
        p => e.set_attr("hitPolicy", p.name()),
    }
    set_opt(e, "aggregation", t.aggregation.map(Aggregation::name));
}

fn decision_element(prefix: &str, d: &Decision) -> Element {
    // The schema requires a name, even an empty one.
    let mut e = el(prefix, "decision")
        .with_attr("id", &d.id)
        .with_attr("name", &d.name);
    if let Some(v) = &d.variable {
        e.push(el(prefix, "variable").with_attr("name", v));
    }
    let mut t = el(prefix, "decisionTable");
    table_attrs(&mut t, &d.table);
    for c in table_children(prefix, &d.table) {
        t.push(c);
    }
    e.push(t);
    e
}

/// A new DMN 1.6 file with `defs`'s decisions.
pub fn write_dmn(defs: &Definitions) -> String {
    let mut root = el("", "definitions")
        .with_attr("xmlns", DMN_NS)
        .with_attr(
            "id",
            if defs.id.is_empty() {
                "definitions"
            } else {
                defs.id.as_str()
            },
        )
        // The schema requires a name, even an empty one.
        .with_attr("name", &defs.name);
    let namespace = if defs.namespace.is_empty() {
        "https://helenkwok.github.io/StochCraft/dmn"
    } else {
        defs.namespace.as_str()
    };
    root.set_attr("namespace", namespace);
    for d in &defs.decisions {
        root.push(decision_element("", d));
    }
    xml::write(&Document::new(root))
}

/// Puts line breaks and indentation between the children of `e`, an element StochCraft built
/// (inside parsed content that keeps its own whitespace), and so on down: `indent` is the
/// indentation of `e` itself.
fn lay_out(e: &mut Element, indent: &str) {
    if !e.children.iter().any(|n| matches!(n, Node::Element(_))) {
        return;
    }
    let nodes = std::mem::take(&mut e.children);
    arrange(e, nodes, indent);
}

/// Makes `nodes` the children of `e` (whose own indentation is `indent`), each on a line of its
/// own one step further in, the whitespace that was between them dropped. Those StochCraft built
/// are laid out in turn ([`lay_out`]); a parsed one is kept as it was, whatever it holds.
fn arrange(e: &mut Element, nodes: Vec<Node>, indent: &str) {
    let inner = format!("{indent}  ");
    let mut out = Vec::new();
    for n in nodes {
        match n {
            Node::Text(t) if t.trim().is_empty() => {}
            Node::Element(mut c) => {
                if c.is_new() {
                    lay_out(&mut c, &inner);
                }
                out.push(Node::Text(format!("\n{inner}")));
                out.push(Node::Element(c));
            }
            other => {
                out.push(Node::Text(format!("\n{inner}")));
                out.push(other);
            }
        }
    }
    out.push(Node::Text(format!("\n{indent}")));
    e.children = out;
}

/// The indentation of the line `e`'s end tag is on, if its content ends with a line break.
fn closing_indent(e: &Element) -> Option<String> {
    match e.children.last() {
        Some(Node::Text(t)) if t.contains('\n') && t.trim().is_empty() => {
            t.rsplit('\n').next().map(str::to_owned)
        }
        _ => None,
    }
}

/// How `new`'s items continue `old`'s ([`pair`]).
struct Pairs {
    /// For each of `new`'s items, the one of `old`'s it continues, if any.
    from: Vec<Option<usize>>,
    /// `old`'s items without an id whose place in `new` is not known: items without ids were
    /// both added (or removed) and edited, so none of these is taken to continue as any
    /// particular one of `new`'s, and none is kept.
    unknown: Vec<usize>,
}

/// For each of `new`'s items, the one of `old`'s it continues, if any: the one with the same id;
/// for an item without an id, one without an id that is equal to it. The items without ids left
/// over are taken in order, as edited in place, only when as many are left on each side; when
/// some were added or removed as well, which continues which is not known (DMN gives an element
/// no identity but its optional id), so none is paired and the old ones left are `unknown`. Each
/// of `old`'s is taken once.
fn pair<T: PartialEq>(old: &[T], new: &[T], id: impl Fn(&T) -> Option<&str>) -> Pairs {
    let mut used = vec![false; old.len()];
    let mut from: Vec<Option<usize>> = vec![None; new.len()];
    // Two passes: by id, then equal items without ids.
    for pass in 0..2 {
        for (j, n) in new.iter().enumerate() {
            if from[j].is_some() || (pass == 0) != id(n).is_some() {
                continue;
            }
            let fits = |i: usize, o: &T| {
                !used[i]
                    && if pass == 0 {
                        id(o) == id(n)
                    } else {
                        id(o).is_none() && o == n
                    }
            };
            if let Some(i) = old.iter().enumerate().position(|(i, o)| fits(i, o)) {
                used[i] = true;
                from[j] = Some(i);
            }
        }
    }
    let old_left: Vec<usize> = (0..old.len())
        .filter(|&i| !used[i] && old.get(i).is_some_and(|o| id(o).is_none()))
        .collect();
    let new_left: Vec<usize> = (0..new.len())
        .filter(|&j| from[j].is_none() && new.get(j).is_some_and(|n| id(n).is_none()))
        .collect();
    let mut unknown = Vec::new();
    if old_left.len() == new_left.len() {
        for (i, j) in old_left.into_iter().zip(new_left) {
            from[j] = Some(i);
        }
    } else if !new_left.is_empty() {
        unknown = old_left;
    }
    Pairs { from, unknown }
}

/// Whether `e`, an element of a decision table (a column, a rule or a rule's cell), holds
/// something the model does not keep of it: attributes the model does not hold (of `e` itself,
/// those other than `own`; inside it, any but an input expression's type, so its cells' ids
/// count), elements of other namespaces, extension elements, comments or processing
/// instructions. Rebuilt from the model, it would lose them.
fn holds_more(e: &Element, ns: &str, own: &[&str]) -> bool {
    let declares = |name: &str| name == "xmlns" || name.starts_with("xmlns:");
    let extra = |x: &Element, held: &[&str]| {
        x.attrs
            .iter()
            .any(|(name, _)| !declares(name) && !held.contains(&name.as_str()))
    };
    let odd = |x: &Element| {
        x.children
            .iter()
            .any(|n| matches!(n, Node::Comment(_) | Node::Pi(_)))
    };
    if extra(e, own) || odd(e) {
        return true;
    }
    e.descendants().into_iter().any(|x| {
        let held: &[&str] = if x.is(ns, "inputExpression") {
            &["typeRef"]
        } else {
            &[]
        };
        x.ns.as_deref() != Some(ns) || x.local() == "extensionElements" || extra(x, held) || odd(x)
    })
}

/// Whether `pairs` keeps all of `old` items, each where it was.
fn in_place(pairs: &Pairs, old: usize) -> bool {
    pairs.from.len() == old && pairs.from.iter().enumerate().all(|(j, p)| *p == Some(j))
}

/// Sets attribute `name` to `now` where it differs from `was` (so an attribute that is not
/// edited keeps its exact spelling).
fn set_changed(e: &mut Element, name: &str, was: Option<&str>, now: Option<&str>) {
    if was != now {
        set_opt(e, name, now);
    }
}

/// Where a child of an element was, among the others, when [`Patch::split`] took it out.
enum Slot {
    Node(Node),
    Part(usize, usize),
}

/// Writes an edited table into the parsed `decisionTable` element it was read from. Every
/// column, rule and cell element that is still there is kept, with what the model does not hold
/// of it (the id of a cell, extension elements, attributes of other namespaces, comments): only
/// what was edited is written. An element that keeps its children in their order keeps its
/// whitespace too; one whose children change is laid out again, one level only.
struct Patch<'a> {
    ns: &'a str,
    prefix: &'a str,
}

impl Patch<'_> {
    /// Takes `e`'s children out: the elements of `kinds` (in the DMN namespace), each kind's in
    /// order, and where each was among the rest.
    fn split(&self, e: &mut Element, kinds: &[&str]) -> (Vec<Vec<Option<Element>>>, Vec<Slot>) {
        let mut parts: Vec<Vec<Option<Element>>> = kinds.iter().map(|_| Vec::new()).collect();
        let mut slots = Vec::new();
        for n in std::mem::take(&mut e.children) {
            let kind = match &n {
                Node::Element(c) => kinds.iter().position(|k| c.is(self.ns, k)),
                _ => None,
            };
            match (kind, n) {
                (Some(k), Node::Element(c)) => {
                    if let Some(list) = parts.get_mut(k) {
                        slots.push(Slot::Part(k, list.len()));
                        list.push(Some(c));
                    }
                }
                (_, n) => slots.push(Slot::Node(n)),
            }
        }
        (parts, slots)
    }

    /// Puts `e`'s children back: `built`, per kind, for what [`Patch::split`] took out. If `same`
    /// (the same elements, in the same order) each goes where its old one was; otherwise the
    /// first `lead` kinds, then the rest, then the other kinds, all laid out again ([`arrange`]).
    fn put(
        e: &mut Element,
        slots: Vec<Slot>,
        built: Vec<Vec<Element>>,
        same: bool,
        lead: usize,
        indent: &str,
    ) {
        if same {
            let mut built: Vec<Vec<Option<Element>>> = built
                .into_iter()
                .map(|v| v.into_iter().map(Some).collect())
                .collect();
            e.children = slots
                .into_iter()
                .filter_map(|s| match s {
                    Slot::Node(n) => Some(n),
                    Slot::Part(k, i) => built.get_mut(k)?.get_mut(i)?.take().map(Node::Element),
                })
                .collect();
            return;
        }
        let rest = slots.into_iter().filter_map(|s| match s {
            Slot::Node(n) => Some(n),
            Slot::Part(..) => None,
        });
        let mut built = built.into_iter();
        let first: Vec<Node> = built
            .by_ref()
            .take(lead)
            .flatten()
            .map(Node::Element)
            .collect();
        let nodes = first
            .into_iter()
            .chain(rest)
            .chain(built.flatten().map(Node::Element))
            .collect();
        arrange(e, nodes, indent);
    }

    /// Sets the text of cell `c` (its `text` child's).
    fn set_text(&self, c: &mut Element, text: &str) {
        match c.child_mut(&[self.ns], "text") {
            Some(t) => t.children = vec![Node::Text(xml::escape(text))],
            None => c.push(el(self.prefix, "text").with_text(text)),
        }
    }

    /// An optional texted child (`local`), found or not, as it is to be (`now`, from `was`); and
    /// whether it stays as it was there.
    fn optional(
        &self,
        found: Option<Element>,
        local: &str,
        was: Option<&str>,
        now: Option<&str>,
    ) -> (Vec<Element>, bool) {
        match (found, now) {
            (Some(mut c), Some(v)) => {
                if was != Some(v) {
                    self.set_text(&mut c, v);
                }
                (vec![c], true)
            }
            (Some(_), None) => (Vec::new(), false),
            (None, Some(v)) => (vec![texted(self.prefix, local, v)], false),
            (None, None) => (Vec::new(), true),
        }
    }

    fn table(
        &self,
        table: &mut Element,
        old: &Table,
        new: &Table,
        indent: &str,
    ) -> Result<(), DmnError> {
        let inner = format!("{indent}  ");
        let inputs = pair(&old.inputs, &new.inputs, |c| c.id.as_deref());
        let outputs = pair(&old.outputs, &new.outputs, |c| c.id.as_deref());
        let annotations = pair(&old.annotations, &new.annotations, |_| None);
        let rules = pair(&old.rules, &new.rules, |r| r.id.as_deref());
        let (mut parts, slots) = self.split(table, &["input", "output", "annotation", "rule"]);
        // An item without an id whose place in the edited table is not known is not kept; that
        // is refused only if it holds what the model does not, which would otherwise be lost or
        // given to another item.
        let kinds = [
            ("input", "inputs", &inputs),
            ("output", "outputs", &outputs),
            ("annotation", "annotation columns", &annotations),
            ("rule", "rules", &rules),
        ];
        // What the model holds of a column or rule itself: a rule's id, a column's id, label,
        // name and type.
        let own = ["id", "label", "name", "typeRef"];
        let entries = ["inputEntry", "outputEntry", "annotationEntry"];
        for (k, (kind, plural, p)) in kinds.iter().enumerate() {
            let held = p.unknown.iter().find(|&&i| {
                let item = parts.get(k).and_then(|v| v.get(i)).and_then(Option::as_ref);
                // A column's cells, in every rule, go with it.
                let cells = entries.get(k).is_some_and(|entry| {
                    parts.get(3).into_iter().flatten().flatten().any(|r| {
                        r.elements()
                            .filter(|c| c.is(self.ns, entry))
                            .nth(i)
                            .is_some_and(|c| holds_more(c, self.ns, &[]))
                    })
                });
                cells || item.is_some_and(|e| holds_more(e, self.ns, &own))
            });
            if let Some(i) = held {
                return err(
                    format!("{kind} {}", i + 1),
                    format!(
                        "{plural} without ids were added or removed and others edited in the same change, so which edited one continues this {kind} is not known, and saving would lose what the file keeps of it (cell ids, extensions, comments); give the {plural} ids, or save the edits and the added or removed {plural} separately"
                    ),
                );
            }
        }
        let mut kept = |k: usize, from: &Option<usize>| -> Option<(usize, Element)> {
            let i = (*from)?;
            Some((i, parts.get_mut(k)?.get_mut(i)?.take()?))
        };
        let built_inputs: Vec<Element> = new
            .inputs
            .iter()
            .zip(&inputs.from)
            .map(
                |(c, from)| match (kept(0, from), from.and_then(|i| old.inputs.get(i))) {
                    (Some((_, mut e)), Some(was)) => {
                        self.input(&mut e, was, c, &inner);
                        e
                    }
                    _ => input_element(self.prefix, c),
                },
            )
            .collect();
        let built_outputs: Vec<Element> = new
            .outputs
            .iter()
            .zip(&outputs.from)
            .map(
                |(c, from)| match (kept(1, from), from.and_then(|i| old.outputs.get(i))) {
                    (Some((_, mut e)), Some(was)) => {
                        self.output(&mut e, was, c, &inner);
                        e
                    }
                    _ => output_element(self.prefix, c),
                },
            )
            .collect();
        let built_annotations: Vec<Element> = new
            .annotations
            .iter()
            .zip(&annotations.from)
            .map(|(a, from)| match kept(2, from) {
                Some((i, mut e)) => {
                    set_changed(
                        &mut e,
                        "name",
                        old.annotations.get(i).map(String::as_str),
                        Some(a.as_str()),
                    );
                    e
                }
                None => el(self.prefix, "annotation").with_attr("name", a),
            })
            .collect();
        let columns: [&[Option<usize>]; 3] = [&inputs.from, &outputs.from, &annotations.from];
        let built_rules: Vec<Element> = new
            .rules
            .iter()
            .zip(&rules.from)
            .map(
                |(r, from)| match (kept(3, from), from.and_then(|i| old.rules.get(i))) {
                    (Some((_, mut e)), Some(was)) => {
                        self.rule(&mut e, was, r, &columns, &inner);
                        e
                    }
                    _ => rule_element(self.prefix, r),
                },
            )
            .collect();
        let same = in_place(&inputs, old.inputs.len())
            && in_place(&outputs, old.outputs.len())
            && in_place(&annotations, old.annotations.len())
            && in_place(&rules, old.rules.len());
        Self::put(
            table,
            slots,
            vec![built_inputs, built_outputs, built_annotations, built_rules],
            same,
            0,
            indent,
        );
        Ok(())
    }

    fn input(&self, e: &mut Element, old: &Input, new: &Input, fallback: &str) {
        set_changed(e, "id", old.id.as_deref(), new.id.as_deref());
        set_changed(e, "label", old.label.as_deref(), new.label.as_deref());
        let indent = closing_indent(e).unwrap_or_else(|| fallback.to_owned());
        let (mut parts, slots) = self.split(e, &["inputExpression", "inputValues"]);
        let first =
            |parts: &mut [Vec<Option<Element>>], k: usize| parts.get_mut(k)?.first_mut()?.take();
        let (mut x, x_same) = self.optional(
            first(parts.as_mut_slice(), 0),
            "inputExpression",
            Some(old.expression.as_str()),
            Some(new.expression.as_str()),
        );
        for x in &mut x {
            set_changed(
                x,
                "typeRef",
                old.type_ref.as_deref(),
                new.type_ref.as_deref(),
            );
        }
        let (values, values_same) = self.optional(
            first(parts.as_mut_slice(), 1),
            "inputValues",
            old.values.as_deref(),
            new.values.as_deref(),
        );
        Self::put(e, slots, vec![x, values], x_same && values_same, 0, &indent);
    }

    fn output(&self, e: &mut Element, old: &Output, new: &Output, fallback: &str) {
        set_changed(e, "id", old.id.as_deref(), new.id.as_deref());
        set_changed(e, "label", old.label.as_deref(), new.label.as_deref());
        set_changed(e, "name", old.name.as_deref(), new.name.as_deref());
        set_changed(
            e,
            "typeRef",
            old.type_ref.as_deref(),
            new.type_ref.as_deref(),
        );
        let indent = closing_indent(e).unwrap_or_else(|| fallback.to_owned());
        let (mut parts, slots) = self.split(e, &["outputValues", "defaultOutputEntry"]);
        let first =
            |parts: &mut [Vec<Option<Element>>], k: usize| parts.get_mut(k)?.first_mut()?.take();
        let (values, values_same) = self.optional(
            first(parts.as_mut_slice(), 0),
            "outputValues",
            old.values.as_deref(),
            new.values.as_deref(),
        );
        let (default, default_same) = self.optional(
            first(parts.as_mut_slice(), 1),
            "defaultOutputEntry",
            old.default.as_deref(),
            new.default.as_deref(),
        );
        Self::put(
            e,
            slots,
            vec![values, default],
            values_same && default_same,
            0,
            &indent,
        );
    }

    /// Rule `e`, read as `old`, written as `new`, its cells following their columns
    /// (`columns`: for the inputs, outputs and annotations, the old column each new one is).
    fn rule(
        &self,
        e: &mut Element,
        old: &Rule,
        new: &Rule,
        columns: &[&[Option<usize>]; 3],
        fallback: &str,
    ) {
        set_changed(e, "id", old.id.as_deref(), new.id.as_deref());
        let indent = closing_indent(e).unwrap_or_else(|| fallback.to_owned());
        let kinds = ["inputEntry", "outputEntry", "annotationEntry"];
        let (mut parts, slots) = self.split(e, &["description", kinds[0], kinds[1], kinds[2]]);
        let mut same = true;
        let found = parts.get_mut(0).and_then(|d| d.first_mut()?.take());
        let description = match (found, &new.description) {
            (Some(mut d), Some(text)) => {
                if old.description != new.description {
                    d.children = vec![Node::Text(xml::escape(text))];
                }
                vec![d]
            }
            (Some(_), None) => {
                same = false;
                Vec::new()
            }
            (None, Some(text)) => {
                same = false;
                vec![el(self.prefix, "description").with_text(text)]
            }
            (None, None) => Vec::new(),
        };
        let mut built = vec![description];
        let was = [&old.inputs, &old.outputs, &old.annotations];
        let now = [&new.inputs, &new.outputs, &new.annotations];
        for (k, local) in kinds.iter().enumerate() {
            let (Some(pairs), Some(was), Some(now)) = (columns.get(k), was.get(k), now.get(k))
            else {
                continue;
            };
            let cells = parts.get_mut(k + 1);
            same &= cells.as_ref().is_some_and(|c| in_place(pairs, c.len()));
            let mut cells = cells.map(std::mem::take).unwrap_or_default();
            built.push(
                now.iter()
                    .zip(pairs.iter())
                    .map(|(text, from)| {
                        let kept = from.and_then(|i| Some((i, cells.get_mut(i)?.take()?)));
                        match kept {
                            Some((i, mut c)) => {
                                if was.get(i) != Some(text) {
                                    self.set_text(&mut c, text);
                                }
                                c
                            }
                            None => texted(self.prefix, local, text),
                        }
                    })
                    .collect(),
            );
        }
        Self::put(e, slots, built, same, 1, &indent);
    }
}

/// The indentation of the line a child at `index` of `parent` starts on, from the text before it.
fn indent_before(parent: &Element, index: usize) -> String {
    match index.checked_sub(1).and_then(|i| parent.children.get(i)) {
        Some(Node::Text(t)) => t.rsplit('\n').next().unwrap_or_default().to_owned(),
        _ => String::new(),
    }
}

/// The file's bytes with `after`'s tables written in where they differ from `before`'s (what
/// was read from `original`): `original` itself when nothing changed. Decisions cannot be added
/// or removed this way yet.
pub fn save_dmn(
    original: &[u8],
    before: &Definitions,
    after: &Definitions,
) -> Result<Vec<u8>, DmnError> {
    if before == after {
        return Ok(original.to_vec());
    }
    let ids = |d: &Definitions| d.decisions.iter().map(|x| x.id.clone()).collect::<Vec<_>>();
    if ids(before) != ids(after) {
        return err(
            "",
            "decisions cannot be added to or removed from a DMN file yet",
        );
    }
    let mut doc =
        xml::parse(original).map_err(|e| DmnError::new(format!("not well-formed XML: {e}")))?;
    if doc.ascii_only() {
        return err(
            "",
            "the file is not in UTF-8, which saving an edited table needs",
        );
    }
    let Some(ns) = doc.root.ns.clone() else {
        return err("", "not a DMN file");
    };
    for (old, new) in before.decisions.iter().zip(&after.decisions) {
        if old == new {
            continue;
        }
        let Some(de) = doc
            .root
            .elements_mut()
            .find(|e| e.is(&ns, "decision") && e.attr("id").as_deref() == Some(new.id.as_str()))
        else {
            return err(format!("decision “{}”", new.id), "it is not in the file");
        };
        if old.name != new.name {
            de.set_attr("name", &new.name);
        }
        if old.table == new.table {
            continue;
        }
        let at = de
            .children
            .iter()
            .position(|n| matches!(n, Node::Element(e) if e.is(&ns, "decisionTable")));
        let Some(at) = at else {
            return err(format!("decision “{}”", new.id), "it has no decision table");
        };
        let indent = indent_before(de, at);
        let Some(Node::Element(table)) = de.children.get_mut(at) else {
            continue;
        };
        let prefix = match table.prefix() {
            "" => String::new(),
            p => format!("{p}:"),
        };
        table_attrs(table, &new.table);
        Patch {
            ns: &ns,
            prefix: &prefix,
        }
        .table(table, &old.table, &new.table, &indent)
        .map_err(|mut e| {
            e.place = format!("decision “{}”, {}", new.id, e.place);
            e
        })?;
    }
    let bytes = xml::write(&doc).into_bytes();
    // What was written must read back as what was meant.
    match read_dmn(&bytes) {
        Ok(back) if back == *after => Ok(bytes),
        Ok(_) => err("", "the edited file does not read back as edited"),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::panic)]

    use super::*;

    const FILE: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<dmn:definitions xmlns:dmn="https://www.omg.org/spec/DMN/20240513/MODEL/" xmlns:dmndi="https://www.omg.org/spec/DMN/20230324/DMNDI/" id="hub" name="Hub" namespace="https://example.org/hub">
  <!-- A comment that saving keeps. -->
  <dmn:inputData id="size" name="size"/>
  <dmn:decision id="chute" name="Chute">
    <dmn:variable name="chute"/>
    <dmn:informationRequirement id="r1"><dmn:requiredInput href="#size"/></dmn:informationRequirement>
    <dmn:decisionTable id="t1" hitPolicy="FIRST">
      <dmn:input id="i1" label="Size">
        <dmn:inputExpression typeRef="number"><dmn:text>size</dmn:text></dmn:inputExpression>
      </dmn:input>
      <dmn:output id="o1"/>
      <dmn:annotation name="Why"/>
      <dmn:rule id="r_small">
        <dmn:inputEntry><dmn:text>&lt; 10</dmn:text></dmn:inputEntry>
        <dmn:outputEntry><dmn:text>"small"</dmn:text></dmn:outputEntry>
        <dmn:annotationEntry><dmn:text>fits a tote</dmn:text></dmn:annotationEntry>
      </dmn:rule>
      <dmn:rule id="r_rest">
        <dmn:inputEntry><dmn:text>-</dmn:text></dmn:inputEntry>
        <dmn:outputEntry><dmn:text>"large"</dmn:text></dmn:outputEntry>
        <dmn:annotationEntry><dmn:text></dmn:text></dmn:annotationEntry>
      </dmn:rule>
    </dmn:decisionTable>
  </dmn:decision>
  <dmndi:DMNDI><dmndi:DMNDiagram id="d1"/></dmndi:DMNDI>
</dmn:definitions>
"##;

    fn read(text: &str) -> Definitions {
        match read_dmn(text.as_bytes()) {
            Ok(d) => d,
            Err(e) => panic!("{e}"),
        }
    }

    fn refusal(text: &str) -> String {
        match read_dmn(text.as_bytes()) {
            Ok(d) => panic!("accepted: {d:?}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn a_table_is_read_with_its_cells_as_written() {
        let d = read(FILE);
        assert_eq!((d.id.as_str(), d.name.as_str()), ("hub", "Hub"));
        let c = &d.decisions[0];
        assert_eq!(
            (c.id.as_str(), c.variable.as_deref()),
            ("chute", Some("chute"))
        );
        assert_eq!(c.output_name(0), "chute");
        let t = &c.table;
        assert_eq!(t.hit_policy, HitPolicy::First);
        assert_eq!(t.inputs[0].expression, "size");
        assert_eq!(t.inputs[0].type_ref.as_deref(), Some("number"));
        assert_eq!(t.annotations, vec!["Why".to_owned()]);
        assert_eq!(t.rules[0].inputs, vec!["< 10".to_owned()]);
        assert_eq!(t.rules[0].outputs, vec!["\"small\"".to_owned()]);
        assert_eq!(t.rules[0].annotations, vec!["fits a tote".to_owned()]);
        assert_eq!(t.rules[1].id.as_deref(), Some("r_rest"));
    }

    #[test]
    fn an_unchanged_file_saves_byte_for_byte_and_an_edited_one_reads_back_as_edited() {
        let before = read(FILE);
        assert_eq!(
            save_dmn(FILE.as_bytes(), &before, &before).ok(),
            Some(FILE.as_bytes().to_vec())
        );
        let mut after = before.clone();
        let t = &mut after.decisions[0].table;
        t.rules[0].inputs[0] = "< 12".into();
        t.rules.push(Rule {
            id: Some("r_new".into()),
            inputs: vec!["> 100".into()],
            outputs: vec!["\"pallet\"".into()],
            annotations: vec!["too big".into()],
            ..Rule::default()
        });
        t.rules.swap(1, 2);
        let bytes = match save_dmn(FILE.as_bytes(), &before, &after) {
            Ok(b) => b,
            Err(e) => panic!("{e}"),
        };
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert_eq!(read(&text), after);
        // Everything outside the table is as it was: the comment, the input data, the diagram.
        assert!(
            text.contains("<!-- A comment that saving keeps. -->"),
            "{text}"
        );
        assert!(
            text.contains(r#"<dmn:inputData id="size" name="size"/>"#),
            "{text}"
        );
        assert!(text.contains(r#"<dmndi:DMNDI><dmndi:DMNDiagram id="d1"/></dmndi:DMNDI>"#));
        assert!(text.contains("<dmn:text>&lt; 12</dmn:text>"), "{text}");
        // Laid out on lines of their own, indented below the table.
        assert!(text.contains("\n      <dmn:rule id=\"r_new\">\n"), "{text}");
        // Saving the result again, unchanged, keeps it byte for byte.
        let again = read(&text);
        assert_eq!(save_dmn(&bytes, &again, &again).ok(), Some(bytes.clone()));
    }

    #[test]
    fn a_new_file_reads_back_as_written() {
        let mut d = read(FILE);
        d.decisions[0].table.hit_policy = HitPolicy::Collect;
        d.decisions[0].table.aggregation = Some(Aggregation::Count);
        d.decisions[0].table.outputs[0].default = Some("0".into());
        d.decisions[0].table.rules[0].outputs[0] = "1".into();
        d.decisions[0].table.rules[1].outputs[0] = "2".into();
        let text = write_dmn(&d);
        assert!(text.contains(r#"xmlns="https://www.omg.org/spec/DMN/20240513/MODEL/""#));
        assert_eq!(read(&text), d);
        // Written twice, the same bytes.
        assert_eq!(write_dmn(&read(&text)), text);
    }

    #[test]
    fn older_dmn_versions_are_read() {
        let old = FILE.replace(
            "https://www.omg.org/spec/DMN/20240513/MODEL/",
            "http://www.omg.org/spec/DMN/20180521/MODEL/",
        );
        assert_eq!(read(&old), read(FILE));
    }

    #[test]
    fn what_a_table_run_cannot_honour_is_refused_by_name() {
        let literal = FILE.replace(
            r#"<dmn:decisionTable id="t1" hitPolicy="FIRST">"#,
            r#"<dmn:literalExpression><dmn:text>1</dmn:text></dmn:literalExpression><dmn:decisionTable id="t1" hitPolicy="FIRST">"#,
        );
        assert!(refusal(&literal).contains("more than one decision logic"));
        let only_literal = r#"<definitions xmlns="https://www.omg.org/spec/DMN/20240513/MODEL/" id="x" namespace="n"><decision id="a" name="A"><literalExpression><text>1</text></literalExpression></decision></definitions>"#;
        assert!(refusal(only_literal).contains("its logic is a literalExpression"));
        let graph = FILE.replace(
            r##"<dmn:requiredInput href="#size"/>"##,
            r##"<dmn:requiredDecision href="#other"/>"##,
        );
        assert!(refusal(&graph).contains("decision requirement graphs"));
        let any = FILE.replace(r#"hitPolicy="FIRST""#, r#"hitPolicy="ANY""#);
        let message = refusal(&any);
        assert!(
            message.contains("ANY") && message.contains("Chute"),
            "{message}"
        );
        let names = FILE.replace("&lt; 10", "&lt; limit");
        assert!(refusal(&names).contains("rule 1, input 1"));
        let bkm = FILE.replace(
            "<dmn:inputData id=\"size\" name=\"size\"/>",
            "<dmn:businessKnowledgeModel id=\"b\" name=\"Rates\"/>",
        );
        assert!(refusal(&bkm).contains("business knowledge models"));
        let java = FILE.replace(
            r#"<dmn:decisionTable id="t1" hitPolicy="FIRST">"#,
            r#"<dmn:decisionTable id="t1" hitPolicy="FIRST" expressionLanguage="http://www.java.com/java">"#,
        );
        assert!(refusal(&java).contains("expression language"));
        assert!(refusal("<definitions/>").contains("not a DMN file"));
    }

    #[test]
    fn feel_is_named_by_its_uris_exactly() {
        let with = |lang: &str| {
            FILE.replace(
                r#"<dmn:decisionTable id="t1" hitPolicy="FIRST">"#,
                &format!(
                    r#"<dmn:decisionTable id="t1" hitPolicy="FIRST" expressionLanguage="{lang}">"#
                ),
            )
        };
        for ok in [
            "https://www.omg.org/spec/DMN/20240513/FEEL/",
            "https://www.omg.org/spec/DMN/20191111/FEEL/",
            "http://www.omg.org/spec/DMN/20180521/FEEL/",
            "http://www.omg.org/spec/FEEL/20140401",
            "http://www.omg.org/spec/DMN/20240513/FEEL",
            "feel",
        ] {
            assert_eq!(read(&with(ok)), read(FILE), "{ok}");
        }
        for not in ["https://example.org/FEELish", "FEEL-like", "urn:other"] {
            assert!(refusal(&with(not)).contains("expression language"), "{not}");
        }
    }

    #[test]
    fn output_values_and_defaults_are_feel_text() {
        let output = |inner: &str| {
            FILE.replace(
                r#"<dmn:output id="o1"/>"#,
                &format!(r#"<dmn:output id="o1">{inner}</dmn:output>"#),
            )
        };
        let other = output(
            r#"<dmn:defaultOutputEntry expressionLanguage="urn:other"><dmn:text>"large"</dmn:text></dmn:defaultOutputEntry>"#,
        );
        assert!(
            refusal(&other).contains("output 1 default"),
            "{}",
            refusal(&other)
        );
        let values = output(
            r#"<dmn:outputValues expressionLanguage="urn:other"><dmn:text>"small","large"</dmn:text></dmn:outputValues>"#,
        );
        assert!(refusal(&values).contains("expression language"));
        let imported = output(
            r#"<dmn:defaultOutputEntry><dmn:importedValues importType="urn:x"><dmn:importedElement>x</dmn:importedElement></dmn:importedValues></dmn:defaultOutputEntry>"#,
        );
        assert!(refusal(&imported).contains("imported values"));
        // Text in FEEL is read, as before.
        let fine = output(
            r#"<dmn:defaultOutputEntry><dmn:text>"large"</dmn:text></dmn:defaultOutputEntry>"#,
        );
        assert_eq!(
            read(&fine).decisions[0].table.outputs[0].default.as_deref(),
            Some("\"large\"")
        );
    }

    #[test]
    fn names_are_written_even_when_empty() {
        let mut d = read(FILE);
        d.name.clear();
        d.decisions[0].name.clear();
        let text = write_dmn(&d);
        assert!(text.contains(r#"<definitions xmlns="https://www.omg.org/spec/DMN/20240513/MODEL/" id="hub" name="""#), "{text}");
        assert!(text.contains(r#"<decision id="chute" name="">"#), "{text}");
        assert_eq!(read(&text), d);
    }

    #[test]
    fn an_edited_cell_changes_only_its_text() {
        let before = read(FILE);
        let mut after = before.clone();
        after.decisions[0].table.rules[0].inputs[0] = "< 12".into();
        let bytes = save_dmn(FILE.as_bytes(), &before, &after).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            String::from_utf8_lossy(&bytes),
            FILE.replace("&lt; 10", "&lt; 12")
        );
    }

    #[test]
    fn what_the_model_does_not_hold_survives_an_edit() {
        let file = FILE
            .replace(
                r#"<dmn:decisionTable id="t1" hitPolicy="FIRST">"#,
                "<dmn:decisionTable id=\"t1\" hitPolicy=\"FIRST\">\n      <dmn:extensionElements><v:label xmlns:v=\"urn:v\">A<v:marker/>B</v:label></dmn:extensionElements>",
            )
            .replace(
                "<dmn:inputEntry><dmn:text>&lt; 10</dmn:text></dmn:inputEntry>",
                "<dmn:extensionElements><v:note xmlns:v=\"urn:v\">C<v:marker/>D</v:note></dmn:extensionElements>\n        <dmn:inputEntry id=\"ie1\" v:x=\"1\" xmlns:v=\"urn:v\"><dmn:text>&lt; 10</dmn:text></dmn:inputEntry>",
            )
            .replace(
                r#"<dmn:outputEntry><dmn:text>"small"</dmn:text></dmn:outputEntry>"#,
                r#"<dmn:outputEntry id="oe1"><dmn:text>"small"</dmn:text></dmn:outputEntry>"#,
            );
        let before = read(&file);
        let mut after = before.clone();
        let t = &mut after.decisions[0].table;
        t.rules[0].outputs[0] = "\"tiny\"".into();
        t.rules.insert(
            0,
            Rule {
                inputs: vec!["> 100".into()],
                outputs: vec!["\"pallet\"".into()],
                annotations: vec![String::new()],
                ..Rule::default()
            },
        );
        let bytes = save_dmn(file.as_bytes(), &before, &after).unwrap_or_else(|e| panic!("{e}"));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert_eq!(read(&text), after);
        for kept in [
            // The table's extension, its mixed content as it was.
            "<dmn:extensionElements><v:label xmlns:v=\"urn:v\">A<v:marker/>B</v:label></dmn:extensionElements>",
            // The rule's extension, and the cell's id and attribute of another namespace.
            "<dmn:extensionElements><v:note xmlns:v=\"urn:v\">C<v:marker/>D</v:note></dmn:extensionElements>",
            "<dmn:inputEntry id=\"ie1\" v:x=\"1\" xmlns:v=\"urn:v\"><dmn:text>&lt; 10</dmn:text></dmn:inputEntry>",
            // The edited cell keeps its id.
            "<dmn:outputEntry id=\"oe1\"><dmn:text>&quot;tiny&quot;</dmn:text></dmn:outputEntry>",
        ] {
            assert!(text.contains(kept), "{kept}\n{text}");
        }
        // The new rule comes first, laid out below the table.
        let new_rule = text.find("&gt; 100").unwrap_or(usize::MAX);
        assert!(new_rule < text.find("ie1").unwrap_or(0), "{text}");
        // Removing a rule keeps the one that stays as it was, its cells' ids with it.
        let mut fewer = before.clone();
        fewer.decisions[0].table.rules.remove(1);
        let bytes = save_dmn(file.as_bytes(), &before, &fewer).unwrap_or_else(|e| panic!("{e}"));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert_eq!(read(&text), fewer);
        assert!(text.contains("id=\"ie1\""), "{text}");
        assert!(!text.contains("r_rest"), "{text}");
    }

    /// Rules without ids have no identity but their place (DMN's `id` is optional): with one
    /// inserted and one edited in the same change, which edited rule continues which old one is
    /// not known, so a save that would lose a rule's cell ids or move them to another rule is
    /// refused. Edited in place, given an id, or holding nothing the model does not, it is saved.
    #[test]
    fn rules_without_ids_keep_their_cells_ids_or_the_save_is_refused() {
        let file = FILE
            .replace(r#"<dmn:rule id="r_small">"#, "<dmn:rule>")
            .replace(r#"<dmn:rule id="r_rest">"#, "<dmn:rule>")
            .replace(
                r#"<dmn:outputEntry><dmn:text>"small"</dmn:text></dmn:outputEntry>"#,
                r#"<dmn:outputEntry id="oe1"><dmn:text>"small"</dmn:text></dmn:outputEntry>"#,
            );
        let before = read(&file);
        let x = Rule {
            inputs: vec!["> 100".into()],
            outputs: vec!["\"pallet\"".into()],
            annotations: vec![String::new()],
            ..Rule::default()
        };
        // [A, B] to [X, A', B]: refused, naming the rule.
        let mut after = before.clone();
        let t = &mut after.decisions[0].table;
        t.rules[0].outputs[0] = "\"tiny\"".into();
        t.rules.insert(0, x.clone());
        let refused = save_dmn(file.as_bytes(), &before, &after);
        let message = refused
            .map(|_| String::new())
            .unwrap_or_else(|e| e.to_string());
        assert!(
            message.contains("rule 1") && message.contains("give the rules ids"),
            "{message}"
        );
        // The same with the new rule given an id: A' continues A, its cell's id with it.
        let mut named = after.clone();
        named.decisions[0].table.rules[0].id = Some("r_pallet".into());
        let bytes = save_dmn(file.as_bytes(), &before, &named).unwrap_or_else(|e| panic!("{e}"));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert_eq!(read(&text), named);
        assert!(
            text.contains(r#"<dmn:outputEntry id="oe1"><dmn:text>&quot;tiny&quot;</dmn:text></dmn:outputEntry>"#),
            "{text}"
        );
        // Edited in place: kept where it was.
        let mut edited = before.clone();
        edited.decisions[0].table.rules[0].outputs[0] = "\"tiny\"".into();
        let bytes = save_dmn(file.as_bytes(), &before, &edited).unwrap_or_else(|e| panic!("{e}"));
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.contains(r#"id="oe1""#), "{text}");
        // Holding nothing the model does not (no cell id): inserted and edited, it is rebuilt.
        let plain = file.replace(r#" id="oe1""#, "");
        let before = read(&plain);
        let mut after = before.clone();
        let t = &mut after.decisions[0].table;
        t.rules[0].outputs[0] = "\"tiny\"".into();
        t.rules.insert(0, x);
        let bytes = save_dmn(plain.as_bytes(), &before, &after).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(read(&String::from_utf8_lossy(&bytes)), after);
    }
}
