//! Runtime format v2: source layout stays outside the numbered provider payload.
//!
//! Only LF separates protocol rows. Unicode separators are ordinary content. Unlike the
//! reference parser, duplicate, skipped, reversed, or extra item numbers and unclosed
//! outer fences fail closed instead of becoming continuation text.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

pub const FORMAT_VERSION: u32 = 2;
pub const DEFAULT_MARK: char = '\u{23ce}';
pub const MARKS: [char; 3] = [DEFAULT_MARK, '\u{2424}', '\u{21b5}'];
pub const BATCH_ITEMS: usize = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    ItemCount { expected: usize, actual: usize },
    ItemSequence { expected: usize, actual: usize },
    NumberOverflow,
    TextBeforeFirstItem,
    UnclosedFence,
    LineCount { expected: usize, actual: usize },
    EmbeddedLineBreak,
    BlankLineChanged,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ItemCount { expected, actual } => {
                write!(f, "expected {expected} items, got {actual}")
            }
            Self::ItemSequence { expected, actual } => {
                write!(f, "expected item {expected}, got {actual}")
            }
            Self::NumberOverflow => f.write_str("numbered item index is too large"),
            Self::TextBeforeFirstItem => f.write_str("text before item 1"),
            Self::UnclosedFence => f.write_str("unclosed outer reply fence"),
            Self::LineCount { expected, actual } => write!(
                f,
                "translated line-break structure changed: expected {expected} lines, got {actual}"
            ),
            Self::EmbeddedLineBreak => f.write_str("a translated line contains a line break"),
            Self::BlankLineChanged => f.write_str("a source blank line gained translated content"),
        }
    }
}

impl Error for CodecError {}

fn horizontal(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\u{000c}' | '\u{000b}')
}

fn layout(c: char) -> bool {
    horizontal(c) || c == '\n'
}

