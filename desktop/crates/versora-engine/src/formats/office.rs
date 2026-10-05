use super::{apply_edits, translatable, xml, Context, Document, Edit, Encoding, FormatError};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

const MAX_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;
#[derive(Debug)]
pub(super) struct OfficeDocument {
    original: Vec<u8>,
    parts: BTreeMap<String, (Vec<u8>, Vec<Edit>)>,
}

pub(super) fn extract(bytes: &[u8], extension: &str, game: bool) -> Result<Document, FormatError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| FormatError(format!("Cannot open .{extension} ZIP package: {e}")))?;
    let mut names = BTreeSet::new();
    let mut expanded = 0u64;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| FormatError(e.to_string()))?;
        if entry.enclosed_name().is_none() || !names.insert(entry.name().to_string()) {
            return Err(FormatError(
                "Office package contains an unsafe or duplicate member name".into(),
            ));
        }
        if entry.name().starts_with("_xmlsignatures/") {
            return Err(FormatError(
                "Signed Office packages require signature removal before translation".into(),
            ));
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| FormatError("Office package size overflow".into()))?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err(FormatError(
                "Office package exceeds the 256 MiB expanded safety limit".into(),
            ));
        }
    }
    if !names.contains("[Content_Types].xml") {
        return Err(FormatError(
            "Office package is missing [Content_Types].xml".into(),
        ));
    }
    let mut units = Vec::new();
    let mut parts = BTreeMap::new();
    if extension == "docx" {
        let name = "word/document.xml";
        let data = read_part(&mut archive, name)?;
        let nodes = xml::parse(&data)?;
        if nodes[0].local() != "document" {
            return Err(FormatError("DOCX main part has no document root".into()));
        }
        let mut edits = Vec::new();
        for (index, paragraph) in nodes.iter().enumerate().filter(|(_, n)| n.local() == "p") {
            if paragraph.empty {
                continue;
            }
            // A tab/break remains an original Word element and separates logical text blocks.
            // Nested table/text-box paragraphs are visited independently, never counted twice.
            let mut blocks = Vec::new();
            let mut current = Vec::new();
            collect_word_blocks(&nodes, index, &mut current, &mut blocks);
            if !current.is_empty() {
                blocks.push(current);
            }
            for texts in blocks {
                add_rich_text(&data, &nodes, &texts, game, true, &mut units, &mut edits)?;
            }
        }
        parts.insert(name.to_string(), (data, edits));
    } else {
        if !names.contains("xl/workbook.xml") {
            return Err(FormatError(
                "XLSX package is missing xl/workbook.xml".into(),
            ));
        }
        let mut used_shared = BTreeSet::new();
        for name in names
            .iter()
            .filter(|name| name.starts_with("xl/worksheets/") && name.ends_with(".xml"))
        {
            let data = read_part(&mut archive, name)?;
            let nodes = xml::parse(&data)?;
            let mut edits = Vec::new();
            for (index, cell) in nodes.iter().enumerate().filter(|(_, n)| n.local() == "c") {
                if cell
                    .children
                    .iter()
                    .any(|child| nodes[*child].local() == "f")
                {
                    continue;
                }
                match cell.attr("t") {
                    Some("s") => {
                        let value = cell
                            .children
                            .iter()
                            .find(|child| nodes[**child].local() == "v")
                            .ok_or_else(|| {
                                FormatError("Shared-string cell is missing its index".into())
                            })?;
                        let value = xml::text_of(&data, &nodes, *value)?;
                        let shared: usize = value
                            .trim()
                            .parse()
                            .map_err(|_| FormatError("Invalid XLSX shared-string index".into()))?;
                        used_shared.insert(shared);
                    }
                    Some("inlineStr") => {
                        let texts = descendant_text_nodes(&nodes, index);
                        add_rich_text(&data, &nodes, &texts, game, false, &mut units, &mut edits)?;
                    }
                    Some("str") => {
                        if let Some(value) = cell
                            .children
                            .iter()
                            .find(|child| nodes[**child].local() == "v")
                        {
                            add_rich_text(
                                &data,
                                &nodes,
                                &[*value],
                                game,
                                false,
                                &mut units,
                                &mut edits,
                            )?;
                        }
                    }
                    _ => {} // Numbers, booleans, errors, dates and formula result types are retained.
                }
            }
            if !edits.is_empty() {
                parts.insert(name.clone(), (data, edits));
            }
        }
        if !used_shared.is_empty() {
            let name = "xl/sharedStrings.xml";
            let data = read_part(&mut archive, name)?;
            let nodes = xml::parse(&data)?;
            let strings: Vec<_> = nodes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.local() == "si")
                .map(|(i, _)| i)
                .collect();
            let mut edits = Vec::new();
            for shared in used_shared {
                let index = strings.get(shared).ok_or_else(|| {
                    FormatError(format!("XLSX shared-string index {shared} is out of range"))
                })?;
                let texts = descendant_text_nodes(&nodes, *index);
                add_rich_text(&data, &nodes, &texts, game, false, &mut units, &mut edits)?;
            }
            if !edits.is_empty() {
                parts.insert(name.to_string(), (data, edits));
            }
        }
    }
    Ok(Document {
        units,
        context: Context::Office(OfficeDocument {
            original: bytes.to_vec(),
            parts,
        }),
    })
}
fn read_part(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, FormatError> {
    let mut entry = archive.by_name(name).map_err(|e| {
        FormatError(format!(
            "Office package is missing or cannot read {name}: {e}"
        ))
    })?;
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut data)
        .map_err(|e| FormatError(format!("Cannot decompress {name}: {e}")))?;
    Ok(data)
}
fn descendant_text_nodes(nodes: &[xml::Node], index: usize) -> Vec<usize> {
    let mut out = Vec::new();
    fn collect(nodes: &[xml::Node], index: usize, out: &mut Vec<usize>) {
        for child in &nodes[index].children {
            if nodes[*child].local() == "t" {
                out.push(*child);
            } else if nodes[*child].local() != "rPh" {
                collect(nodes, *child, out);
            } // Phonetic annotations are not duplicate visible content.
        }
    }
    collect(nodes, index, &mut out);
    out
}
fn collect_word_blocks(
    nodes: &[xml::Node],
    index: usize,
    current: &mut Vec<usize>,
    blocks: &mut Vec<Vec<usize>>,
) {
    for child in &nodes[index].children {
        match nodes[*child].local() {
            "p" => {}
            "t" => current.push(*child),
            "tab" | "br" | "cr" => {
                if !current.is_empty() {
                    blocks.push(std::mem::take(current));
                }
            }
            _ => collect_word_blocks(nodes, *child, current, blocks),
        }
    }
}
fn add_rich_text(
    data: &[u8],
    nodes: &[xml::Node],
    texts: &[usize],
    game: bool,
    word: bool,
    units: &mut Vec<String>,
    edits: &mut Vec<Edit>,
) -> Result<(), FormatError> {
    let mut original = String::new();
    for index in texts {
        original.push_str(&xml::text_of(data, nodes, *index)?);
    }
    if if word && !game {
        original.trim().is_empty()
    } else {
        !translatable(&original, game)
    } {
        return Ok(());
    }
    let unit = units.len();
    units.push(original);
    for (position, index) in texts.iter().enumerate() {
        let node = &nodes[*index];
        if node.empty {
            return Err(FormatError(
                "Cannot replace a self-closing Office text element".into(),
            ));
        }
        if position == 0 {
            let mut opening =
                std::str::from_utf8(&data[node.full.start..node.content.start])?.to_string();
            let space = regex::Regex::new(r#"\bxml:space\s*=\s*(?:"[^"]*"|'[^']*')"#).unwrap();
            if space.is_match(&opening) {
                opening = space
                    .replace(&opening, "xml:space=\"preserve\"")
                    .into_owned();
            } else {
                opening.insert_str(opening.len() - 1, " xml:space=\"preserve\"");
            }
            let closing = std::str::from_utf8(&data[node.content.end..node.full.end])?.to_string();
            let prefix = if word {
                Some(
                    node.name
                        .rsplit_once(':')
                        .map(|(p, _)| format!("{p}:"))
                        .unwrap_or_default(),
                )
            } else {
                None
            };
            edits.push(Edit {
                range: node.full.clone(),
                unit,
                encoding: Encoding::OfficeText {
                    opening,
                    closing,
                    word_prefix: prefix,
                },
            });
        } else {
            edits.push(Edit {
                range: node.content.clone(),
                unit,
                encoding: Encoding::Empty,
            });
        }
    }
    Ok(())
}

pub(super) fn encode_text(
    text: &str,
    opening: &str,
    closing: &str,
    word_prefix: Option<&str>,
) -> Result<String, FormatError> {
    xml::escape_checked(text)?;
    let mut out = opening.to_string();
    if let Some(prefix) = word_prefix {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut chunk = String::new();
        for c in normalized.chars() {
            if c == '\n' || c == '\t' {
                out.push_str(&xml::escape(&chunk));
                chunk.clear();
                out.push_str(closing);
                out.push_str(&format!(
                    "<{prefix}{}/>",
                    if c == '\n' { "br" } else { "tab" }
                ));
                out.push_str(opening);
            } else {
                chunk.push(c);
            }
        }
        out.push_str(&xml::escape(&chunk));
    } else {
        out.push_str(&xml::escape(text));
    }
    out.push_str(closing);
    Ok(out)
}

pub(super) fn write(
    document: &OfficeDocument,
    units: &[String],
    translations: &[String],
) -> Result<Vec<u8>, FormatError> {
    if units == translations {
        return Ok(document.original.clone());
    }
    let mut archive = ZipArchive::new(Cursor::new(document.original.as_slice()))
        .map_err(|e| FormatError(e.to_string()))?;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer.set_comment(
        std::str::from_utf8(archive.comment())
            .map_err(|e| FormatError(e.to_string()))?
            .to_string(),
    );
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| FormatError(e.to_string()))?;
        if let Some((bytes, edits)) = document.parts.get(entry.name()) {
            let translated = apply_edits(bytes, edits, units, translations)?;
            // Validates the modified XML before returning any output.
            xml::parse(&translated)?;
            let mut options = SimpleFileOptions::default().compression_method(entry.compression());
            if let Some(time) = entry.last_modified() {
                options = options.last_modified_time(time);
            }
            if let Some(mode) = entry.unix_mode() {
                options = options.unix_permissions(mode);
            }
            writer
                .start_file(entry.name(), options)
                .map_err(|e| FormatError(e.to_string()))?;
            writer
                .write_all(&translated)
                .map_err(|e| FormatError(e.to_string()))?;
        } else {
            writer
                .raw_copy_file(entry)
                .map_err(|e| FormatError(e.to_string()))?;
        }
    }
    Ok(writer
        .finish()
        .map_err(|e| FormatError(e.to_string()))?
        .into_inner())
}
