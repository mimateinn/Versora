//! Bounded supported subset of YAML 1.2 with value-only translation and i64
//! integer limits. Parser scalar ambiguities are rejected before loading. Like
//! the reference safe_dump writer, changed documents normalize presentation.
use super::{translatable, Context, Document, FormatError};
use std::{collections::HashMap, fmt};
use yaml_rust2::{
    parser::{Event, Parser, Tag},
    scanner::TScalarStyle,
    Yaml, YamlEmitter, YamlLoader,
};

const MAX_BYTES: usize = 80 * 1024 * 1024;
const MAX_SCALAR_BYTES: usize = 4 * 1024 * 1024;
const MAX_NODES: usize = 200_000;
const MAX_DEPTH: usize = 128;
const MAX_DOCUMENTS: usize = 64;

#[derive(Debug)]
pub(super) struct YamlDocument {
    original: Vec<u8>,
    documents: Vec<Yaml>,
    game: bool,
    bom: bool,
    ending: &'static str,
    trailing_ending: bool,
}

#[derive(Clone, Copy, Default)]
struct Cost {
    nodes: usize,
    bytes: usize,
    depth: usize,
}
impl Cost {
    fn add(&mut self, value: Self) -> Result<(), FormatError> {
        self.nodes = self
            .nodes
            .checked_add(value.nodes)
            .ok_or_else(|| FormatError("YAML node count overflow".into()))?;
        self.bytes = self
            .bytes
            .checked_add(value.bytes)
            .ok_or_else(|| FormatError("YAML scalar size overflow".into()))?;
        if self.nodes > MAX_NODES || self.bytes > MAX_BYTES {
            return Err(FormatError(
                "YAML exceeds 200000 expanded nodes or 80 MiB expanded scalar data".into(),
            ));
        }
        Ok(())
    }
}

fn tag_supported(tag: &Option<Tag>, collection: Option<&str>) -> bool {
    tag.as_ref().is_none_or(|tag| {
        tag.handle == "tag:yaml.org,2002:"
            && match collection {
                Some(kind) => tag.suffix == kind,
                None => matches!(
                    tag.suffix.as_str(),
                    "str" | "bool" | "int" | "float" | "null"
                ),
            }
    })
}

fn explicit_scalar_valid(tag: &Tag, value: &str) -> bool {
    // Match yaml-rust2 0.13.0's exact plain-scalar resolution rules. Its loader
    // returns BadValue for invalid typed scalars, but also uses BadValue as the
    // pending mapping-key sentinel. Reject these events before building a tree.
    match tag.suffix.as_str() {
        "str" => true,
        "int" => value.parse::<i64>().is_ok(),
        "bool" => matches!(
            value,
            "true" | "True" | "TRUE" | "false" | "False" | "FALSE"
        ),
        "null" => matches!(value, "~" | "null"),
        "float" => match value {
            ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" | "-.inf" | "-.Inf"
            | "-.INF" | ".nan" | ".NaN" | ".NAN" => true,
            _ => value.bytes().any(|byte| byte.is_ascii_digit()) && value.parse::<f64>().is_ok(),
        },
        _ => false,
    }
}

fn core_radix_integer(value: &str) -> bool {
    let unsigned = value
        .strip_prefix('+')
        .or_else(|| value.strip_prefix('-'))
        .unwrap_or(value);
    unsigned.strip_prefix("0x").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) || unsigned.strip_prefix("0o").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| matches!(byte, b'0'..=b'7'))
    })
}

fn validate_implicit_scalar(value: &str) -> Result<(), FormatError> {
    if matches!(value, "Null" | "NULL") {
        // These are core-schema null values, but the upstream loader returns
        // String and its emitter would quote them after changing another value.
        return Err(FormatError(
            "Unsupported YAML null spelling; use null or ~".into(),
        ));
    }
    let unsigned = value
        .strip_prefix('+')
        .or_else(|| value.strip_prefix('-'))
        .unwrap_or(value);
    let decimal = !unsigned.is_empty() && unsigned.bytes().all(|byte| byte.is_ascii_digit());
    let radix = core_radix_integer(value);
    if (decimal || radix) && !matches!(Yaml::from_str(value), Yaml::Integer(_)) {
        // Signed radix forms and values outside i64 can otherwise fall through
        // to String/Real, changing core-schema integer data without an error.
        return Err(FormatError("Unsupported YAML plain integer representation; use a supported i64 integer or quote text".into()));
    }
    Ok(())
}

