//! Saving an edited model into the file it was opened from.
//!
//! The file is changed only where the model changed: names, parameters in the scenario that was
//! opened, the experiment settings, and the elements and connections added or removed (with their
//! diagram shapes). Everything else (other scenarios, vendor extensions, gateways and other
//! elements StochCraft does not simulate, comments, formatting) is written back exactly as it
//! was read. The result is read back before it is returned; if it would not give the model that
//! was saved, saving fails with the reason instead of writing a file that means something else.

use std::collections::{BTreeMap, BTreeSet};

use stochcraft_model::{Connection, Element as ModelElement, ElementKind, Model};

use crate::bpsim::{NS2, NSS};
use crate::duration::trim_float;
use crate::read::{self, BPMN, XPDL};
use crate::write::{
    bpmn_local, condition_element, dist_element, numeric, probability_element, property_element,
    size,
};
use crate::xml::{self, Element, Node};
use crate::{Format, SaveError};

const BPMNDI: &str = "http://www.omg.org/spec/BPMN/20100524/DI";
const DC: &str = "http://www.omg.org/spec/DD/20100524/DC";
const DI: &str = "http://www.omg.org/spec/DD/20100524/DI";

fn fail(reason: impl Into<String>) -> SaveError {
    SaveError::Unsupported(reason.into())
}

/// Saves `after` into `bytes`, the file `before` was read from with scenario `scenario`.
pub fn save_edited(
    bytes: &[u8],
    format: Format,
    scenario: Option<&str>,
    before: &Model,
    after: &Model,
) -> Result<Vec<u8>, SaveError> {
    let mut doc = xml::parse(bytes).map_err(|e| fail(e.to_string()))?;
    let resources = read::resources(&doc.root, scenario);
    let mut ed = Editor {
        format,
        ids: all_ids(&doc.root),
        ascii: doc.ascii_only(),
        unit: indent_unit(&doc.root),
    };
    ed.apply(&mut doc.root, scenario, before, after, &resources)?;
    let unit = ed.unit.clone();
    tidy(&mut doc.root, "", &unit);
    let out = xml::write(&doc).into_bytes();
    // Read it back: the file must mean exactly the model that was saved.
    let back = crate::read_scenario(&out, scenario).map_err(|e| fail(e.to_string()))?;
    match difference(&back.model, after) {
        None => Ok(out),
        Some(d) => Err(fail(format!(
            "the file would not read back as the model ({d})"
        ))),
    }
}

/// What differs between two models, for an error message.
fn difference(got: &Model, want: &Model) -> Option<String> {
    if got.name != want.name {
        return Some(format!("its name reads as {:?}", got.name));
    }
    // An element's properties are assigned together, so their order means nothing; a file
    // lists them in its own order.
    let sorted = |e: &ModelElement| {
        let mut e = e.clone();
        e.properties.sort_by(|a, b| a.name.cmp(&b.name));
        e
    };
    for (g, w) in got.elements.iter().zip(&want.elements) {
        if sorted(g) != sorted(w) {
            return Some(format!("element `{}` reads as {g:?}", w.id));
        }
    }
    if got.elements.len() != want.elements.len() {
        return Some(format!(
            "it has {} elements instead of {}",
            got.elements.len(),
            want.elements.len()
        ));
    }
    // Connections are compared as a set, plus the order of each element's outgoing ones (an
    // exclusive gateway tries their conditions in that order); a file with sub-processes keeps
    // flows in several places, so their order overall means nothing.
    if let Some(w) = want
        .connections
        .iter()
        .find(|w| !got.connections.contains(w))
    {
        let g = got.connections.iter().find(|g| g.id == w.id);
        return Some(format!("connection `{}` reads as {g:?}", w.id));
    }
    if got.connections.len() != want.connections.len() {
        return Some(format!(
            "it has {} connections instead of {}",
            got.connections.len(),
            want.connections.len()
        ));
    }
    for e in &want.elements {
        let outs = |m: &Model| -> Vec<String> {
            m.connections
                .iter()
                .filter(|c| c.from == e.id)
                .map(|c| c.id.to_string())
                .collect()
        };
        if outs(got) != outs(want) {
            return Some(format!(
                "the connections out of `{}` read in the order {:?}",
                e.id,
                outs(got)
            ));
        }
    }
    if got.experiment != want.experiment {
        return Some(format!("its experiment reads as {:?}", got.experiment));
    }
    None
}

fn all_ids(root: &Element) -> BTreeSet<String> {
    std::iter::once(root)
        .chain(root.descendants())
        .flat_map(|e| [e.attr("id"), e.attr("Id")])
        .flatten()
        .collect()
}

fn is_ws(n: &Node) -> bool {
    matches!(n, Node::Text(t) if t.trim().is_empty())
}

/// The indentation step of a file: what its root's children are indented by.
fn indent_unit(root: &Element) -> String {
    root.children
        .iter()
        .find_map(|n| match n {
            Node::Text(t) if is_ws(n) && t.contains('\n') => {
                t.rsplit('\n').next().map(str::to_owned)
            }
            _ => None,
        })
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "  ".into())
}

/// Gives the elements StochCraft added the indentation of their surroundings. `indent` is the
/// whitespace that starts the line `e` is on.
fn tidy(e: &mut Element, indent: &str, unit: &str) {
    let child_indent = e
        .children
        .iter()
        .enumerate()
        .find_map(|(i, n)| match (n, e.children.get(i + 1)) {
            (Node::Text(t), Some(Node::Element(_))) if is_ws(n) && t.contains('\n') => {
                t.rsplit('\n').next().map(str::to_owned)
            }
            _ => None,
        })
        .unwrap_or_else(|| format!("{indent}{unit}"));
    let has_text = e
        .children
        .iter()
        .any(|n| matches!(n, Node::Text(_) | Node::CData(_)) && !is_ws(n));
    if has_text {
        return;
    }
    // New children are put on lines of their own where the element already lays its children
    // out that way (or is new itself, or was empty); in compact XML they stay inline.
    let multiline = e
        .children
        .iter()
        .any(|n| matches!(n, Node::Text(t) if is_ws(n) && t.contains('\n')));
    let old_children = e
        .children
        .iter()
        .any(|n| matches!(n, Node::Element(c) if !c.is_new()));
    // A new element holding one leaf stays on one line (`<Quantity><NumericParameter/></Quantity>`).
    let one_leaf = {
        let mut kids = e.elements();
        matches!((kids.next(), kids.next()), (Some(k), None) if k.elements().next().is_none())
    };
    let lay_out = (e.is_new() && !one_leaf) || multiline || (!old_children && e.was_empty_tag());
    let mut i = 0;
    while i < e.children.len() {
        let new = matches!(&e.children[i], Node::Element(c) if c.is_new());
        if new && lay_out {
            if i == 0 || !is_ws(&e.children[i - 1]) {
                e.children
                    .insert(i, Node::Text(format!("\n{child_indent}")));
                i += 1;
            }
            if matches!(e.children.get(i + 1), Some(Node::Element(_))) {
                e.children
                    .insert(i + 1, Node::Text(format!("\n{child_indent}")));
            }
        }
        i += 1;
    }
    if lay_out && matches!(e.children.last(), Some(Node::Element(c)) if c.is_new()) {
        e.children.push(Node::Text(format!("\n{indent}")));
    }
    for c in e.elements_mut() {
        tidy(c, &child_indent, unit);
    }
}

