use super::{edits_document, translatable, Document, Edit, Encoding, FormatError};

pub(super) fn plain(bytes: &[u8], game: bool, markdown: bool) -> Result<Document, FormatError> {
    let raw = std::str::from_utf8(bytes)?;
    let mut pos = if raw.starts_with('\u{feff}') { 3 } else { 0 };
    let mut units = Vec::new();
    let mut edits = Vec::new();
    let mut paragraph: Option<std::ops::Range<usize>> = None;
    let mut fence = None;
    while pos < bytes.len() {
        let start = pos;
        while pos < bytes.len() && !matches!(bytes[pos], b'\r' | b'\n') {
            pos += 1;
        }
        let end = pos;
        if bytes.get(pos..pos + 2) == Some(b"\r\n") {
            pos += 2;
        } else if pos < bytes.len() {
            pos += 1;
        }
        let line = &raw[start..end];
        let mut code = false;
        if markdown {
            if let Some((mark, count)) = fence {
                if markdown_fence(line)
                    .is_some_and(|(m, n, rest)| m == mark && n >= count && rest.trim().is_empty())
                {
                    fence = None;
                }
                code = true;
            } else if let Some((mark, count, _)) = markdown_fence(line) {
                fence = Some((mark, count));
                code = true;
            } else {
                let mut column = 0;
                for byte in line.bytes() {
                    match byte {
                        b' ' => column += 1,
                        b'\t' => column += 4 - column % 4,
                        _ => break,
                    }
                    if column >= 4 {
                        code = true;
                        break;
                    }
                }
            }
        }
        if code || line.trim().is_empty() {
            if let Some(range) = paragraph.take() {
                plain_range(raw, range, game, markdown, &mut units, &mut edits)?;
            }
        } else if let Some(range) = &mut paragraph {
            range.end = end;
        } else {
            paragraph = Some(start..end);
        }
    }
    if let Some(range) = paragraph {
        plain_range(raw, range, game, markdown, &mut units, &mut edits)?;
    }
    // The reference batches complete paragraphs; an oversized paragraph is still
    // one unit. Original BOM, blank separators and each final line ending are raw.
    Ok(edits_document(bytes, units, edits))
}

fn markdown_fence(line: &str) -> Option<(u8, usize, &str)> {
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return None;
    }
    let body = &line[indent..];
    let mark = *body.as_bytes().first()?;
    if !matches!(mark, b'`' | b'~') {
        return None;
    }
    let count = body.bytes().take_while(|b| *b == mark).count();
    let rest = &body[count..];
    (count >= 3 && (mark != b'`' || !rest.contains('`'))).then_some((mark, count, rest))
}

