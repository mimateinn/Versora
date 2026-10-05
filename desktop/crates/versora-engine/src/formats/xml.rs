use super::FormatError;
use quick_xml::{events::Event, Reader};
use std::{collections::BTreeMap, ops::Range};

#[derive(Debug)]
pub(super) struct Node {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub full: Range<usize>,
    pub content: Range<usize>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub text: Vec<Range<usize>>,
    pub empty: bool,
}
impl Node {
    pub fn local(&self) -> &str {
        self.name.rsplit(':').next().unwrap_or(&self.name)
    }
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k.rsplit(':').next() == Some(name))
            .map(|(_, v)| v.as_str())
    }
}

pub(super) fn parse(bytes: &[u8]) -> Result<Vec<Node>, FormatError> {
    std::str::from_utf8(bytes)?;
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut nodes: Vec<Node> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader
            .read_event()
            .map_err(|e| FormatError(format!("Malformed XML: {e}")))?;
        let end = reader.buffer_position() as usize;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                if stack.len() >= 128 || nodes.len() >= 1_000_000 {
                    return Err(FormatError(
                        "XML exceeds the supported nesting or element count limit".into(),
                    ));
                }
                let empty = matches!(event, Event::Empty(_));
                let name = std::str::from_utf8(e.name().as_ref())?.to_string();
                let mut attributes = BTreeMap::new();
                for attr in e.attributes() {
                    let attr =
                        attr.map_err(|e| FormatError(format!("Malformed XML attribute: {e}")))?;
                    let key = std::str::from_utf8(attr.key.as_ref())?.to_string();
                    let value = attr
                        .decoded_and_normalized_value_with(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                            1,
                            quick_xml::escape::resolve_xml_entity,
                        )
                        .map_err(|e| FormatError(e.to_string()))?
                        .into_owned();
                    attributes.insert(key, value);
                }
                let parent = stack.last().copied();
                let index = nodes.len();
                nodes.push(Node {
                    name,
                    attributes,
                    full: start..end,
                    content: end..end,
                    parent,
                    children: Vec::new(),
                    text: Vec::new(),
                    empty,
                });
                if let Some(parent) = parent {
                    nodes[parent].children.push(index);
                }
                if !empty {
                    stack.push(index);
                }
            }
            Event::End(_) => {
                let index = stack.pop().ok_or_else(|| {
                    FormatError("XML contains an unmatched closing element".into())
                })?;
                nodes[index].content.end = start;
                nodes[index].full.end = end;
            }
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) => {
                if let Some(index) = stack.last() {
                    nodes[*index].text.push(start..end);
                } else if !std::str::from_utf8(&bytes[start..end])?.trim().is_empty() {
                    return Err(FormatError(
                        "XML text must be inside the root element".into(),
                    ));
                }
            }
            Event::Decl(declaration) => {
                let version = declaration
                    .version()
                    .map_err(|e| FormatError(e.to_string()))?;
                if version.as_ref() != b"1.0" {
                    return Err(FormatError("Only XML 1.0 is supported".into()));
                }
                if let Some(encoding) = declaration.encoding() {
                    let encoding = encoding.map_err(|e| FormatError(e.to_string()))?;
                    if !encoding.as_ref().eq_ignore_ascii_case(b"UTF-8") {
                        return Err(FormatError("XML input must be UTF-8 encoded".into()));
                    }
                }
            }
            Event::DocType(_) => {
                return Err(FormatError(
                    "XML documents with a DTD are not supported".into(),
                ))
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err(FormatError("XML document ended inside an element".into()));
    }
    if nodes.is_empty() {
        return Err(FormatError("XML document contains no root element".into()));
    }
    if nodes.iter().filter(|node| node.parent.is_none()).count() != 1 {
        return Err(FormatError("XML must have one root element".into()));
    }
    Ok(nodes)
}

pub(super) fn text_ranges(nodes: &[Node], index: usize) -> Vec<Range<usize>> {
    let mut ranges = nodes[index].text.clone();
    for child in &nodes[index].children {
        ranges.extend(text_ranges(nodes, *child));
    }
    ranges.sort_by_key(|r| r.start);
    ranges
}
pub(super) fn text_of(bytes: &[u8], nodes: &[Node], index: usize) -> Result<String, FormatError> {
    let mut out = String::new();
    for range in text_ranges(nodes, index) {
        let raw = std::str::from_utf8(&bytes[range])?;
        if raw.starts_with("<![CDATA[") && raw.ends_with("]]>") {
            out.push_str(&raw[9..raw.len() - 3]);
        } else {
            out.push_str(&unescape(raw)?);
        }
    }
    Ok(out)
}
pub(super) fn unescape(text: &str) -> Result<String, FormatError> {
    quick_xml::escape::unescape_with(text, quick_xml::escape::resolve_xml_entity)
        .map(|s| s.into_owned())
        .map_err(|e| FormatError(format!("Invalid XML entity: {e}")))
}
pub(super) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub(super) fn escape_checked(text: &str) -> Result<String, FormatError> {
    if text.chars().any(|c| !matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')) {
        return Err(FormatError("Translation contains a character that XML 1.0 cannot represent".into()));
    }
    Ok(escape(text))
}