/// Finds the first element (depth first) matching `pred`.
fn find_mut<'a>(e: &'a mut Element, pred: &dyn Fn(&Element) -> bool) -> Option<&'a mut Element> {
    if pred(e) {
        return Some(e);
    }
    for c in e.elements_mut() {
        if let Some(f) = find_mut(c, pred) {
            return Some(f);
        }
    }
    None
}

fn find<'a>(e: &'a Element, pred: &dyn Fn(&Element) -> bool) -> Option<&'a Element> {
    std::iter::once(e).chain(e.descendants()).find(|x| pred(x))
}

/// Removes every element matching `pred`, with the whitespace before it, and returns how many.
fn remove_all(e: &mut Element, pred: &dyn Fn(&Element) -> bool) -> usize {
    let mut removed = 0;
    let mut i = 0;
    while i < e.children.len() {
        if matches!(&e.children[i], Node::Element(c) if pred(c)) {
            e.children.remove(i);
            removed += 1;
            if i > 0 && is_ws(&e.children[i - 1]) {
                e.children.remove(i - 1);
                i -= 1;
            }
            continue;
        }
        if let Node::Element(c) = &mut e.children[i] {
            removed += remove_all(c, pred);
        }
        i += 1;
    }
    removed
}

/// The children index after which to append (before the whitespace that ends `e`).
fn end_of(e: &Element) -> usize {
    match e.children.last() {
        Some(n) if is_ws(n) => e.children.len() - 1,
        _ => e.children.len(),
    }
}

/// The index of the first child element named one of `locals` (any namespace).
fn first_named(e: &Element, locals: &[&str]) -> Option<usize> {
    e.children
        .iter()
        .position(|n| matches!(n, Node::Element(c) if locals.contains(&c.local())))
}

/// The index just after the last child element named one of `locals`.
fn after_last_named(e: &Element, locals: &[&str]) -> Option<usize> {
    e.children
        .iter()
        .rposition(|n| matches!(n, Node::Element(c) if locals.contains(&c.local())))
        .map(|i| i + 1)
}

/// Inserts `child` among `e`'s children so that it comes after every child named in `before`
/// (schema order) and before the others, or at the end.
fn insert_ordered(e: &mut Element, child: Element, before: &[&str]) {
    let at = after_last_named(e, before)
        .or_else(|| {
            e.children
                .iter()
                .position(|n| matches!(n, Node::Element(_)))
                .filter(|_| !before.is_empty())
        })
        .unwrap_or_else(|| end_of(e));
    e.children.insert(at, Node::Element(child));
}

fn id_attr(format: Format) -> &'static str {
    match format {
        Format::Bpmn => "id",
        Format::Xpdl => "Id",
    }
}

fn has_id(e: &Element, attr: &str, id: &str) -> bool {
    e.attr(attr).as_deref() == Some(id)
}

/// A name with `prefix` (none if empty).
fn qname(prefix: &str, local: &str) -> String {
    if prefix.is_empty() {
        local.to_owned()
    } else {
        format!("{prefix}:{local}")
    }
}

/// Moves a built element (and its children) into namespace `ns` with `prefix`.
fn rename(e: &mut Element, prefix: &str, ns: &str) {
    e.name = qname(prefix, e.local());
    e.ns = Some(ns.to_owned());
    for c in e.elements_mut() {
        rename(c, prefix, ns);
    }
}

fn num(v: f64) -> String {
    trim_float(v)
}

struct Editor {
    format: Format,
    ids: BTreeSet<String>,
    ascii: bool,
    unit: String,
}

/// Where new elements of a namespace go: the prefix the file already uses for it, or a new one
/// declared on the element itself.
struct Ns {
    prefix: String,
    uri: String,
    declare: bool,
}

impl Ns {
    fn of(root: &Element, uri: &str, preferred: &str) -> Self {
        match find(root, &|e| e.ns.as_deref() == Some(uri)) {
            Some(e) => Ns {
                prefix: e.prefix().to_owned(),
                uri: uri.to_owned(),
                declare: false,
            },
            None => Ns {
                prefix: preferred.to_owned(),
                uri: uri.to_owned(),
                declare: true,
            },
        }
    }

    /// BPMN's namespace, which a BPMN file always declares on its root.
    fn bpmn(root: &Element) -> Self {
        Ns {
            declare: false,
            ..Ns::of(root, BPMN, "bpmn")
        }
    }

    fn el(&self, local: &str) -> Element {
        let mut e = Element::new(&qname(&self.prefix, local), Some(&self.uri));
        if self.declare {
            let attr = if self.prefix.is_empty() {
                "xmlns".to_owned()
            } else {
                format!("xmlns:{}", self.prefix)
            };
            e.set_attr(&attr, &self.uri);
        }
        e
    }
}

fn category(k: &ElementKind) -> u8 {
    match k {
        ElementKind::Source { .. } => 0,
        ElementKind::Sink => 1,
        ElementKind::Server { .. } | ElementKind::Delay { .. } => 2,
        ElementKind::ExclusiveGateway { .. } => 3,
        ElementKind::ParallelGateway => 4,
    }
}

/// Whether two connections join the same elements under the same id (their probability and
/// condition are parameters, saved separately).
fn same_link(a: &Connection, b: &Connection) -> bool {
    a.id == b.id && a.from == b.from && a.to == b.to
}

impl Editor {
    fn fresh(&mut self, base: &str) -> String {
        let mut id = base.to_owned();
        let mut n = 1;
        while self.ids.contains(&id) {
            n += 1;
            id = format!("{base}_{n}");
        }
        self.ids.insert(id.clone());
        id
    }

    /// Sets a text attribute, as character references where the file's encoding needs it.
    fn set_text_attr(&self, e: &mut Element, name: &str, value: &str) {
        if self.ascii {
            e.set_attr_escaped(name, &xml::escape_ascii(value));
        } else {
            e.set_attr(name, value);
        }
    }

