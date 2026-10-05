//! Selectable-text PDF extraction and A4 text-reflow output.
//!
//! This intentionally matches the reference adapter's text-only contract. It is
//! not OCR, and translated output does not retain source graphics or layout.
//! The shipped OFL font is embedded once per PDF; no installed font is needed.

use super::{translatable, Context, Document, FormatError};
use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use std::{collections::BTreeMap, io::Write};
use ttf_parser::{Face, GlyphId};

const FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/fonts/NotoSansCJKtc-Regular.otf"
));
const MAX_INPUT_BYTES: usize = 80 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PAGES: usize = 4096;
const PAGE_WIDTH: f32 = 595.276;
const PAGE_HEIGHT: f32 = 841.89;
const MARGIN: f32 = 56.693; // 20 mm
const FONT_SIZE: f32 = 11.0;
const LEADING: f32 = 16.0;

#[derive(Debug)]
pub(super) struct PdfDocument {
    original: Vec<u8>,
    paragraphs: Vec<Paragraph>,
}

#[derive(Debug)]
struct Paragraph {
    source: String,
    unit: Option<usize>,
}

pub(super) fn extract(bytes: &[u8], game: bool) -> Result<Document, FormatError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(FormatError("PDF exceeds the 80 MiB input limit".into()));
    }
    // The dependency can reject malformed/encrypted documents. Convert a parser
    // panic into a document error rather than aborting the worker/application.
    let pages = std::panic::catch_unwind(|| extract_pages(bytes))
        .map_err(|_| FormatError("Cannot extract PDF text: malformed PDF".into()))??;
    let mut units = Vec::new();
    let mut paragraphs = Vec::new();
    for page in pages {
        let normalized = page.replace("\r\n", "\n").replace('\r', "\n");
        let mut paragraph = String::new();
        for line in normalized.lines().chain(std::iter::once("")) {
            if line.trim().is_empty() {
                if !paragraph.is_empty() {
                    let source = std::mem::take(&mut paragraph);
                    let unit = if !game || translatable(&source, true) {
                        let index = units.len();
                        units.push(source.clone());
                        Some(index)
                    } else {
                        None
                    };
                    paragraphs.push(Paragraph { source, unit });
                }
            } else {
                if !paragraph.is_empty() {
                    paragraph.push('\n');
                }
                paragraph.push_str(line.trim());
            }
        }
    }
    if paragraphs.is_empty() {
        return Err(FormatError(
            "PDF has no selectable text. OCR is unavailable; image-only pages are not translated"
                .into(),
        ));
    }
    Ok(Document {
        units,
        context: Context::Pdf(PdfDocument {
            original: bytes.to_vec(),
            paragraphs,
        }),
    })
}

fn extract_pages(bytes: &[u8]) -> Result<Vec<String>, FormatError> {
    let document = pdf_extract::Document::load_mem(bytes)
        .map_err(|e| FormatError(format!("Cannot open PDF: {e}")))?;
    if document.is_encrypted() {
        return Err(FormatError(
            "Encrypted PDFs must be decrypted before translation".into(),
        ));
    }
    let page_numbers = document.get_pages();
    if page_numbers.len() > MAX_PAGES {
        return Err(FormatError("PDF exceeds the 4096-page limit".into()));
    }
    let mut pages = Vec::with_capacity(page_numbers.len());
    let mut remaining = MAX_TEXT_BYTES;
    for page_number in page_numbers.keys() {
        let mut sink = BoundedText {
            bytes: Vec::new(),
            limit: remaining,
            exceeded: false,
        };
        // The convenience by-pages API stops at any page error. Calling the
        // page adapter explicitly ensures a broken later page cannot produce
        // an apparently successful partial translation.
        let extraction = {
            let writer: &mut dyn Write = &mut sink;
            let mut output = pdf_extract::PlainTextOutput::new(writer);
            pdf_extract::output_doc_page(&document, &mut output, *page_number)
        };
        if sink.exceeded {
            return Err(FormatError(
                "PDF exceeds the 16 MiB extracted-text limit".into(),
            ));
        }
        extraction
            .map_err(|e| FormatError(format!("Cannot extract PDF page {page_number}: {e}")))?;
        remaining -= sink.bytes.len();
        pages.push(
            String::from_utf8(sink.bytes)
                .map_err(|_| FormatError("PDF extractor produced invalid Unicode text".into()))?,
        );
    }
    Ok(pages)
}

