//! Minimal namespace-aware XML tree used to edit OOXML/ODF parts.
//!
//! Text and attribute values are kept in their original escaped form so untouched content
//! round-trips byte-for-byte (apart from `<a></a>` → `<a/>`); only the nodes the cleaner
//! edits are re-escaped.

use quick_xml::events::Event;
use quick_xml::Reader;

pub const NS_XML: &str = "http://www.w3.org/XML/1998/namespace";
pub const NS_XMLNS: &str = "http://www.w3.org/2000/xmlns/";

#[derive(Debug, Clone)]
pub enum Node {
    Element(Element),
    /// Escaped character data.
    Text(String),
    /// Comments, processing instructions, declarations and CDATA, verbatim.
    Raw(String),
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    pub ns: String,
    pub local: String,
    /// Escaped value without quotes.
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub name: String,
    pub ns: String,
    pub local: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct XmlDoc {
    pub bom: bool,
    pub nodes: Vec<Node>,
}

fn split_name(name: &str) -> (Option<&str>, &str) {
    match name.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, name),
    }
}

fn lookup(scopes: &[Vec<(String, String)>], prefix: &str) -> String {
    if prefix == "xml" {
        return NS_XML.to_string();
    }
    for scope in scopes.iter().rev() {
        if let Some((_, uri)) = scope.iter().rev().find(|(p, _)| p == prefix) {
            return uri.clone();
        }
    }
    String::new()
}

fn open_element(raw: &quick_xml::events::BytesStart, scopes: &mut Vec<Vec<(String, String)>>) -> Result<Element, String> {
    let name = String::from_utf8_lossy(raw.name().as_ref()).to_string();
    let mut decls = Vec::new();
    let mut attrs = Vec::new();
    for a in raw.attributes().with_checks(false) {
        let a = a.map_err(|e| format!("XML attribute error: {}", e))?;
        let key = String::from_utf8_lossy(a.key.as_ref()).to_string();
        let value = String::from_utf8_lossy(&a.value).to_string();
        if key == "xmlns" {
            decls.push((String::new(), unescape(&value)));
        } else if let Some(p) = key.strip_prefix("xmlns:") {
            decls.push((p.to_string(), unescape(&value)));
        }
        attrs.push((key, value));
    }
    scopes.push(decls);
    let (prefix, local) = split_name(&name);
    let ns = lookup(scopes, prefix.unwrap_or(""));
    let attrs = attrs
        .into_iter()
        .map(|(key, value)| {
            let (prefix, local) = split_name(&key);
            let ns = match prefix {
                _ if key == "xmlns" => NS_XMLNS.to_string(),
                Some("xmlns") => NS_XMLNS.to_string(),
                Some(p) => lookup(scopes, p),
                None => String::new(),
            };
            Attr { local: local.to_string(), name: key, ns, value }
        })
        .collect();
    Ok(Element { local: local.to_string(), name, ns, attrs, children: Vec::new() })
}

pub fn unescape(raw: &str) -> String {
    quick_xml::escape::unescape(raw).map(|c| c.into_owned()).unwrap_or_else(|_| raw.to_string())
}

pub fn escape(text: &str) -> String {
    quick_xml::escape::escape(text).into_owned()
}

