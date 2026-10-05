use pdf_writer::{Content, Finish, Name, Pdf, Rect, Ref, Str};
use versora_engine::formats::{extract, supported_extension, write_document};

// Real PDF objects/content, rather than a mocked parser response or fake header.
fn source_pdf(pages: &[&[&str]]) -> Vec<u8> {
    let mut pdf = Pdf::new();
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let font = Ref::new(3);
    pdf.catalog(catalog).pages(tree);
    let ids: Vec<_> = (0..pages.len())
        .map(|i| Ref::new(4 + i as i32 * 2))
        .collect();
    pdf.pages(tree)
        .kids(ids.iter().copied())
        .count(pages.len() as i32);
    pdf.type1_font(font).base_font(Name(b"Helvetica"));
    for (lines, page_id) in pages.iter().zip(ids) {
        let content_id = Ref::new(page_id.get() + 1);
        let mut page = pdf.page(page_id);
        page.parent(tree)
            .media_box(Rect::new(0.0, 0.0, 595.0, 842.0))
            .contents(content_id);
        page.resources().fonts().pair(Name(b"F1"), font);
        page.finish();
        let mut content = Content::new();
        content
            .begin_text()
            .set_font(Name(b"F1"), 11.0)
            .set_leading(16.0);
        content.next_line(57.0, 774.0);
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                content.next_line_using_leading();
            }
            content.show(Str(line.as_bytes()));
        }
        content.end_text();
        pdf.stream(content_id, &content.finish());
    }
    pdf.finish()
}

fn artifact(name: &str, bytes: &[u8]) {
    if let Some(directory) = std::env::var_os("VERSORA_PDF_ARTIFACT_DIR") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(name), bytes).unwrap();
    }
}

fn image_only_pdf() -> Vec<u8> {
    let mut pdf = Pdf::new();
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let page_id = Ref::new(3);
    let image = Ref::new(4);
    let content_id = Ref::new(5);
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids([page_id]).count(1);
    let mut page = pdf.page(page_id);
    page.parent(tree)
        .media_box(Rect::new(0.0, 0.0, 595.0, 842.0))
        .contents(content_id);
    page.resources().x_objects().pair(Name(b"Im0"), image);
    page.finish();
    // Four actual RGB pixels, displayed on the page. No text/font resource.
    let pixels = [255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255];
    let mut image_stream = pdf.stream(image, &pixels);
    image_stream.pair(Name(b"Type"), Name(b"XObject"));
    image_stream.pair(Name(b"Subtype"), Name(b"Image"));
    image_stream.pair(Name(b"Width"), 2);
    image_stream.pair(Name(b"Height"), 2);
    image_stream.pair(Name(b"BitsPerComponent"), 8);
    image_stream.pair(Name(b"ColorSpace"), Name(b"DeviceRGB"));
    image_stream.finish();
    let mut content = Content::new();
    content.transform([200.0, 0.0, 0.0, 200.0, 57.0, 500.0]);
    content.x_object(Name(b"Im0"));
    pdf.stream(content_id, &content.finish());
    pdf.finish()
}

#[test]
fn pdf_selectable_text_pages_and_unchanged_bytes_are_preserved() {
    assert!(supported_extension(".PDF", "document"));
    let source = source_pdf(&[&["Hello world!"], &["Second source page."]]);
    let document = extract(&source, "pdf", "document").unwrap();
    assert_eq!(document.units, ["Hello world!", "Second source page."]);
    assert_eq!(write_document(&document, &document.units).unwrap(), source);
    assert!(write_document(&document, &["One translation".into()]).is_err());
    artifact("source.pdf", &source);
}

#[test]
fn pdf_unicode_translation_roundtrips_with_embedded_font() {
    let source = source_pdf(&[&["Translate this paragraph."]]);
    let original = source.clone();
    let document = extract(&source, "pdf", "document").unwrap();
    let translated = "繁體中文：檔案翻譯。日本語 한국어 café Ω Ж 𠀋";
    let result = write_document(&document, &[translated.into()]).unwrap();
    assert_eq!(source, original);
    assert!(result.starts_with(b"%PDF-"));
    assert!(result
        .windows(b"/FontFile3".len())
        .any(|x| x == b"/FontFile3"));
    assert!(result
        .windows(b"/OpenType".len())
        .any(|x| x == b"/OpenType"));
    assert!(result
        .windows(b"/ToUnicode".len())
        .any(|x| x == b"/ToUnicode"));
    assert_eq!(
        extract(&result, "pdf", "document").unwrap().units,
        [translated]
    );
    artifact("translated-unicode.pdf", &result);
}

