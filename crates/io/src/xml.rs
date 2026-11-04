//! A small XML tree that keeps what it does not understand.
//!
//! Text and attribute values are stored exactly as they appear in the file (still escaped), and
//! element and attribute names keep their prefixes, so writing the tree back reproduces every
//! element, attribute, comment and processing instruction StochCraft did not touch. Namespaces
//! are resolved at parse time so lookups match on (namespace, local name) whatever prefix a tool
//! chose (`semantic:task`, `bpmn:task` or a default namespace).
//!
//! A parsed document written back is byte for byte the file it came from: each element keeps the
//! exact text of its start tag (attribute quotes, spacing, `<a/>` or `<a></a>`) until its name or
//! attributes change, and the whitespace around the root element is kept.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// A node in the tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    /// Character data, still escaped as in the file.
    Text(String),
    CData(String),
    Comment(String),
    /// A processing instruction's content (without `<?` and `?>`).
    Pi(String),
}

/// An element.
#[derive(Debug, Clone)]
pub struct Element {
    /// The name as written, with its prefix (`semantic:task`).
    pub name: String,
    /// The namespace URI the prefix resolves to, if any.
    pub ns: Option<String>,
    /// Attributes in file order: (name as written, value still escaped).
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    /// How a parsed element's start tag was written, used while its name and attributes are
    /// still the ones it was parsed with.
    raw: Option<Raw>,
}

#[derive(Debug, Clone)]
struct Raw {
    /// The tag between `<` and `>` (without the `/` of an empty-element tag).
    tag: String,
    /// Written as `<a/>` rather than `<a></a>`.
    empty: bool,
    /// The name and attributes it was parsed with.
    name: String,
    attrs: Vec<(String, String)>,
}

/// Elements are equal when their names, namespaces, attributes and children are, however their
/// tags were spelt.
impl PartialEq for Element {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.ns == other.ns
            && self.attrs == other.attrs
            && self.children == other.children
    }
}

/// A parsed document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The XML declaration's content, if the file had one (`version="1.0" encoding="UTF-8"`).
    pub decl: Option<String>,
    /// Comments and processing instructions before the root element. In a parsed document the
    /// text between them (whitespace) is kept too, starting with the text after the declaration.
    pub prolog: Vec<Node>,
    pub root: Element,
    /// What follows the root element in a parsed document: whitespace, comments and processing
    /// instructions.
    pub epilog: Vec<Node>,
}

impl Document {
    /// A document built in memory, written with a declaration and indented.
    pub fn new(root: Element) -> Self {
        Self {
            decl: None,
            prolog: Vec::new(),
            root,
            epilog: Vec::new(),
        }
    }

    /// Whether the declaration names an encoding other than UTF-8 (the content is then ASCII,
    /// since only UTF-8 is read, and new text must be written as character references).
    pub fn ascii_only(&self) -> bool {
        self.decl.as_deref().is_some_and(|d| {
            let d = d.to_ascii_lowercase();
            d.contains("encoding") && !d.contains("utf-8") && !d.contains("utf8")
        })
    }
}

/// Why a file is not well-formed XML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError(pub String);

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not well-formed XML: {}", self.0)
    }
}

impl Element {
    pub fn new(name: &str, ns: Option<&str>) -> Self {
        Self {
            name: name.to_owned(),
            ns: ns.map(str::to_owned),
            attrs: Vec::new(),
            children: Vec::new(),
            raw: None,
        }
    }

    /// The local name, without prefix.
    pub fn local(&self) -> &str {
        local(&self.name)
    }

    /// Whether this element is `local` in namespace `ns`.
    pub fn is(&self, ns: &str, local_name: &str) -> bool {
        self.ns.as_deref() == Some(ns) && self.local() == local_name
    }

    /// Whether this element is `local` in any of `nss`.
    pub fn is_any(&self, nss: &[&str], local_name: &str) -> bool {
        nss.iter().any(|ns| self.is(ns, local_name))
    }

