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

/// Whether an `expressionLanguage` attribute names FEEL (or is absent, FEEL by default).
fn feel(e: &Element, place: &str) -> Result<(), DmnError> {
    match e.attr("expressionLanguage") {
        Some(lang) if !lang.contains("FEEL") => err(
            place,
            format!("the expression language “{lang}” is not supported (only FEEL is)"),
        ),
        _ => Ok(()),
    }
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
    for c in t.children_named(&[ns], "output") {
        table.outputs.push(Output {
            id: opt_attr(c, "id"),
            label: opt_attr(c, "label"),
            name: opt_attr(c, "name"),
            type_ref: opt_attr(c, "typeRef"),
            values: c.child(&[ns], "outputValues").and_then(|v| text_of(v, ns)),
            default: c
                .child(&[ns], "defaultOutputEntry")
                .and_then(|v| text_of(v, ns)),
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
            feel(x, &place)?;
            rule.outputs.push(text_of(x, ns).unwrap_or_default());
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

/// The children of a decision table: its inputs, outputs, annotation columns and rules, in the
/// schema's order.
fn table_children(prefix: &str, t: &Table) -> Vec<Element> {
    let mut out = Vec::new();
    for input in &t.inputs {
        let mut c = el(prefix, "input");
        set_opt(&mut c, "id", input.id.as_deref());
        set_opt(&mut c, "label", input.label.as_deref());
        let mut x = texted(prefix, "inputExpression", &input.expression);
        set_opt(&mut x, "typeRef", input.type_ref.as_deref());
        c.push(x);
        if let Some(v) = &input.values {
            c.push(texted(prefix, "inputValues", v));
        }
        out.push(c);
    }
    for output in &t.outputs {
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
        out.push(c);
    }
    for a in &t.annotations {
        out.push(el(prefix, "annotation").with_attr("name", a));
    }
    for rule in &t.rules {
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
        out.push(c);
    }
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
    let mut e = el(prefix, "decision").with_attr("id", &d.id);
    if !d.name.is_empty() {
        e.set_attr("name", &d.name);
    }
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
    let mut root = el("", "definitions").with_attr("xmlns", DMN_NS).with_attr(
        "id",
        if defs.id.is_empty() {
            "definitions"
        } else {
            defs.id.as_str()
        },
    );
    if !defs.name.is_empty() {
        root.set_attr("name", &defs.name);
    }
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

/// Puts line breaks and indentation between the children of `e` (built in memory, inside parsed
/// content that keeps its own whitespace), `indent` being the indentation of `e` itself.
fn lay_out(e: &mut Element, indent: &str) {
    if !e.children.iter().any(|n| matches!(n, Node::Element(_))) {
        return;
    }
    let inner = format!("{indent}  ");
    let mut out = Vec::new();
    for n in std::mem::take(&mut e.children) {
        if let Node::Element(mut c) = n {
            lay_out(&mut c, &inner);
            out.push(Node::Text(format!("\n{inner}")));
            out.push(Node::Element(c));
        } else {
            out.push(n);
        }
    }
    out.push(Node::Text(format!("\n{indent}")));
    e.children = out;
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
        // Its description and extension elements stay; the columns and rules are written anew.
        let kept: Vec<Node> = std::mem::take(&mut table.children)
            .into_iter()
            .filter(|n| match n {
                Node::Element(e) => {
                    e.ns.as_deref() != Some(ns.as_str())
                        || matches!(e.local(), "description" | "extensionElements")
                }
                _ => false,
            })
            .collect();
        table.children = kept;
        for c in table_children(&prefix, &new.table) {
            table.push(c);
        }
        lay_out(table, &indent);
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
}
