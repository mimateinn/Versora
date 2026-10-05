//! Pure model-id and output-name decisions. Actual exclusive-create/reservation belongs to the
//! writer; a name chosen from a directory snapshot is not a filesystem race lock.

use crate::glossary::is_windows_device;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    EmptyFilename,
    InvalidFilename,
    NameSpaceExhausted,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyFilename => "a source filename is required",
            Self::InvalidFilename => "the source filename is not a safe Windows filename",
            Self::NameSpaceExhausted => "no unique output filename is available",
        })
    }
}

impl Error for NameError {}

pub fn valid_model_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-'))
}

/// Preserve plausible custom ids; explicit Auto has no model. The provider default is injected.
pub fn resolve_model(
    provider: &str,
    chosen: Option<&str>,
    default: Option<&str>,
) -> Option<String> {
    if provider.trim().eq_ignore_ascii_case("auto") {
        return None;
    }
    let chosen = chosen.unwrap_or("").trim();
    if valid_model_id(chosen) {
        return Some(chosen.to_owned());
    }
    default
        .map(str::trim)
        .filter(|id| valid_model_id(id))
        .map(str::to_owned)
}

/// Stable display suggestions: injected provider default, reported ids, then bundled suggestions.
pub fn model_suggestions(
    default: Option<&str>,
    reported: &[String],
    bundled: &[String],
) -> Vec<String> {
    let mut seen = HashSet::new();
    default
        .into_iter()
        .chain(reported.iter().map(String::as_str))
        .chain(bundled.iter().map(String::as_str))
        .filter(|id| valid_model_id(id) && seen.insert((*id).to_owned()))
        .map(str::to_owned)
        .collect()
}

/// Only a filesystem tag is sanitized. The caller must send the original language text to models.
/// Bounded to 64 Unicode scalar values to prevent free-text targets exhausting component limits.
pub fn target_tag(target: &str) -> String {
    let mut out = String::new();
    let mut replacing = false;
    for c in target.chars() {
        if c.is_alphanumeric() || matches!(c, '_' | '-') {
            out.push(c);
            replacing = false;
        } else if !replacing {
            out.push('_');
            replacing = true;
        }
    }
    let tag: String = out.trim_matches('_').chars().take(64).collect();
    if tag.is_empty() {
        "target".to_owned()
    } else {
        tag
    }
}

fn basename(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn split_extension(name: &str) -> (&str, &str) {
    if let Some((stem, extension)) = name.rsplit_once('.') {
        if !stem.is_empty() && !extension.is_empty() {
            return (stem, &name[stem.len()..]);
        }
    }
    (name, "")
}

fn valid_filename(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name.ends_with([' ', '.'])
        && !name.chars().any(|c| {
            c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        })
}

fn bounded_name(stem: &str, tail: &str) -> Result<String, NameError> {
    let suffix_units = tail.encode_utf16().count();
    if suffix_units >= 255 {
        return Err(NameError::InvalidFilename);
    }
    let mut units = 0;
    let stem: String = stem
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= 255 - suffix_units
        })
        .collect();
    if stem.is_empty() {
        return Err(NameError::InvalidFilename);
    }
    Ok(format!("{stem}{tail}"))
}

/// Return a basename such as `manual.繁體中文.md`; never return caller-supplied directories.
/// Preserve source spelling, normalize suffix case, and protect Windows device stems.
pub fn output_name(input_name: &str, target: &str) -> Result<String, NameError> {
    let name = basename(input_name);
    if name.is_empty() {
        return Err(NameError::EmptyFilename);
    }
    if !valid_filename(name) {
        return Err(NameError::InvalidFilename);
    }
    let (stem, extension) = split_extension(name);
    let stem = if is_windows_device(stem) {
        format!("file_{stem}")
    } else {
        stem.to_owned()
    };
    bounded_name(
        &stem,
        &format!(".{}{}", target_tag(target), extension.to_lowercase()),
    )
}

/// Deterministic `(2)`, `(3)` collision suffixes, using caller-supplied Windows directory names.
/// No clock or random input: the writer must atomically reserve the resulting filename and retry
/// after a concurrent collision. Existing translated output is never intentionally overwritten.
pub fn unique_output_name(base: &str, existing: &[String]) -> Result<String, NameError> {
    if !valid_filename(base) || is_windows_device(base) || base.encode_utf16().count() > 255 {
        return Err(NameError::InvalidFilename);
    }
    let occupied: HashSet<String> = existing.iter().map(|name| name.to_lowercase()).collect();
    if !occupied.contains(&base.to_lowercase()) {
        return Ok(base.to_owned());
    }
    let (stem, extension) = split_extension(base);
    for i in 2..=existing.len().saturating_add(2) {
        let candidate = bounded_name(stem, &format!(" ({i}){extension}"))?;
        if !occupied.contains(&candidate.to_lowercase()) {
            return Ok(candidate);
        }
    }
    Err(NameError::NameSpaceExhausted)
}

/// Reference's English batch root tag. `kind` is also sanitized rather than trusted as a path.
pub fn english_job_name(kind: &str, name: &str) -> String {
    fn safe_part(value: &str, fallback: &str) -> String {
        let mut safe = String::new();
        let mut replacing = false;
        for c in value.chars() {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                safe.push(c);
                replacing = false;
            } else if !replacing {
                safe.push('_');
                replacing = true;
            }
        }
        let mut safe = safe.trim_matches(['.', '_']).to_owned();
        if safe.is_empty() {
            safe = fallback.to_owned();
        }
        if !safe.starts_with(|c: char| c.is_ascii_alphabetic()) {
            safe.insert_str(0, "job_");
        }
        safe.chars().take(40).collect()
    }
    format!("{}_{}", safe_part(kind, "job"), safe_part(name, "job"))
}