    /// An unprefixed attribute's value, unescaped.
    pub fn attr(&self, name: &str) -> Option<String> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| unescape(v))
    }

    pub fn set_attr(&mut self, name: &str, value: &str) {
        let escaped = escape(value);
        debug_assert!(!escaped.contains('"'));
        match self.attrs.iter_mut().find(|(n, _)| n == name) {
            Some(a) => a.1 = escaped,
            None => self.attrs.push((name.to_owned(), escaped)),
        }
    }

    /// Sets an attribute to a value that is already escaped.
    pub fn set_attr_escaped(&mut self, name: &str, escaped: &str) {
        match self.attrs.iter_mut().find(|(n, _)| n == name) {
            Some(a) => a.1 = escaped.to_owned(),
            None => self.attrs.push((name.to_owned(), escaped.to_owned())),
        }
    }

    /// Whether StochCraft built this element (it was not parsed from the file).
    pub fn is_new(&self) -> bool {
        self.raw.is_none()
    }

    /// Whether this element was parsed from an empty-element tag (`<a/>`).
    pub fn was_empty_tag(&self) -> bool {
        self.raw.as_ref().is_some_and(|r| r.empty)
    }

    /// Removes an unprefixed attribute.
    pub fn remove_attr(&mut self, name: &str) {
        self.attrs.retain(|(n, _)| n != name);
    }

    pub fn with_attr(mut self, name: &str, value: &str) -> Self {
        self.set_attr(name, value);
        self
    }

    pub fn push(&mut self, child: Element) {
        self.children.push(Node::Element(child));
    }

    pub fn with_child(mut self, child: Element) -> Self {
        self.push(child);
        self
    }

    pub fn with_text(mut self, text: &str) -> Self {
        self.children.push(Node::Text(escape(text)));
        self
    }

    /// Child elements.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// Child elements named `local` in any of `nss`, in any order (lenient reading).
    pub fn children_named<'a>(
        &'a self,
        nss: &'a [&'a str],
        local_name: &'a str,
    ) -> impl Iterator<Item = &'a Element> {
        self.elements().filter(move |e| e.is_any(nss, local_name))
    }

    pub fn child(&self, nss: &[&str], local_name: &str) -> Option<&Element> {
        self.elements().find(|e| e.is_any(nss, local_name))
    }

    /// Child elements, to change.
    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    pub fn child_mut(&mut self, nss: &[&str], local_name: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.is_any(nss, local_name))
    }

    /// The prefix of this element's name (`""` for none).
    pub fn prefix(&self) -> &str {
        prefix(&self.name)
    }

    /// Every descendant element (depth first, document order).
    pub fn descendants(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        fn walk<'a>(e: &'a Element, out: &mut Vec<&'a Element>) {
            for c in e.elements() {
                out.push(c);
                walk(c, out);
            }
        }
        walk(self, &mut out);
        out
    }

    /// The element's text content (all text children, unescaped, trimmed).
    pub fn text(&self) -> String {
        let mut s = String::new();
        for n in &self.children {
            match n {
                Node::Text(t) => s.push_str(&unescape(t)),
                Node::CData(t) => s.push_str(t),
                _ => {}
            }
        }
        s.trim().to_owned()
    }
}

pub fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, l)| l)
}

fn prefix(name: &str) -> &str {
    name.split_once(':').map_or("", |(p, _)| p)
}

/// Unescapes the five predefined entities and character references; anything else is kept.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';') else {
            out.push_str(tail);
            return out;
        };
        let entity = &tail[1..end];
        let ch = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                u32::from_str_radix(&entity[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if entity.starts_with('#') => entity[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => out.push(c),
            None => out.push_str(&tail[..=end]),
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Escapes text for an attribute value or element content.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// [`escape`], with every non-ASCII character as a character reference (for files declared in an
/// encoding other than UTF-8).
pub fn escape_ascii(s: &str) -> String {
    escape(s)
        .chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_string()
            } else {
                format!("&#{};", u32::from(c))
            }
        })
        .collect()
}

/// In-scope namespace bindings while parsing: (prefix, uri), innermost last.
type Scope = Vec<(String, String)>;

fn resolve(scope: &Scope, prefix: &str) -> Option<String> {
    scope
        .iter()
        .rev()
        .find(|(p, _)| p == prefix)
        .map(|(_, u)| u.clone())
}

fn start(
    raw_name: &str,
    raw_attrs: Vec<(String, String)>,
    tag: &str,
    empty: bool,
    scope: &mut Scope,
) -> (Element, usize) {
    let name = raw_name.to_owned();
    let mut pushed = 0;
    for (n, v) in &raw_attrs {
        if n == "xmlns" {
            scope.push((String::new(), unescape(v)));
            pushed += 1;
        } else if let Some(p) = n.strip_prefix("xmlns:") {
            scope.push((p.to_owned(), unescape(v)));
            pushed += 1;
        }
    }
    let ns = resolve(scope, prefix(&name)).filter(|u| !u.is_empty());
    let raw = Raw {
        tag: tag.to_owned(),
        empty,
        name: name.clone(),
        attrs: raw_attrs.clone(),
    };
    (
        Element {
            name,
            ns,
            attrs: raw_attrs,
            children: Vec::new(),
            raw: Some(raw),
        },
        pushed,
    )
}