    fn apply(
        &mut self,
        root: &mut Element,
        scenario: Option<&str>,
        before: &Model,
        after: &Model,
        resources: &read::Resources,
    ) -> Result<(), SaveError> {
        let old: BTreeMap<&str, &ModelElement> =
            before.elements.iter().map(|e| (e.id.as_str(), e)).collect();
        let new: BTreeMap<&str, &ModelElement> =
            after.elements.iter().map(|e| (e.id.as_str(), e)).collect();
        // Elements whose kind changed between event and task are replaced.
        let replaced: BTreeSet<&str> = before
            .elements
            .iter()
            .filter(|e| {
                new.get(e.id.as_str())
                    .is_none_or(|n| category(&n.kind) != category(&e.kind))
            })
            .map(|e| e.id.as_str())
            .collect();
        let added: Vec<&ModelElement> = after
            .elements
            .iter()
            .filter(|e| !old.contains_key(e.id.as_str()) || replaced.contains(e.id.as_str()))
            .collect();
        let old_conns: BTreeMap<&str, &Connection> = before
            .connections
            .iter()
            .map(|c| (c.id.as_str(), c))
            .collect();
        let new_conns: BTreeMap<&str, &Connection> = after
            .connections
            .iter()
            .map(|c| (c.id.as_str(), c))
            .collect();
        let touches =
            |c: &Connection| replaced.contains(c.from.as_str()) || replaced.contains(c.to.as_str());
        let removed_conns: BTreeSet<&str> = before
            .connections
            .iter()
            .filter(|c| {
                !new_conns
                    .get(c.id.as_str())
                    .is_some_and(|n| same_link(n, c))
                    || touches(c)
            })
            .map(|c| c.id.as_str())
            .collect();
        let added_conns: Vec<&Connection> = after
            .connections
            .iter()
            .filter(|c| {
                !old_conns
                    .get(c.id.as_str())
                    .is_some_and(|o| same_link(o, c))
                    || touches(c)
            })
            .collect();

        for id in &replaced {
            self.remove_node(root, id);
        }
        for id in &removed_conns {
            self.remove_flow(root, id);
        }
        for e in &added {
            self.add_node(root, after, e)?;
        }
        for c in &added_conns {
            self.add_flow(root, after, c)?;
        }
        if before.name != after.name {
            self.set_model_name(root, &after.name);
        }
        for e in &after.elements {
            let was = old
                .get(e.id.as_str())
                .filter(|_| !replaced.contains(e.id.as_str()));
            if was.is_some_and(|w| w.name != e.name) {
                self.set_node_name(root, e.id.as_str(), &e.name);
            }
            // An exclusive gateway's default connection (new gateways included).
            if let ElementKind::ExclusiveGateway { default } = &e.kind {
                let old_default = match was.map(|w| &w.kind) {
                    Some(ElementKind::ExclusiveGateway { default }) => default.clone(),
                    _ => None,
                };
                if &old_default != default {
                    self.set_default(root, e.id.as_str(), default.as_ref().map(|d| d.as_str()))?;
                }
            }
        }
        self.parameters(root, scenario, &old, &replaced, after, resources)?;
        self.routing(root, scenario, before, after)?;
        if before.experiment != after.experiment {
            self.experiment(root, scenario, before, after)?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Structure

    fn remove_node(&mut self, root: &mut Element, id: &str) {
        let attr = id_attr(self.format);
        // The flows to and from the node go with it, whether or not the model has them.
        let flows: Vec<String> = root
            .descendants()
            .into_iter()
            .filter(|e| self.is_flow(e))
            .filter(|e| {
                let (f, t) = self.ends(e);
                f.as_deref() == Some(id) || t.as_deref() == Some(id)
            })
            .filter_map(|e| e.attr(attr))
            .collect();
        for f in &flows {
            self.remove_flow(root, f);
        }
        remove_all(root, &|e| has_id(e, attr, id) && self.is_node(e));
        remove_all(root, &|e| {
            e.is(BPMNDI, "BPMNShape") && e.attr("bpmnElement").as_deref() == Some(id)
        });
        remove_params(root, id);
    }

    fn remove_flow(&mut self, root: &mut Element, id: &str) {
        let attr = id_attr(self.format);
        remove_all(root, &|e| has_id(e, attr, id) && self.is_flow(e));
        remove_all(root, &|e| {
            (e.is(BPMN, "incoming") || e.is(BPMN, "outgoing")) && e.text() == id
        });
        remove_all(root, &|e| {
            e.is(BPMNDI, "BPMNEdge") && e.attr("bpmnElement").as_deref() == Some(id)
        });
        remove_params(root, id);
    }

    fn is_node(&self, e: &Element) -> bool {
        match self.format {
            Format::Bpmn => e.ns.as_deref() == Some(BPMN) && !self.is_flow(e),
            Format::Xpdl => e.is_any(&XPDL, "Activity"),
        }
    }

    fn is_flow(&self, e: &Element) -> bool {
        match self.format {
            Format::Bpmn => e.is(BPMN, "sequenceFlow"),
            Format::Xpdl => e.is_any(&XPDL, "Transition"),
        }
    }

    fn ends(&self, e: &Element) -> (Option<String>, Option<String>) {
        match self.format {
            Format::Bpmn => (e.attr("sourceRef"), e.attr("targetRef")),
            Format::Xpdl => (e.attr("From"), e.attr("To")),
        }
    }

    /// The process (BPMN `process`, XPDL `Activities`) holding node `id`.
    fn container_of<'a>(&self, root: &'a mut Element, id: &str) -> Option<&'a mut Element> {
        let attr = id_attr(self.format);
        let format = self.format;
        find_mut(root, &move |e| {
            let holder = match format {
                Format::Bpmn => e.is(BPMN, "process"),
                Format::Xpdl => e.is_any(&XPDL, "Activities"),
            };
            holder && e.elements().any(|c| has_id(c, attr, id))
        })
    }