/// Preflight uses the event iterator, so excessive nesting/aliases are rejected
/// before YamlLoader recursively builds and clones an expanded object tree.
fn preflight(source: &str) -> Result<(), FormatError> {
    let mut parser = Parser::new_from_str(source);
    let mut stack: Vec<(usize, Cost)> = Vec::new();
    let mut anchors = HashMap::new();
    let mut allocated = Cost::default();
    let mut documents = 0;
    loop {
        let (event, _) = parser
            .next_token()
            .map_err(|error| FormatError(format!("Cannot parse YAML: {error}")))?;
        let complete = match event {
            Event::StreamEnd => break,
            Event::DocumentStart => {
                documents += 1;
                if documents > MAX_DOCUMENTS {
                    return Err(FormatError("YAML exceeds 64 documents".into()));
                }
                anchors.clear();
                None
            }
            Event::SequenceStart(anchor, tag) => {
                if !tag_supported(&tag, Some("seq")) {
                    return Err(FormatError(
                        "YAML custom tags are unsupported; remove custom tags before translation"
                            .into(),
                    ));
                }
                if stack.len() >= MAX_DEPTH {
                    return Err(FormatError(
                        "YAML exceeds the 128-level nesting limit".into(),
                    ));
                }
                allocated.add(Cost {
                    nodes: 1,
                    bytes: 0,
                    depth: 1,
                })?;
                stack.push((
                    anchor,
                    Cost {
                        nodes: 1,
                        bytes: 0,
                        depth: 1,
                    },
                ));
                None
            }
            Event::MappingStart(anchor, tag) => {
                if !tag_supported(&tag, Some("map")) {
                    return Err(FormatError(
                        "YAML custom tags are unsupported; remove custom tags before translation"
                            .into(),
                    ));
                }
                if stack.len() >= MAX_DEPTH {
                    return Err(FormatError(
                        "YAML exceeds the 128-level nesting limit".into(),
                    ));
                }
                allocated.add(Cost {
                    nodes: 1,
                    bytes: 0,
                    depth: 1,
                })?;
                stack.push((
                    anchor,
                    Cost {
                        nodes: 1,
                        bytes: 0,
                        depth: 1,
                    },
                ));
                None
            }
            Event::SequenceEnd | Event::MappingEnd => Some(
                stack
                    .pop()
                    .ok_or_else(|| FormatError("YAML collection framing is invalid".into()))?,
            ),
            Event::Scalar(value, style, anchor, tag) => {
                if !tag_supported(&tag, None) {
                    return Err(FormatError(
                        "YAML custom tags are unsupported; remove custom tags before translation"
                            .into(),
                    ));
                }
                if style != TScalarStyle::Plain
                    && tag.as_ref().is_some_and(|tag| tag.suffix != "str")
                {
                    // The upstream loader resolves quoted/block nodes as String
                    // before considering scalar tags. Reject that unsupported
                    // combination rather than changing an explicit scalar type.
                    return Err(FormatError("YAML quoted/block scalars with explicit non-string types are unsupported; use plain typed values".into()));
                }
                if value.len() > MAX_SCALAR_BYTES {
                    return Err(FormatError("YAML scalar exceeds the 4 MiB limit".into()));
                }
                if style == TScalarStyle::Plain {
                    if let Some(tag) = &tag {
                        if !explicit_scalar_valid(tag, &value) {
                            return Err(FormatError(format!(
                                "Invalid YAML !!{} scalar",
                                tag.suffix
                            )));
                        }
                    } else {
                        validate_implicit_scalar(&value)?;
                    }
                }
                let cost = Cost {
                    nodes: 1,
                    bytes: value.len(),
                    depth: 1,
                };
                allocated.add(cost)?;
                Some((anchor, cost))
            }
            Event::Alias(anchor) => {
                let cost = *anchors.get(&anchor).ok_or_else(|| {
                    FormatError(
                        "YAML forward, cyclic or cross-document alias is unsupported".into(),
                    )
                })?;
                allocated.add(cost)?;
                Some((0, cost))
            }
            _ => None,
        };
        if let Some((anchor, cost)) = complete {
            if anchor != 0 {
                // Loader keeps an additional clone for each anchor.
                allocated.add(cost)?;
                anchors.insert(anchor, cost);
            }
            if let Some((_, parent)) = stack.last_mut() {
                parent.add(cost)?;
                parent.depth = parent.depth.max(cost.depth + 1);
                if parent.depth > MAX_DEPTH {
                    return Err(FormatError(
                        "Expanded YAML exceeds the 128-level nesting limit".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn check_tree(value: &Yaml, depth: usize) -> Result<(), FormatError> {
    if depth > MAX_DEPTH {
        return Err(FormatError(
            "Expanded YAML exceeds the 128-level nesting limit".into(),
        ));
    }
    match value {
        Yaml::Array(values) => {
            for value in values {
                check_tree(value, depth + 1)?;
            }
        }
        Yaml::Hash(values) => {
            for (key, value) in values {
                if key.as_str() == Some("<<") {
                    return Err(FormatError("YAML merge keys are unsupported; expand merged mappings before translation".into()));
                }
                check_tree(key, depth + 1)?;
                check_tree(value, depth + 1)?;
            }
        }
        Yaml::Alias(_) | Yaml::BadValue => {
            return Err(FormatError(
                "YAML contains an unresolved alias or invalid typed scalar".into(),
            ))
        }
        _ => {}
    }
    Ok(())
}

fn collect(value: &Yaml, game: bool, units: &mut Vec<String>) {
    match value {
        Yaml::String(value) if translatable(value, game) => units.push(value.clone()),
        Yaml::Array(values) => {
            for value in values {
                collect(value, game, units);
            }
        }
        Yaml::Hash(values) => {
            for (_, value) in values {
                collect(value, game, units);
            }
        }
        _ => {}
    }
}

pub(super) fn extract(bytes: &[u8], game: bool) -> Result<Document, FormatError> {
    if bytes.len() > MAX_BYTES {
        return Err(FormatError("YAML exceeds the 80 MiB document limit".into()));
    }
    let raw = std::str::from_utf8(bytes)?;
    let bom = raw.starts_with('\u{feff}');
    let source = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    preflight(source)?;
    let documents = YamlLoader::load_from_str(source)
        .map_err(|error| FormatError(format!("Cannot load YAML: {error}")))?;
    let mut units = Vec::new();
    for document in &documents {
        check_tree(document, 0)?;
        collect(document, game, &mut units);
    }
    let ending = match source
        .bytes()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
    {
        Some(position) if source.as_bytes()[position..].starts_with(b"\r\n") => "\r\n",
        Some(position) if source.as_bytes()[position] == b'\r' => "\r",
        _ => "\n",
    };
    Ok(Document {
        units,
        context: Context::Yaml(YamlDocument {
            original: bytes.to_vec(),
            documents,
            game,
            bom,
            ending,
            trailing_ending: source.ends_with(['\r', '\n']),
        }),
    })
}

fn normalize_numeric_types(value: &mut Yaml) -> Result<(), FormatError> {
    match value {
        Yaml::Real(value) if !matches!(Yaml::from_str(value), Yaml::Real(_)) => {
            // Explicit `!!float 2` is a Real even though plain `2` resolves to
            // Integer. Keep its numeric type when the emitter drops tags.
            value.push_str(".0");
        }
        Yaml::Array(values) => {
            for value in values {
                normalize_numeric_types(value)?;
            }
        }
        Yaml::Hash(values) => {
            let entries = std::mem::take(values);
            for (mut key, mut value) in entries {
                normalize_numeric_types(&mut key)?;
                normalize_numeric_types(&mut value)?;
                if values.insert(key, value).is_some() {
                    return Err(FormatError(
                        "YAML has colliding numeric keys after scalar-type normalization".into(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn translate(
    value: &mut Yaml,
    game: bool,
    translations: &[String],
    next: &mut usize,
) -> Result<(), FormatError> {
    match value {
        Yaml::String(value) if translatable(value, game) => {
            let replacement = translations
                .get(*next)
                .ok_or_else(|| FormatError("YAML translation count mismatch".into()))?;
            if replacement.len() > MAX_SCALAR_BYTES {
                return Err(FormatError("Translated YAML scalar exceeds 4 MiB".into()));
            }
            *value = replacement.clone();
            *next += 1;
        }
        Yaml::Array(values) => {
            for value in values {
                translate(value, game, translations, next)?;
            }
        }
        Yaml::Hash(values) => {
            for (_, value) in values.iter_mut() {
                translate(value, game, translations, next)?;
            }
        }
        _ => {}
    }
    Ok(())
}

struct BoundedWriter(String);
impl fmt::Write for BoundedWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.0.len().saturating_add(value.len()) > MAX_BYTES {
            return Err(fmt::Error);
        }
        self.0.push_str(value);
        Ok(())
    }
}

fn quote_ambiguous_strings(source: String) -> Result<String, FormatError> {
    let mut parser = Parser::new_from_str(&source);
    let mut replacements = Vec::new();
    let mut characters = source.char_indices();
    let mut marker_characters = 0usize;
    let mut marker_bytes = 0usize;
    loop {
        let (event, marker) = parser
            .next_token()
            .map_err(|error| FormatError(format!("Written YAML does not parse: {error}")))?;
        match event {
            Event::StreamEnd => break,
            Event::Scalar(value, TScalarStyle::Plain, _, None)
                if validate_implicit_scalar(&value).is_err() || core_radix_integer(&value) =>
            {
                // The upstream emitter omits quotes for string-valued +0x/+0o
                // and ordinary 0o tokens. Integer nodes emit decimal notation,
                // so a radix token here is always string-valued. Quote it
                // scalar so a core-schema reader also sees a string. Despite
                // its API docs, this parser's marker index counts characters.
                // Convert ordered markers to UTF-8 bytes in a single pass.
                if marker.index() < marker_characters {
                    return Err(FormatError(
                        "YAML scalar replacement markers are not ordered".into(),
                    ));
                }
                while marker_characters < marker.index() {
                    let (start, character) = characters.next().ok_or_else(|| {
                        FormatError("YAML scalar marker is outside its source".into())
                    })?;
                    marker_bytes = start + character.len_utf8();
                    marker_characters += 1;
                }
                let end = marker_bytes
                    .checked_add(value.len())
                    .ok_or_else(|| FormatError("YAML scalar replacement range overflow".into()))?;
                if !value.is_ascii() || source.get(marker_bytes..end) != Some(value.as_str()) {
                    return Err(FormatError(
                        "YAML scalar replacement marker does not match its source".into(),
                    ));
                }
                let encoded = serde_json::to_string(&value)
                    .map_err(|error| FormatError(error.to_string()))?;
                replacements.push((marker_bytes..end, encoded));
            }
            _ => {}
        }
    }
    if replacements.is_empty() {
        return Ok(source);
    }
    let mut output = BoundedWriter(String::with_capacity(source.len()));
    let mut position = 0;
    for (range, encoded) in replacements {
        if range.start < position {
            return Err(FormatError("YAML scalar replacement ranges overlap".into()));
        }
        fmt::Write::write_str(&mut output, &source[position..range.start])
            .and_then(|_| fmt::Write::write_str(&mut output, &encoded))
            .map_err(|_| FormatError("YAML output exceeds 80 MiB".into()))?;
        position = range.end;
    }
    fmt::Write::write_str(&mut output, &source[position..])
        .map_err(|_| FormatError("YAML output exceeds 80 MiB".into()))?;
    Ok(output.0)
}

pub(super) fn write(
    document: &YamlDocument,
    units: &[String],
    translations: &[String],
) -> Result<Vec<u8>, FormatError> {
    if units == translations {
        return Ok(document.original.clone());
    }
    let mut translated_bytes = 0usize;
    for value in translations {
        translated_bytes = translated_bytes
            .checked_add(value.len())
            .ok_or_else(|| FormatError("Translated YAML scalar size overflow".into()))?;
        if translated_bytes > MAX_BYTES {
            return Err(FormatError(
                "Translated YAML scalar data exceeds 80 MiB".into(),
            ));
        }
    }
    let mut documents = document.documents.clone();
    let mut next = 0;
    for value in &mut documents {
        normalize_numeric_types(value)?;
        translate(value, document.game, translations, &mut next)?;
    }
    if next != translations.len() {
        return Err(FormatError("YAML translation count mismatch".into()));
    }
    let mut output = BoundedWriter(String::new());
    for (index, value) in documents.iter().enumerate() {
        if index != 0 {
            fmt::Write::write_str(&mut output, "\n")
                .map_err(|_| FormatError("YAML output exceeds 80 MiB".into()))?;
        }
        YamlEmitter::new(&mut output).dump(value).map_err(|error| {
            FormatError(format!("Cannot write YAML (80 MiB output limit): {error}"))
        })?;
    }
    output.0 = quote_ambiguous_strings(output.0)?;
    // Verify serializer/type fidelity before allowing a document to be saved.
    let reparsed = YamlLoader::load_from_str(&output.0)
        .map_err(|error| FormatError(format!("Written YAML does not parse: {error}")))?;
    if reparsed != documents {
        return Err(FormatError(
            "YAML writer changed a scalar type or document structure".into(),
        ));
    }
    if document.trailing_ending {
        output.0.push('\n');
    }
    let expanded_length = output
        .0
        .len()
        .saturating_add(
            output
                .0
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count()
                .saturating_mul(document.ending.len() - 1),
        )
        .saturating_add(usize::from(document.bom) * 3);
    if expanded_length > MAX_BYTES {
        return Err(FormatError("YAML output exceeds 80 MiB".into()));
    }
    let output = output.0.replace('\n', document.ending);
    if output.len().saturating_add(usize::from(document.bom) * 3) > MAX_BYTES {
        return Err(FormatError("YAML output exceeds 80 MiB".into()));
    }
    let mut bytes = Vec::with_capacity(output.len() + usize::from(document.bom) * 3);
    if document.bom {
        bytes.extend_from_slice(b"\xef\xbb\xbf");
    }
    bytes.extend_from_slice(output.as_bytes());
    Ok(bytes)
}
