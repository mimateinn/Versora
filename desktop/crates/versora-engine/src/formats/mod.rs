//! Byte-based document extraction and writeback. No document parser performs I/O.
//! Untouched content is copied verbatim; translation counts are checked before writing.
mod markup;
mod office;
mod pdf;
mod tabular;
mod text;
mod xml;
mod yaml;

use std::{fmt, ops::Range, sync::OnceLock};

const MAX_OUTPUT_BYTES: usize = 80_000_000;

#[derive(Debug)]
pub struct FormatError(pub String);
impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for FormatError {}
impl From<std::str::Utf8Error> for FormatError {
    fn from(e: std::str::Utf8Error) -> Self {
        Self(format!("Document must be UTF-8: {e}"))
    }
}

#[derive(Debug)]
pub struct Document {
    pub units: Vec<String>,
    context: Context,
}
#[derive(Debug)]
enum Context {
    Edits { original: Vec<u8>, edits: Vec<Edit> },
    Office(office::OfficeDocument),
    Pdf(pdf::PdfDocument),
    Yaml(yaml::YamlDocument),
}
#[derive(Debug, Clone)]
pub(super) struct Edit {
    pub range: Range<usize>,
    pub unit: usize,
    pub encoding: Encoding,
}
#[derive(Debug, Clone)]
pub(super) enum Encoding {
    Raw,
    Json,
    Csv {
        delimiter: u8,
    },
    Html,
    Quoted {
        quote: char,
        triple: bool,
    },
    Po,
    Empty,
    OfficeText {
        opening: String,
        closing: String,
        word_prefix: Option<String>,
    },
    Xliff {
        codes: Vec<(String, String)>,
        wrapper: Option<(String, String)>,
    },
}

pub fn supported_extension(extension: &str, mode: &str) -> bool {
    let ext = extension.trim_start_matches('.').to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "json"
            | "csv"
            | "tsv"
            | "yaml"
            | "yml"
            | "pdf"
            | "srt"
            | "vtt"
            | "html"
            | "htm"
            | "xlf"
            | "xliff"
            | "po"
            | "pot"
            | "docx"
            | "xlsx"
    ) || (mode == "game" && matches!(ext.as_str(), "lua" | "js" | "ts" | "gd"))
}

pub fn extract(bytes: &[u8], extension: &str, mode: &str) -> Result<Document, FormatError> {
    let ext = extension.trim_start_matches('.').to_ascii_lowercase();
    let game = mode == "game";
    match ext.as_str() {
        "txt" | "md" | "markdown" => text::plain(bytes, game, ext != "txt"),
        "json" => text::json(bytes, game),
        "csv" => tabular::extract(bytes, b',', game),
        "tsv" => tabular::extract(bytes, b'\t', game),
        "yaml" | "yml" => yaml::extract(bytes, game),
        "pdf" => pdf::extract(bytes, game),
        "srt" | "vtt" => text::subtitles(bytes, game, &ext),
        "html" | "htm" => markup::html(bytes, game),
        "xlf" | "xliff" => markup::xliff(bytes, game),
        "po" | "pot" => text::po(bytes, game),
        "lua" | "js" | "ts" | "gd" if game => text::script(bytes, &ext),
        "docx" | "xlsx" => office::extract(bytes, &ext, game),
        _ => Err(FormatError(format!(
            "Unsupported document extension: .{ext} (mode {mode})"
        ))),
    }
}

pub fn write_document(
    document: &Document,
    translations: &[String],
) -> Result<Vec<u8>, FormatError> {
    if document.units.len() != translations.len() {
        return Err(FormatError(format!(
            "Translation count mismatch: expected {}, received {}",
            document.units.len(),
            translations.len()
        )));
    }
    let output = match &document.context {
        Context::Edits { original, edits } => {
            apply_edits(original, edits, &document.units, translations)
        }
        Context::Office(office) => office::write(office, &document.units, translations),
        Context::Pdf(pdf) => pdf::write(pdf, translations),
        Context::Yaml(yaml) => yaml::write(yaml, &document.units, translations),
    }?;
    if output.len() > MAX_OUTPUT_BYTES {
        return Err(FormatError(
            "Document output exceeds the 80 MB limit".into(),
        ));
    }
    Ok(output)
}

fn append_output(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), FormatError> {
    if out
        .len()
        .checked_add(bytes.len())
        .is_none_or(|size| size > MAX_OUTPUT_BYTES)
    {
        return Err(FormatError(
            "Document output exceeds the 80 MB limit".into(),
        ));
    }
    out.extend_from_slice(bytes);
    Ok(())
}