fn attrs_of(e: &quick_xml::events::BytesStart<'_>) -> Result<Vec<(String, String)>, XmlError> {
    let mut out = Vec::new();
    for a in e.attributes().with_checks(false) {
        let a = a.map_err(|err| XmlError(err.to_string()))?;
        let key = a.key.as_ref().to_owned();
        // Stored ready for double quotes: a single-quoted value may contain `"` unescaped.
        let value = a.value.as_ref().replace('"', "&quot;");
        out.push((key, value));
    }
    Ok(out)
}

/// Parses a document. The input must be UTF-8 (as BPMN and XPDL files are in practice); a byte
/// order mark is skipped.
pub fn parse(bytes: &[u8]) -> Result<Document, XmlError> {
    let text = std::str::from_utf8(bytes).map_err(|e| XmlError(format!("not UTF-8: {e}")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut decl = None;
    let mut prolog = Vec::new();
    let mut stack: Vec<(Element, usize)> = Vec::new();
    let mut scope: Scope = vec![("xml".into(), "http://www.w3.org/XML/1998/namespace".into())];
    let mut root: Option<Element> = None;
    let mut epilog = Vec::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|e| XmlError(format!("at byte {}: {e}", reader.buffer_position())))?;
        let node = match event {
            Event::Decl(d) => {
                // The event's content starts with the `xml` of `<?xml`.
                let content = (*d).trim();
                let content = content.strip_prefix("xml").unwrap_or(content).trim();
                decl = Some(content.to_owned());
                continue;
            }
            Event::Start(e) => {
                let attrs = attrs_of(&e)?;
                stack.push(start(e.name().as_ref(), attrs, &e, false, &mut scope));
                continue;
            }
            Event::Empty(e) => {
                let attrs = attrs_of(&e)?;
                let (el, pushed) = start(e.name().as_ref(), attrs, &e, true, &mut scope);
                scope.truncate(scope.len() - pushed);
                Node::Element(el)
            }
            Event::End(_) => {
                let Some((el, pushed)) = stack.pop() else {
                    return Err(XmlError("unexpected end tag".into()));
                };
                scope.truncate(scope.len() - pushed);
                Node::Element(el)
            }
            Event::Text(t) => Node::Text((*t).to_owned()),
            Event::GeneralRef(r) => Node::Text(format!("&{};", &*r)),
            Event::CData(t) => Node::CData((*t).to_owned()),
            Event::Comment(t) => Node::Comment((*t).to_owned()),
            Event::PI(t) => Node::Pi((*t).to_owned()),
            Event::DocType(_) => continue,
            Event::Eof => break,
        };
        match stack.last_mut() {
            Some((parent, _)) => match (parent.children.last_mut(), node) {
                // A text node split around an entity reference is joined again.
                (Some(Node::Text(prev)), Node::Text(t)) => prev.push_str(&t),
                (_, node) => parent.children.push(node),
            },
            None => {
                let outside = if root.is_some() {
                    &mut epilog
                } else {
                    &mut prolog
                };
                match node {
                    Node::Element(e) if root.is_none() => root = Some(e),
                    Node::Element(_) => {
                        return Err(XmlError("more than one root element".into()));
                    }
                    Node::Text(t) if t.trim().is_empty() => outside.push(Node::Text(t)),
                    Node::Text(_) | Node::CData(_) => {
                        return Err(XmlError("text outside the root element".into()));
                    }
                    other => {
                        // Mark where text would go, so a parsed document writes its prolog as is.
                        if outside.is_empty() {
                            outside.push(Node::Text(String::new()));
                        }
                        outside.push(other);
                    }
                }
            }
        }
    }
    if !stack.is_empty() {
        return Err(XmlError("unclosed element at end of file".into()));
    }
    let root = root.ok_or_else(|| XmlError("no root element".into()))?;
    if !matches!(prolog.first(), Some(Node::Text(_))) {
        prolog.insert(0, Node::Text(String::new()));
    }
    Ok(Document {
        decl,
        prolog,
        root,
        epilog,
    })
}