/// Lossless single-line representation: double literal marks and frame LF with one space.
pub fn encode(text: &str, mark: char) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == mark {
            out.push(mark);
            out.push(mark);
        } else if c == '\n' {
            out.push(' ');
            out.push(mark);
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// Inverse of [`encode`]. Bare marks also mean LF, tolerating missing model framing spaces.
pub fn decode(text: &str, mark: char) -> String {
    let doubled = format!("{mark}{mark}");
    let framed = format!(" {mark} ");
    let mut rest = text;
    let mut out = String::with_capacity(text.len());
    while !rest.is_empty() {
        if rest.starts_with(&doubled) {
            out.push(mark);
            rest = &rest[doubled.len()..];
        } else if rest.starts_with(&framed) {
            out.push('\n');
            rest = &rest[framed.len()..];
        } else {
            let c = rest.chars().next().expect("nonempty suffix");
            out.push(if c == mark { '\n' } else { c });
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

pub fn choose_mark(items: &[String]) -> char {
    MARKS
        .into_iter()
        .find(|mark| items.iter().all(|text| !text.contains(*mark)))
        .unwrap_or(DEFAULT_MARK)
}

pub fn numbered(items: &[String], mark: char) -> String {
    items
        .iter()
        .enumerate()
        .map(|(i, text)| format!("{}. {}", i + 1, encode(text, mark)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Original leading/final whitespace and each physical line's whitespace are immutable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub head: String,
    pub tail: String,
    pub text: String,
    edges: Vec<(String, String)>,
    blank: Vec<bool>,
}

impl Shape {
    pub fn of(item: &str) -> Self {
        let left = item.trim_start_matches(layout);
        let head_len = item.len() - left.len();
        let core = left.trim_end_matches(layout);
        let mut edges = Vec::new();
        let mut bodies = Vec::new();
        let mut blank = Vec::new();
        for line in core.split('\n') {
            let after_indent = line.trim_start_matches(horizontal);
            let indent_len = line.len() - after_indent.len();
            let body = after_indent.trim_end_matches(horizontal);
            edges.push((
                line[..indent_len].to_owned(),
                after_indent[body.len()..].to_owned(),
            ));
            blank.push(body.is_empty());
            bodies.push(body);
        }
        Self {
            head: item[..head_len].to_owned(),
            tail: left[core.len()..].to_owned(),
            text: bodies.join("\n"),
            edges,
            blank,
        }
    }

    pub fn line_count(&self) -> usize {
        self.edges.len()
    }

    pub fn apply(&self, lines: &[String]) -> Result<String, CodecError> {
        if lines.len() != self.edges.len() {
            return Err(CodecError::LineCount {
                expected: self.edges.len(),
                actual: lines.len(),
            });
        }
        let mut out = self.head.clone();
        for (i, ((indent, trail), line)) in self.edges.iter().zip(lines).enumerate() {
            if line.contains('\n') {
                return Err(CodecError::EmbeddedLineBreak);
            }
            // Deliberate correction: a retained source blank cannot acquire hallucinated text.
            if self.blank[i] && !line.is_empty() {
                return Err(CodecError::BlankLineChanged);
            }
            if i != 0 {
                out.push('\n');
            }
            out.push_str(indent);
            out.push_str(line);
            out.push_str(trail);
        }
        out.push_str(&self.tail);
        Ok(out)
    }

    pub fn apply_text(&self, text: &str) -> Result<String, CodecError> {
        self.apply(&text.split('\n').map(str::to_owned).collect::<Vec<_>>())
    }
}

/// Collapse a model's redundant physical LF immediately after a line-break mark.
/// This follows the reference's mark + horizontal whitespace + optional CR + LF rule.
pub fn reply_lines(raw: &str, mark: char) -> Vec<String> {
    let raw = raw.trim_matches(layout);
    let mut rest = raw;
    let mut normalized = String::with_capacity(raw.len());
    while !rest.is_empty() {
        let c = rest.chars().next().expect("nonempty suffix");
        normalized.push(c);
        rest = &rest[c.len_utf8()..];
        if c == mark {
            let after_spaces = rest.trim_start_matches([' ', '\t']);
            let after_cr = after_spaces.strip_prefix('\r').unwrap_or(after_spaces);
            if let Some(after_lf) = after_cr.strip_prefix('\n') {
                rest = after_lf;
            }
        }
    }
    decode(&normalized, mark)
        .split('\n')
        .map(|line| line.trim_matches(horizontal).to_owned())
        .collect()
}

fn numbered_prefix(line: &str) -> Result<Option<(usize, &str)>, CodecError> {
    let line = line.trim_start_matches(char::is_whitespace);
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || !matches!(line.as_bytes().get(digits), Some(b'.' | b')')) {
        return Ok(None);
    }
    let n = line[..digits]
        .parse::<usize>()
        .map_err(|_| CodecError::NumberOverflow)?;
    let mut body = &line[digits + 1..];
    if let Some(c) = body.chars().next().filter(|c| c.is_whitespace()) {
        body = &body[c.len_utf8()..];
    }
    Ok(Some((n, body)))
}

/// Strict ordered raw item parser. Continuation lines may not begin another item number.
pub fn parse_raw_numbered(reply: &str, expected: usize) -> Result<Vec<String>, CodecError> {
    let reply = reply.trim_matches(layout);
    if expected == 0 {
        return if reply.is_empty() {
            Ok(Vec::new())
        } else {
            Err(CodecError::ItemCount {
                expected: 0,
                actual: 1,
            })
        };
    }
    let mut lines = reply.split('\n').collect::<Vec<_>>();
    if lines[0]
        .trim_start_matches(char::is_whitespace)
        .starts_with("```")
    {
        if lines.len() < 2 || lines.last().expect("at least one line").trim() != "```" {
            return Err(CodecError::UnclosedFence);
        }
        lines.remove(0);
        lines.pop();
    }
    let mut items: Vec<String> = Vec::new();
    for line in lines {
        if let Some((n, body)) = numbered_prefix(line)? {
            let next = items.len() + 1;
            if n != next || items.len() >= expected {
                return Err(CodecError::ItemSequence {
                    expected: next,
                    actual: n,
                });
            }
            items.push(body.to_owned());
        } else if let Some(last) = items.last_mut() {
            last.push('\n');
            last.push_str(line);
        } else if !line.trim().is_empty() {
            return Err(CodecError::TextBeforeFirstItem);
        }
    }
    if items.len() != expected {
        return Err(CodecError::ItemCount {
            expected,
            actual: items.len(),
        });
    }
    Ok(items)
}

pub fn parse_numbered(reply: &str, expected: usize, mark: char) -> Result<Vec<String>, CodecError> {
    Ok(parse_raw_numbered(reply, expected)?
        .into_iter()
        .map(|raw| reply_lines(&raw, mark).join("\n"))
        .collect())
}

/// A single unnumbered reply is safe to map only when the caller explicitly asked for one item.
/// The runtime may use this after strict parsing fails, then must still apply source Shape.
pub fn single_unnumbered(reply: &str, mark: char) -> Result<String, CodecError> {
    let trimmed = reply.trim_matches(layout);
    let mut lines = trimmed.split('\n').collect::<Vec<_>>();
    if lines[0]
        .trim_start_matches(char::is_whitespace)
        .starts_with("```")
    {
        if lines.len() < 2 || lines.last().expect("at least one line").trim() != "```" {
            return Err(CodecError::UnclosedFence);
        }
        lines.remove(0);
        lines.pop();
    }
    // A numbered-but-invalid multi-item answer is never an unnumbered fallback.
    for line in &lines {
        if numbered_prefix(line)?.is_some() {
            return Err(CodecError::ItemSequence {
                expected: 1,
                actual: numbered_prefix(line)?.expect("numbered prefix").0,
            });
        }
    }
    Ok(reply_lines(&lines.join("\n"), mark).join("\n"))
}

/// Stable index batches using Unicode code points, matching Python len rather than UTF-8 bytes.
/// Oversized single units remain intact; upstream format extraction owns any paragraph splitting.
pub fn batches(items: &[String], max_chars: usize) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut current = Vec::new();
    let mut size: usize = 0;
    for (i, text) in items.iter().enumerate() {
        let need = text.chars().count().saturating_add(8);
        if !current.is_empty()
            && (size.saturating_add(need) > max_chars || current.len() >= BATCH_ITEMS)
        {
            out.push(std::mem::take(&mut current));
            size = 0;
        }
        current.push(i);
        size = size.saturating_add(need);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Stable exact-string de-duplication. Layout/case are part of the string identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DedupPlan {
    pub unique: Vec<String>,
    pub indices: Vec<usize>,
}

impl DedupPlan {
    pub fn of(items: &[String]) -> Self {
        let mut unique = Vec::new();
        let mut indices = Vec::with_capacity(items.len());
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for item in items {
            let index = *seen.entry(item.as_str()).or_insert_with(|| {
                unique.push(item.clone());
                unique.len() - 1
            });
            indices.push(index);
        }
        Self { unique, indices }
    }

    pub fn restore(&self, translated: &[String]) -> Result<Vec<String>, CodecError> {
        if translated.len() != self.unique.len() {
            return Err(CodecError::ItemCount {
                expected: self.unique.len(),
                actual: translated.len(),
            });
        }
        Ok(self
            .indices
            .iter()
            .map(|&i| translated[i].clone())
            .collect())
    }
}