pub(super) fn edits_document(bytes: &[u8], units: Vec<String>, edits: Vec<Edit>) -> Document {
    Document {
        units,
        context: Context::Edits {
            original: bytes.to_vec(),
            edits,
        },
    }
}

pub(super) fn apply_edits(
    original: &[u8],
    edits: &[Edit],
    units: &[String],
    translations: &[String],
) -> Result<Vec<u8>, FormatError> {
    let mut sorted: Vec<_> = edits.iter().collect();
    sorted.sort_by_key(|edit| (edit.range.start, edit.range.end));
    if original.len() > MAX_OUTPUT_BYTES {
        return Err(FormatError(
            "Document output exceeds the 80 MB limit".into(),
        ));
    }
    let mut out = Vec::with_capacity(original.len());
    let mut pos = 0;
    for edit in sorted {
        if edit.range.start < pos
            || edit.range.end > original.len()
            || edit.range.start > edit.range.end
        {
            return Err(FormatError(
                "Overlapping or invalid document replacement range".into(),
            ));
        }
        append_output(&mut out, &original[pos..edit.range.start])?;
        let translation = &translations[edit.unit];
        if translation == &units[edit.unit]
            && !matches!(&edit.encoding, Encoding::Po | Encoding::Xliff { .. })
        {
            append_output(&mut out, &original[edit.range.clone()])?;
        } else {
            let encoded = match &edit.encoding {
                Encoding::Raw => translation.clone(),
                Encoding::Json => {
                    serde_json::to_string(translation).map_err(|e| FormatError(e.to_string()))?
                }
                Encoding::Csv { delimiter } => tabular::encode_cell(translation, *delimiter)?,
                Encoding::Html => xml::escape_checked(translation)?,
                Encoding::Quoted { quote, triple } => {
                    text::encode_quoted(translation, *quote, *triple)
                }
                Encoding::Po => text::encode_quoted(translation, '"', false),
                Encoding::Empty => String::new(),
                Encoding::OfficeText {
                    opening,
                    closing,
                    word_prefix,
                } => office::encode_text(translation, opening, closing, word_prefix.as_deref())?,
                Encoding::Xliff { codes, wrapper } => {
                    let inner = markup::encode_xliff(translation, codes)?;
                    match wrapper {
                        Some((opening, closing)) => format!("{opening}{inner}{closing}"),
                        None => inner,
                    }
                }
            };
            append_output(&mut out, encoded.as_bytes())?;
        }
        pos = edit.range.end;
    }
    append_output(&mut out, &original[pos..])?;
    Ok(out)
}

/// Same conservative game/localization filtering used by the reference implementation.
pub(super) fn translatable(text: &str, game: bool) -> bool {
    let s = text.trim();
    if s.is_empty() {
        return false;
    }
    static RULES: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    let rules = RULES.get_or_init(|| {
        [
            r"(?i)^(https?://|s?ftps?://|mailto:)",
            r"^#?[0-9a-fA-F]{3,8}$",
            r"^[0-9a-fA-F]{8}-(?:[0-9a-fA-F]{4}-){3}[0-9a-fA-F]{12}$",
            r"^-?\d+(?:\.\d+)?$",
            r"(?i)^[\w./\\-]+\.(png|jpe?g|gif|webp|svg|wav|ogg|mp3|json|tscn|res|gd|lua|js|cs)$",
            r"^\{[A-Za-z0-9_]+\}$",
        ]
        .iter()
        .map(|r| regex::Regex::new(r).unwrap())
        .collect()
    });
    if rules.iter().any(|r| r.is_match(s)) {
        return false;
    }
    if !s.contains(' ') && !s.contains('\n') {
        static IDENTIFIERS: OnceLock<Vec<regex::Regex>> = OnceLock::new();
        let identifiers = IDENTIFIERS.get_or_init(|| {
            [
                r"^[A-Z][A-Z0-9_]{1,64}$",
                r"^[a-z_][a-z0-9_]{0,64}$",
                r"^[a-z]+(?:[A-Z][a-z0-9]*)+$",
                r"^[A-Z][a-zA-Z0-9]{1,40}$",
            ]
            .iter()
            .map(|r| regex::Regex::new(r).unwrap())
            .collect()
        });
        if identifiers[..3].iter().any(|r| r.is_match(s)) {
            return false;
        }
        if s.chars().count() < 28 && identifiers[3].is_match(s) {
            return false;
        }
    }
    !game || (s.chars().count() >= 2 && (s.chars().any(|c| c.is_whitespace() || ",.!?\"'，。！？、".contains(c)) || s.chars().any(|c| matches!(c, '\u{4e00}'..='\u{9fff}' | '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7af}'))))
}
