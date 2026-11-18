//! A bounded XML element tree read with `quick-xml`. The DOCTYPE is
//! skipped and never resolved (no DTD, no external or general entity:
//! an entity reference refuses the file), comments and processing
//! instructions are ignored, and depth and element count are capped.

use std::collections::BTreeMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::ImportError;

const MAX_DEPTH: usize = 64;
const MAX_ELEMENTS: usize = 100_000;

/// One element: its name, attributes, own text (trimmed) and children.
#[derive(Debug, Default)]
pub struct Node {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub text: String,
    pub children: Vec<Node>,
}

impl Node {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.get(key).map(String::as_str)
    }

    /// `key`'s value, matched exactly, else ASCII-case-insensitively: the
    /// Xfire client reads `name=`/`Name=` and `state=`/`State=` alike (Classic
    /// v3 writes both in one file, `xfirecomponents.xml`).
    pub fn attr_ci(&self, key: &str) -> Option<&str> {
        self.attr(key).or_else(|| {
            self.attrs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_str())
        })
    }

    /// Every descendant (self excluded) named `name`, in document order.
    pub fn descendants<'a>(&'a self, name: &str, out: &mut Vec<&'a Node>) {
        for c in &self.children {
            if c.name == name {
                out.push(c);
            }
            c.descendants(name, out);
        }
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
}

fn xml_err(file: &str, detail: impl std::fmt::Display) -> ImportError {
    ImportError::Xml {
        file: file.to_owned(),
        detail: detail.to_string(),
    }
}

fn open(file: &str, e: &BytesStart<'_>) -> Result<Node, ImportError> {
    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let mut attrs = BTreeMap::new();
    for a in e.attributes() {
        let a = a.map_err(|x| xml_err(file, x))?;
        let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let value = a.unescape_value().map_err(|x| xml_err(file, x))?;
        attrs.insert(key, value.into_owned());
    }
    Ok(Node {
        name,
        attrs,
        ..Node::default()
    })
}

/// Parses `bytes` (UTF-8) into its root element.
pub fn parse(file: &str, bytes: &[u8]) -> Result<Node, ImportError> {
    let text = std::str::from_utf8(bytes).map_err(|e| xml_err(file, e))?;
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut count = 0usize;
    loop {
        match reader.read_event().map_err(|e| xml_err(file, e))? {
            Event::Start(e) => {
                count += 1;
                if stack.len() > MAX_DEPTH || count > MAX_ELEMENTS {
                    return Err(xml_err(file, "too deep or too many elements"));
                }
                stack.push(open(file, &e)?);
            }
            Event::Empty(e) => {
                count += 1;
                let node = open(file, &e)?;
                stack
                    .last_mut()
                    .ok_or_else(|| xml_err(file, "unbalanced"))?
                    .children
                    .push(node);
            }
            Event::End(_) => {
                let node = stack.pop().ok_or_else(|| xml_err(file, "unbalanced"))?;
                stack
                    .last_mut()
                    .ok_or_else(|| xml_err(file, "unbalanced"))?
                    .children
                    .push(node);
            }
            Event::Text(t) => {
                let s = t.decode().map_err(|e| xml_err(file, e))?;
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(s.trim());
                }
            }
            Event::CData(t) => {
                let s = t.decode().map_err(|e| xml_err(file, e))?;
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(s.trim());
                }
            }
            Event::GeneralRef(_) => return Err(xml_err(file, "entity references are refused")),
            Event::Eof => break,
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::DocType(_) => {}
        }
    }
    let mut doc = stack.pop().filter(|_| stack.is_empty());
    let root = doc
        .as_mut()
        .and_then(|d| (d.children.len() == 1).then(|| d.children.remove(0)));
    root.ok_or_else(|| xml_err(file, "not one root element"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_refuses_entities() {
        let n = parse(
            "t",
            b"<?xml version=\"1.0\"?><!DOCTYPE A SYSTEM \"a.dtd\"><A x=\"1\"><B> hi </B><C/></A>",
        )
        .unwrap();
        assert_eq!(n.name, "A");
        assert_eq!(n.attr("x"), Some("1"));
        assert_eq!(n.attr_ci("X"), Some("1"));
        assert_eq!(n.attr("X"), None);
        assert_eq!(n.children[0].text, "hi");
        assert!(parse("t", b"<A>&ext;</A>").is_err());
        assert!(parse("t", b"<A></A><B/>").is_err());
    }
}