fn plain_range(
    raw: &str,
    range: std::ops::Range<usize>,
    game: bool,
    markdown: bool,
    units: &mut Vec<String>,
    edits: &mut Vec<Edit>,
) -> Result<(), FormatError> {
    fn push_unit(
        raw: &str,
        range: std::ops::Range<usize>,
        units: &mut Vec<String>,
        edits: &mut Vec<Edit>,
    ) {
        if raw[range.clone()].trim().is_empty() {
            return;
        }
        let unit = units.len();
        units.push(raw[range.clone()].into());
        edits.push(Edit {
            range,
            unit,
            encoding: Encoding::Raw,
        });
    }
    fn push(
        raw: &str,
        range: std::ops::Range<usize>,
        lines: &Option<Vec<std::ops::Range<usize>>>,
        units: &mut Vec<String>,
        edits: &mut Vec<Edit>,
    ) {
        if let Some(lines) = lines {
            let first = lines.partition_point(|line| line.end <= range.start);
            for line in &lines[first..] {
                if line.start >= range.end {
                    break;
                }
                push_unit(
                    raw,
                    line.start.max(range.start)..line.end.min(range.end),
                    units,
                    edits,
                );
            }
        } else {
            push_unit(raw, range, units, edits);
        }
    }
    let bytes = raw.as_bytes();
    let lines = if game {
        let mut lines = Vec::new();
        let mut pos = range.start;
        while pos < range.end {
            let start = pos;
            while pos < range.end && !matches!(bytes[pos], b'\r' | b'\n') {
                pos += 1;
            }
            if translatable(&raw[start..pos], true) {
                lines.push(start..pos);
            }
            if bytes.get(pos..pos + 2) == Some(b"\r\n") {
                pos += 2;
            } else {
                pos += 1;
            }
        }
        Some(lines)
    } else {
        None
    };
    if !markdown {
        push(raw, range, &lines, units, edits);
        return Ok(());
    }
    // Keep inline code raw, just as fenced/indented code is kept raw. Prose on
    // either side becomes separate units, so providers never receive code bytes.
    let mut runs: Vec<(std::ops::Range<usize>, bool, Option<usize>)> = Vec::new();
    let mut pos = range.start;
    while pos < range.end {
        if bytes[pos] != b'`' {
            pos += 1;
            continue;
        }
        let slashes = bytes[range.start..pos]
            .iter()
            .rev()
            .take_while(|b| **b == b'\\')
            .count();
        let open = pos;
        while pos < range.end && bytes[pos] == b'`' {
            pos += 1;
        }
        if runs.len() >= 100_000 {
            return Err(FormatError(
                "Markdown paragraph exceeds 100000 inline code delimiter runs".into(),
            ));
        }
        runs.push((open..pos, slashes % 2 != 0, None));
    }
    // Index the next equal-length delimiter once. Repeated scans from unmatched
    // openers can otherwise become quadratic on malformed Markdown.
    let mut next = std::collections::HashMap::new();
    for index in (0..runs.len()).rev() {
        // An escape consumes the first tick only when this run opens a span.
        // Inside code, backslashes are literal and closing runs use full width.
        let opening_width = runs[index].0.len() - usize::from(runs[index].1);
        runs[index].2 = next.get(&opening_width).copied();
        next.insert(runs[index].0.len(), index);
    }
    let mut prose = range.start;
    let mut index = 0;
    while index < runs.len() {
        let (opening, escaped, closing) = &runs[index];
        if let Some(closing) = *closing {
            push(
                raw,
                prose..opening.start + usize::from(*escaped),
                &lines,
                units,
                edits,
            );
            prose = runs[closing].0.end;
            index = closing + 1;
            continue;
        }
        index += 1;
    }
    push(raw, prose..range.end, &lines, units, edits);
    Ok(())
}

pub(super) fn json(bytes: &[u8], game: bool) -> Result<Document, FormatError> {
    let raw = std::str::from_utf8(bytes)?;
    let start = if raw.starts_with('\u{feff}') { 3 } else { 0 };
    serde_json::from_str::<serde_json::Value>(&raw[start..])
        .map_err(|e| FormatError(format!("Malformed JSON: {e}")))?;
    let mut units = Vec::new();
    let mut edits = Vec::new();
    let mut pos = start;
    while pos < bytes.len() {
        if bytes[pos] != b'"' {
            pos += 1;
            continue;
        }
        let end = quoted_end(raw, pos, '"', false)?;
        let next = bytes[end..]
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .map(|i| end + i);
        if next.map(|i| bytes[i] != b':').unwrap_or(true) {
            let value: String =
                serde_json::from_str(&raw[pos..end]).map_err(|e| FormatError(e.to_string()))?;
            if translatable(&value, game) {
                let unit = units.len();
                units.push(value);
                edits.push(Edit {
                    range: pos..end,
                    unit,
                    encoding: Encoding::Json,
                });
            }
        }
        pos = end;
    }
    Ok(edits_document(bytes, units, edits))
}

pub(super) fn subtitles(
    bytes: &[u8],
    game: bool,
    extension: &str,
) -> Result<Document, FormatError> {
    let raw = std::str::from_utf8(bytes)?;
    if extension == "vtt" && !raw.trim_start_matches('\u{feff}').starts_with("WEBVTT") {
        return Err(FormatError("VTT must begin with WEBVTT".into()));
    }
    let time = if extension == "srt" {
        r"\d{2,}:\d{2}:\d{2},\d{3}"
    } else {
        r"(?:\d{2,}:)?\d{2}:\d{2}\.\d{3}"
    };
    let cue = regex::Regex::new(&format!(
        r"(?m)^{time}[ \t]*-->[ \t]*{time}[^\r\n]*(?:\r\n|\n|\r)"
    ))
    .unwrap();
    // A CRLF is one line ending, never two cue separators.
    let separator = regex::Regex::new(r"(?:\r\n|\n)[ \t]*(?:\r\n|\n)").unwrap();
    let mut units = Vec::new();
    let mut edits = Vec::new();
    let cues: Vec<_> = cue.find_iter(raw).collect();
    if cues.is_empty() && extension == "srt" && !raw.trim().is_empty() {
        return Err(FormatError("SRT contains no valid timestamp cues".into()));
    }
    for timestamp in cues {
        let start = timestamp.end();
        let mut end = separator
            .find(&raw[start..])
            .map(|m| start + m.start())
            .unwrap_or(raw.len());
        while end > start && matches!(bytes[end - 1], b'\r' | b'\n') {
            end -= 1;
        }
        let body = &raw[start..end];
        if (!game && !body.trim().is_empty()) || translatable(body, game) {
            let unit = units.len();
            units.push(body.to_string());
            edits.push(Edit {
                range: start..end,
                unit,
                encoding: Encoding::Raw,
            });
        }
    }
    Ok(edits_document(bytes, units, edits))
}