struct BoundedText {
    bytes: Vec<u8>,
    limit: usize,
    exceeded: bool,
}
impl Write for BoundedText {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .bytes
            .len()
            .checked_add(bytes.len())
            .is_none_or(|size| size > self.limit)
        {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "PDF text limit",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn write(
    document: &PdfDocument,
    translations: &[String],
) -> Result<Vec<u8>, FormatError> {
    // A no-op translation should not discard the source's layout, images or
    // metadata. Reflow applies only when text actually changes.
    if document.paragraphs.iter().all(|p| {
        p.unit
            .map(|i| translations.get(i) == Some(&p.source))
            .unwrap_or(true)
    }) {
        return Ok(document.original.clone());
    }
    let mut paragraphs = Vec::with_capacity(document.paragraphs.len());
    let mut bytes = 0usize;
    for paragraph in &document.paragraphs {
        let text = match paragraph.unit {
            Some(index) => translations
                .get(index)
                .ok_or_else(|| FormatError("PDF translation index is missing".into()))?,
            None => &paragraph.source,
        };
        bytes = bytes
            .checked_add(text.len())
            .ok_or_else(|| FormatError("PDF translated-text size overflow".into()))?;
        if bytes > MAX_TEXT_BYTES {
            return Err(FormatError(
                "PDF exceeds the 16 MiB translated-text limit".into(),
            ));
        }
        paragraphs.push(text.as_str());
    }
    render_text(&paragraphs)
}

#[derive(Clone, Copy)]
struct Character {
    cid: u16,
    width: f32,
    font: usize,
}

fn render_text(paragraphs: &[&str]) -> Result<Vec<u8>, FormatError> {
    let face = Face::parse(FONT, 0)
        .map_err(|e| FormatError(format!("Cannot read bundled PDF font: {e:?}")))?;
    let cff = face
        .tables()
        .cff
        .ok_or_else(|| FormatError("Bundled PDF font has no CFF outlines".into()))?;
    let scale = 1000.0 / f32::from(face.units_per_em());
    let mut characters = BTreeMap::new();
    let mut font_maps: Vec<BTreeMap<u16, char>> = vec![BTreeMap::new()];
    for paragraph in paragraphs {
        for ch in paragraph.chars().chain(std::iter::once(' ')) {
            if matches!(ch, '\n' | '\r' | '\t' | '\u{2028}' | '\u{2029}') {
                continue;
            }
            if ch.is_control() {
                return Err(FormatError(format!(
                    "PDF output contains an unsupported control character U+{:04X}",
                    ch as u32
                )));
            }
            if characters.contains_key(&ch) {
                continue;
            }
            let glyph = face
                .glyph_index(ch)
                .filter(|g| *g != GlyphId(0))
                .ok_or_else(|| {
                    FormatError(format!(
                    "Bundled PDF font cannot render U+{:04X}; this script or symbol is unsupported",
                    ch as u32
                ))
                })?;
            let cid = cff.glyph_cid(glyph).ok_or_else(|| {
                FormatError("Bundled PDF font is not a CID-keyed CFF font".into())
            })?;
            let width = f32::from(face.glyph_hor_advance(glyph).ok_or_else(|| {
                FormatError("Bundled PDF font has no character advance width".into())
            })?) * scale;
            // Unicode aliases can share a CID (for example space and NBSP).
            // Separate Type0 resources keep their ToUnicode maps unambiguous;
            // all resources share one embedded, unmodified font file.
            let font = if let Some(index) = font_maps.iter().position(|map| !map.contains_key(&cid))
            {
                index
            } else {
                font_maps.push(BTreeMap::new());
                font_maps.len() - 1
            };
            font_maps[font].insert(cid, ch);
            characters.insert(ch, Character { cid, width, font });
        }
    }

    let lines = wrap_lines(paragraphs, &characters)?;
    let lines_per_page = ((PAGE_HEIGHT - 2.0 * MARGIN) / LEADING).floor() as usize;
    let page_count = lines.len().max(1).div_ceil(lines_per_page);
    if page_count > MAX_PAGES {
        return Err(FormatError(
            "Reflowed PDF exceeds the 4096-page limit".into(),
        ));
    }
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let descendant = Ref::new(3);
    let descriptor = Ref::new(4);
    let font_file = Ref::new(5);
    let font_refs: Vec<_> = (0..font_maps.len())
        .map(|i| Ref::new(6 + i as i32 * 2))
        .collect();
    let font_names: Vec<_> = (0..font_maps.len())
        .map(|i| format!("F{}", i + 1).into_bytes())
        .collect();
    let mut pdf = Pdf::new();
    pdf.catalog(catalog).pages(tree);
    let page_refs: Vec<_> = (0..page_count)
        .map(|i| Ref::new(6 + (font_maps.len() + i) as i32 * 2))
        .collect();
    pdf.pages(tree)
        .kids(page_refs.iter().copied())
        .count(page_count as i32);

    // Identity-H codes are actual CFF CIDs, never assumed OpenType glyph IDs.
    // The dependency's custom Encoding parser is incomplete, so standard
    // Identity-H also ensures PDFs we emit can be read back by this adapter.
    for (i, font) in font_refs.iter().copied().enumerate() {
        let unicode = Ref::new(font.get() + 1);
        let mut type0 = pdf.indirect(font).dict();
        type0.pair(Name(b"Type"), Name(b"Font"));
        type0.pair(Name(b"Subtype"), Name(b"Type0"));
        type0.pair(Name(b"BaseFont"), Name(b"NotoSansCJKtc-Regular"));
        type0.pair(Name(b"Encoding"), Name(b"Identity-H"));
        type0
            .insert(Name(b"DescendantFonts"))
            .array()
            .item(descendant);
        type0.pair(Name(b"ToUnicode"), unicode);
        type0.finish();
        pdf.stream(unicode, &unicode_cmap(&font_maps[i]));
    }

    let mut cid_font = pdf.indirect(descendant).dict();
    cid_font.pair(Name(b"Type"), Name(b"Font"));
    cid_font.pair(Name(b"Subtype"), Name(b"CIDFontType0"));
    cid_font.pair(Name(b"BaseFont"), Name(b"NotoSansCJKtc-Regular"));
    cid_font.pair(Name(b"FontDescriptor"), descriptor);
    cid_font.pair(Name(b"DW"), 1000);
    let mut system = cid_font.insert(Name(b"CIDSystemInfo")).dict();
    system.pair(Name(b"Registry"), Str(b"Adobe"));
    system.pair(Name(b"Ordering"), Str(b"Identity"));
    system.pair(Name(b"Supplement"), 0);
    system.finish();
    let mut widths = BTreeMap::new();
    for character in characters.values() {
        widths.insert(character.cid, character.width);
    }
    let mut width_array = cid_font.insert(Name(b"W")).array();
    for (cid, width) in widths {
        width_array.item(i32::from(cid));
        width_array.push().array().item(width);
    }
    width_array.finish();
    cid_font.finish();

    let bbox = face.global_bounding_box();
    let mut desc = pdf.indirect(descriptor).dict();
    desc.pair(Name(b"Type"), Name(b"FontDescriptor"));
    desc.pair(Name(b"FontName"), Name(b"NotoSansCJKtc-Regular"));
    desc.pair(Name(b"Flags"), 4); // symbolic composite font
    desc.pair(Name(b"ItalicAngle"), 0);
    desc.pair(Name(b"Ascent"), f32::from(face.ascender()) * scale);
    desc.pair(Name(b"Descent"), f32::from(face.descender()) * scale);
    desc.pair(
        Name(b"CapHeight"),
        f32::from(face.capital_height().unwrap_or(face.ascender())) * scale,
    );
    desc.pair(Name(b"StemV"), 80);
    desc.insert(Name(b"FontBBox")).array().items([
        f32::from(bbox.x_min) * scale,
        f32::from(bbox.y_min) * scale,
        f32::from(bbox.x_max) * scale,
        f32::from(bbox.y_max) * scale,
    ]);
    desc.pair(Name(b"FontFile3"), font_file);
    desc.finish();
    pdf.stream(font_file, FONT)
        .pair(Name(b"Subtype"), Name(b"OpenType"));

    for (i, page_id) in page_refs.into_iter().enumerate() {
        let stream_id = Ref::new(page_id.get() + 1);
        let mut page = pdf.page(page_id);
        page.parent(tree)
            .media_box(Rect::new(0.0, 0.0, PAGE_WIDTH, PAGE_HEIGHT))
            .contents(stream_id);
        let mut resources = page.resources();
        let mut fonts = resources.fonts();
        for (name, font) in font_names.iter().zip(&font_refs) {
            fonts.pair(Name(name), *font);
        }
        fonts.finish();
        resources.finish();
        page.finish();
        let mut content = Content::new();
        content.begin_text();
        content.set_font(Name(&font_names[0]), FONT_SIZE);
        content.set_leading(LEADING);
        content.next_line(MARGIN, PAGE_HEIGHT - MARGIN - FONT_SIZE);
        let start = i * lines_per_page;
        for (offset, line) in lines.iter().skip(start).take(lines_per_page).enumerate() {
            if offset > 0 {
                content.next_line_using_leading();
            }
            let mut encoded = Vec::with_capacity(line.len() * 2);
            let mut active_font = None;
            for ch in line.chars() {
                let character = characters[&ch];
                if active_font != Some(character.font) {
                    if !encoded.is_empty() {
                        content.show(Str(&encoded));
                        encoded.clear();
                    }
                    content.set_font(Name(&font_names[character.font]), FONT_SIZE);
                    active_font = Some(character.font);
                }
                encoded.extend_from_slice(&character.cid.to_be_bytes());
            }
            content.show(Str(&encoded));
        }
        content.end_text();
        pdf.stream(stream_id, &content.finish());
    }
    Ok(pdf.finish())
}

fn wrap_lines(
    paragraphs: &[&str],
    characters: &BTreeMap<char, Character>,
) -> Result<Vec<String>, FormatError> {
    let mut lines = Vec::new();
    let available = PAGE_WIDTH - 2.0 * MARGIN;
    for (paragraph_index, paragraph) in paragraphs.iter().enumerate() {
        if paragraph_index > 0 {
            lines.push(String::new());
        }
        let normalized = paragraph
            .replace("\r\n", "\n")
            .replace(['\r', '\u{2028}', '\u{2029}'], "\n")
            .replace('\t', "    ");
        for explicit_line in normalized.split('\n') {
            let mut line = String::new();
            let mut width = 0.0;
            for ch in explicit_line.chars() {
                let advance = characters[&ch].width * FONT_SIZE / 1000.0;
                if width + advance > available && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    width = 0.0;
                }
                line.push(ch);
                width += advance;
            }
            lines.push(line);
        }
        if lines.len() > MAX_PAGES * 48 {
            return Err(FormatError(
                "Reflowed PDF exceeds the 4096-page limit".into(),
            ));
        }
    }
    Ok(lines)
}

fn cmap_header(name: &str, kind: u8) -> String {
    format!(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n/CMapName /{name} def\n/CMapType {kind} def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n"
    )
}
fn cmap_footer(output: &mut String) {
    output.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
}
fn unicode_cmap(characters: &BTreeMap<u16, char>) -> Vec<u8> {
    let mut output = cmap_header("Versora-Unicode", 2);
    let entries: Vec<_> = characters.iter().collect();
    for chunk in entries.chunks(100) {
        output.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (cid, ch) in chunk {
            let mut buffer = [0; 2];
            let encoded = ch.encode_utf16(&mut buffer);
            let unicode: String = encoded.iter().map(|u| format!("{u:04X}")).collect();
            output.push_str(&format!("<{cid:04X}> <{unicode}>\n"));
        }
        output.push_str("endbfchar\n");
    }
    cmap_footer(&mut output);
    output.into_bytes()
}
