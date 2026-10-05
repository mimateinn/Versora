//! CSV/TSV values use the real CSV parser; writeback replaces only selected cells.
//! Delimiters, record endings, the UTF-8 BOM and untouched fields remain byte exact.
use super::{edits_document, translatable, Document, Edit, Encoding, FormatError};
use std::ops::Range;

const MAX_BYTES: usize = 80 * 1024 * 1024;
const MAX_CELLS: usize = 200_000;
const MAX_CELL_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn extract(bytes: &[u8], delimiter: u8, game: bool) -> Result<Document, FormatError> {
    if bytes.len() > MAX_BYTES {
        return Err(FormatError(
            "CSV/TSV exceeds the 80 MiB document limit".into(),
        ));
    }
    let text = std::str::from_utf8(bytes)?;
    let bom = usize::from(text.starts_with('\u{feff}')) * 3;
    let input = &bytes[bom..];
    let mut parser = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(input);
    let mut parsed = Vec::new();
    let mut cells = 0usize;
    for row in parser.records() {
        let row = row.map_err(|error| FormatError(format!("Cannot parse CSV/TSV: {error}")))?;
        cells = cells
            .checked_add(row.len())
            .ok_or_else(|| FormatError("CSV/TSV cell count overflow".into()))?;
        if cells > MAX_CELLS || row.iter().any(|cell| cell.len() > MAX_CELL_BYTES) {
            return Err(FormatError(
                "CSV/TSV exceeds 200000 cells or the 4 MiB cell limit".into(),
            ));
        }
        parsed.push(row);
    }
    // The CSV crate deliberately accepts some malformed quoting. This bounded
    // scanner requires unambiguous cell boundaries before applying byte edits.
    let ranges = field_ranges(input, delimiter)?;
    if ranges.len() != parsed.len() {
        return Err(FormatError("CSV/TSV record framing is ambiguous".into()));
    }
    let mut units = Vec::new();
    let mut edits = Vec::new();
    for (row, ranges) in parsed.iter().zip(ranges) {
        if row.len() != ranges.len() {
            return Err(FormatError("CSV/TSV field framing is ambiguous".into()));
        }
        for (value, range) in row.iter().zip(ranges) {
            if translatable(value, game) {
                let unit = units.len();
                units.push(value.to_string());
                edits.push(Edit {
                    range: range.start + bom..range.end + bom,
                    unit,
                    encoding: Encoding::Csv { delimiter },
                });
            }
        }
    }
    Ok(edits_document(bytes, units, edits))
}

fn field_ranges(input: &[u8], delimiter: u8) -> Result<Vec<Vec<Range<usize>>>, FormatError> {
    let mut rows = Vec::new();
    let mut position = 0usize;
    while position < input.len() {
        if matches!(input[position], b'\r' | b'\n') {
            consume_ending(input, &mut position);
            continue;
        }
        let mut row = Vec::new();
        loop {
            let start = position;
            if input.get(position) == Some(&b'"') {
                position += 1;
                loop {
                    match input.get(position) {
                        Some(b'"') if input.get(position + 1) == Some(&b'"') => position += 2,
                        Some(b'"') => {
                            position += 1;
                            break;
                        }
                        Some(_) => position += 1,
                        None => return Err(FormatError("Unclosed CSV/TSV quoted field".into())),
                    }
                }
                if input
                    .get(position)
                    .is_some_and(|byte| *byte != delimiter && !matches!(byte, b'\r' | b'\n'))
                {
                    return Err(FormatError(
                        "Unexpected bytes after a CSV/TSV closing quote".into(),
                    ));
                }
            } else {
                while input
                    .get(position)
                    .is_some_and(|byte| *byte != delimiter && !matches!(byte, b'\r' | b'\n'))
                {
                    if input[position] == b'"' {
                        return Err(FormatError("Quote inside an unquoted CSV/TSV field".into()));
                    }
                    position += 1;
                }
            }
            row.push(start..position);
            if input.get(position) == Some(&delimiter) {
                position += 1;
            } else {
                consume_ending(input, &mut position);
                break;
            }
        }
        rows.push(row);
    }
    Ok(rows)
}

fn consume_ending(input: &[u8], position: &mut usize) {
    match input.get(*position) {
        Some(b'\r') => {
            *position += 1;
            if input.get(*position) == Some(&b'\n') {
                *position += 1;
            }
        }
        Some(b'\n') => *position += 1,
        _ => {}
    }
}

pub(super) fn encode_cell(value: &str, delimiter: u8) -> Result<String, FormatError> {
    if value.len() > MAX_CELL_BYTES {
        return Err(FormatError("Translated CSV/TSV cell exceeds 4 MiB".into()));
    }
    let mut writer = csv::WriterBuilder::new()
        .has_headers(false)
        .delimiter(delimiter)
        .terminator(csv::Terminator::Any(b'\n'))
        .from_writer(Vec::new());
    writer
        .write_record([value])
        .map_err(|error| FormatError(format!("Cannot write CSV/TSV cell: {error}")))?;
    let mut encoded = writer
        .into_inner()
        .map_err(|error| FormatError(format!("Cannot finish CSV/TSV cell: {error}")))?;
    if encoded.pop() != Some(b'\n') {
        return Err(FormatError(
            "CSV/TSV writer omitted its record terminator".into(),
        ));
    }
    String::from_utf8(encoded).map_err(|error| FormatError(error.to_string()))
}
