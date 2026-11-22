//! The element tree the rest of the crate reads, built from quick-xml's events, and the escapes
//! the writer needs.
//!
//! Names are split into a prefix and a local name. [`resolve_namespaces`] then reads the `xmlns`
//! declarations: an element in the given namespace loses its prefix (so it is found by its
//! local name whatever prefix the file chose), and an element in another namespace keeps or gets
//! one (so it is another vendor's extension). Comments, processing instructions and the declaration are dropped; CDATA is text.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// A node in an element.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Node {
    Element(Element),
    Text(String),
}

/// An element, without namespaces resolved.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Element {
    /// The part of the name after any prefix.
    pub name: String,
    /// The prefix, if the name had one (`ext` in `ext:thing`).
    pub prefix: Option<String>,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// The child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// The first child with this local name and no prefix.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements()
            .find(|e| e.prefix.is_none() && e.name == name)
    }

    /// The children with this local name and no prefix.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.elements()
            .filter(move |e| e.prefix.is_none() && e.name == name)
    }

    /// The text directly inside, trimmed.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for n in &self.children {
            if let Node::Text(t) = n {
                s.push_str(t);
            }
        }
        s.trim().to_owned()
    }

    /// The name as written.
    pub fn full_name(&self) -> String {
        match &self.prefix {
            Some(p) => format!("{p}:{}", self.name),
            None => self.name.clone(),
        }
    }
}

type Res<T> = Result<T, String>;

/// Resolves the namespaces under `root` against `wanted`, as the module's note says. An element
/// with no namespace in scope and no prefix is left as it is (a file may omit the namespace);
/// one with a prefix that is not declared is left an extension.
pub(crate) fn resolve_namespaces(root: &mut Element, wanted: &str) {
    let mut scope: Vec<(String, String)> = Vec::new();
    resolve_in(root, wanted, &mut scope);
}

fn resolve_in(e: &mut Element, wanted: &str, scope: &mut Vec<(String, String)>) {
    let mark = scope.len();
    for (k, v) in &e.attrs {
        if k == "xmlns" {
            scope.push((String::new(), v.clone()));
        } else if let Some(p) = k.strip_prefix("xmlns:") {
            scope.push((p.to_owned(), v.clone()));
        }
    }
    let key = e.prefix.clone().unwrap_or_default();
    let uri = scope
        .iter()
        .rev()
        .find(|(p, _)| *p == key)
        .map(|(_, u)| u.clone());
    match uri {
        Some(u) if u == wanted => e.prefix = None,
        Some(_) if e.prefix.is_none() => e.prefix = Some("ns".to_owned()),
        _ => {}
    }
    for c in &mut e.children {
        if let Node::Element(child) = c {
            resolve_in(child, wanted, scope);
        }
    }
    scope.truncate(mark);
}

/// How deep elements may nest, so a hostile file cannot run the stack out.
const MAX_DEPTH: usize = 256;

fn attributes(e: &quick_xml::events::BytesStart<'_>) -> Res<Vec<(String, String)>> {
    let mut out = Vec::new();
    for a in e.attributes().with_checks(false) {
        let a = a.map_err(|err| err.to_string())?;
        let key = a.key.as_ref().to_owned();
        out.push((key, unescape(a.value.as_ref())?));
    }
    Ok(out)
}

fn open(e: &quick_xml::events::BytesStart<'_>) -> Res<Element> {
    let full: String = e.name().as_ref().to_owned();
    let (prefix, name) = match full.split_once(':') {
        Some((p, n)) => (Some(p.to_owned()), n.to_owned()),
        None => (None, full),
    };
    Ok(Element {
        name,
        prefix,
        attrs: attributes(e)?,
        children: Vec::new(),
    })
}

/// The text an entity reference stands for.
fn reference(name: &str) -> Res<String> {
    unescape(&format!("&{name};"))
}

/// Parses a document and returns its root element. Reading is quick-xml's; a document type
/// declaration is refused, so no entity can expand into more text.
pub(crate) fn parse(text: &str) -> Res<Element> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("at byte {}: {e}", reader.buffer_position()))?;
        match event {
            Event::Start(e) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(format!("elements nested more than {MAX_DEPTH} deep"));
                }
                stack.push(open(&e)?);
            }
            Event::Empty(e) => {
                let el = open(&e)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Element(el)),
                    None if root.is_none() => root = Some(el),
                    None => return Err("more than one root element".to_owned()),
                }
            }
            Event::End(_) => {
                let Some(el) = stack.pop() else {
                    return Err("an end tag with no start tag".to_owned());
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Element(el)),
                    None if root.is_none() => root = Some(el),
                    None => return Err("more than one root element".to_owned()),
                }
            }
            Event::Text(t) => {
                let s: String = (*t).to_owned();
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Text(s)),
                    None if s.trim().is_empty() => {}
                    None => return Err("text outside the root element".to_owned()),
                }
            }
            Event::GeneralRef(r) => {
                let s = reference(&r)?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Text(s)),
                    None => return Err("text outside the root element".to_owned()),
                }
            }
            Event::CData(t) => {
                let s: String = (*t).to_owned();
                match stack.last_mut() {
                    Some(parent) => parent.children.push(Node::Text(s)),
                    None => return Err("text outside the root element".to_owned()),
                }
            }
            Event::DocType(_) => {
                return Err("a document type declaration is not allowed".to_owned());
            }
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err("an element is not closed at the end of the file".to_owned());
    }
    root.ok_or_else(|| "no root element".to_owned())
}

fn unescape(raw: &str) -> Res<String> {
    if !raw.contains('&') {
        return Ok(raw.to_owned());
    }
    let mut out = String::new();
    let mut rest = raw;
    while let Some(p) = rest.find('&') {
        out.push_str(rest.get(..p).unwrap_or_default());
        let after = rest.get(p + 1..).unwrap_or_default();
        let Some(end) = after.find(';') else {
            return Err("an `&` is not part of an entity".to_owned());
        };
        let ent = after.get(..end).unwrap_or_default();
        match ent {
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "amp" => out.push('&'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let code = ent
                    .strip_prefix("#x")
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()));
                match code.and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => return Err(format!("the entity `&{ent};` is not known")),
                }
            }
        }
        rest = after.get(end + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    Ok(out)
}

/// Escapes text or an attribute value for writing.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn reads_elements_attributes_text_and_entities() {
        let r = parse(
            "<?xml version=\"1.0\"?><!-- c --><a x='1 &amp; 2'><b>1 &lt; 2</b><c/><e:f>t</e:f></a>",
        );
        let a = r.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(a.name, "a");
        assert_eq!(a.attr("x"), Some("1 & 2"));
        assert_eq!(a.child("b").map(Element::text).as_deref(), Some("1 < 2"));
        assert!(a.child("c").is_some());
        // A prefixed element is not found by `child`.
        assert!(a.child("f").is_none());
        assert_eq!(a.elements().count(), 3);
    }

    #[test]
    fn refuses_what_is_not_well_formed() {
        assert!(parse("<a><b></a>").is_err());
        assert!(parse("<a>").is_err());
        assert!(parse("<!DOCTYPE a><a/>").is_err());
        assert!(parse("<a>&nope;</a>").is_err());
        assert!(parse("<a/>x").is_err());
    }

    #[test]
    fn escape_then_parse_gives_the_text_back() {
        let s = "a < b & \"c\" > d";
        let doc = format!("<a v=\"{}\">{}</a>", escape(s), escape(s));
        let a = parse(&doc).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(a.attr("v"), Some(s));
        assert_eq!(a.text(), s);
    }
}