/// Writes a document. A parsed document comes back as it was read, apart from what was changed;
/// elements StochCraft built are indented, unless they sit in parsed content, which keeps its own
/// whitespace as text nodes.
pub fn write(doc: &Document) -> String {
    let mut out = String::new();
    // A parsed document's prolog starts with the (possibly empty) text after its declaration.
    let parsed = matches!(doc.prolog.first(), Some(Node::Text(_)));
    match (&doc.decl, parsed) {
        (Some(d), _) => {
            out.push_str("<?xml ");
            out.push_str(d);
            out.push_str("?>");
        }
        (None, false) => out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
        (None, true) => {}
    }
    if parsed {
        for n in doc
            .prolog
            .iter()
            .chain(std::iter::once(&Node::Text(String::new())))
        {
            write_node(n, &mut out, None);
        }
        write_element(&doc.root, &mut out, Some(0));
        for n in &doc.epilog {
            write_node(n, &mut out, None);
        }
        return out;
    }
    out.push('\n');
    for n in &doc.prolog {
        write_node(n, &mut out, None);
        out.push('\n');
    }
    write_element(&doc.root, &mut out, Some(0));
    out.push('\n');
    for n in &doc.epilog {
        write_node(n, &mut out, None);
    }
    out
}

fn has_text(e: &Element) -> bool {
    e.children
        .iter()
        .any(|n| matches!(n, Node::Text(_) | Node::CData(_)))
}

/// `indent`: the depth to pretty-print at, or `None` inside parsed content that carries its own
/// whitespace.
fn write_element(e: &Element, out: &mut String, indent: Option<usize>) {
    out.push('<');
    let raw = e.raw.as_ref().filter(|r| r.name == e.name);
    let tag = raw.and_then(|r| {
        if r.attrs == e.attrs {
            Some(r.tag.clone())
        } else {
            patch_tag(&r.tag, &e.attrs)
        }
    });
    match &tag {
        Some(t) => out.push_str(t),
        None => {
            out.push_str(&e.name);
            for (n, v) in &e.attrs {
                out.push(' ');
                out.push_str(n);
                out.push_str("=\"");
                out.push_str(v);
                out.push('"');
            }
        }
    }
    let raw = raw.filter(|_| tag.is_some());
    if e.children.is_empty() && raw.is_none_or(|r| r.empty) {
        out.push_str("/>");
        return;
    }
    out.push('>');
    // Only elements built in memory are indented: parsed ones keep their own whitespace.
    let pretty = indent.filter(|_| e.raw.is_none() && !has_text(e));
    for c in &e.children {
        if let Some(d) = pretty {
            out.push('\n');
            out.push_str(&"  ".repeat(d + 1));
        }
        write_node(c, out, pretty.map(|d| d + 1));
    }
    if let Some(d) = pretty {
        out.push('\n');
        out.push_str(&"  ".repeat(d));
    }
    out.push_str("</");
    out.push_str(&e.name);
    out.push('>');
}

