//! Pure terminology rules; file loading, canonical-directory probing and atomic writes belong
//! to the desktop service. Existing rows are never sorted, deduplicated, or silently replaced.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Term {
    pub term: String,
    pub translation: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlossaryError {
    InvalidJson(String),
    ExpectedArray,
}

impl fmt::Display for GlossaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => write!(f, "invalid glossary JSON: {message}"),
            Self::ExpectedArray => f.write_str("glossary JSON must be an array"),
        }
    }
}

impl Error for GlossaryError {}

/// Windows device names are unsafe even before a file extension; preserve intent by prefixing.
pub fn is_windows_device(name: &str) -> bool {
    let first = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches([' ', '.'])
        .to_uppercase();
    if matches!(
        first.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    ["COM", "LPT"].iter().any(|prefix| {
        first.strip_prefix(prefix).is_some_and(|rest| {
            matches!(
                rest,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

/// Unicode letters/numbers, underscore and hyphen remain; runs of other characters become '_'.
/// The 64-character limit counts Unicode scalar values rather than UTF-8 bytes. Reserved Windows
/// names receive '_' (a deliberate correction to the reference's unsafe CON/COM1 result).
pub fn safe_project_name(name: &str) -> String {
    let mut out = String::new();
    let mut replacing = false;
    for c in name.trim().chars() {
        if c.is_alphanumeric() || matches!(c, '_' | '-') {
            out.push(c);
            replacing = false;
        } else if !replacing {
            out.push('_');
            replacing = true;
        }
    }
    let mut safe: String = out.trim_matches('_').chars().take(64).collect();
    if safe.is_empty() {
        safe = "default".to_owned();
    }
    if is_windows_device(&safe) {
        safe.insert(0, '_');
    }
    safe
}

/// Resolve typed spelling against caller-supplied existing names without touching disk.
/// Use this only for a Windows case-insensitive project store. The service must supply genuine
/// project directory names and verify containment; this function does not infer filesystem identity.
/// Unicode lowercase comparison is deterministic, but is not full filesystem normalization.
pub fn canonical_project_name(name: &str, existing: &[String]) -> String {
    let safe = safe_project_name(name);
    let key = safe.to_lowercase();
    existing
        .iter()
        .find(|old| old.to_lowercase() == key)
        .cloned()
        .unwrap_or(safe)
}

/// Remove blank-term rows and trim the values exactly as the reference load path does.
pub fn clean_terms(terms: &[Term]) -> Vec<Term> {
    terms
        .iter()
        .filter_map(|row| {
            let term = row.term.trim();
            (!term.is_empty()).then(|| Term {
                term: term.to_owned(),
                translation: row.translation.trim().to_owned(),
            })
        })
        .collect()
}

fn scalar_text(value: Option<&Value>) -> Option<String> {
    match value {
        None => Some(String::new()),
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Bool(value)) => Some(if *value { "True" } else { "False" }.to_owned()),
        Some(Value::Number(value)) => Some(value.to_string()),
        // Deliberate correction: null/structured objects are not terminology strings.
        Some(Value::Null | Value::Array(_) | Value::Object(_)) => None,
    }
}

/// Import both modern {term,translation} rows and legacy [term,translation] pairs.
/// Malformed JSON/non-array roots return an error so a service cannot mistake corruption for an
/// empty glossary and overwrite user data. Invalid row shapes are skipped, matching reference.
pub fn parse_glossary_json(text: &str) -> Result<Vec<Term>, GlossaryError> {
    let data: Value =
        serde_json::from_str(text).map_err(|e| GlossaryError::InvalidJson(e.to_string()))?;
    let rows = data.as_array().ok_or(GlossaryError::ExpectedArray)?;
    let mut terms = Vec::new();
    for row in rows {
        let fields = match row {
            Value::Object(row) => Some((row.get("term"), row.get("translation"))),
            Value::Array(row) if row.len() >= 2 => Some((row.first(), row.get(1))),
            _ => None,
        };
        if let Some((term, translation)) = fields {
            if let (Some(term), Some(translation)) = (scalar_text(term), scalar_text(translation)) {
                terms.push(Term { term, translation });
            }
        }
    }
    Ok(clean_terms(&terms))
}

pub fn glossary_json(terms: &[Term]) -> Result<String, GlossaryError> {
    let clean = clean_terms(terms);
    serde_json::to_string_pretty(&clean)
        .map(|s| s + "\n")
        .map_err(|e| GlossaryError::InvalidJson(e.to_string()))
}

pub fn glossary_to_prompt_block(terms: &[Term]) -> String {
    if terms.is_empty() {
        return String::new();
    }
    let mut lines = vec!["Preferred terminology (must follow when the term appears):".to_owned()];
    for row in terms {
        lines.push(if row.translation.is_empty() {
            format!("- {}", row.term)
        } else {
            format!("- {} → {}", row.term, row.translation)
        });
    }
    lines.join("\n")
}

/// Append glossary last, keeping its explicit priority above the caller's shipped/custom prompt.
pub fn append_glossary(system: &str, terms: &[Term]) -> String {
    let block = glossary_to_prompt_block(terms);
    if block.is_empty() {
        system.to_owned()
    } else {
        format!("{system}\n\n{block}\nThese glossary terms override everything above.")
    }
}