#[test]
fn pdf_same_outline_unicode_characters_retain_distinct_text_mapping() {
    let source = source_pdf(&[&["Translate this source."]]);
    let document = extract(&source, "pdf", "document").unwrap();
    let translated = "A B\u{00a0}C：全形Ａ及拉丁A";
    let result = write_document(&document, &[translated.into()]).unwrap();
    let text = pdf_extract::extract_text_from_mem(&result).unwrap();
    assert!(text.contains(translated), "Actual extracted text: {text:?}");
    artifact("translated-unicode-aliases.pdf", &result);
}

#[test]
fn pdf_reflow_wraps_and_paginates_without_dropping_text() {
    let source = source_pdf(&[&["Translate a long paragraph."]]);
    let document = extract(&source, "pdf", "document").unwrap();
    let translated = "繁體中文檔案翻譯測試。".repeat(600);
    let result = write_document(&document, &[translated.clone()]).unwrap();
    let pages = pdf_extract::extract_text_from_mem_by_pages(&result).unwrap();
    assert!(pages.len() >= 3, "Actual page count: {}", pages.len());
    let recovered: String = pages
        .iter()
        .flat_map(|page| page.chars())
        .filter(|c| !c.is_whitespace())
        .collect();
    assert_eq!(recovered, translated);
    artifact("translated-multipage.pdf", &result);
}

#[test]
fn pdf_malformed_and_image_only_documents_report_errors_without_ocr_fallback() {
    assert!(extract(b"This is not a PDF", "pdf", "document").is_err());
    assert!(extract(b"%PDF-1.7\nmalformed", "pdf", "document").is_err());
    let blank = source_pdf(&[&[]]);
    let error = extract(&blank, "pdf", "document").unwrap_err().to_string();
    assert!(error.contains("no selectable text"));
    assert!(error.contains("OCR"));
    let image_only = image_only_pdf();
    let error = extract(&image_only, "pdf", "document")
        .unwrap_err()
        .to_string();
    assert!(error.contains("no selectable text"));
    assert!(error.contains("OCR"));
    artifact("image-only.pdf", &image_only);
}

#[test]
fn pdf_broken_later_page_never_returns_a_successful_partial_document() {
    let mut source = source_pdf(&[&["Valid first page."], &["Broken second page."]]);
    // Change the second page's font reference to a missing object, retaining
    // byte lengths and the original valid cross-reference offsets.
    let offset = source.windows(7).rposition(|s| s == b"/F1 3 0").unwrap();
    source[offset + 4] = b'9';
    assert!(extract(&source, "pdf", "document").is_err());
}

#[test]
fn pdf_unrenderable_symbols_and_controls_fail_explicitly() {
    let source = source_pdf(&[&["Translate this source."]]);
    let document = extract(&source, "pdf", "document").unwrap();
    let error = write_document(&document, &["Unsupported \u{10ffff}".into()])
        .unwrap_err()
        .to_string();
    assert!(error.contains("U+10FFFF"));
    assert!(write_document(&document, &["Invalid\0control".into()]).is_err());
}

#[test]
fn pdf_tabs_newlines_and_literal_markup_remain_plain_text() {
    let source = source_pdf(&[&["Translate this source."]]);
    let document = extract(&source, "pdf", "document").unwrap();
    let result = write_document(&document, &["第一行 <tag>&文字\r\n第二行\t內容".into()]).unwrap();
    let text = pdf_extract::extract_text_from_mem(&result).unwrap();
    assert!(text.contains("第一行 <tag>&文字"), "Actual text: {text:?}");
    assert!(text.contains("第二行"));
    assert!(text.contains("內容"));
    artifact("translated-line-controls.pdf", &result);
}