/// A parsed start tag with its attributes changed to `attrs`: the attributes that keep their
/// values keep their exact text and the whitespace before them (line breaks included), changed
/// ones get their new value, removed ones go and new ones are added at the end. `None` if the
/// tag cannot be taken apart.
fn patch_tag(tag: &str, attrs: &[(String, String)]) -> Option<String> {
    // (start of the whitespace before it, name, end of the attribute) for each attribute.
    let b = tag.as_bytes();
    let mut i = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
    let head_end = i;
    let mut found: Vec<(usize, &str, usize)> = Vec::new();
    loop {
        let ws = i;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let name_start = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let name = tag.get(name_start..i)?;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if b.get(i) != Some(&b'=') {
            return None;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let quote = *b.get(i)?;
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        let close = tag.get(i + 1..)?.find(char::from(quote))? + i + 1;
        i = close + 1;
        found.push((ws, name, i));
    }
    let tail = &tag[found.last().map_or(head_end, |f| f.2)..];
    let mut out = tag[..head_end].to_owned();
    for (n, v) in attrs {
        match found.iter().find(|f| f.1 == n) {
            Some(&(ws, name, end)) => {
                let text = &tag[ws..end];
                let unchanged = text
                    .split_once('=')
                    .map(|(_, q)| q.trim())
                    .and_then(|q| q.get(1..q.len().saturating_sub(1)))
                    .is_some_and(|old| old.replace('"', "&quot;") == *v);
                if unchanged {
                    out.push_str(text);
                } else {
                    let lead = &tag[ws..ws + (text.len() - text.trim_start().len())];
                    out.push_str(lead);
                    out.push_str(name);
                    out.push_str("=\"");
                    out.push_str(v);
                    out.push('"');
                }
            }
            None => {
                out.push(' ');
                out.push_str(n);
                out.push_str("=\"");
                out.push_str(v);
                out.push('"');
            }
        }
    }
    out.push_str(tail);
    Some(out)
}

fn write_node(n: &Node, out: &mut String, indent: Option<usize>) {
    match n {
        Node::Element(e) => write_element(e, out, indent),
        Node::Text(t) => out.push_str(t),
        Node::CData(t) => {
            out.push_str("<![CDATA[");
            out.push_str(t);
            out.push_str("]]>");
        }
        Node::Comment(t) => {
            out.push_str("<!--");
            out.push_str(t);
            out.push_str("-->");
        }
        Node::Pi(t) => {
            out.push_str("<?");
            out.push_str(t);
            out.push_str("?>");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- made by hand -->
<semantic:definitions xmlns:semantic="urn:m" xmlns:x="urn:x" id="d">
  <semantic:task id="t1" name="A &amp; B"><x:unknown a='say "hi"'>keep &lt;me&gt; &#169;</x:unknown></semantic:task>
  <![CDATA[raw <stuff>]]>
  <?tool keep this?>
  <inner xmlns="urn:default"><leaf/></inner>
</semantic:definitions>"#;

    #[test]
    fn namespaces_resolve_whatever_the_prefix() {
        let d = parse(SAMPLE.as_bytes()).unwrap_or_else(|e| unreachable!("{e}"));
        assert!(d.root.is("urn:m", "definitions"));
        let task = d.root.child(&["urn:m"], "task");
        assert_eq!(task.and_then(|t| t.attr("name")).as_deref(), Some("A & B"));
        let unknown = task.and_then(|t| t.child(&["urn:x"], "unknown"));
        assert_eq!(unknown.map(|u| u.text()).as_deref(), Some("keep <me> ©"));
        assert_eq!(
            unknown.and_then(|u| u.attr("a")).as_deref(),
            Some("say \"hi\"")
        );
        let leaf = d
            .root
            .child(&["urn:default"], "inner")
            .and_then(|i| i.child(&["urn:default"], "leaf"));
        assert!(leaf.is_some());
        assert_eq!(
            d.prolog,
            vec![
                Node::Text("\n".into()),
                Node::Comment(" made by hand ".into()),
                Node::Text("\n".into())
            ]
        );
    }

    #[test]
    fn writing_keeps_everything_and_reparses_to_the_same_tree() {
        let d = parse(SAMPLE.as_bytes()).unwrap_or_else(|e| unreachable!("{e}"));
        let out = write(&d);
        for kept in [
            "<!-- made by hand -->",
            "<![CDATA[raw <stuff>]]>",
            "<?tool keep this?>",
            "&#169;",
            "&amp;",
        ] {
            assert!(out.contains(kept), "{kept} lost in:\n{out}");
        }
        let again = parse(out.as_bytes()).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(again.root, d.root);
        // Byte for byte, quotes and empty-tag spelling included.
        assert_eq!(out, SAMPLE);
        for exact in [
            "<a x='1' />",
            "<a></a>",
            "<?xml version=\"1.0\"?><r\n  b = \"2\"/>\n\n",
            "<r/>",
            "<!-- c --><r/><!-- after -->",
        ] {
            let d = parse(exact.as_bytes()).unwrap_or_else(|e| unreachable!("{e}"));
            assert_eq!(write(&d), exact);
        }
    }

    #[test]
    fn changed_tags_are_written_afresh() {
        let mut d =
            parse(b"<r>\n  <a x='1' />\n  <b></b>\n</r>").unwrap_or_else(|e| unreachable!("{e}"));
        for e in d.root.elements_mut() {
            match e.name.as_str() {
                "a" => e.set_attr("y", "2"),
                _ => e.push(Element::new("c", None)),
            }
        }
        assert_eq!(write(&d), "<r>\n  <a x='1' y=\"2\" />\n  <b><c/></b>\n</r>");
        // Changed and removed attributes keep the layout of the rest of the tag.
        let mut d =
            parse(b"<r a='1'\n   b=\"2\"\n   c=\"3\"/>").unwrap_or_else(|e| unreachable!("{e}"));
        d.root.set_attr("b", "two");
        d.root.remove_attr("c");
        assert_eq!(write(&d), "<r a='1'\n   b=\"two\"/>");
    }

    #[test]
    fn built_elements_are_escaped_and_indented() {
        let root = Element::new("r", None)
            .with_attr("q", "a\"<b")
            .with_child(Element::new("c", None).with_text("x & y"));
        let out = write(&Document::new(root));
        assert_eq!(
            out,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<r q=\"a&quot;&lt;b\">\n  <c>x &amp; y</c>\n</r>\n"
        );
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        for bad in [
            "",
            "<a>",
            "<a></b>",
            "<a/><b/>",
            "text",
            "<a x='1' x='2'/>junk",
            "\u{0}",
        ] {
            let _ = parse(bad.as_bytes());
        }
        assert!(parse(b"<a>").is_err());
        assert!(parse(b"<a/><b/>").is_err());
        assert!(parse(&[0xff, 0xfe]).is_err());
    }
}