    fn first_container<'a>(&self, root: &'a mut Element) -> Option<&'a mut Element> {
        match self.format {
            Format::Bpmn => root.child_mut(&[BPMN], "process"),
            Format::Xpdl => find_mut(root, &|e| e.is_any(&XPDL, "WorkflowProcess"))
                .and_then(|wp| wp.child_mut(&XPDL, "Activities")),
        }
    }

    /// Inserts a node or flow next to its neighbours in the model, so the file lists it in the
    /// same order: after the previous one that is in the file, else before the next, else at the
    /// end of the first process.
    fn place(
        &self,
        root: &mut Element,
        prev: Option<&str>,
        next: Option<&str>,
        el: Element,
        flows: bool,
    ) -> Result<(), SaveError> {
        let attr = id_attr(self.format);
        // After the previous one (offset 1), or before the next (offset 0).
        for (neighbour, offset) in [(prev, 1), (next, 0)] {
            let Some(id) = neighbour else { continue };
            if let Some(c) = self.holder_of(root, id, flows)
                && let Some(i) = c
                    .children
                    .iter()
                    .position(|n| matches!(n, Node::Element(x) if has_id(x, attr, id)))
            {
                c.children.insert(i + offset, Node::Element(el));
                return Ok(());
            }
        }
        let holder = if flows {
            self.flow_holder(root)
        } else {
            self.first_container(root)
        };
        let c = holder.ok_or_else(|| fail("the file has no process to add elements to"))?;
        let at = end_of(c);
        c.children.insert(at, Node::Element(el));
        Ok(())
    }

    fn holder_of<'a>(
        &self,
        root: &'a mut Element,
        id: &str,
        flows: bool,
    ) -> Option<&'a mut Element> {
        if !flows || self.format == Format::Bpmn {
            return self.container_of(root, id);
        }
        find_mut(root, &|e| {
            e.is_any(&XPDL, "Transitions") && e.elements().any(|c| has_id(c, "Id", id))
        })
    }

    /// Where a flow goes when no neighbouring flow is in the file.
    fn flow_holder<'a>(&self, root: &'a mut Element) -> Option<&'a mut Element> {
        match self.format {
            Format::Bpmn => root.child_mut(&[BPMN], "process"),
            Format::Xpdl => {
                let wp = find_mut(root, &|e| e.is_any(&XPDL, "WorkflowProcess"))?;
                if wp.child(&XPDL, "Transitions").is_none() {
                    let ns = wp.ns.clone().unwrap_or_default();
                    let t = Element::new(&qname(wp.prefix(), "Transitions"), Some(&ns));
                    let at = after_last_named(wp, &["Activities"]).unwrap_or_else(|| end_of(wp));
                    wp.children.insert(at, Node::Element(t));
                }
                wp.child_mut(&XPDL, "Transitions")
            }
        }
    }

    fn neighbours<'a>(
        ids: &[&'a str],
        id: &str,
        exists: impl Fn(&str) -> bool,
    ) -> (Option<&'a str>, Option<&'a str>) {
        let i = ids.iter().position(|x| *x == id).unwrap_or(0);
        let prev = ids[..i].iter().rev().copied().find(|x| exists(x));
        let next = ids[i + 1..].iter().copied().find(|x| exists(x));
        (prev, next)
    }

    fn add_node(
        &mut self,
        root: &mut Element,
        after: &Model,
        e: &ModelElement,
    ) -> Result<(), SaveError> {
        let attr = id_attr(self.format);
        let ids: Vec<&str> = after.elements.iter().map(|x| x.id.as_str()).collect();
        let present: BTreeSet<String> = root
            .descendants()
            .into_iter()
            .filter(|x| self.is_node(x))
            .filter_map(|x| x.attr(attr))
            .collect();
        let (prev, next) = Self::neighbours(&ids, e.id.as_str(), |x| present.contains(x));
        let node = match self.format {
            Format::Bpmn => {
                let ns = Ns::bpmn(root);
                let mut n = ns.el(bpmn_local(&e.kind));
                n.set_attr("id", e.id.as_str());
                if !e.name.is_empty() {
                    self.set_text_attr(&mut n, "name", &e.name);
                }
                n
            }
            Format::Xpdl => {
                let ns = self.xpdl_ns(root);
                let mut n = ns.el("Activity");
                n.set_attr("Id", e.id.as_str());
                if !e.name.is_empty() {
                    self.set_text_attr(&mut n, "Name", &e.name);
                }
                match e.kind {
                    ElementKind::Source { .. } => n.push(
                        ns.el("Event")
                            .with_child(ns.el("StartEvent").with_attr("Trigger", "None")),
                    ),
                    ElementKind::Sink => n.push(
                        ns.el("Event")
                            .with_child(ns.el("EndEvent").with_attr("Result", "None")),
                    ),
                    ElementKind::ExclusiveGateway { .. } => {
                        n.push(ns.el("Route").with_attr("GatewayType", "Exclusive"))
                    }
                    ElementKind::ParallelGateway => {
                        n.push(ns.el("Route").with_attr("GatewayType", "Parallel"))
                    }
                    _ => n.push(ns.el("Implementation").with_child(ns.el("Task"))),
                }
                n
            }
        };
        self.place(root, prev, next, node, false)?;
        self.add_shape(root, e, prev);
        Ok(())
    }

    fn xpdl_ns(&self, root: &Element) -> Ns {
        let uri = root.ns.clone().unwrap_or_else(|| XPDL[0].to_owned());
        let prefix = find(root, &|e| {
            e.is_any(&XPDL, "Activity") || e.is_any(&XPDL, "WorkflowProcess")
        })
        .map_or_else(|| root.prefix().to_owned(), |e| e.prefix().to_owned());
        Ns {
            prefix,
            uri,
            declare: false,
        }
    }

    fn add_flow(
        &mut self,
        root: &mut Element,
        after: &Model,
        c: &Connection,
    ) -> Result<(), SaveError> {
        let attr = id_attr(self.format);
        let ids: Vec<&str> = after.connections.iter().map(|x| x.id.as_str()).collect();
        let present: BTreeSet<String> = root
            .descendants()
            .into_iter()
            .filter(|x| self.is_flow(x))
            .filter_map(|x| x.attr(attr))
            .collect();
        let (prev, next) = Self::neighbours(&ids, c.id.as_str(), |x| present.contains(x));
        let flow = match self.format {
            Format::Bpmn => {
                let ns = Ns::bpmn(root);
                ns.el("sequenceFlow")
                    .with_attr("id", c.id.as_str())
                    .with_attr("sourceRef", c.from.as_str())
                    .with_attr("targetRef", c.to.as_str())
            }
            Format::Xpdl => self
                .xpdl_ns(root)
                .el("Transition")
                .with_attr("Id", c.id.as_str())
                .with_attr("From", c.from.as_str())
                .with_attr("To", c.to.as_str()),
        };
        // With no neighbouring flow in the file, a BPMN flow goes where its source node is (a
        // sub-process, for one inside it).
        let home = (prev.is_none() && next.is_none() && self.format == Format::Bpmn)
            .then(|| self.container_of(root, c.from.as_str()))
            .flatten();
        match home {
            Some(h) => {
                let at = end_of(h);
                h.children.insert(at, Node::Element(flow));
            }
            None => self.place(root, prev, next, flow, true)?,
        }
        if self.format == Format::Bpmn {
            let ns = Ns::bpmn(root);
            const HEAD: [&str; 5] = [
                "documentation",
                "extensionElements",
                "auditing",
                "monitoring",
                "categoryValueRef",
            ];
            if let Some(n) = find_mut(root, &|e| {
                has_id(e, "id", c.from.as_str()) && e.ns.as_deref() == Some(BPMN)
            }) {
                let mut order: Vec<&str> = HEAD.to_vec();
                order.extend(["incoming", "outgoing"]);
                insert_ordered(n, ns.el("outgoing").with_text(c.id.as_str()), &order);
            }
            if let Some(n) = find_mut(root, &|e| {
                has_id(e, "id", c.to.as_str()) && e.ns.as_deref() == Some(BPMN)
            }) {
                let mut order: Vec<&str> = HEAD.to_vec();
                order.push("incoming");
                insert_ordered(n, ns.el("incoming").with_text(c.id.as_str()), &order);
            }
        }
        self.add_edge(root, c);
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Diagrams

    /// The bounds (x, y, width, height) of a node's shape.
    fn bounds(&self, root: &Element, id: &str) -> Option<(f64, f64, f64, f64)> {
        let f = |e: &Element, a: &str| e.attr(a).and_then(|v| v.trim().parse::<f64>().ok());
        match self.format {
            Format::Bpmn => {
                let shape = find(root, &|e| {
                    e.is(BPMNDI, "BPMNShape") && e.attr("bpmnElement").as_deref() == Some(id)
                })?;
                let b = shape.child(&[DC], "Bounds")?;
                Some((f(b, "x")?, f(b, "y")?, f(b, "width")?, f(b, "height")?))
            }
            Format::Xpdl => {
                let act = find(root, &|e| {
                    e.is_any(&XPDL, "Activity") && has_id(e, "Id", id)
                })?;
                let info = act
                    .child(&XPDL, "NodeGraphicsInfos")?
                    .child(&XPDL, "NodeGraphicsInfo")?;
                let c = info.child(&XPDL, "Coordinates")?;
                Some((
                    f(c, "XCoordinate")?,
                    f(c, "YCoordinate")?,
                    f(info, "Width")?,
                    f(info, "Height")?,
                ))
            }
        }
    }

    /// Where a new node's shape goes: to the right of the previous element, or of everything.
    fn position(&self, root: &Element, prev: Option<&str>, h: f64) -> (f64, f64) {
        match prev.and_then(|p| self.bounds(root, p)) {
            Some((x, y, pw, ph)) => (x + pw + 50.0, y + ph / 2.0 - h / 2.0),
            None => (40.0, 40.0),
        }
    }

    fn add_shape(&mut self, root: &mut Element, e: &ModelElement, prev: Option<&str>) {
        let (w, h) = size(&e.kind);
        let (x, y) = self.position(root, prev, h);
        match self.format {
            Format::Bpmn => {
                let process = root
                    .children_named(&[BPMN], "process")
                    .find(|p| p.elements().any(|c| has_id(c, "id", e.id.as_str())))
                    .and_then(|p| p.attr("id"));
                let has_plane = find(root, &|x| x.is(BPMNDI, "BPMNPlane")).is_some();
                if !has_plane {
                    return;
                }
                let di = Ns::of(root, BPMNDI, "bpmndi");
                let dc = Ns::of(root, DC, "dc");
                let shape_id = self.fresh(&format!("{}_di", e.id));
                let mut shape = di
                    .el("BPMNShape")
                    .with_attr("id", &shape_id)
                    .with_attr("bpmnElement", e.id.as_str());
                shape.push(
                    dc.el("Bounds")
                        .with_attr("x", &num(x))
                        .with_attr("y", &num(y))
                        .with_attr("width", &num(w))
                        .with_attr("height", &num(h)),
                );
                let plane = find_mut(root, &|p| {
                    p.is(BPMNDI, "BPMNPlane")
                        && (process.is_none() || p.attr("bpmnElement") == process)
                });
                let plane = match plane {
                    Some(p) => Some(p),
                    None => find_mut(root, &|p| p.is(BPMNDI, "BPMNPlane")),
                };
                if let Some(p) = plane {
                    // Shapes before edges, as tools write them.
                    let at = first_named(p, &["BPMNEdge"]).unwrap_or_else(|| end_of(p));
                    p.children.insert(at, Node::Element(shape));
                }
            }
            Format::Xpdl => {
                let ns = self.xpdl_ns(root);
                // Copy the drawing tool's attributes from a neighbouring node.
                let template = find(root, &|x| x.is_any(&XPDL, "NodeGraphicsInfo")).cloned();
                let Some(template) = template else { return };
                let mut info = ns.el("NodeGraphicsInfo");
                for (n, v) in &template.attrs {
                    if !matches!(n.as_str(), "Width" | "Height") {
                        info.attrs.push((n.clone(), v.clone()));
                    }
                }
                info.set_attr("Width", &num(w));
                info.set_attr("Height", &num(h));
                info.push(
                    ns.el("Coordinates")
                        .with_attr("XCoordinate", &num(x))
                        .with_attr("YCoordinate", &num(y)),
                );
                if let Some(act) = find_mut(root, &|x| {
                    x.is_any(&XPDL, "Activity") && has_id(x, "Id", e.id.as_str())
                }) {
                    act.push(ns.el("NodeGraphicsInfos").with_child(info));
                }
            }
        }
    }

    fn add_edge(&mut self, root: &mut Element, c: &Connection) {
        let (Some(a), Some(b)) = (
            self.bounds(root, c.from.as_str()),
            self.bounds(root, c.to.as_str()),
        ) else {
            return;
        };
        let from = (a.0 + a.2, a.1 + a.3 / 2.0);
        let to = (b.0, b.1 + b.3 / 2.0);
        match self.format {
            Format::Bpmn => {
                let di = Ns::of(root, BPMNDI, "bpmndi");
                let dip = Ns::of(root, DI, "di");
                let id = self.fresh(&format!("{}_di", c.id));
                let wp = |p: (f64, f64)| {
                    dip.el("waypoint")
                        .with_attr("x", &num(p.0))
                        .with_attr("y", &num(p.1))
                };
                let edge = di
                    .el("BPMNEdge")
                    .with_attr("id", &id)
                    .with_attr("bpmnElement", c.id.as_str())
                    .with_child(wp(from))
                    .with_child(wp(to));
                let shape_of = c.from.as_str().to_owned();
                if let Some(p) = find_mut(root, &|p| {
                    p.is(BPMNDI, "BPMNPlane")
                        && p.elements()
                            .any(|s| s.attr("bpmnElement").as_deref() == Some(shape_of.as_str()))
                }) {
                    let at = end_of(p);
                    p.children.insert(at, Node::Element(edge));
                }
            }
            Format::Xpdl => {
                let ns = self.xpdl_ns(root);
                let template = find(root, &|x| x.is_any(&XPDL, "ConnectorGraphicsInfo")).cloned();
                let Some(template) = template else { return };
                let mut info = ns.el("ConnectorGraphicsInfo");
                info.attrs = template.attrs.clone();
                for p in [from, to] {
                    info.push(
                        ns.el("Coordinates")
                            .with_attr("XCoordinate", &num(p.0))
                            .with_attr("YCoordinate", &num(p.1)),
                    );
                }
                if let Some(t) = find_mut(root, &|x| {
                    x.is_any(&XPDL, "Transition") && has_id(x, "Id", c.id.as_str())
                }) {
                    t.push(ns.el("ConnectorGraphicsInfos").with_child(info));
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Names

    fn set_model_name(&self, root: &mut Element, name: &str) {
        match self.format {
            Format::Bpmn => {
                let on_root = root.attr("name").is_some_and(|n| !n.is_empty());
                let target = if on_root {
                    Some(root)
                } else {
                    root.child_mut(&[BPMN], "process")
                };
                if let Some(t) = target {
                    self.set_or_remove(t, "name", name);
                }
            }
            Format::Xpdl => self.set_or_remove(root, "Name", name),
        }
    }

    fn set_node_name(&self, root: &mut Element, id: &str, name: &str) {
        let (attr, name_attr) = match self.format {
            Format::Bpmn => ("id", "name"),
            Format::Xpdl => ("Id", "Name"),
        };
        let format = self.format;
        if let Some(n) = find_mut(root, &|e| {
            has_id(e, attr, id)
                && match format {
                    Format::Bpmn => e.ns.as_deref() == Some(BPMN),
                    Format::Xpdl => e.is_any(&XPDL, "Activity"),
                }
        }) {
            self.set_or_remove(n, name_attr, name);
        }
    }

    fn set_or_remove(&self, e: &mut Element, attr: &str, value: &str) {
        if value.is_empty() {
            e.remove_attr(attr);
        } else {
            self.set_text_attr(e, attr, value);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Resources

    /// Gives task `task` a resource of its own and returns its id.
    fn add_resource(&mut self, root: &mut Element, task: &ModelElement) -> String {
        let rid = self.fresh(&format!("{}_resource", task.id));
        match self.format {
            Format::Bpmn => {
                let ns = Ns::bpmn(root);
                let mut r = ns.el("resource").with_attr("id", &rid);
                if !task.name.is_empty() {
                    self.set_text_attr(&mut r, "name", &task.name);
                }
                let at = first_named(root, &["process"]).unwrap_or_else(|| end_of(root));
                root.children.insert(at, Node::Element(r));
                if let Some(n) = find_mut(root, &|e| {
                    has_id(e, "id", task.id.as_str()) && e.ns.as_deref() == Some(BPMN)
                }) {
                    let p = ns
                        .el("performer")
                        .with_child(ns.el("resourceRef").with_text(&rid));
                    let at = first_named(
                        n,
                        &[
                            "standardLoopCharacteristics",
                            "multiInstanceLoopCharacteristics",
                        ],
                    )
                    .unwrap_or_else(|| end_of(n));
                    n.children.insert(at, Node::Element(p));
                }
            }
            Format::Xpdl => {
                let ns = self.xpdl_ns(root);
                let mut part = ns.el("Participant").with_attr("Id", &rid);
                if !task.name.is_empty() {
                    self.set_text_attr(&mut part, "Name", &task.name);
                }
                part.push(ns.el("ParticipantType").with_attr("Type", "RESOURCE"));
                if root.child(&XPDL, "Participants").is_none() {
                    let at = first_named(
                        root,
                        &[
                            "Applications",
                            "DataFields",
                            "PartnerLinkTypes",
                            "Pools",
                            "MessageFlows",
                            "Associations",
                            "Artifacts",
                            "WorkflowProcesses",
                            "ExtendedAttributes",
                        ],
                    )
                    .unwrap_or_else(|| end_of(root));
                    root.children
                        .insert(at, Node::Element(ns.el("Participants")));
                }
                if let Some(ps) = root.child_mut(&XPDL, "Participants") {
                    let at = end_of(ps);
                    ps.children.insert(at, Node::Element(part));
                }
                if let Some(a) = find_mut(root, &|e| {
                    e.is_any(&XPDL, "Activity") && has_id(e, "Id", task.id.as_str())
                }) {
                    let p = ns
                        .el("Performers")
                        .with_child(ns.el("Performer").with_text(&rid));
                    insert_ordered(
                        a,
                        p,
                        &[
                            "Description",
                            "Limit",
                            "Route",
                            "Implementation",
                            "BlockActivity",
                            "Event",
                            "Transaction",
                        ],
                    );
                }
            }
        }
        rid
    }

    /// Takes away a task's resource assignment (it becomes a delay).
    fn remove_resource(&self, root: &mut Element, task: &str) {
        match self.format {
            Format::Bpmn => {
                if let Some(n) = find_mut(root, &|e| {
                    has_id(e, "id", task) && e.ns.as_deref() == Some(BPMN)
                }) {
                    remove_all(n, &|e| {
                        e.is(BPMN, "performer")
                            || e.is(BPMN, "humanPerformer")
                            || e.is(BPMN, "potentialOwner")
                    });
                }
            }
            Format::Xpdl => {
                if let Some(a) = find_mut(root, &|e| {
                    e.is_any(&XPDL, "Activity") && has_id(e, "Id", task)
                }) {
                    remove_all(a, &|e| {
                        e.is_any(&XPDL, "Performers") || e.is_any(&XPDL, "Performer")
                    });
                }
            }
        }
    }

    /// Sets (or removes) a BPMN exclusive gateway's `default` flow. XPDL marks it with a
    /// transition condition instead, which is not written yet.
    fn set_default(
        &self,
        root: &mut Element,
        gateway: &str,
        flow: Option<&str>,
    ) -> Result<(), SaveError> {
        if self.format == Format::Xpdl {
            return Err(fail(format!(
                "setting the default connection of `{gateway}` in an XPDL file is not supported yet"
            )));
        }
        if let Some(n) = find_mut(root, &|e| {
            has_id(e, "id", gateway) && e.ns.as_deref() == Some(BPMN)
        }) {
            self.set_or_remove(n, "default", flow.unwrap_or(""));
        }
        Ok(())
    }

    /// Writes the probabilities and conditions of connections that changed (or are new).
    fn routing(
        &mut self,
        root: &mut Element,
        scenario: Option<&str>,
        before: &Model,
        after: &Model,
    ) -> Result<(), SaveError> {
        let mut writes: Vec<(String, &'static str, Option<Element>)> = Vec::new();
        for c in &after.connections {
            let was = before.connections.iter().find(|o| same_link(o, c));
            let (op, oc) = was.map_or((None, None), |o| (o.probability, o.condition.clone()));
            if op != c.probability {
                writes.push((
                    c.id.to_string(),
                    "Probability",
                    c.probability.map(probability_element),
                ));
            }
            if oc != c.condition {
                let v = c.condition.as_deref().map(|x| {
                    let mut v = condition_element(x);
                    self.text_attrs(&mut v);
                    v
                });
                writes.push((c.id.to_string(), "Condition", v));
            }
        }
        // Writing nothing where there was nothing leaves the file alone.
        writes.retain(|(id, _, v)| {
            v.is_some() || before.connections.iter().any(|o| o.id.as_str() == id)
        });
        if writes.is_empty() {
            return Ok(());
        }
        let s = self.ensure_scenario(root, scenario)?;
        for (id, name, v) in writes {
            set_param(s, &id, "ControlParameters", name, v);
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Parameters

    fn parameters(
        &mut self,
        root: &mut Element,
        scenario: Option<&str>,
        old: &BTreeMap<&str, &ModelElement>,
        replaced: &BTreeSet<&str>,
        after: &Model,
        resources: &read::Resources,
    ) -> Result<(), SaveError> {
        // Every change, worked out first, then written into the scenario.
        let mut writes: Vec<(String, &'static str, &'static str, Option<Element>)> = Vec::new();
        let mut property_writes: Vec<(String, String, Option<Element>)> = Vec::new();
        let mut new_resources: Vec<(&ModelElement, bool)> = Vec::new();
        for e in &after.elements {
            let was_element = old
                .get(e.id.as_str())
                .filter(|_| !replaced.contains(e.id.as_str()));
            let was = was_element.map(|w| &w.kind);
            let id = e.id.to_string();
            let old_props = was_element.map_or(&[][..], |w| w.properties.as_slice());
            for p in old_props {
                if e.property(&p.name).is_none() {
                    property_writes.push((id.clone(), p.name.clone(), None));
                }
            }
            for p in &e.properties {
                if old_props.iter().find(|o| o.name == p.name) != Some(p) {
                    let mut v = property_element(p);
                    self.text_attrs(&mut v);
                    property_writes.push((id.clone(), p.name.clone(), Some(v)));
                }
            }
            match &e.kind {
                ElementKind::Source {
                    interarrival,
                    limit,
                } => {
                    let (oi, ol) = match was {
                        Some(ElementKind::Source {
                            interarrival,
                            limit,
                        }) => (Some(interarrival), Some(*limit)),
                        _ => (None, None),
                    };
                    if oi != Some(interarrival) {
                        writes.push((
                            id.clone(),
                            "ControlParameters",
                            "InterTriggerTimer",
                            Some(dist_element(interarrival)),
                        ));
                    }
                    if ol != Some(*limit) {
                        let v = limit.map(numeric);
                        if v.is_some() || ol.is_some() {
                            writes.push((id.clone(), "ControlParameters", "TriggerCount", v));
                        }
                    }
                }
                ElementKind::Server { service: d, .. } | ElementKind::Delay { duration: d } => {
                    let od = match was {
                        Some(ElementKind::Server { service, .. }) => Some(service),
                        Some(ElementKind::Delay { duration }) => Some(duration),
                        _ => None,
                    };
                    if od != Some(d) {
                        writes.push((
                            id.clone(),
                            "TimeParameters",
                            "ProcessingTime",
                            Some(dist_element(d)),
                        ));
                    }
                    let was_server = matches!(was, Some(ElementKind::Server { .. }));
                    match (&e.kind, was) {
                        (
                            ElementKind::Server { capacity, .. },
                            Some(ElementKind::Server { capacity: oc, .. }),
                        ) if capacity != oc => {
                            let r = resources.of_task.get(&id).ok_or_else(|| {
                                fail(format!(
                                    "`{id}` has no resource of its own to set the capacity of"
                                ))
                            })?;
                            let users = resources.users.get(r).map_or(0, Vec::len);
                            if users > 1 {
                                return Err(fail(format!(
                                    "the capacity of `{id}` is resource `{r}`'s quantity, which {users} tasks share; shared resources can be edited from M2.5"
                                )));
                            }
                            writes.push((
                                r.clone(),
                                "ResourceParameters",
                                "Quantity",
                                Some(numeric(u64::from(*capacity))),
                            ));
                        }
                        (ElementKind::Server { .. }, _) if !was_server => {
                            new_resources.push((e, true))
                        }
                        (ElementKind::Delay { .. }, _) if was_server => {
                            new_resources.push((e, false))
                        }
                        _ => {}
                    }
                }
                ElementKind::Sink
                | ElementKind::ExclusiveGateway { .. }
                | ElementKind::ParallelGateway => {}
            }
        }
        for (e, server) in new_resources {
            if server {
                let rid = self.add_resource(root, e);
                if let ElementKind::Server { capacity, .. } = e.kind {
                    writes.push((
                        rid,
                        "ResourceParameters",
                        "Quantity",
                        Some(numeric(u64::from(capacity))),
                    ));
                }
            } else {
                let task = e.id.to_string();
                if let Some(s) = scenario_mut(root, scenario) {
                    for ep in s.elements_mut().filter(|x| {
                        x.is_any(&NSS, "ElementParameters")
                            && x.attr("elementRef").as_deref() == Some(task.as_str())
                    }) {
                        remove_all(ep, &|x| x.is_any(&NSS, "Selection"));
                    }
                }
                self.remove_resource(root, &task);
            }
        }
        if writes.is_empty() && property_writes.is_empty() {
            return Ok(());
        }
        let s = self.ensure_scenario(root, scenario)?;
        for (element, group, name, v) in writes {
            set_param(s, &element, group, name, v);
        }
        for (element, name, v) in property_writes {
            set_property(s, &element, &name, v);
        }
        Ok(())
    }

    /// Writes a built element's text attributes (a property's name, a text value or an
    /// expression) as character references where the file's encoding needs it.
    fn text_attrs(&self, e: &mut Element) {
        if self.ascii {
            for a in ["name", "value"] {
                if let Some(v) = e.attr(a).filter(|v| !v.is_ascii()) {
                    self.set_text_attr(e, a, &v);
                }
            }
            for c in e.elements_mut() {
                self.text_attrs(c);
            }
        }
    }

    /// The scenario that was opened, created (with its `BPSimData`) if the file had none.
    fn ensure_scenario<'a>(
        &mut self,
        root: &'a mut Element,
        scenario: Option<&str>,
    ) -> Result<&'a mut Element, SaveError> {
        if crate::bpsim::find_data(root).is_none() {
            let id = self.fresh("scenario");
            let bpsim = Ns::of(root, NS2, "bpsim");
            let data = bpsim.el("BPSimData").with_child(
                Element::new(&qname(&bpsim.prefix, "Scenario"), Some(NS2))
                    .with_attr("id", &id)
                    .with_attr("name", "Scenario"),
            );
            match self.format {
                Format::Bpmn => {
                    let b = Ns::bpmn(root);
                    let defs_id = root.attr("id").unwrap_or_default();
                    let rel = b
                        .el("relationship")
                        .with_attr("type", "BPSimData")
                        .with_child(b.el("extensionElements").with_child(data))
                        .with_child(b.el("source").with_text(&defs_id))
                        .with_child(b.el("target").with_text(&defs_id));
                    let at = end_of(root);
                    root.children.insert(at, Node::Element(rel));
                }
                Format::Xpdl => {
                    let at = end_of(root);
                    root.children.insert(at, Node::Element(data));
                }
            }
            return scenario_mut(root, Some(&id)).ok_or_else(|| fail("could not add a scenario"));
        }
        if scenario.is_none() {
            let id = self.fresh("scenario");
            let data = data_mut(root).ok_or_else(|| fail("no BPSim data"))?;
            let ns = data.ns.clone().unwrap_or_else(|| NS2.to_owned());
            let sc = Element::new(&qname(data.prefix(), "Scenario"), Some(&ns))
                .with_attr("id", &id)
                .with_attr("name", "Scenario");
            let at = end_of(data);
            data.children.insert(at, Node::Element(sc));
            return scenario_mut(root, Some(&id)).ok_or_else(|| fail("could not add a scenario"));
        }
        scenario_mut(root, scenario)
            .ok_or_else(|| fail("the scenario that was opened is no longer in the file"))
    }

    fn experiment(
        &mut self,
        root: &mut Element,
        scenario: Option<&str>,
        before: &Model,
        after: &Model,
    ) -> Result<(), SaveError> {
        let (b, a) = (&before.experiment, &after.experiment);
        let s = self.ensure_scenario(root, scenario)?;
        let ns = s.ns.clone().unwrap_or_else(|| NS2.to_owned());
        let prefix = s.prefix().to_owned();
        if s.child(&NSS, "ScenarioParameters").is_none() {
            let at = s
                .children
                .iter()
                .position(|n| matches!(n, Node::Element(_)))
                .unwrap_or_else(|| end_of(s));
            s.children.insert(
                at,
                Node::Element(Element::new(
                    &qname(&prefix, "ScenarioParameters"),
                    Some(&ns),
                )),
            );
        }
        let Some(sp) = s.child_mut(&NSS, "ScenarioParameters") else {
            return Ok(());
        };
        if b.replications != a.replications {
            sp.set_attr("replication", &a.replications.to_string());
        }
        if b.seed != a.seed {
            sp.set_attr("seed", &(a.seed as i64).to_string());
        }
        if b.time_unit != a.time_unit {
            sp.set_attr("baseTimeUnit", a.time_unit.bpsim_name());
        }
        let float = |v: f64| {
            let mut e =
                Element::new("bpsim:FloatingParameter", Some(NS2)).with_attr("value", &num(v));
            rename(&mut e, &prefix, &ns);
            e
        };
        if b.warmup != a.warmup || b.duration != a.duration {
            set_value(
                sp,
                &prefix,
                &ns,
                "Duration",
                Some(float(a.warmup + a.duration)),
                &["Start"],
            );
        }
        if b.warmup != a.warmup && (a.warmup != 0.0 || sp.child(&NSS, "Warmup").is_some()) {
            set_value(
                sp,
                &prefix,
                &ns,
                "Warmup",
                Some(float(a.warmup)),
                &["Start", "Duration"],
            );
        }
        Ok(())
    }
}

/// The `BPSimData` that was read (2.0 preferred over 1.0).
fn data_mut(root: &mut Element) -> Option<&mut Element> {
    let has2 = find(root, &|e| e.is(NS2, "BPSimData")).is_some();
    let want = if has2 { NS2 } else { crate::bpsim::NS1 };
    find_mut(root, &move |e| e.is(want, "BPSimData"))
}

/// Scenario `id`, or the first input scenario.
fn scenario_mut<'a>(root: &'a mut Element, id: Option<&str>) -> Option<&'a mut Element> {
    let data = data_mut(root)?;
    data.elements_mut().find(|s| {
        s.is_any(&NSS, "Scenario")
            && match id {
                Some(id) => s.attr("id").as_deref() == Some(id),
                None => s.attr("result").is_none(),
            }
    })
}

/// Removes every scenario's parameters for `id` (an element or connection that is gone).
fn remove_params(root: &mut Element, id: &str) {
    remove_all(root, &|e| {
        e.is_any(&NSS, "ElementParameters") && e.attr("elementRef").as_deref() == Some(id)
    });
}

/// The order of each parameter group's parameters in the BPSim 2.0 schema.
fn param_order(group: &str) -> &'static [&'static str] {
    match group {
        "TimeParameters" => &[
            "TransferTime",
            "QueueTime",
            "WaitTime",
            "SetUpTime",
            "ProcessingTime",
            "ValidationTime",
            "ReworkTime",
            "LagTime",
            "Duration",
            "ElapsedTime",
        ],
        "ControlParameters" => &[
            "Probability",
            "Condition",
            "InterTriggerTimer",
            "TriggerCount",
        ],
        "ResourceParameters" => &["Selection", "Availability", "Quantity", "Role"],
        _ => &[],
    }
}

/// Sets (or with `None` removes) parameter `group/name` of `element` in scenario `s`: in the
/// last of its `ElementParameters` that has it, else the last one, else a new one.
fn set_param(s: &mut Element, element: &str, group: &str, name: &str, value: Option<Element>) {
    let ns = s.ns.clone().unwrap_or_else(|| NS2.to_owned());
    let prefix = s.prefix().to_owned();
    let is_ep = |e: &Element| {
        e.is_any(&NSS, "ElementParameters") && e.attr("elementRef").as_deref() == Some(element)
    };
    let has = |e: &Element| {
        e.child(&NSS, group)
            .and_then(|g| g.child(&NSS, name))
            .is_some()
    };
    let Some(value) = value else {
        for ep in s.elements_mut().filter(|e| is_ep(e)) {
            if let Some(g) = ep.child_mut(&NSS, group) {
                remove_all(g, &|p| p.is_any(&NSS, name));
            }
        }
        return;
    };
    let mut value = value;
    rename(&mut value, &prefix, &ns);
    let Some(g) = group_mut(s, element, group, &has) else {
        return;
    };
    let order = param_order(group);
    let pos = order.iter().position(|p| *p == name).unwrap_or(order.len());
    set_value(g, &prefix, &ns, name, Some(value), &order[..pos]);
}

/// Parameter group `group` of `element` in scenario `s`: in the last of its `ElementParameters`
/// for which `has` holds, else the last one, else a new one; the group is made (in the schema's
/// group order) if that `ElementParameters` lacks it.
fn group_mut<'a>(
    s: &'a mut Element,
    element: &str,
    group: &str,
    has: &dyn Fn(&Element) -> bool,
) -> Option<&'a mut Element> {
    let ns = s.ns.clone().unwrap_or_else(|| NS2.to_owned());
    let prefix = s.prefix().to_owned();
    let is_ep = |e: &Element| {
        e.is_any(&NSS, "ElementParameters") && e.attr("elementRef").as_deref() == Some(element)
    };
    let target = s
        .children
        .iter()
        .rposition(|n| matches!(n, Node::Element(e) if is_ep(e) && has(e)))
        .or_else(|| {
            s.children
                .iter()
                .rposition(|n| matches!(n, Node::Element(e) if is_ep(e)))
        });
    let at = match target {
        Some(i) => i,
        None => {
            let ep = Element::new(&qname(&prefix, "ElementParameters"), Some(&ns))
                .with_attr("elementRef", element);
            let at = after_last_named(s, &["ElementParameters"])
                .or_else(|| after_last_named(s, &["ScenarioParameters"]))
                .unwrap_or_else(|| end_of(s));
            s.children.insert(at, Node::Element(ep));
            at
        }
    };
    let Some(Node::Element(ep)) = s.children.get_mut(at) else {
        return None;
    };
    if ep.child(&NSS, group).is_none() {
        let groups = &crate::bpsim::GROUPS;
        let pos = groups
            .iter()
            .position(|g| *g == group)
            .unwrap_or(groups.len());
        let earlier: Vec<&str> = groups[..pos].to_vec();
        let at = after_last_named(ep, &earlier)
            .or_else(|| {
                ep.children
                    .iter()
                    .position(|n| matches!(n, Node::Element(_)))
            })
            .unwrap_or_else(|| end_of(ep));
        ep.children.insert(
            at,
            Node::Element(Element::new(&qname(&prefix, group), Some(&ns))),
        );
    }
    ep.child_mut(&NSS, group)
}

/// Sets (or with `None` removes) property `name` of `element` in scenario `s`. `value` is a whole
/// `Property` element; an existing one keeps its place and result requests and takes the new
/// type and value.
fn set_property(s: &mut Element, element: &str, name: &str, value: Option<Element>) {
    let is_ep = |e: &Element| {
        e.is_any(&NSS, "ElementParameters") && e.attr("elementRef").as_deref() == Some(element)
    };
    let is_prop =
        |p: &Element| p.is_any(&NSS, "Property") && p.attr("name").as_deref() == Some(name);
    let Some(mut value) = value else {
        for ep in s.elements_mut().filter(|e| is_ep(e)) {
            if let Some(g) = ep.child_mut(&NSS, "PropertyParameters") {
                remove_all(g, &is_prop);
            }
        }
        return;
    };
    let ns = s.ns.clone().unwrap_or_else(|| NS2.to_owned());
    let prefix = s.prefix().to_owned();
    rename(&mut value, &prefix, &ns);
    let has = |e: &Element| {
        e.child(&NSS, "PropertyParameters")
            .is_some_and(|g| g.elements().any(is_prop))
    };
    let Some(g) = group_mut(s, element, "PropertyParameters", &has) else {
        return;
    };
    match g.elements_mut().filter(|p| is_prop(p)).last() {
        // An existing property takes the new type and value, keeping its place and its result
        // requests (which come first).
        Some(p) => {
            match value.attr("type") {
                Some(t) => p.set_attr("type", &t),
                None => p.remove_attr("type"),
            }
            remove_all(p, &|v| v.local() != "ResultRequest");
            let values: Vec<Node> = std::mem::take(&mut value.children)
                .into_iter()
                .filter(|n| matches!(n, Node::Element(_)))
                .collect();
            for v in values {
                let at = end_of(p);
                p.children.insert(at, v);
            }
        }
        // A new property goes last.
        None => {
            let at = end_of(g);
            g.children.insert(at, Node::Element(value));
        }
    }
}

/// Sets parameter `name` of `parent` to `value`, keeping its result requests; a new parameter
/// goes after any of `earlier`.
fn set_value(
    parent: &mut Element,
    prefix: &str,
    ns: &str,
    name: &str,
    value: Option<Element>,
    earlier: &[&str],
) {
    if parent.child(&NSS, name).is_none() {
        let p = Element::new(&qname(prefix, name), Some(ns));
        let at = after_last_named(parent, earlier)
            .or_else(|| {
                parent
                    .children
                    .iter()
                    .position(|n| matches!(n, Node::Element(_)))
            })
            .unwrap_or_else(|| end_of(parent));
        parent.children.insert(at, Node::Element(p));
    }
    let Some(p) = parent.child_mut(&NSS, name) else {
        return;
    };
    // The value replaces the old values; result requests (which come first) stay.
    remove_all(p, &|v| v.local() != "ResultRequest");
    if let Some(v) = value {
        let at = end_of(p);
        p.children.insert(at, Node::Element(v));
    }
}