pub(super) fn script(bytes: &[u8], extension: &str) -> Result<Document, FormatError> {
    let raw = std::str::from_utf8(bytes)?;
    let mut units = Vec::new();
    let mut edits = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let rest = &bytes[pos..];
        if (extension == "lua" && rest.starts_with(b"--"))
            || ((extension == "js" || extension == "ts") && rest.starts_with(b"//"))
            || (extension == "gd" && rest[0] == b'#')
        {
            if extension == "lua" && rest.starts_with(b"--[[") {
                let offset = raw[pos + 4..]
                    .find("]]")
                    .ok_or_else(|| FormatError("Unclosed Lua block comment".into()))?;
                pos += offset + 6;
            } else {
                pos += rest.iter().position(|b| *b == b'\n').unwrap_or(rest.len());
            }
            continue;
        }
        if (extension == "js" || extension == "ts") && rest.starts_with(b"/*") {
            let offset = raw[pos + 2..]
                .find("*/")
                .ok_or_else(|| FormatError("Unclosed script block comment".into()))?;
            pos += offset + 4;
            continue;
        }
        let quote = bytes[pos] as char;
        if quote != '\'' && quote != '"' {
            pos += 1;
            continue;
        }
        let triple = rest.len() >= 3 && rest[0] == rest[1] && rest[1] == rest[2];
        let width = if triple { 3 } else { 1 };
        let end = quoted_end(raw, pos, quote, triple)?;
        let inner = decode_escapes(&raw[pos + width..end - width])?;
        if translatable(&inner, true) {
            let unit = units.len();
            units.push(inner);
            edits.push(Edit {
                range: pos..end,
                unit,
                encoding: Encoding::Quoted { quote, triple },
            });
        }
        pos = end;
    }
    Ok(edits_document(bytes, units, edits))
}

fn quoted_end(raw: &str, start: usize, quote: char, triple: bool) -> Result<usize, FormatError> {
    let bytes = raw.as_bytes();
    let width = if triple { 3 } else { 1 };
    let mut pos = start + width;
    while pos < bytes.len() {
        if bytes[pos] == b'\\' {
            pos += 2;
            continue;
        }
        if bytes[pos] == quote as u8
            && (!triple
                || bytes
                    .get(pos..pos + 3)
                    .map(|s| s.iter().all(|b| *b == quote as u8))
                    .unwrap_or(false))
        {
            return Ok(pos + width);
        }
        if !triple && matches!(bytes[pos], b'\r' | b'\n') {
            return Err(FormatError(
                "Unescaped newline inside a quoted string".into(),
            ));
        }
        pos += 1;
    }
    Err(FormatError("Unclosed quoted string".into()))
}
fn decode_escapes(inner: &str) -> Result<String, FormatError> {
    let mut chars = inner.chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let escaped = chars
            .next()
            .ok_or_else(|| FormatError("Incomplete string escape".into()))?;
        match escaped {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            'a' => out.push('\u{7}'),
            '\\' | '\'' | '"' | '/' => out.push(escaped),
            '\n' => {}
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            'x' | 'u' => {
                let digits = if escaped == 'x' { 2 } else { 4 };
                let mut hex = String::new();
                for _ in 0..digits {
                    hex.push(chars.next().ok_or_else(|| {
                        FormatError("Incomplete hexadecimal string escape".into())
                    })?);
                }
                let value = u32::from_str_radix(&hex, 16)
                    .map_err(|_| FormatError("Invalid hexadecimal string escape".into()))?;
                if (0xd800..=0xdbff).contains(&value) && escaped == 'u' {
                    if chars.next() != Some('\\') || chars.next() != Some('u') {
                        return Err(FormatError("Unpaired UTF-16 surrogate".into()));
                    }
                    let mut low = String::new();
                    for _ in 0..4 {
                        low.push(
                            chars
                                .next()
                                .ok_or_else(|| FormatError("Incomplete UTF-16 surrogate".into()))?,
                        );
                    }
                    let low = u32::from_str_radix(&low, 16)
                        .map_err(|_| FormatError("Invalid UTF-16 surrogate".into()))?;
                    if !(0xdc00..=0xdfff).contains(&low) {
                        return Err(FormatError("Unpaired UTF-16 surrogate".into()));
                    }
                    out.push(
                        char::from_u32(0x10000 + ((value - 0xd800) << 10) + low - 0xdc00).unwrap(),
                    );
                } else {
                    out.push(
                        char::from_u32(value)
                            .ok_or_else(|| FormatError("Invalid Unicode string escape".into()))?,
                    );
                }
            }
            '0'..='9' => {
                let mut number = escaped.to_string();
                while number.len() < 3 && chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false)
                {
                    number.push(chars.next().unwrap());
                }
                let value: u32 = number
                    .parse()
                    .map_err(|_| FormatError("Invalid decimal string escape".into()))?;
                if value > 255 {
                    return Err(FormatError("Decimal string escape exceeds one byte".into()));
                }
                out.push(char::from_u32(value).unwrap());
            }
            _ => {
                out.push('\\');
                out.push(escaped);
            }
        }
    }
    Ok(out)
}
pub(super) fn encode_quoted(text: &str, quote: char, triple: bool) -> String {
    let delimiter = quote.to_string().repeat(if triple { 3 } else { 1 });
    let mut out = delimiter.clone();
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push_str(&delimiter);
    out
}

