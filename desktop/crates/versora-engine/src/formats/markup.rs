use super::{edits_document, translatable, xml, Document, Edit, Encoding, FormatError};

/// HTML is edited at text nodes. Tags, attributes, comments and raw script/style text remain bytes-identical.
pub(super) fn html(bytes: &[u8], game: bool) -> Result<Document, FormatError> {
    let raw = std::str::from_utf8(bytes)?;
    let mut units = Vec::new();
    let mut edits = Vec::new();
    let mut pos = 0;
    let mut skip: Option<String> = None;
    while pos < raw.len() {
        if raw[pos..].starts_with("<!--") {
            let end = raw[pos + 4..]
                .find("-->")
                .ok_or_else(|| FormatError("Unclosed HTML comment".into()))?
                + pos
                + 7;
            pos = end;
            continue;
        }
        if raw.as_bytes()[pos] == b'<' {
            let end = tag_end(raw, pos)?;
            let tag = raw[pos + 1..end - 1].trim().to_ascii_lowercase();
            let name = tag
                .trim_start_matches('/')
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("");
            if tag.starts_with('/') {
                if skip.as_deref() == Some(name) {
                    skip = None;
                }
            } else if name == "script" || name == "style" {
                skip = Some(name.to_string());
            }
            pos = end;
            continue;
        }
        let end = if let Some(name) = &skip {
            let lower = raw[pos..].to_ascii_lowercase();
            lower
                .find(&format!("</{name}"))
                .map(|p| pos + p)
                .unwrap_or(raw.len())
        } else {
            raw[pos..].find('<').map(|p| pos + p).unwrap_or(raw.len())
        };
        if skip.is_none() {
            let segment = &raw[pos..end];
            let decoded = html_unescape(segment);
            if translatable(&decoded, game) {
                let index = units.len();
                units.push(decoded);
                edits.push(Edit {
                    range: pos..end,
                    unit: index,
                    encoding: Encoding::Html,
                });
            }
        }
        if end == pos {
            return Err(FormatError("HTML scanner made no progress".into()));
        }
        pos = end;
    }
    Ok(edits_document(bytes, units, edits))
}
fn tag_end(raw: &str, start: usize) -> Result<usize, FormatError> {
    let mut quote = 0;
    for (offset, b) in raw.as_bytes()[start + 1..].iter().enumerate() {
        if quote != 0 {
            if *b == quote {
                quote = 0;
            }
        } else if *b == b'\'' || *b == b'"' {
            quote = *b;
        } else if *b == b'>' {
            return Ok(start + offset + 2);
        }
    }
    Err(FormatError("Unclosed HTML tag".into()))
}
fn html_unescape(text: &str) -> String {
    // web_atoms supplies the WHATWG named-entity table, including two-codepoint entities.
    // Bare ampersands and unknown entity spellings retain their visible literal text.
    let re = regex::Regex::new(r"&(?:[A-Za-z][A-Za-z0-9]+|#\d+|#x[0-9A-Fa-f]+);").unwrap();
    re.replace_all(text, |caps: &regex::Captures<'_>| {
        if let Some(&(first, second)) = web_atoms::NAMED_ENTITIES.get(&caps[0][1..]) {
            if first != 0 {
                let mut decoded = char::from_u32(first).unwrap().to_string();
                if second != 0 {
                    decoded.push(char::from_u32(second).unwrap());
                }
                return decoded;
            }
        }
        quick_xml::escape::unescape(&caps[0])
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| caps[0].to_string())
    })
    .into_owned()
}