impl XmlDoc {
    pub fn parse(data: &[u8]) -> Result<XmlDoc, String> {
        let (bom, body) = match data.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
            Some(rest) => (true, rest),
            None => (false, data),
        };
        let text = std::str::from_utf8(body).map_err(|_| "XML part is not UTF-8".to_string())?;
        let mut reader = Reader::from_str(text);
        reader.config_mut().trim_text(false);
        reader.config_mut().check_end_names = true;
        reader.config_mut().expand_empty_elements = false;
        let mut top: Vec<Node> = Vec::new();
        let mut stack: Vec<Element> = Vec::new();
        let mut scopes: Vec<Vec<(String, String)>> = Vec::new();
        fn push(stack: &mut [Element], top: &mut Vec<Node>, node: Node) {
            match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => top.push(node),
            }
        }
        loop {
            let event = reader.read_event().map_err(|e| format!("XML parse error at {}: {}", reader.buffer_position(), e))?;
            match event {
                Event::Start(e) => {
                    let el = open_element(&e, &mut scopes)?;
                    stack.push(el);
                }
                Event::Empty(e) => {
                    let el = open_element(&e, &mut scopes)?;
                    scopes.pop();
                    push(&mut stack, &mut top, Node::Element(el));
                }
                Event::End(_) => {
                    let el = stack.pop().ok_or("XML parse error: unexpected end tag")?;
                    scopes.pop();
                    push(&mut stack, &mut top, Node::Element(el));
                }
                Event::Text(e) => push(&mut stack, &mut top, Node::Text(String::from_utf8_lossy(&e).to_string())),
                Event::CData(e) => push(&mut stack, &mut top, Node::Raw(format!("<![CDATA[{}]]>", String::from_utf8_lossy(&e)))),
                Event::Comment(e) => push(&mut stack, &mut top, Node::Raw(format!("<!--{}-->", String::from_utf8_lossy(&e)))),
                Event::Decl(e) => push(&mut stack, &mut top, Node::Raw(format!("<?{}?>", String::from_utf8_lossy(&e)))),
                Event::PI(e) => push(&mut stack, &mut top, Node::Raw(format!("<?{}?>", String::from_utf8_lossy(&e)))),
                Event::DocType(e) => push(&mut stack, &mut top, Node::Raw(format!("<!DOCTYPE {}>", String::from_utf8_lossy(&e).trim_start()))),
                Event::Eof => break,
            }
        }
        if !stack.is_empty() {
            return Err("XML parse error: unclosed element".to_string());
        }
        if !top.iter().any(|n| matches!(n, Node::Element(_))) {
            return Err("XML parse error: no root element".to_string());
        }
        Ok(XmlDoc { bom, nodes: top })
    }

    pub fn root(&self) -> &Element {
        self.nodes
            .iter()
            .find_map(|n| match n {
                Node::Element(e) => Some(e),
                _ => None,
            })
            .expect("parsed XML has a root element")
    }

    pub fn root_mut(&mut self) -> &mut Element {
        self.nodes
            .iter_mut()
            .find_map(|n| match n {
                Node::Element(e) => Some(e),
                _ => None,
            })
            .expect("parsed XML has a root element")
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        for n in &self.nodes {
            write_node(n, &mut out);
        }
        let mut bytes = Vec::with_capacity(out.len() + 3);
        if self.bom {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        bytes.extend_from_slice(out.as_bytes());
        bytes
    }
}

fn write_node(node: &Node, out: &mut String) {
    match node {
        Node::Text(t) | Node::Raw(t) => out.push_str(t),
        Node::Element(e) => {
            out.push('<');
            out.push_str(&e.name);
            for a in &e.attrs {
                let quote = if a.value.contains('"') { '\'' } else { '"' };
                out.push(' ');
                out.push_str(&a.name);
                out.push('=');
                out.push(quote);
                out.push_str(&a.value);
                out.push(quote);
            }
            if e.children.is_empty() {
                out.push_str("/>");
            } else {
                out.push('>');
                for c in &e.children {
                    write_node(c, out);
                }
                out.push_str("</");
                out.push_str(&e.name);
                out.push('>');
            }
        }
    }
}

impl Element {
    pub fn is(&self, ns: &str, local: &str) -> bool {
        self.local == local && self.ns == ns
    }

    pub fn attr(&self, ns: &str, local: &str) -> Option<String> {
        self.attrs.iter().find(|a| a.ns == ns && a.local == local).map(|a| unescape(&a.value))
    }

    pub fn set_attr(&mut self, ns: &str, local: &str, value: &str) {
        if let Some(a) = self.attrs.iter_mut().find(|a| a.ns == ns && a.local == local) {
            a.value = escape(value);
        }
    }

    pub fn remove_attr(&mut self, ns: &str, local: &str) -> bool {
        let before = self.attrs.len();
        self.attrs.retain(|a| !(a.ns == ns && a.local == local));
        before != self.attrs.len()
    }

    /// Unescaped concatenation of all descendant text.
    pub fn text(&self) -> String {
        let mut out = String::new();
        fn collect(e: &Element, out: &mut String) {
            for c in &e.children {
                match c {
                    Node::Text(t) => out.push_str(&unescape(t)),
                    Node::Raw(r) if r.starts_with("<![CDATA[") => out.push_str(&r[9..r.len() - 3]),
                    Node::Element(child) => collect(child, out),
                    _ => {}
                }
            }
        }
        collect(self, &mut out);
        out
    }