#[derive(Debug)]
struct PoField {
    name: String,
    start: usize,
    end: usize,
    value: String,
}
pub(super) fn po(bytes: &[u8], game: bool) -> Result<Document, FormatError> {
    let raw = std::str::from_utf8(bytes)?;
    let mut units = Vec::new();
    let mut edits = Vec::new();
    let mut fields = Vec::<PoField>::new();
    let mut current: Option<PoField> = None;
    let mut offset = 0;
    let field_re =
        regex::Regex::new(r"^(msgid_plural|msgid|msgstr(?:\[\d+\])?|msgctxt)[ \t]+").unwrap();
    for line in raw.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if let Some(keyword) = field_re.find(trimmed) {
            if let Some(field) = current.take() {
                fields.push(field);
            }
            let qstart = keyword.end();
            if !trimmed[qstart..].starts_with('"') {
                return Err(FormatError("PO field must contain a quoted value".into()));
            }
            let qend = quoted_end(trimmed, qstart, '"', false)?;
            if !trimmed[qend..].trim().is_empty() {
                return Err(FormatError("Unexpected content after a PO string".into()));
            }
            current = Some(PoField {
                name: trimmed[..keyword.end()].trim().into(),
                start: offset + qstart,
                end: offset + qend,
                value: decode_escapes(&trimmed[qstart + 1..qend - 1])?,
            });
        } else if trimmed.trim_start().starts_with('"') {
            let field = current
                .as_mut()
                .ok_or_else(|| FormatError("PO continuation has no field".into()))?;
            let qstart = trimmed.find('"').unwrap();
            let qend = quoted_end(trimmed, qstart, '"', false)?;
            field
                .value
                .push_str(&decode_escapes(&trimmed[qstart + 1..qend - 1])?);
            field.end = offset + qend;
        } else if let Some(field) = current.take() {
            fields.push(field);
        }
        offset += line.len();
    }
    if let Some(field) = current {
        fields.push(field);
    }
    let mut source = "";
    let mut plural = "";
    for field in &fields {
        match field.name.as_str() {
            "msgid" => {
                source = &field.value;
                plural = "";
            }
            "msgid_plural" => plural = &field.value,
            name if name.starts_with("msgstr") => {
                if source.is_empty() {
                    continue;
                } // Header metadata must stay untouched.
                let pick = if !field.value.trim().is_empty() {
                    &field.value
                } else if name == "msgstr[0]" || plural.is_empty() {
                    source
                } else {
                    plural
                };
                if translatable(pick, game) || (field.value.is_empty() && translatable(pick, false))
                {
                    let unit = units.len();
                    units.push(pick.into());
                    edits.push(Edit {
                        range: field.start..field.end,
                        unit,
                        encoding: Encoding::Po,
                    });
                }
            }
            _ => {}
        }
    }
    if fields.is_empty() && !raw.trim().is_empty() {
        return Err(FormatError("PO contains no valid message fields".into()));
    }
    Ok(edits_document(bytes, units, edits))
}