/// XLIFF 1.x trans-units and 2.x segments retain inline code through numbered tokens.
/// Dropping/duplicating a token fails writeback instead of corrupting the localization file.
pub(super) fn xliff(bytes: &[u8], game: bool) -> Result<Document, FormatError> {
    let nodes = xml::parse(bytes)?;
    if nodes[0].local() != "xliff" {
        return Err(FormatError("Expected an XLIFF root element".into()));
    }
    let mut units = Vec::new();
    let mut edits = Vec::new();
    for (index, source) in nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.local() == "source")
    {
        let Some(parent) = source.parent else {
            continue;
        };
        let (text, codes) = inline_unit(bytes, &nodes, index)?;
        if !translatable(&xml::text_of(bytes, &nodes, index)?, game) {
            continue;
        }
        let unit = units.len();
        units.push(text);
        let target = nodes[parent]
            .children
            .iter()
            .find(|child| nodes[**child].local() == "target");
        let (range, wrapper) = if let Some(target) = target {
            let target = &nodes[*target];
            if target.empty {
                let raw = std::str::from_utf8(&bytes[target.full.clone()])?;
                let opening = format!(
                    "{}>",
                    raw.strip_suffix("/>")
                        .ok_or_else(|| FormatError("Invalid self-closing XLIFF target".into()))?
                );
                (
                    target.full.clone(),
                    Some((opening, format!("</{}>", target.name))),
                )
            } else {
                (target.content.clone(), None)
            }
        } else {
            let prefix = source
                .name
                .rsplit_once(':')
                .map(|(p, _)| format!("{p}:target"))
                .unwrap_or_else(|| "target".into());
            (
                source.full.end..source.full.end,
                Some((format!("<{prefix}>"), format!("</{prefix}>"))),
            )
        };
        edits.push(Edit {
            range,
            unit,
            encoding: Encoding::Xliff { codes, wrapper },
        });
    }
    Ok(edits_document(bytes, units, edits))
}
fn inline_unit(
    bytes: &[u8],
    nodes: &[xml::Node],
    index: usize,
) -> Result<(String, Vec<(String, String)>), FormatError> {
    let source = &nodes[index];
    let mut ranges = Vec::new();
    fn collect(nodes: &[xml::Node], index: usize, ranges: &mut Vec<std::ops::Range<usize>>) {
        let node = &nodes[index];
        if node.empty
            || matches!(
                node.local(),
                "x" | "ph" | "bx" | "ex" | "bpt" | "ept" | "it" | "sc" | "ec"
            )
        {
            ranges.push(node.full.clone());
            return;
        }
        ranges.push(node.full.start..node.content.start);
        for child in &node.children {
            collect(nodes, *child, ranges);
        }
        ranges.push(node.content.end..node.full.end);
    }
    for child in &source.children {
        collect(nodes, *child, &mut ranges);
    }
    ranges.sort_by_key(|r| r.start);
    let mut out = String::new();
    let mut codes = Vec::new();
    let mut pos = source.content.start;
    for range in ranges {
        out.push_str(&decode_inline_text(&bytes[pos..range.start])?);
        let token = format!("⟦XLIFF:{}⟧", codes.len());
        out.push_str(&token);
        codes.push((
            token,
            std::str::from_utf8(&bytes[range.clone()])?.to_string(),
        ));
        pos = range.end;
    }
    out.push_str(&decode_inline_text(&bytes[pos..source.content.end])?);
    Ok((out, codes))
}
fn decode_inline_text(bytes: &[u8]) -> Result<String, FormatError> {
    use quick_xml::{events::Event, Reader};
    let mut reader = Reader::from_reader(bytes);
    let mut out = String::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| FormatError(e.to_string()))?
        {
            Event::Text(text) => out.push_str(&xml::unescape(std::str::from_utf8(text.as_ref())?)?),
            Event::CData(text) => out.push_str(std::str::from_utf8(text.as_ref())?),
            Event::GeneralRef(name) => out.push_str(&xml::unescape(&format!(
                "&{};",
                std::str::from_utf8(name.as_ref())?
            ))?),
            Event::Comment(_) => {}
            Event::Eof => break,
            _ => {
                return Err(FormatError(
                    "Unexpected markup inside XLIFF text segment".into(),
                ))
            }
        }
    }
    Ok(out)
}
pub(super) fn encode_xliff(text: &str, codes: &[(String, String)]) -> Result<String, FormatError> {
    xml::escape_checked(text)?;
    let mut positions = Vec::new();
    for (token, raw) in codes {
        let found: Vec<_> = text.match_indices(token).collect();
        if found.len() != 1 {
            return Err(FormatError(format!(
                "XLIFF inline token {token} must occur exactly once"
            )));
        }
        positions.push((found[0].0, token.len(), raw));
    }
    positions.sort_by_key(|(pos, _, _)| *pos);
    let mut result = String::new();
    let mut pos = 0;
    for (start, length, raw) in positions {
        result.push_str(&xml::escape(&text[pos..start]));
        result.push_str(raw);
        pos = start + length;
    }
    result.push_str(&xml::escape(&text[pos..]));
    // Use the original namespaced declarations when validating paired tags: a slice Reader checks names,
    // and does not require namespace declarations for well-formedness.
    xml::parse(format!("<versora-inline>{result}</versora-inline>").as_bytes())?;
    Ok(result)
}