    pub fn set_text(&mut self, text: &str) {
        self.children = vec![Node::Text(escape(text))];
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }

    /// Every descendant element (depth-first), excluding `self`.
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

    /// Visits `self` and every descendant element mutably.
    pub fn walk_mut(&mut self, f: &mut dyn FnMut(&mut Element)) {
        f(self);
        for c in self.children.iter_mut() {
            if let Node::Element(e) = c {
                e.walk_mut(f);
            }
        }
    }

    /// Removes every descendant element matching `pred` (with its subtree); returns the count.
    pub fn remove_where(&mut self, pred: &dyn Fn(&Element) -> bool) -> usize {
        let mut removed = 0;
        self.children.retain(|n| match n {
            Node::Element(e) if pred(e) => {
                removed += 1;
                false
            }
            _ => true,
        });
        for c in self.children.iter_mut() {
            if let Node::Element(e) = c {
                removed += e.remove_where(pred);
            }
        }
        removed
    }

    /// Removes descendant elements matching `wrapper` that no longer contain any element
    /// (only whitespace), bottom-up, so emptied extension lists and alternate-content blocks
    /// disappear with the reference they carried.
    pub fn prune_empty(&mut self, wrapper: &dyn Fn(&Element) -> bool) {
        for c in self.children.iter_mut() {
            if let Node::Element(e) = c {
                e.prune_empty(wrapper);
            }
        }
        self.children.retain(|n| match n {
            Node::Element(e) if wrapper(e) => e.elements().next().is_some(),
            _ => true,
        });
    }

    /// New element named `prefix:local` (or `local` for an empty prefix) in namespace `ns`.
    pub fn new(prefix: &str, local: &str, ns: &str) -> Element {
        Element {
            name: if prefix.is_empty() { local.to_string() } else { format!("{}:{}", prefix, local) },
            ns: ns.to_string(),
            local: local.to_string(),
            attrs: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Prefix of this element's qualified name (`""` for the default namespace).
    pub fn prefix(&self) -> &str {
        self.name.split_once(':').map(|(p, _)| p).unwrap_or("")
    }

    /// Adds an unprefixed attribute (or a prefixed one whose namespace is already declared).
    pub fn push_attr(&mut self, name: &str, ns: &str, value: &str) {
        let local = name.split_once(':').map(|(_, l)| l).unwrap_or(name);
        self.attrs.push(Attr { name: name.to_string(), ns: ns.to_string(), local: local.to_string(), value: escape(value) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_resolves_namespaces() {
        let src = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<w:document xmlns:w=\"urn:w\" xmlns=\"urn:d\"><w:p w:rsidR=\"00AB\" a='x\"y'><!--c--><w:t xml:space=\"preserve\">A &amp; B</w:t><x/></w:p></w:document>";
        let doc = XmlDoc::parse(src.as_bytes()).unwrap();
        assert_eq!(String::from_utf8(doc.to_bytes()).unwrap(), src);
        let root = doc.root();
        assert!(root.is("urn:w", "document"));
        let p = root.elements().next().unwrap();
        assert_eq!(p.attr("urn:w", "rsidR").as_deref(), Some("00AB"));
        assert_eq!(p.attr("", "a").as_deref(), Some("x\"y"));
        assert_eq!(p.text(), "A & B");
        assert!(p.elements().nth(1).unwrap().is("urn:d", "x"));
        assert_eq!(p.elements().next().unwrap().attrs[0].ns, NS_XML);
    }

    #[test]
    fn remove_and_prune() {
        let src = "<r xmlns:p=\"urn:p\"><p:extLst><p:ext uri=\"1\"><p:ref id=\"x\"/></p:ext></p:extLst><keep/></r>";
        let mut doc = XmlDoc::parse(src.as_bytes()).unwrap();
        let root = doc.root_mut();
        assert_eq!(root.remove_where(&|e| e.local == "ref"), 1);
        root.prune_empty(&|e| e.local == "ext" || e.local == "extLst");
        assert_eq!(String::from_utf8(doc.to_bytes()).unwrap(), "<r xmlns:p=\"urn:p\"><keep/></r>");
    }
}
